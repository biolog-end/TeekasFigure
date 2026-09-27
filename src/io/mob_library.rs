//! Import Fabric's capture packages and retain model geometry through scene export.
use serde_json::{json, Value};
#[cfg(test)]
#[path = "mob_library_tests.rs"]
mod tests;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MobView {
    Top,
    Front,
}

impl MobView {
    fn parse(value: &Value) -> Result<Self, String> {
        match value.as_str() {
            Some("top") => Ok(Self::Top),
            Some("front") => Ok(Self::Front),
            _ => Err("Choose a completed top/front mob export / Выберите завершённый экспорт мобов сверху/спереди".into()),
        }
    }

    fn folder_prefix(self) -> &'static str {
        match self {
            Self::Top => "mobs_top",
            Self::Front => "mobs_front",
        }
    }
}

pub struct ImportedMobSet {
    pub name: String,
    pub view: MobView,
}

fn read_mapping(folder: &Path) -> Result<Value, String> {
    let path = folder.join("mob_mapping.json");
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 {
        return Err("Mob mapping is too large / Слишком большой маппинг".into());
    }
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if value["schema_version"] != 1 || value["complete"] != true {
        return Err("Choose a completed mob export / Выберите завершённый экспорт мобов".into());
    }
    MobView::parse(&value["view"])?;
    let mut names = HashSet::new();
    let entries = value["entries"].as_array().ok_or("Missing mob entries")?;
    if entries.is_empty() {
        return Err("No mob captures / Нет изображений мобов".into());
    }
    for row in entries {
        let name = row["source_name"].as_str().ok_or("Missing filename")?;
        if !matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        ) || !name.ends_with(".png")
            || !names.insert(name.to_lowercase())
        {
            return Err("Invalid or duplicate mob filename".into());
        }
        let entity = row["entity_type"].as_str().ok_or("Missing entity type")?;
        if entity.split(':').count() != 2
            || !entity
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_:./-".contains(&c))
        {
            return Err("Invalid entity identifier".into());
        }
        let number = |v: &Value| {
            v.as_f64()
                .filter(|n| n.is_finite())
                .ok_or("Invalid model geometry")
        };
        let size = row["reference_size_blocks"]
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or("Missing capture size")?;
        let span = number(&size[0])?;
        if span <= 0.0
            || (number(&size[1])? - span).abs() > span * 0.001
            || number(&row["depth_blocks"])? <= 0.0
        {
            return Err("Invalid model size".into());
        }
        number(&row["yaw_offset_degrees"])?;
        if let Some(n) = row.get("min_depth_blocks") {
            number(n)?;
        }
        let pivot = row["pivot_blocks"]
            .as_array()
            .filter(|a| a.len() == 3)
            .ok_or("Missing pivot")?;
        for n in pivot {
            number(n)?;
        }
    }
    Ok(value)
}

/// Originals are copied into a fresh set; an unfinished set is never offered in the UI.
pub fn import_set(base: &Path, source: &Path, resolution: u32) -> Result<ImportedMobSet, String> {
    let mapping = read_mapping(source)?;
    let view = MobView::parse(&mapping["view"])?;
    let raw = source
        .join("raw_shapes")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    for row in mapping["entries"].as_array().unwrap() {
        let path = raw.join(row["source_name"].as_str().unwrap());
        let resolved = path.canonicalize().map_err(|e| e.to_string())?;
        if resolved.parent() != Some(raw.as_path()) || !resolved.is_file() {
            return Err("Capture is outside the selected package".into());
        }
        paths.push(path);
    }
    let sets = base.join("frame_sets");
    fs::create_dir_all(&sets).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
    let name = format!("{}_{stamp}", view.folder_prefix());
    let stage = sets.join(format!(".building-{name}"));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        let report = super::library::import(&stage, &paths, true, resolution);
        if !report.errors.is_empty() || report.imported != paths.len() {
            return Err(report.message());
        }
        fs::write(
            stage.join("mob_mapping.json"),
            serde_json::to_vec_pretty(&mapping).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(&stage, sets.join(&name)).map_err(|e| e.to_string())?;
        Ok(ImportedMobSet { name, view })
    })();
    if result.is_err() {
        // Only the freshly created, verified app-owned staging directory may be removed.
        if let (Ok(resolved), Ok(root)) = (stage.canonicalize(), sets.canonicalize()) {
            if resolved.parent() == Some(root.as_path())
                && resolved.file_name() == stage.file_name()
            {
                let _ = fs::remove_dir_all(resolved);
            }
        }
    }
    result
}

pub fn scene_mapping(sources: &[PathBuf]) -> Result<Vec<Value>, String> {
    let mut packages = HashMap::<PathBuf, Option<Value>>::new();
    sources.iter().enumerate().map(|(index, source)| {
        let root = source.parent().and_then(Path::parent).unwrap_or(Path::new(""));
        if !packages.contains_key(root) {
            packages.insert(root.to_path_buf(), if root.join("mob_mapping.json").is_file() { Some(read_mapping(root)?) } else { None });
        }
        let name = source.file_name().unwrap_or_default().to_string_lossy();
        let package = packages[root].as_ref();
        let row = package.and_then(|p| p["entries"].as_array()).and_then(|rows|
            rows.iter().find(|r| r["source_name"].as_str() == Some(name.as_ref())).or_else(|| rows.iter().find(|r|
                source.parent().and_then(Path::file_name).is_some_and(|n| n == "input_shapes") && format!("{}.png",r["source_name"].as_str().unwrap_or("")) == name)));
        let mut row = row.cloned().unwrap_or_else(|| json!({"entity_type":null,"reference_size_blocks":null,"depth_blocks":null,"yaw_offset_degrees":0.0,"pivot_blocks":[0.0,0.0,0.0]}));
        row["view"] = package.map(|p| p["view"].clone()).unwrap_or(Value::Null);
        row["brush_index"] = json!(index);
        row["source_name"] = json!(name);
        Ok(row)
    }).collect()
}
