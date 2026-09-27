// The "Settings" screen: an egui form for configuring the approximator before
// launching generation. Features:
//   * media file picker (lists input_media/)
//   * smart sliders + toggles (conflicting/irrelevant options grey out)
//   * named presets (create / load / save / delete)
//   * English / Russian UI
//   * persists to settings.toml on Start (and on explicit Save)

use std::path::{Path, PathBuf};

use super::{
    native_dialog::{self, BrowsePurpose},
    thumbnails::Thumbnails,
};
use crate::io::{library, media_loader};
use crate::settings::{self, Settings};
use std::collections::HashSet;

use super::i18n::{self, Language};

/// Result of rendering the settings screen for one frame.
pub enum ScreenAction {
    /// Nothing to do this frame.
    None,
    /// User pressed Start with a valid media file and validated settings.
    Start {
        media_path: PathBuf,
        settings: Settings,
        language: Language,
    },
}

/// Owns the editable settings form and all UI-only state.
pub struct SettingsScreen {
    /// Working copy of the settings (bound to the widgets).
    pub settings: Settings,
    /// Current UI language.
    pub language: Language,

    base_dir: PathBuf,
    settings_path: PathBuf,
    presets_dir: PathBuf,
    media_dir: PathBuf,

    /// Discovered media files in input_media/.
    media_files: Vec<PathBuf>,
    selected_media: Option<usize>,

    /// Discovered preset names.
    presets: Vec<String>,
    selected_preset: Option<usize>,
    new_preset_name: String,

    /// Last status line (message, is_error).
    status: Option<(String, bool)>,
    browser: Option<(
        BrowsePurpose,
        std::sync::mpsc::Receiver<native_dialog::DialogResult>,
    )>,
    job: Option<std::sync::mpsc::Receiver<JobResult>>,
    thumbnails: Thumbnails,
    shape_photos: Vec<library::ShapePhoto>,
    marked_media: HashSet<PathBuf>,
    marked_shapes: HashSet<PathBuf>,
    pending_delete: Vec<PathBuf>,
    pending_delete_base: PathBuf,
    undo_base: PathBuf,
    shape_sets: Vec<String>,
    displayed_shape_set: Option<String>,
    frame_form: super::frame_set_form::FrameSetForm,
    frame_progress: Option<std::sync::Arc<crate::io::frame_set::Progress>>,
    adaptation_progress: Option<std::sync::Arc<crate::io::frame_set::Progress>>,
    adaptation_preview: Option<crate::io::palette_adaptation::Report>,
    dropped_files: Vec<PathBuf>,
    can_undo: bool,
    pub dialog_owner: usize,
    conversion_source: PathBuf,
    conversion_destination: PathBuf,
    pub active_gpu: String,
    pub available_gpus: Vec<String>,
    pub shape_layer_limit: usize,
}

struct JobResult {
    result: Result<String, String>,
    select_media: Option<PathBuf>,
    select_shape_set: Option<String>,
    mob_view: Option<crate::io::mob_library::MobView>,
    adapted: Option<crate::io::palette_adaptation::Report>,
}

impl SettingsScreen {
    fn use_mob_settings(&mut self) {
        self.settings.use_original_colors = true;
        self.settings.evolve_opacity = false;
        self.settings.evolve_non_uniform_scale = false;
        self.settings.evolve_hue = false;
        self.settings.evolve_saturation = false;
        self.settings.evolve_brightness = false;
        self.settings.video_recolor = false;
        self.settings.interpolation_steps = 0;
        self.settings.export_scene = true;
    }
    pub fn new(base_dir: &Path, settings: Settings, language: Language) -> Self {
        let mut screen = Self {
            settings,
            language,
            base_dir: base_dir.to_path_buf(),
            settings_path: base_dir.join("settings.toml"),
            presets_dir: base_dir.join("presets"),
            media_dir: base_dir.join("input_media"),
            media_files: Vec::new(),
            selected_media: None,
            presets: Vec::new(),
            selected_preset: None,
            new_preset_name: String::new(),
            status: None,
            browser: None,
            job: None,
            thumbnails: Thumbnails::default(),
            shape_photos: Vec::new(),
            marked_media: HashSet::new(),
            marked_shapes: HashSet::new(),
            pending_delete: Vec::new(),
            pending_delete_base: base_dir.to_path_buf(),
            undo_base: base_dir.to_path_buf(),
            shape_sets: Vec::new(),
            displayed_shape_set: None,
            frame_form: Default::default(),
            frame_progress: None,
            adaptation_progress: None,
            adaptation_preview: None,
            dropped_files: Vec::new(),
            can_undo: false,
            dialog_owner: 0,
            conversion_source: base_dir.join("raw_shapes"),
            conversion_destination: base_dir.join("input_shapes"),
            active_gpu: String::new(),
            available_gpus: Vec::new(),
            shape_layer_limit: 2048,
        };
        screen.refresh_media();
        screen.refresh_library();
        screen.refresh_presets();
        screen
    }

    /// Rescan input_media/ for supported files.
    fn refresh_media(&mut self) {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.media_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(media_loader::is_supported_extension)
                        .unwrap_or(false)
            })
            .collect();
        files.sort_by_key(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });

        // Preserve the current selection by name if possible.
        let prev = self
            .selected_media
            .and_then(|i| self.media_files.get(i).cloned());
        self.media_files = files;
        self.selected_media = match prev {
            Some(prev_path) => self.media_files.iter().position(|p| *p == prev_path),
            None => None,
        };
        if self.selected_media.is_none() && !self.media_files.is_empty() {
            self.selected_media = Some(0);
        }
    }

    /// Rescan the presets directory.
    fn refresh_presets(&mut self) {
        self.presets = settings::list_presets(&self.presets_dir);
        if let Some(i) = self.selected_preset {
            if i >= self.presets.len() {
                self.selected_preset = None;
            }
        }
    }

    pub fn set_status(&mut self, message: impl Into<String>, is_error: bool) {
        self.status = Some((message.into(), is_error));
    }

    pub fn cancel_browser(&mut self) -> bool {
        if !self.pending_delete.is_empty() {
            self.pending_delete.clear();
            return true;
        }
        if !self.dropped_files.is_empty() {
            self.dropped_files.clear();
            return true;
        }
        self.browser.is_some() || self.job.is_some()
    }

    pub fn is_busy(&self) -> bool {
        self.browser.is_some() || self.job.is_some()
    }

    pub fn cancel_file_job(&self) {
        if let Some(progress) = &self.adaptation_progress {
            progress.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(progress) = &self.frame_progress {
            progress
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn refresh_library(&mut self) {
        self.refresh_media();
        let shape_root = self.shape_base();
        self.shape_photos = library::shapes(&shape_root);
        self.shape_sets = library::shape_sets(&self.base_dir);
        if self.displayed_shape_set != self.settings.shape_set {
            self.marked_shapes.clear();
            self.conversion_source = shape_root.join("raw_shapes");
            self.conversion_destination = shape_root.join("input_shapes");
        }
        self.displayed_shape_set = self.settings.shape_set.clone();
        self.marked_media.retain(|p| self.media_files.contains(p));
        self.marked_shapes
            .retain(|p| self.shape_photos.iter().any(|s| s.preview == *p));
        let last = [self.base_dir.clone(), shape_root]
            .into_iter()
            .filter_map(|root| {
                library::last_deleted(&root)
                    .map(|batch| (batch.file_name().unwrap().to_os_string(), root))
            })
            .max_by(|a, b| a.0.cmp(&b.0));
        self.can_undo = last.is_some();
        self.undo_base = last
            .map(|(_, root)| root)
            .unwrap_or_else(|| self.base_dir.clone());
        self.thumbnails.clear();
    }

    fn shape_base(&self) -> PathBuf {
        library::shape_root(&self.base_dir, self.settings.shape_set.as_deref())
    }

    /// Force settings into a self-consistent state (resolve conflicts).
    /// Called every frame so toggles immediately reflect their effects.
    fn enforce_smart_rules(&mut self) {
        // diversity_mode and the min-improvement threshold conflict: diversity
        // wants to always accept the best candidate so penalties can steer it.
        if self.settings.diversity_mode {
            self.settings.use_min_improvement = false;
        }
    }

    /// Is the currently-selected media a video?
    fn selected_is_video(&self) -> bool {
        self.selected_media
            .and_then(|i| self.media_files.get(i))
            .map(|p| media_loader::is_video_path(p))
            .unwrap_or(false)
    }

    /// Render the whole screen and return any requested action.
    pub fn render(&mut self, ctx: &egui::Context) -> ScreenAction {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("ui_language"), self.language));
        self.poll_import(ctx);
        if self.displayed_shape_set != self.settings.shape_set {
            self.refresh_library();
        }
        self.thumbnails.poll(ctx);
        if !self.is_busy() && self.pending_delete.is_empty() && self.dropped_files.is_empty() {
            let import = ctx.input_mut(|input| {
                if input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::O) {
                    Some(BrowsePurpose::Shapes)
                } else if input.consume_key(egui::Modifiers::CTRL, egui::Key::O) {
                    Some(BrowsePurpose::Media)
                } else {
                    None
                }
            });
            if let Some(purpose) = import {
                self.browse(purpose);
            }
        }
        self.enforce_smart_rules();
        let mut action = ScreenAction::None;

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.header(ui);
                ui.separator();
                self.library_toolbar(ui);
                ui.columns(2, |columns| {
                    self.media_section(&mut columns[0]);
                    self.import_section(&mut columns[1]);
                });
                self.conversion_section(ui);
                ui.separator();
                self.gpu_section(ui);
                ui.separator();
                self.presets_section(ui);
                ui.separator();
                self.parameters(ui);
                ui.separator();
                action = self.footer(ui);
            });
        });

        self.file_modals(ctx);
        let busy = self.is_busy();
        let frame_action = self.frame_form.show(
            ctx,
            self.language,
            busy,
            &self.base_dir,
            self.settings.shape_resolution,
        );
        match frame_action {
            Some(super::frame_set_form::Action::Browse) => self.browse(BrowsePurpose::VideoFrames),
            Some(super::frame_set_form::Action::Start(options, use_target)) => {
                self.create_frame_set(options, use_target)
            }
            None => {}
        }

        action
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.horizontal(|ui| {
            ui.heading("TeekasFigure");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut changed = false;
                changed |= ui
                    .selectable_value(&mut self.language, Language::Russian, "Русский")
                    .changed();
                changed |= ui
                    .selectable_value(&mut self.language, Language::English, "English")
                    .changed();
                ui.label(lang.t("Language:", "Язык:"));
                if changed {
                    i18n::save_language(&self.base_dir, self.language);
                }
            });
        });
        ui.label(lang.t(
            "Configure the generator, then press Start.",
            "Настройте генератор и нажмите «Пуск».",
        ));
    }

    fn library_toolbar(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.horizontal_wrapped(|ui| {
            ui.label(lang.t(
                "Add files with the buttons or drag them into this window.",
                "Добавьте файлы кнопками или перетащите их в это окно.",
            ));
            if ui
                .add_enabled(
                    !self.is_busy(),
                    egui::Button::new(lang.t("Refresh", "Обновить")),
                )
                .clicked()
            {
                self.refresh_library();
            }
            if ui
                .add_enabled(
                    self.can_undo && !self.is_busy(),
                    egui::Button::new(lang.t("Undo deletion", "Отменить удаление")),
                )
                .clicked()
            {
                let base = self.undo_base.clone();
                self.start_job(move || {
                    library::undo_delete(&base).map(|()| "Restored / Восстановлено".into())
                });
            }
        });
        if self.is_busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if self.browser.is_some() {
                    lang.t(
                        "Choose files in the Windows dialog…",
                        "Выберите файлы в окне Windows…",
                    )
                } else {
                    lang.t("Processing files…", "Обработка файлов…")
                });
            });
        }
        if let Some(progress) = &self.frame_progress {
            let done = progress
                .completed
                .load(std::sync::atomic::Ordering::Relaxed);
            let total = progress
                .total
                .load(std::sync::atomic::Ordering::Relaxed)
                .max(1);
            ui.label(progress.stage.lock().unwrap().as_str());
            ui.add(
                egui::ProgressBar::new(done as f32 / total as f32)
                    .text(format!("{done} / {total}")),
            );
            if ui
                .button(lang.t("Cancel extraction", "Отменить создание набора"))
                .clicked()
            {
                progress
                    .cancelled
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if let Some(progress) = &self.adaptation_progress {
            let done = progress.completed.load(std::sync::atomic::Ordering::Relaxed);
            let total = progress.total.load(std::sync::atomic::Ordering::Relaxed);
            ui.label(progress.stage.lock().unwrap().as_str());
            if total > 0 {
                ui.add(egui::ProgressBar::new((done as f32 / total as f32).min(1.0)).text(format!("{done} / ~{total}")));
            } else {
                ui.label(format!("{}: {done}", lang.t("Frames processed", "Обработано кадров")));
            }
            if ui.button(lang.t("Cancel adaptation", "Отменить адаптацию")).clicked() {
                progress.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if let Some((message, error)) = &self.status {
            ui.colored_label(
                if *error {
                    egui::Color32::from_rgb(255, 140, 100)
                } else {
                    egui::Color32::from_rgb(130, 210, 170)
                },
                message,
            );
        }
        ui.add_space(6.0);
    }

    fn browse(&mut self, purpose: BrowsePurpose) {
        let folder = match purpose {
            BrowsePurpose::Source => self.conversion_source.clone(),
            BrowsePurpose::Destination => self.conversion_destination.clone(),
            _ => self.base_dir.clone(),
        };
        self.browser = Some((
            purpose,
            native_dialog::open(
                purpose,
                folder,
                self.dialog_owner,
                self.language == Language::Russian,
            ),
        ));
    }

    fn media_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.heading(lang.t("1. What to recreate", "1. Что собираем"));
            ui.label(lang.t(
                "Choose the target image or video. Click its preview to use it.",
                "Образец для финального результата. Нажмите на превью, чтобы выбрать его.",
            ));
            ui.add_enabled_ui(!self.is_busy(), |ui| {
                if ui
                    .button(lang.t("+ Add images / videos…", "+ Добавить фото / видео…"))
                    .on_hover_text("Ctrl+O")
                    .clicked()
                {
                    self.browse(BrowsePurpose::Media);
                }
                let chosen = self
                    .selected_media
                    .and_then(|i| self.media_files.get(i))
                    .cloned();
                let (pick, _) = photo_grid(
                    ui,
                    "target_library",
                    &self.media_files,
                    chosen.as_ref(),
                    &mut self.marked_media,
                    &mut self.thumbnails,
                    lang,
                    false,
                );
                if let Some(pick) = pick {
                    self.selected_media = self.media_files.iter().position(|p| *p == pick);
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .small_button(lang.t("Mark all", "Отметить все"))
                        .clicked()
                    {
                        self.marked_media.extend(self.media_files.clone());
                    }
                    if ui
                        .small_button(lang.t("Clear marks", "Снять отметки"))
                        .clicked()
                    {
                        self.marked_media.clear();
                    }
                    if ui
                        .add_enabled(
                            !self.marked_media.is_empty(),
                            egui::Button::new(format!(
                                "{} ({})",
                                lang.t("Delete", "Удалить"),
                                self.marked_media.len()
                            )),
                        )
                        .clicked()
                    {
                        self.pending_delete = self.marked_media.iter().cloned().collect();
                        self.pending_delete_base = self.base_dir.clone();
                    }
                });
            });
            if let Some(path) = self.selected_media.and_then(|i| self.media_files.get(i)) {
                ui.colored_label(
                    egui::Color32::from_rgb(110, 200, 250),
                    format!(
                        "{}: {}",
                        lang.t("Target", "Выбрано"),
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                );
            } else {
                ui.label(lang.t(
                    "Add a target to start.",
                    "Добавьте образец для начала работы.",
                ));
            }
            self.adaptation_section(ui);
        });
    }

    fn adaptation_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        let target = self.selected_media.and_then(|i| self.media_files.get(i)).cloned();
        let has_raw = self.shape_photos.iter().any(|p| p.preview.parent() == Some(self.shape_base().join("raw_shapes").as_path()));
        ui.separator();
        if ui.add_enabled(!self.is_busy() && target.is_some() && has_raw,
            egui::Button::new(lang.t("Optimize for selected sprite set", "Оптимизировать под выбранный набор"))).clicked() {
            self.adapt_target(target.clone().unwrap());
        }
        ui.small(lang.t(
            "Creates and selects a new target. Original stays in the library. Best suited to original-color sprites.",
            "Создаёт и выбирает новый образец. Исходник остаётся в библиотеке. Для спрайтов с исходными цветами.",
        ));
        if !has_raw {
            ui.small(lang.t("Add original sprites to this set first.", "Сначала добавьте исходные спрайты в этот набор."));
        }
        egui::CollapsingHeader::new(lang.t("Palette adaptation controls", "Параметры адаптации палитры"))
            .show(ui, |ui| {
                ui.add_enabled_ui(!self.is_busy(), |ui| {
                    let options = &mut self.settings.palette_adaptation;
                    slider_u32(ui,true,&mut options.colors,4..=64,lang.t("Palette colors (up to 256)", "Цветов в палитре (до 256)"));
                    slider_f32(ui,true,&mut options.strength,0.0..=1.0,lang.t("Adaptation strength", "Сила адаптации"));
                    slider_f32(ui,true,&mut options.smoothing,0.0..=1.0,lang.t("Edge-preserving smoothing", "Сглаживание с сохранением границ"));
                    slider_f32(ui,true,&mut options.contrast,0.0..=1.0,lang.t("Luma contrast / detail", "Контраст и детали по яркости"));
                    slider_f32(ui,true,&mut options.desaturation,0.0..=1.0,lang.t("Desaturation", "Приглушение насыщенности"));
                    toggle(ui,true,&mut options.mix_colors,lang.t("Mix palette colors (stable dithering)", "Смешивать цвета палитры (стабильный дизеринг)"));
                    slider_u32(ui,options.mix_colors,&mut options.dither_size,1..=16,
                        lang.t("Pattern cell (target px)", "Размер ячейки (px образца)"));
                });
                ui.small(lang.t(
                    "Palette and exposure stay fixed throughout the video. Resolution and FPS use generation settings. Large sprites cannot reproduce a fine pixel pattern; try larger cells or disable mixing.",
                    "Палитра и параметры яркости постоянны для всего видео. Размер и FPS — из настроек генерации. Крупные спрайты не повторят мелкую сетку: увеличьте ячейки или выключите смешивание.",
                ));
            });
        if let Some((source,adapted,palette)) = self.adaptation_preview.as_ref()
            .map(|p| (p.source.clone(),p.path.clone(),p.palette.clone())) {
            if target.as_ref() == Some(&adapted) || target.as_ref() == Some(&source) {
                ui.horizontal(|ui| {
                    for (path,label) in [(&source,lang.t("Original", "Исходник")),(&adapted,lang.t("Adapted", "Адаптировано"))] {
                        ui.vertical(|ui| {
                            ui.label(label);
                            if let Some(texture) = self.thumbnails.get(path) {
                                let size = texture.size_vec2();
                                ui.image((texture.id(),size * (120.0 / size.x).min(80.0 / size.y)));
                            }
                            if ui.add_enabled(!self.is_busy(),egui::Button::new(lang.t("Use", "Выбрать"))).clicked() {
                                self.selected_media = self.media_files.iter().position(|p| p == path);
                            }
                        });
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(lang.t("Set palette:", "Палитра набора:"));
                    for color in &palette {
                        let (rect,_) = ui.allocate_exact_size(egui::vec2(12.0,12.0),egui::Sense::hover());
                        ui.painter().rect_filled(rect,2.0,egui::Color32::from_rgb(color[0],color[1],color[2]));
                    }
                });
            }
        }
    }

    fn adapt_target(&mut self, source: PathBuf) {
        let base = self.base_dir.clone();
        let shape_folder = self.shape_base().join("raw_shapes");
        let settings = self.settings.clone();
        let layer_limit = self.shape_layer_limit;
        let progress = std::sync::Arc::new(crate::io::frame_set::Progress::default());
        self.adaptation_progress = Some(progress.clone());
        self.start_file_job(move || {
            match crate::io::palette_adaptation::create(&base,&source,&shape_folder,settings.shape_resolution,layer_limit,
                settings.max_texture_size,settings.target_fps,settings.preserve_audio,settings.luma_weight,
                &settings.palette_adaptation,&progress) {
                Ok(report) => JobResult {
                    result: Ok(report.message.clone()), select_media: Some(report.path.clone()),
                    select_shape_set: None, mob_view: None, adapted: Some(report),
                },
                Err(error) => JobResult { result:Err(error),select_media:None,select_shape_set:None,mob_view:None,adapted:None },
            }
        });
    }

    fn import_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.heading(lang.t("2. Building blocks", "2. Из каких картинок"));
            ui.label(lang.t("Photos used to build the target. Grayscale copies are prepared automatically.", "Фотографии, из которых строится образец. Чёрно-белые копии готовятся автоматически."));
            ui.add_enabled_ui(!self.is_busy(), |ui| {
                egui::ComboBox::from_id_salt("active_shape_set").selected_text(self.settings.shape_set.as_deref().unwrap_or(lang.t("Main photo library", "Основная библиотека"))).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.shape_set,None,lang.t("Main photo library", "Основная библиотека"));
                    for name in &self.shape_sets { ui.selectable_value(&mut self.settings.shape_set,Some(name.clone()),name); }
                });
                if self.displayed_shape_set != self.settings.shape_set { self.refresh_library(); }
                if ui.button(lang.t("Create set from video…", "Создать набор из видео…")).clicked() {
                    self.frame_form.open = true;
                    if let Some(path) = self.selected_media.and_then(|i|self.media_files.get(i)).filter(|p|media_loader::is_video_path(p)) { self.frame_form.video = Some(path.clone()); }
                }
                if ui.button(lang.t("+ Add photos…", "+ Добавить фотографии…")).on_hover_text("Ctrl+Shift+O").clicked() {
                    self.browse(BrowsePurpose::Shapes);
                }
                let paths: Vec<_> = self.shape_photos.iter().map(|p| p.preview.clone()).collect();
                photo_grid(ui, "shape_library", &paths, None, &mut self.marked_shapes, &mut self.thumbnails, lang, true);
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button(lang.t("Mark all", "Отметить все")).clicked() { self.marked_shapes.extend(paths); }
                    if ui.small_button(lang.t("Clear marks", "Снять отметки")).clicked() { self.marked_shapes.clear(); }
                    if ui.add_enabled(!self.marked_shapes.is_empty(), egui::Button::new(format!("{} ({})", lang.t("Delete", "Удалить"), self.marked_shapes.len()))).clicked() {
                        self.pending_delete = self.shape_photos.iter().filter(|p| self.marked_shapes.contains(&p.preview)).flat_map(|p| p.files()).collect();
                        self.pending_delete_base = self.shape_base();
                    }
                });
            });
            let ready = self.shape_photos.iter().filter(|p| if self.settings.use_original_colors {
                p.preview.parent() == Some(self.shape_base().join("raw_shapes").as_path())
            } else { p.prepared.is_some() }).count();
            ui.label(format!("{}: {ready} / {} · {}", lang.t("Available", "Доступно"), self.shape_photos.len(), if self.settings.use_original_colors {
                lang.t("original colors", "исходные цвета")
            } else { lang.t("grayscale shapes", "чёрно-белые фигуры") }));
            if ready < self.shape_photos.len() {
                ui.small(lang.t("Some files need conversion or are available in the other color mode.", "Часть файлов требует конвертации или доступна в другом цветовом режиме."));
            }
        });
    }

    fn conversion_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        egui::CollapsingHeader::new(lang.t("Folder conversion", "Конвертация папки")).show(ui, |ui| {
            ui.add_enabled_ui(!self.is_busy(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(lang.t("Source folder…", "Исходная папка…")).clicked() { self.browse(BrowsePurpose::Source); }
                    ui.label(self.conversion_source.display().to_string());
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button(lang.t("Output folder…", "Папка результата…")).clicked() { self.browse(BrowsePurpose::Destination); }
                    ui.label(self.conversion_destination.display().to_string());
                });
                ui.label(lang.t("Matching prepared PNG copies are updated. Use the default output folder to include them in generation.", "Готовые одноимённые PNG-копии обновляются. Для сборки используйте папку результата по умолчанию."));
                if ui.button(lang.t("Reset folders", "Папки по умолчанию")).clicked() {
                    self.conversion_source = self.shape_base().join("raw_shapes");
                    self.conversion_destination = self.shape_base().join("input_shapes");
                }
                if ui.button(lang.t("Convert folder to grayscale", "Конвертировать в чёрно-белые фигуры")).clicked() {
                    let source = self.conversion_source.clone();
                    let dest = self.conversion_destination.clone();
                    let resolution = self.settings.shape_resolution;
                    self.start_job(move || crate::io::shape_conversion::convert_folder(&source, &dest, resolution));
                }
            });
        });
    }

    fn start_job(&mut self, work: impl FnOnce() -> Result<String, String> + Send + 'static) {
        self.start_file_job(move || JobResult {
            result: work(),
            select_media: None,
            select_shape_set: None,
            mob_view: None,
            adapted: None,
        });
    }

    fn start_file_job(&mut self, work: impl FnOnce() -> JobResult + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.job = Some(rx);
        self.status = None;
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
    }

    fn import_paths(&mut self, paths: Vec<PathBuf>, shapes: bool) {
        let base = if shapes {
            self.shape_base()
        } else {
            self.base_dir.clone()
        };
        let resolution = self.settings.shape_resolution;
        self.start_file_job(move || {
            let report = library::import(&base, &paths, shapes, resolution);
            let message = report.message();
            JobResult {
                result: if report.errors.is_empty() {
                    Ok(message)
                } else {
                    Err(message)
                },
                select_media: report.first_media,
                select_shape_set: None,
                mob_view: None,
                adapted: None,
            }
        });
    }

    fn create_frame_set(&mut self, options: crate::io::frame_set::Options, use_target: bool) {
        let base = self.base_dir.clone();
        let progress = std::sync::Arc::new(crate::io::frame_set::Progress::default());
        self.frame_progress = Some(progress.clone());
        self.start_file_job(move || {
            match crate::io::frame_set::create(&base, &options, &progress) {
                Ok(name) => {
                    let count = library::scan(
                        &base.join("frame_sets").join(&name).join("raw_shapes"),
                        false,
                    )
                    .len();
                    let mut message = format!("Set created / Набор создан: {count}");
                    let mut target = None;
                    if use_target {
                        let report =
                            library::import(&base, &[options.video], false, options.resolution);
                        if !report.errors.is_empty() {
                            message.push_str(&format!(". {}", report.message()));
                        }
                        target = report.first_media;
                    }
                    JobResult {
                        result: Ok(message),
                        select_media: target,
                        select_shape_set: Some(name),
                        mob_view: None,
                        adapted: None,
                    }
                }
                Err(error) => JobResult {
                    result: Err(error),
                    select_media: None,
                    select_shape_set: None,
                    mob_view: None,
                    adapted: None,
                },
            }
        });
    }

    fn poll_import(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(outcome) => {
                    self.job = None;
                    self.frame_progress = None;
                    self.adaptation_progress = None;
                    if let Some(report) = outcome.adapted {
                        self.adaptation_preview = Some(report);
                    }
                    if let Some(view) = outcome.mob_view {
                        self.use_mob_settings();
                        self.settings.evolve_rotation = view == crate::io::mob_library::MobView::Top;
                    }
                    if let Some(set) = outcome.select_shape_set {
                        self.settings.shape_set = Some(set);
                    }
                    self.refresh_library();
                    if let Some(path) = outcome.select_media {
                        self.selected_media = self.media_files.iter().position(|p| *p == path);
                    }
                    match outcome.result {
                        Ok(m) => self.set_status(m, false),
                        Err(m) => self.set_status(m, true),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    self.frame_progress = None;
                    self.adaptation_progress = None;
                    self.refresh_library();
                    self.set_status("File operation failed / Ошибка обработки файлов", true);
                }
                Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        let selection = self
            .browser
            .as_ref()
            .and_then(|(purpose, rx)| match rx.try_recv() {
                Ok(paths) => Some((*purpose, paths)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                    *purpose,
                    Err("Windows dialog closed unexpectedly / Ошибка окна выбора".into()),
                )),
                Err(_) => None,
            });
        if let Some((purpose, result)) = selection {
            self.browser = None;
            match result {
                Ok(paths) if !paths.is_empty() => {
                    match purpose {
                        BrowsePurpose::Source => self.conversion_source = paths[0].clone(),
                        BrowsePurpose::Destination => {
                            self.conversion_destination = paths[0].clone()
                        }
                        BrowsePurpose::Media => self.import_paths(paths, false),
                        BrowsePurpose::Shapes => self.import_paths(paths, true),
                        BrowsePurpose::VideoFrames => {
                            self.frame_form.video = Some(paths[0].clone())
                        }
                        BrowsePurpose::MinecraftMobs => {
                            let base = self.base_dir.clone();
                            let folder = paths[0].clone();
                            let resolution = self.settings.shape_resolution;
                            self.start_file_job(move || match crate::io::mob_library::import_set(&base,&folder,resolution) {
                            Ok(imported) => JobResult {result:Ok("Mob set imported / Набор мобов добавлен".into()),select_media:None,select_shape_set:Some(imported.name),mob_view:Some(imported.view),adapted:None},
                            Err(error) => JobResult {result:Err(error),select_media:None,select_shape_set:None,mob_view:None,adapted:None},
                        });
                        }
                    }
                }
                Err(e) => self.set_status(e, true),
                _ => {}
            }
        }
    }

    fn file_modals(&mut self, ctx: &egui::Context) {
        let lang = self.language;
        let dropped: Vec<_> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for path in dropped {
            if !self.dropped_files.contains(&path) {
                self.dropped_files.push(path);
            }
        }
        if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
            egui::Area::new(egui::Id::new("drop_hint"))
                .order(egui::Order::Tooltip)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(24.0)
                        .show(ui, |ui| {
                            ui.heading(
                                lang.t("Drop files to add them", "Отпустите файлы, чтобы добавить"),
                            );
                            ui.label(lang.t(
                                "Next, choose target or building blocks.",
                                "Затем выберите: образец или картинки для сборки.",
                            ));
                        });
                });
        }
        if !self.dropped_files.is_empty() && !self.is_busy() {
            egui::Modal::new(egui::Id::new("drop_destination")).show(ctx, |ui| {
                ui.heading(lang.t("Where to add the files?", "Куда добавить файлы?"));
                ui.label(format!(
                    "{}: {}",
                    lang.t("Files", "Файлов"),
                    self.dropped_files.len()
                ));
                ui.label(lang.t(
                    "The app copies files; external originals stay in place.",
                    "Приложение скопирует файлы; исходные фотографии останутся на месте.",
                ));
                if ui
                    .button(lang.t(
                        "1. Target images / videos",
                        "1. Что собираем — фото / видео",
                    ))
                    .clicked()
                {
                    let paths = std::mem::take(&mut self.dropped_files);
                    self.import_paths(paths, false);
                }
                if ui
                    .button(lang.t(
                        "2. Building block photos",
                        "2. Из каких картинок — фотографии",
                    ))
                    .clicked()
                {
                    let paths = std::mem::take(&mut self.dropped_files);
                    self.import_paths(paths, true);
                }
                if ui.button(lang.t("Cancel", "Отмена")).clicked() {
                    self.dropped_files.clear();
                }
            });
        } else if !self.pending_delete.is_empty() && !self.is_busy() {
            egui::Modal::new(egui::Id::new("delete_photos")).show(ctx, |ui| {
                ui.heading(lang.t("Remove marked photos?", "Удалить отмеченные фотографии?"));
                ui.label(lang.t("Library files and their prepared copies will be moved to the app trash. External originals stay in place. Use Undo deletion to restore them, even after restarting.", "Файлы библиотеки и их готовые копии переместятся в корзину приложения. Внешние оригиналы останутся на месте. Кнопка «Отменить удаление» восстановит файлы, даже после перезапуска."));
                ui.horizontal(|ui| {
                    if ui.button(lang.t("Delete", "Удалить")).clicked() {
                        let paths = std::mem::take(&mut self.pending_delete);
                        let base = self.pending_delete_base.clone();
                        self.start_job(move || library::delete(&base, &paths).map(|()| "Removed from library. Undo deletion is available. / Удалено из библиотеки. Можно отменить удаление.".into()));
                    }
                    if ui.button(lang.t("Cancel", "Отмена")).clicked() { self.pending_delete.clear(); }
                });
            });
        }
    }

    fn gpu_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.strong(format!(
            "{}: {}",
            lang.t("Active GPU", "Используемая видеокарта"),
            self.active_gpu
        ));
        egui::ComboBox::from_id_salt("gpu_preference")
            .selected_text(self.settings.gpu_adapter.as_deref().unwrap_or(lang.t(
                "Auto: prefer discrete GPU",
                "Авто: приоритет дискретной GPU",
            )))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.settings.gpu_adapter,
                    None,
                    lang.t(
                        "Auto: prefer discrete GPU",
                        "Авто: приоритет дискретной GPU",
                    ),
                );
                for name in &self.available_gpus {
                    ui.selectable_value(&mut self.settings.gpu_adapter, Some(name.clone()), name);
                }
            });
        ui.label(lang.t(
            "GPU selection takes effect after saving settings and restarting the app.",
            "Выбор GPU применяется после сохранения настроек и перезапуска приложения.",
        ));
        if self
            .settings
            .gpu_adapter
            .as_ref()
            .map(|name| !self.available_gpus.contains(name))
            .unwrap_or(false)
        {
            ui.colored_label(
                egui::Color32::from_rgb(255, 160, 45),
                lang.t(
                    "Saved GPU is unavailable; using the active GPU shown above.",
                    "Сохранённая GPU недоступна; используется видеокарта, указанная выше.",
                ),
            );
        }
    }

    fn presets_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.strong(lang.t("Presets", "Пресеты"));

        ui.horizontal(|ui| {
            let selected_label = self
                .selected_preset
                .and_then(|i| self.presets.get(i))
                .cloned()
                .unwrap_or_else(|| lang.t("— none —", "— нет —").to_string());

            egui::ComboBox::from_id_salt("preset_picker")
                .selected_text(selected_label)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (i, name) in self.presets.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_preset, Some(i), name.clone());
                    }
                });

            let has_selection = self.selected_preset.is_some();
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(lang.t("Load", "Загрузить")),
                )
                .clicked()
            {
                if let Some(name) = self
                    .selected_preset
                    .and_then(|i| self.presets.get(i))
                    .cloned()
                {
                    match settings::load_preset(&self.presets_dir, &name) {
                        Ok(s) => {
                            self.settings = s;
                            self.set_status(
                                format!(
                                    "{} '{}'",
                                    lang.t("Loaded preset", "Загружен пресет"),
                                    name
                                ),
                                false,
                            );
                        }
                        Err(e) => self.set_status(
                            format!("{}: {}", lang.t("Load failed", "Ошибка загрузки"), e),
                            true,
                        ),
                    }
                }
            }

            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(lang.t("Delete", "Удалить")),
                )
                .clicked()
            {
                if let Some(name) = self
                    .selected_preset
                    .and_then(|i| self.presets.get(i))
                    .cloned()
                {
                    match settings::delete_preset(&self.presets_dir, &name) {
                        Ok(_) => {
                            self.selected_preset = None;
                            self.refresh_presets();
                            self.set_status(
                                format!("{} '{}'", lang.t("Deleted preset", "Удалён пресет"), name),
                                false,
                            );
                        }
                        Err(e) => self.set_status(
                            format!("{}: {}", lang.t("Delete failed", "Ошибка удаления"), e),
                            true,
                        ),
                    }
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label(lang.t("New / overwrite:", "Создать / перезаписать:"));
            ui.add(
                egui::TextEdit::singleline(&mut self.new_preset_name)
                    .desired_width(220.0)
                    .hint_text(lang.t("preset name", "имя пресета")),
            );
            if ui
                .button(lang.t("Save as preset", "Сохранить как пресет"))
                .clicked()
            {
                let name = settings::sanitize_preset_name(&self.new_preset_name);
                if name.is_empty() {
                    self.set_status(lang.t("Enter a preset name", "Введите имя пресета"), true);
                } else {
                    match settings::save_preset(&self.presets_dir, &name, &self.settings) {
                        Ok(_) => {
                            self.new_preset_name.clear();
                            self.refresh_presets();
                            self.selected_preset = self.presets.iter().position(|n| *n == name);
                            self.set_status(
                                format!("{} '{}'", lang.t("Saved preset", "Сохранён пресет"), name),
                                false,
                            );
                        }
                        Err(e) => self.set_status(
                            format!("{}: {}", lang.t("Save failed", "Ошибка сохранения"), e),
                            true,
                        ),
                    }
                }
            }
        });
    }

    fn parameters(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        let is_video = self.selected_is_video();

        egui::CollapsingHeader::new(lang.t("General", "Основное"))
            .default_open(true)
            .show(ui, |ui| {
                toggle(ui,true,&mut self.settings.perceptual_scoring,
                    lang.t("Prioritize brightness (BT.709 luma / color)", "Приоритет яркости (BT.709: яркость / цвет)"));
                slider_f32(ui,self.settings.perceptual_scoring,&mut self.settings.luma_weight,1.0..=12.0,
                    lang.t("Luma weight (chroma: 1 + 1)", "Вес яркости (цвет: 1 + 1)"));
                ui.small(lang.t(
                    "Applies to ALL image / video modes. Default: 6:1:1. Off uses RGB. This is a tunable preference, not a complete human-vision model.",
                    "Для ВСЕХ режимов фото / видео. По умолчанию 6:1:1. Без галочки — RGB. Это настраиваемый приоритет, а не полная модель зрения.",
                ));
                slider_u32(
                    ui,
                    true,
                    &mut self.settings.batch_size,
                    1..=4096,
                    lang.t(
                        "Batch size (initial population)",
                        "Размер пакета (нач. популяция)",
                    ),
                );
                slider_u32(
                    ui,
                    true,
                    &mut self.settings.max_shapes,
                    1..=1_000_000,
                    lang.t("Max shapes", "Макс. фигур"),
                );
                slider_u32(
                    ui,
                    true,
                    &mut self.settings.mutations_per_frame,
                    1..=100,
                    lang.t("Placements per frame", "Размещений за кадр"),
                );
                slider_u32(
                    ui,
                    true,
                    &mut self.settings.max_texture_size,
                    16..=2048,
                    lang.t("Max texture size (px)", "Макс. размер текстуры (px)"),
                );
                slider_u32(
                    ui,
                    true,
                    &mut self.settings.vram_budget_mb,
                    128..=4096,
                    lang.t("VRAM budget (MB)", "Бюджет VRAM (МБ)"),
                );
            });

        egui::CollapsingHeader::new(lang.t("Shapes & scale", "Фигуры и масштаб"))
            .default_open(true)
            .show(ui, |ui| {
                slider_f32(
                    ui,
                    true,
                    &mut self.settings.scale_min,
                    0.01..=1.0,
                    lang.t("Min scale", "Мин. масштаб"),
                );
                slider_f32(
                    ui,
                    true,
                    &mut self.settings.scale_max,
                    0.1..=20.0,
                    lang.t("Max scale", "Макс. масштаб"),
                );
                slider_u32(
                    ui,
                    true,
                    &mut self.settings.shape_resolution,
                    16..=1024,
                    lang.t(
                        "Shape resolution (needs prepared shapes)",
                        "Разрешение фигур (нужны подготовленные)",
                    ),
                );
                toggle(
                    ui,
                    true,
                    &mut self.settings.evolve_rotation,
                    lang.t("Evolve rotation", "Поворот фигур"),
                );
                toggle(
                    ui,
                    true,
                    &mut self.settings.evolve_opacity,
                    lang.t("Evolve opacity", "Эволюция прозрачности"),
                );
                toggle(
                    ui,
                    true,
                    &mut self.settings.use_original_colors,
                    lang.t(
                        "Use original shape colors (raw_shapes/)",
                        "Оригинальные цвета фигур (raw_shapes/)",
                    ),
                );
                toggle(
                    ui,
                    true,
                    &mut self.settings.evolve_non_uniform_scale,
                    lang.t(
                        "Non-uniform (per-axis) scale",
                        "Неравномерный масштаб по осям",
                    ),
                );

                // Hue/saturation evolution only makes sense in real-color mode.
                let real_color = self.settings.use_original_colors;
                if !real_color {
                    ui.label(egui::RichText::new(lang.t(
                        "Hue / saturation evolution applies only in original-color mode.",
                        "Эволюция оттенка/насыщенности работает только в режиме реального цвета.",
                    )).weak().italics());
                }
                toggle(
                    ui,
                    real_color,
                    &mut self.settings.evolve_hue,
                    lang.t(
                        "Evolve hue (real-color mode)",
                        "Эволюция оттенка (реальный цвет)",
                    ),
                );
                toggle(
                    ui,
                    real_color,
                    &mut self.settings.evolve_saturation,
                    lang.t(
                        "Evolve saturation (real-color mode)",
                        "Эволюция насыщенности (реальный цвет)",
                    ),
                );
                toggle(
                    ui,
                    real_color,
                    &mut self.settings.evolve_brightness,
                    lang.t(
                        "Evolve brightness (real-color mode)",
                        "Эволюция яркости (реальный цвет)",
                    ),
                );
            });

        egui::CollapsingHeader::new(lang.t("Evolution algorithm", "Алгоритм эволюции"))
            .default_open(false)
            .show(ui, |ui| {
                slider_u32(
                    ui,
                    true,
                    &mut self.settings.num_generations,
                    1..=20,
                    lang.t("Generations per placement", "Поколений на размещение"),
                );
                slider_f32(
                    ui,
                    true,
                    &mut self.settings.survival_rate,
                    0.01..=1.0,
                    lang.t("Survival rate", "Доля выживания"),
                );
                slider_u32(
                    ui,
                    true,
                    &mut self.settings.children_per_parent,
                    1..=50,
                    lang.t("Children per parent", "Детей на родителя"),
                );
                slider_u32(
                    ui,
                    true,
                    &mut self.settings.max_rejections,
                    1..=500,
                    lang.t("Max consecutive rejections", "Макс. отказов подряд"),
                );

                // use_min_improvement conflicts with diversity_mode.
                let umi_enabled = !self.settings.diversity_mode;
                toggle(
                    ui,
                    umi_enabled,
                    &mut self.settings.use_min_improvement,
                    lang.t("Use min-improvement threshold", "Порог мин. улучшения"),
                );
                if self.settings.diversity_mode {
                    ui.label(
                        egui::RichText::new(lang.t(
                            "(disabled — conflicts with diversity mode)",
                            "(выкл. — конфликтует с режимом разнообразия)",
                        ))
                        .weak()
                        .italics(),
                    );
                }
                let mi_enabled = self.settings.use_min_improvement && !self.settings.diversity_mode;
                slider_f32(
                    ui,
                    mi_enabled,
                    &mut self.settings.min_improvement,
                    -50.0..=0.0,
                    lang.t("Min improvement", "Мин. улучшение"),
                );
            });

        egui::CollapsingHeader::new(lang.t("Shape diversity", "Разнообразие фигур"))
            .default_open(false)
            .show(ui, |ui| {
                toggle(
                    ui,
                    true,
                    &mut self.settings.diversity_mode,
                    lang.t("Diversity mode", "Режим разнообразия"),
                );
                let div = self.settings.diversity_mode;
                slider_f32(
                    ui,
                    div,
                    &mut self.settings.diversity_penalty_increment,
                    0.0..=10.0,
                    lang.t("Penalty increment per use", "Штраф за использование"),
                );
                toggle(
                    ui,
                    div,
                    &mut self.settings.diversity_decay_enabled,
                    lang.t("Decay other penalties", "Затухание штрафов остальных"),
                );
                let decay = div && self.settings.diversity_decay_enabled;
                slider_f32(
                    ui,
                    decay,
                    &mut self.settings.diversity_decay_amount,
                    0.0..=10.0,
                    lang.t("Decay amount", "Величина затухания"),
                );
            });

        egui::CollapsingHeader::new(lang.t("Minecraft / scene export", "Minecraft / экспорт раскладки"))
            .default_open(false)
            .show(ui, |ui| {
                if ui.add_enabled(self.job.is_none() && self.browser.is_none(), egui::Button::new(lang.t("Import mobs from Minecraft…", "Импортировать мобов из Minecraft…"))).clicked() { self.browse(BrowsePurpose::MinecraftMobs); }
                toggle(ui, true, &mut self.settings.export_scene,
                    lang.t("Export shapes and every output frame (JSON)", "Сохранять фигуры и каждый кадр (JSON)"));
                ui.label(lang.t(
                    "Saved in output/: source brushes, stable IDs, position, size, rotation and layer order. Works with images and videos.",
                    "В output/: исходные фигуры, постоянные ID, положение, размер, поворот и слои. Для картинок и видео."));
                if ui.button(lang.t("Use settings for real mobs", "Настроить для настоящих мобов")).clicked() {
                    self.use_mob_settings();
                }
                ui.label(egui::RichText::new(lang.t(
                    "Fabric mod: export mobs from above or facing forward in the pause menu, import the set here, then open the resulting scene.json in the mod. The view is detected automatically.",
                    "Мод Fabric: в меню паузы экспортируй мобов сверху или лицом вперёд, импортируй набор здесь, затем открой готовый scene.json в моде. Ракурс определится автоматически." )).weak());
            });

        egui::CollapsingHeader::new(
            lang.t("Progress GIF (image mode)", "GIF процесса (для картинок)"),
        )
        .default_open(false)
        .show(ui, |ui| {
            if is_video {
                ui.label(
                    egui::RichText::new(lang.t(
                        "Selected input is a video — the progress GIF applies to images only.",
                        "Выбрано видео — GIF процесса работает только для изображений.",
                    ))
                    .weak()
                    .italics(),
                );
            }
            let gif_on_enabled = !is_video;
            toggle(
                ui,
                gif_on_enabled,
                &mut self.settings.save_progress_gif,
                lang.t(
                    "Save creation GIF next to the image",
                    "Сохранять GIF создания рядом с картинкой",
                ),
            );
            let gif_params = gif_on_enabled && self.settings.save_progress_gif;
            slider_u32(
                ui,
                gif_params,
                &mut self.settings.gif_frames,
                2..=2000,
                lang.t("Captured frames (approx.)", "Кадров (примерно)"),
            );
            slider_u32(
                ui,
                gif_params,
                &mut self.settings.gif_fps,
                1..=50,
                lang.t("GIF playback FPS", "FPS воспроизведения GIF"),
            );
            slider_u32(
                ui,
                gif_params,
                &mut self.settings.gif_max_width,
                16..=2048,
                lang.t("Max GIF width (px)", "Макс. ширина GIF (px)"),
            );
        });

        egui::CollapsingHeader::new(
            lang.t("Video (used for video input)", "Видео (для видео-файлов)"),
        )
        .default_open(is_video)
        .show(ui, |ui| {
            if !is_video {
                ui.label(
                    egui::RichText::new(lang.t(
                        "Selected input is an image — video options are inactive.",
                        "Выбрано изображение — параметры видео неактивны.",
                    ))
                    .weak()
                    .italics(),
                );
            }
            slider_u32(
                ui,
                is_video,
                &mut self.settings.target_fps,
                1..=60,
                lang.t("Target FPS", "Целевой FPS"),
            );
            slider_u32(
                ui,
                is_video,
                &mut self.settings.interpolation_steps,
                0..=20,
                lang.t("Interpolation steps", "Шаги интерполяции"),
            );
            slider_u32(
                ui,
                is_video,
                &mut self.settings.mutations_per_shape,
                1..=50,
                lang.t("Mutations per shape", "Мутаций на фигуру"),
            );
            slider_f32(
                ui,
                is_video,
                &mut self.settings.displacement_weight,
                0.0..=100.0,
                lang.t("Displacement weight", "Вес смещения"),
            );
            slider_f32(
                ui,
                is_video,
                &mut self.settings.scene_change_tolerance,
                -10.0..=10.0,
                lang.t("Scene-change tolerance", "Порог смены сцены"),
            );
            toggle(
                ui,
                is_video,
                &mut self.settings.preserve_audio,
                lang.t("Keep original audio", "Сохранять исходный звук"),
            );

            let recolor_enabled = is_video && !self.settings.use_original_colors;
            toggle(
                ui,
                recolor_enabled,
                &mut self.settings.video_recolor,
                lang.t(
                    "Recolor shapes to new frame",
                    "Перекрашивать под новый кадр",
                ),
            );
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) -> ScreenAction {
        let lang = self.language;
        let mut action = ScreenAction::None;

        ui.horizontal(|ui| {
            let start = ui.add_sized(
                [160.0, 36.0],
                egui::Button::new(egui::RichText::new(lang.t("▶  Start", "▶  Пуск")).size(18.0)),
            );

            if ui
                .button(lang.t("Save settings.toml", "Сохранить settings.toml"))
                .clicked()
            {
                self.save_settings_file();
            }

            if start.clicked() {
                action = self.try_start();
            }
        });

        if let Some((msg, is_error)) = &self.status {
            let color = if *is_error {
                egui::Color32::from_rgb(255, 90, 90)
            } else {
                egui::Color32::from_rgb(120, 220, 120)
            };
            ui.colored_label(color, msg);
        }

        action
    }

    fn save_settings_file(&mut self) {
        let lang = self.language;
        match self.settings.validate() {
            Ok(_) => match self.settings.save(&self.settings_path) {
                Ok(_) => self.set_status(
                    lang.t("Saved settings.toml", "Сохранено в settings.toml"),
                    false,
                ),
                Err(e) => self.set_status(
                    format!("{}: {}", lang.t("Save failed", "Ошибка сохранения"), e),
                    true,
                ),
            },
            Err(e) => self.set_status(
                format!(
                    "{}: {}",
                    lang.t("Invalid settings", "Неверные настройки"),
                    e
                ),
                true,
            ),
        }
    }

    fn try_start(&mut self) -> ScreenAction {
        let lang = self.language;

        let media_path = match self
            .selected_media
            .and_then(|i| self.media_files.get(i))
            .cloned()
        {
            Some(p) => p,
            None => {
                self.set_status(
                    lang.t(
                        "Select an input file first",
                        "Сначала выберите входной файл",
                    ),
                    true,
                );
                return ScreenAction::None;
            }
        };

        if self.is_busy() || !self.pending_delete.is_empty() || !self.dropped_files.is_empty() {
            self.set_status(
                lang.t("Wait for file processing", "Дождитесь обработки файлов"),
                true,
            );
            return ScreenAction::None;
        }
        if let Err(e) = self.settings.validate() {
            self.set_status(
                format!(
                    "{}: {}",
                    lang.t("Invalid settings", "Неверные настройки"),
                    e
                ),
                true,
            );
            return ScreenAction::None;
        }

        // Persist the form to settings.toml so the next launch reflects it.
        if let Err(e) = self.settings.save(&self.settings_path) {
            log::warn!("Failed to persist settings.toml on start: {}", e);
        }

        ScreenAction::Start {
            media_path,
            settings: self.settings.clone(),
            language: self.language,
        }
    }
}

/// Only visible rows request thumbnails; even very large folders stay scrollable.
fn photo_grid(
    ui: &mut egui::Ui,
    id: &str,
    paths: &[PathBuf],
    chosen: Option<&PathBuf>,
    marked: &mut HashSet<PathBuf>,
    thumbnails: &mut Thumbnails,
    lang: Language,
    mark_on_click: bool,
) -> (Option<PathBuf>, usize) {
    let mut picked = None;
    let columns = ((ui.available_width() / 126.0) as usize).max(1);
    let rows = paths.len().div_ceil(columns);
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(264.0)
        .min_scrolled_height(144.0)
        .auto_shrink([false, false])
        .show_rows(ui, 128.0, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    for path in paths.iter().skip(row * columns).take(columns) {
                        ui.push_id(path, |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(116.0);
                                let selected =
                                    chosen == Some(path) || mark_on_click && marked.contains(path);
                                let button = if let Some(texture) = thumbnails.get(path) {
                                    egui::Button::image(
                                        egui::Image::new(&texture).fit_to_exact_size(
                                            texture.size_vec2()
                                                * (108.0 / texture.size_vec2().x)
                                                    .min(76.0 / texture.size_vec2().y),
                                        ),
                                    )
                                } else {
                                    egui::Button::new(if media_loader::is_video_path(path) {
                                        lang.t("Video", "Видео")
                                    } else {
                                        lang.t("Preview…", "Превью…")
                                    })
                                }
                                .selected(selected);
                                if ui.add_sized([116.0, 80.0], button).clicked() {
                                    picked = Some(path.clone());
                                    if mark_on_click && !marked.remove(path) {
                                        marked.insert(path.clone());
                                    }
                                }
                                let name = path.file_name().unwrap_or_default().to_string_lossy();
                                ui.add(egui::Label::new(name.as_ref()).truncate())
                                    .on_hover_text(path.display().to_string());
                                let mut checked = marked.contains(path);
                                if ui
                                    .checkbox(&mut checked, lang.t("Mark", "Отметить"))
                                    .changed()
                                {
                                    if checked {
                                        marked.insert(path.clone());
                                    } else {
                                        marked.remove(path);
                                    }
                                }
                            });
                        });
                    }
                });
            }
        });
    if paths.is_empty() {
        ui.weak(lang.t("No files yet", "Пока нет файлов"));
    }
    (picked, rows)
}

/// A u32 slider that supports typing an exact number, greyed out when disabled.
fn slider_u32(
    ui: &mut egui::Ui,
    enabled: bool,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
    label: &str,
) {
    ui.add_enabled(
        enabled,
        egui::Slider::new(&mut *value, range.clone())
            .text(label)
            .clamping(egui::SliderClamping::Never),
    );
    unstable_warning(ui, enabled && !range.contains(value));
}

/// An f32 slider that supports typing an exact number, greyed out when disabled.
fn slider_f32(
    ui: &mut egui::Ui,
    enabled: bool,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    label: &str,
) {
    ui.add_enabled(
        enabled,
        egui::Slider::new(&mut *value, range.clone())
            .text(label)
            .max_decimals(4)
            .clamping(egui::SliderClamping::Never),
    );
    unstable_warning(
        ui,
        enabled && (!value.is_finite() || !range.contains(value)),
    );
}

fn unstable_warning(ui: &mut egui::Ui, show: bool) {
    let lang = ui
        .ctx()
        .data(|data| data.get_temp::<Language>(egui::Id::new("ui_language")))
        .unwrap_or_default();
    if show {
        ui.colored_label(
            egui::Color32::from_rgb(255, 160, 45),
            lang.t(
                "⚠ Unstable value: may cause problems",
                "⚠ Нестабильное значение: возможны проблемы",
            ),
        );
    }
}

/// A labelled checkbox, greyed out when disabled.
fn toggle(ui: &mut egui::Ui, enabled: bool, value: &mut bool, label: &str) {
    ui.add_enabled(enabled, egui::Checkbox::new(value, label));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropped_files_wait_for_destination_then_select_the_imported_target() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("app");
        std::fs::create_dir(&base).unwrap();
        let source = temp.path().join("новый образец.png");
        image::RgbImage::new(3, 3).save(&source).unwrap();
        let mut screen = SettingsScreen::new(&base, Settings::default(), Language::Russian);
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                dropped_files: vec![egui::DroppedFile {
                    path: Some(source.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            |ctx| {
                screen.render(ctx);
            },
        );
        assert_eq!(screen.dropped_files, vec![source.clone()]);
        assert!(screen.job.is_none());
        assert!(screen.media_files.is_empty());
        let paths = std::mem::take(&mut screen.dropped_files);
        screen.import_paths(paths, false);
        let deadline = std::time::Instant::now();
        while screen.job.is_some() {
            screen.poll_import(&ctx);
            assert!(deadline.elapsed().as_secs() < 5);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(screen.media_files.len(), 1);
        assert_eq!(screen.selected_media, Some(0));
        assert!(screen.shape_photos.is_empty());
        assert!(source.exists());
        assert!(
            matches!(screen.try_start(), ScreenAction::Start { media_path, .. } if media_path == base.join("input_media/новый образец.png"))
        );
    }

    #[test]
    fn rendering_keeps_manual_values_and_draws_orange_warning() {
        let ctx = egui::Context::default();
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("ui_language"), Language::Russian));
        let mut value = 5_000_000u32;
        let mut decimal = 25.0;
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                slider_u32(ui, true, &mut value, 1..=20, "Generations");
                slider_f32(ui, true, &mut decimal, 0.1..=20.0, "Scale");
            });
        });
        assert_eq!(value, 5_000_000);
        assert_eq!(decimal, 25.0);
        assert!(output.shapes.iter().any(|s| match &s.shape {
            egui::Shape::Text(t) => t.galley.text().contains("Нестабильное значение")
                && t.galley
                    .job
                    .sections
                    .iter()
                    .any(|section| section.format.color == egui::Color32::from_rgb(255, 160, 45)),
            _ => false,
        }));
    }
}
