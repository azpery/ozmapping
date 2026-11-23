//! Onglet Laser : lasers ILDA (Ether Dream) avec leurs formes, et lasers DMX (motifs intégrés).
//! Sécurité : désarmés au démarrage (jamais réarmés tout seuls), arrêt immédiat, zone interdite.

use std::collections::HashMap;

use super::*;
use crate::laser::{self, Shape};

/// État des lasers propre à la session (non enregistré : les lasers repartent désarmés).
#[derive(Default)]
pub(super) struct LaserRuntime {
    /// Lasers armés : sans ça, rien ne sort vers les boîtiers.
    pub(super) armed: bool,
    streams: HashMap<u32, Arc<Mutex<laser::Shared>>>,
    pub(super) selected: Option<usize>,
    /// Trait en cours de dessin (forme « Dessin »).
    stroke: Vec<[f32; 2]>,
    /// Boîtiers trouvés sur le réseau, recherche en cours.
    found: Arc<Mutex<Option<Vec<String>>>>,
    searching: bool,
    cue_name: String,
    /// Son (forme d'onde, spectre) et contours de l'image de chaque sortie suivie, en direct.
    wave: Vec<f32>,
    bands: Vec<f32>,
    contours: HashMap<usize, Vec<Vec<[f32; 2]>>>,
    /// Entrée audio à part pour les lasers (et l'entrée qu'elle a ouverte), niveau du son.
    input: Option<crate::rhythm::AudioInput>,
    input_for: Option<Option<String>>,
    input_error: Option<String>,
    level: f32,
}

/// Contours de l'image d'une sortie (clarté au-dessus du seuil), en traits laser.
fn video_contours(frame: &crate::leds::Frame, threshold: f32) -> Vec<Vec<[f32; 2]>> {
    let (w, h) = (frame.size.0 as usize, frame.size.1 as usize);
    if w < 8 || h < 8 || frame.pixels.len() < w * h * 4 {
        return Vec::new();
    }
    // Clarté, lignes remises de haut en bas ; le seuil devient 0,5.
    let mut field = vec![0.0f32; w * h];
    for y in 0..h {
        let src = (h - 1 - y) * w * 4;
        for x in 0..w {
            let p = &frame.pixels[src + x * 4..src + x * 4 + 3];
            let lum = (0.3 * f32::from(p[0]) + 0.59 * f32::from(p[1]) + 0.11 * f32::from(p[2])) / 255.0;
            field[y * w + x] = lum - threshold + 0.5;
        }
    }
    let Ok((rings, _)) = crate::mask::field_to_rings_mapped(&field, w, h, |p| [p[0] / w as f64, p[1] / h as f64]) else {
        return Vec::new();
    };
    // Les plus grands contours d'abord, 500 points au plus (le laser ne peut pas tout tracer).
    let mut rings: Vec<Vec<[f32; 2]>> = rings
        .into_iter()
        .map(|r| {
            let mut v: Vec<[f32; 2]> = r.iter().map(|p| [p[0] * 2.0 - 1.0, 1.0 - p[1] * 2.0]).collect();
            if let Some(&f) = v.first() {
                v.push(f);
            }
            v
        })
        .collect();
    rings.sort_by_key(|r| std::cmp::Reverse(r.len()));
    let total: usize = rings.iter().map(Vec::len).sum();
    let step = (total / 500).max(1);
    rings
        .into_iter()
        .take(24)
        .map(|r| r.into_iter().step_by(step).collect::<Vec<_>>())
        .filter(|r| r.len() >= 3)
        .collect()
}

impl OzApp {
    /// Image de chaque laser, envoyée à son boîtier (à chaque image de l'interface).
    /// Entrées en direct d'un laser (son, contours de la vidéo, réglages mémorisés).
    pub(super) fn laser_input<'a>(&'a self, cue: &laser::Cue) -> laser::Input<'a> {
        laser::Input {
            video: self.laser.contours.get(&cue.video_output).map(|v| &v[..]),
            wave: &self.laser.wave,
            bands: &self.laser.bands,
            level: self.laser.level,
            library: &self.project.leds.laser_cues,
        }
    }

    pub(super) fn laser_tick(&mut self, ctx: &egui::Context) {
        let t = self.clock() as f32;
        let env = self.rhythm.pulse.env;
        let armed = self.laser.armed;
        // Son des lasers : celui de l'onglet Rythme, ou leur propre entrée audio.
        let settings = self.project.leds.laser_audio.clone();
        let source = match &settings.device {
            None => {
                self.laser.input = None;
                self.laser.input_for = None;
                self.rhythm.input.as_ref().map(|i| (i.wave(), self.rhythm.pulse.level))
            }
            Some(name) => {
                if self.laser.input_for.as_ref() != Some(name) {
                    self.laser.input_for = Some(name.clone());
                    match crate::rhythm::AudioInput::open(name.as_deref(), crate::rhythm::Band::Bass) {
                        Ok(i) => {
                            self.laser.input = Some(i);
                            self.laser.input_error = None;
                        }
                        Err(e) => {
                            self.laser.input = None;
                            self.laser.input_error = Some(format!("{e:#}"));
                        }
                    }
                }
                self.laser.input.as_ref().map(|i| (i.wave(), i.levels().level))
            }
        };
        let gain = settings.gain.clamp(0.05, 20.0);
        match source {
            Some(((wave, sr), level)) => {
                self.laser.level = (level * gain).clamp(0.0, 1.0);
                let lv = self.laser.level;
                self.laser.bands = laser::spectrum(&wave, sr, 16).into_iter().map(|b| b * (0.25 + 0.75 * lv)).collect();
                self.laser.wave = wave.into_iter().map(|x| x * gain).collect();
            }
            None => {
                self.laser.level = 0.0;
                self.laser.wave.clear();
                self.laser.bands.clear();
            }
        }
        // Contours de l'image des sorties suivies par des lasers.
        let outs: Vec<(usize, f32)> = self
            .project
            .leds
            .lasers
            .iter()
            .filter(|l| l.cue.shape == Shape::Video)
            .map(|l| (l.cue.video_output.min(self.project.outputs.len().max(1) - 1), l.cue.threshold))
            .collect();
        self.laser.contours.clear();
        for (k, th) in outs {
            let src = self.frame_of(k);
            let c = video_contours(&src.lock().unwrap(), th);
            self.laser.contours.insert(k, c);
        }
        let frames: Vec<(u32, Vec<laser::Point>)> = self
            .project
            .leds
            .lasers
            .iter()
            .map(|l| {
                // Une image pour un trentième de seconde de balayage.
                let budget = (l.pps / 30).clamp(100, 3000) as usize;
                (l.id, laser::frame(l, t, env, budget, &self.laser_input(&l.cue)))
            })
            .collect();
        for l in &self.project.leds.lasers {
            let shared = self.laser.streams.entry(l.id).or_insert_with(|| {
                let s = Arc::new(Mutex::new(laser::Shared::default()));
                laser::spawn_stream(s.clone());
                s
            });
            let frame = frames.iter().find(|f| f.0 == l.id).map(|f| f.1.clone()).unwrap_or_default();
            let mut s = shared.lock().unwrap();
            s.frame = frame;
            s.host = l.host.clone();
            s.pps = l.pps;
            s.armed = armed;
        }
        // Laser supprimé : son fil s'arrête (plus d'adresse).
        let ids: Vec<u32> = self.project.leds.lasers.iter().map(|l| l.id).collect();
        self.laser.streams.retain(|id, s| {
            let keep = ids.contains(id);
            if !keep {
                let mut s = s.lock().unwrap();
                s.armed = false;
                s.host.clear();
            }
            keep
        });
        if armed || !self.project.leds.lasers.is_empty() && self.right_tab == RightTab::Laser {
            ctx.request_repaint();
        }
    }

    /// Arrêt immédiat de tous les lasers ILDA.
    pub(super) fn laser_stop(&mut self) {
        self.laser.armed = false;
        for s in self.laser.streams.values() {
            s.lock().unwrap().armed = false;
        }
    }

    /// Onglet latéral Laser.
    pub(super) fn laser_ui(&mut self, ui: &mut egui::Ui) {
        // Échap : tout arrêter.
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.laser_stop();
        }
        self.safety_bar(ui);
        self.laser_audio_ui(ui);
        let sec = self.section();
        sec.show(ui, 0, |ui| self.ilda_ui(ui));
        sec.show(ui, 1, |ui| self.dmx_lasers_ui(ui));
    }

    /// Son du laser : source, sensibilité, contrôle en direct (niveau, forme d'onde, spectre).
    fn laser_audio_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(egui::RichText::new(with_icon(ic::WAVES, "Son du laser")).strong())
            .id_salt("laser_audio")
            .show(ui, |ui| {
                let a = &mut self.project.leds.laser_audio;
                let shown = match &a.device {
                    None => "Comme l'onglet Rythme".to_owned(),
                    Some(None) => "Entrée audio par défaut".to_owned(),
                    Some(Some(n)) => n.clone(),
                };
                ui.horizontal(|ui| {
                    ui.label("Source");
                    egui::ComboBox::from_id_salt("laser_audio_src").selected_text(shown).width(200.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut a.device, None, "Comme l'onglet Rythme");
                        ui.selectable_value(&mut a.device, Some(None), "Entrée audio par défaut");
                        for name in crate::rhythm::input_devices() {
                            ui.selectable_value(&mut a.device, Some(Some(name.clone())), name);
                        }
                    });
                });
                fit_slider(ui, "Sensibilité", egui::Slider::new(&mut a.gain, 0.1..=8.0).logarithmic(true).suffix("×"));
                if let Some(e) = &self.laser.input_error {
                    ui.label(egui::RichText::new(e).small().color(theme::DANGER));
                }
                if a.device.is_none() && self.rhythm.input.is_none() {
                    ui.label(
                        egui::RichText::new("Aucune entrée audio dans l'onglet Rythme : choisissez-en une ici, ou là-bas.")
                            .small()
                            .color(WARN),
                    );
                }
                // Contrôle en direct : niveau, forme d'onde, spectre.
                let w = ui.available_width().min(380.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(w, 56.0), Sense::hover());
                let p = ui.painter_at(r);
                p.rect_filled(r, 4.0, Color32::BLACK);
                let lv = self.laser.level;
                p.rect_filled(Rect::from_min_size(r.min, Vec2::new(6.0, r.height())), 2.0, theme::WIDGET);
                p.rect_filled(Rect::from_min_max(egui::pos2(r.min.x, r.max.y - r.height() * lv), egui::pos2(r.min.x + 6.0, r.max.y)), 2.0, ACCENT);
                let half = Rect::from_min_max(egui::pos2(r.min.x + 12.0, r.min.y), egui::pos2(r.center().x, r.max.y));
                if self.laser.wave.len() > 64 {
                    let n = 120;
                    let pts: Vec<egui::Pos2> = (0..n)
                        .map(|k| {
                            let x = self.laser.wave[k * (self.laser.wave.len() - 1) / (n - 1)];
                            egui::pos2(half.min.x + half.width() * k as f32 / (n - 1) as f32, half.center().y - (x * 3.0).clamp(-1.0, 1.0) * half.height() * 0.45)
                        })
                        .collect();
                    p.add(egui::Shape::line(pts, Stroke::new(1.2, ACCENT)));
                }
                let spec = Rect::from_min_max(egui::pos2(r.center().x + 6.0, r.min.y + 4.0), r.max - Vec2::splat(4.0));
                let nb = self.laser.bands.len().max(1);
                for (k, b) in self.laser.bands.iter().enumerate() {
                    let bw = spec.width() / nb as f32;
                    let h = spec.height() * b.clamp(0.0, 1.0);
                    p.rect_filled(Rect::from_min_max(egui::pos2(spec.min.x + k as f32 * bw + 1.0, spec.max.y - h), egui::pos2(spec.min.x + (k + 1) as f32 * bw - 1.0, spec.max.y)), 1.0, crate::leds::hue(k as f32 / nb as f32 * 0.8).map(|v| v as u8).into_color());
                }
                ui.ctx().request_repaint();
            });
    }

    fn safety_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let armed = self.laser.armed;
            let text = if armed { with_icon(ic::WARNING, "LASERS ARMÉS") } else { with_icon(ic::PROHIBIT, "Lasers désarmés (simulation)") };
            let mut b = egui::Button::new(egui::RichText::new(text).strong().color(if armed { BG } else { TEXT_WEAK }))
                .min_size(Vec2::new(ui.available_width() - 90.0, 34.0));
            if armed {
                b = b.fill(theme::DANGER);
            }
            if ui
                .add(b)
                .on_hover_text("Armer : les boîtiers Ether Dream reçoivent les formes. Les lasers repartent toujours désarmés au lancement.")
                .clicked()
            {
                self.laser.armed = !armed;
            }
            let stop = egui::Button::new(egui::RichText::new("STOP").strong().color(Color32::WHITE))
                .fill(theme::DANGER.gamma_multiply(0.8))
                .min_size(Vec2::new(80.0, 34.0));
            if ui.add(stop).on_hover_text("Tout éteindre immédiatement (Échap dans cet onglet)").clicked() {
                self.laser_stop();
            }
        });
        ui.label(
            egui::RichText::new("Sécurité : jamais de faisceau dans le public. Réglez la zone interdite de chaque laser, et respectez la réglementation des spectacles laser.")
                .small()
                .color(WARN),
        );
        ui.add_space(4.0);
    }

    /// Lasers ILDA : liste, réglages, forme et aperçu.
    fn ilda_ui(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, ic::LIGHTNING, "Lasers ILDA (Ether Dream)");
        ui.horizontal_wrapped(|ui| {
            if ui.button(with_icon(ic::PLUS, "Laser ILDA")).clicked() {
                let l = &mut self.project.leds;
                let id = l.lasers.iter().map(|x| x.id).max().unwrap_or(0) + 1;
                let pos = self.led.cursor3d.unwrap_or([0.0, 2.3, (l.room[1] - 1.0).max(0.5)]);
                l.lasers.push(laser::Laser { id, name: format!("Laser ILDA {id}"), pos, ..laser::Laser::default() });
                self.laser.selected = Some(l.lasers.len() - 1);
            }
            if self.laser.searching {
                if let Some(found) = self.laser.found.lock().unwrap().clone() {
                    self.laser.searching = false;
                    if found.is_empty() {
                        self.status = Some("Aucun boîtier Ether Dream trouvé sur le réseau".into());
                    }
                }
                ui.spinner();
            } else if ui.button(with_icon(ic::MAGNIFYING_GLASS, "Rechercher les boîtiers")).clicked() {
                self.laser.searching = true;
                *self.laser.found.lock().unwrap() = None;
                let found = self.laser.found.clone();
                std::thread::spawn(move || {
                    let list = laser::discover(std::time::Duration::from_secs(2));
                    *found.lock().unwrap() = Some(list);
                });
            }
        });
        if let Some(found) = self.laser.found.lock().unwrap().clone().filter(|f| !f.is_empty()) {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Trouvés :").small().color(TEXT_WEAK));
                for ip in found {
                    if ui.small_button(&ip).on_hover_text("Donner cette adresse au laser choisi").clicked() {
                        if let Some(l) = self.laser.selected.and_then(|i| self.project.leds.lasers.get_mut(i)) {
                            l.host = ip.clone();
                        }
                    }
                }
            });
        }
        let mut remove = None;
        for (i, l) in self.project.leds.lasers.iter().enumerate() {
            let st = self.laser.streams.get(&l.id).map(|s| {
                let s = s.lock().unwrap();
                (s.ok, s.status.clone())
            });
            ui.horizontal(|ui| {
                let (ok, msg) = st.unwrap_or((false, "simulation".into()));
                ui.label(egui::RichText::new(ic::CIRCLE).color(if ok { ACCENT } else { TEXT_WEAK })).on_hover_text(msg);
                if ui.selectable_label(self.laser.selected == Some(i), &l.name).clicked() {
                    self.laser.selected = Some(i);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(theme::icon_button(ic::TRASH).small().frame(false)).clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            self.project.leds.lasers.remove(i);
            self.laser.selected = None;
        }
        let Some(li) = self.laser.selected.filter(|&i| i < self.project.leds.lasers.len()) else {
            ui.label(egui::RichText::new("Ajoutez un laser ILDA (boîtier Ether Dream branché sur l'entrée ILDA du laser).").small().color(TEXT_WEAK));
            return;
        };

        let out_names: Vec<String> = self.project.outputs.iter().map(|o| o.name.clone()).collect();
        let cue_names: Vec<String> = self.project.leds.laser_cues.iter().map(|c| c.name.clone()).collect();
        // Réglages du laser.
        let l = &mut self.project.leds.lasers[li];
        egui::CollapsingHeader::new("Boîtier, sécurité, placement").id_salt("laser_dev").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Nom");
                ui.text_edit_singleline(&mut l.name);
            });
            ui.horizontal(|ui| {
                ui.label("Ether Dream");
                ui.add(egui::TextEdit::singleline(&mut l.host).hint_text("vide : simulation").desired_width(130.0));
                ui.label("Points/s");
                ui.add(egui::DragValue::new(&mut l.pps).range(1000..=100_000).speed(100.0))
                    .on_hover_text("Vitesse du scanner du laser (voir sa notice ; souvent 20 000 à 30 000)");
            });
            fit_slider(ui, "Zone interdite (bas)", egui::Slider::new(&mut l.floor, -1.0..=1.0))
                .on_hover_text("Rien n'est tracé sous cette hauteur du champ : les faisceaux restent au-dessus du public");
            percent_slider(ui, &mut l.max, "Luminosité maximale");
            fit_slider(ui, "Ouverture (simulation)", egui::Slider::new(&mut l.fov, 5.0..=90.0).suffix("°"));
            ui.horizontal(|ui| {
                ui.label("Position");
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.add(egui::DragValue::new(&mut l.pos[k]).speed(0.01).prefix(format!("{axis} ")).suffix(" m"));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Rotation");
                for (k, name) in ["cap", "incl.", "roulis"].iter().enumerate() {
                    ui.add(egui::DragValue::new(&mut l.rot[k]).speed(1.0).range(-360.0..=360.0).prefix(format!("{name} ")).suffix("°"));
                }
            });
        });

        // Forme et animation.
        let cue = &mut l.cue;
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Forme").strong());
        ui.horizontal_wrapped(|ui| {
            for sh in Shape::ALL {
                if ui.selectable_label(cue.shape == sh, sh.name()).clicked() {
                    cue.shape = sh;
                }
            }
        });
        percent_slider(ui, &mut cue.size, "Taille");
        ui.horizontal(|ui| {
            ui.label("Position");
            ui.add(egui::DragValue::new(&mut cue.x).range(-1.0..=1.0).speed(0.005).prefix("X "));
            ui.add(egui::DragValue::new(&mut cue.y).range(-1.0..=1.0).speed(0.005).prefix("Y "));
        });
        fit_slider(ui, "Rotation (tours/s)", egui::Slider::new(&mut cue.spin, -2.0..=2.0));
        ui.horizontal(|ui| {
            ui.label("Couleur");
            ui.color_edit_button_srgb(&mut cue.color);
            ui.checkbox(&mut cue.rainbow, "Arc-en-ciel");
            if cue.rainbow {
                ui.add(egui::DragValue::new(&mut cue.color_speed).range(-3.0..=3.0).speed(0.01).prefix("vitesse "));
            }
        });
        percent_slider(ui, &mut cue.pulse, "Pulsation sur les temps");
        percent_slider(ui, &mut cue.audio, "Réagit au son").on_hover_text("La taille suit le volume du son du laser (réglages « Son du laser » en haut)");
        percent_slider(ui, &mut cue.wobble, "Ondulation");
        match cue.shape {
            Shape::Text => {
                ui.horizontal(|ui| {
                    ui.label("Texte");
                    ui.text_edit_singleline(&mut cue.text);
                });
                fit_slider(ui, "Défilement", egui::Slider::new(&mut cue.scroll, -2.0..=2.0))
                    .on_hover_text("0 : fixe ; sinon le texte défile (négatif : vers la droite)");
            }
            Shape::Video => {
                if out_names.len() > 1 {
                    ui.horizontal(|ui| {
                        ui.label("Sortie");
                        let shown = out_names.get(cue.video_output).cloned().unwrap_or_default();
                        egui::ComboBox::from_id_salt("laser_video_out").selected_text(shown).show_ui(ui, |ui| {
                            for (k, n) in out_names.iter().enumerate() {
                                ui.selectable_value(&mut cue.video_output, k, n);
                            }
                        });
                    });
                }
                percent_slider(ui, &mut cue.threshold, "Seuil de clarté")
                    .on_hover_text("Le laser trace le bord des zones plus claires que ce seuil");
            }
            Shape::Scope | Shape::Spectrum | Shape::AudioCircle => {
                ui.label(
                    egui::RichText::new("Le son vient de l'onglet Rythme (micro, carte son). Sans entrée audio : une animation calme.")
                        .small()
                        .color(TEXT_WEAK),
                );
            }
            Shape::Fan | Shape::Sheet => {
                if cue.shape == Shape::Fan {
                    ui.horizontal(|ui| {
                        ui.label("Faisceaux");
                        ui.add(egui::DragValue::new(&mut cue.beams).range(1..=32));
                    });
                }
                percent_slider(ui, &mut cue.sweep, "Balayage");
                fit_slider(ui, "Vitesse du balayage", egui::Slider::new(&mut cue.sweep_speed, 0.0..=3.0));
                ui.label(egui::RichText::new("Effets de faisceaux : ils se voient dans la fumée. Toujours au-dessus du public.").small().color(WARN));
            }
            Shape::Morph => {
                let names: Vec<String> = cue_names.clone();
                if names.len() < 2 {
                    ui.label(egui::RichText::new("Mémorisez au moins deux réglages (plus bas) pour passer de l'un à l'autre.").small().color(WARN));
                } else {
                    for (label, idx) in [("De", &mut cue.morph_a), ("Vers", &mut cue.morph_b)] {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            let shown = names.get(*idx).cloned().unwrap_or_default();
                            egui::ComboBox::from_id_salt(("laser_morph", label)).selected_text(shown).show_ui(ui, |ui| {
                                for (k, n) in names.iter().enumerate() {
                                    ui.selectable_value(idx, k, n);
                                }
                            });
                        });
                    }
                    fit_slider(ui, "Vitesse (allers-retours/s)", egui::Slider::new(&mut cue.morph_speed, 0.02..=2.0).logarithmic(true));
                }
            }
            _ => {}
        }

        // Aperçu (et planche de dessin pour la forme « Dessin / logo »).
        let drawing = cue.shape == Shape::Drawn;
        let w = ui.available_width().min(380.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(w), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::BLACK);
        let to_px = |p: [f32; 2]| rect.center() + Vec2::new(p[0], -p[1]) * (w * 0.5 - 6.0);
        let to_unit = |q: egui::Pos2| {
            let v = (q - rect.center()) / (w * 0.5 - 6.0);
            [v.x.clamp(-1.0, 1.0), (-v.y).clamp(-1.0, 1.0)]
        };
        // Zone interdite.
        let fy = to_px([0.0, l.floor]).y;
        painter.rect_filled(Rect::from_min_max(egui::pos2(rect.min.x, fy), rect.max), 6.0, theme::DANGER.gamma_multiply(0.12));
        painter.hline(rect.x_range(), fy, Stroke::new(1.0, theme::DANGER.gamma_multiply(0.7)));
        let t = self.start.elapsed().as_secs_f32();
        let l = &self.project.leds.lasers[li];
        let frame = laser::frame(l, t, self.rhythm.pulse.env, 700, &self.laser_input(&l.cue));
        for wnd in frame.windows(2) {
            let (a, b) = (wnd[0], wnd[1]);
            if b.lit() {
                let c = [b.r, b.g, b.b].map(|v| (v * 255.0) as u8);
                painter.line_segment([to_px([a.x, a.y]), to_px([b.x, b.y])], Stroke::new(4.0, Color32::from_rgba_premultiplied(c[0] / 5, c[1] / 5, c[2] / 5, 0)));
                painter.line_segment([to_px([a.x, a.y]), to_px([b.x, b.y])], Stroke::new(1.5, Color32::from_rgb(c[0], c[1], c[2])));
            }
        }
        ui.ctx().request_repaint();
        let cue = &mut self.project.leds.lasers[li].cue;
        if drawing {
            // Dessin : clics = points du trait ; Entrée ou double-clic : finir le trait.
            let stroke = &mut self.laser.stroke;
            let pts: Vec<egui::Pos2> = stroke.iter().map(|&p| to_px(p)).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(1.5, WARN)));
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    stroke.push(to_unit(p));
                }
            }
            let finish = resp.double_clicked() || ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            if finish && stroke.len() >= 2 {
                cue.strokes.push(std::mem::take(stroke));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Clic : un point · Entrée ou double-clic : finir le trait").small().color(TEXT_WEAK));
                if ui.small_button("Effacer le dessin").clicked() {
                    cue.strokes.clear();
                    self.laser.stroke.clear();
                }
                if ui.small_button(with_icon(ic::FILE, "Importer un logo (SVG)…")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("SVG", &["svg"]).pick_file() {
                        match crate::mask::import_svg(&path, 1.0) {
                            Ok((rings, _)) => {
                                cue.strokes = svg_strokes(&rings);
                            }
                            Err(e) => self.status = Some(format!("SVG : {e:#}")),
                        }
                    }
                }
            });
        } else if resp.clicked() {
            // Clic dans l'aperçu : la forme y va.
            if let Some(p) = resp.interact_pointer_pos() {
                let u = to_unit(p);
                cue.x = u[0];
                cue.y = u[1];
            }
        }

        // Réglages mémorisés.
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Réglages mémorisés").strong());
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.laser.cue_name).hint_text("Nom").desired_width(140.0));
            if ui.button(with_icon(ic::BOOKMARK_SIMPLE, "Mémoriser")).clicked() {
                let mut c = self.project.leds.lasers[li].cue.clone();
                c.name = if self.laser.cue_name.trim().is_empty() { c.shape.name().to_owned() } else { self.laser.cue_name.trim().to_owned() };
                self.project.leds.laser_cues.push(c);
                self.laser.cue_name.clear();
            }
        });
        let mut apply = None;
        let mut delete = None;
        ui.horizontal_wrapped(|ui| {
            for (k, c) in self.project.leds.laser_cues.iter().enumerate() {
                let r = ui.button(&c.name).on_hover_text("Clic : appliquer au laser choisi · clic droit : supprimer");
                if r.clicked() {
                    apply = Some(k);
                }
                if r.secondary_clicked() {
                    delete = Some(k);
                }
            }
        });
        if let Some(k) = apply {
            let c = self.project.leds.laser_cues[k].clone();
            self.project.leds.lasers[li].cue = c;
        }
        if let Some(k) = delete {
            self.project.leds.laser_cues.remove(k);
        }
    }

    /// Lasers DMX (onglet DMX, type Laser) : motifs intégrés et réglages, en grand.
    fn dmx_lasers_ui(&mut self, ui: &mut egui::Ui) {
        use crate::dmx::{Kind, Role};
        theme::section(ui, ic::LIGHTNING, "Lasers DMX (motifs intégrés)");
        ui.label(
            egui::RichText::new("Les lasers DMX (Laserworld par leur entrée DMX) jouent les motifs de leur mémoire. Ajoutez-les dans l'onglet DMX ; la sortie DMX s'y règle aussi.")
                .small()
                .color(TEXT_WEAK),
        );
        let mut any = false;
        for d in self.project.leds.dmx.iter_mut().filter(|d| d.kind == Kind::Laser) {
            any = true;
            theme::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&d.name).strong());
                    ui.checkbox(&mut d.on, "Allumé");
                    ui.checkbox(&mut d.beat, "Motif sur les temps");
                });
                // Galerie des motifs : 16 cases (valeur du canal Motif), nommables.
                if let Some(ci) = d.channels.iter().position(|c| c.role == Role::Pattern) {
                    if d.labels.len() < 16 {
                        d.labels.resize(16, String::new());
                    }
                    let current = d.channels[ci].value;
                    egui::Grid::new(("laser_patterns", d.id)).num_columns(4).spacing([4.0, 4.0]).show(ui, |ui| {
                        for k in 0..16u8 {
                            let v = k * 16;
                            let label = if d.labels[k as usize].is_empty() { format!("Motif {}", k + 1) } else { d.labels[k as usize].clone() };
                            let on = current >= v && current < v.saturating_add(16);
                            let r = ui.add_sized([80.0, 26.0], egui::Button::selectable(on, label)).on_hover_text(format!("Valeur {v} · clic droit : renommer"));
                            if r.clicked() {
                                d.channels[ci].value = v;
                            }
                            r.context_menu(|ui| {
                                ui.label("Nom du motif");
                                ui.text_edit_singleline(&mut d.labels[k as usize]);
                            });
                            if k % 4 == 3 {
                                ui.end_row();
                            }
                        }
                    });
                }
                for c in d.channels.iter_mut().filter(|c| matches!(c.role, Role::Size | Role::Rotation | Role::PosX | Role::PosY | Role::Color | Role::Speed | Role::Strobe)) {
                    fit_slider(ui, c.role.name(), egui::Slider::new(&mut c.value, 0..=255));
                }
            });
        }
        if !any {
            ui.label(egui::RichText::new("Aucun laser DMX : onglet DMX › « + Laser ».").color(TEXT_WEAK));
        }
    }
}

/// Contours d'un SVG (0-1, y vers le bas) en traits laser (-1..1, y vers le haut), allégés.
fn svg_strokes(rings: &[Vec<[f32; 2]>]) -> Vec<Vec<[f32; 2]>> {
    let total: usize = rings.iter().map(Vec::len).sum();
    let step = (total / 400).max(1);
    rings
        .iter()
        .map(|r| {
            let mut s: Vec<[f32; 2]> = r.iter().step_by(step).map(|p| [p[0] * 2.0 - 1.0, 1.0 - p[1] * 2.0]).collect();
            if let Some(&first) = s.first() {
                s.push(first);
            }
            s
        })
        .filter(|s| s.len() >= 3)
        .collect()
}

/// Couleur RVB (0-255) → couleur d'interface.
trait IntoColor {
    fn into_color(self) -> Color32;
}

impl IntoColor for [u8; 3] {
    fn into_color(self) -> Color32 {
        Color32::from_rgb(self[0], self[1], self[2])
    }
}
