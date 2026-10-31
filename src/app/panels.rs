//! Mise en page : barre du haut, panneau des médias, pré-visualisation, panneau de droite.

use super::*;

impl OzApp {
    pub(super) fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(ic::PROJECTOR_SCREEN).size(22.0).color(ACCENT));
            ui.label(egui::RichText::new("OZ Mapping").size(18.0).strong().color(TEXT));
            ui.add_space(6.0);
            ui.menu_button(with_icon(ic::FOLDER_OPEN, "Projet"), |ui| {
                if ui.button(with_icon(ic::FILE_PLUS, &format!("Nouveau          {CMD}+N"))).clicked() {
                    ui.close();
                    self.new_project(&ctx);
                }
                if ui.button(with_icon(ic::FOLDER_OPEN, &format!("Ouvrir…           {CMD}+O"))).clicked() {
                    ui.close();
                    self.open_project(&ctx);
                }
                ui.separator();
                if ui.button(with_icon(ic::FLOPPY_DISK, &format!("Enregistrer       {CMD}+S"))).clicked() {
                    ui.close();
                    self.save(false);
                }
                if ui.button(with_icon(ic::FLOPPY_DISK, &format!("Enregistrer sous…  Maj+{CMD}+S"))).clicked() {
                    ui.close();
                    self.save(true);
                }
            });
            ui.separator();
            ui.toggle_value(&mut self.output_open, with_icon(ic::MONITOR, "Sortie"))
                .on_hover_text("Ouvrir / fermer la fenêtre de sortie");
            self.screen_menu(ui);
            ui.separator();
            self.tap_widget(ui);
            let mut fs = self.output_fullscreen;
            if ui
                .toggle_value(&mut fs, with_icon(ic::CORNERS_OUT, "Plein écran"))
                .on_hover_text("Sortie en plein écran (F)")
                .changed()
            {
                self.set_fullscreen(&ctx, fs);
            }
            self.record_widget(ui);
            ui.separator();
            let bo = ui
                .toggle_value(&mut self.blackout, with_icon(ic::MOON, "Blackout"))
                .on_hover_text("Fondu au noir (B)");
            if self.blackout {
                bo.highlight();
            }
            let white = ui
                .toggle_value(&mut self.white_screen, with_icon(ic::SUN, "Blanc"))
                .on_hover_text("Projeter un écran blanc (W) : pour photographier l'image projetée ou régler le projecteur");
            if self.white_screen {
                white.highlight();
            }
            let bypass = ui
                .toggle_value(&mut self.bypass_fx, with_icon(ic::PROHIBIT, "Sans effets"))
                .on_hover_text("Affiche les vidéos sans aucun effet, sur tous les calques (E).\nLes réglages sont conservés.");
            if self.bypass_fx {
                bypass.highlight();
            }
            ui.toggle_value(&mut self.grid, with_icon(ic::GRID_FOUR, "Mire"))
                .on_hover_text("Mire de calibration (G)");
            if ui
                .toggle_value(&mut self.edit_mapping, with_icon(ic::POLYGON, "Forme"))
                .on_hover_text("Éditer la déformation ou le masque sur l'aperçu et la sortie (M)")
                .clicked()
                && self.edit_mapping
            {
                self.show_shape_tab(self.edit_target);
            }
            ui.separator();
            ui.label(egui::RichText::new(ic::SUN).size(15.0)).on_hover_text("Luminosité générale (master)");
            ui.add(egui::Slider::new(&mut self.project.master, 0.0..=1.0).show_value(false));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("{:.0} i/s", self.fps)).small().color(TEXT_WEAK))
                    .on_hover_text("Images par seconde de l'interface");
                ui.separator();
                let stats = *self.cpu.lock().unwrap();
                cpu_gauge(ui, stats);
            });
        });
    }

    pub(super) fn media_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            theme::section(ui, ic::FILM_STRIP, "Médias");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(with_icon(ic::PLUS, "Ajouter")).on_hover_text("Ajouter des vidéos ou des photos (ou glisser-déposer)").clicked()
                && let Some(files) = rfd::FileDialog::new()
                    .add_filter("Vidéos et photos", &[VIDEO_EXTENSIONS, crate::mask::IMAGE_EXTENSIONS].concat())
                    .add_filter("Vidéos", VIDEO_EXTENSIONS)
                    .add_filter("Photos", crate::mask::IMAGE_EXTENSIONS)
                    .add_filter("Tous les fichiers", &["*"])
                    .pick_files()
            {
                self.add_media(files);
            }
            });
        });
        ui.label(
            egui::RichText::new("Clic : aperçu · Double-clic, lecture ou 1-9 : envoyer sur le calque")
                .small()
                .color(TEXT_WEAK),
        );
        ui.add_space(2.0);

        let media = self.project.media.clone();
        let cue_path = self.cue.as_ref().map(|c| c.path.clone());
        let mut to_cue = None;
        let mut to_launch = None;
        let mut to_remove = None;
        let mut to_toggle_mix = None;
        let mix_clips = self.project.layers[self.sel()].mix.clips.clone();
        egui::ScrollArea::vertical()
            .id_salt("media")
            .max_height(ui.available_height() * 0.5)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if media.is_empty() {
                    theme::card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new(ic::VIDEO).size(28.0).color(TEXT_WEAK));
                            ui.label(egui::RichText::new("Glissez-déposez vos vidéos et photos ici").color(TEXT_WEAK));
                            ui.label(egui::RichText::new("mp4, mov, webm… jpg, png, heic…").small().color(TEXT_WEAK));
                        });
                    });
                }
                for (i, path) in media.iter().enumerate() {
                    // Calques qui jouent ce média.
                    let on_layers: Vec<String> = self
                        .layers
                        .iter()
                        .enumerate()
                        .filter(|(_, rt)| rt.active().is_some_and(|p| p.path == *path))
                        .map(|(k, _)| format!("C{}", k + 1))
                        .collect();
                    let is_cue = cue_path.as_deref() == Some(path.as_path());
                    let in_mix = mix_clips.contains(path);
                    let thumb = self.thumbs.get(&ctx, path);

                    let playing = !on_layers.is_empty();
                    let row = theme::card()
                        .inner_margin(egui::Margin::same(6))
                        .stroke(Stroke::new(
                            1.0,
                            if is_cue { WARN } else if playing { ACCENT.gamma_multiply(0.6) } else { theme::BORDER },
                        ))
                        .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                        let size = Vec2::new(86.0, 48.0);
                        let img = match &thumb {
                            Some(t) => ui.add(
                                egui::Image::new(t)
                                    .fit_to_exact_size(size)
                                    .corner_radius(4.0)
                                    .sense(Sense::click_and_drag()),
                            ),
                            None => {
                                let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                                ui.painter().rect_filled(r, 4.0, theme::WIDGET);
                                ui.painter().text(
                                    r.center(),
                                    egui::Align2::CENTER_CENTER,
                                    if video::is_still_image(path) { ic::IMAGE } else { ic::FILM_SLATE },
                                    egui::FontId::proportional(18.0),
                                    TEXT_WEAK,
                                );
                                resp
                            }
                        };
                        if i < 9 {
                            // Numéro de raccourci clavier en pastille sur la miniature.
                            let badge = egui::Rect::from_min_size(img.rect.min + Vec2::splat(3.0), Vec2::splat(16.0));
                            ui.painter().rect_filled(badge, 4.0, Color32::from_black_alpha(170));
                            ui.painter().text(
                                badge.center(),
                                egui::Align2::CENTER_CENTER,
                                (i + 1).to_string(),
                                egui::FontId::proportional(10.5),
                                TEXT,
                            );
                        }
                        ui.vertical(|ui| {
                            let name = egui::RichText::new(file_name(path)).strong();
                            let name = if playing { name.color(ACCENT) } else { name.color(TEXT) };
                            let label = ui
                                .add(egui::Label::new(name).truncate().sense(Sense::click_and_drag()))
                                .on_hover_text(format!("{}\nGlisser vers une case du tableau pour en faire un preset", path.display()));
                            ui.horizontal(|ui| {
                                if ui
                                    .add(theme::primary_button(ic::PLAY).small())
                                    .on_hover_text("Envoyer sur le calque sélectionné")
                                    .clicked()
                                {
                                    to_launch = Some(i);
                                }
                                if playing {
                                    ui.label(
                                        egui::RichText::new(with_icon(ic::STACK, &on_layers.join(" ")))
                                            .small()
                                            .color(ACCENT),
                                    )
                                    .on_hover_text("Calques qui jouent ce média");
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui
                                        .add(theme::icon_button(ic::TRASH).small().frame(false))
                                        .on_hover_text("Retirer de la liste")
                                        .clicked()
                                    {
                                        to_remove = Some(i);
                                    }
                                    let mix_icon = egui::RichText::new(ic::FILM_REEL)
                                        .color(if in_mix { ACCENT } else { TEXT_WEAK });
                                    if ui
                                        .add(egui::Button::new(mix_icon).small().frame(false))
                                        .on_hover_text(if in_mix {
                                            "Dans le mix auto du calque sélectionné (cliquer pour retirer)"
                                        } else {
                                            "Ajouter au mix auto du calque sélectionné"
                                        })
                                        .clicked()
                                    {
                                        to_toggle_mix = Some(i);
                                    }
                                });
                            });
                            label
                        })
                        .inner
                        .union(img)
                        })
                        .inner
                    });
                    let resp = row.inner;
                    // Glisser le média : on peut le déposer sur une case du tableau.
                    if resp.drag_started() {
                        egui::DragAndDrop::set_payload(&ctx, MediaDrag::File(path.clone()));
                    }
                    if resp.double_clicked() {
                        to_launch = Some(i);
                    } else if resp.clicked() {
                        to_cue = Some(i);
                    }
                }
            });
        if let Some(i) = to_cue {
            self.cue_media(&ctx, &media[i]);
        }
        if let Some(i) = to_launch {
            self.launch_media(&ctx, i);
        }
        if let Some(i) = to_toggle_mix {
            let sel = self.sel();
            let clips = &mut self.project.layers[sel].mix.clips;
            if clips.contains(&media[i]) {
                clips.retain(|p| *p != media[i]);
            } else {
                clips.push(media[i].clone());
            }
        }
        if let Some(i) = to_remove {
            if cue_path.as_deref() == Some(media[i].as_path()) {
                self.cue = None;
            }
            self.project.media.remove(i);
        }

        ui.add_space(6.0);
        self.generated_media_ui(ui);
        ui.add_space(6.0);
        self.cue_ui(ui);
    }

    /// Visuels générés, présentés comme des médias : envoyer sur un calque, glisser vers une case.
    pub(super) fn generated_media_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        theme::section(ui, ic::SUN, "Visuels générés");
        ui.label(
            egui::RichText::new("Clic : envoyer sur le calque · glisser vers une case du tableau")
                .small()
                .color(TEXT_WEAK),
        );
        let palette = self.project.layers[self.sel()].generator.palette;
        let size = Vec2::new(70.0, 40.0);
        let mut launch = None;
        egui::ScrollArea::vertical().id_salt("generated").max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                for p in Pattern::ALL.into_iter().filter(|p| *p != Pattern::Off) {
                    // Calques qui affichent ce motif.
                    let on_layers: Vec<String> = self
                        .project
                        .layers
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| l.generator.pattern == p)
                        .map(|(k, _)| format!("C{}", k + 1))
                        .collect();
                    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                    let painter = ui.painter();
                    paint_pattern_swatch(painter, rect, p, palette);
                    let band = Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - 14.0), rect.max);
                    painter.rect_filled(band, 0.0, Color32::from_black_alpha(170));
                    let short: String =
                        if p.label().chars().count() > 12 { p.label().chars().take(11).chain(['…']).collect() } else { p.label().to_owned() };
                    painter.text(band.left_center() + Vec2::new(3.0, 0.0), egui::Align2::LEFT_CENTER, short, egui::FontId::proportional(10.0), TEXT);
                    let stroke = if !on_layers.is_empty() {
                        Stroke::new(2.0, ACCENT)
                    } else if resp.hovered() {
                        Stroke::new(1.0, TEXT)
                    } else {
                        Stroke::new(1.0, theme::BORDER)
                    };
                    painter.rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
                    let mut hover = format!("{}\n{}", p.label(), p.description());
                    if !on_layers.is_empty() {
                        hover.push_str(&format!("\nSur : {}", on_layers.join(" ")));
                    }
                    let resp = resp.on_hover_text(hover);
                    if resp.drag_started() {
                        egui::DragAndDrop::set_payload(&ctx, MediaDrag::Pattern(p));
                    }
                    if resp.clicked() {
                        launch = Some(p);
                    }
                }
            });
        });
        if let Some(p) = launch {
            let sel = self.sel();
            self.launch_any(&ctx, sel, &MediaDrag::Pattern(p));
        }
    }

    pub(super) fn cue_ui(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, ic::EYE, "Aperçu");
        let Some(cue) = &self.cue else {
            ui.label(
                egui::RichText::new("Cliquez sur un média pour le regarder ici, sans l'envoyer sur la sortie.")
                    .small()
                    .color(TEXT_WEAK),
            );
            return;
        };
        let w = ui.available_width();
        let aspect = cue.info.width as f32 / cue.info.height.max(1) as f32;
        let size = Vec2::new(w, (w / aspect).min(ui.available_height() * 0.6));
        match (&self.cue_tex, cue.has_frame()) {
            (Some(t), true) => {
                ui.add(
                    egui::Image::new(t)
                        .fit_to_exact_size(size)
                        .maintain_aspect_ratio(true)
                        .corner_radius(6.0),
                );
            }
            _ => {
                let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter().rect_filled(r, 6.0, Color32::BLACK);
            }
        }
        ui.label(egui::RichText::new(file_name(&cue.path)).strong().color(TEXT));
        let details = if cue.info.still {
            format!("Photo  ·  {}×{}", cue.info.width, cue.info.height)
        } else {
            format!(
                "{}×{}  ·  {:.2} i/s  ·  {}",
                cue.info.width,
                cue.info.height,
                cue.info.fps,
                fmt_time(cue.info.duration)
            )
        };
        ui.label(egui::RichText::new(details).small().color(TEXT_WEAK));

        let path = cue.path.clone();
        let mut pos = cue.position() as f32;
        let duration = cue.info.duration;
        let paused = cue.is_paused();
        let mut send = false;
        let mut close = false;
        let mut toggle = false;
        let mut seek = None;
        ui.horizontal(|ui| {
            toggle = ui
                .add(theme::icon_button(if paused { ic::PLAY } else { ic::PAUSE }))
                .on_hover_text(if paused { "Lecture" } else { "Pause" })
                .clicked();
            if duration > 0.0 {
                let resp = ui.add(egui::Slider::new(&mut pos, 0.0..=duration as f32).show_value(false));
                ui.label(egui::RichText::new(fmt_time(pos as f64)).small().color(TEXT_WEAK));
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                    seek = Some(pos as f64);
                }
            }
        });
        ui.horizontal(|ui| {
            let target = &self.project.layers[self.sel()].name;
            send = ui
                .add(theme::primary_button(with_icon(ic::PAPER_PLANE_RIGHT, &format!("Envoyer sur « {target} »"))))
                .clicked();
            close = ui.add(theme::icon_button(ic::X)).on_hover_text("Fermer l'aperçu").clicked();
        });

        if toggle {
            cue.set_paused(!paused);
        }
        if let Some(t) = seek {
            cue.seek(t);
        }
        if send {
            let ctx = ui.ctx().clone();
            self.launch_on(&ctx, self.sel(), &path);
        }
        if close {
            self.cue = None;
        }
    }

    pub(super) fn right_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        // Réglages à gauche (largeur fixée : le panneau ne doit pas s'élargir), onglets collés au bord droit.
        const STRIP_W: f32 = 62.0;
        let content_w = (ui.available_width() - STRIP_W - 14.0).max(120.0);
        let h = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(content_w, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(content_w);
                self.settings_pane(ui);
            });
            ui.separator();
            // Onglets latéraux : les grandes familles de réglages.
            ui.vertical(|ui| {
                ui.set_width(STRIP_W);
                for (tab, icon, name) in [
                    (RightTab::Effects, ic::SPARKLE, "Effets"),
                    (RightTab::Visuals, ic::SUN, "Visuels"),
                    (RightTab::Photo, ic::IMAGE, "Photo"),
                    (RightTab::Shape, ic::POLYGON, "Forme"),
                    (RightTab::Mix, ic::FILM_REEL, "Mix"),
                    (RightTab::Rhythm, ic::WAVES, "Rythme"),
                ] {
                    if side_tab(ui, icon, name, self.right_tab == tab).clicked() {
                        self.right_tab = tab;
                    }
                }
            });
        });
    }

    /// Contenu de l'onglet latéral choisi, avec ses onglets horizontaux.
    pub(super) fn settings_pane(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(with_icon(ic::STACK, &self.project.layers[self.sel()].name))
                .color(ACCENT)
                .small(),
        )
        .on_hover_text("Calque sélectionné : les réglages ci-dessous s'appliquent à lui");
        ui.add_space(2.0);
        // Onglets horizontaux : les sections de l'onglet latéral.
        let sections = self.right_tab.sections();
        if !sections.is_empty() {
            let current = &mut self.sub_tab[self.right_tab as usize];
            *current = (*current).min(sections.len() - 1);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (k, (icon, name)) in sections.iter().enumerate() {
                    let text = egui::RichText::new(with_icon(icon, name)).size(13.0);
                    let text = if *current == k { text.strong().color(TEXT) } else { text.color(TEXT_WEAK) };
                    if ui.selectable_label(*current == k, text).clicked() {
                        *current = k;
                    }
                }
            });
            ui.separator();
        }
        let fx_tab = matches!(self.right_tab, RightTab::Effects | RightTab::Visuals | RightTab::Photo);
        if fx_tab {
            self.hidden_fx_banner(ui);
        }
        let sel = self.sel();
        let before = self.fx_target(sel);
        egui::ScrollArea::vertical()
            .id_salt("right")
            .auto_shrink([false, false])
            .show(ui, |ui| match self.right_tab {
                RightTab::Effects => self.effects_ui(ui),
                RightTab::Visuals => self.generator_ui(ui),
                RightTab::Photo => self.lines_fx_ui(ui),
                RightTab::Shape => self.shape_ui(ui),
                RightTab::Mix => self.mix_ui(ui),
                RightTab::Rhythm => self.rhythm_ui(ui),
            });
        // Un effet vient d'être choisi sur un calque qui ne se voit pas : on le signale.
        // (« Aucun » ne compte pas : retirer les effets d'un calque caché n'a rien d'étonnant.)
        if fx_tab && sel == self.sel() {
            let after = self.fx_target(sel);
            if after != before && after.0 != Effects::default() && self.hidden_reason(sel).is_some() {
                self.fx_hidden_alert = Some(sel);
            }
        }
    }
}
