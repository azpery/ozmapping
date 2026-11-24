//! Onglet Visuels : générateur de motifs, visuels sur le masque, destruction.

use super::*;

/// Calage en cours de la photo du visage : la photo affichée et les points déjà montrés (0..1).
pub(super) struct FaceCalib {
    layer: usize,
    key: u64,
    aspect: f32,
    texture: egui::TextureHandle,
    points: Vec<[f32; 2]>,
}

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
            for (title, patterns) in Pattern::by_family() {
                family_title(ui, title);
                ui.horizontal_wrapped(|ui| {
                    for p in patterns {
                        let resp = ui.selectable_label(g.pattern == p, p.label()).on_hover_text(p.description());
                        if resp.clicked() {
                            g.pattern = p;
                        }
                    }
                });
            }
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
            fit_slider(ui, "Durée d'un motif", egui::Slider::new(&mut g.interval, 2.0..=120.0).suffix(" s"));
            fit_slider(ui, "Fondu", egui::Slider::new(&mut g.transition, 0.0..=10.0).suffix(" s"));
            if g.auto && g.is_active() {
                ui.add(egui::ProgressBar::new((timer / g.interval.max(1.0)).clamp(0.0, 1.0)).desired_height(4.0));
            }
        });
        if next {
            self.generator_next(sel);
        }
        self.shape_fx_ui(ui);
        if sec.0 == 6 {
            self.road_ui(ui);
        }
        if sec.0 == 7 {
            self.face_ui(ui);
        }
    }

    /// Visage qui parle : personnage, expression, parole et mouvements du calque sélectionné.
    fn face_ui(&mut self, ui: &mut egui::Ui) {
        use crate::face::{FaceSettings, Look, characters};
        let sel = self.sel();
        let layer = &mut self.project.layers[sel];
        let (g, photo) = (&mut layer.generator, &mut layer.face_photo);
        let d = FaceSettings::default();
        let dg = crate::generator::Generator::default();
        if g.pattern != Pattern::Face {
            ui.label(
                egui::RichText::new(
                    "Un visage humain en 3D qui parle, cligne des yeux et regarde autour de lui, sur fond noir : projetez-le \
                     sur une tête de mannequin, une sphère ou une boule de cristal pour l'illusion de la tête parlante.",
                )
                .small()
                .color(TEXT_WEAK),
            );
            if ui.button(with_icon(ic::PLAY, "Faire apparaître le visage")).clicked() {
                g.pattern = Pattern::Face;
            }
            return;
        }
        let title = |ui: &mut egui::Ui, text: &str| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(text).strong().color(TEXT));
        };
        let f = &mut g.face;

        title(ui, "Cadrage");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut f.only, true, "Visage seul")
                .on_hover_text("Juste le visage, découpé en ovale sur fond noir : à projeter sur une tête");
            ui.selectable_value(&mut f.only, false, "Tête entière").on_hover_text("Crâne nu, oreilles et haut du cou");
        });
        fx_slider(ui, "Recul (visage plus petit)", &mut g.scale, 0.4..=3.0, dg.scale);

        title(ui, "Personnage");
        ui.horizontal_wrapped(|ui| {
            for (name, c) in characters() {
                if ui.selectable_label(f.same_character(&c), name).clicked() {
                    *f = f.with_character(&c);
                }
            }
        });
        fit_slider(ui, "Âge", egui::Slider::new(&mut f.age, 5.0..=95.0).suffix(" ans").fixed_decimals(0));
        fx_slider(ui, "Genre (féminin → masculin)", &mut f.gender, 0.0..=1.0, d.gender);
        fx_slider(ui, "Corpulence", &mut f.weight, 0.0..=1.0, d.weight);
        fx_slider(ui, "Teint (clair → foncé)", &mut f.skin, 0.0..=1.0, d.skin);
        fx_slider(ui, "Yeux (brun, vert, bleu, gris)", &mut f.eyes, 0.0..=1.0, d.eyes);
        fx_slider(ui, "Sourcils (blond, roux, châtain, noir)", &mut f.hair_color, 0.0..=1.0, d.hair_color);

        title(ui, "Mon visage (photo)");
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(with_icon(ic::IMAGE, "Choisir une photo…"))
                .on_hover_text("Une photo de face, bien éclairée : elle est plaquée sur le visage, qui parle et cligne des yeux avec")
                .clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("Image", crate::mask::IMAGE_EXTENSIONS).pick_file()
            {
                *photo = Some(path);
                // Une nouvelle photo se cale tout de suite, en trois clics.
                self.face_calib_wanted = true;
            }
            if photo.is_some()
                && ui
                    .button(with_icon(ic::EYE, "Caler en 3 clics"))
                    .on_hover_text("Montrez les deux yeux puis la bouche sur la photo : elle se place toute seule sur le visage")
                    .clicked()
            {
                self.face_calib_wanted = true;
            }
            if photo.is_some() && ui.button("Retirer la photo").clicked() {
                *photo = None;
            }
        });
        if let Some(path) = photo.as_ref() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.label(egui::RichText::new(name).small().color(TEXT_WEAK));
            if self.layers[sel].face_photo.as_ref().is_some_and(|(p, loaded)| p == path && loaded.is_none()) {
                ui.label(egui::RichText::new("Photo illisible.").small().color(TEXT_WEAK));
            }
            ui.checkbox(&mut f.photo_eyes, "Garder les yeux de la photo").on_hover_text(
                "Coché : vos yeux, fidèles mais fixes (ils clignent quand même). Décoché : les yeux du modèle, qui \
                 bougent ; choisissez alors leur couleur avec le curseur Yeux",
            );
            ui.label(
                egui::RichText::new(
                    "Pour un résultat fidèle : rendu Naturel, maquillage à zéro, âge et genre proches des vôtres. \
                     Retouches fines du calage :",
                )
                .small()
                .color(TEXT_WEAK),
            );
            fx_slider(ui, "Photo ← →", &mut f.photo_x, -0.5..=0.5, d.photo_x);
            fx_slider(ui, "Photo ↑ ↓", &mut f.photo_y, -0.5..=0.5, d.photo_y);
            fx_slider(ui, "Taille du visage dans la photo", &mut f.photo_zoom, 0.05..=1.5, d.photo_zoom);
            fx_slider(ui, "Étirement vertical", &mut f.photo_stretch, 0.5..=1.8, d.photo_stretch);
            fx_slider(ui, "Rotation °", &mut f.photo_rot, -45.0..=45.0, d.photo_rot);
            fx_slider(ui, "Présence de la photo", &mut f.photo_mix, 0.0..=1.0, d.photo_mix);
        }

        title(ui, "Expression");
        fx_slider(ui, "Sourire (triste → souriant)", &mut f.smile, -1.0..=1.0, d.smile);
        fx_slider(ui, "Sourcils (froncés → levés)", &mut f.brows, -1.0..=1.0, d.brows);
        fx_slider(ui, "Expressivité (mimiques en parlant)", &mut f.expressive, 0.0..=1.0, d.expressive);
        fx_slider(ui, "Maquillage (yeux, lèvres, sourcils)", &mut f.makeup, 0.0..=1.0, d.makeup);

        title(ui, "Parole");
        fx_slider(ui, "Bavardage", &mut f.talk, 0.0..=1.0, d.talk);
        fx_slider(ui, "Articulation", &mut f.articulation, 0.0..=1.0, d.articulation);
        fx_slider(ui, "Débit", &mut g.speed, 0.0..=3.0, dg.speed);
        ui.checkbox(&mut f.follow_sound, "La bouche suit le son").on_hover_text(
            "La bouche s'ouvre avec le volume du son choisi dans l'onglet Rythme (micro, musique, voix) : \
             faites parler le visage avec votre voix ou une bande-son",
        );
        if f.follow_sound && !self.rhythm.active() {
            ui.label(
                egui::RichText::new("Aucun son écouté pour l'instant : choisissez une source dans l'onglet Rythme.")
                    .small()
                    .color(TEXT_WEAK),
            );
        }

        title(ui, "Mouvement");
        fx_slider(ui, "Mouvements de tête", &mut f.motion, 0.0..=1.0, d.motion);
        fx_slider(ui, "Regard qui se promène", &mut f.gaze, 0.0..=1.0, d.gaze);

        title(ui, "Rendu");
        ui.horizontal_wrapped(|ui| {
            for l in Look::ALL {
                ui.selectable_value(&mut f.look, l, l.label()).on_hover_text(l.description());
            }
        });
        fx_slider(ui, "Halo", &mut f.aura, 0.0..=1.0, d.aura);
        ui.label(
            egui::RichText::new(
                "Spectre et Hologramme prennent les couleurs de la palette (Visuels › Couleurs). Le fond est \
                 transparent : la vidéo du calque, s'il y en a une, se voit derrière la tête. Le masque, la \
                 déformation et les effets s'appliquent dessus pour caler le visage sur l'objet.",
            )
            .small()
            .color(TEXT_WEAK),
        );
        self.face_calib_ui(ui.ctx(), sel);
    }

    /// Route cartoon : tracé, météo, voiture et décor du calque sélectionné, pilotés en direct.
    fn road_ui(&mut self, ui: &mut egui::Ui) {
        use crate::road::{Car, Decor, RoadSettings, Weather};
        let sel = self.sel();
        let seed = self.next_random();
        let g = &mut self.project.layers[sel].generator;
        let track = &mut self.layers[sel].road;
        let d = RoadSettings::default();
        let dg = crate::generator::Generator::default();
        if g.pattern != Pattern::Road {
            ui.label(
                egui::RichText::new(
                    "Un voyage en voiture dans un dessin animé psychédélique : la route, la météo, la voiture \
                     et le décor se pilotent en direct.",
                )
                .small()
                .color(TEXT_WEAK),
            );
            if ui.button(with_icon(ic::PLAY, "Prendre la route")).clicked() {
                g.pattern = Pattern::Road;
            }
            return;
        }
        let title = |ui: &mut egui::Ui, text: &str| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(text).strong().color(TEXT));
        };

        title(ui, "Route");
        ui.horizontal_wrapped(|ui| {
            if ui.button("↶ Virage à gauche").on_hover_text("Un virage se forme juste devant la voiture").clicked() {
                track.turn(-1.0);
            }
            if ui.button("Virage à droite ↷").on_hover_text("Un virage se forme juste devant la voiture").clicked() {
                track.turn(1.0);
            }
            if ui.button("Virages en S").clicked() {
                track.chicane();
            }
            if ui.button("Bosse").clicked() {
                track.bump(1.0);
            }
            if ui.button("Creux").clicked() {
                track.bump(-0.8);
            }
            if ui.button("Montagnes russes").on_hover_text("Une série de bosses serrées").clicked() {
                track.rollercoaster();
            }
        });
        fx_slider(ui, "Vitesse", &mut g.speed, 0.0..=3.0, dg.speed);
        fx_slider(ui, "Virages au hasard", &mut g.road.curves, 0.0..=1.0, d.curves);
        fx_slider(ui, "Ondulations au hasard", &mut g.road.bumps, 0.0..=1.0, d.bumps);
        fx_slider(ui, "Collines", &mut g.road.hills, 0.0..=1.0, d.hills);

        title(ui, "Météo");
        let now = g.road.weather();
        ui.horizontal_wrapped(|ui| {
            for w in Weather::ALL {
                if ui.selectable_label(now == Some(w), w.label()).clicked() {
                    g.road.set_weather(w);
                }
            }
            if ui.button(with_icon(ic::LIGHTNING, "Éclair !")).on_hover_text("Un éclair tout de suite").clicked() {
                let mut n = seed | 1;
                track.strike(&mut || {
                    n ^= n << 13;
                    n ^= n >> 7;
                    n ^= n << 17;
                    (n >> 40) as f32 / (1u64 << 24) as f32
                });
            }
        });
        fx_slider(ui, "Nuages", &mut g.road.clouds, 0.0..=1.0, d.clouds);
        fx_slider(ui, "Pluie", &mut g.road.rain, 0.0..=1.0, d.rain);
        fx_slider(ui, "Éclairs", &mut g.road.storm, 0.0..=1.0, d.storm);
        fx_slider(ui, "Lumière du jour", &mut g.road.daylight, 0.0..=1.0, d.daylight);
        fx_slider(ui, "Brouillard", &mut g.road.fog, 0.0..=1.0, d.fog);

        title(ui, "Voiture");
        ui.horizontal_wrapped(|ui| {
            for c in Car::ALL {
                ui.selectable_value(&mut g.road.car, c, c.label());
            }
        });
        fx_slider(ui, "Couleur de la voiture", &mut g.road.car_hue, 0.0..=1.0, d.car_hue);

        title(ui, "Décor");
        ui.horizontal_wrapped(|ui| {
            for c in Decor::ALL {
                ui.selectable_value(&mut g.road.decor, c, c.label());
            }
        });
        fx_slider(ui, "Densité du décor", &mut g.detail, 0.0..=1.0, dg.detail);
        fx_slider(ui, "Délire psychédélique", &mut g.road.psyche, 0.0..=1.0, d.psyche);
        ui.label(
            egui::RichText::new(
                "Les couleurs viennent de la palette (Visuels › Couleurs). Avec le rythme actif et des éclairs réglés, \
                 la foudre tombe sur les temps. Pour garder l'animation : bouton Enregistrer de la barre du haut.",
            )
            .small()
            .color(TEXT_WEAK),
        );
    }

    /// Calage de la photo du visage : on montre les deux yeux puis la bouche, la photo se place seule.
    fn face_calib_ui(&mut self, ctx: &egui::Context, sel: usize) {
        let Some(photo) = self.layers[sel].face_photo.as_ref().and_then(|(_, p)| p.as_ref()) else {
            // Photo pas encore chargée (elle l'est à l'image suivante), ou retirée.
            if self.project.layers[sel].face_photo.is_none() {
                (self.face_calib_wanted, self.face_calib) = (false, None);
            }
            return;
        };
        if self.face_calib.as_ref().is_some_and(|c| c.layer != sel || c.key != photo.key) {
            self.face_calib = None;
        }
        if std::mem::take(&mut self.face_calib_wanted) {
            let image = egui::ColorImage::from_rgba_unmultiplied([photo.width as usize, photo.height as usize], &photo.data);
            self.face_calib = Some(FaceCalib {
                layer: sel,
                key: photo.key,
                aspect: photo.width as f32 / photo.height.max(1) as f32,
                texture: ctx.load_texture("face-photo", image, egui::TextureOptions::LINEAR),
                points: Vec::new(),
            });
        }
        let Some(calib) = &mut self.face_calib else {
            return;
        };
        const STEPS: [&str; 3] = ["Cliquez au centre d'un œil", "Cliquez au centre de l'autre œil", "Cliquez au milieu de la bouche, entre les lèvres"];
        let mut close = false;
        egui::Window::new("Caler la photo sur le visage").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(egui::RichText::new(STEPS[calib.points.len().min(2)]).strong().color(TEXT));
            // La photo tient dans 560 × 520 points.
            let scale = (560.0 / calib.aspect).min(520.0);
            let size = Vec2::new(scale * calib.aspect, scale);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
            let painter = ui.painter_at(rect);
            painter.image(calib.texture.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            for p in &calib.points {
                let at = rect.min + Vec2::new(p[0] * rect.width(), p[1] * rect.height());
                painter.circle_stroke(at, 7.0, Stroke::new(3.0, Color32::BLACK));
                painter.circle_stroke(at, 7.0, Stroke::new(1.5, ACCENT));
            }
            if resp.clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                calib.points.push([(pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height()]);
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(!calib.points.is_empty(), egui::Button::new("Recommencer")).clicked() {
                    calib.points.clear();
                }
                close = ui.button("Annuler").clicked();
            });
        });
        if calib.points.len() >= 3 {
            let (aspect, p) = (calib.aspect, [calib.points[0], calib.points[1], calib.points[2]]);
            let face = &mut self.project.layers[sel].generator.face;
            if let Some(marks) = crate::face_mesh::landmarks(&face.identity()) {
                face.fit_photo(marks, aspect, p[0], p[1], p[2]);
            }
            close = true;
        }
        if close {
            self.face_calib = None;
        }
    }

    /// Onglet Photo : mapping d'un objet photographié, animations proposées, contours animés.
    pub(super) fn photo_ui(&mut self, ui: &mut egui::Ui) {
        let i = self.sel();
        ui.label(egui::RichText::new(with_icon(ic::IMAGE, "Mapper un objet")).strong().color(TEXT));
        ui.label(
            egui::RichText::new(
                "Photographiez l'objet : l'app le détoure (automatiquement, avec retouches à la main), affiche la photo \
                 sur le calque avec son masque autour, et vous la placez où vous voulez sur la sortie.",
            )
            .small()
            .color(TEXT_WEAK),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button(with_icon(ic::SCISSORS, "Photo de l'objet…")).clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("Image", crate::mask::IMAGE_EXTENSIONS).pick_file()
            {
                self.import_mask(&path);
            }
            if ui
                .toggle_value(&mut self.edit_both, with_icon(ic::ARROWS_OUT_CARDINAL, "Placer l'objet"))
                .on_hover_text(
                    "Sur l'aperçu ou la sortie : la croix déplace l'image et son masque ensemble, les coins du cadre \
                     les agrandissent ou les réduisent",
                )
                .changed()
            {
                self.edit_mapping = self.edit_both;
            }
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(with_icon(ic::SPARKLE, "Animations proposées")).strong().color(TEXT));
        ui.label(
            egui::RichText::new("Un clic applique l'animation au calque ; chacune se retouche ensuite dans Effets et Visuels.")
                .small()
                .color(TEXT_WEAK),
        );
        let current = self.layers[i].preset.clone();
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            for look in crate::effects::object_looks() {
                if ui.selectable_label(current.as_deref() == Some(look.name), look.name).on_hover_text(look.hint).clicked() {
                    chosen = Some(look);
                }
            }
        });
        if let Some(look) = chosen {
            let cfg = &mut self.project.layers[i];
            // Contours et animation du masque : posés tout de suite (les presets n'y touchent pas),
            // le reste arrive en fondu.
            cfg.effects = cfg.effects.with_shape_of(&look.effects);
            cfg.auto.enabled = false;
            cfg.generator.auto = false;
            match look.visual {
                Some((pattern, mix)) => {
                    cfg.generator.pattern = pattern;
                    cfg.generator.mix = mix;
                }
                None => cfg.generator.pattern = Pattern::Off,
            }
            self.apply_preset(Some(look.name.to_owned()), look.effects);
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Contours animés").strong().color(TEXT));
        self.lines_fx_ui(ui);
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
