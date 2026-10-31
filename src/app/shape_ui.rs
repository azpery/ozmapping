//! Onglet Forme : cadrage, déformation, masque, et l'éditeur de forme sur l'image.

use super::*;

impl OzApp {
    /// Ouvre l'onglet Forme sur la déformation ou le masque.
    pub(super) fn show_shape_tab(&mut self, target: EditTarget) {
        self.edit_target = target;
        self.edit_both = false;
        self.right_tab = RightTab::Shape;
        self.sub_tab[RightTab::Shape as usize] = if target == EditTarget::Mask { 2 } else { 1 };
    }

    pub(super) fn shape_ui(&mut self, ui: &mut egui::Ui) {
        let aspect = (self.output_size.x / self.output_size.y.max(1.0)).clamp(0.2, 5.0);
        let sec = self.section();
        let i = self.sel();
        sec.show(ui, 0, |ui| {
            ui.label(egui::RichText::new("Comment l'image se place dans la surface").small().color(TEXT_WEAK));
            for f in Fit::ALL {
                ui.selectable_value(&mut self.project.layers[i].fit, f, f.label()).on_hover_text(f.hint());
            }
            ui.add_space(6.0);
            ui.label(egui::RichText::new(with_icon(ic::ARROWS_CLOCKWISE, "Rotation du média")).strong().color(TEXT));
            let rot = &mut self.project.layers[i].media_rotate;
            ui.horizontal(|ui| {
                for deg in [0.0, 90.0, 180.0, 270.0] {
                    if ui.selectable_label(rot.rem_euclid(360.0) == deg, format!("{deg:.0}°")).clicked() {
                        // 270° = -90° : reste dans la plage du curseur ci-dessous.
                        *rot = if deg > 180.0 { deg - 360.0 } else { deg };
                    }
                }
            });
            fx_slider(ui, "Angle °", rot, -180.0..=180.0, 0.0);
            ui.label(
                egui::RichText::new(
                    "Tourne la vidéo ou la photo dans la surface (90° : vidéo filmée en vertical). \
                     Pour une rotation animée : Effets › Géométrie › Rotation auto.",
                )
                .small()
                .color(TEXT_WEAK),
            );
        });
        // L'onglet choisit ce qu'on édite sur l'aperçu.
        match sec.0 {
            1 => self.edit_target = EditTarget::Warp,
            2 => self.edit_target = EditTarget::Mask,
            _ => {}
        }
        if sec.0 != 0 {
            ui.toggle_value(&mut self.edit_mapping, with_icon(ic::PENCIL_SIMPLE, "Éditer sur l'aperçu (M)"));
            ui.label(
                egui::RichText::new(
                    "Glissez les points sur l'aperçu ou sur la sortie. Maj = déplacement précis. La croix déplace toute la forme.",
                )
                .small()
                .color(TEXT_WEAK),
            );
            ui.add_space(4.0);
        }

        let sel = self.sel();
        let mut import_shape = None;
        let cfg = &mut self.project.layers[sel];
        // Taille de l'ensemble : la surface du média et le masque grandissent ensemble,
        // autour du centre de la surface, sans bouger l'un par rapport à l'autre.
        ui.label(egui::RichText::new(with_icon(ic::ARROWS_OUT_CARDINAL, "Taille (média + masque)")).strong().color(TEXT));
        if ui
            .toggle_value(&mut self.edit_both, with_icon(ic::CORNERS_OUT, "À la souris sur l'aperçu"))
            .on_hover_text("Un cadre entoure le média et le masque : tirer un coin pour redimensionner, la croix pour déplacer")
            .changed()
            && self.edit_both
        {
            self.edit_mapping = true;
        }
        ui.horizontal_wrapped(|ui| {
            let mut factor = 1.0f32;
            for (label, f) in [("-10 %", 1.0 / 1.1), ("-1 %", 1.0 / 1.01), ("+1 %", 1.01), ("+10 %", 1.1)] {
                if ui.button(label).clicked() {
                    factor = f;
                }
            }
            let mut drag = 0.0f32;
            ui.add(egui::DragValue::new(&mut drag).speed(0.5).custom_formatter(|_, _| "Glisser pour redimensionner".into()))
                .on_hover_text("Glisser à droite pour agrandir, à gauche pour réduire");
            factor *= (drag / 100.0).exp();
            if factor != 1.0 {
                let center = cfg.warp.center();
                cfg.warp.scale(center, factor);
                cfg.mask.scale(center, factor);
            }
        });
        ui.add_space(4.0);
        sec.show(ui, 1, |ui| {
            let (mut cols, mut rows) = (cfg.warp.cols, cfg.warp.rows);
            ui.horizontal(|ui| {
                ui.label("Grille");
                ui.add(egui::DragValue::new(&mut cols).range(1..=MAX_GRID).suffix(" col."));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut rows).range(1..=MAX_GRID).suffix(" lig."));
            });
            if (cols, rows) != (cfg.warp.cols, cfg.warp.rows) {
                cfg.warp.resize(cols, rows);
                self.edit_target = EditTarget::Warp;
            }
            ui.label(
                egui::RichText::new(
                    "1×1 : mapping 4 coins en perspective. Plus de points : surfaces courbes (colonne, sphère, angle…).",
                )
                .small()
                .weak(),
            );
            ui.horizontal(|ui| {
                if ui
                    .button(with_icon(ic::SQUARES_FOUR, "Aplatir"))
                    .on_hover_text("Supprime les courbures, garde les 4 coins")
                    .clicked()
                {
                    cfg.warp.flatten();
                }
                if ui.button(with_icon(ic::CORNERS_OUT, "Plein cadre")).clicked() {
                    let full = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
                    cfg.warp = Warp::regular(cfg.warp.cols, cfg.warp.rows, &full);
                }
            });
        });
        sec.show(ui, 2, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut cfg.mask.enabled, "Activer");
                ui.checkbox(&mut cfg.mask.invert, "Inverser");
            });
            ui.add(egui::Slider::new(&mut cfg.mask.feather, 0.0..=200.0).text("Adoucissement").suffix(" px"));
            ui.label("Formes :");
            ui.horizontal_wrapped(|ui| {
                for shape in MaskShape::ALL {
                    if ui.button(shape.label()).clicked() {
                        cfg.mask.set_shape(shape.points(aspect));
                        cfg.mask.enabled = true;
                        self.edit_target = EditTarget::Mask;
                        self.edit_both = false;
                        self.edit_mapping = true;
                    }
                }
                if ui
                    .button(with_icon(ic::FILE_PLUS, "Importer un SVG…"))
                    .on_hover_text("Utiliser la forme d'un fichier SVG (logo, pictogramme…). Glisser-déposer un .svg marche aussi.")
                    .clicked()
                {
                    import_shape = rfd::FileDialog::new().add_filter("Image SVG", &["svg"]).pick_file();
                }
                if ui
                    .button(with_icon(ic::IMAGE, "Depuis une photo…"))
                    .on_hover_text(
                        "Détoure automatiquement le sujet d'une photo (ou d'une image PNG, JPG, HEIC…) pour en faire le masque, \
                         avec retouches à la baguette magique. La photo est ajustée à la sortie : prise depuis le \
                         projecteur, le masque tombe pile sur le support. Glisser-déposer une image marche aussi.",
                    )
                    .clicked()
                {
                    import_shape = rfd::FileDialog::new().add_filter("Image", crate::mask::IMAGE_EXTENSIONS).pick_file();
                }
            });
            // Rotation de la forme : les points eux-mêmes tournent (le contour d'édition suit).
            ui.add_space(4.0);
            ui.label(egui::RichText::new(with_icon(ic::ARROWS_CLOCKWISE, "Rotation")).strong().color(TEXT));
            ui.horizontal_wrapped(|ui| {
                let mut turn = 0.0;
                for (label, deg) in [("-90°", -90.0), ("-15°", -15.0), ("-1°", -1.0), ("+1°", 1.0), ("+15°", 15.0), ("+90°", 90.0)] {
                    if ui.button(label).clicked() {
                        turn = deg;
                    }
                }
                // Molette : glisser horizontalement fait tourner la forme en direct.
                let mut drag = 0.0f32;
                ui.add(egui::DragValue::new(&mut drag).speed(0.5).suffix("°").custom_formatter(|_, _| "Glisser pour tourner".into()))
                    .on_hover_text("Glisser à gauche ou à droite pour tourner la forme");
                turn += drag;
                if turn != 0.0 {
                    cfg.mask.rotate(turn, aspect);
                    cfg.mask.enabled = true;
                }
            });
            ui.label(
                egui::RichText::new("Pour une forme qui tourne toute seule : Visuels › Sur le masque › Rotation auto.")
                    .small()
                    .color(TEXT_WEAK),
            );
            let rings = cfg.mask.ring_count();
            ui.label(
                egui::RichText::new(format!(
                    "{} point(s){} · double-clic sur un bord : ajouter un point · clic droit sur un point : le supprimer",
                    cfg.mask.total_points(),
                    if rings > 1 { format!(" en {rings} contours") } else { String::new() },
                ))
                .small()
                .color(TEXT_WEAK),
            );
        });
        if let Some(path) = import_shape {
            self.import_mask(&path);
            return;
        }
        let cfg = &mut self.project.layers[sel];
        ui.add_space(4.0);
        if ui.button(with_icon(ic::STACK, "Copier forme et masque vers tous les calques")).clicked() {
            let (warp, mask) = (cfg.warp.clone(), cfg.mask.clone());
            for l in &mut self.project.layers {
                l.warp = warp.clone();
                l.mask = mask.clone();
            }
        }
    }

    /// Importe un SVG, ou la forme détourée d'une image, comme masque du calque sélectionné.
    pub(super) fn import_mask(&mut self, path: &Path) {
        let aspect = (self.output_size.x / self.output_size.y.max(1.0)).clamp(0.2, 5.0);
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
            // Photo ou image : détourage automatique, à vérifier et retoucher avant d'appliquer.
            match Cutout::open(path) {
                Ok(cut) => {
                    self.cutout = Some(CutoutEditor { cut, name: file_name(path), tex: None, dirty: true, drag_from: None, drag_corner: None, white: false, layer: self.sel() });
                    self.status = None;
                }
                Err(e) => self.status = Some(format!("{} : {e:#}", file_name(path))),
            }
            return;
        }
        match crate::mask::import_svg(path, aspect) {
            Ok((rings, nonzero)) => {
                let sel = self.sel();
                let mask = &mut self.project.layers[sel].mask;
                mask.set_rings(rings, nonzero);
                mask.enabled = true;
                self.show_shape_tab(EditTarget::Mask);
                self.status = None;
            }
            Err(e) => self.status = Some(format!("{} : {e:#}", file_name(path))),
        }
    }

    /// Applique un masque (contours en coordonnées de sortie) au calque sélectionné.
    /// Pose le masque sur le calque `layer`, et le sélectionne pour montrer le résultat.
    pub(super) fn apply_mask_rings(&mut self, layer: usize, rings: Vec<Vec<surface::Point>>, nonzero: bool) {
        let layer = layer.min(self.project.layers.len() - 1);
        self.selected = layer;
        let mask = &mut self.project.layers[layer].mask;
        mask.set_rings(rings, nonzero);
        mask.enabled = true;
        self.show_shape_tab(EditTarget::Mask);
    }

    /// Poignées d'édition du calque sélectionné (et contours des autres calques).
    pub(super) fn shape_editor(&mut self, ui: &mut egui::Ui, rect: Rect, salt: &str) {
        if !self.edit_mapping {
            // Fin de l'édition : la prochaine reprend sur les points (média ou masque).
            self.edit_both = false;
            return;
        }
        let sel = self.sel();
        for (i, l) in self.project.layers.iter().enumerate() {
            if i != sel && l.visible {
                surface::draw_warp_outline(ui, rect, &l.warp, Color32::from_white_alpha(60));
            }
        }
        let cfg = &mut self.project.layers[sel];
        if self.edit_both {
            surface::edit_both(ui, rect, &mut cfg.warp, &mut cfg.mask, salt);
            return;
        }
        match self.edit_target {
            EditTarget::Warp => surface::edit_warp(ui, rect, &mut cfg.warp, salt),
            EditTarget::Mask => surface::edit_mask(ui, rect, &mut cfg.mask, salt),
        }
    }
}
