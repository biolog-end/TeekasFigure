// Video temporal coherence pipeline.
//
// Core idea: shapes are a living ecosystem that adapts to changing frames.
// - Shapes inherit from the previous frame (no fresh random restart per frame)
// - Shapes are ALWAYS processed in chronological (placement) order, bottom to
//   top. The first shape ever placed (often a huge one that paints the
//   background) sits at the bottom and is composited first, so every later
//   shape is evaluated against the same underlying context it had when placed.
//   Reordering would make lower shapes see a wrong background and die en masse.
// - Each shape mutates locally (small position/rotation/scale changes) and may
//   recolor in place to match the new frame's colors. Movement is penalised.
// - A shape DIES only when, even at its best low-movement position, it would
//   make its own region noticeably worse per pixel — i.e. the target pixels
//   under it changed drastically (a real scene change). Redundant shapes
//   (delta ≈ 0) and contributing shapes (delta < 0) always survive, which
//   preserves density and coherence. (The previous implementation killed every
//   shape that merely failed to *improve* the stack, which destroyed all the
//   overlapping detail every frame — the bug behind the "image falls apart".)
//
// Linear interpolation between keyframes is implemented in this module too,
// so callers can request N intermediate frames generated purely from
// shape parameter lerp — no evolution, no GPU eval, just composition.

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::gpu::GpuContext;
use crate::settings::Settings;
use crate::types::CandidateParams;
use super::CandidateGenerator;
use super::hill_climber::evolve_best_candidate;

/// A shape living on the canvas with memory of its previous keyframe state
/// and a stable id used to match shapes across keyframes for interpolation.
#[derive(Clone, Copy, Debug)]
pub struct PlacedShapeRecord {
    /// Stable id assigned when the shape was first placed.
    pub id: u64,
    /// Current parameters of this shape (latest keyframe).
    pub params: CandidateParams,
    /// Parameters from the PREVIOUS keyframe (for movement penalty and interpolation).
    pub prev_params: CandidateParams,
    /// `true` if this shape was created on the current keyframe — used to fade-in
    /// during interpolation instead of popping.
    pub just_born: bool,
}

/// Snapshot of a shape that died on the current keyframe — kept for one
/// transition so it can fade out during interpolation.
#[derive(Clone, Copy, Debug)]
struct DyingShape {
    /// Parameters at the previous keyframe (where it last lived).
    params: CandidateParams,
}

/// Video pipeline: manages the ecosystem of shapes across frames.
pub struct VideoPipeline {
    /// Ordered list of all alive shapes on the canvas (bottom to top).
    pub shapes: Vec<PlacedShapeRecord>,
    /// Shapes that died on the latest keyframe transition. Used only by
    /// `render_interpolated_frame` to fade them out smoothly. Cleared
    /// after the next keyframe transition is processed.
    dying: Vec<DyingShape>,
    /// Current keyframe index (0 for first frame).
    pub frame_index: u32,
    /// Counter for stable shape ids.
    next_id: u64,
    /// RNG for mutations.
    rng: SmallRng,
}

impl VideoPipeline {
    pub fn new() -> Self {
        Self {
            shapes: Vec::new(),
            dying: Vec::new(),
            frame_index: 0,
            next_id: 0,
            rng: SmallRng::from_entropy(),
        }
    }

    /// Record a shape placed by the hill climber during keyframe generation.
    pub fn record_placed_shape(&mut self, params: CandidateParams) {
        let id = self.next_id;
        self.next_id += 1;
        self.shapes.push(PlacedShapeRecord {
            id,
            params,
            prev_params: params, // First keyframe: prev = current
            just_born: self.frame_index > 0, // newborn on subsequent keyframes → fade in
        });
    }

    /// Adapt all shapes to a new target frame.
    ///
    /// Shapes are processed in CHRONOLOGICAL (placement) order, bottom to top.
    /// The canvas is cleared once, then for each shape, in order:
    ///   1. The canvas currently holds every shape BELOW this one (already
    ///      adapted this frame), exactly the context this shape had when first
    ///      placed. We never reorder, so an early huge "background" shape is
    ///      always laid down first and the shapes above it keep their footing.
    ///   2. We try the unchanged shape and `mutations_per_shape` small local
    ///      mutations (tiny position/rotation/scale shift). By default the
    ///      shape's COLOR is preserved — only geometry adapts. If the
    ///      `video_recolor` setting is on, we also add a recolor-in-place
    ///      candidate and let mutations resample the new frame's colors (old
    ///      behaviour).
    ///   3. Movement is chosen by minimising (raw GPU delta + movement penalty),
    ///      with the penalty measured from the shape's CURRENT position this
    ///      keyframe, so the shape drifts to its best nearby fit without jumping
    ///      and standing still costs nothing.
    ///   4. LIFE/DEATH is decided on the chosen candidate's per-pixel delta
    ///      (raw delta / footprint area). Improving (<0) and redundant (≈0)
    ///      shapes always live; a shape dies only when even its best low-
    ///      movement spot worsens its own region beyond `scene_change_tolerance`
    ///      (the target under it changed drastically — a real scene change).
    ///   5. On death the shape is REBORN: a brand-new shape is created through
    ///      the FULL evolutionary cycle (batch_size random population →
    ///      num_generations × keep top survival_rate + breed children_per_parent),
    ///      identical to first-frame placement. It is injected to keep
    ///      population/density constant, fading in over interpolation. If even
    ///      the best evolved shape can't meet the tolerance, the gap is left.
    ///   6. Living/reborn shapes are composited immediately so the next (higher)
    ///      shape is evaluated against the up-to-date canvas.
    ///
    /// Returns the number of shapes that died without an immediate replacement.
    pub fn adapt_to_new_frame(
        &mut self,
        gpu: &GpuContext,
        generator: &mut CandidateGenerator,
        settings: &Settings,
    ) -> u32 {
        self.frame_index += 1;
        // The previous keyframe's "dying" set is no longer needed — those
        // shapes already faded out during the last interpolation segment.
        self.dying.clear();
        // Newborn flags from the previous keyframe are now stale.
        for s in self.shapes.iter_mut() {
            s.just_born = false;
        }

        let mutations_per_shape = settings.mutations_per_shape;

        // Clear the canvas once. We rebuild it in chronological order as we
        // decide each shape's fate, so each shape's GPU delta is measured
        // against the correct underlying layers (those placed before it).
        gpu.clear_canvas();

        // IMPORTANT: iterate in placement (chronological) order. `old_shapes`
        // preserves that order; we never sort. Survivors keep their relative
        // order so the stacking (and thus the background built by early big
        // shapes) stays intact across frames.
        let old_shapes = std::mem::take(&mut self.shapes);
        let mut survivors: Vec<PlacedShapeRecord> = Vec::with_capacity(old_shapes.len());
        let mut dead_count: u32 = 0;

        for record in old_shapes.into_iter() {
            let current = record.params;

            // Build the adaptation candidate set:
            //   [0] (only when `video_recolor` is on) recolor-in-place: same
            //       transform, color resampled from the NEW frame, zero movement
            //       penalty — lets a shape refresh its colour for free.
            //   the unchanged shape (zero movement penalty).
            //   N small local mutations (tiny move/rotate/scale; color is kept
            //       unless `video_recolor` is on).
            //
            // By default a shape KEEPS its original colour for the whole video
            // and only its geometry adapts — auto recolouring washed detail out.
            let mut candidates: Vec<CandidateParams> =
                Vec::with_capacity(mutations_per_shape as usize + 2);
            if settings.video_recolor {
                candidates.push(recolor_in_place(&current, generator));
            }
            candidates.push(current);
            for _ in 0..mutations_per_shape {
                candidates.push(self.create_local_mutation(
                    &current,
                    gpu.canvas_size,
                    generator,
                    settings,
                ));
            }

            // Evaluate all candidates on the GPU against the current canvas
            // (which holds all shapes below this one). `scores[i]` is the delta
            // error of adding candidate i: negative = improves, ~0 = redundant,
            // positive = worsens — all measured in the correct chronological
            // context.
            gpu.dispatch_mse_evaluation(&candidates);
            let scores = gpu.read_fitness_scores();

            // Choose MOVEMENT by (delta + movement penalty). The penalty is
            // measured from `current` — the shape's position THIS keyframe — so
            // staying put (and recolour-in-place) costs nothing and only a real
            // payoff makes a shape drift. (Using the older `prev_params` here
            // was an off-by-one bug: it charged a shape every frame for motion
            // it had already made, nudging stationary shapes toward death.)
            let mut best_idx = 0usize;
            let mut best_penalized = f32::MAX;
            for (i, &score) in scores.iter().enumerate().take(candidates.len()) {
                let penalty = self.movement_penalty(&current, &candidates[i], settings);
                let penalized = score + penalty;
                if penalized < best_penalized {
                    best_penalized = penalized;
                    best_idx = i;
                }
            }

            let chosen = candidates[best_idx];
            let chosen_raw = scores.get(best_idx).copied().unwrap_or(f32::INFINITY);

            // LIFE/DEATH — density-preserving and spec-faithful.
            //
            // Normalise the chosen candidate's raw delta by its footprint area
            // to get a mean per-pixel delta comparable across huge background
            // shapes and tiny detail shapes:
            //
            //   mean_delta < 0   → shape still improves the frame  → LIVE
            //   mean_delta ≈ 0   → redundant but harmless          → LIVE
            //   mean_delta > tol → even its best spot now worsens  → DIE → REBIRTH
            //
            // The last case is a real scene change: the target pixels under the
            // shape changed so much that not even a recolour or a small move can
            // make it fit. Per the algorithm, we do NOT just delete it (that is
            // what slowly thinned the canvas out and washed every later frame to
            // the background). Instead the shape dies AND a fresh random shape is
            // born in its place to fill the gap, keeping population/density
            // constant. The newcomer then adapts over the following frames.
            let area = footprint_area(&chosen, gpu.canvas_size, gpu.shape_resolution).max(1);
            let mean_delta = chosen_raw / area as f32;
            let scene_changed = mean_delta > settings.scene_change_tolerance;

            if !scene_changed {
                // Shape lives — composite immediately so the next (higher)
                // shape is evaluated against the up-to-date canvas.
                gpu.composite_shape(&chosen);

                survivors.push(PlacedShapeRecord {
                    id: record.id,
                    params: chosen,
                    prev_params: current, // remember where we were last keyframe
                    just_born: false,
                });
            } else {
                // Scene changed under this shape — it can no longer match the
                // target here. It fades out during interpolation and a freshly
                // EVOLVED shape is born to replace it in pass 2 below (so we
                // keep evaluating remaining shapes against a stable canvas
                // first, then refill all gaps against the finished survivor
                // canvas).
                self.dying.push(DyingShape { params: current });
                dead_count += 1;
            }
        }

        self.shapes = survivors;

        // REBIRTH (pass 2) — replace every dead shape with a brand-new shape
        // that goes through the FULL evolutionary cycle, exactly like a shape
        // placed on the first frame: a `batch_size` random population, then
        // `num_generations` rounds of (keep top `survival_rate` → breed
        // `children_per_parent` mutated children). The evolution naturally
        // gravitates to the highest-error regions — i.e. the gaps the dead
        // shapes left — so density/detail is rebuilt instead of decaying.
        //
        // `placed_shapes = 0` lets newcomers use the full scale range so they
        // can fill big gaps, not just `scale_min`-sized specks. Each winner is
        // composited immediately so the next rebirth sees an up-to-date canvas
        // and targets the next-worst gap.
        for _ in 0..dead_count {
            let Some((reborn, score)) =
                evolve_best_candidate(gpu, generator, settings, 0, &mut self.rng)
            else {
                break;
            };

            // Accept the newcomer on the same per-pixel basis as the life test:
            // it must bring its region within `scene_change_tolerance`. (With a
            // negative tolerance this means it must strictly improve.) If even
            // the best evolved shape can't clear the bar, the canvas is already
            // good everywhere and further attempts won't help — stop early.
            let area = footprint_area(&reborn, gpu.canvas_size, gpu.shape_resolution).max(1);
            let mean_delta = score / area as f32;
            if mean_delta <= settings.scene_change_tolerance {
                gpu.composite_shape(&reborn);
                let id = self.next_id;
                self.next_id += 1;
                self.shapes.push(PlacedShapeRecord {
                    id,
                    params: reborn,
                    prev_params: reborn, // newborn: no motion, fade-in only
                    just_born: true,
                });
            } else {
                break;
            }
        }

        dead_count
    }

    /// Grow the population toward `max_shapes` by evolving brand-new shapes,
    /// exactly the way the hill climber fills the canvas on the first frame.
    ///
    /// Without this, a frame that started nearly empty (e.g. a black opening
    /// frame where the climber placed a single shape and then converged) would
    /// keep that tiny population *forever*: `adapt_to_new_frame` only nudges the
    /// existing shapes and replaces the ones that die, it never adds NET-new
    /// detail. So as soon as real content appears later in the clip there are
    /// no shapes available to represent it. This pass closes that gap.
    ///
    /// Each newcomer goes through the FULL evolutionary cycle
    /// (`evolve_best_candidate`) and is accepted on the SAME criterion the hill
    /// climber uses:
    ///   * if `use_min_improvement` is on, only a shape whose best score beats
    ///     `min_improvement` (i.e. it improves the frame enough) is kept;
    ///   * otherwise every evolved shape is added until the population reaches
    ///     `max_shapes`.
    /// A run of `max_rejections` consecutive non-improving candidates stops the
    /// growth early for this frame, mirroring the climber's convergence test —
    /// so an still-mostly-empty frame adds little or nothing, and a busy frame
    /// fills right up to `max_shapes`.
    ///
    /// New shapes are flagged `just_born` so they fade in over interpolation,
    /// and each is composited immediately so the next evolution targets the
    /// next-worst remaining gap. Returns the number of shapes added.
    pub fn grow_population(
        &mut self,
        gpu: &GpuContext,
        generator: &mut CandidateGenerator,
        settings: &Settings,
    ) -> u32 {
        let mut added: u32 = 0;
        let mut consecutive_rejections: u32 = 0;

        while (self.shapes.len() as u32) < settings.max_shapes {
            // Converged for this frame: too many candidates in a row failed to
            // improve it enough, so further attempts won't help.
            if consecutive_rejections >= settings.max_rejections {
                break;
            }

            // Full evolution against the up-to-date canvas. `placed_shapes` is
            // the live population so the adaptive scale shrinks as the frame
            // fills, just like on the first keyframe.
            let placed = self.shapes.len() as u32;
            let Some((candidate, score)) =
                evolve_best_candidate(gpu, generator, settings, placed, &mut self.rng)
            else {
                break;
            };

            // Accept on the climber's criterion: when min-improvement gating is
            // on, the candidate must actually improve the frame; otherwise we
            // keep filling toward max_shapes unconditionally.
            let accept = !settings.use_min_improvement || score < settings.min_improvement;
            if accept {
                gpu.composite_shape(&candidate);
                let id = self.next_id;
                self.next_id += 1;
                self.shapes.push(PlacedShapeRecord {
                    id,
                    params: candidate,
                    prev_params: candidate, // newborn: no motion, fade-in only
                    just_born: true,
                });
                added += 1;
                consecutive_rejections = 0;
            } else {
                consecutive_rejections += 1;
            }
        }

        added
    }

    /// Rebuild the canvas from scratch using the current ordered shape list.
    /// Useful before the hill climber starts filling vacancies, so that it
    /// evaluates new candidates against the up-to-date canvas state.
    pub fn rebuild_canvas(&self, gpu: &GpuContext) {
        gpu.clear_canvas();
        for record in &self.shapes {
            gpu.composite_shape(&record.params);
        }
    }

    /// Render an intermediate frame by linearly interpolating each shape's
    /// parameters between the previous keyframe (`prev_params`) and the
    /// current keyframe (`params`). Newborn shapes fade in (alpha 0→full),
    /// dying shapes fade out (alpha full→0). No GPU evaluation, no evolution.
    ///
    /// `t` ∈ (0, 1): 0.0 ≈ previous keyframe, 1.0 ≈ current keyframe.
    pub fn render_interpolated_frame(&self, gpu: &GpuContext, t: f32) {
        let t = t.clamp(0.0, 1.0);
        gpu.clear_canvas();

        // Dying shapes belong on the bottom of the stack (where they were
        // before they died). Order among themselves doesn't matter much,
        // but rendering them first preserves the visual layering.
        for d in &self.dying {
            let mut p = d.params;
            // Fade out from full opacity to 0 over the transition.
            p.alpha = (p.alpha * (1.0 - t)).max(0.0);
            if p.alpha > 0.001 {
                gpu.composite_shape(&p);
            }
        }

        for s in &self.shapes {
            let mut p = lerp_params(&s.prev_params, &s.params, t);
            if s.just_born {
                // Fade-in newborn: alpha grows from 0 to interpolated value.
                p.alpha = (p.alpha * t).max(0.0);
                if p.alpha < 0.001 {
                    continue;
                }
            }
            gpu.composite_shape(&p);
        }
    }

    /// Create a small local mutation of a shape.
    /// Changes are intentionally small to maintain temporal coherence.
    fn create_local_mutation(
        &mut self,
        parent: &CandidateParams,
        canvas_size: (u32, u32),
        generator: &mut CandidateGenerator,
        settings: &Settings,
    ) -> CandidateParams {
        let (cw, ch) = canvas_size;

        // Position: very small shift (±3% of canvas — keeps shapes local)
        let dx = self.rng.gen_range(-(cw as f32 * 0.03)..=(cw as f32 * 0.03));
        let dy = self.rng.gen_range(-(ch as f32 * 0.03)..=(ch as f32 * 0.03));
        let new_x = (parent.x + dx).clamp(0.0, (cw - 1) as f32);
        let new_y = (parent.y + dy).clamp(0.0, (ch - 1) as f32);

        // Rotation: tiny change (±0.15 radians ≈ ±8.5 degrees)
        let dr = self.rng.gen_range(-0.15_f32..=0.15);
        let new_rotation = (parent.rotation + dr).rem_euclid(std::f32::consts::TAU);

        // Scale: very small change (±10%) on the X axis.
        let scale_factor = self.rng.gen_range(0.9_f32..=1.1);
        let new_scale = (parent.scale * scale_factor).clamp(settings.scale_min, settings.scale_max);
        // Y axis: mutated independently when non-uniform scaling is enabled so a
        // shape can gently stretch/squash across frames; otherwise it tracks X.
        let new_scale_y = if settings.evolve_non_uniform_scale {
            let scale_factor_y = self.rng.gen_range(0.9_f32..=1.1);
            (parent.scale_y * scale_factor_y).clamp(settings.scale_min, settings.scale_max)
        } else {
            new_scale
        };

        // Alpha: tiny change (±0.05) or fixed if evolve_opacity is off
        let new_alpha = if settings.evolve_opacity {
            let da = self.rng.gen_range(-0.05_f32..=0.05);
            (parent.alpha + da).clamp(0.1, 1.0)
        } else {
            1.0
        };

        // Color: by default a shape KEEPS its original color across the whole
        // video — only its geometry (position/rotation/scale) adapts. Auto
        // recoloring to the new frame is opt-in via `video_recolor` (the old
        // behaviour, which caused detail to wash out to flat colours).
        let (r, g, b) = if settings.video_recolor {
            generator.sample_color_at(new_x as u32, new_y as u32)
        } else {
            (parent.r, parent.g, parent.b)
        };

        // Hue/saturation: tiny nudges in real-color mode with the matching
        // toggle on, so the palette can drift to track the new frame; otherwise
        // the parent's neutral value is preserved.
        let new_hue_shift = if settings.evolve_hue {
            let dh = self.rng.gen_range(-0.02_f32..=0.02);
            (parent.hue_shift + dh).rem_euclid(1.0)
        } else {
            parent.hue_shift
        };
        let new_saturation_scale = if settings.evolve_saturation {
            let sf = self.rng.gen_range(0.95_f32..=1.05);
            (parent.saturation_scale * sf).clamp(0.0, 2.0)
        } else {
            parent.saturation_scale
        };

        let new_brightness_scale = if settings.evolve_brightness {
            let bf = self.rng.gen_range(0.95_f32..=1.05);
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
            r, g, b,
            alpha: new_alpha,
            scale_y: new_scale_y,
            use_original_color: parent.use_original_color,
            hue_shift: new_hue_shift,
            saturation_scale: new_saturation_scale,
            brightness_scale: new_brightness_scale,
            _padding: [0.0; 2],
        }
    }

    /// Calculate movement penalty based on how far a shape moved from its previous position.
    /// Uses both position distance AND scale/rotation change.
    /// Higher penalty = shape moved too much = discouraged.
    fn movement_penalty(
        &self,
        prev: &CandidateParams,
        current: &CandidateParams,
        settings: &Settings,
    ) -> f32 {
        // Position distance (in pixels).
        let dx = current.x - prev.x;
        let dy = current.y - prev.y;
        let pos_dist = (dx * dx + dy * dy).sqrt();

        // Scale change (ratio).
        let scale_ratio = if prev.scale > 0.001 {
            (current.scale / prev.scale - 1.0).abs()
        } else {
            0.0
        };

        // Rotation change (in radians, wrapped to [0, π]).
        let rot_diff = (current.rotation - prev.rotation).abs();
        let rot_diff = rot_diff.min(std::f32::consts::TAU - rot_diff);

        // Combined penalty: position is most important, scale and rotation less so.
        let penalty = pos_dist + scale_ratio * 50.0 + rot_diff * 10.0;

        settings.displacement_weight * penalty
    }
}

/// Produce a copy of a shape with identical geometry but its colour resampled
/// from the (new) target frame at its current centre. This lets a stationary
/// shape refresh its colour for free (no movement penalty) as the video's
/// content shifts, which is what prevents slow colour drift / wash-out in
/// regions that barely move between frames.
pub fn recolor_in_place(
    shape: &CandidateParams,
    generator: &CandidateGenerator,
) -> CandidateParams {
    let (r, g, b) = generator.sample_color_at(shape.x as u32, shape.y as u32);
    CandidateParams {
        r,
        g,
        b,
        ..*shape
    }
}

/// Estimate the on-canvas footprint of a shape in pixels: the area of the
/// shape's bounding square (scale × shape_resolution)², clipped to the canvas.
///
/// This matches the region the GPU MSE evaluator iterates over (a conservative
/// box around the rotated shape), so dividing the raw delta by this area yields
/// a mean per-pixel delta that is comparable across shapes of any size. The
/// rotation-AABB inflation cancels out when we use the same measure for the
/// life/death threshold, so a simple side² estimate is sufficient and stable.
pub fn footprint_area(
    shape: &CandidateParams,
    canvas_size: (u32, u32),
    shape_resolution: u32,
) -> u32 {
    // Use both axes so stretched/squashed shapes (non-uniform scaling) report a
    // realistic footprint; for uniform shapes scale_y == scale so this reduces
    // to the original side² estimate.
    let side_x = (shape.scale * shape_resolution as f32).abs();
    let side_y = (shape.scale_y * shape_resolution as f32).abs();
    let area = (side_x * side_y).round();
    let canvas_area = (canvas_size.0 as f32) * (canvas_size.1 as f32);
    // Clamp into [1, canvas_area] and convert safely to u32.
    area.clamp(1.0, canvas_area.max(1.0)) as u32
}

/// Linearly interpolate every numeric field of two candidate params.
/// `shape_index` is taken from `b` (the current keyframe) since shapes keep
/// their texture across frames anyway. `_padding` stays zero.
pub fn lerp_params(a: &CandidateParams, b: &CandidateParams, t: f32) -> CandidateParams {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;

    // Rotation needs special care so we take the shortest arc instead of
    // spinning the long way round when `b.rotation` wraps past 2π.
    let mut da = b.rotation - a.rotation;
    let tau = std::f32::consts::TAU;
    if da > std::f32::consts::PI {
        da -= tau;
    } else if da < -std::f32::consts::PI {
        da += tau;
    }
    let rot = (a.rotation + da * t).rem_euclid(tau);

    CandidateParams {
        shape_index: b.shape_index,
        x: a.x * inv + b.x * t,
        y: a.y * inv + b.y * t,
        rotation: rot,
        scale: a.scale * inv + b.scale * t,
        r: a.r * inv + b.r * t,
        g: a.g * inv + b.g * t,
        b: a.b * inv + b.b * t,
        alpha: a.alpha * inv + b.alpha * t,
        scale_y: a.scale_y * inv + b.scale_y * t,
        use_original_color: b.use_original_color,
        hue_shift: a.hue_shift * inv + b.hue_shift * t,
        saturation_scale: a.saturation_scale * inv + b.saturation_scale * t,
        brightness_scale: a.brightness_scale * inv + b.brightness_scale * t,
        _padding: [0.0; 2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32, rot: f32, scale: f32, alpha: f32) -> CandidateParams {
        CandidateParams {
            shape_index: 0,
            x, y, rotation: rot, scale,
            r: 0.0, g: 0.0, b: 0.0,
            alpha,
            scale_y: scale,
            use_original_color: 0.0,
            hue_shift: 0.0,
            saturation_scale: 1.0,
            brightness_scale: 1.0,
            _padding: [0.0; 2],
        }
    }

    #[test]
    fn lerp_endpoints() {
        let a = p(0.0, 0.0, 0.0, 1.0, 1.0);
        let b = p(10.0, 20.0, 1.0, 2.0, 0.5);
        let r0 = lerp_params(&a, &b, 0.0);
        assert!((r0.x - a.x).abs() < 1e-5);
        assert!((r0.y - a.y).abs() < 1e-5);
        let r1 = lerp_params(&a, &b, 1.0);
        assert!((r1.x - b.x).abs() < 1e-5);
        assert!((r1.scale - b.scale).abs() < 1e-5);
    }

    #[test]
    fn lerp_midpoint_position() {
        let a = p(0.0, 0.0, 0.0, 1.0, 1.0);
        let b = p(10.0, 20.0, 0.0, 1.0, 1.0);
        let r = lerp_params(&a, &b, 0.5);
        assert!((r.x - 5.0).abs() < 1e-5);
        assert!((r.y - 10.0).abs() < 1e-5);
    }

    #[test]
    fn lerp_rotation_short_arc() {
        // From 0.1 rad to (TAU - 0.1) rad — short arc goes the "negative"
        // direction (~0.2 rad), not ~6.08 rad.
        let a = p(0.0, 0.0, 0.1, 1.0, 1.0);
        let b_rot = std::f32::consts::TAU - 0.1;
        let b = p(0.0, 0.0, b_rot, 1.0, 1.0);
        let r = lerp_params(&a, &b, 0.5);
        // Expected midpoint along short arc ≈ 0.0 (or TAU, both equivalent
        // mod TAU). Distance from 0.0 along the circle should be tiny.
        let tau = std::f32::consts::TAU;
        let dist = r.rotation.min(tau - r.rotation);
        assert!(dist < 0.05, "midpoint rotation took the long way: {}", r.rotation);
    }

    #[test]
    fn lerp_clamps_t() {
        let a = p(0.0, 0.0, 0.0, 1.0, 1.0);
        let b = p(10.0, 0.0, 0.0, 1.0, 1.0);
        let r_neg = lerp_params(&a, &b, -1.0);
        let r_big = lerp_params(&a, &b, 5.0);
        assert!((r_neg.x - a.x).abs() < 1e-5);
        assert!((r_big.x - b.x).abs() < 1e-5);
    }

    #[test]
    fn record_assigns_unique_ids() {
        let mut vp = VideoPipeline::new();
        vp.record_placed_shape(p(0.0, 0.0, 0.0, 1.0, 1.0));
        vp.record_placed_shape(p(1.0, 0.0, 0.0, 1.0, 1.0));
        vp.record_placed_shape(p(2.0, 0.0, 0.0, 1.0, 1.0));
        let ids: Vec<u64> = vp.shapes.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn footprint_area_scales_with_size() {
        // scale 1.0 × resolution 128 → side 128 → area 16384.
        let big = p(50.0, 50.0, 0.0, 1.0, 1.0);
        let small = p(50.0, 50.0, 0.0, 0.1, 1.0);
        let a_big = footprint_area(&big, (1000, 1000), 128);
        let a_small = footprint_area(&small, (1000, 1000), 128);
        assert_eq!(a_big, 16384);
        assert!(a_small < a_big);
        assert!(a_small >= 1);
    }

    #[test]
    fn footprint_area_clamped_to_canvas() {
        // A huge shape cannot exceed the canvas area.
        let huge = p(50.0, 50.0, 0.0, 50.0, 1.0);
        let a = footprint_area(&huge, (100, 100), 128);
        assert_eq!(a, 100 * 100);
    }

    #[test]
    fn footprint_area_never_zero() {
        let tiny = p(0.0, 0.0, 0.0, 0.0001, 1.0);
        assert!(footprint_area(&tiny, (640, 480), 128) >= 1);
    }

    #[test]
    fn recolor_in_place_keeps_geometry_changes_color() {
        // 4×4 solid red target: any sampled colour must be (1, 0, 0).
        let mut px = Vec::new();
        for _ in 0..(4 * 4) {
            px.extend_from_slice(&[255, 0, 0, 255]);
        }
        let gen = CandidateGenerator::new(Settings::default(), px, (4, 4));

        // A shape sitting on the target with a stale (blue) colour.
        let mut shape = p(2.0, 2.0, 0.7, 1.5, 0.8);
        shape.shape_index = 3;
        shape.r = 0.0;
        shape.g = 0.0;
        shape.b = 1.0;

        let recolored = recolor_in_place(&shape, &gen);

        // Geometry / identity untouched.
        assert_eq!(recolored.shape_index, shape.shape_index);
        assert!((recolored.x - shape.x).abs() < 1e-6);
        assert!((recolored.y - shape.y).abs() < 1e-6);
        assert!((recolored.rotation - shape.rotation).abs() < 1e-6);
        assert!((recolored.scale - shape.scale).abs() < 1e-6);
        assert!((recolored.alpha - shape.alpha).abs() < 1e-6);

        // Colour refreshed to the target's red.
        assert!((recolored.r - 1.0).abs() < 1e-6);
        assert!(recolored.g.abs() < 1e-6);
        assert!(recolored.b.abs() < 1e-6);
    }
}
