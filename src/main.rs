mod algorithm;
mod app;
mod error;
mod gpu;
mod io;
mod overlay;
mod settings;
mod types;
mod ui;

use std::sync::Arc;

use winit::event_loop::EventLoop;
use winit::window::Window;

use crate::app::App;
use crate::error::AppError;
use crate::gpu::GpuContext;
use crate::settings::Settings;

fn main() {
    env_logger::init();
    log::info!("TeekasFigure starting...");

    if let Err(e) = run() {
        log::error!("Fatal error: {}", e);
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), AppError> {
    // 1. Base directory (next to the executable).
    let base_dir = io::media_loader::get_base_dir();
    log::info!("Base directory: {}", base_dir.display());

    // 2. Ensure the working directories exist.
    let created_dirs = io::media_loader::ensure_directories(&base_dir)?;
    if !created_dirs.is_empty() {
        log::info!("Created directories: {:?}", created_dirs);
    }

    // 3. Load settings.toml (the UI pulls its initial values from here). An
    //    invalid file is not fatal — the Settings screen lets the user fix it
    //    and revalidates before starting.
    let settings_path = base_dir.join("settings.toml");
    let settings = Settings::load_or_create(&settings_path)?;
    if let Err(e) = settings.validate() {
        log::warn!("settings.toml has invalid values ({}). You can fix them in the UI.", e);
    } else {
        log::info!("Settings loaded and validated");
    }

    // 4. Load the saved UI language preference.
    let language = ui::i18n::load_language(&base_dir);

    // 5. Event loop + window. The window starts at a comfortable size for the
    //    Settings form and is resized to the media when generation starts.
    let event_loop = EventLoop::new()
        .map_err(|e| AppError::GpuInit(format!("Failed to create event loop: {}", e)))?;

    let window_attrs = Window::default_attributes()
        .with_title("TeekasFigure - Settings")
        .with_inner_size(winit::dpi::PhysicalSize::new(1000u32, 780u32))
        .with_resizable(true);

    #[allow(deprecated)]
    let window = Arc::new(
        event_loop
            .create_window(window_attrs)
            .map_err(|e| AppError::GpuInit(format!("Failed to create window: {}", e)))?,
    );

    // 6. Instance + surface, then a device/queue compatible with the surface.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let surface = instance
        .create_surface(window.clone())
        .map_err(|e| AppError::GpuInit(format!("Failed to create surface: {}", e)))?;

    let (device, queue, surface_format) =
        pollster::block_on(GpuContext::init_device_with_surface(&instance, &surface))?;
    let device = Arc::new(device);
    let queue = Arc::new(queue);

    // 7. egui setup using the shared device.
    let egui_ctx = egui::Context::default();
    let viewport_id = egui_ctx.viewport_id();
    let egui_state =
        egui_winit::State::new(egui_ctx.clone(), viewport_id, &window, None, None, None);
    let egui_renderer = egui_wgpu::Renderer::new(&device, surface_format, None, 1, false);

    // 8. Build the app on the Settings screen and run.
    let app = App::new(
        device,
        queue,
        surface,
        surface_format,
        window,
        egui_state,
        egui_renderer,
        egui_ctx,
        base_dir,
        settings,
        language,
    );

    app.run(event_loop);

    Ok(())
}
