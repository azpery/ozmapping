//! Enregistrement de la sortie dans un fichier vidéo.

use super::*;

impl OzApp {
    /// Bouton d'enregistrement de la sortie (barre du haut) : réglages, démarrage, arrêt.
    pub(super) fn record_widget(&mut self, ui: &mut egui::Ui) {
        // Fin automatique (durée atteinte) ou erreur de ffmpeg.
        let auto_stop = self.recorder.as_ref().map(|r| {
            let r = r.lock().unwrap();
            (r.done(), r.error())
        });
        if let Some((done, err)) = auto_stop {
            if let Some(e) = err {
                *self.rec_result.lock().unwrap() = Some(Err(e));
                self.stop_recording();
            } else if done {
                self.stop_recording();
            }
        }
        if let Some(rec) = &self.recorder {
            let (secs, dropped) = {
                let r = rec.lock().unwrap();
                (r.elapsed(), r.dropped)
            };
            // Pastille rouge qui clignote.
            let on = (secs * 2.0).fract() < 0.5;
            let label = egui::RichText::new(format!("{} REC {}", ic::CIRCLE, fmt_time(f64::from(secs))))
                .color(if on { theme::DANGER } else { theme::DANGER.gamma_multiply(0.5) })
                .strong();
            let mut hover = "Arrêter l'enregistrement".to_owned();
            if dropped > 0 {
                hover.push_str(&format!("\n{dropped} images perdues (encodage trop lent : baisser la définition)"));
            }
            if ui.button(label).on_hover_text(hover).clicked() {
                self.stop_recording();
            }
            return;
        }
        ui.menu_button(with_icon(ic::CIRCLE, "Enregistrer"), |ui| {
            ui.label(egui::RichText::new("Enregistrer la sortie en vidéo (MP4)").strong());
            ui.label(
                egui::RichText::new("Tout ce qui passe sur la sortie, du démarrage à l'arrêt.").small().color(TEXT_WEAK),
            );
            ui.horizontal(|ui| {
                ui.label("Définition");
                for h in recorder::HEIGHTS {
                    let name = match h {
                        720 => "720p",
                        1080 => "1080p",
                        _ => "4K",
                    };
                    ui.selectable_value(&mut self.rec_height, h, name);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Images/s");
                for f in recorder::FPS {
                    ui.selectable_value(&mut self.rec_fps, f, f.to_string());
                }
            });
            ui.horizontal(|ui| {
                ui.label("Durée max");
                ui.add(egui::DragValue::new(&mut self.rec_max_secs).range(0.0..=36_000.0).speed(1.0).suffix(" s"))
                    .on_hover_text("0 = jusqu'à ce qu'on arrête");
            });
            let size = self.record_size();
            ui.label(egui::RichText::new(format!("{} × {} pixels", size.0, size.1)).small().color(TEXT_WEAK));
            if ui.add(theme::primary_button(with_icon(ic::CIRCLE, "Démarrer…"))).clicked() {
                ui.close();
                self.start_recording();
            }
        });
    }

    /// Taille de la vidéo : la hauteur choisie, aux proportions de la sortie (dimensions paires).
    pub(super) fn record_size(&self) -> (u32, u32) {
        let aspect = (self.output_size.x / self.output_size.y.max(1.0)).clamp(0.2, 5.0);
        let h = self.rec_height;
        (((h as f32 * aspect).round() as u32) & !1, h & !1)
    }

    pub(super) fn start_recording(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Vidéo MP4", &["mp4"])
            .set_file_name(recorder::default_file_name());
        if let Some(dir) = self.project_path.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut path) = dialog.save_file() else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("mp4");
        }
        match recorder::Recorder::start(&path, self.record_size(), self.rec_fps, self.rec_max_secs) {
            Ok(r) => {
                self.recorder = Some(Arc::new(Mutex::new(r)));
                *self.rec_result.lock().unwrap() = None;
            }
            Err(e) => *self.rec_result.lock().unwrap() = Some(Err(format!("{e:#}"))),
        }
    }

    /// Arrête l'enregistrement ; ffmpeg finit le fichier en arrière-plan (sans figer l'interface).
    pub(super) fn stop_recording(&mut self) {
        let Some(mut rec) = self.recorder.take() else {
            return;
        };
        let result = self.rec_result.clone();
        std::thread::spawn(move || {
            // Le dernier rendu peut encore tenir une référence : on attend qu'il la rende.
            let rec = loop {
                match Arc::try_unwrap(rec) {
                    Ok(m) => break m.into_inner().unwrap(),
                    Err(r) => {
                        rec = r;
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            };
            let res = rec.finish().map_err(|e| format!("{e:#}"));
            let mut slot = result.lock().unwrap();
            // Une erreur déjà signalée (ffmpeg) reste affichée.
            if !matches!(*slot, Some(Err(_))) || res.is_err() {
                *slot = Some(res);
            }
        });
    }
}
