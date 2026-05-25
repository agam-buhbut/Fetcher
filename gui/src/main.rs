use eframe::egui;

mod app;
mod ui;

use app::App;

fn main() -> anyhow::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native("Task Manager", opts, Box::new(|_| Ok(Box::new(App::new()))))
        .map_err(|e| anyhow::anyhow!("{e}"))
}
