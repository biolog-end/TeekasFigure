// Settings module: TOML loading, validation, and defaults

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Default TOML content with inline comments documenting each parameter.
const DEFAULT_SETTINGS_TOML: &str = r#"# GPU Image Approximator Configuration

# Number of candidates in initial population (1–4096)
batch_size = 1000

# Maximum shapes to place before stopping (1–100000)
max_shapes = 4000

# Successful placements per rendered frame (1–100)
mutations_per_frame = 1

# Maximum texture dimension for input media (16–2048 pixels)
max_texture_size = 512

# VRAM budget in megabytes (128–4096)
vram_budget_mb = 2048

# Minimum scale factor at max_shapes (0.01–1.0)
scale_min = 0.02

# Maximum scale factor at start (0.1–20.0)
scale_max = 5.0

# Shape texture resolution in pixels (16–1024)
shape_resolution = 128

# Mutation attempts per shape in video mode (1–50)
mutations_per_shape = 10

# Displacement penalty weight for video temporal coherence (0.0–100.0)
displacement_weight = 0.01

# Scene-change tolerance for video mode (-10.0–10.0). A living shape only dies when,
# even at its best low-movement position with resampled color, it makes its own
# region worse than this per-pixel error threshold (i.e. the target pixels under it
# changed drastically — a real scene change). Larger = shapes are kept more
# aggressively (denser, more coherent); smaller = shapes die more eagerly.
# NEGATIVE values are allowed and make the test STRICT: a shape must actively
# IMPROVE its region by at least |value| per pixel to survive (and a reborn shape
# must improve by that much to be placed) — more turnover, more aggressive rebuild.
scene_change_tolerance = 0.005

# Number of linearly-interpolated frames inserted between two evolved keyframes (0–20).
# 0 disables interpolation (every saved frame is fully evolved). When > 0, output
# framerate becomes target_fps * (interpolation_steps + 1) and motion looks much smoother.
interpolation_steps = 2

# Target FPS for video processing (1–60). Video is resampled to this framerate.
target_fps = 12

# Whether shapes can have variable opacity (true) or always fully opaque (false)
evolve_opacity = true

# Whether existing shapes RE-COLOR themselves to match the new frame during video
# adaptation (true) or keep their original color and only move/rotate/scale (false).
# Default is false: a shape placed on frame 1 keeps its color for the whole video and
# only its position/rotation/scale adapt. Set true for the old behaviour where shapes
# resample the new frame's colors as they adapt. (Reborn shapes filling gaps always
# take a fresh color regardless, since they have no previous color.)
video_recolor = false

# --- Evolution Algorithm Parameters ---

# Number of evolutionary generations per shape placement (1–20)
num_generations = 4

# Minimum improvement threshold (negative = must improve by this much)
# If best candidate improves less than this, shape is rejected
min_improvement = -0.5

# Whether to use min_improvement threshold (true) or always accept best candidate (false)
use_min_improvement = true

# How many consecutive rejections before stopping (1–500)
max_rejections = 50

# Fraction of population that survives each generation (0.01–1.0)
survival_rate = 0.10

# Number of children per surviving parent (1–50)
children_per_parent = 9

# --- Shape Diversity Mode ---

# Enable shape diversity mode (true/false). When enabled, frequently-used shapes
# get a penalty, encouraging the algorithm to use a variety of shapes.
diversity_mode = false

# Penalty added to a shape each time it is placed on the canvas (0.0–10.0)
diversity_penalty_increment = 0.1

# Whether other shapes' penalties decrease when one shape is chosen (true/false)
diversity_decay_enabled = true

# Amount by which other shapes' penalties decrease per placement (0.0–10.0)
diversity_decay_amount = 0.01
"#;

/// All configurable parameters loaded from settings.toml.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Number of candidates evaluated per batch (1–4096).
    pub batch_size: u32,
    /// Maximum shapes to place before stopping (1–100000).
    pub max_shapes: u32,
    /// Successful placements per rendered frame (1–100).
    pub mutations_per_frame: u32,
    /// Maximum texture dimension for input media (16–2048 pixels).
    pub max_texture_size: u32,
    /// VRAM budget in megabytes (128–4096).
    pub vram_budget_mb: u32,
    /// Minimum scale factor at max_shapes (0.01–1.0).
    pub scale_min: f32,
    /// Maximum scale factor at start (0.1–2.0).
    pub scale_max: f32,
    /// Shape texture resolution in pixels (16–1024).
    pub shape_resolution: u32,
    /// Mutation attempts per shape in video mode (1–50).
    pub mutations_per_shape: u32,
    /// Displacement penalty weight for video temporal coherence (0.0–100.0).
    pub displacement_weight: f32,
    /// Scene-change tolerance for video mode (-10.0–10.0). Per-pixel error threshold
    /// above which a living shape is considered no longer matching the target and dies.
    /// Negative values require a shape to actively improve its region to survive.
    pub scene_change_tolerance: f32,
    /// Number of linearly interpolated frames inserted between two evolved keyframes (0–20).
    pub interpolation_steps: u32,
    /// Target FPS for video processing (1–60). Video is resampled to this framerate before processing.
    pub target_fps: u32,
    /// Whether shapes can have variable opacity (true) or are always fully opaque (false).
    pub evolve_opacity: bool,
    /// Whether existing shapes re-color to the new frame during video adaptation (true)
    /// or keep their original color and only adapt geometry (false). Default false.
    pub video_recolor: bool,
    /// Number of evolutionary generations per shape placement (1–20).
    pub num_generations: u32,
    /// Minimum improvement threshold (negative). Shape rejected if best delta > this.
    pub min_improvement: f32,
    /// Whether to use min_improvement threshold (true) or always accept best (false).
    pub use_min_improvement: bool,
    /// Consecutive rejections before stopping (1–500).
    pub max_rejections: u32,
    /// Fraction of population that survives each generation (0.01–1.0).
    pub survival_rate: f32,
    /// Number of children per surviving parent (1–50).
    pub children_per_parent: u32,
    /// Enable shape diversity mode.
    pub diversity_mode: bool,
    /// Penalty added to a shape each time it is placed on the canvas.
    pub diversity_penalty_increment: f32,
    /// Whether other shapes' penalties decrease when one shape is chosen.
    pub diversity_decay_enabled: bool,
    /// Amount by which other shapes' penalties decrease per placement.
    pub diversity_decay_amount: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            batch_size: 1000,
            max_shapes: 4000,
            mutations_per_frame: 10,
            max_texture_size: 512,
            vram_budget_mb: 2048,
            scale_min: 0.02,
            scale_max: 8.0,
            shape_resolution: 128,
            mutations_per_shape: 10,
            displacement_weight: 0.01,
            scene_change_tolerance: 0.005,
            interpolation_steps: 2,
            target_fps: 12,
            evolve_opacity: true,
            video_recolor: false,
            num_generations: 4,
            min_improvement: -0.5,
            use_min_improvement: true,
            max_rejections: 50,
            survival_rate: 0.10,
            children_per_parent: 9,
            diversity_mode: false,
            diversity_penalty_increment: 0.1,
            diversity_decay_enabled: true,
            diversity_decay_amount: 0.01,
        }
    }
}

impl Settings {
    /// Load settings from the given TOML file path, or create a default file if missing.
    ///
    /// - If the file does not exist, creates it with documented defaults and returns `Settings::default()`.
    /// - If the file exists but contains malformed TOML, returns `AppError::SettingsParse`.
    /// - If the file parses successfully, returns the deserialized `Settings`.
    pub fn load_or_create(path: &Path) -> Result<Self, AppError> {
        if !path.exists() {
            // Create parent directories if needed
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| AppError::SettingsParse {
                    location: path.display().to_string(),
                    message: format!("failed to create directory: {}", e),
                })?;
            }
            fs::write(path, DEFAULT_SETTINGS_TOML).map_err(|e| AppError::SettingsParse {
                location: path.display().to_string(),
                message: format!("failed to write default settings: {}", e),
            })?;
            log::info!("Created default settings file at {}", path.display());
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(|e| AppError::SettingsParse {
            location: path.display().to_string(),
            message: format!("failed to read file: {}", e),
        })?;

        let settings: Settings = toml::from_str(&content).map_err(|e| {
            let location = if let Some(span) = e.span() {
                // Convert byte offset to line:column
                let line = content[..span.start].lines().count();
                let col = content[..span.start]
                    .lines()
                    .last()
                    .map(|l| l.len() + 1)
                    .unwrap_or(1);
                format!("{}:{}:{}", path.display(), line, col)
            } else {
                path.display().to_string()
            };
            AppError::SettingsParse {
                location,
                message: e.message().to_string(),
            }
        })?;

        Ok(settings)
    }

    /// Validate all parameter ranges. Returns `Ok(())` if all values are within bounds,
    /// or the first `AppError::SettingsValidation` encountered.
    pub fn validate(&self) -> Result<(), AppError> {
        self.validate_u32("batch_size", self.batch_size, 1, 4096)?;
        self.validate_u32("max_shapes", self.max_shapes, 1, 100_000)?;
        self.validate_u32("mutations_per_frame", self.mutations_per_frame, 1, 100)?;
        self.validate_u32("max_texture_size", self.max_texture_size, 16, 2048)?;
        self.validate_u32("vram_budget_mb", self.vram_budget_mb, 128, 4096)?;
        self.validate_f32("scale_min", self.scale_min, 0.01, 1.0)?;
        self.validate_f32("scale_max", self.scale_max, 0.1, 20.0)?;
        self.validate_u32("shape_resolution", self.shape_resolution, 16, 1024)?;
        self.validate_u32("mutations_per_shape", self.mutations_per_shape, 1, 50)?;
        self.validate_f32("displacement_weight", self.displacement_weight, 0.0, 100.0)?;
        self.validate_f32("scene_change_tolerance", self.scene_change_tolerance, -10.0, 10.0)?;
        self.validate_u32("interpolation_steps", self.interpolation_steps, 0, 20)?;
        self.validate_u32("target_fps", self.target_fps, 1, 60)?;
        self.validate_u32("num_generations", self.num_generations, 1, 20)?;
        self.validate_f32("min_improvement", self.min_improvement, -10000.0, 0.0)?;
        self.validate_u32("max_rejections", self.max_rejections, 1, 500)?;
        self.validate_f32("survival_rate", self.survival_rate, 0.01, 1.0)?;
        self.validate_u32("children_per_parent", self.children_per_parent, 1, 50)?;
        self.validate_f32("diversity_penalty_increment", self.diversity_penalty_increment, 0.0, 10.0)?;
        self.validate_f32("diversity_decay_amount", self.diversity_decay_amount, 0.0, 10.0)?;
        Ok(())
    }

    fn validate_u32(&self, name: &str, value: u32, min: u32, max: u32) -> Result<(), AppError> {
        if value < min || value > max {
            return Err(AppError::SettingsValidation {
                name: name.to_string(),
                value: value.to_string(),
                range: format!("{min}–{max}"),
            });
        }
        Ok(())
    }

    fn validate_f32(&self, name: &str, value: f32, min: f32, max: f32) -> Result<(), AppError> {
        if value < min || value > max || value.is_nan() {
            return Err(AppError::SettingsValidation {
                name: name.to_string(),
                value: value.to_string(),
                range: format!("{min}–{max}"),
            });
        }
        Ok(())
    }

    fn validate_u8(&self, name: &str, value: u8, min: u8, max: u8) -> Result<(), AppError> {
        if value < min || value > max {
            return Err(AppError::SettingsValidation {
                name: name.to_string(),
                value: value.to_string(),
                range: format!("{min}–{max}"),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_values() {
        let settings = Settings::default();
        assert_eq!(settings.batch_size, 1000);
        assert_eq!(settings.max_shapes, 4000);
        assert_eq!(settings.mutations_per_frame, 10);
        assert_eq!(settings.max_texture_size, 512);
        assert_eq!(settings.vram_budget_mb, 2048);
        assert!((settings.scale_min - 0.02).abs() < f32::EPSILON);
        assert!((settings.scale_max - 8.0).abs() < f32::EPSILON);
        assert_eq!(settings.shape_resolution, 128);
        assert_eq!(settings.mutations_per_shape, 10);
        assert!((settings.displacement_weight - 0.01).abs() < f32::EPSILON);
        assert_eq!(settings.interpolation_steps, 2);
    }

    #[test]
    fn test_validate_defaults_pass() {
        let settings = Settings::default();
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_validate_out_of_range_batch_size() {
        let mut settings = Settings::default();
        settings.batch_size = 5000;
        let err = settings.validate().unwrap_err();
        match err {
            AppError::SettingsValidation { name, value, range } => {
                assert_eq!(name, "batch_size");
                assert_eq!(value, "5000");
                assert!(range.contains("1"));
                assert!(range.contains("4096"));
            }
            _ => panic!("Expected SettingsValidation error"),
        }
    }

    #[test]
    fn test_validate_out_of_range_scale_min() {
        let mut settings = Settings::default();
        settings.scale_min = 0.001;
        let err = settings.validate().unwrap_err();
        match err {
            AppError::SettingsValidation { name, .. } => {
                assert_eq!(name, "scale_min");
            }
            _ => panic!("Expected SettingsValidation error"),
        }
    }

    #[test]
    fn test_load_or_create_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let settings = Settings::load_or_create(&path).unwrap();
        assert_eq!(settings, Settings::default());
        assert!(path.exists());
    }

    #[test]
    fn test_load_or_create_malformed_toml() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "batch_size = \"not a number\"").unwrap();
        let result = Settings::load_or_create(file.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::SettingsParse { location, message } => {
                assert!(!location.is_empty());
                assert!(!message.is_empty());
            }
            _ => panic!("Expected SettingsParse error"),
        }
    }

    #[test]
    fn test_load_or_create_partial_toml_uses_defaults() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "batch_size = 2048").unwrap();
        let settings = Settings::load_or_create(file.path()).unwrap();
        assert_eq!(settings.batch_size, 2048);
        // All other fields should be defaults
        assert_eq!(settings.max_shapes, 4000);
        assert_eq!(settings.mutations_per_frame, 10);
    }

    #[test]
    fn test_validate_nan_rejected() {
        let mut settings = Settings::default();
        settings.scale_min = f32::NAN;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn test_validate_boundary_values_pass() {
        let mut settings = Settings::default();
        // Test minimum boundary values
        settings.batch_size = 1;
        settings.max_shapes = 1;
        settings.mutations_per_frame = 1;
        settings.max_texture_size = 16;
        settings.vram_budget_mb = 128;
        settings.scale_min = 0.01;
        settings.scale_max = 0.1;
        settings.shape_resolution = 16;
        settings.mutations_per_shape = 1;
        settings.displacement_weight = 0.0;
        assert!(settings.validate().is_ok());

        // Test maximum boundary values
        settings.batch_size = 4096;
        settings.max_shapes = 100_000;
        settings.mutations_per_frame = 100;
        settings.max_texture_size = 2048;
        settings.vram_budget_mb = 4096;
        settings.scale_min = 1.0;
        settings.scale_max = 2.0;
        settings.shape_resolution = 1024;
        settings.mutations_per_shape = 50;
        settings.displacement_weight = 10.0;
        assert!(settings.validate().is_ok());
    }
}
