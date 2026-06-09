// Settings module: TOML loading, validation, and defaults

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Default TOML content with inline comments documenting each parameter.
const DEFAULT_SETTINGS_TOML: &str = r#"# GPU Image Approximator Configuration

# Number of candidates in initial population (1–4096)
batch_size = 1000

# Maximum shapes to place before stopping (1–1000000)
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

# Use the shapes' ORIGINAL colors instead of tinting them (true/false). Default false.
# When false (default), shapes are loaded from input_shapes/ as grayscale brushes and
# tinted with colors sampled from the target image (classic behaviour). When true,
# shapes are loaded from raw_shapes/ KEEPING their original RGB colors — the algorithm
# never recolors them, it only places/moves/rotates/scales them as-is. Put your colored
# PNG shapes (ideally with transparency) into a raw_shapes/ folder next to the executable.
use_original_colors = false

# Allow NON-UNIFORM (per-axis) scaling of shapes (true/false). Default false.
# When false (default), a shape's width and height scale together (uniform zoom).
# When true, a shape can scale independently along its X and Y axes, so it can stretch
# and squash (e.g. a circle can become an ellipse, a square a rectangle) during evolution.
evolve_non_uniform_scale = false

# --- Real-color mode evolution (only used when use_original_colors = true) ---

# Evolve the HUE of the shapes' original colors (true/false). Default false.
# Only has an effect in real-color mode (use_original_colors = true). When true,
# each shape may rotate the hue of its original texture colors to better match the
# target, searching the full color wheel while keeping the shape's brightness.
evolve_hue = false

# Evolve the SATURATION of the shapes' original colors (true/false). Default false.
# Only has an effect in real-color mode (use_original_colors = true). When true,
# each shape may scale the saturation of its original texture colors (toward
# greyscale or more vivid) to better match the target.
evolve_saturation = false

# Evolve the BRIGHTNESS (value) of the shapes' original colors (true/false). Default false.
# Only has an effect in real-color mode (use_original_colors = true). When true,
# each shape may darken or brighten its original texture colors to better match the target.
evolve_brightness = false

# Whether existing shapes RE-COLOR themselves to match the new frame during video
# adaptation (true) or keep their original color and only move/rotate/scale (false).
# Default is false: a shape placed on frame 1 keeps its color for the whole video and
# only its position/rotation/scale adapt. Set true for the old behaviour where shapes
# resample the new frame's colors as they adapt. (Reborn shapes filling gaps always
# take a fresh color regardless, since they have no previous color.)
video_recolor = false

# Preserve the original audio track of the source video in the rendered MP4
# (true/false). Default true. If the source has no audio this is a no-op.
preserve_audio = true

# --- Progress GIF (image mode only) ---

# Save an animated GIF of the creation process next to the result image (true/false).
# Default false. Only applies when the input is an image: as shapes are placed, the
# canvas is periodically captured and assembled into <name>_process.gif.
save_progress_gif = false

# Playback speed of the progress GIF in frames per second (1–50).
gif_fps = 20

# Approximate number of frames captured for the progress GIF (2–2000). Captures are
# spread evenly across the shape-placement process.
gif_frames = 120

# Maximum width (px) of the progress GIF; larger canvases are downscaled to keep the
# file small (16–2048). Height scales to preserve aspect ratio.
gif_max_width = 480

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
    /// Maximum shapes to place before stopping (1–1000000).
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
    /// Use the shapes' original RGB colors (true) instead of tinting grayscale brushes (false).
    /// When true, shapes are loaded from `raw_shapes/` keeping their colors and are never recolored.
    pub use_original_colors: bool,
    /// Allow non-uniform (per-axis) scaling so shapes can stretch/squash along a single axis (true)
    /// or only scale uniformly (false).
    pub evolve_non_uniform_scale: bool,
    /// Evolve the hue of shapes' original colors (only in real-color mode). Default false.
    pub evolve_hue: bool,
    /// Evolve the saturation of shapes' original colors (only in real-color mode). Default false.
    pub evolve_saturation: bool,
    /// Evolve the brightness (value) of shapes' original colors (only in real-color mode). Default false.
    pub evolve_brightness: bool,
    /// Whether existing shapes re-color to the new frame during video adaptation (true)
    /// or keep their original color and only adapt geometry (false). Default false.
    pub video_recolor: bool,
    /// Preserve the source video's audio track in the rendered MP4 (true) if present.
    pub preserve_audio: bool,
    /// Save an animated GIF of the creation process (image mode only).
    pub save_progress_gif: bool,
    /// Progress GIF playback speed in frames per second (1–50).
    pub gif_fps: u32,
    /// Approximate number of frames captured for the progress GIF (2–2000).
    pub gif_frames: u32,
    /// Maximum width (px) of the progress GIF; larger canvases are downscaled (16–2048).
    pub gif_max_width: u32,
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
            use_original_colors: false,
            evolve_non_uniform_scale: false,
            evolve_hue: false,
            evolve_saturation: false,
            evolve_brightness: false,
            video_recolor: false,
            preserve_audio: true,
            save_progress_gif: false,
            gif_fps: 20,
            gif_frames: 120,
            gif_max_width: 480,
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

    /// Serialize these settings to a TOML file, overwriting any existing file.
    ///
    /// Unlike the documented `DEFAULT_SETTINGS_TOML`, this writes a plain
    /// serialized form (all fields present, no comments). Used by the Settings
    /// UI so that changing values in the form persists to `settings.toml`.
    pub fn save(&self, path: &Path) -> Result<(), AppError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::SaveFailed {
                reason: format!("failed to create settings directory: {}", e),
            })?;
        }
        let toml = toml::to_string_pretty(self).map_err(|e| AppError::SaveFailed {
            reason: format!("failed to serialize settings: {}", e),
        })?;
        fs::write(path, toml).map_err(|e| AppError::SaveFailed {
            reason: format!("failed to write settings file: {}", e),
        })?;
        Ok(())
    }

    /// Validate all parameter ranges. Returns `Ok(())` if all values are within bounds,
    /// or the first `AppError::SettingsValidation` encountered.
    pub fn validate(&self) -> Result<(), AppError> {
        self.validate_u32("batch_size", self.batch_size, 1, 4096)?;
        self.validate_u32("max_shapes", self.max_shapes, 1, 1_000_000)?;
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
        self.validate_u32("gif_fps", self.gif_fps, 1, 50)?;
        self.validate_u32("gif_frames", self.gif_frames, 2, 2000)?;
        self.validate_u32("gif_max_width", self.gif_max_width, 16, 2048)?;
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

/// Sanitize a user-supplied preset name into a safe file stem.
/// Keeps alphanumerics, spaces, dashes and underscores; trims the result.
pub fn sanitize_preset_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cleaned.trim().to_string()
}

/// List the names (file stems) of all settings presets stored as `*.toml`
/// files in `dir`, sorted case-insensitively. Returns an empty list if the
/// directory does not exist.
pub fn list_presets(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("toml"))
                        .unwrap_or(false)
            })
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Load a named preset from `dir`.
pub fn load_preset(dir: &Path, name: &str) -> Result<Settings, AppError> {
    let path = dir.join(format!("{}.toml", name));
    Settings::load_or_create(&path).and_then(|s| {
        // load_or_create would silently create a default file for a missing
        // preset; guard against that by validating existence first.
        if path.exists() {
            Ok(s)
        } else {
            Err(AppError::SettingsParse {
                location: path.display().to_string(),
                message: "preset not found".to_string(),
            })
        }
    })
}

/// Save `settings` as a named preset inside `dir` (created if missing).
pub fn save_preset(dir: &Path, name: &str, settings: &Settings) -> Result<(), AppError> {
    let name = sanitize_preset_name(name);
    if name.is_empty() {
        return Err(AppError::SaveFailed {
            reason: "preset name is empty".to_string(),
        });
    }
    fs::create_dir_all(dir).map_err(|e| AppError::SaveFailed {
        reason: format!("failed to create presets directory: {}", e),
    })?;
    settings.save(&dir.join(format!("{}.toml", name)))
}

/// Delete a named preset from `dir`.
pub fn delete_preset(dir: &Path, name: &str) -> Result<(), AppError> {
    let path = dir.join(format!("{}.toml", name));
    fs::remove_file(&path).map_err(|e| AppError::SaveFailed {
        reason: format!("failed to delete preset '{}': {}", name, e),
    })
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
