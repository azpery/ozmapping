//! Onglet « Disposition » : chaque sortie telle que son vidéoprojecteur la reçoit (orientation,
//! miroirs, écran), avec ses calques et leurs masques. L'aperçu de la fenêtre principale, lui,
//! reste droit : c'est là qu'on règle les formes.

use super::*;

/// Images hors écran des vignettes de cet onglet (à part de celles des fenêtres de sortie).
const THUMB_SLOT: usize = 0x4000;
const MASK_COLOR: Color32 = Color32::from_rgb(255, 170, 60);

impl OzApp {
    pub(super) fn layout_ui(&mut self, ui: &mut egui::Ui) {
        self.sync_outputs();
        ui.label(egui::RichText::new(with_icon(ic::PROJECTOR_SCREEN, "Disposition des vidéoprojecteurs")).strong().color(TEXT));
        ui.label(
            egui::RichText::new(
                "Chaque sortie telle que son VP la reçoit. Tourner ou retourner l'image ne change pas l'aperçu : \
                 les masques et les formes se règlent toujours sur l'image droite.",
            )
            .small()
            .color(TEXT_WEAK),
        );
        ui.add_space(8.0);
        for k in 0..self.project.outputs.len() {
            theme::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.layout_card(ui, k);
            });
            ui.add_space(8.0);
        }
    }

    fn layout_card(&mut self, ui: &mut egui::Ui, k: usize) {
        let phys = self.phys_size(k);
        let logical = self.out_size(k);
        ui.horizontal(|ui| {
            let name = self.project.outputs[k].name.clone();
            if ui.selectable_label(self.preview_out == k, egui::RichText::new(with_icon(ic::MONITOR, &name)).strong()).on_hover_text("Montrer cette sortie dans l'aperçu").clicked() {
                self.show_output(k);
            }
            ui.label(egui::RichText::new(format!("VP {:.0}×{:.0}", phys.x, phys.y)).small().color(TEXT_WEAK));
        });
        ui.horizontal(|ui| {
            ui.label("Écran");
            if k == 0 {
                self.screen_menu(ui);
            } else {
                self.extra_screen_menu(ui, k);
            }
        });
        ui.add_space(4.0);
        self.layout_thumbnail(ui, k, phys);
        ui.add_space(6.0);

        // Orientation.
        let o = &mut self.project.outputs[k];
        ui.label(egui::RichText::new(with_icon(ic::ARROWS_CLOCKWISE, "Rotation")).strong().color(TEXT));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for deg in [0.0f32, 90.0, 180.0, -90.0, 45.0, -45.0] {
                let on = (normalize(o.rotate) - deg).abs() < 0.01 || (deg == 180.0 && (normalize(o.rotate) + 180.0).abs() < 0.01);
                if ui.selectable_label(on, format!("{deg:.0}°")).clicked() {
                    o.rotate = deg;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for step in [-1.0f32, -0.1, 0.1, 1.0] {
                if ui.button(format!("{step:+}°")).clicked() {
                    o.rotate = normalize(o.rotate + step);
                }
            }
            ui.add(egui::DragValue::new(&mut o.rotate).speed(0.05).range(-180.0..=180.0).fixed_decimals(2).suffix("°"))
                .on_hover_text("Angle exact (glisser pour l'ajuster finement, ou double-clic pour le taper). Sens horaire.");
        });
        ui.add_space(2.0);
        ui.checkbox(&mut o.mirror_x, "Miroir gauche-droite").on_hover_text("Rétroprojection : le VP est derrière l'écran");
        ui.checkbox(&mut o.mirror_y, "Miroir haut-bas").on_hover_text("Pour un VP au plafond, utilisez plutôt la rotation 180°");
        if logical != phys {
            ui.label(egui::RichText::new(format!("Image composée en {:.0}×{:.0} (VP sur le côté)", logical.x, logical.y)).small().color(TEXT_WEAK));
        }

        // VP qui n'est pas en face : l'image est pré-déformée pour arriver droite sur la surface.
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(with_icon(ic::CROSSHAIR, "Angle du VP")).strong().color(TEXT))
                .on_hover_text("Le VP n'est pas en face de la surface : indiquez de combien il est tourné, l'image arrive rectangulaire");
            if (o.tilt_h != 0.0 || o.tilt_v != 0.0) && ui.small_button("En face").on_hover_text("Remettre les angles à 0").clicked() {
                o.tilt_h = 0.0;
                o.tilt_v = 0.0;
            }
        });
        let max = project::MAX_TILT;
        ui.add(egui::Slider::new(&mut o.tilt_h, -max..=max).step_by(0.1).fixed_decimals(1).suffix("°").text("Horizontal"))
            .on_hover_text("+ : VP à gauche de la surface, tourné vers la droite · − : VP à droite, tourné vers la gauche");
        ui.add(egui::Slider::new(&mut o.tilt_v, -max..=max).step_by(0.1).fixed_decimals(1).suffix("°").text("Vertical"))
            .on_hover_text("+ : VP plus bas que la surface, tourné vers le haut · − : VP plus haut (au plafond), tourné vers le bas");
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Rapport de projection").small().color(TEXT_WEAK));
            ui.add(egui::DragValue::new(&mut o.throw).speed(0.01).range(0.2..=10.0).fixed_decimals(2))
                .on_hover_text("Distance du VP divisée par la largeur de l'image (fiche du VP, souvent 1,2 à 2). Plus il est petit, plus un même angle déforme l'image.");
        });

        // Réglage des 4 coins : caler l'image sur la surface, au pixel près.
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(with_icon(ic::CORNERS_OUT, "4 coins")).strong().color(TEXT));
            let mut on = self.corner_edit == Some(k);
            let label = if on { "Terminer" } else { "Régler" };
            if ui
                .toggle_value(&mut on, label)
                .on_hover_text("Attraper les coins de l'image ici ou dans la fenêtre du VP · flèches : 1 pixel (Maj : 10) · Échap : terminer")
                .changed()
            {
                self.corner_edit = on.then_some(k);
                if on {
                    self.edit_mapping = false;
                }
            }
            let o = &mut self.project.outputs[k];
            if o.corners_moved() && ui.small_button("Réinitialiser").on_hover_text("Coins à leur place calculée").clicked() {
                o.corners = [[0.0; 2]; 4];
            }
        });
        if self.corner_edit == Some(k) {
            ui.label(
                egui::RichText::new(format!(
                    "Coin {} choisi · glisser (Maj : finement) · flèches : 1 pixel du VP (Maj : 10)",
                    self.corner_sel + 1
                ))
                .small()
                .color(ACCENT),
            );
        }

        // Calques de cette sortie : leurs masques et leurs formes se règlent sur l'aperçu.
        ui.add_space(6.0);
        ui.label(egui::RichText::new(with_icon(ic::STACK, "Calques")).strong().color(TEXT));
        let layers: Vec<usize> = (0..self.project.layers.len()).rev().filter(|&i| self.project.layers[i].output == k).collect();
        if layers.is_empty() {
            ui.label(egui::RichText::new("Aucun calque sur cette sortie").small().color(TEXT_WEAK));
        }
        for i in layers {
            ui.horizontal(|ui| {
                let l = &mut self.project.layers[i];
                let eye = if l.visible { ic::EYE } else { ic::EYE_SLASH };
                if ui.small_button(eye).on_hover_text("Afficher / masquer le calque").clicked() {
                    l.visible = !l.visible;
                }
                let editing = |what| self.edit_mapping && self.selected == i && !self.edit_both && self.edit_target == what;
                let (mask_on, warp_on) = (editing(EditTarget::Mask), editing(EditTarget::Warp));
                let name = self.project.layers[i].name.clone();
                if ui.selectable_label(self.selected == i, name).clicked() {
                    self.select_on_output(i, k);
                }
                let has_mask = self.project.layers[i].mask.enabled;
                let mask_text = if has_mask { "Masque" } else { "+ Masque" };
                if ui
                    .selectable_label(mask_on, with_icon(ic::SHAPES, mask_text))
                    .on_hover_text("Régler le masque de ce calque sur l'aperçu (poignées orange)")
                    .clicked()
                {
                    self.edit_layer_shape(i, k, EditTarget::Mask, !mask_on);
                }
                if ui
                    .selectable_label(warp_on, with_icon(ic::POLYGON, "Forme"))
                    .on_hover_text("Placer et déformer l'image de ce calque sur l'aperçu (poignées vertes)")
                    .clicked()
                {
                    self.edit_layer_shape(i, k, EditTarget::Warp, !warp_on);
                }
            });
        }
        // Formes de masque toutes faites pour le calque choisi, s'il est sur cette sortie.
        let sel = self.sel();
        if self.project.layers[sel].output == k && self.edit_mapping && self.edit_target == EditTarget::Mask {
            let aspect = (logical.x / logical.y.max(1.0)).clamp(0.2, 5.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for shape in MaskShape::ALL {
                    if ui.small_button(shape.label()).clicked() {
                        let mask = &mut self.project.layers[sel].mask;
                        mask.set_shape(shape.points(aspect));
                        mask.enabled = true;
                    }
                }
                let mask = &mut self.project.layers[sel].mask;
                ui.checkbox(&mut mask.enabled, "Actif");
                ui.checkbox(&mut mask.invert, "Inverser");
            });
        }
    }

    /// Vue du VP : l'image qu'il reçoit (en direct), le contour de l'image composée avec son
    /// haut marqué, et les calques de la sortie. Clic sur un calque : le choisir ; glisser :
    /// le déplacer (image et masque ensemble, ou l'un des deux seulement).
    fn layout_thumbnail(&mut self, ui: &mut egui::Ui, k: usize, phys: Vec2) {
        let move_id = egui::Id::unique("layout_move");
        // Même choix que l'édition sur l'aperçu principal : quand on y édite, il suit celle-ci.
        let mut mode: u8 = if self.edit_mapping {
            if self.edit_both {
                0
            } else if self.edit_target == EditTarget::Mask {
                1
            } else {
                2
            }
        } else {
            ui.data(|d| d.get_temp(move_id)).unwrap_or(0)
        };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Glisser déplace :").small().color(TEXT_WEAK));
            for (m, label, hint) in [
                (0u8, "Calque + masque", "L'image du calque et son masque ensemble (ici et sur l'aperçu principal)"),
                (1, "Masque", "Seulement le masque (la forme découpée) : l'image reste en place"),
                (2, "Image", "Seulement l'image du calque : le masque reste en place"),
            ] {
                if ui.selectable_label(mode == m, egui::RichText::new(label).small()).on_hover_text(hint).clicked() {
                    mode = m;
                    // L'aperçu principal édite la même chose : le cadre de l'ensemble, les points
                    // du masque, ou les coins de l'image.
                    let sel = self.sel();
                    if m == 0 {
                        self.edit_mapping = true;
                        self.edit_both = true;
                    } else {
                        // Comme « Masque » / « Forme » de la liste : un masque est créé s'il n'y en a pas.
                        let out = self.project.layers[sel].output;
                        self.edit_layer_shape(sel, out, if m == 1 { EditTarget::Mask } else { EditTarget::Warp }, true);
                    }
                }
            }
        });
        ui.data_mut(|d| d.insert_temp(move_id, mode));

        let aspect = (phys.x / phys.y.max(1.0)).clamp(0.2, 5.0);
        let w = ui.available_width().min(420.0);
        let h = (w / aspect).min(320.0);
        let size = Vec2::new(h * aspect, h);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        self.paint_output(ui, rect, k, THUMB_SLOT + k);
        let pr = self.project.outputs[k].projection([rect.width(), rect.height()]);
        let to_screen = |p: [f32; 2]| {
            let q = pr.place(p);
            rect.min + Vec2::new(q[0], q[1])
        };
        let to_image = |p: egui::Pos2| pr.unplace([p.x - rect.min.x, p.y - rect.min.y]);

        // Calque sous le pointeur : le plus haut dont l'image (dans son masque) couvre ce point.
        let layers: Vec<usize> = (0..self.project.layers.len()).filter(|&i| self.project.layers[i].output == k && self.project.layers[i].visible).collect();
        let hit = |app: &Self, pos: egui::Pos2| {
            let p = to_image(pos)?;
            layers.iter().rev().copied().find(|&i| {
                let l = &app.project.layers[i];
                inside(&l.warp.outline(), p) && l.mask.contains(p)
            })
        };
        let corner_mode = self.corner_edit == Some(k);
        let hovered = resp.hover_pos().filter(|_| !corner_mode).and_then(|p| hit(self, p));
        if corner_mode {
            // Réglage des coins : les calques ne bougent pas.
        } else if (resp.clicked() || resp.drag_started())
            && let Some(i) = resp.interact_pointer_pos().and_then(|p| hit(self, p))
        {
            self.select_on_output(i, k);
        } else if resp.clicked() {
            self.show_output(k);
        }
        let sel = self.sel();
        if resp.dragged()
            && !corner_mode
            && self.project.layers[sel].output == k
            && let Some(pos) = resp.interact_pointer_pos()
            && let (Some(a), Some(b)) = (to_image(pos - resp.drag_delta()), to_image(pos))
        {
            let d = [b[0] - a[0], b[1] - a[1]];
            let l = &mut self.project.layers[sel];
            if mode != 1 {
                l.warp.translate(d);
            }
            if mode != 2 {
                for r in 0..l.mask.ring_count() {
                    for p in l.mask.ring_mut(r) {
                        p[0] += d[0];
                        p[1] += d[1];
                    }
                }
            }
        }
        let resp = resp.on_hover_text("Ce que reçoit le VP · clic : choisir un calque · glisser : le déplacer");
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if hovered.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Outside);
        // Contour de l'image composée, et une flèche vers son haut.
        let corners: Vec<egui::Pos2> = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].into_iter().map(to_screen).collect();
        painter.add(egui::Shape::closed_line(corners, Stroke::new(1.0, ACCENT.gamma_multiply(0.4))));
        let (tip, base) = (to_screen([0.5, 0.04]), to_screen([0.5, 0.14]));
        painter.arrow(base, tip - base, Stroke::new(2.0, ACCENT.gamma_multiply(0.7)));
        // Calques : contour de l'image (vert pour le calque choisi), masque en orange.
        let curve = |pts: &[[f32; 2]]| -> Vec<egui::Pos2> {
            // Les bords se courbent en perspective : on les échantillonne.
            let mut out = Vec::with_capacity(pts.len() * 4);
            for (j, &a) in pts.iter().enumerate() {
                let b = pts[(j + 1) % pts.len()];
                for t in 0..4 {
                    let t = t as f32 / 4.0;
                    out.push(to_screen([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]));
                }
            }
            out
        };
        for &i in &layers {
            let l = &self.project.layers[i];
            let (chosen, over) = (i == sel, hovered == Some(i));
            let color = if chosen { ACCENT } else if over { TEXT } else { Color32::from_white_alpha(90) };
            painter.add(egui::Shape::closed_line(curve(&l.warp.outline()), Stroke::new(if chosen { 2.0 } else { 1.0 }, color)));
            if l.mask.enabled {
                for ring in l.mask.all_rings().filter(|r| r.len() > 2) {
                    let stroke = Stroke::new(if chosen { 1.8 } else { 1.0 }, MASK_COLOR.gamma_multiply(if chosen { 1.0 } else { 0.6 }));
                    painter.add(egui::Shape::closed_line(curve(ring), stroke));
                }
            }
            if chosen || over {
                let c = l.warp.evaluator()(0.5, 0.5);
                painter.text(to_screen(c), egui::Align2::CENTER_CENTER, &l.name, egui::FontId::proportional(11.0), color);
            }
        }
        if corner_mode {
            self.corner_handles(ui, rect, k);
        }
    }

    /// Poignées des 4 coins de l'image de la sortie `k`, dans `rect` (l'image du VP en entier :
    /// sa fenêtre, ou la vue de l'onglet Disposition).
    pub(super) fn corner_handles(&mut self, ui: &mut egui::Ui, rect: Rect, k: usize) {
        let size = rect.size();
        let frac = self.project.outputs[k].corners([size.x, size.y]);
        let pts: Vec<egui::Pos2> = frac.iter().map(|f| rect.min + Vec2::new(f[0] * size.x, f[1] * size.y)).collect();
        let painter = ui.painter_at(rect);
        painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.5, ACCENT)));
        painter.line_segment([pts[0], pts[2]], Stroke::new(1.0, ACCENT.gamma_multiply(0.3)));
        painter.line_segment([pts[1], pts[3]], Stroke::new(1.0, ACCENT.gamma_multiply(0.3)));
        let fine = ui.input(|i| i.modifiers.shift);
        for (i, &p) in pts.iter().enumerate() {
            let resp = ui
                .interact(Rect::from_center_size(p, Vec2::splat(30.0)), ui.scope_id().with(("vp_corner", k, i)), Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text("Glisser ce coin (Maj : finement) · flèches : 1 pixel (Maj : 10)");
            if resp.clicked() || resp.drag_started() {
                self.corner_sel = i;
            }
            if resp.dragged() {
                let d = resp.drag_delta() * if fine { 0.1 } else { 1.0 };
                let c = &mut self.project.outputs[k].corners[i];
                c[0] += d.x / size.x.max(1.0);
                c[1] += d.y / size.y.max(1.0);
            }
            let chosen = self.corner_sel == i;
            let r = if chosen || resp.hovered() { 10.0 } else { 8.0 };
            painter.circle(p, r, if chosen { ACCENT } else { Color32::from_black_alpha(190) }, Stroke::new(2.0, ACCENT));
            painter.text(p, egui::Align2::CENTER_CENTER, (i + 1).to_string(), egui::FontId::proportional(11.0), if chosen { Color32::BLACK } else { ACCENT });
        }
    }

    /// Réglage des coins en cours : les flèches déplacent le coin choisi d'un pixel du VP
    /// (Maj : 10) au lieu de changer de média ; Échap termine.
    pub(super) fn corner_keys(&mut self, ctx: &egui::Context) {
        let Some(k) = self.corner_edit else { return };
        if k >= self.project.outputs.len() {
            self.corner_edit = None;
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let shift = ctx.input(|i| i.modifiers.shift);
        let mods = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
        let step = if shift { 10.0 } else { 1.0 };
        let phys = self.phys_size(k);
        for (key, d) in [(Key::ArrowLeft, [-1.0, 0.0]), (Key::ArrowRight, [1.0, 0.0]), (Key::ArrowUp, [0.0, -1.0]), (Key::ArrowDown, [0.0, 1.0])] {
            if ctx.input_mut(|i| i.consume_key(mods, key)) {
                let c = &mut self.project.outputs[k].corners[self.corner_sel.min(3)];
                c[0] += d[0] * step / phys.x.max(1.0);
                c[1] += d[1] * step / phys.y.max(1.0);
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.corner_edit = None;
        }
    }

    /// Montre la sortie `k` dans l'aperçu (et choisit l'un de ses calques).
    fn show_output(&mut self, k: usize) {
        self.preview_out = k;
        if self.project.layers[self.sel()].output != k
            && let Some(i) = self.project.layers.iter().rposition(|l| l.output == k)
        {
            self.selected = i;
        }
        self.last_sel = self.sel();
    }

    fn select_on_output(&mut self, i: usize, k: usize) {
        self.selected = i;
        self.preview_out = k;
        self.last_sel = self.sel();
    }

    /// Édition du masque ou de la forme du calque `i` sur l'aperçu (`on` = false : arrêter).
    fn edit_layer_shape(&mut self, i: usize, k: usize, what: EditTarget, on: bool) {
        self.select_on_output(i, k);
        self.edit_both = false;
        self.edit_target = what;
        self.edit_mapping = on;
        if on && what == EditTarget::Mask && !self.project.layers[i].mask.enabled {
            let o = self.out_size(k);
            let mask = &mut self.project.layers[i].mask;
            if mask.points.len() < 3 {
                mask.set_shape(MaskShape::Rectangle.points((o.x / o.y.max(1.0)).clamp(0.2, 5.0)));
            }
            mask.enabled = true;
        }
    }
}

/// Angle ramené entre -180° et 180°.
fn normalize(deg: f32) -> f32 {
    let d = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if d <= -180.0 { 180.0 } else { d }
}

/// Le point est-il dans le polygone (règle pair-impair) ?
fn inside(poly: &[[f32; 2]], p: [f32; 2]) -> bool {
    let mut odd = false;
    for (j, &a) in poly.iter().enumerate() {
        let b = poly[(j + poly.len() - 1) % poly.len()];
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            odd = !odd;
        }
    }
    odd
}
