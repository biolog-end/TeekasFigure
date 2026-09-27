use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Background {
    Keep,
    Border,
    Ai,
}
impl Background {
    fn argument(self) -> &'static str {
        match self {
            Self::Keep => "none",
            Self::Border => "border",
            Self::Ai => "ai",
        }
    }
}

#[derive(Clone)]
pub struct Options {
    pub video: PathBuf,
    pub start: f64,
    pub end: Option<f64>,
    pub interval: f64,
    pub max_side: u32,
    pub background: Background,
    pub tolerance: u8,
    pub crop: bool,
    pub skip_blank: bool,
    pub resolution: u32,
}

#[derive(Default)]
pub struct Progress {
    pub cancelled: AtomicBool,
    pub completed: AtomicU64,
    pub total: AtomicU64,
    pub stage: Mutex<String>,
}
impl Progress {
    fn stage(&self, text: &str) {
        *self.stage.lock().unwrap() = text.into();
        self.completed.store(0, Ordering::Relaxed);
    }
    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err("Cancelled / Отменено".into())
        } else {
            Ok(())
        }
    }
}

pub fn parse_time(text: &str) -> Result<f64, String> {
    let fields: Vec<_> = text.trim().split(':').collect();
    if fields.is_empty() || fields.len() > 3 {
        return Err("Use seconds or HH:MM:SS / Введите секунды или ЧЧ:ММ:СС".into());
    }
    let mut result = 0.0;
    for (index, field) in fields.iter().enumerate() {
        let value = field
            .trim()
            .parse::<f64>()
            .map_err(|_| "Invalid time / Неверное время")?;
        if !value.is_finite() || value < 0.0 || index > 0 && value >= 60.0 {
            return Err("Invalid time / Неверное время".into());
        }
        result = result * 60.0 + value;
    }
    Ok(result)
}

pub fn model_path(base: &Path) -> Option<PathBuf> {
    let mut folders = vec![base.join("models")];
    if let Some(folder) = std::env::var_os("U2NET_HOME") {
        folders.push(folder.into());
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        folders.push(PathBuf::from(&profile).join(".u2net"));
        folders.push(PathBuf::from(profile).join(".rembg/models"));
    }
    folders
        .into_iter()
        .flat_map(|p| [p.join("u2netp.onnx"), p.join("u2net.onnx")])
        .find(|p| p.is_file())
}

fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

/// Bounded output draining; each child can be killed directly (no shell/process tree).
fn run(
    mut command: Command,
    progress: &Arc<Progress>,
    working_dir: &Path,
    timeout: Duration,
) -> Result<String, String> {
    progress.check()?;
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start tool / Не удалось запустить инструмент: {e}"))?;
    let tail = Arc::new(Mutex::new(String::new()));
    let mut readers = Vec::new();
    let streams: Vec<Box<dyn std::io::Read + Send>> = vec![
        Box::new(child.stdout.take().unwrap()),
        Box::new(child.stderr.take().unwrap()),
    ];
    for stream in streams {
        let tail = tail.clone();
        let progress = progress.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if let Some(line) = line.strip_prefix("PROGRESS\t") {
                    let values: Vec<_> = line.split('\t').collect();
                    if values.len() >= 2 {
                        progress
                            .completed
                            .store(values[0].parse().unwrap_or(0), Ordering::Relaxed);
                        progress
                            .total
                            .store(values[1].parse().unwrap_or(1), Ordering::Relaxed);
                    }
                } else if let Some(frame) = line.strip_prefix("frame=") {
                    progress
                        .completed
                        .store(frame.trim().parse().unwrap_or(0), Ordering::Relaxed);
                } else {
                    let mut log = tail.lock().unwrap();
                    if log.len() + line.len() > 8192 {
                        log.clear();
                    }
                    log.push_str(&line.chars().take(2048).collect::<String>());
                    log.push('\n');
                }
            }
        }));
    }
    let started = Instant::now();
    let mut last_check = Instant::now();
    let result = loop {
        if let Err(error) = progress.check() {
            break Err(error);
        }
        if started.elapsed() > timeout {
            break Err("Processing timeout / Превышено время ожидания инструмента".into());
        }
        if last_check.elapsed() >= Duration::from_secs(1) {
            if let Err(error) = super::output::check_disk_space(working_dir, 0) {
                break Err(error.to_string());
            }
            if crate::monitor::available_memory() < 256 * 1024 * 1024 {
                break Err("Low RAM / Недостаточно памяти".into());
            }
            last_check = Instant::now();
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(format!("Tool exited / Код завершения: {status}"))
                }
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => break Err(e.to_string()),
        }
    };
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    for reader in readers {
        let _ = reader.join();
    }
    let output = tail.lock().unwrap().clone();
    result
        .map(|()| output.clone())
        .map_err(|e| format!("{e}\n{output}"))
}

pub fn create(base: &Path, options: &Options, progress: &Arc<Progress>) -> Result<String, String> {
    if !options.start.is_finite()
        || options.start < 0.0
        || options.end.is_some_and(|end| !end.is_finite())
        || !options.interval.is_finite()
        || options.interval <= 0.0
        || options.max_side == 0
    {
        return Err(
            "Invalid range, interval or size / Неверный диапазон, интервал или размер".into(),
        );
    }
    super::shape_conversion::validate_resolution(options.resolution)?;
    if u64::from(options.max_side).pow(2).saturating_mul(32)
        > crate::monitor::available_memory() / 3
    {
        return Err(
            "Frame size exceeds memory budget / Размер кадра превышает бюджет памяти".into(),
        );
    }
    let sets = base.join("frame_sets");
    std::fs::create_dir_all(&sets).map_err(|e| e.to_string())?;
    progress.stage("Reading video / Чтение видео");
    let mut probe = command("ffprobe");
    probe
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "format=duration:stream=width,height",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(&options.video);
    let metadata = run(probe, progress, &sets, Duration::from_secs(30))?;
    let value = |key: &str| {
        metadata
            .lines()
            .filter_map(|line| line.split_once('='))
            .find(|(name, _)| *name == key)
            .and_then(|(_, number)| number.parse::<f64>().ok())
    };
    let duration = value("duration")
        .filter(|n| n.is_finite() && *n > 0.0)
        .ok_or("Cannot read video duration / Не удалось прочитать длительность видео")?;
    let source_pixels = value("width").unwrap_or(0.0) * value("height").unwrap_or(0.0);
    if !source_pixels.is_finite() || source_pixels <= 0.0 {
        return Err("No video stream / Видеодорожка не найдена".into());
    }
    let processing_bytes = source_pixels * 32.0 + (options.max_side as f64).powi(2) * 32.0;
    if processing_bytes > crate::monitor::available_memory() as f64 / 3.0 {
        return Err(
            "Video decoding exceeds memory budget / Декодирование видео превышает бюджет памяти"
                .into(),
        );
    }
    let end = options.end.unwrap_or(duration).min(duration);
    if !end.is_finite() || end <= options.start {
        return Err("End must be after start within the video / Конец должен быть позже начала в пределах видео".into());
    }
    let frames = ((end - options.start) / options.interval).ceil();
    if !frames.is_finite() || frames > u32::MAX as f64 {
        return Err("Too many output files / Слишком много выходных файлов".into());
    }
    let estimated = (frames as u64).saturating_mul(
        u64::from(options.max_side).pow(2).saturating_mul(4)
            + u64::from(options.resolution).pow(2) * 4,
    );
    super::output::check_disk_space(&sets, estimated).map_err(|e| e.to_string())?;
    let model = if options.background == Background::Ai {
        Some(model_path(base).ok_or("U²-Net model not found. Use models/u2net.onnx or the existing rembg cache / Модель U²-Net не найдена: models/u2net.onnx или кэш rembg")?)
    } else {
        None
    };
    if options.background != Background::Keep || options.skip_blank || options.crop {
        progress.stage("Checking image tools / Проверка обработки изображений");
        let mut python = command("python");
        python.arg("-c").arg(if model.is_some() {
            "import PIL, numpy, onnxruntime"
        } else {
            "import PIL, numpy"
        });
        run(python,progress,&sets,Duration::from_secs(30))
            .map_err(|error| format!("Install Python packages listed in README / Установите пакеты Python по инструкции README\n{error}"))?;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let stem: String = options
        .video
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(50)
        .collect();
    let name = format!("{stem}-{stamp}");
    let staging = sets.join(format!(".building-{name}"));
    std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let raw = staging.join("raw_shapes");
    let result = (|| {
        std::fs::create_dir(&raw).map_err(|e| e.to_string())?;
        progress.stage("Extracting frames / Извлечение кадров");
        progress.total.store(frames as u64, Ordering::Relaxed);
        let mut ffmpeg = command("ffmpeg");
        ffmpeg.args(["-v","error","-nostdin","-nostats","-filter_threads","2","-progress","pipe:1","-ss"]).arg(options.start.to_string()).args(["-threads","2","-i"]).arg(&options.video)
            .arg("-t").arg((end-options.start).to_string()).args(["-an","-sn","-vf"])
            .arg(format!("fps=fps=1/{}:start_time=0:round=up,scale='min({},iw)':'min({},ih)':force_original_aspect_ratio=decrease", options.interval, options.max_side,options.max_side))
            .arg("-frames:v").arg((frames as u64).to_string()).args(["-threads","2","-y"]).arg(raw.join("frame_%06d.png"));
        run(
            ffmpeg,
            progress,
            &staging,
            Duration::from_secs(24 * 60 * 60),
        )?;
        if options.background != Background::Keep || options.skip_blank || options.crop {
            progress.stage("Cleaning backgrounds / Обработка фона");
            let script = staging.join("background.py");
            std::fs::write(&script, include_str!("../../scripts/frame_background.py"))
                .map_err(|e| e.to_string())?;
            let mut python = command("python");
            python
                .arg("-u")
                .arg(&script)
                .arg(&raw)
                .arg("--mode")
                .arg(options.background.argument())
                .arg("--tolerance")
                .arg(options.tolerance.to_string());
            if options.crop {
                python.arg("--crop");
            }
            if options.skip_blank {
                python.arg("--skip-blank");
            }
            if let Some(model) = model {
                python.arg("--model").arg(model);
            }
            run(
                python,
                progress,
                &staging,
                Duration::from_secs(24 * 60 * 60),
            )?;
            let _ = std::fs::remove_file(script);
        }
        progress.check()?;
        let paths = super::library::scan(&raw, false);
        if paths.is_empty() {
            return Err("No objects remain. Try Keep background or another range / Объекты не найдены: сохраните фон или выберите другой диапазон".into());
        }
        progress.stage("Preparing shapes / Подготовка фигур");
        let ready = staging.join("input_shapes");
        std::fs::create_dir(&ready).map_err(|e| e.to_string())?;
        progress.total.store(paths.len() as u64, Ordering::Relaxed);
        for (index, path) in paths.iter().enumerate() {
            progress.check()?;
            super::output::check_disk_space(&ready, u64::from(options.resolution).pow(2) * 4)
                .map_err(|e| e.to_string())?;
            let image =
                super::shape_conversion::open_bounded(path, crate::monitor::available_memory() / 3)
                    .map_err(|e| e.to_string())?;
            super::shape_conversion::process_shape(&image, options.resolution)
                .save(ready.join(path.file_name().unwrap()))
                .map_err(|e| e.to_string())?;
            progress
                .completed
                .store(index as u64 + 1, Ordering::Relaxed);
        }
        std::fs::write(
            staging.join("source.txt"),
            format!(
                "{}\nstart={}\nend={}\ninterval={}\nbackground={:?}\n",
                options.video.display(),
                options.start,
                end,
                options.interval,
                options.background
            ),
        )
        .map_err(|e| e.to_string())?;
        progress.check()?;
        std::fs::rename(&staging, sets.join(&name)).map_err(|e| e.to_string())?;
        Ok(name)
    })();
    if result.is_err() {
        // Only this freshly generated staging tree may be removed.
        if let (Ok(parent), Ok(path)) = (sets.canonicalize(), staging.canonicalize()) {
            if path.parent() == Some(parent.as_path())
                && path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".building-")
            {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn time_input_accepts_minutes_and_rejects_invalid_ranges() {
        assert_eq!(parse_time("2:00").unwrap(), 120.0);
        assert_eq!(parse_time("00:03:01.5").unwrap(), 181.5);
        for value in ["nan", "-1", "2:90", "", "1:2:3:4"] {
            assert!(parse_time(value).is_err());
        }
    }

    #[test]
    #[ignore = "requires Python; verifies cancellation of an already running child"]
    fn cancellation_interrupts_a_running_tool() {
        let temp = tempfile::tempdir().unwrap();
        let progress = Arc::new(Progress::default());
        let observed = progress.clone();
        let cancel = std::thread::spawn(move || {
            let start = Instant::now();
            while observed.completed.load(Ordering::Relaxed) == 0 {
                assert!(start.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(10));
            }
            observed.cancelled.store(true, Ordering::Relaxed);
        });
        let mut child = command("python");
        child.args([
            "-u",
            "-c",
            "import time; print('PROGRESS\\t1\\t2\\t1', flush=True); time.sleep(30)",
        ]);
        let started = Instant::now();
        let result = run(child, &progress, temp.path(), Duration::from_secs(35));
        cancel.join().unwrap();
        assert!(result.unwrap_err().contains("Cancelled"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    #[ignore = "requires FFmpeg; verifies actual sampled timestamps and cancellation"]
    fn extracts_only_the_requested_range_and_builds_an_isolated_set() {
        let temp = tempfile::tempdir().unwrap();
        let video = temp.path().join("Фрагмент видео.mkv");
        let mut generate = command("ffmpeg");
        let status = generate
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x48:rate=10:duration=4",
                "-c:v",
                "ffv1",
            ])
            .arg(&video)
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        let progress = Arc::new(Progress::default());
        let options = Options {
            video: video.clone(),
            start: 1.0,
            end: Some(3.0),
            interval: 0.5,
            max_side: 64,
            background: Background::Keep,
            tolerance: 40,
            crop: false,
            skip_blank: false,
            resolution: 32,
        };
        let set = create(temp.path(), &options, &progress).unwrap();
        let root = super::super::library::shape_root(temp.path(), Some(&set));
        let frames = super::super::library::scan(&root.join("raw_shapes"), false);
        assert_eq!(frames.len(), 4);
        assert_eq!(super::super::library::shapes(&root).len(), 4);
        assert!(!temp.path().join("raw_shapes").exists());
        for (index, frame) in frames.iter().enumerate() {
            let mut reference = command("ffmpeg");
            let output = reference
                .args(["-v", "error", "-ss"])
                .arg((1.0 + index as f64 * 0.5).to_string())
                .arg("-i")
                .arg(&video)
                .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(
                image::open(frame).unwrap().to_rgb8().into_raw(),
                output.stdout,
                "Incorrect timestamp for sampled frame {index}"
            );
        }
        progress.cancelled.store(true, Ordering::Relaxed);
        assert!(create(temp.path(), &options, &progress).is_err());
        assert_eq!(super::super::library::shape_sets(temp.path()), vec![set]);
        // Processing a plain black clip leaves no useful brushes; discard only
        // its temporary set and keep the previously completed library intact.
        let blank = temp.path().join("Пустой кадр.mkv");
        let status = command("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=black:size=64x48:duration=1",
                "-c:v",
                "ffv1",
            ])
            .arg(&blank)
            .output()
            .unwrap();
        assert!(status.status.success());
        let blank_options = Options {
            video: blank,
            start: 0.0,
            end: None,
            background: Background::Border,
            crop: true,
            skip_blank: true,
            ..options
        };
        assert!(
            create(temp.path(), &blank_options, &Arc::new(Progress::default()))
                .unwrap_err()
                .contains("No objects remain")
        );
        assert_eq!(
            std::fs::read_dir(temp.path().join("frame_sets"))
                .unwrap()
                .count(),
            1
        );
    }
}
