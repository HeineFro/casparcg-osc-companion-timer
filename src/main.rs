#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([720.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "CasparCG OSC Companion Timer",
        options,
        Box::new(|_cc| Box::new(app::App::new())),
    )
}
