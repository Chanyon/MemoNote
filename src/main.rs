#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod model;
mod settings;

use gpui_kit::assets::AllAssets;
use gpui_kit::*;

use crate::app::{CUSTOM_TITLEBAR, WhiteboardApp};
use crate::settings::{AppSettings, Language, apply_theme};

fn main() {
    let app = gpui_kit::application().with_assets(AllAssets);

    app.run(move |cx| {
        gpui_kit::init(cx);

        let settings = AppSettings::load();
        apply_theme(&settings.theme, cx);
        let title = match settings.language {
            Language::Zh => "白板笔记",
            Language::En => "Whiteboard Notes",
        };

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Maximized(Bounds::centered(
                None,
                size(px(1280.), px(820.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                appears_transparent: CUSTOM_TITLEBAR,
                ..Default::default()
            }),
            ..Default::default()
        };

        let app_settings = settings.clone();
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| WhiteboardApp::new(window, cx, app_settings.clone()))
        })
        .expect("failed to open window");
    });
}
