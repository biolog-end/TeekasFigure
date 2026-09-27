use super::Language;
use crate::io::frame_set::{self, Background, Options};
use std::path::{Path, PathBuf};

pub enum Action {
    Browse,
    Start(Options, bool),
}
pub struct FrameSetForm {
    pub open: bool,
    pub video: Option<PathBuf>,
    start: String,
    end: String,
    interval: f64,
    max_side: u32,
    background: Background,
    tolerance: u8,
    crop: bool,
    skip_blank: bool,
    use_target: bool,
    error: String,
}
impl Default for FrameSetForm {
    fn default() -> Self {
        Self {
            open: false,
            video: None,
            start: "0:00".into(),
            end: String::new(),
            interval: 2.0,
            max_side: 512,
            background: Background::Keep,
            tolerance: 40,
            crop: true,
            skip_blank: true,
            use_target: true,
            error: String::new(),
        }
    }
}
impl FrameSetForm {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        lang: Language,
        busy: bool,
        base: &Path,
        resolution: u32,
    ) -> Option<Action> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let mut action = None;
        egui::Window::new(lang.t("Shape set from video", "Набор картинок из видео"))
            .open(&mut open).default_width(510.0).collapsible(false).vscroll(true).show(ctx, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if ui.button(lang.t("Choose video…", "Выбрать видео…")).clicked() { action = Some(Action::Browse); }
                    if let Some(video) = &self.video { ui.label(video.file_name().unwrap_or_default().to_string_lossy()); }
                    ui.label(lang.t("Select a time range and how often to capture a frame.", "Выберите диапазон времени и частоту снимков."));
                    egui::Grid::new("frame_time_range").num_columns(2).show(ui, |ui| {
                        ui.label(lang.t("From (MM:SS or HH:MM:SS)", "С (ММ:СС или ЧЧ:ММ:СС)"));
                        ui.text_edit_singleline(&mut self.start); ui.end_row();
                        ui.label(lang.t("Until (blank = end of video)", "До (пусто = конец видео)"));
                        ui.text_edit_singleline(&mut self.end); ui.end_row();
                        ui.label(lang.t("Capture every, seconds", "Снимок каждые, секунд"));
                        ui.add(egui::DragValue::new(&mut self.interval).speed(0.1)); ui.end_row();
                        ui.label(lang.t("Maximum frame side, pixels", "Максимальная сторона кадра, пикселей"));
                        ui.add(egui::DragValue::new(&mut self.max_side).speed(16.0)); ui.end_row();
                    });
                    ui.small(lang.t("Example: 2:00 to 3:00, every 2 seconds → about 30 pictures.", "Например: с 2:00 до 3:00, каждые 2 секунды → около 30 картинок."));
                    egui::ComboBox::from_id_salt("frame_background_mode").selected_text(match self.background {
                        Background::Keep => lang.t("Keep background", "Сохранить фон"),
                        Background::Border => lang.t("Remove plain background", "Удалить однотонный фон"),
                        Background::Ai => lang.t("AI foreground (U²-Net)", "Выделить объект — ИИ (U²-Net)"),
                    }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.background,Background::Keep,lang.t("Keep background", "Сохранить фон"));
                        ui.selectable_value(&mut self.background,Background::Border,lang.t("Remove plain background (Bad Apple)", "Удалить однотонный фон (Bad Apple)"));
                        ui.selectable_value(&mut self.background,Background::Ai,lang.t("AI foreground (U²-Net)", "Выделить объект — ИИ (U²-Net)"));
                    });
                    if self.background == Background::Border {
                        ui.add(egui::Slider::new(&mut self.tolerance,0..=255).text(lang.t("Color tolerance", "Допуск цвета")));
                        ui.small(lang.t("Removes the dominant border color connected to the frame edges.", "Убирает основной цвет фона, связанный с краями кадра."));
                    }
                    if self.background == Background::Ai {
                        ui.small(lang.t("Runs locally on CPU. Complex scenes can need manual cleanup.", "Работает локально на CPU. Сложные сцены могут требовать ручной доработки."));
                        if frame_set::model_path(base).is_none() {
                            ui.colored_label(egui::Color32::from_rgb(255,160,45),lang.t("U²-Net model is missing. Setup instructions are in README.", "Модель U²-Net не найдена. Инструкция по установке — в README."));
                        }
                    }
                    ui.checkbox(&mut self.crop,lang.t("Crop transparent margins", "Обрезать прозрачные поля"));
                    ui.checkbox(&mut self.skip_blank,lang.t("Skip solid-color frames", "Пропускать одноцветные кадры"));
                    ui.checkbox(&mut self.use_target,lang.t("Use this video as the target too", "Выбрать это же видео образцом для сборки"));
                    ui.small(lang.t("A separate set is created and selected. Existing photos stay in their libraries.", "Будет создан и выбран отдельный набор. Имеющиеся фотографии сохранятся в своих библиотеках."));
                    if ui.add_enabled(self.video.is_some(),egui::Button::new(lang.t("Create set", "Создать набор"))).clicked() {
                        let parsed = (|| {
                            let start = frame_set::parse_time(&self.start)?;
                            let end = if self.end.trim().is_empty() { None } else { Some(frame_set::parse_time(&self.end)?) };
                            if end.map(|end| end <= start).unwrap_or(false) || !self.interval.is_finite() || self.interval <= 0.0 || self.max_side == 0 {
                                return Err("Check the range, interval and size / Проверьте диапазон, интервал и размер".to_string());
                            }
                            Ok(Options { video: self.video.clone().unwrap(),start,end,interval:self.interval,max_side:self.max_side,
                                background:self.background,tolerance:self.tolerance,crop:self.crop,skip_blank:self.skip_blank,resolution })
                        })();
                        match parsed { Ok(options) => action=Some(Action::Start(options,self.use_target)),Err(e)=>self.error=e }
                    }
                    if !self.error.is_empty() { ui.colored_label(egui::Color32::LIGHT_RED,&self.error); }
                });
            });
        self.open = open && !matches!(action, Some(Action::Start(..)));
        action
    }
}
