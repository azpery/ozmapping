//! Fenêtre de sortie : choix de l'écran, plein écran, aperçu dans la fenêtre de contrôle.

use super::*;

impl OzApp {
    /// Menu de choix de l'écran de sortie.
    pub(super) fn screen_menu(&mut self, ui: &mut egui::Ui) {
        // Liste rafraîchie régulièrement : un projecteur peut être branché en cours de route.
        let now = ui.input(|i| i.time);
        if now - self.screens_at > 2.0 {
            self.screens_at = now;
            self.screens = screens::list();
        }
        let current = self.screens.iter().find(|s| Some(&s.key) == self.output_screen.as_ref());
        let text = current.map_or("Écran…".to_owned(), |s| s.label.split("  ·").next().unwrap_or_default().to_owned());
        let mut chosen = None;
        egui::ComboBox::from_id_salt("output_screen")
            .selected_text(text)
            .width(150.0)
            .show_ui(ui, |ui| {
                if self.screens.is_empty() {
                    ui.label("Aucun écran détecté");
                }
                for screen in &self.screens {
                    let selected = Some(&screen.key) == self.output_screen.as_ref();
                    if ui.selectable_label(selected, &screen.label).clicked() {
                        chosen = Some(screen.clone());
                    }
                }
            })
            .response
            .on_hover_text("Écran de la sortie : la fenêtre s'y place en plein écran");
        if let Some(screen) = chosen {
            self.output_screen = Some(screen.key.clone());
            self.output_open = true;
            self.output_move = Some(OutputMove { screen, stage: 0, at: 0.0 });
        }
    }

    /// Fait avancer le déplacement de la sortie vers l'écran choisi (dans la fenêtre de sortie).
    pub(super) fn drive_output_move(&mut self, ctx: &egui::Context) {
        let fullscreen = self.output_fullscreen;
        let Some(mv) = &mut self.output_move else { return };
        let now = ctx.input(|i| i.time);
        ctx.request_repaint();
        match mv.stage {
            0 => {
                // macOS anime la sortie du plein écran : on attend avant de déplacer.
                if fullscreen {
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                    mv.at = now + if cfg!(target_os = "macos") { 1.0 } else { 0.2 };
                } else {
                    mv.at = now;
                }
                mv.stage = 1;
            }
            1 if now >= mv.at => {
                let pos = screens::window_position(&mv.screen, ctx.pixels_per_point());
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos));
                mv.at = now + 0.4;
                mv.stage = 2;
            }
            2 if now >= mv.at => {
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
                self.output_fullscreen = true;
                self.output_move = None;
            }
            _ => {}
        }
    }

    pub(super) fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.output_open = true;
        self.output_fullscreen = on;
        ctx.send_viewport_cmd_to(output_viewport_id(), ViewportCommand::Fullscreen(on));
    }

    pub(super) fn preview(&mut self, ui: &mut egui::Ui) {
        // Choisir un calque montre sa sortie.
        let sel = self.sel();
        if sel != self.last_sel {
            self.last_sel = sel;
            self.preview_out = self.project.layers[sel].output.min(self.project.outputs.len().max(1) - 1);
        }
        self.preview_out = self.preview_out.min(self.project.outputs.len().max(1) - 1);
        let full = ui.available_rect_before_wrap();
        let tabs = Rect::from_min_size(full.min + Vec2::new(14.0, 2.0), Vec2::new(full.width() - 28.0, 24.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(tabs), |ui| self.output_tabs(ui));
        let avail = Rect::from_min_max(egui::pos2(full.min.x, tabs.max.y + 6.0), full.max).shrink(14.0);
        let out = self.preview_out;
        let out_size = self.out_size(out);
        let aspect = (out_size.x / out_size.y.max(1.0)).clamp(0.2, 5.0);
        let size = if avail.width() / avail.height() > aspect {
            Vec2::new(avail.height() * aspect, avail.height())
        } else {
            Vec2::new(avail.width(), avail.width() / aspect)
        };
        let rect = Rect::from_center_size(avail.center(), size);
        ui.allocate_rect(avail, Sense::hover());

        let painter = ui.painter_at(avail.expand(14.0));
        if let Some(r) = self.renderer.clone() {
            let params = self.draw_params_for(out);
            // Enregistrement : la sortie 1 est aussi rendue hors écran, à la définition choisie.
            if let Some(rec) = self.recorder.clone() {
                let main = if out == 0 { params.clone() } else { self.draw_params_for(0) };
                painter.add(renderer::capture_callback(rect, r.clone(), main, rec));
            }
            painter.add(renderer::paint_callback(rect, r, params));
        }
        painter.rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Outside);
        painter.text(
            rect.left_top() + Vec2::new(2.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{}  {}  ·  {:.0}×{:.0}", ic::MONITOR, self.project.outputs.get(out).map_or("Sortie", |o| o.name.as_str()), out_size.x, out_size.y),
            egui::FontId::proportional(12.0),
            TEXT_WEAK,
        );
        // États importants rappelés au-dessus de l'aperçu.
        let mut tags = Vec::new();
        if self.blackout {
            tags.push(with_icon(ic::MOON, "BLACKOUT"));
        }
        if self.white_screen {
            tags.push(with_icon(ic::SUN, "BLANC"));
        }
        if self.bypass_fx {
            tags.push(with_icon(ic::PROHIBIT, "SANS EFFETS"));
        }
        if self.edit_mapping {
            tags.push(with_icon(ic::POLYGON, "ÉDITION FORME"));
        }
        if !tags.is_empty() {
            painter.text(
                rect.right_top() + Vec2::new(-2.0, -6.0),
                egui::Align2::RIGHT_BOTTOM,
                tags.join("    "),
                egui::FontId::proportional(12.0),
                WARN,
            );
        }
        self.leds_overlay(&painter, rect);
        // Formes du calque choisi : seulement sur sa sortie.
        if self.project.layers[sel].output.min(self.project.outputs.len().max(1) - 1) == out {
            self.shape_editor(ui, rect, "preview");
        }
    }

    /// Taille (pixels) de la fenêtre d'une sortie, c'est-à-dire de l'image du VP.
    pub(super) fn phys_size(&self, k: usize) -> Vec2 {
        if k == 0 { self.output_size } else { self.outs.get(k).map_or(Vec2::new(1920.0, 1080.0), |o| o.size) }
    }

    /// Taille de l'image composée d'une sortie (aperçu, masques, déformations) : celle de sa
    /// fenêtre, largeur et hauteur échangées quand le VP est tourné sur le côté.
    pub(super) fn out_size(&self, k: usize) -> Vec2 {
        let p = self.phys_size(k);
        let l = self.project.outputs.get(k).map_or([p.x, p.y], |o| o.logical([p.x, p.y]));
        Vec2::new(l[0], l[1])
    }

    /// Image d'une sortie dans `rect` (sa fenêtre) : telle quelle, ou tournée et retournée
    /// comme le VP l'attend.
    /// `slot` : image hors écran à utiliser (une par endroit où la sortie est montrée).
    pub(super) fn paint_output(&mut self, ui: &egui::Ui, rect: Rect, k: usize, slot: usize) {
        let Some(r) = self.renderer.clone() else { return };
        let params = self.draw_params_for(k);
        match self.project.outputs.get(k).filter(|o| o.oriented()) {
            Some(o) => {
                let phys = rect.size() * ui.ctx().pixels_per_point();
                let corners = o.corners([phys.x, phys.y]);
                let l = o.logical([phys.x, phys.y]);
                let size = (l[0].round().clamp(1.0, 8192.0) as u32, l[1].round().clamp(1.0, 8192.0) as u32);
                ui.painter().add(renderer::paint_oriented_callback(rect, r, params, slot, size, corners));
            }
            None => {
                ui.painter().add(renderer::paint_callback(rect, r, params));
            }
        }
    }

    /// Mode édition dans la fenêtre d'une sortie : poignées des formes du calque choisi (sur
    /// une sortie tournée ou retournée, elles ne tomberaient pas au bon endroit : on édite
    /// alors sur l'aperçu, qui reste droit).
    fn output_editor(&mut self, ui: &mut egui::Ui, rect: Rect, k: usize, salt: &str) {
        if self.project.outputs.get(k).is_some_and(|o| o.oriented()) {
            ui.painter().text(
                rect.center_top() + Vec2::new(0.0, 24.0),
                egui::Align2::CENTER_TOP,
                "Sortie tournée ou retournée : les formes se règlent sur l'aperçu de la fenêtre principale",
                egui::FontId::proportional(16.0),
                WARN,
            );
        } else {
            self.shape_editor(ui, rect, salt);
        }
    }

    /// Une entrée d'état par sortie du projet.
    pub(super) fn sync_outputs(&mut self) {
        let n = self.project.outputs.len().max(1);
        while self.outs.len() < n {
            // Une sortie qui a un écran (projet rouvert) y retourne.
            let k = self.outs.len();
            let screen = self.project.outputs[k].screen.as_ref().and_then(|key| self.screens.iter().find(|s| &s.key == key)).cloned();
            self.outs.push(ExtraOut { mv: screen.map(|screen| OutputMove { screen, stage: 0, at: 0.0 }), ..ExtraOut::default() });
        }
        self.outs.truncate(n);
    }

    /// Onglets des sorties au-dessus de l'aperçu ; réglages de la sortie montrée.
    fn output_tabs(&mut self, ui: &mut egui::Ui) {
        self.sync_outputs();
        let n = self.project.outputs.len();
        let mut pick = None;
        let mut remove = None;
        ui.horizontal(|ui| {
            for k in 0..n {
                let name = self.project.outputs[k].name.clone();
                let layers = self.project.layers.iter().filter(|l| l.output == k).count();
                let r = ui
                    .selectable_label(self.preview_out == k, with_icon(ic::MONITOR, &name))
                    .on_hover_text(format!("{layers} calque(s) · clic droit : renommer, supprimer"));
                if r.clicked() {
                    pick = Some(k);
                }
                r.context_menu(|ui| {
                    ui.text_edit_singleline(&mut self.project.outputs[k].name);
                    if k > 0 && ui.button(with_icon(ic::TRASH, "Supprimer cette sortie")).clicked() {
                        remove = Some(k);
                        ui.close();
                    }
                });
            }
            if ui.small_button(ic::PLUS).on_hover_text("Nouvelle sortie : une autre fenêtre, sur un autre écran, avec ses propres calques").clicked() {
                let k = self.project.outputs.len();
                self.project.outputs.push(project::OutputDef { name: format!("Sortie {}", k + 1), ..Default::default() });
                self.sync_outputs();
                self.outs[k].open = true;
                // Un premier calque pour cette sortie.
                if self.project.layers.len() < MAX_LAYERS {
                    self.add_layer();
                    let i = self.sel();
                    self.project.layers[i].output = k;
                }
                pick = Some(k);
            }
            // Sortie 2 et suivantes : fenêtre, écran, plein écran.
            let k = self.preview_out;
            if k > 0 && k < self.outs.len() {
                ui.separator();
                ui.toggle_value(&mut self.outs[k].open, "Fenêtre").on_hover_text("Ouvrir / fermer la fenêtre de cette sortie");
                self.extra_screen_menu(ui, k);
                let mut fs = self.outs[k].fullscreen;
                if ui.toggle_value(&mut fs, "Plein écran").changed() {
                    self.outs[k].open = true;
                    self.outs[k].fullscreen = fs;
                    ui.ctx().send_viewport_cmd_to(extra_viewport_id(k), ViewportCommand::Fullscreen(fs));
                }
            }
        });
        if let Some(k) = remove {
            // Ses calques et ses écrans de la scène reviennent à la sortie 1.
            self.project.outputs.remove(k);
            for l in &mut self.project.layers {
                if l.output == k {
                    l.output = 0;
                } else if l.output > k {
                    l.output -= 1;
                }
            }
            for s in &mut self.project.leds.surfaces {
                if s.output == k {
                    s.output = 0;
                } else if s.output > k {
                    s.output -= 1;
                }
            }
            self.sync_outputs();
            self.preview_out = 0;
        }
        if let Some(k) = pick {
            self.preview_out = k;
            // On travaille sur un calque de cette sortie.
            if self.project.layers[self.sel()].output != k {
                if let Some(i) = self.project.layers.iter().rposition(|l| l.output == k) {
                    self.selected = i;
                }
            }
            self.last_sel = self.sel();
        }
    }

    /// Choix de l'écran d'une sortie 2 et suivantes.
    pub(super) fn extra_screen_menu(&mut self, ui: &mut egui::Ui, k: usize) {
        let current = self.project.outputs[k].screen.clone();
        let text = self.screens.iter().find(|s| Some(&s.key) == current.as_ref()).map_or("Écran…".to_owned(), |s| s.label.split("  ·").next().unwrap_or_default().to_owned());
        let mut chosen = None;
        egui::ComboBox::from_id_salt(("extra_screen", k)).selected_text(text).width(120.0).show_ui(ui, |ui| {
            for screen in &self.screens {
                if ui.selectable_label(Some(&screen.key) == current.as_ref(), &screen.label).clicked() {
                    chosen = Some(screen.clone());
                }
            }
        });
        if let Some(screen) = chosen {
            self.project.outputs[k].screen = Some(screen.key.clone());
            self.outs[k].open = true;
            self.outs[k].mv = Some(OutputMove { screen, stage: 0, at: 0.0 });
        }
    }

    /// Fenêtre d'une sortie 2 et suivantes.
    pub(super) fn extra_output_window(&mut self, ui: &mut egui::Ui, k: usize) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) {
            self.outs[k].open = false;
            self.outs[k].fullscreen = false;
            return;
        }
        if let Some(fs) = ctx.input(|i| i.viewport().fullscreen) {
            self.outs[k].fullscreen = fs;
        }
        // Déplacement vers l'écran choisi : sortir du plein écran, déplacer, y revenir.
        let now = ctx.input(|i| i.time);
        let fullscreen = self.outs[k].fullscreen;
        if let Some(mv) = &mut self.outs[k].mv {
            ctx.request_repaint();
            match mv.stage {
                0 => {
                    if fullscreen {
                        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                        mv.at = now + if cfg!(target_os = "macos") { 1.0 } else { 0.2 };
                    } else {
                        mv.at = now;
                    }
                    mv.stage = 1;
                }
                1 if now >= mv.at => {
                    let pos = screens::window_position(&mv.screen, ctx.pixels_per_point());
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos));
                    mv.at = now + 0.4;
                    mv.stage = 2;
                }
                2 if now >= mv.at => {
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
                    self.outs[k].fullscreen = true;
                    self.outs[k].mv = None;
                }
                _ => {}
            }
        }
        // F : plein écran de cette fenêtre ; les autres raccourcis restent ceux de l'app.
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F)) {
            let fs = !self.outs[k].fullscreen;
            self.outs[k].fullscreen = fs;
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(fs));
        }
        if self.outs[k].fullscreen && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.outs[k].fullscreen = false;
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        self.handle_shortcuts(&ctx);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(ui, |ui| {
            let rect = ui.max_rect();
            self.outs[k].size = rect.size() * ctx.pixels_per_point();
            self.paint_output(ui, rect, k, k);
            self.sketch_capture(ui, Some(k));
            let bg = ui.interact(rect, ui.scope_id().with(("extra_bg", k)), Sense::click());
            if bg.double_clicked() && !self.edit_mapping {
                let fs = !self.outs[k].fullscreen;
                self.outs[k].fullscreen = fs;
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(fs));
            }
            if self.corner_edit == Some(k) {
                self.corner_handles(ui, rect, k);
            } else if self.edit_mapping && self.project.layers[self.sel()].output == k {
                self.output_editor(ui, rect, k, &format!("output{k}"));
            } else if self.outs[k].fullscreen {
                ctx.set_cursor_icon(egui::CursorIcon::None);
            }
        });
    }

    pub(super) fn output_window(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) {
            self.output_open = false;
            self.output_fullscreen = false;
            return;
        }
        if let Some(fs) = ctx.input(|i| i.viewport().fullscreen) {
            self.output_fullscreen = fs;
        }
        self.drive_output_move(&ctx);
        self.handle_shortcuts(&ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::BLACK))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                self.output_size = rect.size() * ctx.pixels_per_point();
                self.paint_output(ui, rect, 0, 0);
                self.sketch_capture(ui, Some(0));
                let bg = ui.interact(rect, ui.scope_id().with("output_bg"), Sense::click());
                if bg.double_clicked() && !self.edit_mapping {
                    let fs = !self.output_fullscreen;
                    self.set_fullscreen(&ctx, fs);
                }
                if self.corner_edit == Some(0) {
                    self.corner_handles(ui, rect, 0);
                } else if self.edit_mapping && self.project.layers[self.sel()].output == 0 {
                    self.output_editor(ui, rect, 0, "output");
                } else if self.output_fullscreen {
                    ctx.set_cursor_icon(egui::CursorIcon::None);
                }
            });
    }
}

pub(super) fn output_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("oz_output")
}

pub(super) fn extra_viewport_id(k: usize) -> ViewportId {
    ViewportId::from_hash_of(("oz_output", k))
}

/// État d'une sortie 2 et suivantes : fenêtre ouverte, plein écran, taille, déplacement en cours.
pub(super) struct ExtraOut {
    pub(super) open: bool,
    pub(super) fullscreen: bool,
    pub(super) size: Vec2,
    pub(super) mv: Option<OutputMove>,
}

impl Default for ExtraOut {
    fn default() -> Self {
        Self { open: true, fullscreen: false, size: Vec2::new(1920.0, 1080.0), mv: None }
    }
}

/// Déplacement de la fenêtre de sortie vers un autre écran, en plusieurs temps
/// (sortir du plein écran, déplacer, repasser en plein écran).
pub(super) struct OutputMove {
    pub(super) screen: screens::Screen,
    pub(super) stage: u8,
    pub(super) at: f64,
}
