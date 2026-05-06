// qsim_ui — quantum circuit simulator UI prototype.
//
// Top-level binary: configures the eframe window and launches the app.
// Module wiring lives in `app::QSimApp`; everything else is a sibling
// module so swapping the host (eframe / wasm / kittest) only edits this
// file.

mod app;
mod components;
mod dsl;
mod grid;
mod qiskit;
mod state;
mod theme;
mod tiling;
mod turbospin;
mod workspace;

use app::QSimApp;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("qsim — quantum circuit editor"),
        ..Default::default()
    };

    eframe::run_native(
        "qsim",
        native_options,
        Box::new(|cc| Ok(Box::new(QSimApp::new(cc)))),
    )
}
