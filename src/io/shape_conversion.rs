//! Shared conversion used by the application and the preparation example.
use image::{DynamicImage, Rgba, RgbaImage};
use std::path::{Path, PathBuf};

pub fn process_shape(img: &DynamicImage, resolution: u32) -> RgbaImage {
    let mut rgba = img.to_rgba8();
    for pixel in rgba.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        let lum = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64).round() as u8;
        *pixel = Rgba([lum, lum, lum, a]);
    }
    let resized = DynamicImage::ImageRgba8(rgba)
        .resize(
            resolution,
            resolution,
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8();
    // Only padding outside the photo is transparent; retain the photo's alpha.
    let mut canvas = RgbaImage::new(resolution, resolution);
    image::imageops::replace(
        &mut canvas,
        &resized,
        ((resolution - resized.width()) / 2) as i64,
        ((resolution - resized.height()) / 2) as i64,
    );
    canvas
}

/// Inspect decoded dimensions and bound decoder allocations before loading pixels.
pub fn open_bounded(path: &Path, budget: u64) -> image::ImageResult<DynamicImage> {
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let (width, height) = reader.into_dimensions()?;
    if u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(16)
        > budget
    {
        return Err(image::ImageError::IoError(std::io::Error::new(std::io::ErrorKind::OutOfMemory,
            "Source image exceeds decoding memory budget / Исходное изображение превышает бюджет декодирования")));
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(budget);
    reader.limits(limits);
    reader.decode()
}

pub fn validate_resolution(resolution: u32) -> Result<(), String> {
    if resolution == 0
        || u64::from(resolution).saturating_pow(2).saturating_mul(4) > 256 * 1024 * 1024
    {
        return Err("Conversion exceeds the 256 MiB per-image working budget / Слишком большое разрешение для конвертации".into());
    }
    Ok(())
}

pub fn convert_folder(
    source: &Path,
    destination: &Path,
    resolution: u32,
) -> Result<String, String> {
    validate_resolution(resolution)?;
    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    if source.canonicalize().map_err(|e| e.to_string())?
        == destination.canonicalize().map_err(|e| e.to_string())?
    {
        return Err(
            "Choose different source and destination folders / Выберите разные папки".into(),
        );
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(source)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    let mut stems = std::collections::BTreeMap::<String, usize>::new();
    for path in &paths {
        *stems
            .entry(
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase(),
            )
            .or_default() += 1;
    }
    let mut count = 0;
    let mut failed = Vec::new();
    for path in paths {
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                [
                    "png", "jpg", "jpeg", "jpe", "webp", "bmp", "gif", "tif", "tiff", "tga", "ico",
                    "qoi", "pnm", "ppm", "pgm", "pbm", "dds", "ff", "hdr",
                ]
                .contains(&e.to_lowercase().as_str())
            })
            .unwrap_or(false)
        {
            continue;
        }
        // Update legacy stem.png files; disambiguate when two originals share a stem.
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let extended = destination.join(format!(
            "{}.png",
            path.file_name().unwrap().to_string_lossy()
        ));
        let name = if stems[&stem.to_lowercase()] > 1 || extended.exists() {
            path.file_name().unwrap().to_string_lossy()
        } else {
            stem
        };
        let output = destination.join(format!("{name}.png"));
        let result = open_bounded(&path, 512 * 1024 * 1024)
            .and_then(|img| process_shape(&img, resolution).save(&output));
        match result {
            Ok(()) => count += 1,
            Err(e) => failed.push(format!("{}: {e}", path.display())),
        }
    }
    let message = format!(
        "Converted / Конвертировано: {count}. Errors / Ошибок: {}. {}",
        failed.len(),
        failed
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
    if count == 0 || !failed.is_empty() {
        Err(message)
    } else {
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opaque_dark_pixels_stay_opaque_and_alpha_is_preserved() {
        let mut img = RgbaImage::new(3, 3);
        for (i, p) in img.pixels_mut().enumerate() {
            *p = Rgba([10, 20, 30, [255, 254, 128, 0][i % 4]]);
        }
        let output = process_shape(&DynamicImage::ImageRgba8(img.clone()), 3);
        for (before, after) in img.pixels().zip(output.pixels()) {
            assert_eq!(before[3], after[3]);
            assert_eq!(after[0], after[1]);
            assert_eq!(after[1], after[2]);
        }
    }
    #[test]
    fn aspect_padding_does_not_fade_photo() {
        let output = process_shape(
            &DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 2, Rgba([12, 12, 12, 255]))),
            4,
        );
        assert_eq!(output.get_pixel(2, 0)[3], 0);
        assert_eq!(output.get_pixel(2, 1)[3], 255);
    }

    #[test]
    fn reconversion_replaces_legacy_ghost_without_losing_same_stem_sources() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw");
        let ready = dir.path().join("ready");
        std::fs::create_dir_all(&raw).unwrap();
        std::fs::create_dir_all(&ready).unwrap();
        RgbaImage::from_pixel(2, 2, Rgba([20, 20, 20, 255]))
            .save(raw.join("photo.png"))
            .unwrap();
        RgbaImage::new(2, 2).save(ready.join("photo.png")).unwrap();
        convert_folder(&raw, &ready, 2).unwrap();
        assert_eq!(
            image::open(ready.join("photo.png"))
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)[3],
            255
        );
        assert_eq!(std::fs::read_dir(&ready).unwrap().count(), 1);
        image::RgbImage::new(2, 2)
            .save(raw.join("photo.jpg"))
            .unwrap();
        convert_folder(&raw, &ready, 2).unwrap();
        assert!(ready.join("photo.jpg.png").exists());
        assert!(ready.join("photo.png.png").exists());
        assert!(convert_folder(&raw, &raw, 2).is_err());
    }
}
