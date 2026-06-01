// Video temporal coherence pipeline.
//
// Core idea: shapes are a living ecosystem that adapts to changing frames.
// - Shapes inherit from previous frame (no reset)
// - Each shape mutates locally (small position/rotation/scale changes)
// - Movement penalty prevents shapes from jumping across the screen
// - Shapes that can't adapt die and are replaced by new random ones
// - Canvas is rebuilt from the full ordered shape list after adaptation

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::gpu::GpuContext;
use crate::settings::Settings;
use crate::types::CandidateParams;
use super::CandidateGenerator;

/// A shape living on the canvas with memory of its previous state.
#[derive(Clone, Copy, Debug)]
pub struct PlacedShapeRecord {
    /// Current parameters of this shape
    pub params: CandidateParams,
    /// Parameters from the PREVIOUS frame (for movement penalty)
    pub prev_params: CandidateParams,
}

/// Video pipeline: manages the ecosystem of shapes across frames.
pub struct VideoPipeline {
    /// Ordered list of all shapes on the canvas (bottom to top)
    pub shapes: Vec<PlacedShapeRecord>,
    /// Current frame index
    pub frame_index: u32,
    /// RNG for mutations
    rng: SmallRng,
}

impl VideoPipeline {
    pub fn new() -> Self {
        Self {
            shapes: Vec::new(),
            frame_index: 0,
            rng: SmallRng::from_entropy(),
        }
    }

    /// Record a shape placed during frame generation.
    pub fn record_placed_shape(&mut self, params: CandidateParams) {
        self.shapes.push(PlacedShapeRecord {
            params,
            prev_params: params, // First frame: prev = current
        });
    }

    /// Adapt all shapes to a new target frame.
    ///
    /// For each shape:
    /// 1. Generate small mutations (local search)
    /// 2. Evaluate each mutation WITH movement penalty
    /// 3. Keep the best mutation if it improves fitness
    /// 4. If shape can't adapt at all → mark as dead
    ///
    /// After all shapes are processed:
    /// - Remove dead shapes
    /// - Rebuild canvas from surviving shapes (preserving order)
    ///
    /// Returns number of dead shapes removed.
    pub fn adapt_to_new_frame(
        &mut self,
        gpu: &GpuContext,
        generator: &mut CandidateGenerator,
        settings: &Settings,
    ) -> u32 {
        self.frame_index += 1;
        let mut dead_indices: Vec<usize> = Vec::new();
        let num_shapes = self.shapes.len();
        let mutations_per_shape = settings.mutations_per_shape;

        for idx in 0..num_shapes {
            let current = self.shapes[idx].params;
            let prev = self.shapes[idx].prev_params;

            // Generate mutations: small local changes to position, rotation, scale
            let mut candidates = Vec::with_capacity(mutations_per_shape as usize + 1);
            candidates.push(current); // Include the unchanged shape as a candidate

            for _ in 0..mutations_per_shape {
                candidates.push(self.create_local_mutation(
                    &current,
                    gpu.canvas_size,
                    generator,
                    settings,
                ));
            }

            // Evaluate all candidates on GPU
            gpu.dispatch_mse_evaluation(&candidates);
            let scores = gpu.read_fitness_scores();

            // Find best candidate WITH movement penalty
            let mut best_idx = 0;
            let mut best_penalized = f32::MAX;

            for (i, &score) in scores.iter().enumerate().take(candidates.len()) {
                let penalty = self.movement_penalty(&prev, &candidates[i], settings);
                let penalized = score + penalty;
                if penalized < best_penalized {
                    best_penalized = penalized;
                    best_idx = i;
                }
            }

            // Check if the shape is still useful (negative delta = improves canvas)
            if best_penalized < settings.min_improvement * 0.1 {
                // Shape adapted successfully — update it
                self.shapes[idx].prev_params = current; // Remember where we were
                self.shapes[idx].params = candidates[best_idx];
            } else {
                // Shape is dead — can't contribute to new frame
                dead_indices.push(idx);
            }
        }

        // Remove dead shapes (reverse order to preserve indices)
        let dead_count = dead_indices.len() as u32;
        for &idx in dead_indices.iter().rev() {
            self.shapes.remove(idx);
        }

        dead_count
    }

    /// Rebuild the canvas from scratch using the current ordered shape list.
    /// Must be called after adapt_to_new_frame to reflect the new state.
    pub fn rebuild_canvas(&self, gpu: &GpuContext) {
        gpu.clear_canvas();
        for record in &self.shapes {
            gpu.composite_shape(&record.params);
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

        // Scale: very small change (±10%)
        let scale_factor = self.rng.gen_range(0.9_f32..=1.1);
        let new_scale = (parent.scale * scale_factor).clamp(settings.scale_min, settings.scale_max);

        // Alpha: tiny change (±0.05) or fixed if evolve_opacity is off
        let new_alpha = if settings.evolve_opacity {
            let da = self.rng.gen_range(-0.05_f32..=0.05);
            (parent.alpha + da).clamp(0.1, 1.0)
        } else {
            1.0
        };

        // Color: re-sample from target at new position (adapts to new frame's colors)
        let (r, g, b) = generator.sample_color_at(new_x as u32, new_y as u32);

        CandidateParams {
            shape_index: parent.shape_index,
            x: new_x,
            y: new_y,
            rotation: new_rotation,
            scale: new_scale,
            r, g, b,
            alpha: new_alpha,
            _padding: [0.0, 0.0, 0.0],
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
        // Position distance (normalized by canvas diagonal)
        let dx = current.x - prev.x;
        let dy = current.y - prev.y;
        let pos_dist = (dx * dx + dy * dy).sqrt();

        // Scale change (ratio)
        let scale_ratio = if prev.scale > 0.001 {
            (current.scale / prev.scale - 1.0).abs()
        } else {
            0.0
        };

        // Rotation change (in radians, wrapped to [0, π])
        let rot_diff = (current.rotation - prev.rotation).abs();
        let rot_diff = rot_diff.min(std::f32::consts::TAU - rot_diff);

        // Combined penalty: position is most important, scale and rotation less so
        let penalty = pos_dist + scale_ratio * 50.0 + rot_diff * 10.0;

        settings.displacement_weight * penalty
    }
}
