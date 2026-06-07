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

/// Generate the progress-GIF filename: `{source_stem}_process.gif`
pub fn process_gif_filename(source_path: &Path) -> String {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    format!("{}_process.gif", stem)
}

/// Read the current canvas texture from the GPU into an RGBA image (CPU-side).
///
/// Handles the 256-byte row-alignment that wgpu requires for texture→buffer
/// copies and strips the padding. Shared by [`save_canvas_png`] and the
/// progress-GIF capture path.
pub fn read_canvas_image(gpu: &GpuContext) -> Result<image::RgbaImage, AppError> {
    let (width, height) = gpu.canvas_size;
    let unpadded_bytes_per_row = width * 4;
    let padded_bytes_per_row = align_to(unpadded_bytes_per_row, 256);
    let buffer_size = (padded_bytes_per_row * height) as u64;

    let staging_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Canvas Readback Buffer"),
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

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
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + (unpadded_bytes_per_row) as usize;
        pixels.extend_from_slice(&data[start..end]);
    }
    drop(data);
    staging_buffer.unmap();

    image::RgbaImage::from_raw(width, height, pixels).ok_or_else(|| AppError::SaveFailed {
        reason: "Failed to create image from canvas pixel data".to_string(),
    })
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
    let img = read_canvas_image(gpu)?;

    img.save(&output_path).map_err(|e| AppError::SaveFailed {
        reason: format!("Failed to save PNG to '{}': {}", output_path.display(), e),
    })?;

    Ok(output_path)
}

/// Encode a sequence of frames as an animated, infinitely-looping GIF.
///
/// All frames should share the same dimensions (the capture path downscales
/// them to a common width). `fps` controls playback speed (frame delay).
pub fn save_progress_gif(
    frames: &[image::RgbaImage],
    output_folder: &Path,
    filename: &str,
    fps: u32,
) -> Result<PathBuf, AppError> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};

    if frames.is_empty() {
        return Err(AppError::SaveFailed {
            reason: "no frames captured for progress GIF".to_string(),
        });
    }

    std::fs::create_dir_all(output_folder).map_err(|e| AppError::SaveFailed {
        reason: format!("Failed to create output folder '{}': {}", output_folder.display(), e),
    })?;
    let output_path = output_folder.join(filename);

    let file = std::fs::File::create(&output_path).map_err(|e| AppError::SaveFailed {
        reason: format!("Failed to create GIF '{}': {}", output_path.display(), e),
    })?;

    // Speed 1 (best quality) .. 30 (fastest). 15 balances size/quality/time.
    let mut encoder = GifEncoder::new_with_speed(file, 15);
    encoder
        .set_repeat(Repeat::Infinite)
        .map_err(|e| AppError::SaveFailed {
            reason: format!("Failed to set GIF repeat: {}", e),
        })?;

    let fps = fps.max(1);
    let delay = Delay::from_numer_denom_ms(1000, fps);
    for img in frames {
        let frame = Frame::from_parts(img.clone(), 0, 0, delay);
        encoder.encode_frame(frame).map_err(|e| AppError::SaveFailed {
            reason: format!("Failed to encode GIF frame: {}", e),
        })?;
    }
    // Drop the encoder to flush/finish the file.
    drop(encoder);

    Ok(output_path)
}

/// Downscale an image so its width does not exceed `max_width`, preserving
/// aspect ratio. Returns the image unchanged if it already fits.
pub fn downscale_to_width(img: image::RgbaImage, max_width: u32) -> image::RgbaImage {
    if img.width() <= max_width || max_width == 0 {
        return img;
    }
    let new_h = ((img.height() as u64 * max_width as u64) / img.width() as u64).max(1) as u32;
    image::imageops::resize(&img, max_width, new_h, image::imageops::FilterType::Triangle)
}

/// Align a value up to the given alignment (must be a power of 2).
fn align_to(value: u32, alignment: u32) -> u32 {
    (value + alignment - 1) & !(alignment - 1)
}

/// True if `name` matches the `frame_<digits>.png` output-sequence pattern
/// (e.g. `frame_00042.png`).
fn is_frame_sequence_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("frame_") else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(".png") else {
        return false;
    };
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// Delete leftover `frame_NNNNN.png` files from a previous video run.
///
/// Video frames are written as `frame_00000.png`, `frame_00001.png`, … and
/// later encoded by FFmpeg via the `frame_%05d.png` pattern. If a previous run
/// produced MORE frames than the current one, the stale higher-numbered files
/// would still be on disk and FFmpeg would append them to the new video (e.g. a
/// new 200-frame run after an old 400-frame run yields a 400-frame mix). Always
/// clearing them before a run guarantees the encoded video contains only the
/// frames of the current run.
///
/// Only files matching the exact `frame_<digits>.png` pattern are removed;
/// `*_result.png/.mp4` and `snapshot_*.png` are left untouched. Returns the
/// number of files removed.
pub fn clean_frame_sequence(output_folder: &Path) -> usize {
    let read_dir = match std::fs::read_dir(output_folder) {
        Ok(rd) => rd,
        Err(_) => return 0, // folder doesn't exist yet — nothing to clean
    };

    let mut removed = 0;
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if is_frame_sequence_name(name) && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
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

    #[test]
    fn test_is_frame_sequence_name() {
        assert!(is_frame_sequence_name("frame_00000.png"));
        assert!(is_frame_sequence_name("frame_42.png"));
        assert!(is_frame_sequence_name("frame_00399.png"));
        // Non-matching names must be ignored.
        assert!(!is_frame_sequence_name("frame_.png"));
        assert!(!is_frame_sequence_name("frame_12.jpg"));
        assert!(!is_frame_sequence_name("frame_12a.png"));
        assert!(!is_frame_sequence_name("snapshot_20240101_000000.png"));
        assert!(!is_frame_sequence_name("video_result.png"));
        assert!(!is_frame_sequence_name("frame.png"));
    }

    #[test]
    fn test_clean_frame_sequence_removes_only_frames() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        // Stale frame sequence + files that must survive.
        std::fs::write(p.join("frame_00000.png"), b"x").unwrap();
        std::fs::write(p.join("frame_00001.png"), b"x").unwrap();
        std::fs::write(p.join("frame_00399.png"), b"x").unwrap();
        std::fs::write(p.join("video_result.mp4"), b"x").unwrap();
        std::fs::write(p.join("video_result.png"), b"x").unwrap();
        std::fs::write(p.join("snapshot_20240101_000000.png"), b"x").unwrap();

        let removed = clean_frame_sequence(p);
        assert_eq!(removed, 3);
        assert!(!p.join("frame_00000.png").exists());
        assert!(!p.join("frame_00399.png").exists());
        // Survivors untouched.
        assert!(p.join("video_result.mp4").exists());
        assert!(p.join("video_result.png").exists());
        assert!(p.join("snapshot_20240101_000000.png").exists());
    }

    #[test]
    fn test_clean_frame_sequence_missing_folder_is_safe() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does_not_exist");
        assert_eq!(clean_frame_sequence(&missing), 0);
    }
}
