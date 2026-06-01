// Application struct and main event loop using winit 0.30 ApplicationHandler trait

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::EventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::Window;

use crate::algorithm::{CandidateGenerator, HillClimber};
use crate::gpu::GpuContext;
use crate::io;
use crate::overlay::OverlayState;
use crate::settings::Settings;
use crate::types::{GenerationState, StepResult};

/// Compute window dimensions that fit within 90% of the display while preserving aspect ratio.
///
/// If the target fits within 90% of the display in both dimensions, the target size is used as-is.
/// Otherwise, the target is scaled down uniformly so that neither dimension exceeds 90% of the
/// corresponding display dimension.
///
/// # Arguments
/// * `target_size` - (width, height) of the target image/video in pixels
/// * `display_size` - (width, height) of the display/monitor in pixels
///
/// # Returns
/// The computed (width, height) for the window, preserving the target's aspect ratio.
pub fn compute_window_size(target_size: (u32, u32), display_size: (u32, u32)) -> (u32, u32) {
    let (tw, th) = target_size;
    let (dw, dh) = display_size;

    // Maximum allowed dimensions: 90% of display
    let max_w = (dw as f64 * 0.9).floor() as u32;
    let max_h = (dh as f64 * 0.9).floor() as u32;

    // If target already fits, use it as-is
    if tw <= max_w && th <= max_h {
        return (tw, th);
    }

    // Scale down preserving aspect ratio
    let scale_x = max_w as f64 / tw as f64;
    let scale_y = max_h as f64 / th as f64;
    let scale = scale_x.min(scale_y);

    let new_w = (tw as f64 * scale).floor() as u32;
    let new_h = (th as f64 * scale).floor() as u32;

    // Ensure at least 1×1
    (new_w.max(1), new_h.max(1))
}

/// Holds all application state for the GPU Image Approximator.
///
/// Manages the GPU context, algorithm state, overlay, egui integration,
/// and the winit event loop via the `ApplicationHandler` trait.
pub struct App {
    /// GPU resources: device, queue, textures, buffers, pipelines
    pub gpu: GpuContext,
    /// Hill climbing algorithm state
    pub climber: HillClimber,
    /// Candidate batch generator
    pub generator: CandidateGenerator,
    /// egui overlay state (statistics + notifications)
    pub overlay: OverlayState,
    /// Application settings
    pub settings: Settings,
    /// The winit window (wrapped in Arc for wgpu surface compatibility)
    pub window: Arc<Window>,
    /// Computed window dimensions (width, height)
    pub window_size: (u32, u32),
    /// The wgpu surface for presenting frames
    pub surface: wgpu::Surface<'static>,
    /// egui winit integration state
    pub egui_state: egui_winit::State,
    /// egui wgpu renderer
    pub egui_renderer: egui_wgpu::Renderer,
    /// egui context
    pub egui_ctx: egui::Context,
    /// Path to the source media file (for completion filename derivation)
    pub source_path: PathBuf,
    /// Path to the output folder
    pub output_folder: PathBuf,
    /// Whether auto-save on completion has been performed
    auto_saved: bool,
    /// Frame timing for FPS calculation
    frame_times: Vec<Instant>,
    /// Last frame timestamp
    last_frame: Instant,
    /// Video pipeline state (None for image mode)
    pub video_pipeline: Option<crate::algorithm::VideoPipeline>,
    /// Video decoder process (None for image mode)
    pub video_decoder: Option<crate::io::video::VideoProcessor>,
}

impl App {
    /// Create a new App with all components initialized.
    ///
    /// # Arguments
    /// * `gpu` - Initialized GPU context with all resources
    /// * `climber` - Hill climbing algorithm state
    /// * `generator` - Candidate batch generator
    /// * `overlay` - egui overlay state
    /// * `settings` - Application settings
    /// * `window` - The winit window
    /// * `surface` - The wgpu surface for presentation
    /// * `egui_state` - egui winit integration state
    /// * `egui_renderer` - egui wgpu renderer
    /// * `source_path` - Path to the source media file
    /// * `output_folder` - Path to the output folder
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gpu: GpuContext,
        climber: HillClimber,
        generator: CandidateGenerator,
        overlay: OverlayState,
        settings: Settings,
        window: Arc<Window>,
        window_size: (u32, u32),
        surface: wgpu::Surface<'static>,
        egui_state: egui_winit::State,
        egui_renderer: egui_wgpu::Renderer,
        source_path: PathBuf,
        output_folder: PathBuf,
    ) -> Self {
        let is_video = overlay.is_video;
        let egui_ctx = egui::Context::default();
        let now = Instant::now();

        // Configure the surface
        let size = window.inner_size();
        let config = gpu.create_surface_config(size.width, size.height, gpu.surface_format);
        surface.configure(&gpu.device, &config);

        Self {
            gpu,
            climber,
            generator,
            overlay,
            settings,
            window,
            window_size,
            surface,
            egui_state,
            egui_renderer,
            egui_ctx,
            source_path,
            output_folder,
            auto_saved: false,
            frame_times: Vec::with_capacity(60),
            last_frame: now,
            video_pipeline: if is_video { Some(crate::algorithm::VideoPipeline::new()) } else { None },
            video_decoder: None,
        }
    }

    /// Run the application event loop. Consumes self and the event loop.
    pub fn run(self, event_loop: EventLoop<()>) {
        let mut app_handler = AppHandler {
            app: Some(self),
        };
        event_loop.run_app(&mut app_handler).unwrap();
    }

    /// Compute rolling average FPS from recent frame times.
    fn compute_fps(&mut self) -> f32 {
        let now = Instant::now();
        self.frame_times.push(now);

        // Keep only the last 60 frame timestamps for rolling average
        if self.frame_times.len() > 60 {
            self.frame_times.remove(0);
        }

        if self.frame_times.len() < 2 {
            return 0.0;
        }

        let oldest = self.frame_times[0];
        let elapsed = now.duration_since(oldest).as_secs_f32();
        if elapsed > 0.0 {
            (self.frame_times.len() - 1) as f32 / elapsed
        } else {
            0.0
        }
    }

    /// Execute generation iterations for the current frame.
    ///
    /// Runs up to `mutations_per_frame` successful iterations (Accepted results).
    /// Skips generation if paused or completed.
    /// Auto-saves on completion (max_shapes reached).
    fn run_generation_step(&mut self) {
        if self.climber.state != GenerationState::Running {
            return;
        }

        let mutations_per_frame = self.settings.mutations_per_frame;
        let mut accepted_count = 0u32;

        // Keep stepping until we get the required number of accepted mutations
        // or hit a completion/error state. Limit total attempts to avoid infinite loops.
        let max_attempts = mutations_per_frame * 100; // Safety limit
        let mut attempts = 0u32;

        while accepted_count < mutations_per_frame && attempts < max_attempts {
            let result = self.climber.step(&self.gpu, &mut self.generator, &self.settings);
            attempts += 1;

            match result {
                StepResult::Accepted(candidate) => {
                    accepted_count += 1;
                    // Record shape in video pipeline for temporal coherence
                    if let Some(ref mut pipeline) = self.video_pipeline {
                        pipeline.record_placed_shape(candidate);
                    }
                    if self.climber.placed_shapes <= 5 {
                        log::info!(
                            "Shape #{}: pos=({:.0},{:.0}) scale={:.3} color=({:.2},{:.2},{:.2}) alpha={:.2}",
                            self.climber.placed_shapes, candidate.x, candidate.y,
                            candidate.scale, candidate.r, candidate.g, candidate.b, candidate.alpha
                        );
                    }
                }
                StepResult::Rejected => {
                    // Continue trying
                }
                StepResult::Completed => {
                    // Generation finished — auto-save
                    self.auto_save_on_completion();
                    break;
                }
                StepResult::Error(msg) => {
                    log::error!("Generation step error: {}", msg);
                    self.overlay.add_notification(
                        format!("Generation error: {}", msg),
                        5.0,
                        true,
                    );
                    break;
                }
            }
        }
    }

    /// Auto-save the canvas when generation completes (max_shapes reached).
    fn auto_save_on_completion(&mut self) {
        if self.auto_saved {
            return;
        }

        // Check if we're in video mode and have more frames
        if self.overlay.is_video {
            self.handle_video_frame_complete();
            return;
        }

        // Image mode: just save
        self.auto_saved = true;
        self.window.set_title("GPU Image Approximator - Completed");

        let filename = io::output::completion_filename(&self.source_path);
        match io::output::save_canvas_png(&self.gpu, &self.output_folder, &filename) {
            Ok(path) => {
                log::info!("Auto-save on completion: {}", path.display());
                self.overlay.add_notification(
                    format!("Completed! Saved: {}", filename),
                    5.0,
                    false,
                );
            }
            Err(e) => {
                log::error!("Auto-save failed: {}", e);
                self.overlay.add_notification(
                    format!("Auto-save failed: {}", e),
                    5.0,
                    true,
                );
            }
        }
    }

    /// Handle video frame completion: save current frame, advance to next.
    fn handle_video_frame_complete(&mut self) {
        // Save current frame as PNG
        let frame_num = self.overlay.frame_number;
        log::info!(
            "Frame {} complete with {} shapes in pipeline",
            frame_num,
            self.video_pipeline.as_ref().map(|p| p.shapes.len()).unwrap_or(0)
        );
        let filename = format!("frame_{:05}.png", frame_num);
        match io::output::save_canvas_png(&self.gpu, &self.output_folder, &filename) {
            Ok(path) => {
                log::info!("Saved video frame: {}", path.display());
            }
            Err(e) => {
                log::error!("Failed to save frame {}: {}", frame_num, e);
            }
        }

        // Try to load next frame from video decoder
        if let Some(ref mut decoder) = self.video_decoder {
            if let Some(frame_data) = decoder.next_frame() {
                // Update target texture with new frame
                self.gpu.queue.write_texture(
                    wgpu::ImageCopyTexture {
                        texture: &self.gpu.target,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &frame_data,
                    wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * self.gpu.canvas_size.0),
                        rows_per_image: Some(self.gpu.canvas_size.1),
                    },
                    wgpu::Extent3d {
                        width: self.gpu.canvas_size.0,
                        height: self.gpu.canvas_size.1,
                        depth_or_array_layers: 1,
                    },
                );

                // Update generator's target pixels for color sampling
                self.generator = CandidateGenerator::new(
                    self.settings.clone(),
                    frame_data,
                    self.gpu.canvas_size,
                );

                // Adapt existing shapes to new frame
                if let Some(ref mut pipeline) = self.video_pipeline {
                    let dead = pipeline.adapt_to_new_frame(&self.gpu, &mut self.generator, &self.settings);
                    log::info!("Frame {}: {} shapes died, {} remain", frame_num + 1, dead, pipeline.shapes.len());

                    // Rebuild canvas from adapted shapes
                    pipeline.rebuild_canvas(&self.gpu);
                }

                // Reset climber for new shapes to fill gaps
                self.climber = HillClimber::new();
                // Pass surviving shape count so climber only generates missing shapes
                if let Some(ref pipeline) = self.video_pipeline {
                    self.climber.placed_shapes = pipeline.shapes.len() as u32;
                    log::info!(
                        "Frame {}: {} shapes survived, will generate up to {} new ones",
                        self.overlay.frame_number + 1,
                        pipeline.shapes.len(),
                        self.settings.max_shapes.saturating_sub(pipeline.shapes.len() as u32)
                    );
                }
                self.overlay.frame_number += 1;
                self.overlay.placed_shapes = self.climber.placed_shapes;

                self.window.set_title(&format!(
                    "GPU Image Approximator - Frame {} - Running",
                    self.overlay.frame_number
                ));
            } else {
                // No more frames — video complete
                self.auto_saved = true;
                self.climber.state = crate::types::GenerationState::Completed;
                self.window.set_title("GPU Image Approximator - Video Complete");
                self.overlay.add_notification("Video processing complete!".to_string(), 10.0, false);

                // Encode all frames to MP4
                log::info!("All frames processed, encoding to MP4...");
                let stem = self.source_path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("video");
                let output_mp4 = self.output_folder.join(format!("{}_result.mp4", stem));

                let fps = if let Some(ref decoder) = self.video_decoder {
                    decoder.fps
                } else {
                    30.0
                };

                // Use FFmpeg to encode saved frames to MP4
                let frames_pattern = self.output_folder.join("frame_%05d.png");
                let encode_result = std::process::Command::new("ffmpeg")
                    .args([
                        "-y",
                        "-framerate", &format!("{}", fps),
                        "-i", frames_pattern.to_str().unwrap_or(""),
                        "-c:v", "libx264",
                        "-pix_fmt", "yuv420p",
                        "-v", "quiet",
                        output_mp4.to_str().unwrap_or(""),
                    ])
                    .output();

                match encode_result {
                    Ok(output) if output.status.success() => {
                        log::info!("Video encoded: {}", output_mp4.display());
                        self.overlay.add_notification(
                            format!("Video saved: {}", output_mp4.display()),
                            10.0,
                            false,
                        );
                    }
                    _ => {
                        log::error!("FFmpeg encoding failed");
                        self.overlay.add_notification(
                            "FFmpeg encoding failed".to_string(),
                            5.0,
                            true,
                        );
                    }
                }
            }
        } else {
            // No decoder — shouldn't happen in video mode
            self.auto_saved = true;
            self.climber.state = crate::types::GenerationState::Completed;
        }
    }

    /// Render a frame: copy canvas to surface, render egui overlay, present.
    fn render_frame(&mut self) {
        // Compute FPS
        let fps = self.compute_fps();
        self.overlay.fps = fps;

        // Update overlay state from algorithm
        self.overlay.placed_shapes = self.climber.placed_shapes;
        self.overlay.current_mse = self.climber.current_mse;

        // Get surface texture
        let surface_texture = match self.surface.get_current_texture() {
            Ok(tex) => tex,
            Err(wgpu::SurfaceError::Lost) => {
                // Reconfigure surface
                let size = self.window.inner_size();
                let config = self.gpu.create_surface_config(size.width, size.height, self.gpu.surface_format);
                self.surface.configure(&self.gpu.device, &config);
                return;
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                log::error!("Out of GPU memory for surface texture");
                return;
            }
            Err(e) => {
                log::warn!("Surface texture error: {:?}", e);
                return;
            }
        };

        // Create command encoder
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Frame Encoder"),
        });

        // Blit canvas to surface texture (handles format conversion)
        let surface_view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu.blit_canvas_to_surface(&mut encoder, &surface_view);

        // Run egui frame
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            self.overlay.render(ctx);
        });

        // Handle egui platform output (cursor changes, etc.)
        self.egui_state.handle_platform_output(&self.window, full_output.platform_output);

        // Tessellate egui shapes
        let paint_jobs = self.egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);

        // Update egui textures
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [
                self.window.inner_size().width,
                self.window.inner_size().height,
            ],
            pixels_per_point: full_output.pixels_per_point,
        };

        for (id, image_delta) in &full_output.textures_delta.set {
            self.egui_renderer.update_texture(&self.gpu.device, &self.gpu.queue, *id, image_delta);
        }

        self.egui_renderer.update_buffers(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        // Render egui overlay on top of the surface
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Keep the canvas content
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // egui_wgpu 0.30 requires RenderPass<'static> — use forget_lifetime()
            // to opt into runtime borrow checking instead of compile-time.
            let mut render_pass = render_pass.forget_lifetime();
            self.egui_renderer.render(&mut render_pass, &paint_jobs, &screen_descriptor);
        }

        // Submit and present
        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        surface_texture.present();

        // Free egui textures that are no longer needed
        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

/// Wrapper struct that implements `ApplicationHandler` for winit 0.30.
///
/// This is needed because `ApplicationHandler::resumed` requires `&mut self`,
/// and we need to own the `App` to pass it into the event loop.
struct AppHandler {
    app: Option<App>,
}

impl ApplicationHandler for AppHandler {
    fn resumed(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        // Window is already created before entering the event loop.
        // Nothing to do here for our use case.
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(app) = self.app.as_mut() else {
            return;
        };

        // Pass events to egui first
        let egui_response = app.egui_state.on_window_event(&app.window, &event);
        if egui_response.consumed {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                log::info!("Close requested, shutting down");
                // Ensure generation is stopped
                if let Some(app) = self.app.as_mut() {
                    app.climber.state = GenerationState::Completed;
                }
                event_loop.exit();
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    logical_key,
                    state: ElementState::Pressed,
                    ..
                },
                ..
            } => {
                match logical_key {
                    Key::Named(NamedKey::Space) => {
                        // Toggle pause/running
                        match app.climber.state {
                            GenerationState::Running => {
                                app.climber.state = GenerationState::Paused;
                                app.window.set_title("GPU Image Approximator - Paused");
                                log::info!("Generation paused");
                            }
                            GenerationState::Paused => {
                                app.climber.state = GenerationState::Running;
                                app.window.set_title("GPU Image Approximator - Running");
                                log::info!("Generation resumed");
                            }
                            GenerationState::Completed => {
                                // Cannot unpause when completed
                            }
                        }
                    }
                    Key::Character(ref c) if c.as_str() == "s" || c.as_str() == "S" => {
                        // Snapshot save
                        log::info!("Snapshot save requested (S key)");
                        let filename = io::output::snapshot_filename();
                        match io::output::save_canvas_png(
                            &app.gpu,
                            &app.output_folder,
                            &filename,
                        ) {
                            Ok(path) => {
                                log::info!("Snapshot saved: {}", path.display());
                                app.overlay.add_notification(
                                    format!("Saved: {}", filename),
                                    3.0,
                                    false,
                                );
                            }
                            Err(e) => {
                                log::error!("Snapshot save failed: {}", e);
                                app.overlay.add_notification(
                                    format!("Save failed: {}", e),
                                    5.0,
                                    true,
                                );
                            }
                        }
                    }
                    Key::Named(NamedKey::Escape) => {
                        log::info!("Escape pressed, shutting down");
                        app.climber.state = GenerationState::Completed;
                        event_loop.exit();
                    }
                    _ => {}
                }
            }

            WindowEvent::Resized(new_size) => {
                if new_size.width > 0 && new_size.height > 0 {
                    let config = app.gpu.create_surface_config(new_size.width, new_size.height, app.gpu.surface_format);
                    app.surface.configure(&app.gpu.device, &config);
                }
            }

            WindowEvent::RedrawRequested => {
                // Run generation iterations (skipped if paused/completed)
                app.run_generation_step();

                // Always render the frame (canvas + overlay)
                app.render_frame();

                // Request next frame for continuous 60 FPS rendering
                app.window.request_redraw();
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        // Request redraw to maintain 60 FPS rendering loop
        if let Some(app) = self.app.as_ref() {
            app.window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_fits_within_display() {
        // Target is smaller than 90% of display in both dimensions
        let result = compute_window_size((800, 600), (1920, 1080));
        assert_eq!(result, (800, 600));
    }

    #[test]
    fn test_target_exceeds_width() {
        // Target width exceeds 90% of display width (1920 * 0.9 = 1728)
        let result = compute_window_size((2000, 500), (1920, 1080));
        // max_w = 1728, scale = 1728/2000 = 0.864
        // new_w = floor(2000 * 0.864) = 1728, new_h = floor(500 * 0.864) = 432
        assert_eq!(result.0, 1728);
        assert_eq!(result.1, 432);
    }

    #[test]
    fn test_target_exceeds_height() {
        // Target height exceeds 90% of display height (1080 * 0.9 = 972)
        let result = compute_window_size((500, 1200), (1920, 1080));
        // max_h = 972, scale = 972/1200 = 0.81
        // new_w = floor(500 * 0.81) = 405, new_h = floor(1200 * 0.81) = 972
        assert_eq!(result.0, 405);
        assert_eq!(result.1, 972);
    }

    #[test]
    fn test_target_exceeds_both_dimensions() {
        // Both dimensions exceed 90% of display
        let result = compute_window_size((3000, 2000), (1920, 1080));
        // max_w = 1728, max_h = 972
        // scale_x = 1728/3000 = 0.576, scale_y = 972/2000 = 0.486
        // scale = 0.486 (height is the limiting factor)
        // new_w = floor(3000 * 0.486) = 1458, new_h = floor(2000 * 0.486) = 972
        assert_eq!(result.0, 1458);
        assert_eq!(result.1, 972);
        // Neither dimension exceeds 90% of display
        assert!(result.0 <= 1728);
        assert!(result.1 <= 972);
    }

    #[test]
    fn test_exact_90_percent_boundary() {
        // Target is exactly 90% of display
        let result = compute_window_size((1728, 972), (1920, 1080));
        assert_eq!(result, (1728, 972));
    }

    #[test]
    fn test_small_target_on_large_display() {
        let result = compute_window_size((100, 100), (3840, 2160));
        assert_eq!(result, (100, 100));
    }

    #[test]
    fn test_square_target_exceeding_display() {
        // Square target larger than display
        let result = compute_window_size((2000, 2000), (1920, 1080));
        // max_w = 1728, max_h = 972
        // scale_x = 1728/2000 = 0.864, scale_y = 972/2000 = 0.486
        // scale = 0.486
        // new_w = floor(2000 * 0.486) = 972, new_h = floor(2000 * 0.486) = 972
        assert_eq!(result.0, 972);
        assert_eq!(result.1, 972);
        assert!(result.0 <= 1728);
        assert!(result.1 <= 972);
    }

    #[test]
    fn test_aspect_ratio_preserved() {
        // Verify aspect ratio is preserved within tolerance
        let result = compute_window_size((1600, 900), (1000, 800));
        // max_w = 900, max_h = 720
        // scale_x = 900/1600 = 0.5625, scale_y = 720/900 = 0.8
        // scale = 0.5625
        // new_w = floor(1600 * 0.5625) = 900, new_h = floor(900 * 0.5625) = 506
        let original_ratio = 1600.0_f64 / 900.0;
        let result_ratio = result.0 as f64 / result.1 as f64;
        // Within 1 pixel tolerance (as per Property 4 in design)
        assert!((original_ratio - result_ratio).abs() < 0.01);
        assert!(result.0 <= 900);
        assert!(result.1 <= 720);
    }
}
