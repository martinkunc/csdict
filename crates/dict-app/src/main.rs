//! csdict – a fast, cross-platform dictionary (Czech ↔ English, German,
//! Ancient Greek, …).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod library;
mod online;
mod ratelimit;
mod settings;
mod speech;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CSDict")
            // Matches the .desktop file name, so Linux desktops show its icon.
            .with_app_id("dev.csdict.CSDict")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([520.0, 340.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_icon(icon()),
        #[cfg(feature = "screenshot")]
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native("csdict", options, Box::new(|cc| Ok(Box::new(app::App::new(cc, std::env::args().nth(1))))))
}

/// Window / taskbar icon (the same artwork as the .app / .exe / .desktop icons).
fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon/png/icon-256.png")).expect("valid icon png")
}
