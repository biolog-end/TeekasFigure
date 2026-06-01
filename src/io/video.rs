use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::error::AppError;

/// Processes video files using FFmpeg for decode and encode.
///
/// Spawns an FFmpeg decode process that outputs raw RGBA frames to stdout,
/// and an FFmpeg encode process that accepts raw RGBA frames on stdin.
pub struct VideoProcessor {
    /// FFmpeg decode process (outputs raw RGBA frames to stdout)
    decoder: Child,
    /// FFmpeg encode process (accepts raw RGBA frames on stdin)
    encoder: Option<Child>,
    /// Input video path
    input_path: PathBuf,
    /// Output video path
    output_path: PathBuf,
    /// Video width
    pub width: u32,
    /// Video height
    pub height: u32,
    /// Frames per second
    pub fps: f64,
    /// Total number of frames (estimated)
    pub total_frames: u32,
    /// Current frame index
    pub frame_index: u32,
}

/// Video metadata detected via ffprobe.
struct VideoInfo {
    width: u32,
    height: u32,
    fps: f64,
    total_frames: u32,
}

impl VideoProcessor {
    /// Create a new VideoProcessor by probing the input video and spawning
    /// FFmpeg decode and encode processes.
    ///
    /// - `input`: path to the source video file
    /// - `output`: path to the output video file
    /// - `target_fps`: resample video to this framerate before processing
    pub fn new(input: &Path, output: &Path, target_fps: u32) -> Result<Self, AppError> {
        let info = Self::probe_video(input)?;

        let decoder = Self::spawn_decoder(input, target_fps)?;
        // Use target_fps for the output as well
        let output_fps = target_fps as f64;
        let encoder = Self::spawn_encoder(output, info.width, info.height, output_fps)?;

        Ok(Self {
            decoder,
            encoder: Some(encoder),
            input_path: input.to_path_buf(),
            output_path: output.to_path_buf(),
            width: info.width,
            height: info.height,
            fps: output_fps,
            total_frames: info.total_frames,
            frame_index: 0,
        })
    }

    /// Read the next raw RGBA frame from the decoder stdout.
    ///
    /// Returns `None` when there are no more frames (EOF).
    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        let frame_size = (self.width as usize) * (self.height as usize) * 4;
        let mut buffer = vec![0u8; frame_size];

        let stdout = self.decoder.stdout.as_mut()?;

        // Read exactly frame_size bytes; partial reads are accumulated.
        let mut bytes_read = 0;
        while bytes_read < frame_size {
            match stdout.read(&mut buffer[bytes_read..]) {
                Ok(0) => {
                    // EOF reached — no more frames
                    return None;
                }
                Ok(n) => {
                    bytes_read += n;
                }
                Err(_) => {
                    return None;
                }
            }
        }

        self.frame_index += 1;
        Some(buffer)
    }

    /// Write a raw RGBA frame to the encoder stdin.
    ///
    /// The `data` slice must be exactly `width * height * 4` bytes.
    pub fn encode_frame(&mut self, data: &[u8]) -> Result<(), AppError> {
        let expected_size = (self.width as usize) * (self.height as usize) * 4;
        if data.len() != expected_size {
            return Err(AppError::Ffmpeg(format!(
                "encode_frame: expected {} bytes, got {}",
                expected_size,
                data.len()
            )));
        }

        if let Some(ref mut encoder) = self.encoder {
            let stdin = encoder.stdin.as_mut().ok_or_else(|| {
                AppError::Ffmpeg("Encoder stdin is not available".to_string())
            })?;

            stdin.write_all(data).map_err(|e| {
                AppError::Ffmpeg(format!("Failed to write frame to encoder: {}", e))
            })?;
        } else {
            return Err(AppError::Ffmpeg(
                "Encoder has already been finalized".to_string(),
            ));
        }

        Ok(())
    }

    /// Finalize the video processing: close encoder stdin, wait for both
    /// FFmpeg processes to finish.
    pub fn finalize(&mut self) -> Result<(), AppError> {
        // Close encoder stdin to signal end of input, then wait for it.
        if let Some(mut encoder) = self.encoder.take() {
            // Drop stdin to close the pipe
            drop(encoder.stdin.take());

            let status = encoder.wait().map_err(|e| {
                AppError::Ffmpeg(format!("Failed to wait for encoder process: {}", e))
            })?;

            if !status.success() {
                return Err(AppError::Ffmpeg(format!(
                    "Encoder process exited with status: {}",
                    status
                )));
            }
        }

        // Wait for decoder to finish (it may already be done if we read all frames).
        let status = self.decoder.wait().map_err(|e| {
            AppError::Ffmpeg(format!("Failed to wait for decoder process: {}", e))
        })?;

        // Decoder may exit with non-zero if we didn't consume all frames — that's OK.
        // Only report error if it was a signal/crash.
        if !status.success() {
            // FFmpeg returns 0 on success, but may return non-zero if pipe was closed early.
            // We treat this as non-fatal since we may not consume all frames.
            log::warn!(
                "Decoder process exited with status: {} (may be normal if not all frames consumed)",
                status
            );
        }

        Ok(())
    }

    /// Probe the input video using ffprobe to detect dimensions, fps, and frame count.
    fn probe_video(input: &Path) -> Result<VideoInfo, AppError> {
        let input_str = input.to_str().ok_or_else(|| {
            AppError::Ffmpeg("Input path contains invalid UTF-8".to_string())
        })?;

        // Get video stream info: width, height, r_frame_rate, nb_frames
        let output = Command::new("ffprobe")
            .args([
                "-v", "quiet",
                "-select_streams", "v:0",
                "-show_entries", "stream=width,height,r_frame_rate,nb_frames",
                "-of", "csv=p=0",
                input_str,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                AppError::Ffmpeg(format!(
                    "Failed to run ffprobe (is FFmpeg installed?): {}",
                    e
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Ffmpeg(format!(
                "ffprobe failed: {}",
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.trim();

        // Expected format: width,height,r_frame_rate,nb_frames
        // e.g.: 1920,1080,30/1,900
        // nb_frames may be "N/A" for some containers
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 3 {
            return Err(AppError::Ffmpeg(format!(
                "Unexpected ffprobe output format: '{}'",
                line
            )));
        }

        let width: u32 = parts[0].parse().map_err(|e| {
            AppError::Ffmpeg(format!("Failed to parse video width '{}': {}", parts[0], e))
        })?;

        let height: u32 = parts[1].parse().map_err(|e| {
            AppError::Ffmpeg(format!("Failed to parse video height '{}': {}", parts[1], e))
        })?;

        let fps = Self::parse_frame_rate(parts[2])?;

        // nb_frames may be "N/A" or missing — estimate from duration if needed
        let total_frames = if parts.len() > 3 && parts[3] != "N/A" {
            parts[3].parse().unwrap_or_else(|_| {
                Self::estimate_frame_count(input, fps).unwrap_or(0)
            })
        } else {
            Self::estimate_frame_count(input, fps).unwrap_or(0)
        };

        Ok(VideoInfo {
            width,
            height,
            fps,
            total_frames,
        })
    }

    /// Parse a frame rate string like "30/1" or "29.97" into an f64.
    fn parse_frame_rate(rate_str: &str) -> Result<f64, AppError> {
        if let Some((num, den)) = rate_str.split_once('/') {
            let numerator: f64 = num.parse().map_err(|e| {
                AppError::Ffmpeg(format!("Failed to parse fps numerator '{}': {}", num, e))
            })?;
            let denominator: f64 = den.parse().map_err(|e| {
                AppError::Ffmpeg(format!("Failed to parse fps denominator '{}': {}", den, e))
            })?;
            if denominator == 0.0 {
                return Err(AppError::Ffmpeg(
                    "Frame rate denominator is zero".to_string(),
                ));
            }
            Ok(numerator / denominator)
        } else {
            rate_str.parse().map_err(|e| {
                AppError::Ffmpeg(format!("Failed to parse fps '{}': {}", rate_str, e))
            })
        }
    }

    /// Estimate frame count from video duration and fps using ffprobe.
    fn estimate_frame_count(input: &Path, fps: f64) -> Option<u32> {
        let input_str = input.to_str()?;

        let output = Command::new("ffprobe")
            .args([
                "-v", "quiet",
                "-select_streams", "v:0",
                "-show_entries", "format=duration",
                "-of", "csv=p=0",
                input_str,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let duration: f64 = stdout.trim().parse().ok()?;

        Some((duration * fps).round() as u32)
    }

    /// Spawn the FFmpeg decoder process that outputs raw RGBA frames to stdout.
    /// Applies fps filter to resample video to target_fps before decoding.
    fn spawn_decoder(input: &Path, target_fps: u32) -> Result<Child, AppError> {
        let input_str = input.to_str().ok_or_else(|| {
            AppError::Ffmpeg("Input path contains invalid UTF-8".to_string())
        })?;

        let fps_filter = format!("fps={}", target_fps);

        Command::new("ffmpeg")
            .args([
                "-i", input_str,
                "-vf", &fps_filter,
                "-f", "rawvideo",
                "-pix_fmt", "rgba",
                "-v", "quiet",
                "-",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| {
                AppError::Ffmpeg(format!(
                    "Failed to spawn FFmpeg decoder (is FFmpeg installed?): {}",
                    e
                ))
            })
    }

    /// Spawn the FFmpeg encoder process that accepts raw RGBA frames on stdin.
    fn spawn_encoder(
        output: &Path,
        width: u32,
        height: u32,
        fps: f64,
    ) -> Result<Child, AppError> {
        let output_str = output.to_str().ok_or_else(|| {
            AppError::Ffmpeg("Output path contains invalid UTF-8".to_string())
        })?;

        let size = format!("{}x{}", width, height);
        let rate = format!("{}", fps);

        Command::new("ffmpeg")
            .args([
                "-y",
                "-f", "rawvideo",
                "-pix_fmt", "rgba",
                "-s", &size,
                "-r", &rate,
                "-i", "-",
                "-c:v", "libx264",
                "-pix_fmt", "yuv420p",
                "-v", "quiet",
                output_str,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                AppError::Ffmpeg(format!(
                    "Failed to spawn FFmpeg encoder (is FFmpeg installed?): {}",
                    e
                ))
            })
    }
}
