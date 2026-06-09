// Core data types: CandidateParams, PlacedShape, GenerationState, StepResult, ShapeLayer, EvalUniforms

/// GPU-compatible candidate shape parameters.
/// Each candidate represents a proposed shape placement with position, rotation, scale, and color.
/// Aligned to 64 bytes (multiple of 16) for efficient GPU storage/uniform buffer packing.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CandidateParams {
    /// Index into the shape texture array (0 to num_shapes - 1)
    pub shape_index: u32,
    /// X position on the canvas (pixels)
    pub x: f32,
    /// Y position on the canvas (pixels)
    pub y: f32,
    /// Rotation angle in radians
    pub rotation: f32,
    /// Scale factor for the shape along its local X axis
    pub scale: f32,
    /// Red channel (0.0–1.0)
    pub r: f32,
    /// Green channel (0.0–1.0)
    pub g: f32,
    /// Blue channel (0.0–1.0)
    pub b: f32,
    /// Alpha/opacity (0.1–1.0)
    pub alpha: f32,
    /// Scale factor for the shape along its local Y axis. Equals `scale` for
    /// uniform scaling; differs only when `evolve_non_uniform_scale` is enabled
    /// (lets shapes stretch/squash along a single axis).
    pub scale_y: f32,
    /// Whether to render the shape in its ORIGINAL texture colors (1.0) or to
    /// tint it by (r, g, b) using the shape's luminance (0.0). Set from the
    /// `use_original_colors` setting and inherited by all mutations.
    pub use_original_color: f32,
    /// Hue rotation applied to the shape's ORIGINAL colors, in turns [0.0, 1.0)
    /// (0.0 = no shift, 0.5 = +180°). Only meaningful in original-color mode and
    /// only evolved when `evolve_hue` is enabled; otherwise stays 0.0.
    pub hue_shift: f32,
    /// Saturation multiplier applied to the shape's ORIGINAL colors (1.0 = no
    /// change, 0.0 = greyscale, >1.0 = more vivid). Only meaningful in
    /// original-color mode and only evolved when `evolve_saturation` is enabled;
    /// otherwise stays 1.0.
    pub saturation_scale: f32,
    /// Brightness (value) multiplier applied to the shape's ORIGINAL colors
    /// (1.0 = no change, 0.0 = black, >1.0 = brighter). Only meaningful in
    /// original-color mode and only evolved when `evolve_brightness` is enabled;
    /// otherwise stays 1.0.
    pub brightness_scale: f32,
    /// Padding to keep the struct at 64 bytes (multiple of 16) for GPU buffer packing.
    pub _padding: [f32; 2],
}

/// A shape that has been placed on the canvas, with tracking for temporal coherence in video mode.
#[derive(Clone, Debug)]
pub struct PlacedShape {
    /// The parameters used to render this shape
    pub params: CandidateParams,
    /// The centroid position from the previous frame, used for displacement penalty calculation
    pub prev_centroid: (f32, f32),
}

/// The current state of the generation process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationState {
    /// Actively generating and evaluating candidates
    Running,
    /// Paused by user (Space key); canvas is displayed but no new iterations run
    Paused,
    /// Generation finished (max_shapes reached or video complete)
    Completed,
}

/// Result of a single algorithm step (one batch evaluation cycle).
#[derive(Clone, Debug)]
pub enum StepResult {
    /// A candidate was accepted and composited onto the canvas
    Accepted(CandidateParams),
    /// No candidate in the batch improved the MSE; batch discarded
    Rejected,
    /// The generation process has completed (max_shapes reached)
    Completed,
    /// An error occurred during the step
    Error(String),
}

/// A preprocessed shape layer ready for GPU upload.
/// Contains RGBA8 pixel data at the configured shape_resolution.
#[derive(Clone, Debug)]
pub struct ShapeLayer {
    /// Raw RGBA8 pixel data: shape_resolution × shape_resolution × 4 bytes
    pub pixels: Vec<u8>,
}

/// Uniform buffer data for the MSE evaluation compute shader.
/// Passed to the GPU as a uniform buffer each dispatch.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EvalUniforms {
    /// Width of the canvas/target texture in pixels
    pub canvas_width: u32,
    /// Height of the canvas/target texture in pixels
    pub canvas_height: u32,
    /// Number of candidates in the current batch
    pub num_candidates: u32,
    /// Resolution of each shape texture (width = height = shape_resolution)
    pub shape_resolution: u32,
    /// Weight for the displacement penalty in video mode temporal coherence
    pub displacement_weight: f32,
}
