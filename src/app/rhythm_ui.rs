//! Onglet Rythme : source audio, tempo tapé, réactions au rythme.

use super::*;

impl OzApp {
    /// Voyant de temps + bouton TAP (barre du haut).
    pub(super) fn tap_widget(&mut self, ui: &mut egui::Ui) {
        let pulse = self.rhythm.pulse;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        let on = self.rhythm.active();
        let color = if on { ACCENT.gamma_multiply(0.25 + 0.75 * pulse.env) } else { theme::BORDER };
        ui.painter().circle_filled(rect.center(), 5.0, color);
        let label = match self.rhythm.source {
            rhythm::Source::Audio(_) if on => "SON".to_owned(),
            rhythm::Source::Manual => "MANUEL".to_owned(),
            _ => format!("{:.0} BPM", self.rhythm.tempo.bpm),
        };
        if ui
            .button(with_icon(ic::WAVES, &format!("TAP (T) · {label}")))
            .on_hover_text("Taper en rythme pour donner le tempo (T). Réglages dans l'onglet Rythme.")
            .clicked()
        {
            self.tap();
        }
    }

    pub(super) fn rhythm_ui(&mut self, ui: &mut egui::Ui) {
        if self.section().0 == 0 {
            self.rhythm_source_ui(ui);
        } else {
            self.rhythm_react_ui(ui);
        }
    }

    pub(super) fn rhythm_source_ui(&mut self, ui: &mut egui::Ui) {
        let current = match &self.rhythm.source {
            rhythm::Source::Off => "Aucune".to_owned(),
            rhythm::Source::Audio(None) => match &self.rhythm.input {
                Some(input) => format!("Par défaut ({})", input.device),
                None => "Entrée audio par défaut".to_owned(),
            },
            rhythm::Source::Audio(Some(n)) => n.clone(),
            rhythm::Source::Tempo => "Tempo tapé (BPM)".to_owned(),
            rhythm::Source::Manual => "Manuel (touche T)".to_owned(),
        };
        let mut chosen = None;
        let combo = egui::ComboBox::from_id_salt("rhythm_source").selected_text(current).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(self.rhythm.source == rhythm::Source::Off, "Aucune").clicked() {
                chosen = Some(rhythm::Source::Off);
            }
            if ui.selectable_label(self.rhythm.source == rhythm::Source::Tempo, "Tempo tapé (BPM)").clicked() {
                chosen = Some(rhythm::Source::Tempo);
            }
            if ui
                .selectable_label(self.rhythm.source == rhythm::Source::Manual, "Manuel (touche T)")
                .on_hover_text("Chaque appui sur T donne une pulsation ; sans appui, plus rien ne bouge")
                .clicked()
            {
                chosen = Some(rhythm::Source::Manual);
            }
            ui.separator();
            if ui.selectable_label(self.rhythm.source == rhythm::Source::Audio(None), "Entrée audio par défaut").clicked() {
                chosen = Some(rhythm::Source::Audio(None));
            }
            for name in rhythm::input_devices() {
                let source = rhythm::Source::Audio(Some(name.clone()));
                if ui.selectable_label(self.rhythm.source == source, &name).clicked() {
                    chosen = Some(source);
                }
            }
        });
        combo.response.on_hover_text(
            "Son : micro ou entrée de la carte son (pour la sortie de l'ordinateur, utiliser une entrée virtuelle \
             comme BlackHole sur Mac ou « Mixage stéréo » sous Windows).\nTempo : taper le rythme avec le bouton TAP ou la touche T.\nManuel : une pulsation à chaque appui sur T, rien sans appui.",
        );
        if let Some(source) = chosen {
            self.rhythm.set_source(source);
        }
        if let Some(e) = &self.rhythm.error {
            ui.colored_label(WARN, e);
        }
        if matches!(self.rhythm.source, rhythm::Source::Audio(_)) {
            ui.horizontal(|ui| {
                ui.label("Réagir aux");
                for band in rhythm::Band::ALL {
                    if ui
                        .selectable_label(self.rhythm.band == band, band.label())
                        .on_hover_text(band.description())
                        .clicked()
                    {
                        self.rhythm.set_band(band);
                    }
                }
            });
        }

        let pulse = self.rhythm.pulse;
        match self.rhythm.source {
            rhythm::Source::Audio(_) if self.rhythm.input.is_some() => {
                for (label, v) in [("Intensité", pulse.level), ("Graves", pulse.bass), ("Aigus", pulse.high)] {
                    ui.horizontal(|ui| {
                        ui.add_sized([56.0, 14.0], egui::Label::new(egui::RichText::new(label).small().color(TEXT_WEAK)));
                        ui.add(egui::ProgressBar::new(v).desired_height(8.0).fill(ACCENT.gamma_multiply(0.8)));
                    });
                }
            }
            rhythm::Source::Tempo => {
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button(with_icon(ic::WAVES, "TAP"))).on_hover_text("Touche T").clicked() {
                        self.tap();
                    }
                    let mut bpm = self.rhythm.tempo.bpm;
                    if ui.add(egui::DragValue::new(&mut bpm).range(30.0..=300.0).speed(0.2).max_decimals(1).suffix(" BPM")).changed() {
                        self.rhythm.tempo.set_bpm(bpm, self.clock());
                    }
                    if ui.small_button("÷2").clicked() {
                        let b = self.rhythm.tempo.bpm / 2.0;
                        self.rhythm.tempo.set_bpm(b, self.clock());
                    }
                    if ui.small_button("×2").clicked() {
                        let b = self.rhythm.tempo.bpm * 2.0;
                        self.rhythm.tempo.set_bpm(b, self.clock());
                    }
                });
            }
            rhythm::Source::Manual => {
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button(with_icon(ic::LIGHTNING, "PULSE"))).on_hover_text("Touche T").clicked() {
                        self.tap();
                    }
                    ui.label(
                        egui::RichText::new("Une pulsation par appui sur T ; sans appui, plus rien ne bouge.")
                            .small()
                            .color(TEXT_WEAK),
                    );
                });
            }
            _ => {}
        }
        if self.rhythm.active() {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), Sense::hover());
            ui.painter().rect_filled(rect, 3.0, ACCENT.gamma_multiply(0.1 + 0.9 * pulse.env));
        }

    }

    pub(super) fn rhythm_react_ui(&mut self, ui: &mut egui::Ui) {
        let sel = self.sel();
        let react = &mut self.project.layers[sel].react;
        if reset_button(ui, "Ne plus réagir au rythme", "Coupe toutes les réactions au rythme de ce calque", react.enabled) {
            react.enabled = false;
        }
        ui.toggle_value(&mut react.enabled, with_icon(ic::LIGHTNING, "Réagir au rythme"))
            .on_hover_text("Les effets de ce calque bougent avec le son ou le tempo");
        ui.label(egui::RichText::new("Réactions toutes prêtes").small().color(TEXT_WEAK));
        ui.horizontal_wrapped(|ui| {
            for (name, desc, preset) in rhythm::React::presets() {
                let on = react.enabled && react.same_feel(&preset);
                if ui.selectable_label(on, name).on_hover_text(desc).clicked() {
                    *react = rhythm::React { auto_beats: react.auto_beats, mix_beats: react.mix_beats, ..preset };
                }
            }
        });
        ui.add_space(4.0);
        ui.add_enabled_ui(react.enabled, |ui| {
            percent_slider(ui, &mut react.zoom, "Coup de zoom").on_hover_text("Zoom bref sur chaque temps");
            percent_slider(ui, &mut react.flash, "Flash").on_hover_text("Éclair de lumière sur chaque temps");
            percent_slider(ui, &mut react.shake, "Secousse RVB").on_hover_text("Décalage des couleurs sur les temps et les aigus");
            percent_slider(ui, &mut react.drive, "Suit le volume")
                .on_hover_text("Ondes, liquide, tourbillon et saturation s'amplifient avec le volume (avec le tempo : sur les temps)");
            ui.add(
                egui::Slider::new(&mut react.hue, 0.0..=0.5)
                    .text("Saut de teinte")
                    .custom_formatter(|v, _| format!("{:.0}°", v * 360.0)),
            )
            .on_hover_text("La teinte tourne d'autant à chaque temps");
            percent_slider(ui, &mut react.invert, "Négatif").on_hover_text("Flash négatif sur chaque temps");
            percent_slider(ui, &mut react.gate, "Stroboscope")
                .on_hover_text("Noir entre les temps : l'image ne s'allume que sur le rythme");
            percent_slider(ui, &mut react.glitch, "Glitch").on_hover_text("L'image se casse en pixels sur chaque temps");
            percent_slider(ui, &mut react.shock, "Onde de choc").on_hover_text("Ondulation qui part sur chaque temps");
            percent_slider(ui, &mut react.kaleido, "Kaléido").on_hover_text("Le kaléidoscope s'ouvre sur chaque temps");
            ui.add(
                egui::Slider::new(&mut react.spin, 0.0..=1.0)
                    .text("Rotation par temps")
                    .custom_formatter(|v, _| format!("{:.0}°", v * 90.0)),
            )
            .on_hover_text("L'image tourne d'autant à chaque temps, en glissant");
            ui.checkbox(&mut react.flip, "Miroir un temps sur deux");
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Enchaînements sur les temps (0 = désactivé)").small().color(TEXT_WEAK));
            ui.add(egui::Slider::new(&mut react.auto_beats, 0..=32).text("AUTO : effet suivant tous les").suffix(" temps"))
                .on_hover_text("Remplace l'intervalle en secondes du mode AUTO (onglet Effets)");
            ui.add(egui::Slider::new(&mut react.mix_beats, 0..=32).text("Mix auto : coupe tous les").suffix(" temps"))
                .on_hover_text("Remplace les durées de séquence du mix auto (onglet Mix)");
        });
        if self.rhythm.source == rhythm::Source::Manual {
            ui.label(
                egui::RichText::new("Mode manuel : AUTO et mix gardent leur durée en secondes ; T ne pilote que les réactions.")
                    .small()
                    .color(TEXT_WEAK),
            );
        }
        if !self.rhythm.active() {
            ui.label(egui::RichText::new("Choisir une source du rythme (onglet Source) pour activer les réactions.").small().color(WARN));
        }
    }
}
