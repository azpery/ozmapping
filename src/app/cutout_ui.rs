//! Fenêtre « Masque depuis une photo » (détourage).

use super::*;

impl OzApp {
    /// Fenêtre de détourage d'une photo.
    pub(super) fn cutout_window(&mut self, ctx: &egui::Context) {
        let Some(ed) = &mut self.cutout else { return };
        if ed.dirty {
            ed.dirty = false;
            let a = ACCENT;
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [ed.cut.width, ed.cut.height],
                &ed.cut.preview([a.r(), a.g(), a.b()]),
            );
            match &mut ed.tex {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => ed.tex = Some(ctx.load_texture("cutout", image, egui::TextureOptions::LINEAR)),
            }
        }
        let aspect = (self.output_size.x / self.output_size.y.max(1.0)).clamp(0.2, 5.0);
        let mut close = false;
        let mut apply = None;
        let layer_names: Vec<String> = self.project.layers.iter().map(|l| l.name.clone()).collect();
        egui::Modal::new(egui::Id::unique("cutout")).show(ctx, |ui| {
            let ed = self.cutout.as_mut().unwrap();
            ui.set_max_width(820.0);
            theme::section(ui, ic::IMAGE, &format!("Masque depuis une photo · {}", ed.name));
            ui.label(
                egui::RichText::new(
                    "Le sujet détecté est en clair, le fond assombri. Glisser : encadrer l'objet voulu (Maj : un cadre de plus) · \
                     clic : ajouter une zone · clic droit (ou Alt+clic) : la retirer.",
                )
                .small()
                .color(TEXT_WEAK),
            );
            if let Some(tex) = &ed.tex {
                let (w, h) = (ed.cut.width as f32, ed.cut.height as f32);
                let scale = (800.0 / w).min(480.0 / h);
                let size = Vec2::new(w * scale, h * scale);
                let resp = ui.add(egui::Image::new((tex.id(), size)).sense(Sense::click_and_drag()));
                let resp = resp.on_hover_cursor(egui::CursorIcon::Crosshair);
                let to_px = |pos: egui::Pos2| ((pos - resp.rect.min) / scale).to_pos2();
                let remove = resp.secondary_clicked() || (resp.clicked() && ui.input(|i| i.modifiers.alt));
                if (resp.clicked() || resp.secondary_clicked())
                    && let Some(pos) = resp.interact_pointer_pos()
                {
                    let p = to_px(pos);
                    ed.cut.wand(p.x.max(0.0) as usize, p.y.max(0.0) as usize, !remove);
                    ed.dirty = true;
                }
                let painter = ui.painter_at(resp.rect);
                let to_screen = |p: egui::Pos2| resp.rect.min + p.to_vec2() * scale;
                // Coins de l'image projetée : on les attrape avant de tracer un cadre.
                if resp.drag_started_by(egui::PointerButton::Primary)
                    && let (Some(q), Some(pos)) = (ed.cut.projection, resp.interact_pointer_pos())
                {
                    ed.drag_corner = (0..4)
                        .map(|k| (k, to_screen(egui::pos2(q[k][0], q[k][1])).distance(pos)))
                        .filter(|&(_, d)| d < 14.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(k, _)| k);
                }
                if let Some(k) = ed.drag_corner {
                    if let (Some(q), Some(pos)) = (&mut ed.cut.projection, resp.interact_pointer_pos()) {
                        let p = to_px(pos);
                        q[k] = [p.x.clamp(0.0, ed.cut.width as f32), p.y.clamp(0.0, ed.cut.height as f32)];
                    }
                    if resp.drag_stopped() {
                        ed.drag_corner = None;
                    }
                } else if resp.drag_started_by(egui::PointerButton::Primary) {
                    // Cadrage : glisser pour encadrer l'objet voulu.
                    ed.drag_from = resp.interact_pointer_pos().map(to_px);
                }
                if let Some(q) = ed.cut.projection {
                    let pts: Vec<egui::Pos2> = q.iter().map(|p| to_screen(egui::pos2(p[0], p[1]))).collect();
                    let gold = Color32::from_rgb(255, 200, 40);
                    painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(2.0, gold)));
                    for (k, p) in pts.iter().enumerate() {
                        painter.circle(*p, 6.0, gold.gamma_multiply(0.4), Stroke::new(2.0, gold));
                        let label = ["1", "2", "3", "4"][k];
                        painter.text(*p + Vec2::new(9.0, -9.0), egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(11.0), gold);
                    }
                }
                for &[x0, y0, x1, y1] in ed.cut.frames() {
                    let r = Rect::from_min_max(
                        to_screen(egui::pos2(x0 as f32, y0 as f32)),
                        to_screen(egui::pos2(x1 as f32, y1 as f32)),
                    );
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                }
                if let (Some(from), Some(pos)) = (ed.drag_from, resp.interact_pointer_pos()) {
                    let to = to_px(pos);
                    let r = Rect::from_two_pos(to_screen(from), to_screen(to));
                    painter.rect_stroke(r, 0.0, Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
                    if resp.drag_stopped() {
                        let r = Rect::from_two_pos(from, to);
                        let add = ui.input(|i| i.modifiers.shift);
                        ed.cut.add_frame(
                            [r.min.x.max(0.0) as usize, r.min.y.max(0.0) as usize, r.max.x.max(0.0) as usize, r.max.y.max(0.0) as usize],
                            add,
                        );
                        ed.drag_from = None;
                        ed.dirty = true;
                    }
                } else if resp.drag_stopped() {
                    ed.drag_from = None;
                }
            }
            ui.horizontal(|ui| {
                let mut on = ed.cut.projection.is_some();
                if ui
                    .checkbox(&mut on, "Caler sur l'image projetée visible sur la photo")
                    .on_hover_text(
                        "Pour une photo prise de côté où l'on voit le rectangle projeté : ses 4 coins (en jaune, \
                         déplaçables) servent de repère, le masque tombe pile à sa place malgré la perspective.",
                    )
                    .changed()
                {
                    if on {
                        if !ed.cut.enable_projection() {
                            self.status = Some("Image projetée non trouvée : placez les 4 coins à la main.".into());
                        }
                    } else {
                        ed.cut.projection = None;
                    }
                }
                if ed.cut.projection.is_some()
                    && ui.button(with_icon(ic::CROSSHAIR, "Redétecter")).on_hover_text("Chercher de nouveau les coins").clicked()
                    && !ed.cut.enable_projection()
                {
                    self.status = Some("Image projetée non trouvée : placez les 4 coins à la main.".into());
                }
                ui.toggle_value(&mut ed.white, with_icon(ic::SUN, "Projeter du blanc"))
                    .on_hover_text("La sortie affiche un écran blanc : idéal pour prendre la photo (bords nets, détection fiable)");
            });
            if ed.cut.projection.is_some() {
                ui.label(
                    egui::RichText::new("Coins dans l'ordre : 1 haut-gauche, 2 haut-droit, 3 bas-droit, 4 bas-gauche de l'image projetée.")
                        .small()
                        .color(TEXT_WEAK),
                );
            }
            if !ed.cut.has_alpha() {
                ui.horizontal(|ui| {
                    ui.label("Méthode");
                    for m in cutout::Method::ALL {
                        if ui.selectable_label(ed.cut.method == m, m.label()).on_hover_text(m.description()).clicked()
                            && ed.cut.method != m
                        {
                            ed.cut.method = m;
                            ed.cut.retune(true);
                            ed.dirty = true;
                        }
                    }
                });
                if ed.cut.method == cutout::Method::Subject {
                    if let Some(e) = &ed.cut.model_error {
                        ui.colored_label(WARN, format!("Détection du sujet indisponible ({e}) : détourage par les couleurs."));
                    } else {
                        let resp = percent_slider(ui, &mut ed.cut.sensitivity, "Sensibilité")
                            .on_hover_text("Plus haut : sujet plus large (inclut les parties moins évidentes)");
                        if resp.changed() || resp.drag_stopped() {
                            ed.cut.retune(resp.drag_started() || !resp.dragged());
                            ed.dirty = true;
                        }
                    }
                }
            } else {
                ui.label(egui::RichText::new("Image déjà détourée : sa transparence est utilisée.").small().color(TEXT_WEAK));
            }
            ui.horizontal(|ui| {
                if ui.button(with_icon(ic::MAGIC_WAND, "Auto")).on_hover_text("Refaire le détourage automatique").clicked() {
                    ed.cut.auto();
                    ed.dirty = true;
                }
                if !ed.cut.frames().is_empty()
                    && ui
                        .button(with_icon(ic::CORNERS_OUT, "Toute l'image"))
                        .on_hover_text("Retirer les cadres : chercher le sujet dans toute l'image")
                        .clicked()
                {
                    ed.cut.clear_frames();
                    ed.dirty = true;
                }
                let undo = ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)));
                if ui
                    .add_enabled(ed.cut.can_undo(), egui::Button::new(with_icon(ic::ARROW_COUNTER_CLOCKWISE, "Annuler")))
                    .on_hover_text(format!("{CMD}+Z"))
                    .clicked()
                    || undo
                {
                    ed.cut.undo();
                    ed.dirty = true;
                }
                if ui.button(with_icon(ic::ARROWS_CLOCKWISE, "Inverser")).on_hover_text("Échanger sujet et fond").clicked() {
                    ed.cut.invert();
                    ed.dirty = true;
                }
                let mut fill = ed.cut.fill_holes;
                if ui.checkbox(&mut fill, "Boucher les trous").changed() {
                    ed.cut.set_fill_holes(fill);
                    ed.dirty = true;
                }
                ui.label(
                    egui::RichText::new(format!("Sujet : {:.0} % de l'image", ed.cut.coverage() * 100.0)).small().color(TEXT_WEAK),
                );
            });
            ui.add(egui::Slider::new(&mut ed.cut.tolerance, 4.0..=60.0).text("Tolérance de la baguette").max_decimals(0))
                .on_hover_text("Écart de couleur accepté autour du point cliqué");
            percent_slider(ui, &mut ed.cut.smooth, "Lissage du contour");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let target = &layer_names[ed.layer.min(layer_names.len() - 1)];
                let label = with_icon(ic::CHECK_SQUARE, &format!("Appliquer au calque « {target} »"));
                if ui.add(theme::primary_button(label)).clicked() {
                    apply = Some(ed.cut.rings(aspect));
                }
                if ui.button("Fermer").clicked() {
                    close = true;
                }
            });
        });
        match apply {
            Some(Ok((rings, nonzero))) => {
                let layer = self.cutout.as_ref().map_or(self.sel(), |ed| ed.layer);
                self.apply_mask_rings(layer, rings, nonzero);
                self.cutout = None;
            }
            Some(Err(e)) => self.status = Some(format!("Détourage : {e:#}")),
            None if close => self.cutout = None,
            None => {}
        }
    }
}

/// Détourage d'une photo en cours (fenêtre « Masque depuis une photo »).
pub(super) struct CutoutEditor {
    pub(super) cut: Cutout,
    pub(super) name: String,
    pub(super) tex: Option<egui::TextureHandle>,
    pub(super) dirty: bool,
    /// Début du cadre en cours de tracé (pixels de l'image de travail).
    pub(super) drag_from: Option<egui::Pos2>,
    /// Coin de l'image projetée en cours de déplacement.
    pub(super) drag_corner: Option<usize>,
    /// La sortie affiche un écran blanc (pour photographier l'image projetée).
    pub(super) white: bool,
    /// Calque qui recevra le masque : celui qui était sélectionné à l'ouverture de la fenêtre.
    pub(super) layer: usize,
}
