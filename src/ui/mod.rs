// User-facing configuration UI (Settings screen, i18n).

pub mod i18n;
pub mod settings_screen;

pub use i18n::Language;
pub use settings_screen::{ScreenAction, SettingsScreen};
