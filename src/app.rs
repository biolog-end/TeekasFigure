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
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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
    Generation(Box<GenerationView>),
}

struct GenerationView {
    gpu: Arc<GpuContext>,
    overlay: OverlayState,
    latest: Arc<Mutex<OverlayState>>,
    worker: std::thread::JoinHandle<()>,
    settings: Settings,
    last_metrics: (Instant, u64, u64),
    rate: f64,
    evaluation_percent: f64,
    source_name: String,
    panel_visible: bool,
}
impl GenerationView {
    fn start(mut generation: GenerationContext, window: Arc<Window>) -> Self {
        let gpu = generation.gpu.clone();
        let overlay = generation.overlay.clone();
        let latest = Arc::new(Mutex::new(overlay.clone()));
        let published = latest.clone();
        let settings = generation.settings.clone();
        let source_name = generation
            .source_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let worker = std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                while generation.gpu.control.checkpoint() {
                    if generation
                        .gpu
                        .control
                        .snapshot
                        .swap(false, Ordering::Relaxed)
                    {
                        generation.snapshot();
                    }
                    generation.run_generation_step(&window);
                    generation.overlay.placed_shapes = generation
                        .video_pipeline
                        .as_ref()
                        .map(|p| p.shapes.len() as u32)
                        .unwrap_or(generation.climber.placed_shapes);
                    *published.lock().unwrap() = generation.overlay.clone();
                    if generation.climber.state == GenerationState::Completed {
                        break;
                    }
                    if crate::monitor::available_memory() < 256 * 1024 * 1024 {
                        panic!("Generation stopped: less than 256 MiB free RAM / Генерация остановлена: мало памяти");
                    }
                }
            }));
            if let Err(error) = outcome {
                let message = error
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "Generation worker failed".into());
                generation.overlay.add_notification(message, 3600.0, true);
            }
            *published.lock().unwrap() = generation.overlay.clone();
        });
        Self {
            gpu,
            overlay,
            latest,
            worker,
            settings,
            last_metrics: (Instant::now(), 0, 0),
            rate: 0.0,
            evaluation_percent: 0.0,
            source_name,
            panel_visible: true,
        }
    }
    fn snapshot(&self) {
        let result = crate::io::output::save_canvas_png(
            &self.gpu,
            &crate::io::media_loader::get_base_dir().join("output"),
            &crate::io::output::snapshot_filename(),
        );
        let (message, error) = match result {
            Ok(path) => (format!("Saved / Сохранено: {}", path.display()), false),
            Err(e) => (e.to_string(), true),
        };
        self.latest
            .lock()
            .unwrap()
            .add_notification(message, 15.0, error);
    }
    fn render_controls(
        &mut self,
        ctx: &egui::Context,
        active_gpu: &str,
        metrics: crate::monitor::Sample,
    ) {
        let control = self.gpu.control.clone();
        let elapsed = self.last_metrics.0.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            let count = control.evaluated.load(Ordering::Relaxed);
            let ns = control.evaluation_ns.load(Ordering::Relaxed);
            self.rate = count.saturating_sub(self.last_metrics.1) as f64 / elapsed;
            self.evaluation_percent =
                (ns.saturating_sub(self.last_metrics.2) as f64 / 1e9 / elapsed * 100.0)
                    .clamp(0.0, 100.0);
            self.last_metrics = (Instant::now(), count, ns);
        }
        let lang = self.overlay.language;
        self.overlay.placed_shapes = control.placed.load(Ordering::Relaxed) as u32;
        if !self.panel_visible {
            egui::Area::new(egui::Id::new("show_generation_controls"))
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
                .show(ctx, |ui| {
                    if ui
                        .button(lang.t("Show panel · H", "Показать панель · H"))
                        .clicked()
                    {
                        self.panel_visible = true;
                    }
                });
            return;
        }
        egui::SidePanel::right("generation_controls")
            .default_width(330.0).min_width(260.0).max_width(440.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false])
                    .max_height((ui.available_height() - 78.0).max(40.0))
                    .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(lang.t("Progress", "Ход работы"));
                    if ui.small_button(lang.t("Hide · H", "Скрыть · H")).clicked() { self.panel_visible = false; }
                });
                ui.label(&self.source_name);
                ui.label(format!("{}: {}", lang.t("Shapes", "Фигур"), self.overlay.placed_shapes));
                let current = control.search_generation.load(Ordering::Relaxed);
                let total = control.search_total.load(Ordering::Relaxed).max(1);
                ui.add(egui::ProgressBar::new(current as f32 / total as f32).text(format!("{}: {current} / {total}", lang.t("Current search", "Текущий поиск"))));
                ui.small(lang.t("One search places at most one shape. The last pass evaluates the final offspring.", "Один поиск добавляет не более одной фигуры. Последний проход оценивает итоговое поколение."));
                let rejected = control.rejections.load(Ordering::Relaxed);
                ui.label(format!("{}: {rejected} / {}", lang.t("Rejected searches in a row", "Отклонено поисков подряд"), self.settings.max_rejections));
                let best = f32::from_bits(control.best_score.load(Ordering::Relaxed));
                if best.is_finite() {
                    ui.label(format!("{}: {best:.3}", lang.t("Best delta (negative improves)", "Лучшее изменение ошибки (минус — улучшение)")));
                    if self.settings.use_min_improvement { ui.label(format!("{}: {:.3}",lang.t("Acceptance threshold", "Порог принятия"),self.settings.min_improvement)); }
                }
                ui.separator();
                ui.label(format!("GPU: {active_gpu}"));
                let percent = |v: Option<f64>| {
                    v.map(|n| format!("{n:.1}%"))
                        .unwrap_or_else(|| lang.t("unavailable", "нет данных").into())
                };
                let mib = |v: Option<u64>| {
                    v.map(|n| format!("{:.0} MiB", n as f64 / 1048576.0))
                        .unwrap_or_else(|| lang.t("unavailable", "нет данных").into())
                };
                ui.label(format!(
                    "CPU: {} · RAM: {}",
                    percent(metrics.cpu_percent),
                    mib(metrics.ram_bytes)
                ));
                ui.label(format!(
                    "{}: {}",
                    lang.t(
                        "GPU process (busiest engine)",
                        "GPU приложения (самый занятый движок)"
                    ),
                    percent(metrics.gpu_percent)
                ))
                .on_hover_text(&metrics.gpu_engine);
                ui.label(format!(
                    "{}: {}",
                    lang.t(
                        "Process dedicated GPU memory",
                        "Выделенная GPU-память приложения"
                    ),
                    mib(metrics.gpu_memory_bytes)
                ));
                ui.label(format!(
                    "{}: {:.0}",
                    lang.t("Candidates / second", "Кандидатов / секунду"),
                    self.rate
                ));
                ui.label(format!(
                    "{}: {:.1}%",
                    lang.t(
                        "Time in GPU evaluation + transfers + wait",
                        "Время на GPU-оценку + передачу + ожидание"
                    ),
                    self.evaluation_percent
                ));
                ui.label(format!(
                    "{}: {}",
                    lang.t(
                        "Evolution generations evaluated (total)",
                        "Вычислено поколений (всего)"
                    ),
                    control.generations.load(Ordering::Relaxed)
                ));
                ui.small(lang.t(
                    "OS counters: once per second. Evaluation time is not GPU utilization.",
                    "Системные счётчики: раз в секунду. Время оценки — не загрузка GPU.",
                ));
                });
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    let paused = control.paused.load(Ordering::Relaxed);
                    if ui
                        .add_enabled(
                            !self.worker.is_finished(),
                            egui::Button::new(if paused {
                                lang.t("Resume", "Продолжить")
                            } else {
                                lang.t("Pause", "Пауза")
                            }),
                        )
                        .clicked()
                    {
                        control.paused.store(!paused, Ordering::Relaxed);
                    }
                    if ui.button(lang.t("Snapshot", "Снимок")).clicked() {
                        self.snapshot();
                    }
                    if ui
                        .button(lang.t("Stop / Settings", "Остановить / Настройки"))
                        .clicked()
                    {
                        control.cancelled.store(true, Ordering::Relaxed);
                    }
                });
                if control.cancelled.load(Ordering::Relaxed) {
                    ui.label(lang.t("Stopping…", "Остановка…"));
                } else if self.worker.is_finished() {
                    ui.label(lang.t("Processing finished", "Обработка завершена"));
                }
            });
    }
}
impl Drop for GenerationView {
    fn drop(&mut self) {
        self.gpu.control.cancelled.store(true, Ordering::Relaxed);
    }
}

/// All state that only exists while generation is running.
pub struct GenerationContext {
    pub gpu: Arc<GpuContext>,
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
    output_frame_index: u64,
    scene_writer: Option<io::scene::SceneWriter>,
    image_scene_shapes: Vec<(u64, crate::types::CandidateParams)>,
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
    active_gpu: String,
    available_gpus: Vec<String>,
    monitor: crate::monitor::Monitor,
    pending_exit: bool,
    next_redraw: Option<Instant>,
    repaint_delay: Duration,
    occluded: bool,
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
        active_gpu: String,
        available_gpus: Vec<String>,
    ) -> Self {
        // Configure the surface for the initial (settings) window size.
        let size = window.inner_size();
        surface.configure(
            &device,
            &make_surface_config(size.width, size.height, surface_format),
        );

        let mut settings_screen = SettingsScreen::new(&base_dir, settings, language);
        settings_screen.active_gpu = active_gpu.clone();
        settings_screen.available_gpus = available_gpus.clone();
        settings_screen.shape_layer_limit = (device.limits().max_texture_array_layers as usize).min(2048);
        settings_screen.dialog_owner = dialog_owner(&window);
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
            active_gpu,
            available_gpus,
            monitor: crate::monitor::Monitor::new(),
            pending_exit: false,
            next_redraw: Some(Instant::now()),
            repaint_delay: Duration::ZERO,
            occluded: false,
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
        self.surface.configure(
            &self.device,
            &make_surface_config(width, height, self.surface_format),
        );
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
        settings.validate()?;
        if settings.max_texture_size > self.device.limits().max_texture_dimension_2d
            || settings.shape_resolution > self.device.limits().max_texture_dimension_2d
        {
            return Err(AppError::GpuInit(
                "Requested resolution exceeds this GPU's limit / Разрешение превышает предел GPU"
                    .into(),
            ));
        }
        let is_video = io::media_loader::is_video_path(&media_path);
        let mut video_decoder = if is_video {
            Some(crate::io::video::VideoProcessor::new_scaled(
                &media_path,
                &self.output_folder.join("result.mp4"),
                settings.target_fps,
                settings.max_texture_size,
            )?)
        } else {
            None
        };
        let (target_data, target_size) = if let Some(decoder) = &mut video_decoder {
            let data = decoder
                .next_frame()
                .ok_or_else(|| AppError::Ffmpeg("Cannot decode first frame".into()))?;
            (data, (decoder.width, decoder.height))
        } else {
            let img = io::shape_conversion::open_bounded(
                &media_path,
                crate::monitor::available_memory() / 3,
            )
            .map_err(|e| AppError::GpuInit(e.to_string()))?;
            let img = if img.width() > settings.max_texture_size
                || img.height() > settings.max_texture_size
            {
                img.resize(
                    settings.max_texture_size,
                    settings.max_texture_size,
                    image::imageops::FilterType::Triangle,
                )
            } else {
                img
            };
            let rgba = img.to_rgba8();
            let size = rgba.dimensions();
            (rgba.into_raw(), size)
        };
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
        // Leave room for controls even when processing a tiny image, retaining aspect ratio.
        let preview_scale = (900.0 / target_size.0 as f64)
            .max(650.0 / target_size.1 as f64)
            .max(1.0);
        let win_size = compute_window_size(
            (
                (target_size.0 as f64 * preview_scale) as u32,
                (target_size.1 as f64 * preview_scale) as u32,
            ),
            display_size,
        );
        let _ = self
            .window
            .request_inner_size(winit::dpi::PhysicalSize::new(win_size.0, win_size.1));
        self.reconfigure_surface(win_size.0, win_size.1);

        // 3. Load and preprocess shapes (raw_shapes/ keeps original colors).
        let shape_root = io::library::shape_root(&self.base_dir, settings.shape_set.as_deref());
        let (shapes_folder, preserve_color) = if settings.use_original_colors {
            (shape_root.join("raw_shapes"), true)
        } else {
            (shape_root.join("input_shapes"), false)
        };
        log::info!(
            "Loading shapes from '{}' (original colors: {})",
            shapes_folder.display(),
            settings.use_original_colors
        );
        let shape_count = std::fs::read_dir(&shapes_folder)
            .map_err(|_| AppError::NoShapes {
                path: shapes_folder.clone(),
            })?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(io::media_loader::is_supported_image_extension)
                        .unwrap_or(false)
            })
            .count()
            .min(self.device.limits().max_texture_array_layers as usize)
            .min(2048);
        crate::gpu::safety::validate(
            &settings,
            &self.device.limits(),
            target_size,
            shape_count as u32,
            is_video,
        )?;
        let (mut shapes, mut shape_sources) = io::shape_preprocessor::load_with_sources(
            &shapes_folder,
            settings.shape_resolution,
            preserve_color,
            shape_count,
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
            shape_sources.truncate(max_layers);
        }
        io::shape_preprocessor::check_vram_budget(
            settings.shape_resolution,
            shapes.len() as u32,
            settings.vram_budget_mb,
        )?;
        log::info!("Loaded {} shape(s)", shapes.len());

        // 4. Build GPU resources on the shared device.
        self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let gpu_result = GpuContext::new_from_device(
            self.device.clone(),
            self.queue.clone(),
            self.surface_format,
            &target_data,
            target_size,
            &shapes,
            &settings,
        );
        self.device.poll(wgpu::Maintain::Wait);
        let validation = pollster::block_on(self.device.pop_error_scope());
        let memory = pollster::block_on(self.device.pop_error_scope());
        if let Some(error) = validation.or(memory) {
            return Err(AppError::GpuInit(error.to_string()));
        }
        let gpu = Arc::new(gpu_result?);

        // 5. Algorithm components.
        let climber = HillClimber::new();
        let generator = CandidateGenerator::new(settings.clone(), target_data, target_size);
        let overlay = OverlayState::with_language(settings.max_shapes, is_video, language);

        let scene_writer = if settings.export_scene {
            let fps = video_decoder
                .as_ref()
                .map(|d| d.fps * (settings.interpolation_steps as f64 + 1.0))
                .unwrap_or(1.0);
            Some(io::scene::SceneWriter::new(
                &self.output_folder,
                &media_path,
                target_size,
                fps,
                is_video,
                &settings,
                &shapes,
                &shape_sources,
            )?)
        } else {
            None
        };

        let ctx = GenerationContext {
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
            video_decoder,
            output_frame_index: 0,
            scene_writer,
            image_scene_shapes: Vec::new(),
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
                log::info!(
                    "Removed {} leftover frame_*.png file(s) from a previous run",
                    removed
                );
            }
        }

        self.window
            .set_title("TeekasFigure - Running  (Space: pause, S: snapshot, Esc: settings)");
        Ok(ctx)
    }

    /// Render a frame for whichever screen is active.
    fn render_frame(&mut self) {
        // Retry transient surface failures even if the settings UI was idle.
        self.repaint_delay = Duration::from_millis(250);
        if self.occluded
            || self.window.inner_size().width == 0
            || self.window.inner_size().height == 0
        {
            return;
        }
        let fps = self.compute_fps();
        if let Screen::Generation(ref mut g) = self.screen {
            if let Ok(latest) = g.latest.lock() {
                g.overlay = latest.clone();
            }
            g.overlay.fps = fps;
            g.overlay.placed_shapes = g.gpu.control.placed.load(Ordering::Relaxed) as u32;
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

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Frame Encoder"),
            });

        // Run egui for the active screen, capturing any requested action.
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let ctx = self.egui_ctx.clone();
        let mut start_request: Option<(PathBuf, Settings, Language)> = None;
        let mut canvas_rect = None;
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
                g.render_controls(ectx, &self.active_gpu, self.monitor.sample());
                canvas_rect = Some(ectx.available_rect());
                g.overlay.render(ectx);
            }
        });
        self.repaint_delay = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|viewport| viewport.repaint_delay)
            .unwrap_or(Duration::MAX);

        // Background: blit the canvas (generation) or clear to dark (settings).
        match &self.screen {
            Screen::Generation(g) => {
                let rect = canvas_rect.expect("Generation canvas layout");
                let ppp = full_output.pixels_per_point;
                g.gpu.blit_canvas_to_surface(
                    &mut encoder,
                    &surface_view,
                    [
                        rect.min.x * ppp,
                        rect.min.y * ppp,
                        (rect.width() * ppp).max(1.0),
                        (rect.height() * ppp).max(1.0),
                    ],
                );
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
                    self.screen = Screen::Generation(Box::new(GenerationView::start(
                        gen,
                        self.window.clone(),
                    )));
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
        let max_attempts = mutations_per_frame.saturating_mul(100);
        let slice_started = Instant::now();
        let mut attempts = 0u32;

        while accepted_count < mutations_per_frame && attempts < max_attempts {
            if !self.gpu.control.checkpoint() {
                return;
            }
            if slice_started.elapsed().as_millis() >= 20 && attempts > 0 {
                break;
            }
            let result = self
                .climber
                .step(&self.gpu, &mut self.generator, &self.settings);
            attempts += 1;
            if !self.gpu.control.checkpoint() {
                return;
            }

            match result {
                StepResult::Accepted(candidate) => {
                    accepted_count += 1;
                    if let Some(ref mut pipeline) = self.video_pipeline {
                        pipeline.record_placed_shape(candidate);
                    } else if self.scene_writer.is_some() {
                        self.image_scene_shapes
                            .push((self.image_scene_shapes.len() as u64, candidate));
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
                    self.overlay
                        .add_notification(format!("Generation error: {}", msg), 5.0, true);
                    break;
                }
            }
        }

        self.maybe_capture_gif_frame();
    }

    /// Capture a downscaled canvas frame for the progress GIF when the next
    /// placed-shape threshold is reached (no-op unless enabled in image mode).
    fn maybe_capture_gif_frame(&mut self) {
        if !self.settings.save_progress_gif
            || self.overlay.is_video
            || self.gif_frames.len() >= self.settings.gif_frames as usize
            || self.climber.placed_shapes < self.gif_next_capture
        {
            return;
        }
        match io::output::read_canvas_image(&self.gpu) {
            Ok(img) => {
                let img = io::output::downscale_to_width(img, self.settings.gif_max_width);
                if img.as_raw().len() as u64 * 3 > crate::monitor::available_memory() {
                    self.settings.save_progress_gif = false;
                    self.gif_next_capture = u32::MAX;
                    self.overlay.add_notification(
                        "GIF capture stopped: low RAM / Захват GIF остановлен: мало памяти".into(),
                        30.0,
                        true,
                    );
                    return;
                }
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
        if !self.gpu.control.checkpoint() {
            return;
        }
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
                self.overlay.add_notification(
                    format!("Completed! Saved: {}", filename),
                    5.0,
                    false,
                );
            }
            Err(e) => {
                log::error!("Auto-save failed: {}", e);
                self.overlay
                    .add_notification(format!("Auto-save failed: {}", e), 5.0, true);
            }
        }

        if let Some(writer) = &mut self.scene_writer {
            if let Err(e) = writer.write_frame(self.image_scene_shapes.iter().copied()) {
                self.scene_failure(e);
                return;
            }
        }
        self.finish_scene();
        self.save_progress_gif_if_enabled();
    }

    fn scene_failure(&mut self, error: AppError) {
        log::error!("{error}");
        self.overlay
            .add_notification(error.to_string(), 3600.0, true);
        self.climber.state = GenerationState::Completed;
        self.auto_saved = true;
        // Drop writes an incomplete manifest with the successfully saved count.
        self.scene_writer = None;
    }

    fn finish_scene(&mut self) {
        if let Some(mut writer) = self.scene_writer.take() {
            match writer.finish() {
                Ok(path) => self.overlay.add_notification(
                    format!("Scene / Раскладка: {}", path.display()),
                    3600.0,
                    false,
                ),
                Err(error) => self.scene_failure(error),
            }
        }
    }

    /// Assemble and save the progress GIF (image mode) if enabled and frames
    /// were captured. Captures one final frame of the completed canvas first.
    fn save_progress_gif_if_enabled(&mut self) {
        if !self.settings.save_progress_gif {
            return;
        }
        // Always include the finished canvas as the last frame.
        if let Ok(img) = io::output::read_canvas_image(&self.gpu) {
            self.gif_frames.push(io::output::downscale_to_width(
                img,
                self.settings.gif_max_width,
            ));
        }
        if self.gif_frames.is_empty() {
            return;
        }
        let gif_name = io::output::process_gif_filename(&self.source_path);
        match io::output::save_progress_gif_cancellable(
            &self.gif_frames,
            &self.output_folder,
            &gif_name,
            self.settings.gif_fps,
            Some(&self.gpu.control.cancelled),
        ) {
            Ok(path) => {
                log::info!(
                    "Saved progress GIF ({} frames): {}",
                    self.gif_frames.len(),
                    path.display()
                );
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
            if self.video_pipeline.is_some() {
                for step in 1..=interp {
                    if !self.gpu.control.checkpoint() {
                        return;
                    }
                    let t = step as f32 / (interp as f32 + 1.0);
                    self.video_pipeline
                        .as_ref()
                        .unwrap()
                        .render_interpolated_frame(&self.gpu, t);
                    let inter_filename = format!("frame_{:05}.png", self.output_frame_index);
                    self.output_frame_index = self
                        .output_frame_index
                        .checked_add(1)
                        .expect("Video frame index overflow");
                    if let Err(e) =
                        io::output::save_canvas_png(&self.gpu, &self.output_folder, &inter_filename)
                    {
                        self.overlay
                            .add_notification(format!("Save stopped: {e}"), 3600.0, true);
                        self.climber.state = GenerationState::Completed;
                        return;
                    } else {
                        log::debug!("Saved interpolated frame: {}", inter_filename);
                    }
                    if let Some(writer) = &mut self.scene_writer {
                        if let Err(error) = writer.write_frame(
                            self.video_pipeline.as_ref().unwrap().interpolated_shapes(t),
                        ) {
                            self.scene_failure(error);
                            return;
                        }
                    }
                }
                self.video_pipeline
                    .as_ref()
                    .unwrap()
                    .rebuild_canvas(&self.gpu);
            }
        }

        let frame_num = self.overlay.frame_number;
        log::info!(
            "Frame {} complete with {} shapes in pipeline",
            frame_num,
            self.video_pipeline
                .as_ref()
                .map(|p| p.shapes.len())
                .unwrap_or(0)
        );
        let filename = format!("frame_{:05}.png", self.output_frame_index);
        self.output_frame_index = self
            .output_frame_index
            .checked_add(1)
            .expect("Video frame index overflow");
        match io::output::save_canvas_png(&self.gpu, &self.output_folder, &filename) {
            Ok(path) => log::info!("Saved video keyframe: {}", path.display()),
            Err(e) => {
                self.overlay
                    .add_notification(format!("Save stopped: {e}"), 3600.0, true);
                self.climber.state = GenerationState::Completed;
                return;
            }
        }

        if let (Some(writer), Some(pipeline)) = (&mut self.scene_writer, &self.video_pipeline) {
            if let Err(error) = writer.write_frame(pipeline.shapes.iter().map(|s| (s.id, s.params)))
            {
                self.scene_failure(error);
                return;
            }
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
                    if !(pipeline.shapes.is_empty() && self.generator.empty_canvas_is_solution()) {
                        let before = pipeline.shapes.len();
                        let dead = pipeline.adapt_to_new_frame(
                            &self.gpu,
                            &mut self.generator,
                            &self.settings,
                        );
                        // After adapting/replacing existing shapes, grow the
                        // population toward max_shapes so new content appearing in
                        // later frames actually gets represented (the frame may have
                        // started nearly empty — e.g. a black opening frame — and
                        // would otherwise stay frozen at a handful of shapes for the
                        // whole clip).
                        if !self.gpu.control.checkpoint() {
                            return;
                        }
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
                }

                // Video growth is handled by the pipeline above. Mark the climber
                // full so its next step Completes and simply advances to the next frame.
                self.climber = HillClimber::new();
                self.climber.placed_shapes = self.settings.max_shapes;
                if let Some(ref pipeline) = self.video_pipeline {
                    log::info!(
                        "Frame {}: {} shapes after adaptation + rebirth + growth",
                        self.overlay.frame_number + 1,
                        pipeline.shapes.len(),
                    );
                }
                self.overlay.frame_number = self.overlay.frame_number.saturating_add(1);
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
        self.finish_scene();
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
        args.push("-vf".into());
        args.push("pad=ceil(iw/2)*2:ceil(ih/2)*2".into());
        args.push("-pix_fmt".into());
        args.push("yuv420p".into());
        args.push("-v".into());
        args.push("quiet".into());
        args.push(output_mp4_str);

        let encode_result =
            crate::io::video::encode_cancellable(&args, &self.gpu.control.cancelled);
        if self.gpu.control.cancelled.load(Ordering::Relaxed) {
            return;
        }

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
        // egui marks RedrawRequested itself as needing paint. Re-requesting it
        // here would create an unbounded loop and defeat the refresh timer.
        if egui_response.repaint && !matches!(event, WindowEvent::RedrawRequested) {
            app.window.request_redraw();
        }
        if egui_response.consumed {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                log::info!("Close requested, shutting down");
                app.pending_exit = true;
                if let Screen::Generation(g) = &app.screen {
                    g.gpu.control.cancelled.store(true, Ordering::Relaxed);
                } else if let Screen::Settings(s) = &app.screen {
                    s.cancel_file_job();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match logical_key {
                Key::Named(NamedKey::Space) => {
                    if let Screen::Generation(g) = &app.screen {
                        let control = &g.gpu.control;
                        control.paused.fetch_xor(true, Ordering::Relaxed);
                    }
                }
                Key::Character(ref c) if c.eq_ignore_ascii_case("s") => {
                    if let Screen::Generation(g) = &app.screen {
                        g.snapshot();
                    }
                }
                Key::Character(ref c) if c.eq_ignore_ascii_case("h") => {
                    if let Screen::Generation(g) = &mut app.screen {
                        g.panel_visible = !g.panel_visible;
                    }
                }
                Key::Named(NamedKey::Escape) => {
                    if let Screen::Generation(g) = &app.screen {
                        g.gpu.control.cancelled.store(true, Ordering::Relaxed);
                    } else if let Screen::Settings(s) = &mut app.screen {
                        if !s.cancel_browser() {
                            app.pending_exit = true;
                        }
                    }
                }
                _ => {}
            },

            WindowEvent::Resized(new_size) => {
                app.reconfigure_surface(new_size.width, new_size.height);
                app.window.request_redraw();
            }
            WindowEvent::Occluded(occluded) => {
                app.occluded = occluded;
                app.window.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                let stopped = match &app.screen {
                    Screen::Generation(g)
                        if g.gpu.control.cancelled.load(Ordering::Relaxed)
                            && g.worker.is_finished() =>
                    {
                        Some((g.settings.clone(), g.overlay.language))
                    }
                    _ => None,
                };
                if let Some((settings, language)) = stopped {
                    let mut screen = SettingsScreen::new(&app.base_dir, settings, language);
                    screen.active_gpu = app.active_gpu.clone();
                    screen.available_gpus = app.available_gpus.clone();
                    screen.shape_layer_limit = (app.device.limits().max_texture_array_layers as usize).min(2048);
                    screen.dialog_owner = dialog_owner(&app.window);
                    app.screen = Screen::Settings(screen);
                    app.window.set_title("TeekasFigure - Settings");
                }
                if app.pending_exit && matches!(&app.screen, Screen::Settings(s) if !s.is_busy()) {
                    event_loop.exit();
                    return;
                }
                app.render_frame();
                // Refresh only the presentation. Evolution stays on the worker.
                // Idle settings wait for egui/input; hidden/minimized windows
                // do not continuously submit presentation work to the GPU.
                let delay = if app.occluded {
                    Some(Duration::from_secs(1))
                } else {
                    match &app.screen {
                        Screen::Generation(g) => Some(generation_refresh_interval(
                            g.panel_visible,
                            g.gpu.control.paused.load(Ordering::Relaxed),
                            g.worker.is_finished(),
                        )),
                        Screen::Settings(s) if s.is_busy() || app.pending_exit => {
                            Some(Duration::from_millis(100))
                        }
                        Screen::Settings(_) => (app.repaint_delay != Duration::MAX)
                            .then_some(app.repaint_delay.max(Duration::from_millis(33))),
                    }
                };
                app.next_redraw = delay.and_then(|delay| Instant::now().checked_add(delay));
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(app) = self.app.as_mut() {
            if app.next_redraw.is_some_and(|next| next <= Instant::now()) {
                app.window.request_redraw();
                app.next_redraw = None;
            }
            event_loop.set_control_flow(match app.next_redraw {
                Some(next) => winit::event_loop::ControlFlow::WaitUntil(next),
                None => winit::event_loop::ControlFlow::Wait,
            });
        }
    }
}

fn generation_refresh_interval(panel_visible: bool, paused: bool, finished: bool) -> Duration {
    if paused || finished {
        Duration::from_millis(250)
    } else if panel_visible {
        Duration::from_millis(67)
    } else {
        Duration::from_millis(100)
    }
}

fn dialog_owner(window: &winit::window::Window) -> usize {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get() as usize,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a real GPU; renders the generation screen and verifies visible shapes"]
    fn generation_canvas_stays_visible_beside_controls() {
        use crate::types::{CandidateParams, ShapeLayer};
        let settings = Settings {
            batch_size: 32,
            shape_resolution: 128,
            ..Settings::default()
        };
        let shape = image::load_from_memory(include_bytes!("../assets/teekasfigure.png"))
            .unwrap()
            .resize_exact(128, 128, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let shapes = [ShapeLayer {
            pixels: shape.into_raw(),
        }];
        let output_gpu =
            GpuContext::new(&vec![0; 1000 * 780 * 4], (1000, 780), &shapes, &settings).unwrap();
        let gpu = Arc::new(
            GpuContext::new_from_device(
                output_gpu.device.clone(),
                output_gpu.queue.clone(),
                wgpu::TextureFormat::Rgba8Unorm,
                &vec![0; 256 * 256 * 4],
                (256, 256),
                &shapes,
                &settings,
            )
            .unwrap(),
        );
        // A black opening must finish immediately without evaluating millions of candidates.
        let mut generator =
            CandidateGenerator::new(settings.clone(), vec![0; 256 * 256 * 4], (256, 256));
        let mut climber = HillClimber::new();
        assert!(matches!(
            climber.step(&gpu, &mut generator, &settings),
            StepResult::Completed
        ));
        assert_eq!(gpu.control.evaluated.load(Ordering::Relaxed), 0);
        assert_eq!(climber.placed_shapes, 0);
        assert_eq!(climber.current_mse, 0.0);
        assert!(io::output::read_canvas_image(&gpu)
            .unwrap()
            .pixels()
            .all(|p| p.0 == [0, 0, 0, 255]));
        // Content appearing after the black frame must re-enable evolution.
        generator = CandidateGenerator::new(settings.clone(), vec![255; 256 * 256 * 4], (256, 256));
        assert!(!generator.empty_canvas_is_solution());
        let forced = Settings {
            use_min_improvement: false,
            ..settings.clone()
        };
        assert!(!CandidateGenerator::new(forced, vec![0; 4], (1, 1)).empty_canvas_is_solution());
        gpu.composite_shape(&CandidateParams {
            shape_index: 0,
            x: 128.0,
            y: 128.0,
            rotation: 0.0,
            scale: 1.7,
            scale_y: 1.7,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            alpha: 1.0,
            use_original_color: 1.0,
            hue_shift: 0.0,
            saturation_scale: 1.0,
            brightness_scale: 1.0,
            _padding: [0.0; 2],
        });
        gpu.control.placed.store(1, Ordering::Relaxed);
        gpu.control.begin_search(10);
        gpu.control.searched(-2.0);
        let overlay = OverlayState::with_language(4000, false, Language::Russian);
        let mut view = GenerationView {
            gpu: gpu.clone(),
            overlay: overlay.clone(),
            latest: Arc::new(Mutex::new(overlay)),
            worker: std::thread::spawn(|| {}),
            settings,
            last_metrics: (Instant::now(), 0, 0),
            rate: 133444.0,
            evaluation_percent: 94.0,
            source_name: "Проверка видимого холста.png".into(),
            panel_visible: true,
        };
        let ctx = egui::Context::default();
        let mut renderer =
            egui_wgpu::Renderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, None, 1, false);
        for iteration in 0..6 {
            view.panel_visible = iteration < 3;
            let mut canvas_rect = None;
            let result = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    view.render_controls(
                        ctx,
                        "NVIDIA GeForce RTX 3050 Ti Laptop GPU · Vulkan",
                        crate::monitor::Sample::default(),
                    );
                    canvas_rect = Some(ctx.available_rect());
                    view.overlay.render(ctx);
                },
            );
            let rect = canvas_rect.unwrap();
            if view.panel_visible {
                assert!(
                    rect.width() > 500.0 && rect.width() < 740.0,
                    "Canvas area: {rect:?}"
                );
            } else {
                assert_eq!(
                    rect.width(),
                    1000.0,
                    "Hiding the panel restores the full canvas width"
                );
            }
            let jobs = ctx.tessellate(result.shapes, result.pixels_per_point);
            let descriptor = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [1000, 780],
                pixels_per_point: 1.0,
            };
            for (id, delta) in &result.textures_delta.set {
                renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            gpu.blit_canvas_to_surface(
                &mut encoder,
                &output_gpu.canvas_view,
                [rect.min.x, rect.min.y, rect.width(), rect.height()],
            );
            renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &descriptor);
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &output_gpu.canvas_view,
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
                renderer.render(&mut pass.forget_lifetime(), &jobs, &descriptor);
            }
            gpu.queue.submit(Some(encoder.finish()));
            for id in result.textures_delta.free {
                renderer.free_texture(&id);
            }
            if iteration == 2 {
                io::output::read_canvas_image(&output_gpu)
                    .unwrap()
                    .save("target/generation-preview.png")
                    .unwrap();
            }
        }
        let image = io::output::read_canvas_image(&output_gpu).unwrap();
        let visible_pixels = image
            .enumerate_pixels()
            .filter(|(x, y, p)| *x < 650 && *y > 200 && (p[0] > 60 || p[1] > 60 || p[2] > 60))
            .count();
        assert!(
            visible_pixels > 10_000,
            "The canvas should contain visible, unobscured shapes"
        );
        image.save("target/generation-panel-hidden.png").unwrap();
    }

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
