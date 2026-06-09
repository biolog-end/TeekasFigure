// The "Settings" screen: an egui form for configuring the approximator before
// launching generation. Features:
//   * media file picker (lists input_media/)
//   * smart sliders + toggles (conflicting/irrelevant options grey out)
//   * named presets (create / load / save / delete)
//   * English / Russian UI
//   * persists to settings.toml on Start (and on explicit Save)

use std::path::{Path, PathBuf};

use crate::io::media_loader;
use crate::settings::{self, Settings};

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
}

impl SettingsScreen {
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
        };
        screen.refresh_media();
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
        self.enforce_smart_rules();
        let mut action = ScreenAction::None;

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.header(ui);
                ui.separator();
                self.media_section(ui);
                ui.separator();
                self.presets_section(ui);
                ui.separator();
                self.parameters(ui);
                ui.separator();
                action = self.footer(ui);
            });
        });

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

    fn media_section(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.horizontal(|ui| {
            ui.strong(lang.t("Input file (input_media/):", "Входной файл (input_media/):"));
            if ui.button(lang.t("Refresh", "Обновить")).clicked() {
                self.refresh_media();
            }
        });

        if self.media_files.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 60),
                lang.t(
                    "No media found. Put an image (PNG/JPG/WebP/...) or video (MP4/MOV/MKV/...) into input_media/ and press Refresh.",
                    "Файлы не найдены. Положите изображение (PNG/JPG/WebP/…) или видео (MP4/MOV/MKV/…) в input_media/ и нажмите «Обновить».",
                ),
            );
            return;
        }

        let selected_label = self
            .selected_media
            .and_then(|i| self.media_files.get(i))
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| lang.t("— choose —", "— выбрать —").to_string());

        egui::ComboBox::from_id_salt("media_picker")
            .selected_text(selected_label)
            .width(360.0)
            .show_ui(ui, |ui| {
                for (i, path) in self.media_files.iter().enumerate() {
                    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    ui.selectable_value(&mut self.selected_media, Some(i), name);
                }
            });
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
                .add_enabled(has_selection, egui::Button::new(lang.t("Load", "Загрузить")))
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
                                format!("{} '{}'", lang.t("Loaded preset", "Загружен пресет"), name),
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
                .add_enabled(has_selection, egui::Button::new(lang.t("Delete", "Удалить")))
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
            if ui.button(lang.t("Save as preset", "Сохранить как пресет")).clicked() {
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
                slider_u32(ui, true, &mut self.settings.batch_size, 1..=4096, lang.t("Batch size (initial population)", "Размер пакета (нач. популяция)"));
                slider_u32(ui, true, &mut self.settings.max_shapes, 1..=1_000_000, lang.t("Max shapes", "Макс. фигур"));
                slider_u32(ui, true, &mut self.settings.mutations_per_frame, 1..=100, lang.t("Placements per frame", "Размещений за кадр"));
                slider_u32(ui, true, &mut self.settings.max_texture_size, 16..=2048, lang.t("Max texture size (px)", "Макс. размер текстуры (px)"));
                slider_u32(ui, true, &mut self.settings.vram_budget_mb, 128..=4096, lang.t("VRAM budget (MB)", "Бюджет VRAM (МБ)"));
            });

        egui::CollapsingHeader::new(lang.t("Shapes & scale", "Фигуры и масштаб"))
            .default_open(true)
            .show(ui, |ui| {
                slider_f32(ui, true, &mut self.settings.scale_min, 0.01..=1.0, lang.t("Min scale", "Мин. масштаб"));
                slider_f32(ui, true, &mut self.settings.scale_max, 0.1..=20.0, lang.t("Max scale", "Макс. масштаб"));
                slider_u32(ui, true, &mut self.settings.shape_resolution, 16..=1024, lang.t("Shape resolution (needs prepared shapes)", "Разрешение фигур (нужны подготовленные)"));
                toggle(ui, true, &mut self.settings.evolve_opacity, lang.t("Evolve opacity", "Эволюция прозрачности"));
                toggle(ui, true, &mut self.settings.use_original_colors, lang.t("Use original shape colors (raw_shapes/)", "Оригинальные цвета фигур (raw_shapes/)"));
                toggle(ui, true, &mut self.settings.evolve_non_uniform_scale, lang.t("Non-uniform (per-axis) scale", "Неравномерный масштаб по осям"));

                // Hue/saturation evolution only makes sense in real-color mode.
                let real_color = self.settings.use_original_colors;
                if !real_color {
                    ui.label(egui::RichText::new(lang.t(
                        "Hue / saturation evolution applies only in original-color mode.",
                        "Эволюция оттенка/насыщенности работает только в режиме реального цвета.",
                    )).weak().italics());
                }
                toggle(ui, real_color, &mut self.settings.evolve_hue, lang.t("Evolve hue (real-color mode)", "Эволюция оттенка (реальный цвет)"));
                toggle(ui, real_color, &mut self.settings.evolve_saturation, lang.t("Evolve saturation (real-color mode)", "Эволюция насыщенности (реальный цвет)"));
                toggle(ui, real_color, &mut self.settings.evolve_brightness, lang.t("Evolve brightness (real-color mode)", "Эволюция яркости (реальный цвет)"));
            });

        egui::CollapsingHeader::new(lang.t("Evolution algorithm", "Алгоритм эволюции"))
            .default_open(false)
            .show(ui, |ui| {
                slider_u32(ui, true, &mut self.settings.num_generations, 1..=20, lang.t("Generations per placement", "Поколений на размещение"));
                slider_f32(ui, true, &mut self.settings.survival_rate, 0.01..=1.0, lang.t("Survival rate", "Доля выживания"));
                slider_u32(ui, true, &mut self.settings.children_per_parent, 1..=50, lang.t("Children per parent", "Детей на родителя"));
                slider_u32(ui, true, &mut self.settings.max_rejections, 1..=500, lang.t("Max consecutive rejections", "Макс. отказов подряд"));

                // use_min_improvement conflicts with diversity_mode.
                let umi_enabled = !self.settings.diversity_mode;
                toggle(ui, umi_enabled, &mut self.settings.use_min_improvement, lang.t("Use min-improvement threshold", "Порог мин. улучшения"));
                if self.settings.diversity_mode {
                    ui.label(egui::RichText::new(lang.t(
                        "(disabled — conflicts with diversity mode)",
                        "(выкл. — конфликтует с режимом разнообразия)",
                    )).weak().italics());
                }
                let mi_enabled = self.settings.use_min_improvement && !self.settings.diversity_mode;
                slider_f32(ui, mi_enabled, &mut self.settings.min_improvement, -50.0..=0.0, lang.t("Min improvement", "Мин. улучшение"));
            });

        egui::CollapsingHeader::new(lang.t("Shape diversity", "Разнообразие фигур"))
            .default_open(false)
            .show(ui, |ui| {
                toggle(ui, true, &mut self.settings.diversity_mode, lang.t("Diversity mode", "Режим разнообразия"));
                let div = self.settings.diversity_mode;
                slider_f32(ui, div, &mut self.settings.diversity_penalty_increment, 0.0..=10.0, lang.t("Penalty increment per use", "Штраф за использование"));
                toggle(ui, div, &mut self.settings.diversity_decay_enabled, lang.t("Decay other penalties", "Затухание штрафов остальных"));
                let decay = div && self.settings.diversity_decay_enabled;
                slider_f32(ui, decay, &mut self.settings.diversity_decay_amount, 0.0..=10.0, lang.t("Decay amount", "Величина затухания"));
            });

        egui::CollapsingHeader::new(lang.t("Progress GIF (image mode)", "GIF процесса (для картинок)"))
            .default_open(false)
            .show(ui, |ui| {
                if is_video {
                    ui.label(egui::RichText::new(lang.t(
                        "Selected input is a video — the progress GIF applies to images only.",
                        "Выбрано видео — GIF процесса работает только для изображений.",
                    )).weak().italics());
                }
                let gif_on_enabled = !is_video;
                toggle(ui, gif_on_enabled, &mut self.settings.save_progress_gif, lang.t("Save creation GIF next to the image", "Сохранять GIF создания рядом с картинкой"));
                let gif_params = gif_on_enabled && self.settings.save_progress_gif;
                slider_u32(ui, gif_params, &mut self.settings.gif_frames, 2..=2000, lang.t("Captured frames (approx.)", "Кадров (примерно)"));
                slider_u32(ui, gif_params, &mut self.settings.gif_fps, 1..=50, lang.t("GIF playback FPS", "FPS воспроизведения GIF"));
                slider_u32(ui, gif_params, &mut self.settings.gif_max_width, 16..=2048, lang.t("Max GIF width (px)", "Макс. ширина GIF (px)"));
            });

        egui::CollapsingHeader::new(lang.t("Video (used for video input)", "Видео (для видео-файлов)"))
            .default_open(is_video)
            .show(ui, |ui| {
                if !is_video {
                    ui.label(egui::RichText::new(lang.t(
                        "Selected input is an image — video options are inactive.",
                        "Выбрано изображение — параметры видео неактивны.",
                    )).weak().italics());
                }
                slider_u32(ui, is_video, &mut self.settings.target_fps, 1..=60, lang.t("Target FPS", "Целевой FPS"));
                slider_u32(ui, is_video, &mut self.settings.interpolation_steps, 0..=20, lang.t("Interpolation steps", "Шаги интерполяции"));
                slider_u32(ui, is_video, &mut self.settings.mutations_per_shape, 1..=50, lang.t("Mutations per shape", "Мутаций на фигуру"));
                slider_f32(ui, is_video, &mut self.settings.displacement_weight, 0.0..=100.0, lang.t("Displacement weight", "Вес смещения"));
                slider_f32(ui, is_video, &mut self.settings.scene_change_tolerance, -10.0..=10.0, lang.t("Scene-change tolerance", "Порог смены сцены"));
                toggle(ui, is_video, &mut self.settings.preserve_audio, lang.t("Keep original audio", "Сохранять исходный звук"));

                let recolor_enabled = is_video && !self.settings.use_original_colors;
                toggle(ui, recolor_enabled, &mut self.settings.video_recolor, lang.t("Recolor shapes to new frame", "Перекрашивать под новый кадр"));
            });
    }

    fn footer(&mut self, ui: &mut egui::Ui) -> ScreenAction {
        let lang = self.language;
        let mut action = ScreenAction::None;

        ui.horizontal(|ui| {
            let start = ui.add_sized(
                [160.0, 36.0],
                egui::Button::new(
                    egui::RichText::new(lang.t("▶  Start", "▶  Пуск")).size(18.0),
                ),
            );

            if ui.button(lang.t("Save settings.toml", "Сохранить settings.toml")).clicked() {
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
                Ok(_) => self.set_status(lang.t("Saved settings.toml", "Сохранено в settings.toml"), false),
                Err(e) => self.set_status(format!("{}: {}", lang.t("Save failed", "Ошибка сохранения"), e), true),
            },
            Err(e) => self.set_status(format!("{}: {}", lang.t("Invalid settings", "Неверные настройки"), e), true),
        }
    }

    fn try_start(&mut self) -> ScreenAction {
        let lang = self.language;

        let media_path = match self.selected_media.and_then(|i| self.media_files.get(i)).cloned() {
            Some(p) => p,
            None => {
                self.set_status(lang.t("Select an input file first", "Сначала выберите входной файл"), true);
                return ScreenAction::None;
            }
        };

        if let Err(e) = self.settings.validate() {
            self.set_status(format!("{}: {}", lang.t("Invalid settings", "Неверные настройки"), e), true);
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
        egui::Slider::new(value, range)
            .text(label)
            .clamping(egui::SliderClamping::Always),
    );
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
        egui::Slider::new(value, range)
            .text(label)
            .max_decimals(4)
            .clamping(egui::SliderClamping::Always),
    );
}

/// A labelled checkbox, greyed out when disabled.
fn toggle(ui: &mut egui::Ui, enabled: bool, value: &mut bool, label: &str) {
    ui.add_enabled(enabled, egui::Checkbox::new(value, label));
}
