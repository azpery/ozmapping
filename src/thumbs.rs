//! Miniatures des médias, générées en arrière-plan par un thread unique.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui;

pub const THUMB_W: u32 = 160;
pub const THUMB_H: u32 = 90;

type Results = Arc<Mutex<Vec<(PathBuf, Option<Vec<u8>>)>>>;

pub struct Thumbnails {
    requests: Sender<PathBuf>,
    results: Results,
    requested: HashSet<PathBuf>,
    textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

impl Thumbnails {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel::<PathBuf>();
        let results: Results = Arc::default();
        let thread_results = results.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("thumbnails".into())
            .spawn(move || {
                for path in rx {
                    let image = crate::video::thumbnail(&path, THUMB_W, THUMB_H).ok();
                    thread_results.lock().unwrap().push((path, image));
                    ctx.request_repaint();
                }
            })
            .expect("thread miniatures");
        Self {
            requests: tx,
            results,
            requested: HashSet::new(),
            textures: HashMap::new(),
        }
    }

    /// Miniature d'un média ; la demande est lancée au premier appel.
    pub fn get(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        for (p, data) in self.results.lock().unwrap().drain(..) {
            let tex = data.map(|rgba| {
                let image = egui::ColorImage::from_rgba_unmultiplied([THUMB_W as usize, THUMB_H as usize], &rgba);
                ctx.load_texture(format!("thumb:{}", p.display()), image, egui::TextureOptions::LINEAR)
            });
            self.textures.insert(p, tex);
        }
        if self.requested.insert(path.to_path_buf()) {
            let _ = self.requests.send(path.to_path_buf());
        }
        self.textures.get(path).cloned().flatten()
    }
}
