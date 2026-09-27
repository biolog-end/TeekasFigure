//! Generate sample shape PNGs for the input_shapes/ directory.
//!
//! Run with: cargo run --example generate_shapes
//!
//! This creates a circle and a square shape as 128x128 RGBA PNGs.

use image::{ImageBuffer, Rgba};
use std::path::Path;

fn main() {
    let size = 128u32;
    let output_dir = Path::new("input_shapes");
    std::fs::create_dir_all(output_dir).expect("Failed to create input_shapes directory");

    // Generate circle shape
    generate_circle(output_dir, size);

    // Generate square shape
    generate_square(output_dir, size);

    println!("Generated shapes in {}/", output_dir.display());
}

fn generate_circle(output_dir: &Path, size: u32) {
    let center = size as f32 / 2.0;
    let radius = center - 2.0;

    let img = ImageBuffer::from_fn(size, size, |x, y| {
        let dx = x as f32 - center;
        let dy = y as f32 - center;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist <= radius {
            let alpha = if dist > radius - 1.5 {
                ((radius - dist) / 1.5).clamp(0.0, 1.0)
            } else {
                1.0
            };
            Rgba([255, 255, 255, (alpha * 255.0) as u8])
        } else {
            Rgba([0, 0, 0, 0])
        }
    });

    let path = output_dir.join("circle.png");
    img.save(&path).expect("Failed to save circle.png");
    println!("  Created: {}", path.display());
}

fn generate_square(output_dir: &Path, size: u32) {
    let margin = 8u32;

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_fn(size, size, |x, y| {
        if x >= margin && x < size - margin && y >= margin && y < size - margin {
            Rgba([255u8, 255, 255, 255])
        } else {
            Rgba([0u8, 0, 0, 0])
        }
    });

    let path = output_dir.join("square.png");
    img.save(&path).expect("Failed to save square.png");
    println!("  Created: {}", path.display());
}
