//! App-owned image libraries. External source files are only ever read.
use super::{import::import_files, media_loader, shape_conversion};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct ShapePhoto {
    pub preview: PathBuf,
    pub prepared: Option<PathBuf>,
}
impl ShapePhoto {
    pub fn files(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.preview.clone()];
        if let Some(path) = &self.prepared {
            if *path != self.preview {
                paths.push(path.clone());
            }
        }
        paths
    }
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| media_loader::SUPPORTED_IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn scan(folder: &Path, videos: bool) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && (is_image(p) || videos && media_loader::is_video_path(p)))
        .collect();
    files.sort_by_key(|p| {
        p.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    files
}

pub fn shapes(base: &Path) -> Vec<ShapePhoto> {
    let raw = scan(&base.join("raw_shapes"), false);
    let ready = scan(&base.join("input_shapes"), false);
    let mut stems = HashMap::<String, usize>::new();
    for path in &raw {
        *stems
            .entry(
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase(),
            )
            .or_default() += 1;
    }
    let mut used = HashSet::new();
    let mut photos = Vec::new();
    for path in raw {
        let extended = base.join("input_shapes").join(format!(
            "{}.png",
            path.file_name().unwrap().to_string_lossy()
        ));
        let legacy = base.join("input_shapes").join(format!(
            "{}.png",
            path.file_stem().unwrap().to_string_lossy()
        ));
        let prepared = if stems[&path.file_stem().unwrap().to_string_lossy().to_lowercase()] == 1
            && legacy.is_file()
        {
            Some(legacy)
        } else if extended.is_file() {
            Some(extended)
        } else {
            None
        };
        let prepared = prepared.filter(|p| used.insert(p.clone()));
        photos.push(ShapePhoto {
            preview: path,
            prepared,
        });
    }
    // Existing prepared-only assets remain visible and removable too.
    photos.extend(
        ready
            .into_iter()
            .filter(|p| !used.contains(p))
            .map(|p| ShapePhoto {
                preview: p.clone(),
                prepared: Some(p),
            }),
    );
    photos
}

pub struct ImportReport {
    pub imported: usize,
    pub first_media: Option<PathBuf>,
    pub errors: Vec<String>,
}
impl ImportReport {
    pub fn message(&self) -> String {
        let mut message = format!("Imported / Добавлено: {}", self.imported);
        if !self.errors.is_empty() {
            message.push_str(&format!(
                ". Errors / Ошибок: {}. {}",
                self.errors.len(),
                self.errors
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        message
    }
}

/// Import independently so one bad file cannot hide other successful additions.
pub fn import(base: &Path, paths: &[PathBuf], as_shapes: bool, resolution: u32) -> ImportReport {
    let mut report = ImportReport {
        imported: 0,
        first_media: None,
        errors: Vec::new(),
    };
    let mut existing_stems: HashSet<_> = if as_shapes {
        scan(&base.join("raw_shapes"), false)
    } else {
        Vec::new()
    }
    .iter()
    .map(|p| {
        p.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    })
    .collect();
    let mut prepared_by_source: HashMap<_, _> = if as_shapes { shapes(base) } else { Vec::new() }
        .into_iter()
        .filter_map(|p| p.prepared.map(|ready| (p.preview, ready)))
        .collect();
    for path in paths {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let result = (|| -> Result<PathBuf, String> {
            if !path.is_file()
                || !(is_image(path) || !as_shapes && media_loader::is_video_path(path))
            {
                return Err("Unsupported file / Неподдерживаемый файл".into());
            }
            // Decode before copying a shape, avoiding unprepared library entries on failure.
            let converted = if as_shapes {
                shape_conversion::validate_resolution(resolution)?;
                let img = shape_conversion::open_bounded(path, 512 * 1024 * 1024)
                    .map_err(|e| e.to_string())?;
                Some(shape_conversion::process_shape(&img, resolution))
            } else {
                None
            };
            let destination = base.join(if as_shapes {
                "raw_shapes"
            } else {
                "input_media"
            });
            let imported = if as_shapes {
                let name = available_shape_name(base, path, &mut existing_stems)?;
                super::import::import_as(path, &destination, &name)?
            } else {
                import_files(&[path.clone()], &destination)?.remove(0)
            };
            if let Some(img) = converted {
                let ready = base.join("input_shapes");
                std::fs::create_dir_all(&ready).map_err(|e| e.to_string())?;
                // Full file name keeps jpg/png with the same stem independent.
                let output = prepared_by_source
                    .get(&imported)
                    .cloned()
                    .unwrap_or_else(|| {
                        ready.join(format!(
                            "{}.png",
                            imported.file_name().unwrap().to_string_lossy()
                        ))
                    });
                super::output::check_disk_space(&ready, u64::from(resolution).pow(2) * 4)
                    .map_err(|e| e.to_string())?;
                img.save(&output).map_err(|e| e.to_string())?;
                prepared_by_source.insert(imported.clone(), output);
            }
            Ok(imported)
        })();
        match result {
            Ok(path) => {
                report.imported += 1;
                if !as_shapes && report.first_media.is_none() {
                    report.first_media = Some(path);
                }
            }
            Err(e) => report.errors.push(format!("{name}: {e}")),
        }
    }
    report
}

/// Reserve a name across originals and prepared assets, including legacy files.
/// This avoids treating a pre-existing unrelated PNG as the new photo's copy.
fn available_shape_name(
    base: &Path,
    source: &Path,
    existing_stems: &mut HashSet<String>,
) -> Result<std::ffi::OsString, String> {
    let raw = base.join("raw_shapes");
    if source
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        == raw.canonicalize().ok()
    {
        return Ok(source.file_name().ok_or("Invalid filename")?.to_os_string());
    }
    let stem = source
        .file_stem()
        .ok_or("Invalid filename")?
        .to_string_lossy();
    let ext = source.extension().unwrap_or_default().to_string_lossy();
    for suffix in 0u64.. {
        let stem = if suffix == 0 {
            stem.to_string()
        } else {
            format!("{stem}_{suffix}")
        };
        let name = format!("{stem}.{ext}");
        if !existing_stems.contains(&stem.to_lowercase())
            && !raw.join(&name).exists()
            && !base
                .join("input_shapes")
                .join(format!("{name}.png"))
                .exists()
            && !base
                .join("input_shapes")
                .join(format!("{stem}.png"))
                .exists()
        {
            existing_stems.insert(stem.to_lowercase());
            return Ok(name.into());
        }
    }
    unreachable!()
}

const LIBRARIES: [&str; 3] = ["input_media", "raw_shapes", "input_shapes"];

pub fn shape_root(base: &Path, set: Option<&str>) -> PathBuf {
    match set {
        Some(name)
            if !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', ':']) =>
        {
            base.join("frame_sets").join(name)
        }
        _ => base.to_path_buf(),
    }
}

pub fn shape_sets(base: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(base.join("frame_sets"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && e.path().join("raw_shapes").is_dir()
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

fn owned_directory(base: &Path, name: &str) -> Result<PathBuf, String> {
    let base = base.canonicalize().map_err(|e| e.to_string())?;
    let folder = base.join(name);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let actual = folder.canonicalize().map_err(|e| e.to_string())?;
    if actual != folder {
        return Err(
            "Library directory must not redirect outside the app / Папка библиотеки перенаправлена"
                .into(),
        );
    }
    Ok(actual)
}

fn owned_file(base: &Path, path: &Path) -> Result<(String, PathBuf), String> {
    if !std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err(
            "Only regular library files can be deleted / Можно удалять только файлы библиотеки"
                .into(),
        );
    }
    let actual = path.canonicalize().map_err(|e| e.to_string())?;
    for library in LIBRARIES {
        if actual.parent() == Some(owned_directory(base, library)?.as_path()) {
            return Ok((library.into(), actual));
        }
    }
    Err("File is outside the application library / Файл находится вне библиотеки приложения".into())
}

/// Move a batch, with rollback on failure, into the app's recoverable trash.
pub fn delete(base: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let mut unique = HashSet::new();
    let files: Vec<_> = paths
        .iter()
        .map(|p| owned_file(base, p))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(_, p)| unique.insert(p.clone()))
        .collect();
    if files.is_empty() {
        return Ok(());
    }
    let trash = owned_directory(base, ".library_trash")?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let batch = trash.join(format!("{stamp:032}"));
    std::fs::create_dir(&batch).map_err(|e| e.to_string())?;
    let pairs: Vec<_> = files
        .into_iter()
        .map(|(library, original)| {
            let target = batch.join(library).join(original.file_name().unwrap());
            (original, target)
        })
        .collect();
    for (_, target) in &pairs {
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
    }
    move_batch(&pairs)
}

fn move_batch(pairs: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    for (index, (source, target)) in pairs.iter().enumerate() {
        let result = if target.exists() {
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "File already exists / Файл уже существует: {}",
                    target.display()
                ),
            ))
        } else {
            std::fs::rename(source, target)
        };
        if let Err(e) = result {
            let mut rollback_errors = Vec::new();
            for (old, new) in pairs[..index].iter().rev() {
                if let Err(err) = std::fs::rename(new, old) {
                    rollback_errors.push(err.to_string());
                }
            }
            return Err(format!("{e}. {}", rollback_errors.join("; ")));
        }
    }
    Ok(())
}

pub fn last_deleted(base: &Path) -> Option<PathBuf> {
    let mut batches: Vec<_> = std::fs::read_dir(base.join(".library_trash"))
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            LIBRARIES.iter().any(|name| {
                std::fs::read_dir(p.join(name))
                    .map(|mut r| r.next().is_some())
                    .unwrap_or(false)
            })
        })
        .collect();
    batches.sort();
    batches.pop()
}

pub fn undo_delete(base: &Path) -> Result<(), String> {
    let batch = last_deleted(base)
        .ok_or("Nothing to restore / Нечего восстанавливать")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let trash = owned_directory(base, ".library_trash")?;
    if batch.canonicalize().map_err(|e| e.to_string())?.parent() != Some(trash.as_path()) {
        return Err("Invalid trash path / Неверный путь корзины".into());
    }
    let mut pairs = Vec::new();
    for library in LIBRARIES {
        let dest = owned_directory(base, library)?;
        let source = batch.join(library);
        if !source.exists() {
            continue;
        }
        if source.canonicalize().map_err(|e| e.to_string())? != source {
            return Err("Invalid trash directory".into());
        }
        for entry in std::fs::read_dir(&source).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                return Err("Invalid trash item".into());
            }
            let target = dest.join(entry.file_name());
            if target.exists() {
                return Err(format!(
                    "Restore would overwrite / Восстановление заменит файл: {}",
                    target.display()
                ));
            }
            pairs.push((entry.path(), target));
        }
    }
    move_batch(&pairs)?;
    for library in LIBRARIES {
        let _ = std::fs::remove_dir(batch.join(library));
    }
    let _ = std::fs::remove_dir(batch);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deleting_from_a_video_set_preserves_the_main_library() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("app");
        std::fs::create_dir(&base).unwrap();
        let source = temp.path().join("кадр.png");
        image::RgbImage::from_pixel(4, 4, image::Rgb([20, 30, 40]))
            .save(&source)
            .unwrap();
        assert_eq!(import(&base, &[source.clone()], true, 4).imported, 1);
        let set = shape_root(&base, Some("clip-123"));
        assert_eq!(import(&set, &[source.clone()], true, 4).imported, 1);
        assert_eq!(shape_sets(&base), vec!["clip-123"]);
        delete(&set, &shapes(&set)[0].files()).unwrap();
        assert!(shapes(&set).is_empty());
        assert_eq!(shapes(&base).len(), 1);
        assert!(source.is_file());
        undo_delete(&set).unwrap();
        assert_eq!(shapes(&set).len(), 1);
        for name in ["../escape", "C:\\outside", ".hidden"] {
            assert_eq!(shape_root(&base, Some(name)), base);
        }
    }
    #[test]
    fn importing_never_replaces_an_existing_prepared_only_asset() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("app");
        let ready = base.join("input_shapes");
        std::fs::create_dir_all(&ready).unwrap();
        let original = ready.join("photo.png");
        image::RgbImage::new(2, 2).save(&original).unwrap();
        let before = std::fs::read(&original).unwrap();
        let incoming = temp.path().join("photo.jpg");
        image::RgbImage::from_pixel(2, 2, image::Rgb([255, 255, 255]))
            .save(&incoming)
            .unwrap();
        let report = import(&base, &[incoming], true, 2);
        assert_eq!(report.imported, 1);
        assert_eq!(std::fs::read(original).unwrap(), before);
        assert_eq!(shapes(&base).len(), 2);
    }

    #[test]
    fn import_pair_delete_and_restore_preserve_external_originals() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("app");
        std::fs::create_dir(&base).unwrap();
        let source = temp.path().join("фото.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let bytes = std::fs::read(&source).unwrap();
        let report = import(&base, &[source.clone()], true, 4);
        assert_eq!(report.imported, 1);
        assert!(report.errors.is_empty());
        let photos = shapes(&base);
        assert_eq!(photos.len(), 1);
        assert_eq!(
            image::open(photos[0].prepared.as_ref().unwrap())
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)[3],
            255
        );
        delete(&base, &photos[0].files()).unwrap();
        assert!(shapes(&base).is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        undo_delete(&base).unwrap();
        assert_eq!(shapes(&base).len(), 1);
        assert!(delete(&base, &[source]).is_err());
    }

    #[test]
    fn same_stem_imports_stay_distinct_and_bad_files_are_reported() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("app");
        std::fs::create_dir(&base).unwrap();
        let png = temp.path().join("photo.png");
        let jpg = temp.path().join("photo.jpg");
        image::RgbImage::new(2, 2).save(&png).unwrap();
        image::RgbImage::new(2, 2).save(&jpg).unwrap();
        let bad = temp.path().join("bad.png");
        std::fs::write(&bad, "not an image").unwrap();
        let report = import(&base, &[png, bad, jpg], true, 2);
        assert_eq!(report.imported, 2);
        assert_eq!(report.errors.len(), 1);
        let photos = shapes(&base);
        assert_eq!(photos.len(), 2);
        assert_ne!(photos[0].prepared, photos[1].prepared);
        shape_conversion::convert_folder(&base.join("raw_shapes"), &base.join("input_shapes"), 2)
            .unwrap();
        assert_eq!(shapes(&base).len(), 2);
        delete(&base, &photos[0].files()).unwrap();
        assert_eq!(shapes(&base).len(), 1);
        assert!(photos[1].prepared.as_ref().unwrap().exists());
    }

    #[test]
    fn undo_never_overwrites_and_legacy_prepared_assets_are_visible() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        let ready = base.join("input_shapes");
        std::fs::create_dir(&ready).unwrap();
        let path = ready.join("legacy.png");
        std::fs::write(&path, b"original").unwrap();
        let photos = shapes(base);
        assert_eq!(photos.len(), 1);
        delete(base, &photos[0].files()).unwrap();
        std::fs::write(&path, b"new").unwrap();
        assert!(undo_delete(base).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        std::fs::remove_file(&path).unwrap();
        undo_delete(base).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }
}
