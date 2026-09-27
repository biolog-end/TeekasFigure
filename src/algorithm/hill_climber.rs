// Evolutionary algorithm: tournament selection, mutation, multi-generation refinement

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::gpu::GpuContext;
use crate::settings::Settings;
use crate::types::{CandidateParams, GenerationState, StepResult};

use super::CandidateGenerator;

/// Evolutionary shape placer that uses tournament selection and mutation.
///
/// Algorithm per step:
/// 1. Generate initial population of random candidates
/// 2. Evaluate fitness (delta error) on GPU
/// 3. Select top survivors (survival_rate from settings)
/// 4. Create mutated children from survivors
/// 5. Repeat for num_generations
/// 6. Place the single best candidate if it improves the canvas enough
pub struct HillClimber {
    /// Current mean squared error (tracked for overlay display)
    pub current_mse: f32,
    /// Number of shapes successfully placed on the canvas
    pub placed_shapes: u32,
    /// Current state of the generation process
    pub state: GenerationState,
    /// RNG for mutations
    rng: SmallRng,
    /// Count of consecutive rejections (used for stopping criterion)
    consecutive_rejections: u32,
    /// Per-shape-texture penalty for diversity mode (indexed by shape_index)
    pub shape_penalties: Vec<f32>,
}

impl HillClimber {
    pub fn new() -> Self {
        Self {
            current_mse: f32::INFINITY,
            placed_shapes: 0,
            state: GenerationState::Running,
            rng: SmallRng::from_entropy(),
            consecutive_rejections: 0,
            shape_penalties: Vec::new(),
        }
    }

    /// Execute one evolutionary cycle to find and place the best shape.
    ///
    /// Returns Accepted if a shape was placed, Rejected if no improvement found,
    /// or Completed if the stopping criterion is met.
    pub fn step(
        &mut self,
        gpu: &GpuContext,
        generator: &mut CandidateGenerator,
        settings: &Settings,
    ) -> StepResult {
        if self.placed_shapes == 0 && generator.empty_canvas_is_solution() {
            gpu.clear_canvas();
            self.current_mse = 0.0;
            self.state = GenerationState::Completed;
            return StepResult::Completed;
        }
        // Stopping criterion: if we've had too many consecutive rejections,
        // the image is converged enough
        if self.consecutive_rejections >= settings.max_rejections {
            self.state = GenerationState::Completed;
            return StepResult::Completed;
        }

        // Also stop at max_shapes as a hard limit
        if self.placed_shapes >= settings.max_shapes {
            self.state = GenerationState::Completed;
            return StepResult::Completed;
        }

        // Generate initial population
        let mut population = generator.generate_batch(
            settings.batch_size,
            self.placed_shapes,
            gpu.canvas_size,
            gpu.num_shapes,
        );

        if population.is_empty() {
            return StepResult::Rejected;
        }

        // Assign colors based on target image (average color under shape area)
        for candidate in population.iter_mut() {
            let (r, g, b) = generator.sample_color_at(candidate.x as u32, candidate.y as u32);
            candidate.r = r;
            candidate.g = g;
            candidate.b = b;
        }

        let mut population = CachedPopulation::new(population);
        // The canvas and target stay fixed throughout this search. Keep raw
        // scores for unchanged survivors and evaluate only their new children.
        gpu.control.begin_search(settings.num_generations);
        let mut best_candidate: Option<CandidateParams> = None;
        let mut best_score = f32::MAX;

        // Initialize shape penalties if needed (diversity mode)
        if settings.diversity_mode && self.shape_penalties.len() < gpu.num_shapes as usize {
            self.shape_penalties.resize(gpu.num_shapes as usize, 0.0);
        }

        for _gen in 0..settings.num_generations {
            if !gpu.control.checkpoint() {
                return StepResult::Completed;
            }
            gpu.control
                .generations
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let Some(scores) = population.evaluate_with(|fresh| gpu.evaluate_candidates(fresh))
            else {
                return StepResult::Completed;
            };
            if !gpu.control.checkpoint() {
                return StepResult::Completed;
            }

            // Find the best in this generation (with diversity penalty if enabled)
            for (i, &score) in scores.iter().enumerate() {
                let penalized = if settings.diversity_mode {
                    let shape_idx = population.candidates[i].shape_index as usize;
                    score + self.shape_penalties.get(shape_idx).copied().unwrap_or(0.0)
                } else {
                    score
                };
                if penalized < best_score {
                    best_score = penalized;
                    best_candidate = Some(population.candidates[i]);
                }
            }

            gpu.control.searched(best_score);
            // Select top survivors by fitness (with diversity penalty)
            let penalized_scores: Vec<f32> = if settings.diversity_mode {
                scores
                    .iter()
                    .enumerate()
                    .map(|(i, &s)| {
                        let idx = population.candidates[i].shape_index as usize;
                        s + self.shape_penalties.get(idx).copied().unwrap_or(0.0)
                    })
                    .collect()
            } else {
                scores.clone()
            };

            let num_survivors =
                ((population.candidates.len() as f32 * settings.survival_rate) as usize).max(1);
            let survivors = top_indices_by_score(&penalized_scores, num_survivors);
            population = population.breed(
                &survivors,
                gpu.canvas_size,
                generator,
                settings,
                &mut self.rng,
            );
        }

        // Final evaluation of the last generation
        let Some(scores) = population.evaluate_with(|fresh| gpu.evaluate_candidates(fresh)) else {
            return StepResult::Completed;
        };
        if !gpu.control.checkpoint() {
            return StepResult::Completed;
        }

        for (i, &score) in scores.iter().enumerate() {
            let penalized = if settings.diversity_mode {
                let shape_idx = population.candidates[i].shape_index as usize;
                score + self.shape_penalties.get(shape_idx).copied().unwrap_or(0.0)
            } else {
                score
            };
            if penalized < best_score {
                best_score = penalized;
                best_candidate = Some(population.candidates[i]);
            }
        }

        gpu.control.searched(best_score);
        // Accept if the best candidate improves the canvas enough
        match best_candidate {
            Some(winner)
                if !settings.use_min_improvement || best_score < settings.min_improvement =>
            {
                gpu.composite_shape(&winner);
                self.placed_shapes += 1;
                gpu.control.placed.store(
                    u64::from(self.placed_shapes),
                    std::sync::atomic::Ordering::Relaxed,
                );
                gpu.control
                    .rejections
                    .store(0, std::sync::atomic::Ordering::Relaxed);
                self.consecutive_rejections = 0;

                // Update diversity penalties
                if settings.diversity_mode {
                    let chosen_idx = winner.shape_index as usize;
                    // Increase penalty for the chosen shape
                    if chosen_idx < self.shape_penalties.len() {
                        self.shape_penalties[chosen_idx] += settings.diversity_penalty_increment;
                    }
                    // Decay penalties for all OTHER shapes (if enabled)
                    if settings.diversity_decay_enabled {
                        for (idx, penalty) in self.shape_penalties.iter_mut().enumerate() {
                            if idx != chosen_idx {
                                *penalty = (*penalty - settings.diversity_decay_amount).max(0.0);
                            }
                        }
                    }
                }

                StepResult::Accepted(winner)
            }
            _ => {
                self.consecutive_rejections += 1;
                gpu.control.rejections.store(
                    u64::from(self.consecutive_rejections),
                    std::sync::atomic::Ordering::Relaxed,
                );
                StepResult::Rejected
            }
        }
    }
}

/// Scores live only for one placement/rebirth search, never across canvas edits.
struct CachedPopulation {
    candidates: Vec<CandidateParams>,
    scores: Vec<Option<f32>>,
}
impl CachedPopulation {
    fn new(candidates: Vec<CandidateParams>) -> Self {
        Self {
            scores: vec![None; candidates.len()],
            candidates,
        }
    }

    fn evaluate_with(
        &mut self,
        evaluate: impl FnOnce(&[CandidateParams]) -> Vec<f32>,
    ) -> Option<Vec<f32>> {
        let fresh: Vec<_> = self
            .candidates
            .iter()
            .zip(&self.scores)
            .filter_map(|(candidate, score)| score.is_none().then_some(*candidate))
            .collect();
        if !fresh.is_empty() {
            let scores = evaluate(&fresh);
            // Cancellation can return an incomplete evaluation. Never cache it.
            if scores.len() != fresh.len() {
                return None;
            }
            let mut scores = scores.into_iter();
            for score in &mut self.scores {
                if score.is_none() {
                    *score = scores.next();
                }
            }
        }
        self.scores.iter().copied().collect()
    }

    fn breed(
        &self,
        survivors: &[usize],
        canvas_size: (u32, u32),
        generator: &CandidateGenerator,
        settings: &Settings,
        rng: &mut SmallRng,
    ) -> Self {
        let capacity = survivors.len() * (1 + settings.children_per_parent as usize);
        let mut candidates = Vec::with_capacity(capacity);
        let mut scores = Vec::with_capacity(capacity);
        for &index in survivors {
            let parent = self.candidates[index];
            candidates.push(parent);
            scores.push(self.scores[index]);
            for _ in 0..settings.children_per_parent {
                candidates.push(mutate_candidate(
                    &parent,
                    canvas_size,
                    generator,
                    settings,
                    rng,
                ));
                scores.push(None);
            }
        }
        Self { candidates, scores }
    }
}

/// Mutate a candidate: small random changes to position, rotation, scale, alpha.
/// Color is re-sampled from target at the new position.
///
/// Free function (takes an explicit `rng`) so the initial hill-climbing
/// placement and video rebirth share the exact same mutation behaviour.
pub fn mutate_candidate(
    parent: &CandidateParams,
    canvas_size: (u32, u32),
    generator: &CandidateGenerator,
    settings: &Settings,
    rng: &mut SmallRng,
) -> CandidateParams {
    let (cw, ch) = canvas_size;

    // Position: ±10% of canvas size
    let dx = rng.gen_range(-(cw as f32 * 0.1)..=(cw as f32 * 0.1));
    let dy = rng.gen_range(-(ch as f32 * 0.1)..=(ch as f32 * 0.1));
    let new_x = (parent.x + dx).clamp(0.0, (cw - 1) as f32);
    let new_y = (parent.y + dy).clamp(0.0, (ch - 1) as f32);

    // Rotation: ±0.5 radians
    let new_rotation = if settings.evolve_rotation {
        let dr = rng.gen_range(-0.5_f32..=0.5);
        (parent.rotation + dr).rem_euclid(std::f32::consts::TAU)
    } else {
        0.0
    };

    // Scale: ±30% (multiplicative) on the X axis.
    let scale_factor = rng.gen_range(0.7_f32..=1.3);
    let new_scale = (parent.scale * scale_factor).clamp(settings.scale_min, settings.scale_max);
    // Y axis: mutated independently when non-uniform scaling is on (lets the
    // shape stretch/squash); otherwise it follows the X scale (uniform).
    let new_scale_y = if settings.evolve_non_uniform_scale {
        let scale_factor_y = rng.gen_range(0.7_f32..=1.3);
        (parent.scale_y * scale_factor_y).clamp(settings.scale_min, settings.scale_max)
    } else {
        new_scale
    };

    // Alpha: ±0.2 (or fixed at 1.0 if opacity evolution is disabled)
    let new_alpha = if settings.evolve_opacity {
        let da = rng.gen_range(-0.2_f32..=0.2);
        (parent.alpha + da).clamp(0.1, 1.0)
    } else {
        1.0
    };

    // Color: re-sample from target at new position (ignored at render time when
    // the shape uses its original colors).
    let (r, g, b) = generator.sample_color_at(new_x as u32, new_y as u32);

    // Hue/saturation: only nudged in real-color mode with the matching toggle on;
    // otherwise the parent's neutral value is kept verbatim.
    let new_hue_shift = if settings.evolve_hue {
        let dh = rng.gen_range(-0.05_f32..=0.05);
        (parent.hue_shift + dh).rem_euclid(1.0)
    } else {
        parent.hue_shift
    };
    let new_saturation_scale = if settings.evolve_saturation {
        let sf = rng.gen_range(0.9_f32..=1.1);
        (parent.saturation_scale * sf).clamp(0.0, 2.0)
    } else {
        parent.saturation_scale
    };

    let new_brightness_scale = if settings.evolve_brightness {
        let bf = rng.gen_range(0.9_f32..=1.1);
        (parent.brightness_scale * bf).clamp(0.2, 2.0)
    } else {
        parent.brightness_scale
    };

    CandidateParams {
        shape_index: parent.shape_index,
        x: new_x,
        y: new_y,
        rotation: new_rotation,
        scale: new_scale,
        r,
        g,
        b,
        alpha: new_alpha,
        scale_y: new_scale_y,
        use_original_color: parent.use_original_color,
        hue_shift: new_hue_shift,
        saturation_scale: new_saturation_scale,
        brightness_scale: new_brightness_scale,
        _padding: [0.0; 2],
    }
}

/// Select the top `n` candidates by fitness (lowest score = best).
fn top_indices_by_score(scores: &[f32], n: usize) -> Vec<usize> {
    let mut indexed: Vec<(usize, f32)> = scores.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    indexed.iter().take(n).map(|(i, _)| *i).collect()
}

/// Run ONE full evolutionary search against the current canvas and return the
/// best candidate found plus its fitness (delta error; negative = improves the
/// canvas). This is the same cycle a hill-climbing `step` uses — generate a
/// `batch_size` random population, then repeat `num_generations` times: keep the
/// top `survival_rate` fraction and breed `children_per_parent` mutated children
/// from each.
///
/// Unlike `HillClimber::step` it does NOT composite the winner, does NOT apply
/// the `min_improvement` acceptance gate, and ignores diversity penalties — the
/// caller decides whether to place the result. Used by video rebirth so a shape
/// born to replace a dead one evolves exactly like an originally-placed shape.
///
/// `placed_shapes` only feeds the generator's adaptive scale (pass 0 for the
/// full `scale_min..scale_max` range, e.g. when filling fresh gaps in video).
pub fn evolve_best_candidate(
    gpu: &GpuContext,
    generator: &mut CandidateGenerator,
    settings: &Settings,
    placed_shapes: u32,
    rng: &mut SmallRng,
) -> Option<(CandidateParams, f32)> {
    // Initial random population.
    let mut population = generator.generate_batch(
        settings.batch_size,
        placed_shapes,
        gpu.canvas_size,
        gpu.num_shapes,
    );
    if population.is_empty() {
        return None;
    }

    // Assign colors based on the target at each candidate's position.
    for candidate in population.iter_mut() {
        let (r, g, b) = generator.sample_color_at(candidate.x as u32, candidate.y as u32);
        candidate.r = r;
        candidate.g = g;
        candidate.b = b;
    }

    let mut population = CachedPopulation::new(population);
    let mut best_candidate: Option<CandidateParams> = None;
    let mut best_score = f32::MAX;
    gpu.control.begin_search(settings.num_generations);

    for _gen in 0..settings.num_generations {
        if !gpu.control.checkpoint() {
            return None;
        }
        gpu.control
            .generations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let scores = population.evaluate_with(|fresh| gpu.evaluate_candidates(fresh))?;
        if !gpu.control.checkpoint() {
            return None;
        }

        for (i, &score) in scores.iter().enumerate() {
            if score < best_score {
                best_score = score;
                best_candidate = Some(population.candidates[i]);
            }
        }

        gpu.control.searched(best_score);
        let num_survivors =
            ((population.candidates.len() as f32 * settings.survival_rate) as usize).max(1);
        let survivors = top_indices_by_score(&scores, num_survivors);
        population = population.breed(&survivors, gpu.canvas_size, generator, settings, rng);
    }

    // Final evaluation of the last generation.
    let scores = population.evaluate_with(|fresh| gpu.evaluate_candidates(fresh))?;
    if !gpu.control.checkpoint() {
        return None;
    }
    for (i, &score) in scores.iter().enumerate() {
        if score < best_score {
            best_score = score;
            best_candidate = Some(population.candidates[i]);
        }
    }

    gpu.control.searched(best_score);
    best_candidate.map(|c| (c, best_score))
}

/// Select the candidate with the lowest fitness score.
pub fn select_best(scores: &[f32]) -> Option<(usize, f32)> {
    if scores.is_empty() {
        return None;
    }

    let mut best_idx = 0;
    let mut best_score = scores[0];

    for (i, &score) in scores.iter().enumerate().skip(1) {
        if score < best_score {
            best_score = score;
            best_idx = i;
        }
    }

    Some((best_idx, best_score))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_lock_applies_to_evolution_children() {
        let settings = Settings {
            evolve_rotation: false,
            ..Settings::default()
        };
        let mut generator =
            CandidateGenerator::new(settings.clone(), vec![180; 64 * 64 * 4], (64, 64));
        let mut parent = generator.generate_batch(1, 0, (64, 64), 1)[0];
        parent.rotation = 1.5;
        let mut rng = SmallRng::seed_from_u64(42);
        for _ in 0..300 {
            parent = mutate_candidate(&parent, (64, 64), &generator, &settings, &mut rng);
            assert_eq!(parent.rotation, 0.0);
        }
    }

    #[test]
    fn cached_survivors_match_full_evaluation_without_changing_selection() {
        for children in [0, 9] {
            let settings = Settings {
                batch_size: 100,
                survival_rate: 0.1,
                children_per_parent: children,
                num_generations: 4,
                ..Settings::default()
            };
            let mut generator =
                CandidateGenerator::new(settings.clone(), vec![180; 64 * 64 * 4], (64, 64));
            let mut reference = generator.generate_batch(100, 0, (64, 64), 3);
            let mut cached = CachedPopulation::new(reference.clone());
            let mut reference_rng = SmallRng::seed_from_u64(42);
            let mut cached_rng = reference_rng.clone();
            let score =
                |p: &CandidateParams| (p.x - 20.0).powi(2) + (p.y - 30.0).powi(2) + p.rotation;
            let mut full_count = 0;
            let mut cached_count = 0;
            for pass in 0..=settings.num_generations {
                let raw: Vec<_> = reference.iter().map(score).collect();
                full_count += raw.len();
                let scores = cached
                    .evaluate_with(|fresh| {
                        cached_count += fresh.len();
                        fresh.iter().map(score).collect()
                    })
                    .unwrap();
                assert_eq!(scores, raw);
                assert_eq!(
                    bytemuck::cast_slice::<CandidateParams, u8>(&cached.candidates),
                    bytemuck::cast_slice::<CandidateParams, u8>(&reference)
                );
                // Select using diversity penalties, but cache only raw scores.
                let ranked: Vec<_> = raw
                    .iter()
                    .zip(&reference)
                    .map(|(s, p)| s + p.shape_index as f32 * 100.0)
                    .collect();
                let survivors = top_indices_by_score(
                    &ranked,
                    ((reference.len() as f32 * settings.survival_rate) as usize).max(1),
                );
                if pass < settings.num_generations {
                    let mut next = Vec::new();
                    for &index in &survivors {
                        let parent = reference[index];
                        next.push(parent);
                        for _ in 0..children {
                            next.push(mutate_candidate(
                                &parent,
                                (64, 64),
                                &generator,
                                &settings,
                                &mut reference_rng,
                            ));
                        }
                    }
                    reference = next;
                    cached =
                        cached.breed(&survivors, (64, 64), &generator, &settings, &mut cached_rng);
                }
            }
            assert!(cached_count < full_count);
            if children == 9 {
                assert_eq!(cached_count, 460);
                assert_eq!(full_count, 500);
            } else {
                assert_eq!(cached_count, 100);
            }
        }
        let settings = Settings::default();
        let mut generator = CandidateGenerator::new(settings, vec![255; 4], (1, 1));
        let mut cancelled = CachedPopulation::new(generator.generate_batch(2, 0, (1, 1), 1));
        assert!(cancelled.evaluate_with(|_| Vec::new()).is_none());
        assert!(cancelled.scores.iter().all(Option::is_none));
    }

    #[test]
    #[ignore = "requires GPU; verifies actual evaluation count and cache invalidation between placements"]
    fn placement_and_video_rebirth_only_evaluate_new_children() {
        use std::sync::atomic::Ordering;
        let settings = Settings {
            batch_size: 100,
            shape_resolution: 16,
            max_texture_size: 32,
            num_generations: 10,
            survival_rate: 0.1,
            children_per_parent: 9,
            use_min_improvement: false,
            ..Settings::default()
        };
        let gpu = GpuContext::new(
            &vec![255; 32 * 32 * 4],
            (32, 32),
            &[crate::types::ShapeLayer {
                pixels: vec![255; 16 * 16 * 4],
            }],
            &settings,
        )
        .unwrap();
        let mut generator =
            CandidateGenerator::new(settings.clone(), vec![255; 32 * 32 * 4], (32, 32));
        let mut climber = HillClimber::new();
        for placement in 1..=2 {
            assert!(matches!(
                climber.step(&gpu, &mut generator, &settings),
                StepResult::Accepted(_)
            ));
            assert_eq!(
                gpu.control.evaluated.load(Ordering::Relaxed),
                placement * 1000
            );
            assert_eq!(gpu.control.search_generation.load(Ordering::Relaxed), 11);
        }
        assert!(evolve_best_candidate(
            &gpu,
            &mut generator,
            &settings,
            0,
            &mut SmallRng::seed_from_u64(1)
        )
        .is_some());
        assert_eq!(gpu.control.evaluated.load(Ordering::Relaxed), 3000);
    }

    #[test]
    fn test_select_best_empty_slice() {
        let scores: &[f32] = &[];
        assert_eq!(select_best(scores), None);
    }

    #[test]
    fn test_select_best_single_element() {
        let scores = &[0.5];
        assert_eq!(select_best(scores), Some((0, 0.5)));
    }

    #[test]
    fn test_select_best_distinct_values() {
        let scores = &[0.8, 0.3, 0.6, 0.1, 0.9];
        assert_eq!(select_best(scores), Some((3, 0.1)));
    }

    #[test]
    fn test_select_best_tie_breaking_lowest_index() {
        let scores = &[0.5, 0.2, 0.7, 0.2, 0.2];
        assert_eq!(select_best(scores), Some((1, 0.2)));
    }

    #[test]
    fn test_select_best_negative_values() {
        let scores = &[0.5, -0.1, 0.3, -0.5, 0.2];
        assert_eq!(select_best(scores), Some((3, -0.5)));
    }
}
