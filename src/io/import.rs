use std::path::{Path, PathBuf};

/// Copy user-selected files without replacing any existing original.
pub fn import_files(files: &[PathBuf], destination: &Path) -> Result<Vec<PathBuf>, String> {
    files
        .iter()
        .map(|source| {
            import_as(
                source,
                destination,
                source.file_name().ok_or("Invalid file name")?,
            )
        })
        .collect()
}

pub fn import_as(
    source: &Path,
    destination: &Path,
    name: &std::ffi::OsStr,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut target = destination.join(name);
    if source.canonicalize().ok() == target.canonicalize().ok() && target.exists() {
        return Ok(target);
    }
    super::output::check_disk_space(
        destination,
        input.metadata().map_err(|e| e.to_string())?.len(),
    )
    .map_err(|e| e.to_string())?;
    let mut suffix = 1u64;
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(mut output) => {
                if let Err(e) = std::io::copy(&mut input, &mut output) {
                    drop(output);
                    let _ = std::fs::remove_file(&target);
                    return Err(format!("{}: {e}", source.display()));
                }
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                target = destination.join(format!(
                    "{}_{}.{}",
                    Path::new(name)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    suffix,
                    Path::new(name)
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
                suffix += 1;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_names_do_not_overwrite_originals() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("photo.png");
        let dest = tmp.path().join("imported");
        std::fs::write(&source, b"first").unwrap();
        import_files(&[source.clone()], &dest).unwrap();
        std::fs::write(&source, b"second").unwrap();
        let imported = import_files(&[source], &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("photo.png")).unwrap(), b"first");
        assert_eq!(std::fs::read(&imported[0]).unwrap(), b"second");
        import_files(&imported, &dest).unwrap();
        assert_eq!(std::fs::read_dir(dest).unwrap().count(), 2);
    }
}
