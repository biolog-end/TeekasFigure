//! Prepare custom shape images for use with GPU Image Approximator.
//!
//! Takes PNG images from a source folder, converts them to grayscale with
//! preserved alpha (transparent background), resizes to 128x128, and saves
//! them to the input_shapes/ folder.
//!
//! Usage:
//!   cargo run --example prepare_shapes -- <source_folder>
//!
//! If no source folder is specified, uses "raw_shapes/" next to the executable.
//!
//! Input requirements:
//! - PNG files with transparency (alpha channel)
//! - The shape should be white/light on transparent background
//! - Any size (will be resized to 128x128)
//!
//! If your images don't have transparency, the script will treat non-black
//! pixels as the shape (auto-generating alpha from brightness).

use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba};
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let source_folder = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from("raw_shapes")
    };

    let output_folder = PathBuf::from("input_shapes");

    if !source_folder.exists() {
        eprintln!("Source folder '{}' does not exist.", source_folder.display());
        eprintln!("Create it and place your shape PNG files inside.");
        eprintln!();
        eprintln!("Usage: cargo run --example prepare_shapes -- <source_folder>");
        std::process::exit(1);
    }

    std::fs::create_dir_all(&output_folder).expect("Failed to create input_shapes/");

    let mut count = 0;

    let mut entries: Vec<_> = std::fs::read_dir(&source_folder)
        .expect("Failed to read source folder")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    ext.eq_ignore_ascii_case("png")
                        || ext.eq_ignore_ascii_case("jpg")
                        || ext.eq_ignore_ascii_case("jpeg")
                        || ext.eq_ignore_ascii_case("bmp")
                        || ext.eq_ignore_ascii_case("webp")
                })
                .unwrap_or(false)
        })
        .collect();

    entries.sort();

    if entries.is_empty() {
        eprintln!("No image files found in '{}' (supports PNG, JPG, BMP, WebP)", source_folder.display());
        std::process::exit(1);
    }

    for path in &entries {
        match image::open(path) {
            Ok(img) => {
                let processed = process_shape(&img);
                let stem = path.file_stem().unwrap().to_string_lossy();
                let output_path = output_folder.join(format!("{}.png", stem));
                processed.save(&output_path).expect("Failed to save");
                count += 1;
                println!("  Processed: {} -> {}", path.display(), output_path.display());
            }
            Err(e) => {
                eprintln!("  Skipping '{}': {}", path.display(), e);
            }
        }
    }

    println!();
    println!("Done! Processed {} shape(s) into '{}'", count, output_folder.display());
}

/// Process a shape image:
/// 1. If it has meaningful alpha, use it as-is (convert RGB to grayscale, keep alpha)
/// 2. If it has no alpha (all pixels opaque), generate alpha from brightness
/// 3. Resize to 128x128
fn process_shape(img: &DynamicImage) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();

    // Check if the image has meaningful alpha (not all 255)
    let has_alpha = rgba.pixels().any(|p| p.0[3] < 250);

    let processed = if has_alpha {
        // Image has transparency — convert RGB to grayscale, keep alpha
        ImageBuffer::from_fn(w, h, |x, y| {
            let pixel = rgba.get_pixel(x, y);
            let [r, g, b, a] = pixel.0;
            let lum = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64).round() as u8;
            Rgba([lum, lum, lum, a])
        })
    } else {
        // No alpha — generate alpha from brightness (bright = opaque, dark = transparent)
        ImageBuffer::from_fn(w, h, |x, y| {
            let pixel = rgba.get_pixel(x, y);
            let [r, g, b, _] = pixel.0;
            let lum = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64).round() as u8;
            // Use luminance as both the color and the alpha
            Rgba([255, 255, 255, lum])
        })
    };

    let dynamic_processed = image::DynamicImage::ImageRgba8(processed);
    
    // Функция .resize() (в отличие от resize_exact) сохраняет соотношение сторон
    let resized_aspect = dynamic_processed.resize(128, 128, image::imageops::FilterType::Lanczos3);
    let resized_rgba = resized_aspect.to_rgba8();

    // Создаем пустой холст 128x128, полностью прозрачный
    let mut final_img = ImageBuffer::from_pixel(128, 128, Rgba([0, 0, 0, 0]));

    // Вычисляем отступы для центрирования изображения
    let x_offset = (128 - resized_rgba.width()) / 2;
    let y_offset = (128 - resized_rgba.height()) / 2;

    // Накладываем картинку с правильными пропорциями на центр прозрачного квадрата
    image::imageops::overlay(&mut final_img, &resized_rgba, x_offset as i64, y_offset as i64);

    final_img
}
