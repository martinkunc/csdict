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
            .with_title("csdict")
            .with_app_id("csdict")
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

/// Window icon: a white "D" on a blue rounded square, drawn procedurally.
fn icon() -> egui::IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    let (r0, outer, inner) = (12.0f32, 22.0f32, 14.0f32);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded square mask.
            let dx = (fx - 32.0).abs() - (32.0 - r0);
            let dy = (fy - 32.0).abs() - (32.0 - r0);
            let corner = dx.max(0.0).hypot(dy.max(0.0));
            if corner > r0 {
                continue;
            }
            // Letter D: vertical stem + half ring centred at (30, 32).
            let stem = (16.0..23.0).contains(&fx) && (10.0..54.0).contains(&fy);
            let d = (fx - 30.0).hypot(fy - 32.0);
            let bowl = fx >= 23.0 && d <= outer && d >= inner;
            let bar = (16.0..30.0).contains(&fx) && ((10.0..18.0).contains(&fy) || (46.0..54.0).contains(&fy));
            let px = if stem || bowl || bar { [255, 255, 255, 255] } else { [0x2f, 0x6f, 0xd6, 255] };
            rgba[(y * N + x) * 4..][..4].copy_from_slice(&px);
        }
    }
    egui::IconData { rgba, width: N as u32, height: N as u32 }
}
