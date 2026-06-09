use std::path::Path;

use image::imageops::FilterType;
use log::warn;

use crate::error::AppError;
use crate::types::ShapeLayer;

/// Maximum number of shape files that can be loaded.
///
/// Each shape becomes one layer of a GPU 2D texture array, so this is also
/// bounded by the device's `max_texture_array_layers`. We request the adapter's
/// full limits at device creation (see `gpu::context`), so on typical desktop
/// GPUs up to 2048 layers are available; `begin_generation` additionally clamps
/// the loaded shape count to the device's real limit at runtime.
const MAX_SHAPES: usize = 2048;

/// Load shape image files from a folder, resize them to the given resolution,
/// and convert to grayscale with preserved alpha (or keep original colors).
///
/// Accepts any image format supported by the `image` crate (PNG, JPG/JPEG,
/// WebP, BMP, GIF, TIFF, TGA, ICO, QOI, PNM, …) — see
/// [`crate::io::media_loader::SUPPORTED_IMAGE_EXTENSIONS`].
///
/// When `preserve_color` is false (default behaviour) each shape is converted
/// to a grayscale brush with its alpha preserved — the algorithm later tints it.
/// When `preserve_color` is true the shapes keep their original RGB **and**
/// alpha (for the `use_original_colors` mode): images with a real alpha channel
/// keep their transparency, while fully-opaque photos (JPEG/BMP) stay opaque.
///
/// Returns up to `MAX_SHAPES` shape layers sorted alphabetically by filename.
/// Skips undecodable files with a warning. Returns an error if no valid shapes
/// are found.
pub fn load_and_preprocess(
    folder: &Path,
    shape_resolution: u32,
    preserve_color: bool,
) -> Result<Vec<ShapeLayer>, AppError> {
    // Collect supported image file paths sorted alphabetically.
    let mut shape_paths: Vec<std::path::PathBuf> = std::fs::read_dir(folder)
        .map_err(|_| AppError::NoShapes {
            path: folder.to_path_buf(),
        })?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(crate::io::media_loader::is_supported_image_extension)
                .unwrap_or(false)
        })
        .collect();

    shape_paths.sort();

    if shape_paths.is_empty() {
        return Err(AppError::NoShapes {
            path: folder.to_path_buf(),
        });
    }

    if shape_paths.len() > MAX_SHAPES {
        warn!(
            "Found {} shape images in '{}', loading only the first {} (alphabetical order)",
            shape_paths.len(),
            folder.display(),
            MAX_SHAPES
        );
        shape_paths.truncate(MAX_SHAPES);
    }

    let mut layers = Vec::new();

    for path in &shape_paths {
        match image::open(path) {
            Ok(img) => {
                let resized = img.resize_exact(
                    shape_resolution,
                    shape_resolution,
                    FilterType::Triangle, // bilinear interpolation
                );

                let rgba = resized.to_rgba8();
                let pixels = if preserve_color {
                    convert_to_color_alpha(&rgba, shape_resolution)
                } else {
                    convert_to_grayscale_alpha(&rgba, shape_resolution)
                };

                layers.push(ShapeLayer { pixels });
            }
            Err(e) => {
                warn!(
                    "Skipping undecodable image '{}': {}",
                    path.display(),
                    e
                );
            }
        }
    }

    if layers.is_empty() {
        return Err(AppError::NoShapes {
            path: folder.to_path_buf(),
        });
    }

    Ok(layers)
}

/// Check if the texture array fits within the VRAM budget.
/// Returns Ok(()) if within budget, or AppError::VramBudget if exceeded.
pub fn check_vram_budget(
    shape_resolution: u32,
    num_layers: u32,
    vram_budget_mb: u32,
) -> Result<(), AppError> {
    let total_bytes =
        (shape_resolution as u64) * (shape_resolution as u64) * 4 * (num_layers as u64);
    let budget_bytes = (vram_budget_mb as u64) * 1024 * 1024;
    if total_bytes > budget_bytes {
        let required_mb = ((total_bytes + 1024 * 1024 - 1) / (1024 * 1024)) as u32;
        return Err(AppError::VramBudget {
            required_mb,
            budget_mb: vram_budget_mb,
        });
    }
    Ok(())
}

/// Convert an RGBA8 image to grayscale with preserved alpha.
/// Luminance = round(0.2126 × R + 0.7152 × G + 0.0722 × B)
/// Output pixel: [luminance, luminance, luminance, original_alpha]
fn convert_to_grayscale_alpha(
    img: &image::RgbaImage,
    shape_resolution: u32,
) -> Vec<u8> {
    let pixel_count = (shape_resolution * shape_resolution) as usize;
    let mut output = Vec::with_capacity(pixel_count * 4);

    for pixel in img.pixels() {
        let [r, g, b, a] = pixel.0;
        let luminance = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64).round() as u8;
        output.push(luminance);
        output.push(luminance);
        output.push(luminance);
        output.push(a);
    }

    output
}

/// Keep an RGBA8 image's ORIGINAL colors and alpha.
///
/// The image's own alpha channel is preserved verbatim: a PNG/WebP with real
/// transparency keeps it, and a fully-opaque photo (JPEG/BMP, or any image
/// whose every pixel is opaque) stays fully opaque. This means photographic
/// shapes in `use_original_colors` mode are placed as solid rectangles rather
/// than being faded by their brightness.
fn convert_to_color_alpha(
    img: &image::RgbaImage,
    shape_resolution: u32,
) -> Vec<u8> {
    let pixel_count = (shape_resolution * shape_resolution) as usize;
    let mut output = Vec::with_capacity(pixel_count * 4);

    for pixel in img.pixels() {
        let [r, g, b, a] = pixel.0;
        output.push(r);
        output.push(g);
        output.push(b);
        output.push(a);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_vram_budget_within_budget() {
        // 128² × 4 × 256 = 16 MB, budget = 2048 MB → OK
        assert!(check_vram_budget(128, 256, 2048).is_ok());
    }

    #[test]
    fn test_check_vram_budget_exactly_at_limit() {
        // 512² × 4 × 1 = 1 MB, budget = 1 MB → OK (equal is within budget)
        assert!(check_vram_budget(512, 1, 1).is_ok());
    }

    #[test]
    fn test_check_vram_budget_exceeds_budget() {
        // 1024² × 4 × 256 = 1024 MB, budget = 128 MB → Error
        let result = check_vram_budget(1024, 256, 128);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::VramBudget {
                required_mb,
                budget_mb,
            } => {
                assert_eq!(required_mb, 1024);
                assert_eq!(budget_mb, 128);
            }
            _ => panic!("Expected VramBudget error"),
        }
    }

    #[test]
    fn test_check_vram_budget_minimal_values() {
        // 16² × 4 × 1 = 1024 bytes, budget = 128 MB → OK
        assert!(check_vram_budget(16, 1, 128).is_ok());
    }

    #[test]
    fn test_check_vram_budget_just_over_limit() {
        // 512² × 4 × 2 = 2 MB, budget = 1 MB → Error
        let result = check_vram_budget(512, 2, 1);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::VramBudget {
                required_mb,
                budget_mb,
            } => {
                assert_eq!(required_mb, 2);
                assert_eq!(budget_mb, 1);
            }
            _ => panic!("Expected VramBudget error"),
        }
    }

    #[test]
    fn test_check_vram_budget_large_resolution_overflow_safe() {
        // 1024² × 4 × 256 = 1,073,741,824 bytes = 1024 MB
        // budget = 4096 MB → OK (tests that u64 arithmetic doesn't overflow)
        assert!(check_vram_budget(1024, 256, 4096).is_ok());
    }

    #[test]
    fn test_check_vram_budget_required_mb_rounds_up() {
        // 513² × 4 × 1 = 1,052,676 bytes = 1.004 MB → required_mb should be 2 (rounded up)
        // budget = 1 MB → Error
        let result = check_vram_budget(513, 1, 1);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::VramBudget {
                required_mb,
                budget_mb,
            } => {
                // 513² × 4 = 1,052,676 bytes, ceil(1,052,676 / 1,048,576) = 2
                assert_eq!(required_mb, 2);
                assert_eq!(budget_mb, 1);
            }
            _ => panic!("Expected VramBudget error"),
        }
    }
}
