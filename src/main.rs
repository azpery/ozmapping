// Sous Windows, pas de fenêtre de console derrière l'app (en build release).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod automix;
mod cpu;
mod cutout;
mod effects;
mod face;
mod face_mesh;
mod generator;
mod icons;
mod leds;
mod mask;
mod midi;
mod paint;
mod project;
mod recorder;
mod renderer;
mod rhythm;
mod road;
mod screens;
mod surface;
mod theme;
mod thumbs;
mod video;

use eframe::egui;

fn main() -> eframe::Result {
    // Arguments : un projet .ozmap, ou des vidéos (la première est lancée).
    let files: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OZ Mapping")
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([960.0, 600.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "OZ Mapping",
        options,
        Box::new(|cc| Ok(Box::new(app::OzApp::new(cc, files)))),
    )
}
