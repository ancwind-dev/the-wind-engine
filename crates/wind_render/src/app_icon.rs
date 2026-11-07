//! Значок движка в заголовке окна и на панели задач.
//!
//! Логотип белый: на тёмной панели задач — без подложки, на светлой его не видно —
//! там он на чёрной подложке. Тему Windows сообщает окно (winit), а при смене темы
//! на ходу [`Follower`] меняет значок без перезапуска. Картинки делает `icon/make.py`.

use eframe::egui::{self, IconData, Theme};
use std::sync::Arc;

const DARK: &[u8] = include_bytes!("../../../icon/icon-dark.png");
const LIGHT: &[u8] = include_bytes!("../../../icon/icon-light.png");

/// Значок под тему: для светлой — на чёрной подложке.
pub fn for_theme(theme: Theme) -> IconData {
    let png = if theme == Theme::Light { LIGHT } else { DARK };
    eframe::icon_data::from_png_bytes(png).expect("значок — PNG из icon/make.py")
}

/// Значок при старте, пока окно не сообщило тему. Почти у всех панель задач тёмная.
pub fn initial() -> IconData {
    for_theme(Theme::Dark)
}

/// Следит за темой Windows и меняет значок окна, когда она меняется.
#[derive(Default)]
pub struct Follower {
    shown: Option<Theme>,
}

impl Follower {
    /// Вызывать раз в кадр: дёшево, команда окну уходит только при смене темы.
    pub fn update(&mut self, ctx: &egui::Context) {
        let theme = ctx.system_theme().unwrap_or(Theme::Dark);
        // Первый кадр со значком «тёмной» темы уже стоит — менять незачем.
        let shown = *self.shown.get_or_insert(Theme::Dark);
        if shown != theme {
            self.shown = Some(theme);
            ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(for_theme(theme)))));
        }
    }
}
