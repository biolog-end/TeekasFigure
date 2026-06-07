// Candidate generation with adaptive scale and color heuristic sampling

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::settings::Settings;
use crate::types::CandidateParams;

/// Generates batches of randomized candidate shape placements.
///
/// Uses a fast PRNG (SmallRng) and samples color from the target image
/// with a 3×3 neighborhood average plus random deviation.
pub struct CandidateGenerator {
    rng: SmallRng,
    settings: Settings,
    /// RGBA8 target image pixel data (row-major, 4 bytes per pixel)
    target_pixels: Vec<u8>,
    target_width: u32,
    target_height: u32,
}

impl CandidateGenerator {
    /// Create a new CandidateGenerator.
    ///
    /// # Arguments
    /// * `settings` - Application settings containing scale/color parameters
    /// * `target_pixels` - RGBA8 pixel data of the target image
    /// * `target_size` - (width, height) of the target image
    pub fn new(settings: Settings, target_pixels: Vec<u8>, target_size: (u32, u32)) -> Self {
        Self {
            rng: SmallRng::from_entropy(),
            settings,
            target_pixels,
            target_width: target_size.0,
            target_height: target_size.1,
        }
    }

    /// Get a mutable reference to the internal RNG.
    ///
    /// Used by `VideoEvolution` to generate small random perturbations for shape mutations.
    pub fn rng_mut(&mut self) -> &mut SmallRng {
        &mut self.rng
    }

    /// Generate a batch of candidate shape placements.
    ///
    /// Produces exactly `batch_size` candidates with randomized parameters:
    /// - shape_index: uniform [0, num_shapes - 1]
    /// - x: uniform [0, canvas_width - 1]
    /// - y: uniform [0, canvas_height - 1]
    /// - rotation: uniform [0, 2π)
    /// - scale: uniform [scale_min, adaptive_max]
    /// - color: sampled from target 3×3 neighborhood + random deviation
    /// - alpha: uniform [0.1, 1.0]
    ///
    /// # Arguments
    /// * `batch_size` - Number of candidates to generate
    /// * `placed_shapes` - Current number of shapes already placed on canvas
    /// * `canvas_size` - (width, height) of the canvas
    /// * `num_shapes` - Number of available shape textures
    pub fn generate_batch(
        &mut self,
        batch_size: u32,
        placed_shapes: u32,
        canvas_size: (u32, u32),
        num_shapes: u32,
    ) -> Vec<CandidateParams> {
        let (canvas_width, canvas_height) = canvas_size;
        let adaptive_max = self.compute_adaptive_scale(placed_shapes);

        let mut candidates = Vec::with_capacity(batch_size as usize);

        for _ in 0..batch_size {
            let shape_index = self.rng.gen_range(0..num_shapes);
            let x = self.rng.gen_range(0..canvas_width) as f32;
            let y = self.rng.gen_range(0..canvas_height) as f32;
            let rotation = self.rng.gen_range(0.0..std::f32::consts::TAU);
            let scale = self.rng.gen_range(self.settings.scale_min..=adaptive_max);
            // Per-axis (Y) scale. With non-uniform scaling enabled it is sampled
            // independently so shapes can start stretched/squashed; otherwise it
            // mirrors `scale` for classic uniform sizing.
            let scale_y = if self.settings.evolve_non_uniform_scale {
                self.rng.gen_range(self.settings.scale_min..=adaptive_max)
            } else {
                scale
            };
            let alpha = if self.settings.evolve_opacity {
                self.rng.gen_range(0.1..=1.0_f32)
            } else {
                1.0
            };

            let (r, g, b) = self.sample_color_at(x as u32, y as u32);

            candidates.push(CandidateParams {
                shape_index,
                x,
                y,
                rotation,
                scale,
                r,
                g,
                b,
                alpha,
                scale_y,
                use_original_color: if self.settings.use_original_colors { 1.0 } else { 0.0 },
                _padding: 0.0,
            });
        }

        candidates
    }

    /// Compute the adaptive maximum scale based on progress.
    ///
    /// adaptive_max = scale_max - (scale_max - scale_min) × (placed_shapes / max_shapes)
    ///
    /// This linearly decreases from scale_max (at 0 placed shapes) to scale_min
    /// (at max_shapes placed shapes).
    fn compute_adaptive_scale(&self, placed_shapes: u32) -> f32 {
        let progress = placed_shapes as f32 / self.settings.max_shapes as f32;
        let progress = progress.min(1.0); // clamp to avoid going below scale_min
        self.settings.scale_max
            - (self.settings.scale_max - self.settings.scale_min) * progress
    }

    /// Sample color from the target image at (x, y) using a 3×3 neighborhood average.
    /// Returns normalized (r, g, b) values in [0.0, 1.0] without random deviation.
    /// Used by the evolutionary algorithm to assign colors based on position.
    pub fn sample_color_at(&self, x: u32, y: u32) -> (f32, f32, f32) {
        let (avg_r, avg_g, avg_b) = self.sample_3x3_average(x, y);
        (avg_r as f32 / 255.0, avg_g as f32 / 255.0, avg_b as f32 / 255.0)
    }

    /// Sample the average RGB values from a 3×3 neighborhood centered at (x, y).
    ///
    /// Coordinates are clamped to valid texture bounds [0, width-1] × [0, height-1].
    /// Returns average (r, g, b) as u8 values.
    fn sample_3x3_average(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let mut sum_r: u32 = 0;
        let mut sum_g: u32 = 0;
        let mut sum_b: u32 = 0;

        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                // Clamp sampling coordinates to valid texture bounds
                let sx = (x as i32 + dx).clamp(0, self.target_width as i32 - 1) as u32;
                let sy = (y as i32 + dy).clamp(0, self.target_height as i32 - 1) as u32;

                let idx = ((sy * self.target_width + sx) * 4) as usize;
                sum_r += self.target_pixels[idx] as u32;
                sum_g += self.target_pixels[idx + 1] as u32;
                sum_b += self.target_pixels[idx + 2] as u32;
            }
        }

        ((sum_r / 9) as u8, (sum_g / 9) as u8, (sum_b / 9) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_settings() -> Settings {
        Settings::default()
    }

    /// Create a simple 4×4 RGBA target image filled with a solid color.
    fn solid_target(r: u8, g: u8, b: u8, width: u32, height: u32) -> Vec<u8> {
        let pixel_count = (width * height) as usize;
        let mut pixels = Vec::with_capacity(pixel_count * 4);
        for _ in 0..pixel_count {
            pixels.push(r);
            pixels.push(g);
            pixels.push(b);
            pixels.push(255); // alpha
        }
        pixels
    }

    #[test]
    fn test_generate_batch_produces_exact_count() {
        let settings = default_settings();
        let target = solid_target(128, 64, 200, 64, 64);
        let mut gen = CandidateGenerator::new(settings, target, (64, 64));

        let batch = gen.generate_batch(100, 0, (64, 64), 4);
        assert_eq!(batch.len(), 100);
    }

    #[test]
    fn test_candidate_params_within_bounds() {
        let settings = default_settings();
        let target = solid_target(100, 150, 200, 128, 128);
        let mut gen = CandidateGenerator::new(settings.clone(), target, (128, 128));

        let batch = gen.generate_batch(500, 0, (128, 128), 8);

        for candidate in &batch {
            assert!(candidate.shape_index < 8, "shape_index out of bounds");
            assert!(candidate.x >= 0.0 && candidate.x < 128.0, "x out of bounds: {}", candidate.x);
            assert!(candidate.y >= 0.0 && candidate.y < 128.0, "y out of bounds: {}", candidate.y);
            assert!(candidate.rotation >= 0.0 && candidate.rotation < std::f32::consts::TAU,
                "rotation out of bounds: {}", candidate.rotation);
            assert!(candidate.scale >= settings.scale_min && candidate.scale <= settings.scale_max,
                "scale out of bounds: {}", candidate.scale);
            assert!(candidate.r >= 0.0 && candidate.r <= 1.0, "r out of bounds: {}", candidate.r);
            assert!(candidate.g >= 0.0 && candidate.g <= 1.0, "g out of bounds: {}", candidate.g);
            assert!(candidate.b >= 0.0 && candidate.b <= 1.0, "b out of bounds: {}", candidate.b);
            assert!(candidate.alpha >= 0.1 && candidate.alpha <= 1.0,
                "alpha out of bounds: {}", candidate.alpha);
        }
    }

    #[test]
    fn test_adaptive_scale_at_zero_placed() {
        let settings = default_settings();
        let target = solid_target(0, 0, 0, 4, 4);
        let gen = CandidateGenerator::new(settings.clone(), target, (4, 4));

        let adaptive_max = gen.compute_adaptive_scale(0);
        assert!((adaptive_max - settings.scale_max).abs() < f32::EPSILON,
            "At 0 placed shapes, adaptive_max should equal scale_max");
    }

    #[test]
    fn test_adaptive_scale_at_max_placed() {
        let settings = default_settings();
        let target = solid_target(0, 0, 0, 4, 4);
        let gen = CandidateGenerator::new(settings.clone(), target, (4, 4));

        let adaptive_max = gen.compute_adaptive_scale(settings.max_shapes);
        assert!((adaptive_max - settings.scale_min).abs() < f32::EPSILON,
            "At max_shapes placed, adaptive_max should equal scale_min");
    }

    #[test]
    fn test_adaptive_scale_midpoint() {
        let settings = default_settings();
        let target = solid_target(0, 0, 0, 4, 4);
        let gen = CandidateGenerator::new(settings.clone(), target, (4, 4));

        let midpoint = settings.max_shapes / 2;
        let adaptive_max = gen.compute_adaptive_scale(midpoint);
        let expected = (settings.scale_max + settings.scale_min) / 2.0;
        assert!((adaptive_max - expected).abs() < 0.01,
            "At midpoint, adaptive_max should be approximately the average of scale_min and scale_max");
    }

    #[test]
    fn test_color_sampling_solid_target() {
        let settings = default_settings();
        let target = solid_target(100, 150, 200, 16, 16);
        let mut gen = CandidateGenerator::new(settings.clone(), target, (16, 16));

        // For a solid color target, the 3×3 average should be the same color
        let (avg_r, avg_g, avg_b) = gen.sample_3x3_average(8, 8);
        assert_eq!(avg_r, 100);
        assert_eq!(avg_g, 150);
        assert_eq!(avg_b, 200);
    }

    #[test]
    fn test_color_sampling_clamped_at_corner() {
        let settings = default_settings();
        let target = solid_target(50, 100, 150, 4, 4);
        let mut gen = CandidateGenerator::new(settings, target, (4, 4));

        // Sampling at (0, 0) should clamp negative offsets to (0, 0)
        let (avg_r, avg_g, avg_b) = gen.sample_3x3_average(0, 0);
        // For a solid target, clamping doesn't change the result
        assert_eq!(avg_r, 50);
        assert_eq!(avg_g, 100);
        assert_eq!(avg_b, 150);
    }

    #[test]
    fn test_batch_size_zero() {
        let settings = default_settings();
        let target = solid_target(0, 0, 0, 4, 4);
        let mut gen = CandidateGenerator::new(settings, target, (4, 4));

        let batch = gen.generate_batch(0, 0, (4, 4), 4);
        assert!(batch.is_empty());
    }

    #[test]
    fn test_padding_is_zero() {
        let settings = default_settings();
        let target = solid_target(128, 128, 128, 32, 32);
        let mut gen = CandidateGenerator::new(settings, target, (32, 32));

        let batch = gen.generate_batch(10, 0, (32, 32), 2);
        for candidate in &batch {
            assert_eq!(candidate._padding, 0.0);
        }
    }

    #[test]
    fn test_uniform_scale_sets_scale_y_equal() {
        let settings = default_settings(); // evolve_non_uniform_scale = false
        let target = solid_target(128, 128, 128, 32, 32);
        let mut gen = CandidateGenerator::new(settings, target, (32, 32));

        let batch = gen.generate_batch(50, 0, (32, 32), 2);
        for candidate in &batch {
            assert_eq!(candidate.scale, candidate.scale_y,
                "uniform scaling must keep scale_y == scale");
            assert_eq!(candidate.use_original_color, 0.0);
        }
    }

    #[test]
    fn test_non_uniform_scale_allows_different_axes() {
        let mut settings = default_settings();
        settings.evolve_non_uniform_scale = true;
        let target = solid_target(128, 128, 128, 32, 32);
        let mut gen = CandidateGenerator::new(settings.clone(), target, (32, 32));

        let batch = gen.generate_batch(200, 0, (32, 32), 2);
        // At least one candidate should have differing axes (probabilistically certain).
        let any_different = batch.iter().any(|c| (c.scale - c.scale_y).abs() > f32::EPSILON);
        assert!(any_different, "non-uniform scaling should produce differing axis scales");
        for candidate in &batch {
            assert!(candidate.scale_y >= settings.scale_min && candidate.scale_y <= settings.scale_max,
                "scale_y out of bounds: {}", candidate.scale_y);
        }
    }

    #[test]
    fn test_original_colors_flag_set() {
        let mut settings = default_settings();
        settings.use_original_colors = true;
        let target = solid_target(128, 128, 128, 32, 32);
        let mut gen = CandidateGenerator::new(settings, target, (32, 32));

        let batch = gen.generate_batch(10, 0, (32, 32), 2);
        for candidate in &batch {
            assert_eq!(candidate.use_original_color, 1.0);
        }
    }
}
