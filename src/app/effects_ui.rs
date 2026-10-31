//! Onglet Effets : presets, réglages des effets, mode Auto et mix.

use super::*;

impl OzApp {
    pub(super) fn apply_preset(&mut self, name: Option<String>, fx: Effects) {
        let i = self.sel();
        self.apply_preset_to(i, name, fx, self.project.preset_secs);
        // Un choix manuel reste affiché un intervalle complet avant que le mode Auto ne reprenne la main.
        self.layers[i].auto_timer = 0.0;
    }

    /// Effets visés par le calque `i` (la cible d'une transition en cours, sinon les effets actuels).
    pub(super) fn fx_target(&self, i: usize) -> (Effects, Option<String>) {
        let rt = &self.layers[i];
        let fx = rt.fx_anim.as_ref().map_or(self.project.layers[i].effects, |a| a.to);
        (fx, rt.preset.clone())
    }

    /// Bandeau : l'effet choisi ne se voit pas, proposer de l'appliquer sur un calque visible.
    pub(super) fn hidden_fx_banner(&mut self, ui: &mut egui::Ui) {
        let sel = self.sel();
        if self.fx_hidden_alert != Some(sel) {
            self.fx_hidden_alert = None;
            return;
        }
        let Some(reason) = self.hidden_reason(sel) else {
            // Le calque est redevenu visible : plus rien à signaler.
            self.fx_hidden_alert = None;
            return;
        };
        // Calques visibles, du premier plan vers le fond (comme la liste).
        let visible: Vec<usize> = (0..self.project.layers.len())
            .rev()
            .filter(|&j| j != sel && self.hidden_reason(j).is_none())
            .collect();
        let can_show = !self.project.layers[sel].visible || self.project.layers[sel].opacity <= 0.0;
        let can_raise = !can_show && self.has_content(sel) && sel + 1 < self.project.layers.len();
        let mut raise = false;
        let mut target = None;
        let mut show = false;
        let mut dismiss = false;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 6))
            .corner_radius(6.0)
            .fill(WARN.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0, WARN.gamma_multiply(0.6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(ic::WARNING).size(16.0).color(WARN));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "L'effet ne se voit pas : le calque « {} » n'est pas visible à l'écran ({reason}).",
                                self.project.layers[sel].name
                            ))
                            .color(TEXT),
                        )
                        .wrap(),
                    );
                });
                if visible.is_empty() {
                    ui.label(
                        egui::RichText::new("Aucun autre calque n'est visible pour l'instant.").small().color(TEXT_WEAK),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("Sélectionner un calque visible et y appliquer l'effet :")
                            .small()
                            .color(TEXT_WEAK),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for &j in &visible {
                            if ui
                                .button(with_icon(ic::ARROW_RIGHT, &self.project.layers[j].name))
                                .on_hover_text("Sélectionner ce calque et lui appliquer les mêmes effets")
                                .clicked()
                            {
                                target = Some(j);
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if can_show
                        && ui
                            .button(with_icon(ic::EYE, "Afficher ce calque"))
                            .on_hover_text("Rendre visible le calque sélectionné")
                            .clicked()
                    {
                        show = true;
                    }
                    if can_raise
                        && ui
                            .button(with_icon(ic::CARET_UP, "Passer au premier plan"))
                            .on_hover_text("Placer le calque sélectionné devant tous les autres")
                            .clicked()
                    {
                        raise = true;
                    }
                    if ui.button(with_icon(ic::X, "Ignorer")).clicked() {
                        dismiss = true;
                    }
                });
            });
        ui.add_space(4.0);
        if let Some(j) = target {
            let (fx, name) = self.fx_target(sel);
            self.apply_preset_to(j, name, fx, self.project.preset_secs);
            self.layers[j].auto_timer = 0.0;
            self.selected = j;
            self.fx_hidden_alert = None;
        } else if show {
            let cfg = &mut self.project.layers[sel];
            cfg.visible = true;
            if cfg.opacity <= 0.0 {
                cfg.opacity = 1.0;
            }
            self.fx_hidden_alert = None;
        } else if raise {
            for k in sel..self.project.layers.len() - 1 {
                self.move_layer(k, true);
            }
            self.fx_hidden_alert = None;
        } else if dismiss {
            self.fx_hidden_alert = None;
        }
    }

    /// Retour à l'image brute sur le calque sélectionné : plus d'effets et mode AUTO coupé.
    pub(super) fn clear_effects(&mut self) {
        let i = self.sel();
        self.project.layers[i].auto.enabled = false;
        // Immédiat et complet : effets neutres et animations (rotation, teinte…) remises à zéro,
        // pour retrouver tout de suite l'image d'origine, dans le bon sens.
        self.apply_preset_to(i, None, Effects::default(), 0.0);
        let shape = self.layers[i].phases;
        self.layers[i].phases = Phases {
            shape_ripple: shape.shape_ripple,
            shape_trace: shape.shape_trace,
            shape_breathe: shape.shape_breathe,
            shatter: shape.shatter,
            mask_spin: shape.mask_spin,
            ..Phases::default()
        };
        self.layers[i].auto_timer = 0.0;
    }

    /// Applique des effets au calque `i`, sans toucher à ses animations de forme du masque.
    pub(super) fn apply_preset_to(&mut self, i: usize, name: Option<String>, fx: Effects, secs: f32) {
        let fx = fx.with_shape_of(&self.project.layers[i].effects);
        self.layers[i].preset = name;
        if secs <= 0.01 {
            self.project.layers[i].effects = fx;
            self.layers[i].fx_anim = None;
        } else {
            self.layers[i].fx_anim = Some(FxAnim {
                from: self.project.layers[i].effects,
                to: fx,
                t: 0.0,
                secs,
            });
        }
    }

    pub(super) fn effects_ui(&mut self, ui: &mut egui::Ui) {
        let sec = self.section();
        // En tête de toutes les sections : retour à l'image d'origine.
        let sel = self.sel();
        let fx_now = self.project.layers[sel].effects;
        let has_fx = Effects::default().with_shape_of(&fx_now) != fx_now || self.project.layers[sel].auto.enabled;
        if reset_button(
            ui,
            "Aucun effet",
            "Revenir à l'image d'origine : retire tous les effets du calque et coupe le mode AUTO (0)",
            has_fx,
        ) {
            self.clear_effects();
        }
        self.presets_ui(ui);

        let i = self.sel();
        let fx = &mut self.project.layers[i].effects;
        // Vrai seulement si l'utilisateur a touché un réglage (pas sur une simple mise à jour).
        let mut touched = false;
        let d = Effects::default();
        sec.show(ui, 1, |ui| {
            touched |= fx_slider(ui, "Luminosité", &mut fx.brightness, -1.0..=1.0, d.brightness);
            touched |= fx_slider(ui, "Contraste", &mut fx.contrast, 0.0..=3.0, d.contrast);
            touched |= fx_slider(ui, "Saturation", &mut fx.saturation, 0.0..=3.0, d.saturation);
            touched |= fx_slider(ui, "Teinte", &mut fx.hue, 0.0..=1.0, d.hue);
            touched |= fx_slider(ui, "Cycle teinte /s", &mut fx.hue_speed, -1.0..=1.0, d.hue_speed);
            touched |= fx_slider(ui, "Négatif", &mut fx.invert, 0.0..=1.0, d.invert);
            touched |= fx_slider(ui, "Postérisation", &mut fx.posterize, 0.0..=16.0, d.posterize);
            ui.horizontal(|ui| {
                // Copie locale : le sélecteur réécrit sinon la couleur arrondie à chaque image.
                let mut tint = fx.tint;
                if ui.color_edit_button_rgb(&mut tint).changed() {
                    fx.tint = tint;
                    touched = true;
                }
                touched |= fx_slider(ui, "Colorisation", &mut fx.tint_amount, 0.0..=1.0, d.tint_amount);
            });
            touched |= fx_slider(ui, "Contours", &mut fx.edge, 0.0..=1.0, d.edge);
            touched |= fx_slider(ui, "Vignette", &mut fx.vignette, 0.0..=1.0, d.vignette);
            touched |= fx_slider(ui, "Caméra thermique", &mut fx.thermal, 0.0..=1.0, d.thermal);
            touched |= fx_slider(ui, "Flou", &mut fx.blur, 0.0..=1.0, d.blur);
            touched |= fx_slider(ui, "Halo lumineux", &mut fx.bloom, 0.0..=1.5, d.bloom);
        });
        sec.show(ui, 2, |ui| {
            touched |= fx_slider(ui, "Zoom", &mut fx.zoom, 0.2..=5.0, d.zoom);
            touched |= fx_slider(ui, "Rotation °", &mut fx.rotate, -180.0..=180.0, d.rotate);
            touched |= fx_slider(ui, "Rotation auto °/s", &mut fx.rotate_speed, -360.0..=360.0, d.rotate_speed);
            ui.horizontal(|ui| {
                touched |= ui.checkbox(&mut fx.flip_x, "Retourner H").changed();
                touched |= ui.checkbox(&mut fx.flip_y, "Retourner V").changed();
            });
            egui::ComboBox::from_label("Miroir")
                .selected_text(fx.mirror.label())
                .show_ui(ui, |ui| {
                    for m in Mirror::ALL {
                        touched |= ui.selectable_value(&mut fx.mirror, m, m.label()).changed();
                    }
                });
            touched |= fx_slider(ui, "Kaléidoscope", &mut fx.kaleido, 0.0..=16.0, d.kaleido);
            touched |= fx_slider(ui, "Rotation kaléido °/s", &mut fx.kaleido_spin, -180.0..=180.0, d.kaleido_spin);
        });
        sec.show(ui, 3, |ui| {
            touched |= fx_slider(ui, "Pixelisation", &mut fx.pixelate, 0.0..=1.0, d.pixelate);
            touched |= fx_slider(ui, "Décalage RVB", &mut fx.rgb_shift, 0.0..=0.05, d.rgb_shift);
            touched |= fx_slider(ui, "Ondulation", &mut fx.wave_amp, 0.0..=0.1, d.wave_amp);
            touched |= fx_slider(ui, "Fréquence onde", &mut fx.wave_freq, 0.5..=30.0, d.wave_freq);
            touched |= fx_slider(ui, "Vitesse onde", &mut fx.wave_speed, 0.0..=20.0, d.wave_speed);
            touched |= fx_slider(ui, "Fish-eye", &mut fx.fisheye, -1.5..=1.5, d.fisheye);
            touched |= fx_slider(ui, "Ondes de choc", &mut fx.shock, 0.0..=1.0, d.shock);
            touched |= fx_slider(ui, "Ondes de choc /s", &mut fx.shock_speed, 0.05..=3.0, d.shock_speed);
            touched |= fx_slider(ui, "Mosaïque hexagonale", &mut fx.hexagon, 0.0..=1.0, d.hexagon);
            touched |= fx_slider(ui, "Droste (spirale infinie)", &mut fx.droste, 0.0..=1.0, d.droste);
            touched |= fx_slider(ui, "Zoom Droste /s", &mut fx.droste_speed, -1.5..=1.5, d.droste_speed);
            touched |= fx_slider(ui, "Flou de zoom", &mut fx.zoom_blur, 0.0..=1.0, d.zoom_blur);
        });
        sec.show(ui, 4, |ui| {
            touched |= fx_slider(ui, "Tourbillon", &mut fx.swirl, -4.0..=4.0, d.swirl);
            touched |= fx_slider(ui, "Oscillation tourbillon /s", &mut fx.swirl_speed, 0.0..=1.0, d.swirl_speed);
            touched |= fx_slider(ui, "Tunnel", &mut fx.polar, 0.0..=1.0, d.polar);
            touched |= fx_slider(ui, "Vitesse tunnel", &mut fx.tunnel_speed, -2.0..=2.0, d.tunnel_speed);
            touched |= fx_slider(ui, "Mosaïque miroir", &mut fx.tiles, 1.0..=8.0, d.tiles);
            touched |= fx_slider(ui, "Liquide", &mut fx.liquid, 0.0..=0.08, d.liquid);
            touched |= fx_slider(ui, "Échelle liquide", &mut fx.liquid_scale, 1.0..=20.0, d.liquid_scale);
            touched |= fx_slider(ui, "Vitesse liquide", &mut fx.liquid_speed, 0.0..=8.0, d.liquid_speed);
            touched |= fx_slider(ui, "Palette acide", &mut fx.acid, 0.0..=1.0, d.acid);
            touched |= fx_slider(ui, "Rotation acide /s", &mut fx.acid_speed, -1.0..=1.0, d.acid_speed);
            touched |= fx_slider(ui, "Arc-en-ciel spatial", &mut fx.rainbow, 0.0..=3.0, d.rainbow);
            touched |= fx_slider(ui, "Solarisation", &mut fx.solarize, 0.0..=1.0, d.solarize);
            touched |= fx_slider(ui, "Pulsation", &mut fx.pulse, 0.0..=0.3, d.pulse);
            touched |= fx_slider(ui, "Fréquence pulsation Hz", &mut fx.pulse_rate, 0.05..=5.0, d.pulse_rate);
            touched |= fx_slider(ui, "Écho", &mut fx.echo, 0.0..=1.0, d.echo);
        });
        sec.show(ui, 5, |ui| {
            touched |= fx_slider(ui, "Télé cathodique", &mut fx.crt, 0.0..=1.0, d.crt);
            touched |= fx_slider(ui, "Glitch numérique", &mut fx.glitch, 0.0..=1.0, d.glitch);
            touched |= fx_slider(ui, "Grain de pellicule", &mut fx.grain, 0.0..=1.0, d.grain);
            touched |= fx_slider(ui, "Trame BD", &mut fx.halftone, 0.0..=1.0, d.halftone);
            touched |= fx_slider(ui, "ASCII (taille des caractères)", &mut fx.ascii, 0.0..=1.0, d.ascii);
        });
        sec.show(ui, 6, |ui| {
            ui.label(egui::RichText::new("Objet").strong().color(TEXT));
            ui.horizontal_wrapped(|ui| {
                for o in Object3d::ALL {
                    touched |= ui.selectable_value(&mut fx.obj3d, o, o.label()).on_hover_text(o.description()).changed();
                }
            });
            ui.add_enabled_ui(fx.obj3d != Object3d::Off, |ui| {
                touched |= fx_slider(ui, "Inclinaison °", &mut fx.rot3d_x, -180.0..=180.0, d.rot3d_x);
                touched |= fx_slider(ui, "Rotation °", &mut fx.rot3d_y, -180.0..=180.0, d.rot3d_y);
                touched |= fx_slider(ui, "Rotation auto °/s", &mut fx.spin3d_y, -180.0..=180.0, d.spin3d_y);
                touched |= fx_slider(ui, "Bascule auto °/s", &mut fx.spin3d_x, -180.0..=180.0, d.spin3d_x);
                touched |= fx_slider(ui, "Distance caméra", &mut fx.dist3d, 1.0..=8.0, d.dist3d);
                if fx.obj3d.uses_height() {
                    touched |= fx_slider(ui, "Hauteur du relief", &mut fx.height3d, 0.0..=1.0, d.height3d);
                }
                if fx.obj3d == Object3d::Voxels {
                    touched |= fx_slider(ui, "Colonnes (par hauteur)", &mut fx.voxels, 8.0..=120.0, d.voxels);
                }
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Volume").strong().color(TEXT));
            touched |= fx_slider(ui, "Relief", &mut fx.relief, 0.0..=1.5, d.relief);
            touched |= fx_slider(ui, "Anaglyphe (lunettes 3D)", &mut fx.anaglyph, 0.0..=0.04, d.anaglyph);
            touched |= fx_slider(ui, "Drapeau", &mut fx.flag, 0.0..=1.5, d.flag);
            touched |= fx_slider(ui, "Vitesse drapeau", &mut fx.flag_speed, 0.0..=8.0, d.flag_speed);
            touched |= fx_slider(ui, "Accordéon", &mut fx.fold, 0.0..=1.0, d.fold);
            touched |= fx_slider(ui, "Nombre de plis", &mut fx.fold_count, 2.0..=16.0, d.fold_count);
        });
        sec.show(ui, 7, |ui| {
            touched |= fx_slider(ui, "Stroboscope Hz", &mut fx.strobe, 0.0..=20.0, d.strobe);
        });
        // Un réglage manuel interrompt une transition de preset en cours.
        if touched {
            self.layers[i].fx_anim = None;
            self.layers[i].preset = None;
        }
    }

    pub(super) fn presets_ui(&mut self, ui: &mut egui::Ui) {
        let sec = self.section();
        let sel = self.sel();
        let current = self.project.layers[sel].effects;
        let active = self.layers[sel].preset.clone();
        // Surligné : le preset appliqué en dernier sur ce calque, sinon celui qui correspond aux réglages.
        let is_active = |p: &Preset| match &active {
            Some(name) => *name == p.name,
            None => p.effects.with_shape_of(&current) == current,
        };
        let mut apply: Option<(Option<String>, Effects)> = None;
        let mut auto_next = false;
        sec.show(ui, 0, |ui| {
            ui.horizontal_wrapped(|ui| {
                for p in &self.builtin_presets {
                    if ui.selectable_label(is_active(p), &p.name).clicked() {
                        apply = Some((Some(p.name.clone()), p.effects));
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .button(with_icon(ic::SHUFFLE, "Aléatoire"))
                    .on_hover_text("Combinaison d'effets au hasard")
                    .clicked()
                {
                    apply = Some((None, Effects::random(self.next_random())));
                }
                ui.label(egui::RichText::new(ic::TIMER).color(TEXT_WEAK)).on_hover_text("Durée de transition entre presets");
                ui.add(egui::Slider::new(&mut self.project.preset_secs, 0.0..=3.0).suffix(" s"));
            });

            ui.add_space(4.0);
            auto_ui(
                ui,
                &mut self.project.layers[sel].auto,
                self.layers[sel].auto_timer,
                active.as_deref(),
                &mut auto_next,
            );

            ui.add_space(4.0);
            ui.label(egui::RichText::new(with_icon(ic::BOOKMARK_SIMPLE, "Mes presets")).strong().color(TEXT));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.preset_name).hint_text("Nom").desired_width(130.0));
                if ui
                    .button(with_icon(ic::FLOPPY_DISK, "Mémoriser"))
                    .on_hover_text("Enregistrer les effets actuels comme preset")
                    .clicked()
                {
                    let name = if self.preset_name.trim().is_empty() {
                        format!("Preset {}", self.project.presets.len() + 1)
                    } else {
                        self.preset_name.trim().to_owned()
                    };
                    self.project.presets.push(Preset { name: name.clone(), effects: current });
                    self.layers[sel].preset = Some(name);
                    self.preset_name.clear();
                }
            });
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                if self.project.presets.is_empty() {
                    ui.label(egui::RichText::new("Réglez des effets puis « Mémoriser ».").small().color(TEXT_WEAK));
                }
                for (k, p) in self.project.presets.iter().enumerate() {
                    let resp = ui
                        .selectable_label(is_active(p), &p.name)
                        .on_hover_text("Clic : appliquer · Clic droit : supprimer");
                    if resp.clicked() {
                        apply = Some((Some(p.name.clone()), p.effects));
                    }
                    if resp.secondary_clicked() {
                        remove = Some(k);
                    }
                }
            });
            if let Some(k) = remove {
                self.project.presets.remove(k);
            }
        });
        if let Some((name, fx)) = apply {
            self.apply_preset(name, fx);
        }
        if auto_next {
            self.auto_next(sel);
        }
    }

    pub(super) fn mix_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let sel = self.sel();
        let (timer, len, seg) = {
            let rt = &self.layers[sel];
            (rt.mix_timer, rt.mix_len, rt.mix_seg.clone())
        };
        let media = self.project.media.clone();
        let mut next = false;
        let mut started = false;
        let mix = &mut self.project.layers[sel].mix;

        if reset_button(ui, "Arrêter le mix", "Coupe le mix auto : la vidéo en cours continue normalement", mix.enabled) {
            mix.enabled = false;
        }
        ui.horizontal(|ui| {
            let label = egui::RichText::new(with_icon(ic::FILM_REEL, "MIX AUTO")).strong();
            if ui
                .toggle_value(&mut mix.enabled, label)
                .on_hover_text("Enchaîne au hasard des séquences des vidéos cochées sur ce calque")
                .changed()
                && mix.enabled
            {
                started = true;
            }
            if ui
                .add(theme::icon_button(ic::SKIP_FORWARD))
                .on_hover_text("Séquence suivante tout de suite")
                .clicked()
            {
                next = true;
            }
        });
        ui.label(
            egui::RichText::new(
                "Découpe des séquences dans les vidéos cochées : changements de vidéo, sauts dans le temps, \
                 lecture à l'envers, vitesse variable, coupes franches ou fondus.",
            )
            .small()
            .color(TEXT_WEAK),
        );
        if mix.enabled {
            ui.add(
                egui::ProgressBar::new((timer / len.max(0.01)).clamp(0.0, 1.0))
                    .desired_height(6.0)
                    .fill(ACCENT.gamma_multiply(0.7)),
            );
            if let Some(seg) = &seg {
                let dir = if seg.reverse { with_icon(ic::REWIND, "envers") } else { with_icon(ic::PLAY, "endroit") };
                let cut = if seg.fade > 0.0 { format!("fondu {:.1} s", seg.fade) } else { "coupe".into() };
                ui.label(
                    egui::RichText::new(format!(
                        "{}  ·  {dir}  ·  ×{:.2}  ·  {cut}  ·  {:.1} s",
                        file_name(&seg.clip),
                        seg.speed,
                        seg.length
                    ))
                    .small()
                    .color(TEXT_WEAK),
                );
            }
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("Vidéos du mix : {}", mix.clips.len())).strong().color(TEXT));
            if ui.small_button("Toutes").clicked() {
                mix.clips = media.clone();
            }
            if ui.small_button("Aucune").clicked() {
                mix.clips.clear();
            }
        });
        if media.is_empty() {
            ui.label(egui::RichText::new("Ajoutez d'abord des vidéos dans la liste Médias.").small().color(TEXT_WEAK));
        }
        for path in &media {
            let mut on = mix.clips.contains(path);
            let icon = if on { ic::CHECK_SQUARE } else { ic::SQUARE };
            let text = egui::RichText::new(with_icon(icon, &file_name(path))).color(if on { ACCENT } else { TEXT });
            if ui.add(egui::Button::new(text).frame(false)).clicked() {
                on = !on;
                if on {
                    mix.clips.push(path.clone());
                } else {
                    mix.clips.retain(|p| p != path);
                }
            }
        }
        if mix.enabled && mix.clips.is_empty() {
            ui.label(egui::RichText::new("Cochez au moins une vidéo.").color(WARN));
        }

        ui.add_space(6.0);
        ui.label(egui::RichText::new(with_icon(ic::SCISSORS, "Séquences")).strong().color(TEXT));
        ui.add(egui::Slider::new(&mut mix.seg_min, 0.3..=20.0).logarithmic(true).text("Durée min").suffix(" s"));
        ui.add(egui::Slider::new(&mut mix.seg_max, 0.3..=20.0).logarithmic(true).text("Durée max").suffix(" s"));
        percent_slider(ui, &mut mix.same_clip_chance, "Rester sur la même vidéo")
            .on_hover_text("Sinon on passe à une autre vidéo cochée");
        ui.add_space(4.0);
        ui.label(egui::RichText::new(with_icon(ic::REWIND, "Sens et vitesse")).strong().color(TEXT));
        percent_slider(ui, &mut mix.reverse_chance, "À l'envers");
        percent_slider(ui, &mut mix.speed_chance, "Vitesse variable")
            .on_hover_text("Part des séquences jouées à une autre vitesse que la normale");
        ui.add(egui::Slider::new(&mut mix.speed_min, 0.1..=4.0).logarithmic(true).text("Vitesse min").suffix("×"));
        ui.add(egui::Slider::new(&mut mix.speed_max, 0.1..=4.0).logarithmic(true).text("Vitesse max").suffix("×"));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(with_icon(ic::SHUFFLE, "Transitions")).strong().color(TEXT));
        percent_slider(ui, &mut mix.cut_chance, "Coupes franches").on_hover_text("Sinon fondu enchaîné");
        ui.add(egui::Slider::new(&mut mix.fade, 0.05..=3.0).text("Durée des fondus").suffix(" s"));
        if ui.small_button("Réglages par défaut").clicked() {
            let (enabled, clips) = (mix.enabled, std::mem::take(&mut mix.clips));
            *mix = AutoMix { enabled, clips, ..AutoMix::default() };
        }

        if started {
            // Lit à l'avance la durée des vidéos, pour que les changements soient instantanés.
            let clips = mix.clips.clone();
            std::thread::spawn(move || {
                for c in clips {
                    let _ = video::probe(&c);
                }
            });
        }
        if (started || next) && !mix.clips.is_empty() {
            self.mix_next(&ctx, sel);
        }
    }
}

/// Réglages du mode Auto. `next` passe à true si un changement immédiat est demandé.
pub(super) fn auto_ui(ui: &mut egui::Ui, auto: &mut effects::AutoFx, timer: f32, current: Option<&str>, next: &mut bool) {
    ui.horizontal(|ui| {
        let label = egui::RichText::new(with_icon(ic::MAGIC_WAND, "AUTO")).strong();
        if ui
            .toggle_value(&mut auto.enabled, label)
            .on_hover_text("Enchaîne des effets au hasard sur ce calque (A)")
            .changed()
            && auto.enabled
        {
            *next = true;
        }
        ui.label(egui::RichText::new("Navigation automatique").color(TEXT_WEAK));
        if ui
            .add(theme::icon_button(ic::SKIP_FORWARD))
            .on_hover_text("Passer tout de suite à un autre effet")
            .clicked()
        {
            *next = true;
        }
    });
    if auto.enabled {
        let interval = auto.interval.max(0.5);
        let left = (interval - timer).max(0.0);
        ui.add(
            egui::ProgressBar::new((timer / interval).clamp(0.0, 1.0))
                .desired_height(6.0)
                .fill(ACCENT.gamma_multiply(0.7)),
        );
        let now = current.unwrap_or("effet inédit (« Mémoriser » pour le garder)");
        ui.label(egui::RichText::new(format!("En cours : {now}  ·  prochain dans {left:.0} s")).small().color(TEXT_WEAK));
    }
    ui.horizontal(|ui| {
        ui.label("Thème");
        let before = auto.theme;
        egui::ComboBox::from_id_salt("auto_theme")
            .selected_text(auto.theme.label())
            .width(140.0)
            .show_ui(ui, |ui| {
                for t in AutoTheme::ALL {
                    ui.selectable_value(&mut auto.theme, t, t.label()).on_hover_text(t.description());
                }
            });
        // Changer de thème en cours de route : effet du nouveau thème tout de suite.
        if auto.enabled && auto.theme != before {
            *next = true;
        }
    });
    ui.label(egui::RichText::new(auto.theme.description()).small().color(TEXT_WEAK));
    ui.add(egui::Slider::new(&mut auto.interval, 1.0..=60.0).text("Changer toutes les").suffix(" s"));
    ui.add(egui::Slider::new(&mut auto.transition, 0.0..=10.0).text("Fondu").suffix(" s"));
    ui.add(egui::Slider::new(&mut auto.intensity, 0.3..=1.5).text("Intensité"))
        .on_hover_text("Dosage des effets générés : faible = subtil, fort = extrême");
    if auto.theme == AutoTheme::All {
        percent_slider(ui, &mut auto.novelty, "Effets inédits")
            .on_hover_text("Part des changements qui inventent un nouvel effet au lieu de reprendre un preset");
    }
    ui.checkbox(&mut auto.strobe, "Autoriser le stroboscope").on_hover_text("Inclut les presets avec flashs");
}
