//! Prepare a "Bad Apple!!"-style clip into a set of silhouette shape brushes.
//!
//! The idea: take a video that consists of pure black silhouettes on white
//! backgrounds (and white silhouettes on black backgrounds), sample a frame
//! every N seconds, strip the solid background, and emit each remaining
//! silhouette as a transparent PNG. Those PNGs become the shape "brushes" the
//! approximator uses to re-paint the very same video — Bad Apple drawn out of
//! Bad Apple frames.
//!
//! Pipeline (per sampled frame):
//!   1. Extract one frame every `--interval` seconds with FFmpeg.
//!   2. Downscale so the longest side fits `--max-size` (what the program would
//!      shrink the media to anyway), preserving aspect ratio.
//!   3. Drop near-monochrome frames (>= `--mono` of the pixels are a single
//!      near-pure colour — e.g. fully white intro/flash frames).
//!   4. Decide the background by walking the border: count near-white vs
//!      near-black border pixels; the majority colour is the background.
//!   5. Remove the background colour AND nearby shades (within `--bg-tol`),
//!      making them transparent. The remaining silhouette keeps its colours.
//!   6. Draw a solid `--outline`-pixel outline (default 3 px) around the
//!      silhouette in the background colour, so each shape has a clear, thick
//!      edge instead of a thin anti-aliased fringe.
//!
//! Output PNGs are written to `raw_shapes/` by default, so run the approximator
//! with `use_original_colors = true` to keep the black/white fills intact.
//!
//! Usage:
//!   cargo run --release --example prepare_bad_apple -- [VIDEO] [options]
//!
//! Options:
//!   --interval <sec>   Seconds between sampled frames (default 2.0)
//!   --out <folder>     Output folder for shape PNGs (default "raw_shapes")
//!   --max-size <px>    Longest output side in pixels (default 512)
//!   --mono <0..1>      Drop frames where a single near-pure colour covers
//!                      >= this fraction of pixels (default 0.97)
//!   --bg-tol <0..255>  How far from pure white/black still counts as
//!                      background and is removed (default 40)
//!   --outline <px>     Thickness of the drawn silhouette outline (default 3)
//!
//! Requires FFmpeg in PATH.

use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};
use std::path::{Path, PathBuf};

struct Config {
    input: PathBuf,
    out_dir: PathBuf,
    interval: f64,
    max_size: u32,
    mono: f64,
    bg_tol: u8,
    outline: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            input: PathBuf::from("bad apple addon/Bad Apple!!.mp4"),
            out_dir: PathBuf::from("raw_shapes"),
            interval: 2.0,
            max_size: 512,
            mono: 0.97,
            bg_tol: 40,
            outline: 3,
        }
    }
}

fn parse_args() -> Config {
    let mut cfg = Config::default();
    let mut args = std::env::args().skip(1);
    let mut positional_input = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--interval" => {
                if let Some(v) = args.next() {
                    cfg.interval = v.parse().unwrap_or(cfg.interval);
                }
            }
            "--out" => {
                if let Some(v) = args.next() {
                    cfg.out_dir = PathBuf::from(v);
                }
            }
            "--max-size" => {
                if let Some(v) = args.next() {
                    cfg.max_size = v.parse().unwrap_or(cfg.max_size);
                }
            }
            "--mono" => {
                if let Some(v) = args.next() {
                    cfg.mono = v.parse().unwrap_or(cfg.mono);
                }
            }
            "--bg-tol" => {
                if let Some(v) = args.next() {
                    cfg.bg_tol = v.parse().unwrap_or(cfg.bg_tol);
                }
            }
            "--outline" => {
                if let Some(v) = args.next() {
                    cfg.outline = v.parse().unwrap_or(cfg.outline);
                }
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                if !positional_input && !other.starts_with("--") {
                    cfg.input = PathBuf::from(other);
                    positional_input = true;
                } else {
                    eprintln!("Unknown argument: {}", other);
                }
            }
        }
    }

    if cfg.interval <= 0.0 {
        cfg.interval = 2.0;
    }
    if cfg.max_size < 16 {
        cfg.max_size = 16;
    }
    if cfg.outline < 0 {
        cfg.outline = 0;
    }
    cfg.mono = cfg.mono.clamp(0.0, 1.0);
    cfg
}

fn print_help() {
    println!("prepare_bad_apple — turn a Bad Apple-style clip into silhouette brushes");
    println!();
    println!("Usage: cargo run --release --example prepare_bad_apple -- [VIDEO] [options]");
    println!("  --interval <sec>   Seconds between sampled frames (default 2.0)");
    println!("  --out <folder>     Output folder (default raw_shapes)");
    println!("  --max-size <px>    Longest output side (default 512)");
    println!("  --mono <0..1>      Drop near-monochrome frames threshold (default 0.97)");
    println!("  --bg-tol <0..255>  Background removal tolerance (default 40)");
    println!("  --outline <px>     Silhouette outline thickness (default 3)");
}

fn main() {
    let cfg = parse_args();

    if !cfg.input.exists() {
        eprintln!("Input video '{}' does not exist.", cfg.input.display());
        eprintln!("Pass the path explicitly: cargo run --release --example prepare_bad_apple -- \"path/to/clip.mp4\"");
        std::process::exit(1);
    }

    println!("Input video : {}", cfg.input.display());
    println!("Output dir   : {}", cfg.out_dir.display());
    println!("Interval     : every {} s", cfg.interval);
    println!("Max size     : {} px (longest side)", cfg.max_size);
    println!("Mono cutoff  : {:.0}% single near-pure colour", cfg.mono * 100.0);
    println!("BG tolerance : {}", cfg.bg_tol);
    println!("Outline      : {} px", cfg.outline);
    println!();

    // 1. Extract frames into a temp directory via FFmpeg.
    let temp_dir = std::env::temp_dir().join("bad_apple_frames");
    if temp_dir.exists() {
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
    std::fs::create_dir_all(&temp_dir).expect("Failed to create temp frame directory");

    let frames = extract_frames(&cfg.input, &temp_dir, cfg.interval);
    if frames.is_empty() {
        eprintln!("FFmpeg produced no frames. Is FFmpeg installed and the video valid?");
        std::process::exit(1);
    }
    println!("Extracted {} raw frame(s).", frames.len());

    std::fs::create_dir_all(&cfg.out_dir).expect("Failed to create output folder");

    // 2..6. Process each frame.
    let mut kept = 0u32;
    let mut skipped_mono = 0u32;
    let mut skipped_empty = 0u32;

    for path in &frames {
        let img = match image::open(path) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("  Skipping unreadable frame '{}': {}", path.display(), e);
                continue;
            }
        };

        let resized = resize_to_fit(&img, cfg.max_size);
        let rgba = resized.to_rgba8();

        if is_near_monochrome(&rgba, cfg.bg_tol, cfg.mono) {
            skipped_mono += 1;
            continue;
        }

        let bg = detect_background(&rgba, cfg.bg_tol);
        let shape = build_shape(&rgba, bg, cfg.bg_tol, cfg.outline);

        // Guard against degenerate near-empty results (almost everything removed).
        if opaque_fraction(&shape) < 0.005 {
            skipped_empty += 1;
            continue;
        }

        kept += 1;
        let out_path = cfg.out_dir.join(format!("frame_{:05}.png", kept));
        if let Err(e) = shape.save(&out_path) {
            eprintln!("  Failed to save '{}': {}", out_path.display(), e);
        }
    }

    // Clean up temp frames.
    let _ = std::fs::remove_dir_all(&temp_dir);

    println!();
    println!("Done.");
    println!("  Saved        : {} shape(s) -> {}", kept, cfg.out_dir.display());
    println!("  Skipped mono : {}", skipped_mono);
    println!("  Skipped empty: {}", skipped_empty);
    println!();
    println!("Next steps:");
    println!("  1. Put \"{}\" into input_media/ as the target.", cfg.input.display());
    println!("  2. In settings.toml set use_original_colors = true (keeps black/white fills).");
    println!("  3. Run the app and approximate the video.");
}

/// Run FFmpeg to sample one frame every `interval` seconds into `temp_dir`.
/// Returns the sorted list of produced PNG paths.
fn extract_frames(input: &Path, temp_dir: &Path, interval: f64) -> Vec<PathBuf> {
    let rate = 1.0 / interval; // frames per second to keep
    let fps_filter = format!("fps={}", rate);
    let pattern = temp_dir.join("frame_%05d.png");

    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            input.to_str().unwrap_or(""),
            "-vf",
            &fps_filter,
            "-v",
            "error",
            pattern.to_str().unwrap_or(""),
        ])
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("FFmpeg exited with status {} (some frames may still exist).", s),
        Err(e) => {
            eprintln!("Failed to run FFmpeg: {}", e);
            return Vec::new();
        }
    }

    let mut frames: Vec<PathBuf> = std::fs::read_dir(temp_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("png"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    frames.sort();
    frames
}

/// Resize a frame so its longest side equals `max_size`, preserving aspect
/// ratio. Smaller frames are left untouched.
fn resize_to_fit(img: &DynamicImage, max_size: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    if w <= max_size && h <= max_size {
        return img.clone();
    }
    img.resize(max_size, max_size, image::imageops::FilterType::Lanczos3)
}

/// A pixel is "near white" if every channel is within `tol` of 255.
#[inline]
fn near_white(p: &Rgba<u8>, tol: u8) -> bool {
    let cut = 255u16.saturating_sub(tol as u16) as u8;
    p.0[0] >= cut && p.0[1] >= cut && p.0[2] >= cut
}

/// A pixel is "near black" if every channel is within `tol` of 0.
#[inline]
fn near_black(p: &Rgba<u8>, tol: u8) -> bool {
    p.0[0] <= tol && p.0[1] <= tol && p.0[2] <= tol
}

/// Background colour: which near-pure colour dominates the frame border.
#[derive(Clone, Copy, PartialEq)]
enum Background {
    White,
    Black,
}

impl Background {
    /// The solid colour used to draw the silhouette outline (= the background
    /// colour, i.e. the opposite of the silhouette fill).
    fn outline_rgb(self) -> [u8; 3] {
        match self {
            Background::White => [255, 255, 255],
            Background::Black => [0, 0, 0],
        }
    }

    /// Whether a pixel belongs to this (removable) background within `tol`.
    fn matches(self, p: &Rgba<u8>, tol: u8) -> bool {
        match self {
            Background::White => near_white(p, tol),
            Background::Black => near_black(p, tol),
        }
    }
}

/// True if a single near-pure colour (white or black) covers >= `threshold` of
/// all pixels — i.e. the frame is essentially blank and carries no silhouette.
fn is_near_monochrome(img: &RgbaImage, tol: u8, threshold: f64) -> bool {
    let total = (img.width() as u64 * img.height() as u64).max(1);
    let mut white = 0u64;
    let mut black = 0u64;
    for p in img.pixels() {
        if near_white(p, tol) {
            white += 1;
        } else if near_black(p, tol) {
            black += 1;
        }
    }
    let frac = white.max(black) as f64 / total as f64;
    frac >= threshold
}

/// Walk the 1-pixel border and decide the background by majority near-pure colour.
fn detect_background(img: &RgbaImage, tol: u8) -> Background {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Background::White;
    }
    let mut white = 0u64;
    let mut black = 0u64;
    let mut tally = |p: &Rgba<u8>| {
        if near_white(p, tol) {
            white += 1;
        } else if near_black(p, tol) {
            black += 1;
        }
    };

    for x in 0..w {
        tally(img.get_pixel(x, 0));
        tally(img.get_pixel(x, h - 1));
    }
    for y in 0..h {
        tally(img.get_pixel(0, y));
        tally(img.get_pixel(w - 1, y));
    }

    if black > white {
        Background::Black
    } else {
        Background::White
    }
}

/// Build the final shape:
///   * background pixels (within `tol` of the bg colour) become transparent;
///   * the remaining silhouette keeps its original colours, fully opaque;
///   * a solid `outline`-pixel ring in the bg colour is drawn around the
///     silhouette (into what was background), giving a thick, clear edge.
fn build_shape(img: &RgbaImage, bg: Background, tol: u8, outline: i32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let (wi, hi) = (w as i32, h as i32);

    // Foreground mask: everything that is NOT background.
    let mut fg = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            fg[idx] = !bg.matches(img.get_pixel(x, y), tol);
        }
    }

    // Dilate the foreground mask by `outline` pixels (euclidean disk). The
    // dilated-but-not-foreground pixels form the outline ring.
    let mut outline_mask = vec![false; (w * h) as usize];
    if outline > 0 {
        let r2 = outline * outline;
        for y in 0..hi {
            for x in 0..wi {
                if !fg[(y * wi + x) as usize] {
                    continue;
                }
                // Stamp a disk of radius `outline` around this fg pixel.
                for dy in -outline..=outline {
                    let ny = y + dy;
                    if ny < 0 || ny >= hi {
                        continue;
                    }
                    for dx in -outline..=outline {
                        let nx = x + dx;
                        if nx < 0 || nx >= wi {
                            continue;
                        }
                        if dx * dx + dy * dy > r2 {
                            continue;
                        }
                        let nidx = (ny * wi + nx) as usize;
                        if !fg[nidx] {
                            outline_mask[nidx] = true;
                        }
                    }
                }
            }
        }
    }

    let outline_rgb = bg.outline_rgb();
    ImageBuffer::from_fn(w, h, |x, y| {
        let idx = (y * w + x) as usize;
        if fg[idx] {
            let p = img.get_pixel(x, y);
            Rgba([p.0[0], p.0[1], p.0[2], 255])
        } else if outline_mask[idx] {
            Rgba([outline_rgb[0], outline_rgb[1], outline_rgb[2], 255])
        } else {
            Rgba([0, 0, 0, 0])
        }
    })
}

/// Fraction of pixels that remain opaque after background removal.
fn opaque_fraction(img: &RgbaImage) -> f64 {
    let total = (img.width() as u64 * img.height() as u64).max(1);
    let opaque = img.pixels().filter(|p| p.0[3] > 0).count() as u64;
    opaque as f64 / total as f64
}
