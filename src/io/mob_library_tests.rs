use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static ID: AtomicU64 = AtomicU64::new(0);
fn fixture() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "tf_mobs_{}_{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(p.join("external/raw_shapes")).unwrap();
    let entries:Vec<_>=["cow","creeper"].iter().map(|mob| {
        let name=format!("minecraft__{mob}.png");
        let mut image=image::RgbaImage::from_pixel(4,4,image::Rgba([35,125,72,255]));image.put_pixel(0,0,image::Rgba([0,0,0,0]));
        image.save(p.join("external/raw_shapes").join(&name)).unwrap();
        json!({"source_name":name,"entity_type":format!("minecraft:{mob}"),"reference_size_blocks":[2.0,2.0],"depth_blocks":1.5,"min_depth_blocks":-0.05,"yaw_offset_degrees":180.0,"pivot_blocks":[0.0,0.0,0.125]})
    }).collect();
    fs::write(
        p.join("external/mob_mapping.json"),
        serde_json::to_vec(
            &json!({"schema_version":1,"view":"top","complete":true,"entries":entries}),
        )
        .unwrap(),
    )
    .unwrap();
    p
}
#[test]
fn imports_separate_library_and_maps_loaded_subset_in_actual_order() {
    let p = fixture();
    let original = fs::read(p.join("external/raw_shapes/minecraft__cow.png")).unwrap();
    let imported = import_set(&p.join("app"), &p.join("external"), 16).unwrap();
    assert_eq!(imported.view, MobView::Top);
    let root = p.join("app/frame_sets").join(imported.name);
    let rows = scene_mapping(&[
        root.join("raw_shapes/minecraft__creeper.png"),
        root.join("input_shapes/minecraft__cow.png.png"),
    ])
    .unwrap();
    assert_eq!(rows[0]["entity_type"], "minecraft:creeper");
    assert_eq!(rows[0]["view"], "top");
    assert_eq!(rows[0]["brush_index"], 0);
    assert_eq!(rows[1]["entity_type"], "minecraft:cow");
    assert_eq!(rows[1]["pivot_blocks"][2], 0.125);
    assert_eq!(
        fs::read(root.join("raw_shapes/minecraft__cow.png")).unwrap(),
        original
    );
    assert_eq!(
        fs::read(p.join("external/raw_shapes/minecraft__cow.png")).unwrap(),
        original
    );
    assert!(root.join("input_shapes/minecraft__cow.png.png").is_file());
    assert_eq!(super::super::library::shape_sets(&p.join("app")).len(), 1);
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn front_export_is_imported_and_keeps_its_view_in_scene_mapping() {
    let p = fixture();
    let mut mapping = read_mapping(&p.join("external")).unwrap();
    mapping["view"] = json!("front");
    fs::write(
        p.join("external/mob_mapping.json"),
        serde_json::to_vec(&mapping).unwrap(),
    )
    .unwrap();
    let imported = import_set(&p.join("app"), &p.join("external"), 16).unwrap();
    assert_eq!(imported.view, MobView::Front);
    assert!(imported.name.starts_with("mobs_front_"));
    let root = p.join("app/frame_sets").join(imported.name);
    let rows = scene_mapping(&[root.join("raw_shapes/minecraft__cow.png")]).unwrap();
    assert_eq!(rows[0]["view"], "front");
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn incomplete_export_is_rejected_before_creating_library() {
    let p = fixture();
    let mut v = read_mapping(&p.join("external")).unwrap();
    v["complete"] = json!(false);
    fs::write(p.join("external/mob_mapping.json"), v.to_string()).unwrap();
    assert!(import_set(&p.join("app"), &p.join("external"), 16).is_err());
    assert!(!p.join("app/frame_sets").exists());
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn broken_capture_does_not_publish_partial_set() {
    let p = fixture();
    fs::write(p.join("external/raw_shapes/minecraft__cow.png"), b"broken").unwrap();
    assert!(import_set(&p.join("app"), &p.join("external"), 16).is_err());
    assert_eq!(fs::read_dir(p.join("app/frame_sets")).unwrap().count(), 0);
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn filenames_cannot_escape_capture_package() {
    let p = fixture();
    let mut v = read_mapping(&p.join("external")).unwrap();
    v["entries"][0]["source_name"] = json!("../outside.png");
    fs::write(p.join("external/mob_mapping.json"), v.to_string()).unwrap();
    assert!(read_mapping(&p.join("external")).is_err());
    fs::remove_dir_all(p).unwrap();
}
