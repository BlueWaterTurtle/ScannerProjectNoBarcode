mod app;
mod events;
mod file_ops;
mod processor;
mod scanner_hw;
mod watcher;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PO Scanner")
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "PO Scanner",
        native_options,
        Box::new(|cc| Ok(Box::new(app::ScannerApp::new(cc)))),
    )
}
