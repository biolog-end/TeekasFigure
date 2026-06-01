use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use image::DynamicImage;

use crate::error::AppError;

/// Supported media file extensions (case-insensitive).
const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp"];
const SUPPORTED_VIDEO_EXTENSIONS: &[&str] = &["mp4"];

/// Represents the type of media loaded from the input folder.
pub enum MediaType {
    /// A successfully loaded image (PNG, JPG, BMP).
    Image(DynamicImage),
    /// A detected video file (MP4) — path stored for FFmpeg processing.
    Video(PathBuf),
}

/// Returns the directory containing the current executable.
///
/// Falls back to the current working directory if the executable path
/// cannot be determined.
pub fn get_base_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Ensures that the required directories (`input_media/`, `input_shapes/`, `output/`)
/// exist relative to `base`. Creates any missing directories.
///
/// Returns a list of directory names that were created (useful for notifications).
pub fn ensure_directories(base: &Path) -> Result<Vec<String>, AppError> {
    let required_dirs = ["input_media", "input_shapes", "output"];
    let mut created = Vec::new();

    for dir_name in &required_dirs {
        let dir_path = base.join(dir_name);
        if !dir_path.exists() {
            fs::create_dir_all(&dir_path).map_err(|e| AppError::SaveFailed {
                reason: format!("Failed to create directory '{}': {}", dir_path.display(), e),
            })?;
            created.push(dir_name.to_string());
        }
    }

    Ok(created)
}

/// Finds the alphabetically first file in `folder` whose extension matches
/// a supported format (PNG, JPG, JPEG, BMP, MP4). Extension matching is case-insensitive.
///
/// Returns `None` if the folder doesn't exist, is empty, or contains no supported files.
pub fn find_first_supported_file(folder: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(folder).ok()?;

    let mut supported_files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| is_supported_extension(ext))
                    .unwrap_or(false)
        })
        .collect();

    // Sort alphabetically by filename (case-sensitive on the full path for determinism)
    supported_files.sort_by(|a, b| {
        let name_a = a.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        let name_b = b.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        name_a.cmp(&name_b)
    });

    supported_files.into_iter().next()
}

/// Loads media from the given path. Images are decoded via the `image` crate.
/// Video files (MP4) are returned as a path for later FFmpeg processing.
pub fn load_media(path: &Path) -> Result<MediaType, AppError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();

    if SUPPORTED_VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        // For video files, just verify the file exists and return the path
        if !path.exists() {
            return Err(AppError::NoMedia {
                path: path.to_path_buf(),
            });
        }
        Ok(MediaType::Video(path.to_path_buf()))
    } else if is_supported_image_extension(&extension) {
        // Load image via the image crate
        let img = image::open(path).map_err(|_| AppError::NoMedia {
            path: path.to_path_buf(),
        })?;
        Ok(MediaType::Image(img))
    } else {
        Err(AppError::NoMedia {
            path: path.to_path_buf(),
        })
    }
}

/// Checks if an extension (case-insensitive) is any supported format (image or video).
fn is_supported_extension(ext: &str) -> bool {
    let lower = ext.to_lowercase();
    SUPPORTED_IMAGE_EXTENSIONS.contains(&lower.as_str())
        || SUPPORTED_VIDEO_EXTENSIONS.contains(&lower.as_str())
}

/// Checks if an extension (already lowercased) is a supported image format.
fn is_supported_image_extension(ext: &str) -> bool {
    SUPPORTED_IMAGE_EXTENSIONS.contains(&ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn test_find_first_supported_file_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_first_supported_file(dir.path()).is_none());
    }

    #[test]
    fn test_find_first_supported_file_no_supported() {
        let dir = tempfile::tempdir().unwrap();
        File::create(dir.path().join("readme.txt")).unwrap();
        File::create(dir.path().join("data.csv")).unwrap();
        assert!(find_first_supported_file(dir.path()).is_none());
    }

    #[test]
    fn test_find_first_supported_file_alphabetical() {
        let dir = tempfile::tempdir().unwrap();
        File::create(dir.path().join("banana.png")).unwrap();
        File::create(dir.path().join("apple.jpg")).unwrap();
        File::create(dir.path().join("cherry.bmp")).unwrap();

        let result = find_first_supported_file(dir.path());
        assert!(result.is_some());
        let filename = result.unwrap().file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(filename, "apple.jpg");
    }

    #[test]
    fn test_find_first_supported_file_case_insensitive_extension() {
        let dir = tempfile::tempdir().unwrap();
        File::create(dir.path().join("image.PNG")).unwrap();
        File::create(dir.path().join("photo.JpG")).unwrap();

        let result = find_first_supported_file(dir.path());
        assert!(result.is_some());
        let filename = result.unwrap().file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(filename, "image.PNG");
    }

    #[test]
    fn test_find_first_supported_file_mixed_with_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        File::create(dir.path().join("aaa.txt")).unwrap();
        File::create(dir.path().join("bbb.mp4")).unwrap();
        File::create(dir.path().join("ccc.png")).unwrap();

        let result = find_first_supported_file(dir.path());
        assert!(result.is_some());
        let filename = result.unwrap().file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(filename, "bbb.mp4");
    }

    #[test]
    fn test_ensure_directories_creates_missing() {
        let dir = tempfile::tempdir().unwrap();
        let created = ensure_directories(dir.path()).unwrap();
        assert_eq!(created.len(), 3);
        assert!(created.contains(&"input_media".to_string()));
        assert!(created.contains(&"input_shapes".to_string()));
        assert!(created.contains(&"output".to_string()));

        // All directories should now exist
        assert!(dir.path().join("input_media").exists());
        assert!(dir.path().join("input_shapes").exists());
        assert!(dir.path().join("output").exists());
    }

    #[test]
    fn test_ensure_directories_already_exist() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("input_media")).unwrap();
        fs::create_dir(dir.path().join("input_shapes")).unwrap();
        fs::create_dir(dir.path().join("output")).unwrap();

        let created = ensure_directories(dir.path()).unwrap();
        assert!(created.is_empty());
    }

    #[test]
    fn test_load_media_video() {
        let dir = tempfile::tempdir().unwrap();
        let video_path = dir.path().join("test.mp4");
        File::create(&video_path).unwrap();

        let result = load_media(&video_path);
        assert!(result.is_ok());
        match result.unwrap() {
            MediaType::Video(p) => assert_eq!(p, video_path),
            _ => panic!("Expected Video variant"),
        }
    }

    #[test]
    fn test_load_media_unsupported_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        File::create(&path).unwrap();

        let result = load_media(&path);
        assert!(result.is_err());
    }
}
