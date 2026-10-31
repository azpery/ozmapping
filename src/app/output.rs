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
        let avail = ui.available_rect_before_wrap().shrink(14.0);
        let aspect = (self.output_size.x / self.output_size.y.max(1.0)).clamp(0.2, 5.0);
        let size = if avail.width() / avail.height() > aspect {
            Vec2::new(avail.height() * aspect, avail.height())
        } else {
            Vec2::new(avail.width(), avail.width() / aspect)
        };
        let rect = Rect::from_center_size(avail.center(), size);
        ui.allocate_rect(avail, Sense::hover());

        let painter = ui.painter_at(avail.expand(14.0));
        if let Some(r) = self.renderer.clone() {
            let params = self.draw_params();
            // Enregistrement : la sortie est aussi rendue hors écran, à la définition choisie.
            if let Some(rec) = self.recorder.clone() {
                painter.add(renderer::capture_callback(rect, r.clone(), params.clone(), rec));
            }
            painter.add(renderer::paint_callback(rect, r, params));
        }
        painter.rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Outside);
        painter.text(
            rect.left_top() + Vec2::new(2.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{}  Sortie  ·  {:.0}×{:.0}", ic::MONITOR, self.output_size.x, self.output_size.y),
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
        self.shape_editor(ui, rect, "preview");
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
                if let Some(r) = self.renderer.clone() {
                    ui.painter().add(renderer::paint_callback(rect, r, self.draw_params()));
                }
                let bg = ui.interact(rect, ui.scope_id().with("output_bg"), Sense::click());
                if bg.double_clicked() && !self.edit_mapping {
                    let fs = !self.output_fullscreen;
                    self.set_fullscreen(&ctx, fs);
                }
                if self.edit_mapping {
                    self.shape_editor(ui, rect, "output");
                } else if self.output_fullscreen {
                    ctx.set_cursor_icon(egui::CursorIcon::None);
                }
            });
    }
}

pub(super) fn output_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("oz_output")
}

/// Déplacement de la fenêtre de sortie vers un autre écran, en plusieurs temps
/// (sortir du plein écran, déplacer, repasser en plein écran).
pub(super) struct OutputMove {
    pub(super) screen: screens::Screen,
    pub(super) stage: u8,
    pub(super) at: f64,
}
