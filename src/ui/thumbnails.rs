use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

enum Entry {
    Pending,
    Ready(Option<egui::TextureHandle>),
}

/// One decoder and bounded queues/cache keep large libraries off the UI thread.
pub struct Thumbnails {
    request: mpsc::SyncSender<(u64, PathBuf)>,
    response: mpsc::Receiver<(u64, PathBuf, Option<egui::ColorImage>)>,
    cache: HashMap<PathBuf, (Entry, u64)>,
    tick: u64,
    epoch: u64,
    stop: Arc<AtomicBool>,
}

impl Default for Thumbnails {
    fn default() -> Self {
        let (tx, requests) = mpsc::sync_channel::<(u64, PathBuf)>(16);
        let (results, rx) = mpsc::sync_channel(16);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        std::thread::spawn(move || {
            while let Ok((epoch, path)) = requests.recv() {
                if worker_stop.load(Ordering::Relaxed) {
                    break;
                }
                let image = if crate::io::media_loader::is_video_path(&path) {
                    video_preview(&path, || !worker_stop.load(Ordering::Relaxed))
                } else {
                    crate::io::shape_conversion::open_bounded(&path, 512 * 1024 * 1024)
                        .ok()
                        .map(|img| {
                            let thumb = img.thumbnail(144, 92).to_rgba8();
                            egui::ColorImage::from_rgba_unmultiplied(
                                [thumb.width() as usize, thumb.height() as usize],
                                thumb.as_raw(),
                            )
                        })
                };
                if results.send((epoch, path, image)).is_err() {
                    break;
                }
            }
        });
        Self {
            request: tx,
            response: rx,
            cache: HashMap::new(),
            tick: 0,
            epoch: 0,
            stop,
        }
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Decode only the first video frame, on the library worker. Never blocks the UI.
fn video_preview(path: &Path, keep_running: impl Fn() -> bool) -> Option<egui::ColorImage> {
    use std::{
        io::Read,
        process::{Command, Stdio},
    };
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-v",
            "error",
            "-nostdin",
            "-max_alloc",
            "268435456",
            "-threads",
            "1",
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-filter_threads",
            "1",
            "-vf",
            "scale=144:92:force_original_aspect_ratio=decrease",
            "-frames:v",
            "1",
            "-threads",
            "1",
            "-c:v",
            "png",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        // Output dimensions are fixed. Reject unexpectedly large output.
        stdout.take(128 * 1024).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    });
    let started = Instant::now();
    let success = loop {
        if !keep_running() || started.elapsed() > Duration::from_secs(10) {
            break false;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break false,
        }
    };
    if !success {
        let _ = child.kill();
        let _ = child.wait();
    }
    let bytes = reader.join().ok()??;
    if !success {
        return None;
    }
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    if image.width() > 144 || image.height() > 92 {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires FFmpeg; checks the first frame rather than a later thumbnail"]
    fn video_preview_uses_the_first_frame_and_fits_the_cache() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Два кадра видео.mkv");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=red:s=320x180:r=1:d=1",
                "-f",
                "lavfi",
                "-i",
                "color=blue:s=320x180:r=1:d=1",
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0",
                "-c:v",
                "ffv1",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(status.status.success());
        let preview = video_preview(&path, || true).unwrap();
        assert_eq!(preview.size, [144, 81]);
        assert!(preview.pixels.iter().all(|p| p.r() > 200 && p.b() < 30));
        assert!(video_preview(&path, || false).is_none());
    }
}

impl Thumbnails {
    pub fn clear(&mut self) {
        self.epoch += 1;
        self.cache.clear();
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((epoch, path, img)) = self.response.try_recv() {
            if epoch != self.epoch {
                continue;
            }
            if let Some((entry, _)) = self.cache.get_mut(&path) {
                *entry = Entry::Ready(img.map(|img| {
                    ctx.load_texture(path.to_string_lossy(), img, egui::TextureOptions::LINEAR)
                }));
            }
        }
        if self
            .cache
            .values()
            .any(|(e, _)| matches!(e, Entry::Pending))
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    pub fn get(&mut self, path: &PathBuf) -> Option<egui::TextureHandle> {
        self.tick += 1;
        if let Some((entry, touched)) = self.cache.get_mut(path) {
            *touched = self.tick;
            return match entry {
                Entry::Ready(texture) => texture.clone(),
                Entry::Pending => None,
            };
        }
        if self.request.try_send((self.epoch, path.clone())).is_ok() {
            if self.cache.len() >= 128 {
                let oldest = self
                    .cache
                    .iter()
                    .min_by_key(|(_, (_, tick))| tick)
                    .map(|(p, _)| p.clone());
                if let Some(p) = oldest {
                    self.cache.remove(&p);
                }
            }
            self.cache.insert(path.clone(), (Entry::Pending, self.tick));
        }
        None
    }
}
