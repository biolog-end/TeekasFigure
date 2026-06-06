mod algorithm;
mod app;
mod error;
mod gpu;
mod io;
mod overlay;
mod settings;
mod types;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use winit::event_loop::EventLoop;
use winit::window::Window;

use crate::algorithm::{CandidateGenerator, HillClimber};
use crate::app::{compute_window_size, App};
use crate::error::AppError;
use crate::gpu::GpuContext;
use crate::io::media_loader::MediaType;
use crate::overlay::OverlayState;
use crate::settings::Settings;

fn main() {
    env_logger::init();
    log::info!("GPU Image Approximator starting...");

    if let Err(e) = run() {
        log::error!("Fatal error: {}", e);
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), AppError> {
    // 1. Get base directory (next to executable)
    let base_dir = io::media_loader::get_base_dir();
    log::info!("Base directory: {}", base_dir.display());

    // 2. Ensure directories exist
    let created_dirs = io::media_loader::ensure_directories(&base_dir)?;
    if !created_dirs.is_empty() {
        log::info!("Created directories: {:?}", created_dirs);
    }

    // 3. Load settings
    let settings_path = base_dir.join("settings.toml");
    let settings = Settings::load_or_create(&settings_path)?;
    settings.validate()?;
    log::info!("Settings loaded and validated");

    // 4. Detect input media (poll if not found)
    let media_folder = base_dir.join("input_media");
    let media_path = poll_for_media(&media_folder)?;
    log::info!("Media detected: {}", media_path.display());

    // 5. Load media
    let media = io::media_loader::load_media(&media_path)?;
    let (target_data, target_size, is_video) = match media {
        MediaType::Image(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            (rgba.into_raw(), (w, h), false)
        }
        MediaType::Video(video_path) => {
            // Extract first frame from video using FFmpeg
            log::info!("Video detected, extracting first frame via FFmpeg...");
            let output = std::process::Command::new("ffmpeg")
                .args([
                    "-i", video_path.to_str().unwrap_or(""),
                    "-vframes", "1",
                    "-f", "image2pipe",
                    "-pix_fmt", "rgba",
                    "-vcodec", "rawvideo",
                    "-v", "quiet",
                    "-",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .map_err(|e| AppError::NoMedia {
                    path: video_path.clone(),
                })?;

            if output.stdout.is_empty() {
                // Fallback: use ffmpeg to extract as PNG then load
                let temp_frame = std::env::temp_dir().join("gpu_approx_frame0.png");
                let _ = std::process::Command::new("ffmpeg")
                    .args([
                        "-y",
                        "-i", video_path.to_str().unwrap_or(""),
                        "-vframes", "1",
                        "-v", "quiet",
                        temp_frame.to_str().unwrap_or(""),
                    ])
                    .output();

                if temp_frame.exists() {
                    let img = image::open(&temp_frame).map_err(|_| AppError::NoMedia {
                        path: video_path.clone(),
                    })?;
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    let _ = std::fs::remove_file(&temp_frame);
                    (rgba.into_raw(), (w, h), true)
                } else {
                    return Err(AppError::NoMedia {
                        path: video_path,
                    });
                }
            } else {
                // Parse raw RGBA frame - need dimensions from ffprobe
                let probe = std::process::Command::new("ffprobe")
                    .args([
                        "-v", "quiet",
                        "-select_streams", "v:0",
                        "-show_entries", "stream=width,height",
                        "-of", "csv=p=0",
                        video_path.to_str().unwrap_or(""),
                    ])
                    .output()
                    .map_err(|_| AppError::NoMedia { path: video_path.clone() })?;

                let dims = String::from_utf8_lossy(&probe.stdout);
                let parts: Vec<&str> = dims.trim().split(',').collect();
                if parts.len() < 2 {
                    return Err(AppError::NoMedia { path: video_path });
                }
                let w: u32 = parts[0].parse().unwrap_or(640);
                let h: u32 = parts[1].parse().unwrap_or(480);

                let expected_size = (w * h * 4) as usize;
                let raw_data = if output.stdout.len() >= expected_size {
                    output.stdout[..expected_size].to_vec()
                } else {
                    return Err(AppError::NoMedia { path: video_path });
                };

                (raw_data, (w, h), true)
            }
        }
    };
    log::info!(
        "Media loaded: {}x{}, is_video={}",
        target_size.0,
        target_size.1,
        is_video
    );

    // 6. Load and preprocess shapes
    let shapes_folder = base_dir.join("input_shapes");
    let shapes =
        io::shape_preprocessor::load_and_preprocess(&shapes_folder, settings.shape_resolution)?;
    io::shape_preprocessor::check_vram_budget(
        settings.shape_resolution,
        shapes.len() as u32,
        settings.vram_budget_mb,
    )?;
    log::info!("Loaded {} shape(s)", shapes.len());

    // 7. Create event loop and window
    let event_loop = EventLoop::new().map_err(|e| {
        AppError::GpuInit(format!("Failed to create event loop: {}", e))
    })?;

    // Use a default display size initially; we'll get the actual size from the window's monitor
    let display_size = (1920u32, 1080u32);
    let window_size = compute_window_size(target_size, display_size);
    log::info!(
        "Window size: {}x{} (display: {}x{})",
        window_size.0,
        window_size.1,
        display_size.0,
        display_size.1
    );

    let window_attrs = Window::default_attributes()
        .with_title("GPU Image Approximator - Running")
        .with_inner_size(winit::dpi::PhysicalSize::new(window_size.0, window_size.1))
        .with_resizable(true);

    #[allow(deprecated)]
    let window = Arc::new(
        event_loop
            .create_window(window_attrs)
            .map_err(|e| AppError::GpuInit(format!("Failed to create window: {}", e)))?,
    );

    // Now that we have a window, try to get the actual monitor size and resize if needed
    let actual_display_size = window
        .current_monitor()
        .map(|m| {
            let size = m.size();
            (size.width, size.height)
        })
        .unwrap_or(display_size);

    let window_size = if actual_display_size != display_size {
        let new_size = compute_window_size(target_size, actual_display_size);
        if new_size != window_size {
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(
                new_size.0, new_size.1,
            ));
        }
        log::info!(
            "Adjusted window size: {}x{} (actual display: {}x{})",
            new_size.0,
            new_size.1,
            actual_display_size.0,
            actual_display_size.1
        );
        new_size
    } else {
        window_size
    };

    // 8. Create wgpu instance and surface, then initialize GPU context
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let surface = instance
        .create_surface(window.clone())
        .map_err(|e| AppError::GpuInit(format!("Failed to create surface: {}", e)))?;

    let gpu = GpuContext::new_with_surface(
        &instance,
        &surface,
        &target_data,
        target_size,
        &shapes,
        &settings,
    )?;

    // 9. Initialize egui
    let egui_ctx = egui::Context::default();
    let viewport_id = egui_ctx.viewport_id();
    let egui_state = egui_winit::State::new(
        egui_ctx.clone(),
        viewport_id,
        &window,
        None,
        None,
        None,
    );

    let surface_format = gpu.surface_format;
    let egui_renderer =
        egui_wgpu::Renderer::new(&gpu.device, surface_format, None, 1, false);

    // 10. Create algorithm components
    let climber = HillClimber::new();
    let generator = CandidateGenerator::new(settings.clone(), target_data, target_size);
    let overlay = OverlayState::new(settings.max_shapes, is_video);

    // 11. Create App and run
    let output_folder = base_dir.join("output");
    let mut app = App::new(
        gpu,
        climber,
        generator,
        overlay,
        settings.clone(),
        window,
        window_size,
        surface,
        egui_state,
        egui_renderer,
        media_path.clone(),
        output_folder.clone(),
    );

    // If video mode, set up the video decoder
    if is_video {
        // Clear any leftover frame_NNNNN.png files from a PREVIOUS video run.
        // Frames are encoded via the frame_%05d.png pattern, so stale higher-
        // numbered frames from a longer previous run would otherwise be appended
        // to the new (shorter) video. Clearing guarantees the new video contains
        // only this run's frames.
        let removed = io::output::clean_frame_sequence(&output_folder);
        if removed > 0 {
            log::info!("Removed {} leftover frame_*.png file(s) from a previous run", removed);
        }

        let output_video = output_folder.join(
            format!("{}_result.mp4",
                media_path.file_stem().and_then(|s| s.to_str()).unwrap_or("video"))
        );
        match crate::io::video::VideoProcessor::new(&media_path, &output_video, settings.target_fps) {
            Ok(decoder) => {
                // Skip first frame (we already extracted it)
                app.video_decoder = Some(decoder);
                log::info!("Video decoder initialized");
            }
            Err(e) => {
                log::warn!("Failed to init video decoder: {}. Processing as single image.", e);
            }
        }
    }

    app.run(event_loop);

    Ok(())
}

/// Poll the input_media folder every 2 seconds until a supported file is found.
/// Prints a message to stderr on first poll failure.
fn poll_for_media(media_folder: &PathBuf) -> Result<PathBuf, AppError> {
    if let Some(path) = io::media_loader::find_first_supported_file(media_folder) {
        return Ok(path);
    }

    eprintln!(
        "No supported media file found in '{}'. Waiting for input...",
        media_folder.display()
    );
    log::warn!(
        "No media found in '{}', polling every 2 seconds",
        media_folder.display()
    );

    loop {
        thread::sleep(Duration::from_secs(2));
        if let Some(path) = io::media_loader::find_first_supported_file(media_folder) {
            return Ok(path);
        }
    }
}
