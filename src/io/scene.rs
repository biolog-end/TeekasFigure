//! Streaming, versioned scene packages. No extra GPU readback or fitness work.
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::settings::Settings;
use crate::types::{CandidateParams, ShapeLayer};

#[derive(Debug, Serialize, Deserialize)]
pub struct Brush {
    pub index: u32,
    pub source_name: String,
    pub texture: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub source_name: String,
    pub canvas_px: [u32; 2],
    pub brush_resolution: u32,
    pub fps: f64,
    pub is_video: bool,
    pub complete: bool,
    pub frame_count: u64,
    pub coordinates: String,
    pub layer_order: String,
    pub brushes: Vec<Brush>,
    pub settings: Settings,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FrameShape {
    pub id: u64,
    pub brush_index: u32,
    pub center_px: [f32; 2],
    pub size_px: [f32; 2],
    pub rotation_radians: f32,
    pub opacity: f32,
    pub original_colors: bool,
    pub tint_rgb: [f32; 3],
    pub hue_turns: f32,
    pub saturation: f32,
    pub brightness: f32,
}

impl FrameShape {
    fn new(id: u64, p: CandidateParams, resolution: u32) -> Self {
        Self {
            id,
            brush_index: p.shape_index,
            center_px: [p.x, p.y],
            size_px: [p.scale * resolution as f32, p.scale_y * resolution as f32],
            rotation_radians: p.rotation,
            opacity: p.alpha,
            original_colors: p.use_original_color > 0.5,
            tint_rgb: [p.r, p.g, p.b],
            hue_turns: p.hue_shift,
            saturation: p.saturation_scale,
            brightness: p.brightness_scale,
        }
    }

    fn valid(&self) -> bool {
        self.center_px
            .iter()
            .chain(&self.size_px)
            .chain(&self.tint_rgb)
            .chain([
                &self.rotation_radians,
                &self.opacity,
                &self.hue_turns,
                &self.saturation,
                &self.brightness,
            ])
            .all(|x| x.is_finite())
            && self.size_px.iter().all(|x| *x > 0.0)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Frame {
    pub frame_index: u64,
    pub time_seconds: f64,
    pub shapes: Vec<FrameShape>,
}

pub struct SceneWriter {
    folder: PathBuf,
    frames: Option<BufWriter<File>>,
    committed_bytes: u64,
    manifest: Manifest,
}

fn failure(error: impl std::fmt::Display) -> AppError {
    AppError::SaveFailed {
        reason: format!("Scene export: {error}"),
    }
}

impl SceneWriter {
    pub fn new(
        output: &Path,
        source: &Path,
        size: (u32, u32),
        fps: f64,
        is_video: bool,
        settings: &Settings,
        layers: &[ShapeLayer],
        sources: &[PathBuf],
    ) -> Result<Self, AppError> {
        if layers.len() != sources.len() || !fps.is_finite() || fps <= 0.0 {
            return Err(failure("invalid source mapping or frame rate"));
        }
        fs::create_dir_all(output).map_err(failure)?;
        super::output::check_disk_space(
            output,
            layers.iter().map(|l| l.pixels.len() as u64).sum(),
        )?;
        let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
        let mut suffix = 0u64;
        let folder = loop {
            let path = output.join(format!("scene_{stamp}_{suffix}"));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => suffix += 1,
                Err(e) => return Err(failure(e)),
            }
        };
        // Create the incomplete manifest first: a failed asset write must never
        // leave a scene which advertises itself as a finished export.
        let manifest = Manifest {
            schema_version: 1,
            source_name: source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            canvas_px: [size.0, size.1],
            brush_resolution: settings.shape_resolution,
            fps,
            is_video,
            complete: false,
            frame_count: 0,
            coordinates:
                "pixels; centers; origin top-left; x right; y down; rotation clockwise radians"
                    .into(),
            layer_order: "shapes array is bottom-to-top; IDs persist; absence means removal".into(),
            brushes: sources
                .iter()
                .enumerate()
                .map(|(i, source)| Brush {
                    index: i as u32,
                    source_name: source
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    texture: format!("brushes/{i:04}.png"),
                })
                .collect(),
            settings: settings.clone(),
        };
        let mut writer = Self {
            frames: Some(BufWriter::new(
                File::create(folder.join("frames.jsonl")).map_err(failure)?,
            )),
            committed_bytes: 0,
            folder,
            manifest,
        };
        writer.save_manifest()?;
        fs::create_dir(writer.folder.join("brushes")).map_err(failure)?;
        for (i, layer) in layers.iter().enumerate() {
            image::save_buffer_with_format(
                writer.folder.join(format!("brushes/{i:04}.png")),
                &layer.pixels,
                settings.shape_resolution,
                settings.shape_resolution,
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .map_err(failure)?;
        }
        let mapping = super::mob_library::scene_mapping(sources).map_err(failure)?;
        fs::write(
            writer.folder.join("minecraft_mapping.json"),
            serde_json::to_vec_pretty(&mapping).map_err(failure)?,
        )
        .map_err(failure)?;
        fs::write(
            writer.folder.join("README.txt"),
            include_str!("../../scripts/minecraft_scene_README.txt"),
        )
        .map_err(failure)?;
        Ok(writer)
    }

    /// Only one frame is held in RAM. The same iterator also drives rendering.
    pub fn write_frame(
        &mut self,
        shapes: impl IntoIterator<Item = (u64, CandidateParams)>,
    ) -> Result<(), AppError> {
        if self.manifest.complete {
            return Err(failure("cannot append to a completed scene"));
        }
        let frame = Frame {
            frame_index: self.manifest.frame_count,
            time_seconds: self.manifest.frame_count as f64 / self.manifest.fps,
            shapes: shapes
                .into_iter()
                .map(|(id, p)| FrameShape::new(id, p, self.manifest.brush_resolution))
                .collect(),
        };
        if frame
            .shapes
            .iter()
            .any(|s| !s.valid() || s.brush_index as usize >= self.manifest.brushes.len())
        {
            return Err(failure("invalid transform or brush index"));
        }
        let data = serde_json::to_vec(&frame).map_err(failure)?;
        super::output::check_disk_space(&self.folder, data.len() as u64 + 1)?;
        let frames = self.frames.as_mut().unwrap();
        frames.write_all(&data).map_err(failure)?;
        frames.write_all(b"\n").map_err(failure)?;
        frames.flush().map_err(failure)?;
        self.committed_bytes += data.len() as u64 + 1;
        self.manifest.frame_count += 1;
        Ok(())
    }

    pub fn finish(&mut self) -> Result<PathBuf, AppError> {
        let frames = self.frames.as_mut().unwrap();
        frames.flush().map_err(failure)?;
        frames.get_ref().sync_all().map_err(failure)?;
        self.manifest.complete = true;
        if let Err(e) = self.save_manifest() {
            self.manifest.complete = false;
            return Err(e);
        }
        Ok(self.folder.clone())
    }

    fn save_manifest(&mut self) -> Result<(), AppError> {
        let data = serde_json::to_vec_pretty(&self.manifest).map_err(failure)?;
        let pending = self.folder.join("scene.json.pending");
        fs::write(&pending, data).map_err(failure)?;
        fs::rename(pending, self.folder.join("scene.json")).map_err(failure)
    }
}

impl Drop for SceneWriter {
    fn drop(&mut self) {
        if !self.manifest.complete {
            // Cancellation/failure retains useful partial frames, explicitly
            // marked incomplete. Never overwrite any previous run's files.
            if let Some(frames) = self.frames.take() {
                // Discard buffered/truncated data from a failed frame write.
                // Previously flushed full lines remain usable on cancellation.
                let (file, _) = frames.into_parts();
                let _ = file.set_len(self.committed_bytes);
            }
            let _ = self.save_manifest();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate() -> CandidateParams {
        CandidateParams {
            shape_index: 0,
            x: 10.5,
            y: 20.25,
            rotation: 0.75,
            scale: 2.0,
            scale_y: 2.0,
            r: 0.1,
            g: 0.2,
            b: 0.3,
            alpha: 1.0,
            use_original_color: 1.0,
            hue_shift: 0.0,
            saturation_scale: 1.0,
            brightness_scale: 1.0,
            _padding: [0.0; 2],
        }
    }

    fn writer(output: &Path) -> SceneWriter {
        SceneWriter::new(
            output,
            Path::new("видео.mp4"),
            (100, 80),
            24.0,
            true,
            &Settings {
                shape_resolution: 16,
                export_scene: true,
                ..Settings::default()
            },
            &[ShapeLayer {
                pixels: [37, 81, 129, 200].repeat(256),
            }],
            &[PathBuf::from("крипер.png")],
        )
        .unwrap()
    }

    #[test]
    fn scene_preserves_texture_names_transforms_order_and_stable_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let mut writer = writer(tmp.path());
        let p = candidate();
        writer.write_frame([(40, p), (7, p)]).unwrap();
        writer
            .write_frame([(7, CandidateParams { x: 11.0, ..p })])
            .unwrap();
        let folder = writer.finish().unwrap();
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(folder.join("scene.json")).unwrap()).unwrap();
        assert!(manifest.complete);
        assert_eq!(manifest.frame_count, 2);
        assert_eq!(manifest.brushes[0].source_name, "крипер.png");
        assert_eq!(manifest.source_name, "видео.mp4");
        let frames: Vec<Frame> = fs::read_to_string(folder.join("frames.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            frames[0].shapes.iter().map(|s| s.id).collect::<Vec<_>>(),
            [40, 7]
        );
        assert_eq!(frames[1].shapes[0].id, 7);
        assert_eq!(frames[1].shapes[0].center_px, [11.0, 20.25]);
        assert_eq!(frames[0].shapes[0].size_px, [32.0, 32.0]);
        assert_eq!(frames[0].shapes[0].rotation_radians, 0.75);
        assert_eq!(frames[1].time_seconds, 1.0 / 24.0);
        assert_eq!(
            image::open(folder.join("brushes/0000.png"))
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)
                .0,
            [37, 81, 129, 200]
        );
        assert!(folder.join("minecraft_mapping.json").is_file());
        // A second run gets a fresh directory, preserving the completed run.
        let other = self::writer(tmp.path());
        assert_ne!(other.folder, folder);
        assert!(folder.join("scene.json").is_file());
    }

    #[test]
    fn interrupted_scene_retains_complete_lines_and_incomplete_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let mut writer = writer(tmp.path());
        let folder = writer.folder.clone();
        writer.write_frame([(3, candidate())]).unwrap();
        assert!(writer
            .write_frame([(
                3,
                CandidateParams {
                    x: f32::NAN,
                    ..candidate()
                }
            )])
            .is_err());
        assert!(writer
            .write_frame([(
                3,
                CandidateParams {
                    shape_index: 9,
                    ..candidate()
                }
            )])
            .is_err());
        drop(writer);
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(folder.join("scene.json")).unwrap()).unwrap();
        assert!(!manifest.complete);
        assert_eq!(manifest.frame_count, 1);
        assert_eq!(
            fs::read_to_string(folder.join("frames.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }

    #[test]
    fn failed_write_does_not_commit_a_frame_or_flush_its_buffer_later() {
        let tmp = tempfile::tempdir().unwrap();
        let mut writer = writer(tmp.path());
        let folder = writer.folder.clone();
        writer.write_frame([(3, candidate())]).unwrap();
        // Inject a write failure without modifying disk permissions or globals.
        writer.frames = Some(BufWriter::new(
            File::open(folder.join("frames.jsonl")).unwrap(),
        ));
        assert!(writer.write_frame([(4, candidate())]).is_err());
        drop(writer);
        let lines = fs::read_to_string(folder.join("frames.jsonl")).unwrap();
        assert_eq!(lines.lines().count(), 1);
        let frame: Frame = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
        assert_eq!(frame.shapes[0].id, 3);
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(folder.join("scene.json")).unwrap()).unwrap();
        assert_eq!(manifest.frame_count, 1);
        assert!(!manifest.complete);
    }
}
