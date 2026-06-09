// Application struct and main event loop using winit 0.30 ApplicationHandler trait.
//
// The app runs in a single window with two screens:
//   * Screen::Settings    — the egui configuration form (start state)
//   * Screen::Generation  — the live approximation + statistics overlay
//
// Pressing "Start" in the Settings screen loads the chosen media + shapes,
// builds the GPU resources on the already-created device, and switches to the
// Generation screen. The window + surface + egui are shared across both.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::EventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::Window;

use crate::algorithm::{CandidateGenerator, HillClimber};
use crate::error::AppError;
use crate::gpu::GpuContext;
use crate::io;
use crate::overlay::OverlayState;
use crate::settings::Settings;
use crate::types::{GenerationState, StepResult};
use crate::ui::{Language, ScreenAction, SettingsScreen};

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

/// Build a surface configuration for the given size/format (device-independent).
fn make_surface_config(
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::SurfaceConfiguration {
    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: width.max(1),
        height: height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
    }
}

/// The two screens the window can display.
enum Screen {
    /// Configuration form (start state).
    Settings(SettingsScreen),
    /// Live generation + overlay.
    Generation(Box<GenerationContext>),
}

/// All state that only exists while generation is running.
pub struct GenerationContext {
    pub gpu: GpuContext,
    pub climber: HillClimber,
    pub generator: CandidateGenerator,
    pub overlay: OverlayState,
    pub settings: Settings,
    pub source_path: PathBuf,
    pub output_folder: PathBuf,
    auto_saved: bool,
    pub video_pipeline: Option<crate::algorithm::VideoPipeline>,
    pub video_decoder: Option<crate::io::video::VideoProcessor>,
    /// Index of the next output frame file (video PNG sequence).
    output_frame_index: u32,
    /// Captured (downscaled) frames for the image-mode progress GIF.
    gif_frames: Vec<image::RgbaImage>,
    /// Placed-shape count at which the next GIF frame should be captured.
    gif_next_capture: u32,
    /// Placed-shape interval between GIF captures.
    gif_capture_stride: u32,
}

/// Holds all shared application state and the active screen.
pub struct App {
    /// Shared GPU device (cloned into GpuContext on Start).
    device: Arc<wgpu::Device>,
    /// Shared GPU queue.
    queue: Arc<wgpu::Queue>,
    /// The wgpu surface for presenting frames.
    surface: wgpu::Surface<'static>,
    /// Surface texture format.
    surface_format: wgpu::TextureFormat,
    /// The winit window (Arc for wgpu surface compatibility).
    window: Arc<Window>,
    /// egui winit integration state.
    egui_state: egui_winit::State,
    /// egui wgpu renderer.
    egui_renderer: egui_wgpu::Renderer,
    /// egui context.
    egui_ctx: egui::Context,
    /// Base directory (for shapes, output, presets).
    base_dir: PathBuf,
    /// Output folder.
    output_folder: PathBuf,
    /// Active screen.
    screen: Screen,
    /// Frame timing for FPS calculation.
    frame_times: Vec<Instant>,
}

impl App {
    /// Create a new App starting on the Settings screen.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        surface: wgpu::Surface<'static>,
        surface_format: wgpu::TextureFormat,
        window: Arc<Window>,
        egui_state: egui_winit::State,
        egui_renderer: egui_wgpu::Renderer,
        egui_ctx: egui::Context,
        base_dir: PathBuf,
        settings: Settings,
        language: Language,
    ) -> Self {
        // Configure the surface for the initial (settings) window size.
        let size = window.inner_size();
        surface.configure(&device, &make_surface_config(size.width, size.height, surface_format));

        let settings_screen = SettingsScreen::new(&base_dir, settings, language);
        let output_folder = base_dir.join("output");

        Self {
            device,
            queue,
            surface,
            surface_format,
            window,
            egui_state,
            egui_renderer,
            egui_ctx,
            base_dir,
            output_folder,
            screen: Screen::Settings(settings_screen),
            frame_times: Vec::with_capacity(60),
        }
    }

    /// Run the application event loop. Consumes self and the event loop.
    pub fn run(self, event_loop: EventLoop<()>) {
        let mut app_handler = AppHandler { app: Some(self) };
        event_loop.run_app(&mut app_handler).unwrap();
    }

    /// Compute rolling average FPS from recent frame times.
    fn compute_fps(&mut self) -> f32 {
        let now = Instant::now();
        self.frame_times.push(now);
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

    /// Reconfigure the surface for a new size.
    fn reconfigure_surface(&self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface
            .configure(&self.device, &make_surface_config(width, height, self.surface_format));
    }

    /// Transition from Settings to Generation: load media + shapes, build GPU
    /// resources on the shared device, and assemble the GenerationContext.
    fn begin_generation(
        &mut self,
        media_path: PathBuf,
        settings: Settings,
        language: Language,
    ) -> Result<GenerationContext, AppError> {
        // 1. Load the target frame (image, or first frame of a video).
        let (target_data, target_size, is_video) = io::media_loader::load_target(&media_path)?;
        log::info!(
            "Media loaded: {}x{}, is_video={}",
            target_size.0,
            target_size.1,
            is_video
        );

        // 2. Resize the window to fit the media within 90% of the display.
        let display_size = self
            .window
            .current_monitor()
            .map(|m| {
                let s = m.size();
                (s.width, s.height)
            })
            .unwrap_or((1920, 1080));
        let win_size = compute_window_size(target_size, display_size);
        let _ = self
            .window
            .request_inner_size(winit::dpi::PhysicalSize::new(win_size.0, win_size.1));
        self.reconfigure_surface(win_size.0, win_size.1);

        // 3. Load and preprocess shapes (raw_shapes/ keeps original colors).
        let (shapes_folder, preserve_color) = if settings.use_original_colors {
            (self.base_dir.join("raw_shapes"), true)
        } else {
            (self.base_dir.join("input_shapes"), false)
        };
        log::info!(
            "Loading shapes from '{}' (original colors: {})",
            shapes_folder.display(),
            settings.use_original_colors
        );
        let mut shapes = io::shape_preprocessor::load_and_preprocess(
            &shapes_folder,
            settings.shape_resolution,
            preserve_color,
        )?;
        // Each shape occupies one layer of a GPU texture array, so the count is
        // hard-bounded by the device's `max_texture_array_layers`. Clamp here to
        // avoid a device error when more brushes were prepared than the GPU can
        // hold as array layers.
        let max_layers = self.device.limits().max_texture_array_layers as usize;
        if shapes.len() > max_layers {
            log::warn!(
                "Loaded {} shapes but GPU supports only {} texture array layers; \
                 using the first {}.",
                shapes.len(),
                max_layers,
                max_layers
            );
            shapes.truncate(max_layers);
        }
        io::shape_preprocessor::check_vram_budget(
            settings.shape_resolution,
            shapes.len() as u32,
            settings.vram_budget_mb,
        )?;
        log::info!("Loaded {} shape(s)", shapes.len());

        // 4. Build GPU resources on the shared device.
        let gpu = GpuContext::new_from_device(
            self.device.clone(),
            self.queue.clone(),
            self.surface_format,
            &target_data,
            target_size,
            &shapes,
            &settings,
        )?;

        // 5. Algorithm components.
        let climber = HillClimber::new();
        let generator = CandidateGenerator::new(settings.clone(), target_data, target_size);
        let overlay = OverlayState::with_language(settings.max_shapes, is_video, language);

        let mut ctx = GenerationContext {
            gpu,
            climber,
            generator,
            overlay,
            settings: settings.clone(),
            source_path: media_path.clone(),
            output_folder: self.output_folder.clone(),
            auto_saved: false,
            video_pipeline: if is_video {
                Some(crate::algorithm::VideoPipeline::new())
            } else {
                None
            },
            video_decoder: None,
            output_frame_index: 0,
            gif_frames: Vec::new(),
            // Spread ~gif_frames captures across the placement process. Only
            // collect for image mode when the toggle is on.
            gif_next_capture: if !is_video && settings.save_progress_gif {
                (settings.max_shapes / settings.gif_frames.max(1)).max(1)
            } else {
                u32::MAX
            },
            gif_capture_stride: (settings.max_shapes / settings.gif_frames.max(1)).max(1),
        };

        // 6. Video decoder setup.
        if is_video {
            let removed = io::output::clean_frame_sequence(&self.output_folder);
            if removed > 0 {
                log::info!("Removed {} leftover frame_*.png file(s) from a previous run", removed);
            }
            let stem = media_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("video");
            let output_video = self.output_folder.join(format!("{}_result.mp4", stem));
            match crate::io::video::VideoProcessor::new(&media_path, &output_video, settings.target_fps) {
                Ok(decoder) => {
                    ctx.video_decoder = Some(decoder);
                    log::info!("Video decoder initialized");
                }
                Err(e) => {
                    log::warn!("Failed to init video decoder: {}. Processing as single image.", e);
                }
            }
        }

        self.window
            .set_title("TeekasFigure - Running  (Space: pause, S: snapshot, Esc: quit)");
        Ok(ctx)
    }

    /// Render a frame for whichever screen is active.
    fn render_frame(&mut self) {
        let fps = self.compute_fps();
        if let Screen::Generation(ref mut g) = self.screen {
            g.overlay.fps = fps;
            g.overlay.placed_shapes = g
                .video_pipeline
                .as_ref()
                .map(|p| p.shapes.len() as u32)
                .unwrap_or(g.climber.placed_shapes);
            g.overlay.current_mse = g.climber.current_mse;
        }

        // Acquire surface texture.
        let surface_texture = match self.surface.get_current_texture() {
            Ok(tex) => tex,
            Err(wgpu::SurfaceError::Lost) => {
                let size = self.window.inner_size();
                self.reconfigure_surface(size.width, size.height);
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

        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Frame Encoder"),
        });

        // Background: blit the canvas (generation) or clear to dark (settings).
        match &self.screen {
            Screen::Generation(g) => {
                g.gpu.blit_canvas_to_surface(&mut encoder, &surface_view);
            }
            Screen::Settings(_) => {
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Settings Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &surface_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.05,
                                g: 0.06,
                                b: 0.09,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
        }

        // Run egui for the active screen, capturing any requested action.
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let ctx = self.egui_ctx.clone();
        let mut start_request: Option<(PathBuf, Settings, Language)> = None;
        let full_output = ctx.run(raw_input, |ectx| match &mut self.screen {
            Screen::Settings(s) => {
                if let ScreenAction::Start {
                    media_path,
                    settings,
                    language,
                } = s.render(ectx)
                {
                    start_request = Some((media_path, settings, language));
                }
            }
            Screen::Generation(g) => {
                g.overlay.render(ectx);
            }
        });

        self.egui_state
            .handle_platform_output(&self.window, full_output.platform_output);

        let paint_jobs = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [
                self.window.inner_size().width.max(1),
                self.window.inner_size().height.max(1),
            ],
            pixels_per_point: full_output.pixels_per_point,
        };

        for (id, image_delta) in &full_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, image_delta);
        }

        self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut render_pass = render_pass.forget_lifetime();
            self.egui_renderer
                .render(&mut render_pass, &paint_jobs, &screen_descriptor);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        surface_texture.present();

        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        // Handle a Start request after rendering (so the settings screen exists
        // when we need to report errors).
        if let Some((media_path, settings, language)) = start_request {
            match self.begin_generation(media_path, settings, language) {
                Ok(gen) => {
                    self.frame_times.clear();
                    self.screen = Screen::Generation(Box::new(gen));
                }
                Err(e) => {
                    log::error!("Failed to start generation: {}", e);
                    if let Screen::Settings(s) = &mut self.screen {
                        s.set_status(format!("{}", e), true);
                    }
                }
            }
        }
    }
}

impl GenerationContext {
    /// Execute generation iterations for the current frame.
    fn run_generation_step(&mut self, window: &Window) {
        if self.climber.state != GenerationState::Running {
            return;
        }

        let mutations_per_frame = self.settings.mutations_per_frame;
        let mut accepted_count = 0u32;
        let max_attempts = mutations_per_frame * 100;
        let mut attempts = 0u32;

        while accepted_count < mutations_per_frame && attempts < max_attempts {
            let result = self.climber.step(&self.gpu, &mut self.generator, &self.settings);
            attempts += 1;

            match result {
                StepResult::Accepted(candidate) => {
                    accepted_count += 1;
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
                StepResult::Rejected => {}
                StepResult::Completed => {
                    self.auto_save_on_completion(window);
                    break;
                }
                StepResult::Error(msg) => {
                    log::error!("Generation step error: {}", msg);
                    self.overlay.add_notification(format!("Generation error: {}", msg), 5.0, true);
                    break;
                }
            }
        }

        self.maybe_capture_gif_frame();
    }

    /// Capture a downscaled canvas frame for the progress GIF when the next
    /// placed-shape threshold is reached (no-op unless enabled in image mode).
    fn maybe_capture_gif_frame(&mut self) {
        if self.climber.placed_shapes < self.gif_next_capture {
            return;
        }
        match io::output::read_canvas_image(&self.gpu) {
            Ok(img) => {
                let img = io::output::downscale_to_width(img, self.settings.gif_max_width);
                self.gif_frames.push(img);
            }
            Err(e) => log::warn!("Progress GIF frame capture failed: {}", e),
        }
        self.gif_next_capture = self
            .gif_next_capture
            .saturating_add(self.gif_capture_stride.max(1));
    }

    /// Auto-save the canvas when generation completes (max_shapes reached).
    fn auto_save_on_completion(&mut self, window: &Window) {
        if self.auto_saved {
            return;
        }

        if self.overlay.is_video {
            self.handle_video_frame_complete(window);
            return;
        }

        self.auto_saved = true;
        window.set_title("TeekasFigure - Completed");

        let filename = io::output::completion_filename(&self.source_path);
        match io::output::save_canvas_png(&self.gpu, &self.output_folder, &filename) {
            Ok(path) => {
                log::info!("Auto-save on completion: {}", path.display());
                self.overlay
                    .add_notification(format!("Completed! Saved: {}", filename), 5.0, false);
            }
            Err(e) => {
                log::error!("Auto-save failed: {}", e);
                self.overlay
                    .add_notification(format!("Auto-save failed: {}", e), 5.0, true);
            }
        }

        self.save_progress_gif_if_enabled();
    }

    /// Assemble and save the progress GIF (image mode) if enabled and frames
    /// were captured. Captures one final frame of the completed canvas first.
    fn save_progress_gif_if_enabled(&mut self) {
        if !self.settings.save_progress_gif {
            return;
        }
        // Always include the finished canvas as the last frame.
        if let Ok(img) = io::output::read_canvas_image(&self.gpu) {
            self.gif_frames
                .push(io::output::downscale_to_width(img, self.settings.gif_max_width));
        }
        if self.gif_frames.is_empty() {
            return;
        }
        let gif_name = io::output::process_gif_filename(&self.source_path);
        match io::output::save_progress_gif(
            &self.gif_frames,
            &self.output_folder,
            &gif_name,
            self.settings.gif_fps,
        ) {
            Ok(path) => {
                log::info!("Saved progress GIF ({} frames): {}", self.gif_frames.len(), path.display());
                self.overlay
                    .add_notification(format!("Saved GIF: {}", gif_name), 6.0, false);
            }
            Err(e) => {
                log::error!("Progress GIF save failed: {}", e);
                self.overlay
                    .add_notification(format!("GIF save failed: {}", e), 6.0, true);
            }
        }
        // Free the captured frames now that the GIF is written.
        self.gif_frames = Vec::new();
    }

    /// Handle video frame completion: save current frame, advance to next.
    fn handle_video_frame_complete(&mut self, window: &Window) {
        // Render interpolated frames between the previous and current keyframe.
        let interp = self.settings.interpolation_steps;
        if interp > 0 && self.overlay.frame_number > 0 {
            if let Some(ref pipeline) = self.video_pipeline {
                for step in 1..=interp {
                    let t = step as f32 / (interp as f32 + 1.0);
                    pipeline.render_interpolated_frame(&self.gpu, t);
                    let inter_filename = format!("frame_{:05}.png", self.output_frame_index);
                    self.output_frame_index += 1;
                    if let Err(e) =
                        io::output::save_canvas_png(&self.gpu, &self.output_folder, &inter_filename)
                    {
                        log::error!("Failed to save interpolated frame {}: {}", inter_filename, e);
                    } else {
                        log::debug!("Saved interpolated frame: {}", inter_filename);
                    }
                }
                pipeline.rebuild_canvas(&self.gpu);
            }
        }

        let frame_num = self.overlay.frame_number;
        log::info!(
            "Frame {} complete with {} shapes in pipeline",
            frame_num,
            self.video_pipeline.as_ref().map(|p| p.shapes.len()).unwrap_or(0)
        );
        let filename = format!("frame_{:05}.png", self.output_frame_index);
        self.output_frame_index += 1;
        match io::output::save_canvas_png(&self.gpu, &self.output_folder, &filename) {
            Ok(path) => log::info!("Saved video keyframe: {}", path.display()),
            Err(e) => log::error!("Failed to save frame {}: {}", frame_num, e),
        }

        if let Some(ref mut decoder) = self.video_decoder {
            if let Some(frame_data) = decoder.next_frame() {
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

                self.generator = CandidateGenerator::new(
                    self.settings.clone(),
                    frame_data,
                    self.gpu.canvas_size,
                );

                if let Some(ref mut pipeline) = self.video_pipeline {
                    let before = pipeline.shapes.len();
                    let dead =
                        pipeline.adapt_to_new_frame(&self.gpu, &mut self.generator, &self.settings);
                    // After adapting/replacing existing shapes, grow the
                    // population toward max_shapes so new content appearing in
                    // later frames actually gets represented (the frame may have
                    // started nearly empty — e.g. a black opening frame — and
                    // would otherwise stay frozen at a handful of shapes for the
                    // whole clip).
                    let grown = pipeline.grow_population(
                        &self.gpu,
                        &mut self.generator,
                        &self.settings,
                    );
                    log::info!(
                        "Frame {}: adapted {} shapes, {} died (scene change), {} grown, {} total",
                        frame_num + 1,
                        before,
                        dead,
                        grown,
                        pipeline.shapes.len()
                    );
                    pipeline.rebuild_canvas(&self.gpu);
                }

                // The climber no longer drives video shape placement (growth is
                // handled by the pipeline above). Mark it full so its next step
                // Completes and simply advances to the next frame.
                self.climber = HillClimber::new();
                self.climber.placed_shapes = self.settings.max_shapes;
                if let Some(ref pipeline) = self.video_pipeline {
                    log::info!(
                        "Frame {}: {} shapes after adaptation + rebirth + growth",
                        self.overlay.frame_number + 1,
                        pipeline.shapes.len(),
                    );
                }
                self.overlay.frame_number += 1;
                self.overlay.placed_shapes = self
                    .video_pipeline
                    .as_ref()
                    .map(|p| p.shapes.len() as u32)
                    .unwrap_or(self.climber.placed_shapes);

                window.set_title(&format!(
                    "TeekasFigure - Frame {} - Running",
                    self.overlay.frame_number
                ));
            } else {
                self.finalize_video(window);
            }
        } else {
            self.auto_saved = true;
            self.climber.state = GenerationState::Completed;
        }
    }

    /// Encode the saved frames into the final MP4.
    fn finalize_video(&mut self, window: &Window) {
        self.auto_saved = true;
        self.climber.state = GenerationState::Completed;
        window.set_title("TeekasFigure - Video Complete");
        self.overlay
            .add_notification("Video processing complete!".to_string(), 10.0, false);

        log::info!("All frames processed, encoding to MP4...");
        let stem = self
            .source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video");
        let output_mp4 = self.output_folder.join(format!("{}_result.mp4", stem));

        let fps = self.video_decoder.as_ref().map(|d| d.fps).unwrap_or(30.0);
        let output_fps = fps * (self.settings.interpolation_steps as f64 + 1.0);

        let frames_pattern = self.output_folder.join("frame_%05d.png");

        // Build the FFmpeg args. When audio preservation is on, add the source
        // video as a second input and map its audio stream (optional via "?"),
        // so a source without audio still encodes fine.
        let framerate = format!("{}", output_fps);
        let frames_pattern_str = frames_pattern.to_str().unwrap_or("").to_string();
        let output_mp4_str = output_mp4.to_str().unwrap_or("").to_string();
        let source_str = self.source_path.to_str().unwrap_or("").to_string();

        let mut args: Vec<String> = vec![
            "-y".into(),
            "-framerate".into(),
            framerate,
            "-i".into(),
            frames_pattern_str,
        ];

        let want_audio = self.settings.preserve_audio && !source_str.is_empty();
        if want_audio {
            args.push("-i".into());
            args.push(source_str);
            args.push("-map".into());
            args.push("0:v:0".into());
            args.push("-map".into());
            args.push("1:a:0?".into()); // optional: skip if source has no audio
            args.push("-c:a".into());
            args.push("aac".into());
            args.push("-shortest".into());
        }

        args.push("-c:v".into());
        args.push("libx264".into());
        args.push("-pix_fmt".into());
        args.push("yuv420p".into());
        args.push("-v".into());
        args.push("quiet".into());
        args.push(output_mp4_str);

        let encode_result = std::process::Command::new("ffmpeg").args(&args).output();

        let encoded_ok = matches!(&encode_result, Ok(o) if o.status.success());
        let file_ready = std::fs::metadata(&output_mp4)
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false);

        if encoded_ok && file_ready {
            let size_bytes = std::fs::metadata(&output_mp4).map(|m| m.len()).unwrap_or(0);
            let total_frames = self.output_frame_index;
            log::info!(
                "VIDEO READY: '{}' written ({} frames, {} bytes). The file is complete and playable.",
                output_mp4.display(), total_frames, size_bytes
            );
            println!(
                "VIDEO READY: {} ({} frames, {} bytes) — file is complete and playable.",
                output_mp4.display(),
                total_frames,
                size_bytes
            );
            window.set_title("TeekasFigure - Video Ready (file saved)");
            self.overlay.add_notification(
                format!("Video ready: {}", output_mp4.display()),
                15.0,
                false,
            );
        } else {
            let detail = match &encode_result {
                Ok(o) if !o.status.success() => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    format!("ffmpeg exited with {}: {}", o.status, stderr.trim())
                }
                Ok(_) => "ffmpeg succeeded but output file is missing or empty".to_string(),
                Err(e) => format!("failed to launch ffmpeg: {}", e),
            };
            log::error!("FFmpeg encoding failed: {}", detail);
            window.set_title("TeekasFigure - Video encoding FAILED");
            self.overlay
                .add_notification(format!("Video encoding failed: {}", detail), 15.0, true);
        }
    }

    /// Save a manual snapshot of the current canvas.
    fn snapshot(&mut self) {
        log::info!("Snapshot save requested (S key)");
        let filename = io::output::snapshot_filename();
        match io::output::save_canvas_png(&self.gpu, &self.output_folder, &filename) {
            Ok(path) => {
                log::info!("Snapshot saved: {}", path.display());
                self.overlay
                    .add_notification(format!("Saved: {}", filename), 3.0, false);
            }
            Err(e) => {
                log::error!("Snapshot save failed: {}", e);
                self.overlay
                    .add_notification(format!("Save failed: {}", e), 5.0, true);
            }
        }
    }
}

/// Wrapper struct that implements `ApplicationHandler` for winit 0.30.
struct AppHandler {
    app: Option<App>,
}

impl ApplicationHandler for AppHandler {
    fn resumed(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {}

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(app) = self.app.as_mut() else {
            return;
        };

        // Pass events to egui first.
        let egui_response = app.egui_state.on_window_event(&app.window, &event);
        if egui_response.consumed {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                log::info!("Close requested, shutting down");
                if let Screen::Generation(ref mut g) = app.screen {
                    g.climber.state = GenerationState::Completed;
                }
                event_loop.exit();
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                match logical_key {
                    Key::Named(NamedKey::Space) => {
                        if let Screen::Generation(ref mut g) = app.screen {
                            match g.climber.state {
                                GenerationState::Running => {
                                    g.climber.state = GenerationState::Paused;
                                    app.window.set_title("TeekasFigure - Paused");
                                    log::info!("Generation paused");
                                }
                                GenerationState::Paused => {
                                    g.climber.state = GenerationState::Running;
                                    app.window.set_title("TeekasFigure - Running");
                                    log::info!("Generation resumed");
                                }
                                GenerationState::Completed => {}
                            }
                        }
                    }
                    Key::Character(ref c) if c.as_str() == "s" || c.as_str() == "S" => {
                        if let Screen::Generation(ref mut g) = app.screen {
                            g.snapshot();
                        }
                    }
                    Key::Named(NamedKey::Escape) => {
                        log::info!("Escape pressed, shutting down");
                        if let Screen::Generation(ref mut g) = app.screen {
                            g.climber.state = GenerationState::Completed;
                        }
                        event_loop.exit();
                    }
                    _ => {}
                }
            }

            WindowEvent::Resized(new_size) => {
                app.reconfigure_surface(new_size.width, new_size.height);
            }

            WindowEvent::RedrawRequested => {
                if let Screen::Generation(ref mut g) = app.screen {
                    // SAFETY: run_generation_step needs &Window while g is borrowed
                    // from app.screen; window is a separate Arc field.
                    let window = app.window.clone();
                    g.run_generation_step(&window);
                }
                app.render_frame();
                app.window.request_redraw();
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
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
        let result = compute_window_size((800, 600), (1920, 1080));
        assert_eq!(result, (800, 600));
    }

    #[test]
    fn test_target_exceeds_width() {
        let result = compute_window_size((2000, 500), (1920, 1080));
        assert_eq!(result.0, 1728);
        assert_eq!(result.1, 432);
    }

    #[test]
    fn test_target_exceeds_height() {
        let result = compute_window_size((500, 1200), (1920, 1080));
        assert_eq!(result.0, 405);
        assert_eq!(result.1, 972);
    }

    #[test]
    fn test_target_exceeds_both_dimensions() {
        let result = compute_window_size((3000, 2000), (1920, 1080));
        assert_eq!(result.0, 1458);
        assert_eq!(result.1, 972);
        assert!(result.0 <= 1728);
        assert!(result.1 <= 972);
    }

    #[test]
    fn test_exact_90_percent_boundary() {
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
        let result = compute_window_size((2000, 2000), (1920, 1080));
        assert_eq!(result.0, 972);
        assert_eq!(result.1, 972);
        assert!(result.0 <= 1728);
        assert!(result.1 <= 972);
    }

    #[test]
    fn test_aspect_ratio_preserved() {
        let result = compute_window_size((1600, 900), (1000, 800));
        let original_ratio = 1600.0_f64 / 900.0;
        let result_ratio = result.0 as f64 / result.1 as f64;
        assert!((original_ratio - result_ratio).abs() < 0.01);
        assert!(result.0 <= 900);
        assert!(result.1 <= 720);
    }
}
