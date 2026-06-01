use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("GPU initialization failed: {0}")]
    GpuInit(String),

    #[error("Settings error: parameter '{name}' has value {value}, expected range {range}")]
    SettingsValidation {
        name: String,
        value: String,
        range: String,
    },

    #[error("Settings parse error at {location}: {message}")]
    SettingsParse { location: String, message: String },

    #[error("No shape files found in {path}")]
    NoShapes { path: PathBuf },

    #[error("VRAM budget exceeded: need {required_mb} MB, budget is {budget_mb} MB")]
    VramBudget { required_mb: u32, budget_mb: u32 },

    #[error("No supported media file in {path}")]
    NoMedia { path: PathBuf },

    #[error("GPU compute timeout after {timeout_ms} ms")]
    ComputeTimeout { timeout_ms: u64 },

    #[error("Save failed: {reason}")]
    SaveFailed { reason: String },

    #[error("FFmpeg error: {0}")]
    Ffmpeg(String),
}
