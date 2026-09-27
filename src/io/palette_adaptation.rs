//! Target preparation for a finite original-color sprite library. All heavy
//! work runs on the file worker, never in the generation/render loop.
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::Ordering, mpsc, Arc},
    time::{Duration, Instant},
};

use image::{imageops::FilterType, RgbaImage};
use serde::{Deserialize, Serialize};

use super::{frame_set::Progress, library, output, shape_conversion};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub colors: u32,
    pub strength: f32,
    pub smoothing: f32,
    pub contrast: f32,
    pub desaturation: f32,
    pub mix_colors: bool,
    /// Pattern cell width in *processed target* pixels, not source pixels.
    pub dither_size: u32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            colors: 32,
            strength: 1.0,
            smoothing: 0.35,
            contrast: 0.15,
            desaturation: 0.15,
            mix_colors: true,
            dither_size: 4,
        }
    }
}
impl Options {
    fn validate(&self) -> Result<(), String> {
        // This is the format/complexity limit of the palette, not a cap on
        // evolutionary settings. 256 entries allow a bounded reusable LUT.
        if !(1..=256).contains(&self.colors) || self.dither_size == 0 {
            return Err("Palette: 1–256 colors; pattern size must be positive / Палитра: 1–256 цветов; размер рисунка должен быть положительным".into());
        }
        for value in [
            self.strength,
            self.smoothing,
            self.contrast,
            self.desaturation,
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err("Adaptation amounts must be between 0 and 1 / Сила обработки должна быть от 0 до 1".into());
            }
        }
        Ok(())
    }
}

pub struct Report {
    pub source: PathBuf,
    pub path: PathBuf,
    pub palette: Vec<[u8; 3]>,
    pub message: String,
}

fn check(progress: &Progress) -> Result<(), String> {
    if progress.cancelled.load(Ordering::Relaxed) {
        Err("Cancelled / Отменено".into())
    } else {
        Ok(())
    }
}
fn stage(progress: &Progress, name: &str, total: u64) {
    *progress.stage.lock().unwrap() = name.into();
    progress.completed.store(0, Ordering::Relaxed);
    progress.total.store(total, Ordering::Relaxed);
}
fn rgb(pixel: &[u8]) -> [f32; 3] {
    [
        pixel[0] as f32 / 255.0,
        pixel[1] as f32 / 255.0,
        pixel[2] as f32 / 255.0,
    ]
}
fn luma(color: [f32; 3]) -> f32 {
    color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722
}
fn feature(color: [f32; 3], weight: f32) -> [f32; 3] {
    let y = luma(color);
    // Normalize before multiplying, including for large manual weights.
    let inv = 1.0 / (weight + 2.0);
    [
        y * (weight * inv).sqrt(),
        (color[2] - y) / 1.8556 * inv.sqrt(),
        (color[0] - y) / 1.5748 * inv.sqrt(),
    ]
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

struct Sample {
    color: [u8; 3],
    vector: [f32; 3],
    weight: f32,
}

fn palette(
    folder: &Path,
    resolution: u32,
    layer_limit: usize,
    count: usize,
    weight: f32,
    progress: &Progress,
) -> Result<(Vec<[u8; 3]>, usize, usize), String> {
    // Match the loader's ordering and resize, without keeping every sprite in RAM.
    let mut files = library::scan(folder, false);
    files.sort();
    files.truncate(layer_limit.min(2048));
    if files.is_empty() {
        return Err(
            "No original-color sprites in this set / В наборе нет исходных цветных спрайтов".into(),
        );
    }
    if resolution == 0
        || u64::from(resolution).saturating_pow(2).saturating_mul(16)
            > crate::monitor::available_memory() / 6
    {
        return Err("Sprite resolution exceeds preparation memory budget / Разрешение спрайтов превышает бюджет памяти обработки".into());
    }
    stage(
        progress,
        "Measuring sprite palette / Измерение палитры спрайтов",
        files.len() as u64,
    );
    let per_file = (65_536 / files.len()).clamp(32, 2048);
    let mut histogram = BTreeMap::<[u8; 3], f32>::new();
    let mut usable = 0;
    let mut skipped = 0;
    for (index, path) in files.iter().enumerate() {
        check(progress)?;
        match shape_conversion::open_bounded(
            path,
            (crate::monitor::available_memory() / 6).min(512 * 1024 * 1024),
        ) {
            Ok(image) => {
                let image = image
                    .resize_exact(resolution, resolution, FilterType::Triangle)
                    .to_rgba8();
                let visible = image.pixels().filter(|p| p[3] != 0).count();
                if visible > 0 {
                    usable += 1;
                    let stride = visible.div_ceil(per_file);
                    let samples = visible.div_ceil(stride);
                    // Equal sampling budget per sprite: a large opaque photo
                    // cannot erase all the colors of small transparent sprites.
                    for (rank, pixel) in image.pixels().filter(|p| p[3] != 0).enumerate() {
                        if rank % stride == 0 {
                            *histogram.entry([pixel[0], pixel[1], pixel[2]]).or_default() +=
                                pixel[3] as f32 / (255.0 * samples as f32);
                        }
                    }
                } else {
                    skipped += 1;
                }
            }
            Err(error) => {
                log::warn!("Palette skipped {}: {error}", path.display());
                skipped += 1;
            }
        }
        progress
            .completed
            .store(index as u64 + 1, Ordering::Relaxed);
    }
    if histogram.is_empty() {
        return Err("The set contains no decodable visible pixels / В наборе нет читаемых непрозрачных пикселей".into());
    }
    let samples: Vec<_> = histogram
        .into_iter()
        .map(|(color, weight_value)| Sample {
            color,
            vector: feature(rgb(&color), weight),
            weight: weight_value,
        })
        .collect();
    let k = count.min(samples.len());
    stage(
        progress,
        "Clustering palette / Выделение основных цветов",
        8,
    );
    let dominant = samples
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.weight.total_cmp(&b.weight))
        .unwrap()
        .0;
    let mut centers = vec![samples[dominant].vector];
    // Preserve meaningful dark/light anchors even if they occupy few pixels.
    let significant: Vec<_> = samples
        .iter()
        .filter(|s| s.weight >= usable as f32 * 0.0001)
        .collect();
    for anchor in [
        significant
            .iter()
            .min_by(|a, b| luma(rgb(&a.color)).total_cmp(&luma(rgb(&b.color)))),
        significant
            .iter()
            .max_by(|a, b| luma(rgb(&a.color)).total_cmp(&luma(rgb(&b.color)))),
    ]
    .into_iter()
    .flatten()
    {
        if centers.len() < k && !centers.contains(&anchor.vector) {
            centers.push(anchor.vector);
        }
    }
    let mut nearest = vec![f32::INFINITY; samples.len()];
    // Deterministic farthest-point initialization. No random palette per video frame.
    for center in &centers {
        for (near, sample) in nearest.iter_mut().zip(&samples) {
            *near = near.min(distance(sample.vector, *center));
        }
    }
    while centers.len() < k {
        check(progress)?;
        let next = samples
            .iter()
            .enumerate()
            .max_by(|(a, sa), (b, sb)| {
                (nearest[*a] * sa.weight.sqrt()).total_cmp(&(nearest[*b] * sb.weight.sqrt()))
            })
            .unwrap()
            .0;
        let center = samples[next].vector;
        centers.push(center);
        for (near, sample) in nearest.iter_mut().zip(&samples) {
            *near = near.min(distance(sample.vector, center));
        }
    }
    let mut assignments = vec![0; samples.len()];
    for iteration in 0..8 {
        check(progress)?;
        let mut sums = vec![[0.0; 3]; k];
        let mut weights = vec![0.0; k];
        for (index, sample) in samples.iter().enumerate() {
            if index % 1024 == 0 {
                check(progress)?;
            }
            let best = centers
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    distance(sample.vector, **a).total_cmp(&distance(sample.vector, **b))
                })
                .unwrap()
                .0;
            assignments[index] = best;
            for channel in 0..3 {
                sums[best][channel] += sample.vector[channel] * sample.weight;
            }
            weights[best] += sample.weight;
        }
        for index in 0..k {
            if weights[index] > 0.0 {
                centers[index] = sums[index].map(|s| s / weights[index]);
            }
        }
        progress.completed.store(iteration + 1, Ordering::Relaxed);
    }
    // Use the closest real sampled color of each cluster, not an invented
    // centroid color that no sprite can actually supply.
    let mut representatives = vec![None::<(f32, [u8; 3])>; k];
    for (sample, &cluster) in samples.iter().zip(&assignments) {
        let cost = distance(sample.vector, centers[cluster]);
        if representatives[cluster].map_or(true, |(best, _)| cost < best) {
            representatives[cluster] = Some((cost, sample.color));
        }
    }
    let mut colors: Vec<_> = representatives
        .into_iter()
        .flatten()
        .map(|(_, color)| color)
        .collect();
    colors.sort();
    colors.dedup();
    colors.sort_by(|a, b| luma(rgb(a)).total_cmp(&luma(rgb(b))));
    Ok((colors, usable, skipped))
}

#[derive(Clone, Copy)]
struct Mapping {
    first: usize,
    second: usize,
    mix: f32,
}
struct Processor {
    colors: Vec<[u8; 3]>,
    lut: Vec<Mapping>,
    options: Options,
}
impl Processor {
    fn new(
        colors: Vec<[u8; 3]>,
        options: &Options,
        weight: f32,
        progress: &Progress,
    ) -> Result<Self, String> {
        stage(
            progress,
            "Preparing color mapping / Подготовка цветового соответствия",
            32,
        );
        let vectors: Vec<_> = colors.iter().map(|c| feature(rgb(c), weight)).collect();
        let mut lut = Vec::with_capacity(32 * 32 * 32);
        for r in 0..32 {
            check(progress)?;
            for g in 0..32 {
                for b in 0..32 {
                    let target =
                        feature([r as f32 / 31.0, g as f32 / 31.0, b as f32 / 31.0], weight);
                    let first = vectors
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| {
                            distance(target, **a).total_cmp(&distance(target, **b))
                        })
                        .unwrap()
                        .0;
                    let anchor = vectors[first];
                    let mut mapping = Mapping {
                        first,
                        second: first,
                        mix: 0.0,
                    };
                    let mut best = distance(target, anchor);
                    if options.mix_colors {
                        for (second, end) in vectors.iter().enumerate() {
                            let direction =
                                [end[0] - anchor[0], end[1] - anchor[1], end[2] - anchor[2]];
                            let length = distance(*end, anchor);
                            if length < 1e-8 {
                                continue;
                            }
                            let t = (((target[0] - anchor[0]) * direction[0]
                                + (target[1] - anchor[1]) * direction[1]
                                + (target[2] - anchor[2]) * direction[2])
                                / length)
                                .clamp(0.0, 1.0);
                            let mixed = [
                                anchor[0] + direction[0] * t,
                                anchor[1] + direction[1] * t,
                                anchor[2] + direction[2] * t,
                            ];
                            let cost = distance(target, mixed);
                            // Prefer a single color for negligible improvements:
                            // this avoids filling flat exact-color areas with noise.
                            if t > 0.02 && cost + 0.00002 < best {
                                best = cost;
                                mapping.second = second;
                                mapping.mix = t;
                            }
                        }
                    }
                    lut.push(mapping);
                }
            }
            progress.completed.store(r + 1, Ordering::Relaxed);
        }
        Ok(Self {
            colors,
            lut,
            options: options.clone(),
        })
    }

    fn process(&self, input: &RgbaImage, progress: &Progress) -> Result<RgbaImage, String> {
        let (width, height) = input.dimensions();
        let data = input.as_raw();
        let brightness: Vec<_> = data.chunks_exact(4).map(|p| luma(rgb(p))).collect();
        let mut output = input.clone();
        // An ordered pattern anchored to image coordinates, identical in
        // every frame; no random noise and no frame-by-frame auto exposure.
        const BAYER: [[u8; 8]; 8] = [
            [0, 48, 12, 60, 3, 51, 15, 63],
            [32, 16, 44, 28, 35, 19, 47, 31],
            [8, 56, 4, 52, 11, 59, 7, 55],
            [40, 24, 36, 20, 43, 27, 39, 23],
            [2, 50, 14, 62, 1, 49, 13, 61],
            [34, 18, 46, 30, 33, 17, 45, 29],
            [10, 58, 6, 54, 9, 57, 5, 53],
            [42, 26, 38, 22, 41, 25, 37, 21],
        ];
        for y in 0..height {
            check(progress)?;
            for x in 0..width {
                let index = (y as usize * width as usize + x as usize) * 4;
                let alpha = data[index + 3];
                if alpha == 0 || self.options.strength == 0.0 {
                    continue;
                }
                let original = rgb(&data[index..]);
                let y0 = brightness[index / 4];
                let mut smooth = original;
                if self.options.smoothing > 0.0 || self.options.contrast > 0.0 {
                    let mut sum = [0.0; 3];
                    let mut total = 0.0;
                    // Small bilateral neighborhood: preserve eyes, contours
                    // and color boundaries rather than blurring the whole frame.
                    for dy in -1i64..=1 {
                        for dx in -1i64..=1 {
                            let nx = (x as i64 + dx).clamp(0, width as i64 - 1) as usize;
                            let ny = (y as i64 + dy).clamp(0, height as i64 - 1) as usize;
                            let offset = (ny * width as usize + nx) * 4;
                            let neighbor = rgb(&data[offset..]);
                            let difference = distance(original, neighbor);
                            let dy_luma = y0 - brightness[offset / 4];
                            let spatial = if dx == 0 && dy == 0 { 1.0 } else { 0.6 };
                            let amount = spatial * (data[offset + 3] as f32 / 255.0)
                                / (1.0 + difference * 24.0 + dy_luma * dy_luma * 180.0);
                            for channel in 0..3 {
                                sum[channel] += neighbor[channel] * amount;
                            }
                            total += amount;
                        }
                    }
                    if total > 0.0 {
                        smooth = sum.map(|s| s / total);
                    }
                }
                let detail = y0 - luma(smooth);
                let mut prepared = [0.0; 3];
                for channel in 0..3 {
                    prepared[channel] = original[channel]
                        + (smooth[channel] - original[channel]) * self.options.smoothing;
                }
                let yp = luma(prepared);
                // Mild fixed S-contrast plus local luma detail. Fixed over
                // the video, so cuts do not make exposure parameters jump.
                let contrast = self.options.contrast * ((yp - 0.5) * 0.35 + detail * 1.5);
                for channel in 0..3 {
                    prepared[channel] = (prepared[channel]
                        + (yp - prepared[channel]) * self.options.desaturation
                        + contrast)
                        .clamp(0.0, 1.0);
                }
                let levels = prepared.map(|c| (c * 31.0).round() as usize);
                let mapping = self.lut[(levels[0] * 32 + levels[1]) * 32 + levels[2]];
                let bx = ((x / self.options.dither_size) % 8) as usize;
                let by = ((y / self.options.dither_size) % 8) as usize;
                let threshold = (BAYER[by][bx] as f32 + 0.5) / 64.0;
                let chosen = if threshold < mapping.mix {
                    mapping.second
                } else {
                    mapping.first
                };
                let color = self.colors[chosen];
                let pixel = output.get_pixel_mut(x, y);
                for channel in 0..3 {
                    pixel[channel] = (data[index + channel] as f32 * (1.0 - self.options.strength)
                        + color[channel] as f32 * self.options.strength)
                        .round() as u8;
                }
                // Alpha is preserved exactly; no transparency is introduced.
                pixel[3] = alpha;
            }
        }
        Ok(output)
    }
}

/// Own only the files we created, and clean partial work on every error/cancel.
struct Scratch {
    folder: PathBuf,
    video: PathBuf,
}
impl Scratch {
    fn new(base: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
        loop {
            let folder = base.join(format!(
                ".palette-{}-{:016x}",
                std::process::id(),
                rand::random::<u64>()
            ));
            match std::fs::create_dir(&folder) {
                Ok(()) => {
                    return Ok(Self {
                        video: folder.join("prepared.mp4"),
                        folder,
                    })
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.video);
        let _ = std::fs::remove_dir(&self.folder);
    }
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
}
fn wait(child: &mut Process, progress: &Progress) -> Result<(), String> {
    let started = Instant::now();
    loop {
        check(progress)?;
        match child.0.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                return Err(format!(
                    "FFmpeg failed ({status}) / Ошибка FFmpeg ({status})"
                ))
            }
            None => {}
        }
        if started.elapsed() > Duration::from_secs(60) {
            return Err("FFmpeg did not finish; stopped / FFmpeg не завершился; остановлен".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}
struct VideoInfo {
    width: u32,
    height: u32,
    fps: f64,
    duration: f64,
}
fn probe(source: &Path, progress: &Progress) -> Result<VideoInfo, String> {
    let mut cmd = command("ffprobe");
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height,avg_frame_rate:stream_side_data=rotation:stream_tags=rotate:format=duration",
        "-of",
        "json",
    ])
    .arg(source);
    let mut child = Process(
        cmd.spawn()
            .map_err(|e| format!("FFprobe unavailable / Не найден FFprobe: {e}"))?,
    );
    let stdout = child.0.stdout.take().ok_or("FFprobe stdout unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(1024 * 1024)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    wait(&mut child, progress)?;
    let bytes = reader
        .join()
        .map_err(|_| "FFprobe reader failed")?
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let stream = json["streams"]
        .as_array()
        .and_then(|s| s.first())
        .ok_or("No video stream / Нет видеодорожки")?;
    let mut width = stream["width"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or("Invalid video width")?;
    let mut height = stream["height"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or("Invalid video height")?;
    let rotation = stream["side_data_list"]
        .as_array()
        .and_then(|items| items.iter().find_map(|item| item["rotation"].as_f64()))
        .or_else(|| {
            stream["tags"]["rotate"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
        })
        .unwrap_or(0.0);
    if rotation.is_finite() && (rotation.round().abs() as u64) % 180 == 90 {
        // FFmpeg autorotates before the scale filter. Preserve portrait aspect.
        std::mem::swap(&mut width, &mut height);
    }
    let rate = stream["avg_frame_rate"].as_str().unwrap_or("0/1");
    let (num, den) = rate.split_once('/').unwrap_or((rate, "1"));
    let fps = num.parse::<f64>().unwrap_or(0.0) / den.parse::<f64>().unwrap_or(1.0);
    let duration = json["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    Ok(VideoInfo {
        width,
        height,
        fps,
        duration,
    })
}

fn size(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let ratio = (max_side.max(1) as f64 / width.max(height) as f64).min(1.0);
    (
        ((width as f64 * ratio).round() as u32).max(1),
        ((height as f64 * ratio).round() as u32).max(1),
    )
}
fn frame_budget(width: u32, height: u32) -> Result<(), String> {
    // Includes both bounded pipe queues, working image, luma and decoder buffers.
    if u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(64)
        > crate::monitor::available_memory() / 3
    {
        return Err("Target exceeds processing memory budget; reduce texture size / Образец превышает бюджет памяти обработки; уменьшите размер текстуры".into());
    }
    Ok(())
}
fn prepare_video(
    source: &Path,
    destination: &Path,
    max_side: u32,
    target_fps: u32,
    audio: bool,
    processor: &Processor,
    progress: &Arc<Progress>,
) -> Result<(u32, u32, u64), String> {
    let info = probe(source, progress)?;
    // Source decoding also needs memory, even for a small prepared target.
    frame_budget(info.width, info.height)?;
    let (width, height) = size(info.width, info.height, max_side);
    frame_budget(width, height)?;
    // Do not duplicate source frames just because target FPS is above source FPS.
    let fps = if info.fps.is_finite() && info.fps > 0.0 {
        (target_fps.max(1) as f64).min(info.fps)
    } else {
        target_fps.max(1) as f64
    };
    let filter = format!("fps={fps},scale={width}:{height}:flags=lanczos");
    let mut cmd = command("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-threads", "2", "-i"])
        .arg(source)
        .args([
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-filter_threads",
            "1",
            "-vf",
            &filter,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ]);
    let mut decoder = Process(
        cmd.spawn()
            .map_err(|e| format!("FFmpeg unavailable / Не найден FFmpeg: {e}"))?,
    );
    let mut stdout = decoder
        .0
        .stdout
        .take()
        .ok_or("Decoder stdout unavailable")?;
    let (frames_tx, frames_rx) = mpsc::sync_channel(1);
    let stop = progress.clone();
    std::thread::spawn(move || {
        let bytes = width as usize * height as usize * 4;
        loop {
            if stop.cancelled.load(Ordering::Relaxed) {
                break;
            }
            let mut frame = vec![0; bytes];
            // Distinguish clean EOF from a truncated frame.
            let result = match stdout.read(&mut frame[..1]) {
                Ok(0) => break,
                Ok(_) => stdout
                    .read_exact(&mut frame[1..])
                    .map(|_| frame)
                    .map_err(|e| e.to_string()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => Err(e.to_string()),
            };
            let failed = result.is_err();
            if frames_tx.send(result).is_err() || failed {
                break;
            }
        }
    });
    let mut cmd = command("ffmpeg");
    let dimensions = format!("{width}x{height}");
    let rate = fps.to_string();
    cmd.args([
        "-v",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &dimensions,
        "-r",
        &rate,
        "-i",
        "pipe:0",
    ]);
    if audio {
        cmd.args(["-threads", "1", "-i"]).arg(source);
    }
    cmd.args(["-map", "0:v:0"]);
    if audio {
        cmd.args(["-map", "1:a:0?", "-c:a", "aac", "-b:a", "128k"]);
    }
    // Lossless RGB avoids YUV420 chroma subsampling erasing the palette/dither.
    cmd.args([
        "-c:v",
        "libx264rgb",
        "-crf",
        "0",
        "-preset",
        "fast",
        "-threads",
        "2",
        "-pix_fmt",
        "rgb24",
        "-movflags",
        "+faststart",
    ])
    .arg(destination)
    .stdin(Stdio::piped())
    .stdout(Stdio::null());
    let mut encoder = Process(cmd.spawn().map_err(|e| e.to_string())?);
    let mut stdin = encoder.0.stdin.take().ok_or("Encoder stdin unavailable")?;
    let (encode_tx, encode_rx) = mpsc::sync_channel::<Vec<u8>>(1);
    let (ack_tx, ack_rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(frame) = encode_rx.recv() {
            let result = stdin.write_all(&frame).map_err(|e| e.to_string());
            let failed = result.is_err();
            if ack_tx.send(result).is_err() || failed {
                break;
            }
        }
    });
    let total = if info.duration.is_finite() && info.duration > 0.0 {
        (info.duration * fps).ceil() as u64
    } else {
        0
    };
    stage(progress, "Adapting video / Адаптация видео", total);
    let mut count = 0u64;
    let mut last_data = Instant::now();
    loop {
        check(progress)?;
        let frame = match frames_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(frame) => frame?,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_data.elapsed() > Duration::from_secs(60) {
                    return Err(
                        "Video decoder stopped responding / Декодер видео перестал отвечать".into(),
                    );
                }
                continue;
            }
        };
        output::check_disk_space(
            destination.parent().unwrap(),
            u64::from(width) * u64::from(height) * 4,
        )
        .map_err(|e| e.to_string())?;
        let image = RgbaImage::from_raw(width, height, frame).ok_or("Invalid decoded frame")?;
        let processed = processor.process(&image, progress)?.into_raw();
        // There is at most one unacknowledged frame; send cannot build a long
        // queue or block behind an earlier frame. Poll ack for Cancel/Close.
        encode_tx
            .send(processed)
            .map_err(|_| "Video encoder stopped / Кодировщик остановлен")?;
        let started = Instant::now();
        loop {
            check(progress)?;
            match ack_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(result) => {
                    result?;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Encoder pipe closed / Канал кодировщика закрыт".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if started.elapsed() > Duration::from_secs(60) {
                        return Err(
                            "Video encoder stopped responding / Кодировщик видео перестал отвечать"
                                .into(),
                        );
                    }
                }
            }
        }
        count += 1;
        progress.completed.store(count, Ordering::Relaxed);
        last_data = Instant::now();
    }
    drop(encode_tx);
    stage(progress, "Finishing video file / Завершение видеофайла", 1);
    wait(&mut decoder, progress)?;
    wait(&mut encoder, progress)?;
    if count == 0 {
        return Err(
            "Video contains no decoded frames / Видео не содержит декодированных кадров".into(),
        );
    }
    Ok((width, height, count))
}

pub fn create(
    base: &Path,
    source: &Path,
    shape_folder: &Path,
    resolution: u32,
    layer_limit: usize,
    max_side: u32,
    fps: u32,
    audio: bool,
    weight: f32,
    options: &Options,
    progress: &Arc<Progress>,
) -> Result<Report, String> {
    options.validate()?;
    if !weight.is_finite() || weight <= 0.0 {
        return Err("Invalid luma weight / Неверный вес яркости".into());
    }
    check(progress)?;
    let (colors, usable, skipped) = palette(
        shape_folder,
        resolution,
        layer_limit,
        options.colors as usize,
        weight,
        progress,
    )?;
    let processor = Processor::new(colors.clone(), options, weight, progress)?;
    let media = base.join("input_media");
    let mut scratch = Scratch::new(&media)?;
    let video = super::media_loader::is_video_path(source);
    let (width, height, frames) = if video {
        prepare_video(
            source,
            &scratch.video,
            max_side,
            fps,
            audio,
            &processor,
            progress,
        )?
    } else {
        stage(progress, "Adapting image / Адаптация изображения", 1);
        let image = shape_conversion::open_bounded(
            source,
            (crate::monitor::available_memory() / 3).min(512 * 1024 * 1024),
        )
        .map_err(|e| e.to_string())?;
        let (width, height) = size(image.width(), image.height(), max_side);
        frame_budget(width, height)?;
        let image = image
            .resize_exact(width, height, FilterType::Lanczos3)
            .to_rgba8();
        let prepared = processor.process(&image, progress)?;
        scratch.video = scratch.folder.join("prepared.png");
        output::check_disk_space(&media, u64::from(width) * u64::from(height) * 8)
            .map_err(|e| e.to_string())?;
        prepared.save(&scratch.video).map_err(|e| e.to_string())?;
        (width, height, 1)
    };
    check(progress)?;
    stage(
        progress,
        "Saving adapted target / Сохранение обработанного образца",
        1,
    );
    let name = format!(
        "{}_palette.{}",
        source.file_stem().unwrap_or_default().to_string_lossy(),
        if video { "mp4" } else { "png" }
    );
    let path = super::import::import_as(&scratch.video, &media, std::ffi::OsStr::new(&name))?;
    if let Err(error) = check(progress) {
        // This path was uniquely created by this job, never an existing target.
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    let dark = colors.iter().map(|c| luma(rgb(c))).fold(1.0f32, f32::min);
    let light = colors.iter().map(|c| luma(rgb(c))).fold(0.0f32, f32::max);
    let mut message = format!("Ready / Готово: {} · {width}×{height} · {frames} frames / кадров · {} colors / цветов · {usable} sprites / спрайтов. Original kept / Исходник сохранён.",path.file_name().unwrap_or_default().to_string_lossy(),colors.len());
    if skipped > 0 {
        message.push_str(&format!(" Skipped / Пропущено: {skipped}."));
    }
    if light - dark < 0.4 {
        message.push_str(" Limited brightness range: some details cannot be represented / Узкий диапазон яркости: часть деталей недоступна этому набору.");
    }
    progress.completed.store(1, Ordering::Relaxed);
    Ok(Report {
        source: source.into(),
        path,
        palette: colors,
        message,
    })
}
