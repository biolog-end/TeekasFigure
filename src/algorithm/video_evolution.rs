// Video temporal coherence: shape mutation, removal, displacement penalty, and vacancy filling

use crate::gpu::GpuContext;
use crate::settings::Settings;
use crate::types::{CandidateParams, PlacedShape};

use super::CandidateGenerator;

/// Manages the temporal coherence algorithm for video frame processing.
///
/// On the first frame, shapes are placed from scratch using the standard hill climbing algorithm.
/// On subsequent frames, existing shapes are mutated to fit the new target. Shapes that cannot
/// improve after `mutations_per_shape` attempts are removed, and new candidates fill the vacancies.
pub struct VideoEvolution {
    /// List of currently placed shapes with their previous centroids.
    pub shape_list: Vec<PlacedShape>,
}

impl VideoEvolution {
    /// Create a new VideoEvolution with an empty shape list.
    pub fn new() -> Self {
        Self {
            shape_list: Vec::new(),
        }
    }

    /// Record a newly placed shape (used during first frame generation).
    ///
    /// Stores the shape's current centroid as `prev_centroid` for displacement
    /// penalty calculation on the next frame.
    pub fn record_shape(&mut self, params: CandidateParams) {
        let centroid = (params.x, params.y);
        self.shape_list.push(PlacedShape {
            params,
            prev_centroid: centroid,
        });
    }

    /// Mutate existing shapes to fit a new target frame.
    ///
    /// For each shape in the shape list, generates `mutations_per_shape` random mutations
    /// (small perturbations to position, rotation, scale, color). Evaluates each mutation's
    /// fitness on the GPU. If any mutation improves fitness (accounting for displacement penalty),
    /// accepts it and updates the shape. If none improve after all attempts, marks the shape
    /// for removal.
    ///
    /// After processing all shapes, removes stale shapes and updates `prev_centroid`
    /// for surviving shapes.
    ///
    /// Returns the number of shapes removed.
    pub fn mutate_for_new_frame(
        &mut self,
        gpu: &GpuContext,
        generator: &mut CandidateGenerator,
        settings: &Settings,
    ) -> u32 {
        let mutations_per_shape = settings.mutations_per_shape;
        let displacement_weight = settings.displacement_weight;
        let mut to_remove: Vec<usize> = Vec::new();

        for (shape_idx, placed) in self.shape_list.iter_mut().enumerate() {
            let mut improved = false;
            let current_params = placed.params;
            let prev_centroid = placed.prev_centroid;

            // Evaluate current shape fitness (baseline)
            gpu.dispatch_mse_evaluation(&[current_params]);
            let baseline_scores = gpu.read_fitness_scores();
            let baseline_fitness = baseline_scores.first().copied().unwrap_or(f32::INFINITY)
                + Self::displacement_penalty(
                    prev_centroid,
                    (current_params.x, current_params.y),
                    displacement_weight,
                );

            for _ in 0..mutations_per_shape {
                // Generate a single mutation of the current shape
                let mutated = Self::mutate_shape(&current_params, generator, gpu.canvas_size);

                // Evaluate the mutated shape
                gpu.dispatch_mse_evaluation(&[mutated]);
                let scores = gpu.read_fitness_scores();
                let mutated_fitness = scores.first().copied().unwrap_or(f32::INFINITY)
                    + Self::displacement_penalty(
                        prev_centroid,
                        (mutated.x, mutated.y),
                        displacement_weight,
                    );

                if mutated_fitness < baseline_fitness {
                    // Accept the mutation
                    placed.params = mutated;
                    improved = true;
                    break;
                }
            }

            if !improved {
                to_remove.push(shape_idx);
            }
        }

        let removed_count = to_remove.len() as u32;

        // Remove stale shapes in reverse order to preserve indices
        for &idx in to_remove.iter().rev() {
            self.shape_list.remove(idx);
        }

        // Update prev_centroid for surviving shapes
        for placed in self.shape_list.iter_mut() {
            placed.prev_centroid = (placed.params.x, placed.params.y);
        }

        removed_count
    }

    /// Compute displacement penalty between two centroids.
    ///
    /// penalty = weight × sqrt((x2 - x1)² + (y2 - y1)²)
    ///
    /// This penalizes shapes that move far from their previous position,
    /// encouraging temporal coherence in video mode.
    pub fn displacement_penalty(
        prev: (f32, f32),
        current: (f32, f32),
        weight: f32,
    ) -> f32 {
        let dx = current.0 - prev.0;
        let dy = current.1 - prev.1;
        weight * (dx * dx + dy * dy).sqrt()
    }

    /// Get the number of vacant slots (max_shapes - current shapes).
    ///
    /// Returns how many new shapes can be added to fill removed shape slots.
    pub fn vacant_slots(&self, max_shapes: u32) -> u32 {
        let current = self.shape_list.len() as u32;
        if current >= max_shapes {
            0
        } else {
            max_shapes - current
        }
    }

    /// Generate a small random mutation of a shape's parameters.
    ///
    /// Perturbations applied:
    /// - Position: ±5% of canvas dimensions
    /// - Rotation: ±0.3 radians
    /// - Scale: ±20% of current scale
    /// - Color channels: ±0.1 (clamped to [0.0, 1.0])
    /// - Alpha: ±0.1 (clamped to [0.1, 1.0])
    fn mutate_shape(
        params: &CandidateParams,
        generator: &mut CandidateGenerator,
        canvas_size: (u32, u32),
    ) -> CandidateParams {
        use rand::Rng;

        let rng = generator.rng_mut();
        let (cw, ch) = canvas_size;

        // Position perturbation: ±5% of canvas size
        let dx = rng.gen_range(-(cw as f32 * 0.05)..=(cw as f32 * 0.05));
        let dy = rng.gen_range(-(ch as f32 * 0.05)..=(ch as f32 * 0.05));
        let new_x = (params.x + dx).clamp(0.0, (cw - 1) as f32);
        let new_y = (params.y + dy).clamp(0.0, (ch - 1) as f32);

        // Rotation perturbation: ±0.3 radians
        let dr = rng.gen_range(-0.3_f32..=0.3);
        let new_rotation = (params.rotation + dr).rem_euclid(std::f32::consts::TAU);

        // Scale perturbation: ±20% of current scale
        let scale_factor = rng.gen_range(0.8_f32..=1.2);
        let new_scale = (params.scale * scale_factor).clamp(0.01, 2.0);

        // Color perturbation: ±0.1 per channel
        let cr = rng.gen_range(-0.1_f32..=0.1);
        let cg = rng.gen_range(-0.1_f32..=0.1);
        let cb = rng.gen_range(-0.1_f32..=0.1);
        let new_r = (params.r + cr).clamp(0.0, 1.0);
        let new_g = (params.g + cg).clamp(0.0, 1.0);
        let new_b = (params.b + cb).clamp(0.0, 1.0);

        // Alpha perturbation: ±0.1
        let da = rng.gen_range(-0.1_f32..=0.1);
        let new_alpha = (params.alpha + da).clamp(0.1, 1.0);

        CandidateParams {
            shape_index: params.shape_index,
            x: new_x,
            y: new_y,
            rotation: new_rotation,
            scale: new_scale,
            r: new_r,
            g: new_g,
            b: new_b,
            alpha: new_alpha,
            scale_y: new_scale,
            use_original_color: params.use_original_color,
            _padding: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_displacement_penalty_zero_distance() {
        let penalty = VideoEvolution::displacement_penalty((10.0, 20.0), (10.0, 20.0), 1.0);
        assert!((penalty - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_displacement_penalty_unit_distance() {
        // Distance of 5.0 (3-4-5 triangle)
        let penalty = VideoEvolution::displacement_penalty((0.0, 0.0), (3.0, 4.0), 1.0);
        assert!((penalty - 5.0).abs() < 1e-5);
    }

    #[test]
    fn test_displacement_penalty_with_weight() {
        // Distance of 5.0, weight of 2.0 → penalty = 10.0
        let penalty = VideoEvolution::displacement_penalty((0.0, 0.0), (3.0, 4.0), 2.0);
        assert!((penalty - 10.0).abs() < 1e-5);
    }

    #[test]
    fn test_displacement_penalty_zero_weight() {
        // Any distance with weight 0 → penalty = 0
        let penalty = VideoEvolution::displacement_penalty((0.0, 0.0), (100.0, 200.0), 0.0);
        assert!((penalty - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_displacement_penalty_negative_coordinates() {
        // From (-3, -4) to (0, 0) → distance = 5.0
        let penalty = VideoEvolution::displacement_penalty((-3.0, -4.0), (0.0, 0.0), 1.0);
        assert!((penalty - 5.0).abs() < 1e-5);
    }

    #[test]
    fn test_displacement_penalty_large_distance() {
        // From (0, 0) to (300, 400) → distance = 500.0, weight = 0.5 → penalty = 250.0
        let penalty = VideoEvolution::displacement_penalty((0.0, 0.0), (300.0, 400.0), 0.5);
        assert!((penalty - 250.0).abs() < 1e-3);
    }

    #[test]
    fn test_displacement_penalty_symmetric() {
        let p1 = VideoEvolution::displacement_penalty((1.0, 2.0), (4.0, 6.0), 1.5);
        let p2 = VideoEvolution::displacement_penalty((4.0, 6.0), (1.0, 2.0), 1.5);
        assert!((p1 - p2).abs() < 1e-5);
    }

    #[test]
    fn test_new_creates_empty_shape_list() {
        let ve = VideoEvolution::new();
        assert!(ve.shape_list.is_empty());
    }

    #[test]
    fn test_record_shape_adds_to_list() {
        let mut ve = VideoEvolution::new();
        let params = CandidateParams {
            shape_index: 0,
            x: 50.0,
            y: 75.0,
            rotation: 1.0,
            scale: 0.5,
            r: 0.5,
            g: 0.5,
            b: 0.5,
            alpha: 0.8,
            scale_y: 0.5,
            use_original_color: 0.0,
            _padding: 0.0,
        };
        ve.record_shape(params);
        assert_eq!(ve.shape_list.len(), 1);
        assert_eq!(ve.shape_list[0].prev_centroid, (50.0, 75.0));
    }

    #[test]
    fn test_vacant_slots_empty_list() {
        let ve = VideoEvolution::new();
        assert_eq!(ve.vacant_slots(100), 100);
    }

    #[test]
    fn test_vacant_slots_partial_fill() {
        let mut ve = VideoEvolution::new();
        for i in 0..30 {
            ve.record_shape(CandidateParams {
                shape_index: 0,
                x: i as f32,
                y: 0.0,
                rotation: 0.0,
                scale: 0.1,
                r: 0.0,
                g: 0.0,
                b: 0.0,
                alpha: 0.5,
                scale_y: 0.1,
                use_original_color: 0.0,
                _padding: 0.0,
            });
        }
        assert_eq!(ve.vacant_slots(100), 70);
    }

    #[test]
    fn test_vacant_slots_full() {
        let mut ve = VideoEvolution::new();
        for i in 0..50 {
            ve.record_shape(CandidateParams {
                shape_index: 0,
                x: i as f32,
                y: 0.0,
                rotation: 0.0,
                scale: 0.1,
                r: 0.0,
                g: 0.0,
                b: 0.0,
                alpha: 0.5,
                scale_y: 0.1,
                use_original_color: 0.0,
                _padding: 0.0,
            });
        }
        assert_eq!(ve.vacant_slots(50), 0);
    }

    #[test]
    fn test_vacant_slots_over_max() {
        let mut ve = VideoEvolution::new();
        for i in 0..60 {
            ve.record_shape(CandidateParams {
                shape_index: 0,
                x: i as f32,
                y: 0.0,
                rotation: 0.0,
                scale: 0.1,
                r: 0.0,
                g: 0.0,
                b: 0.0,
                alpha: 0.5,
                scale_y: 0.1,
                use_original_color: 0.0,
                _padding: 0.0,
            });
        }
        assert_eq!(ve.vacant_slots(50), 0);
    }
}
