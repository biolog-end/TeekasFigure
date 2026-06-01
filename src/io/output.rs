// Output saving: PNG encoding, filename derivation, and folder management

use std::path::{Path, PathBuf};

use chrono::Local;

use crate::error::AppError;
use crate::gpu::GpuContext;

/// Generate the completion filename: `{source_stem}_result.png`
///
/// Extracts the file stem from the source path and appends "_result.png".
/// Falls back to "output_result.png" if the stem cannot be determined.
pub fn completion_filename(source_path: &Path) -> String {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    format!("{}_result.png", stem)
}

/// Generate the snapshot filename: `snapshot_{YYYYMMDD_HHMMSS}.png`
///
/// Uses the current local time to produce a unique timestamp-based filename.
pub fn snapshot_filename() -> String {
    let now = Local::now();
    format!("snapshot_{}.png", now.format("%Y%m%d_%H%M%S"))
}

/// Save the current canvas as a PNG file.
///
/// Reads the canvas texture from GPU to CPU, encodes as PNG, and writes to disk.
/// Creates the output folder if it doesn't exist. Overwrites existing files.
///
/// # Arguments
/// * `gpu` - The GPU context holding the canvas texture
/// * `output_folder` - Path to the output directory
/// * `filename` - The filename to save as (e.g., "photo_result.png")
///
/// # Returns
/// The full path to the saved file on success, or `AppError::SaveFailed` on failure.
pub fn save_canvas_png(
    gpu: &GpuContext,
    output_folder: &Path,
    filename: &str,
) -> Result<PathBuf, AppError> {
    // Ensure output folder exists (create if missing, including parents)
    std::fs::create_dir_all(output_folder).map_err(|e| AppError::SaveFailed {
        reason: format!("Failed to create output folder '{}': {}", output_folder.display(), e),
    })?;

    let output_path = output_folder.join(filename);

    // Read canvas texture to CPU
    let (width, height) = gpu.canvas_size;
    // wgpu requires rows to be aligned to 256 bytes (COPY_BYTES_PER_ROW_ALIGNMENT)
    let unpadded_bytes_per_row = width * 4;
    let padded_bytes_per_row = align_to(unpadded_bytes_per_row, 256);
    let buffer_size = (padded_bytes_per_row * height) as u64;

    let staging_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Canvas Readback Buffer"),
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Copy canvas texture to staging buffer
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Canvas Readback Encoder"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture: &gpu.canvas,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &staging_buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(std::iter::once(encoder.finish()));

    // Map and read the buffer
    let buffer_slice = staging_buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).unwrap();
    });
    gpu.device.poll(wgpu::Maintain::Wait);
    receiver
        .recv()
        .map_err(|e| AppError::SaveFailed {
            reason: format!("Failed to receive buffer map result: {}", e),
        })?
        .map_err(|e| AppError::SaveFailed {
            reason: format!("Failed to map canvas buffer: {:?}", e),
        })?;

    let data = buffer_slice.get_mapped_range();

    // Remove row padding and create a contiguous pixel buffer
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + (unpadded_bytes_per_row) as usize;
        pixels.extend_from_slice(&data[start..end]);
    }
    drop(data);
    staging_buffer.unmap();

    // Encode as PNG and save
    let img = image::RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
        AppError::SaveFailed {
            reason: "Failed to create image from canvas pixel data".to_string(),
        }
    })?;

    img.save(&output_path).map_err(|e| AppError::SaveFailed {
        reason: format!("Failed to save PNG to '{}': {}", output_path.display(), e),
    })?;

    Ok(output_path)
}

/// Align a value up to the given alignment (must be a power of 2).
fn align_to(value: u32, alignment: u32) -> u32 {
    (value + alignment - 1) & !(alignment - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_completion_filename_basic() {
        let path = Path::new("input_media/photo.png");
        assert_eq!(completion_filename(path), "photo_result.png");
    }

    #[test]
    fn test_completion_filename_with_directory() {
        let path = Path::new("/some/deep/path/landscape.jpg");
        assert_eq!(completion_filename(path), "landscape_result.png");
    }

    #[test]
    fn test_completion_filename_no_extension() {
        let path = Path::new("myfile");
        assert_eq!(completion_filename(path), "myfile_result.png");
    }

    #[test]
    fn test_completion_filename_multiple_dots() {
        let path = Path::new("my.photo.final.png");
        assert_eq!(completion_filename(path), "my.photo.final_result.png");
    }

    #[test]
    fn test_completion_filename_empty_path_fallback() {
        // An empty path has no file_stem, so we fall back to "output"
        let path = Path::new("");
        assert_eq!(completion_filename(path), "output_result.png");
    }

    #[test]
    fn test_snapshot_filename_format() {
        let filename = snapshot_filename();
        // Should match pattern: snapshot_YYYYMMDD_HHMMSS.png
        assert!(filename.starts_with("snapshot_"));
        assert!(filename.ends_with(".png"));
        // The timestamp portion should be 15 characters: YYYYMMDD_HHMMSS
        let timestamp_part = &filename["snapshot_".len()..filename.len() - ".png".len()];
        assert_eq!(timestamp_part.len(), 15);
        // Verify format: 8 digits, underscore, 6 digits
        let parts: Vec<&str> = timestamp_part.split('_').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 6);
        assert!(parts[0].chars().all(|c| c.is_ascii_digit()));
        assert!(parts[1].chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_align_to() {
        assert_eq!(align_to(256, 256), 256);
        assert_eq!(align_to(257, 256), 512);
        assert_eq!(align_to(1, 256), 256);
        assert_eq!(align_to(512, 256), 512);
        assert_eq!(align_to(100, 256), 256);
        assert_eq!(align_to(0, 256), 0);
    }
}
