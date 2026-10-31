//! Onglet Visuels : générateur de motifs, visuels sur le masque, destruction.

use super::*;

impl OzApp {
    /// Onglet Rythme : source (son ou tempo) et réaction du calque sélectionné.
    pub(super) fn generator_ui(&mut self, ui: &mut egui::Ui) {
        let sec = self.section();
        let sel = self.sel();
        let has_video = self.layers[sel].active().is_some();
        let timer = self.layers[sel].gen_timer;
        let mut next = false;
        let g = &mut self.project.layers[sel].generator;
        let d = crate::generator::Generator::default();
        if sec.0 < 4
            && reset_button(
                ui,
                "Arrêter le visuel",
                "Plus de visuel généré : le calque affiche sa vidéo",
                g.is_active(),
            )
        {
            g.pattern = Pattern::Off;
            g.auto = false;
        }
        if sec.0 < 4 {
            ui.label(
                egui::RichText::new(
                    "Visuels psychédéliques générés en direct, sans vidéo. Les effets, le masque et la déformation \
                     s'appliquent dessus comme sur une vidéo : essayez-les avec le kaléidoscope ou le tourbillon.",
                )
                .small()
                .color(TEXT_WEAK),
            );
        }
        sec.show(ui, 0, |ui| {
            ui.horizontal_wrapped(|ui| {
                for p in Pattern::ALL.into_iter().filter(|p| *p != Pattern::Off) {
                    let resp = ui.selectable_label(g.pattern == p, p.label()).on_hover_text(p.description());
                    if resp.clicked() {
                        g.pattern = p;
                    }
                }
            });
        });
        sec.show(ui, 1, |ui| {
            ui.horizontal_wrapped(|ui| {
                for p in Palette::ALL {
                    if ui.selectable_label(g.palette == p, p.label()).clicked() {
                        g.palette = p;
                    }
                }
            });
            fx_slider(ui, "Décalage des couleurs", &mut g.hue, 0.0..=1.0, d.hue);
            fx_slider(ui, "Rotation des couleurs /s", &mut g.color_speed, -1.0..=1.0, d.color_speed);
        });
        sec.show(ui, 2, |ui| {
            fx_slider(ui, "Vitesse", &mut g.speed, -3.0..=3.0, d.speed);
            fx_slider(ui, "Taille du motif", &mut g.scale, 0.2..=5.0, d.scale);
            fx_slider(ui, "Détail", &mut g.detail, 0.0..=1.0, d.detail);
            ui.add_enabled_ui(has_video, |ui| {
                fx_slider(ui, "Mélange avec la vidéo", &mut g.mix, 0.0..=1.0, d.mix);
            })
            .response
            .on_disabled_hover_text("Sans vidéo sur ce calque, le visuel est affiché entier");
        });
        sec.show(ui, 3, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut g.auto, "Changer de motif automatiquement");
                if ui
                    .button(with_icon(ic::DICE_FIVE, "Au hasard"))
                    .on_hover_text("Passer tout de suite à un autre motif et une autre palette")
                    .clicked()
                {
                    next = true;
                }
            });
            ui.add(egui::Slider::new(&mut g.interval, 2.0..=120.0).text("Durée d'un motif").suffix(" s"));
            ui.add(egui::Slider::new(&mut g.transition, 0.0..=10.0).text("Fondu").suffix(" s"));
            if g.auto && g.is_active() {
                ui.add(egui::ProgressBar::new((timer / g.interval.max(1.0)).clamp(0.0, 1.0)).desired_height(4.0));
            }
        });
        if next {
            self.generator_next(sel);
        }
        self.shape_fx_ui(ui);
    }

    /// Contours animés de l'image du calque (idéal pour une photo).
    pub(super) fn lines_fx_ui(&mut self, ui: &mut egui::Ui) {
        let i = self.sel();
        let fx = &mut self.project.layers[i].effects;
        let d = Effects::default();
        // « Aucun » est le dernier style de la liste : il devient le gros bouton du haut.
        let none = Effects::LINE_STYLES.len() - 1;
        let mut cleared = *fx;
        cleared.set_lines_style(none);
        if reset_button(ui, "Aucun contour animé", Effects::LINE_STYLES[none].1, cleared != *fx) {
            *fx = cleared;
        }
        ui.scope(|ui| {
            ui.label(
                egui::RichText::new(
                    "Les contours de l'image s'illuminent et bougent : idéal pour donner vie à une photo, \
                     marche aussi sur une vidéo ou un visuel.",
                )
                .small()
                .color(TEXT_WEAK),
            );
            ui.horizontal_wrapped(|ui| {
                for (k, (name, desc)) in Effects::LINE_STYLES.iter().enumerate().take(none) {
                    if ui.button(*name).on_hover_text(*desc).clicked() {
                        fx.set_lines_style(k);
                    }
                }
            });
            ui.label(egui::RichText::new("Lumière").strong().color(TEXT));
            ui.horizontal(|ui| {
                let mut color = fx.lines_color;
                if ui.color_edit_button_rgb(&mut color).on_hover_text("Couleur des contours").changed() {
                    fx.lines_color = color;
                }
                fx_slider(ui, "Arc-en-ciel", &mut fx.lines_rainbow, 0.0..=1.0, d.lines_rainbow);
            });
            fx_slider(ui, "Illumination", &mut fx.lines_glow, 0.0..=3.0, d.lines_glow);
            fx_slider(ui, "Seuil (moins de contours)", &mut fx.lines_threshold, 0.0..=1.0, d.lines_threshold);
            fx_slider(ui, "Halo px", &mut fx.lines_width, 1.0..=16.0, d.lines_width);
            fx_slider(ui, "Assombrir l'image", &mut fx.lines_dim, 0.0..=1.0, d.lines_dim);
            ui.label(egui::RichText::new("Mouvement").strong().color(TEXT));
            fx_slider(ui, "Vagues de lumière", &mut fx.lines_flow, 0.0..=1.0, d.lines_flow);
            fx_slider(ui, "Vitesse vagues /s", &mut fx.lines_flow_speed, -2.0..=2.0, d.lines_flow_speed);
            fx_slider(ui, "Ondulation", &mut fx.lines_wobble, 0.0..=0.02, d.lines_wobble);
            fx_slider(ui, "Vitesse ondulation /s", &mut fx.lines_wobble_speed, 0.0..=3.0, d.lines_wobble_speed);
        });
    }

    /// Animations qui suivent le contour du masque (réglages d'effets du calque).
    pub(super) fn shape_fx_ui(&mut self, ui: &mut egui::Ui) {
        let sec = self.section();
        let i = self.sel();
        let has_mask = self.project.layers[i].mask.is_active();
        let fx = &mut self.project.layers[i].effects;
        let d = Effects::default();
        let builtin = effects::mask_anim_presets();
        let user = &mut self.project.mask_presets;
        let preset_name = &mut self.mask_preset_name;
        sec.show(ui, 4, |ui| {
            if reset_button(
                ui,
                "Aucune animation de forme",
                "Retirer les animations sur la forme du masque",
                fx.with_mask_anim_of(&d) != *fx,
            ) {
                *fx = fx.with_mask_anim_of(&d);
            }
            ui.label(egui::RichText::new(with_icon(ic::STAR, "Presets")).strong().color(TEXT));
            ui.horizontal_wrapped(|ui| {
                for p in &builtin {
                    let on = fx.uses_shape() && fx.same_mask_anim(&p.effects);
                    if ui.selectable_label(on, &p.name).clicked() {
                        *fx = fx.with_mask_anim_of(&p.effects);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(preset_name).hint_text("Nom").desired_width(130.0));
                if ui
                    .button(with_icon(ic::FLOPPY_DISK, "Mémoriser"))
                    .on_hover_text("Enregistrer les animations de forme actuelles comme preset")
                    .clicked()
                {
                    let name = if preset_name.trim().is_empty() {
                        format!("Forme {}", user.len() + 1)
                    } else {
                        preset_name.trim().to_owned()
                    };
                    user.push(Preset { name, effects: d.with_mask_anim_of(fx) });
                    preset_name.clear();
                }
            });
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                for (k, p) in user.iter().enumerate() {
                    let on = fx.uses_shape() && fx.same_mask_anim(&p.effects);
                    let resp = ui.selectable_label(on, &p.name).on_hover_text("Clic : appliquer · Clic droit : supprimer");
                    if resp.clicked() {
                        *fx = fx.with_mask_anim_of(&p.effects);
                    }
                    if resp.secondary_clicked() {
                        remove = Some(k);
                    }
                }
            });
            if let Some(k) = remove {
                user.remove(k);
            }
            ui.separator();
            if !has_mask {
                ui.label(
                    egui::RichText::new(
                        "Ces animations suivent le contour du masque : activez un masque (onglet Forme) pour les voir.",
                    )
                    .small()
                    .color(TEXT_WEAK),
                );
            }
            ui.label(egui::RichText::new("Lumières").strong().color(TEXT));
            ui.horizontal(|ui| {
                let mut color = fx.shape_color;
                if ui.color_edit_button_rgb(&mut color).on_hover_text("Couleur des lumières").changed() {
                    fx.shape_color = color;
                }
                fx_slider(ui, "Arc-en-ciel", &mut fx.shape_rainbow, 0.0..=1.0, d.shape_rainbow);
            });
            fx_slider(ui, "Néon contour", &mut fx.shape_glow, 0.0..=2.0, d.shape_glow);
            fx_slider(ui, "Largeur néon", &mut fx.shape_width, 0.001..=0.05, d.shape_width);
            fx_slider(ui, "Lueur intérieure", &mut fx.shape_inner, 0.0..=1.5, d.shape_inner);

            ui.label(egui::RichText::new("Le long du contour").strong().color(TEXT));
            fx_slider(ui, "Comètes", &mut fx.shape_trace, 0.0..=2.0, d.shape_trace);
            fx_slider(ui, "Nombre de comètes", &mut fx.shape_trace_count, 1.0..=12.0, d.shape_trace_count);
            fx_slider(ui, "Pointillés", &mut fx.shape_dashes, 0.0..=2.0, d.shape_dashes);
            fx_slider(ui, "Nombre de pointillés", &mut fx.shape_dash_count, 4.0..=120.0, d.shape_dash_count);
            fx_slider(ui, "Vitesse tours/s", &mut fx.shape_trace_speed, -1.0..=1.0, d.shape_trace_speed);

            ui.label(egui::RichText::new("Ondes").strong().color(TEXT));
            fx_slider(ui, "Lignes concentriques", &mut fx.shape_ripple, 0.0..=1.0, d.shape_ripple);
            fx_slider(ui, "Échos extérieurs", &mut fx.shape_echo, 0.0..=1.5, d.shape_echo);
            fx_slider(ui, "Onde de forme", &mut fx.shape_wave, 0.0..=0.05, d.shape_wave);
            fx_slider(ui, "Densité des lignes", &mut fx.shape_ripple_freq, 2.0..=60.0, d.shape_ripple_freq);
            fx_slider(ui, "Vitesse lignes /s", &mut fx.shape_ripple_speed, -3.0..=3.0, d.shape_ripple_speed);

            ui.label(egui::RichText::new("Mouvement").strong().color(TEXT));
            fx_slider(ui, "Rotation °", &mut fx.mask_rotate, -180.0..=180.0, d.mask_rotate);
            fx_slider(ui, "Rotation auto °/s", &mut fx.mask_spin, -360.0..=360.0, d.mask_spin);
            fx_slider(ui, "Respiration", &mut fx.shape_breathe, -0.15..=0.15, d.shape_breathe);
            fx_slider(ui, "Fréquence respiration Hz", &mut fx.shape_breathe_rate, 0.05..=4.0, d.shape_breathe_rate);
        });
        let mut shot = false;
        let mut stop_shatter = false;
        sec.show(ui, 5, |ui| {
            if reset_button(
                ui,
                "Pas de destruction",
                "Arrêter la destruction : la forme reste entière",
                fx.shatter != Shatter::Off,
            ) {
                fx.shatter = Shatter::Off;
                stop_shatter = true;
            }
            ui.label(
                egui::RichText::new(
                    "La forme se casse depuis le centre du masque (ou du calque, sans masque) et laisse voir \
                     les calques du dessous, puis se reconstruit. Les fissures prennent la couleur des lumières.",
                )
                .small()
                .color(TEXT_WEAK),
            );
            ui.horizontal_wrapped(|ui| {
                for st in Shatter::ALL.into_iter().filter(|s| *s != Shatter::Off) {
                    if ui.selectable_label(fx.shatter == st, st.label()).on_hover_text(st.description()).clicked() {
                        fx.shatter = st;
                    }
                }
            });
            ui.add_enabled_ui(fx.shatter != Shatter::Off, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .button(with_icon(ic::LIGHTNING, "Casser !"))
                        .on_hover_text("Casse la forme une fois, puis la reconstruit (5 s)")
                        .clicked()
                    {
                        shot = true;
                    }
                });
                fx_slider(ui, "Avancement", &mut fx.shatter_amount, 0.0..=1.0, d.shatter_amount);
                fx_slider(ui, "Boucle cycles/s", &mut fx.shatter_speed, 0.0..=1.0, d.shatter_speed);
                fx_slider(ui, "Morceaux par hauteur", &mut fx.shatter_size, 3.0..=40.0, d.shatter_size);
                fx_slider(ui, "Lueur des fissures", &mut fx.shatter_glow, 0.0..=2.0, d.shatter_glow);
            });
        });
        if shot {
            self.layers[i].shatter_shot = Some(0.0);
        }
        if stop_shatter {
            self.layers[i].shatter_shot = None;
        }
    }
}
