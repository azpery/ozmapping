//! Tableau de presets : cases (calque × colonne), enregistrement, rappel, glisser-déposer, enchaînement.

use super::*;

impl OzApp {
    /// Enregistre l'état du calque `i` (média, effets, réglages) dans la case `k`.
    pub(super) fn record_cell(&mut self, i: usize, k: usize) {
        let mut cell = self.snapshot_cell(i);
        let cfg = &self.project.layers[i];
        cell.name = match (&cell.preset, &cell.clip) {
            (Some(p), _) => p.clone(),
            (None, Some(c)) => c.file_stem().map_or_else(|| file_name(c), |s| s.to_string_lossy().into_owned()),
            (None, None) if cfg.generator.pattern != Pattern::Off => cfg.generator.pattern.label().to_owned(),
            _ => format!("Case {}", k + 1),
        };
        let cells = &mut self.project.layers[i].cells;
        if cells.len() <= k {
            cells.resize(k + 1, None);
        }
        cells[k] = Some(cell);
        self.layers[i].cell = Some(k);
        self.layers[i].cell_pos.remove(&k);
    }

    /// Tous les réglages actuels du calque `i` (média, effets, forme, rythme, mix, lecture), sans nom.
    pub(super) fn snapshot_cell(&self, i: usize) -> Cell {
        let (effects, preset) = self.fx_target(i);
        let player = self.layers[i].active();
        let cfg = &self.project.layers[i];
        // Animations de forme, contours animés et destruction : toujours les réglages actuels
        // (une transition de preset en cours ne les touche pas).
        let effects = effects.with_shape_of(&cfg.effects);
        Cell {
            name: String::new(),
            clip: player.map(|p| p.path.clone()),
            effects,
            preset,
            generator: cfg.generator,
            auto: cfg.auto,
            fit: cfg.fit,
            opacity: cfg.opacity,
            blend: cfg.blend,
            warp: Some(cfg.warp.clone()),
            mask: Some(cfg.mask.clone()),
            react: Some(cfg.react),
            mix: Some(cfg.mix.clone()),
            media_rotate: Some(cfg.media_rotate),
            speed: player.map(VideoPlayer::speed),
            reverse: player.map(VideoPlayer::is_reverse),
            paused: player.map(VideoPlayer::is_paused),
        }
    }

    /// Garde dans la case jouée sur le calque `i` tout ce qui a été réglé depuis qu'elle est lancée.
    pub(super) fn store_active_cell(&mut self, i: usize) {
        let Some(k) = self.layers[i].cell else {
            return;
        };
        if self.project.layers[i].cells.get(k).is_none_or(Option::is_none) {
            return;
        }
        let mut cell = self.snapshot_cell(i);
        if let Some(Some(old)) = self.project.layers[i].cells.get_mut(k) {
            cell.name = std::mem::take(&mut old.name);
            *old = cell;
        }
    }

    /// Avant d'enregistrer le projet : les cases en cours de lecture reprennent leurs derniers réglages.
    pub(super) fn store_active_cells(&mut self) {
        for i in 0..self.layers.len() {
            self.store_active_cell(i);
        }
    }

    /// Joue la case `k` sur le calque `i` : son média (en fondu), ses effets (en transition), ses réglages.
    pub(super) fn play_cell(&mut self, ctx: &egui::Context, i: usize, k: usize) {
        if self.project.layers[i].cells.get(k).is_none_or(Option::is_none) {
            return;
        }
        // La case qu'on quitte garde tout ce qui a été réglé pendant qu'elle jouait.
        self.store_active_cell(i);
        let Some(cell) = self.project.layers[i].cells.get(k).cloned().flatten() else {
            return;
        };
        // On quitte une case : on retient où en était sa vidéo.
        let rt = &mut self.layers[i];
        if let Some(prev) = rt.cell.filter(|&prev| prev != k) {
            let prev_clip = self.project.layers[i].cells.get(prev).cloned().flatten().and_then(|c| c.clip);
            if let Some(p) = rt.active().filter(|p| Some(&p.path) == prev_clip.as_ref()) {
                let pos = p.position();
                rt.cell_pos.insert(prev, pos);
            }
        }
        match &cell.clip {
            // Même média déjà à l'écran : on ne le relance pas, seuls effets et réglages changent.
            Some(path) if self.layers[i].active().is_some_and(|p| p.path == *path) => {}
            Some(path) => {
                // Retour sur une case déjà jouée : la vidéo reprend là où on l'avait laissée.
                let mut opts = video::PlayOptions::new(self.project.looping, video::OUTPUT_MAX);
                opts.start = self.layers[i].cell_pos.get(&k).copied().unwrap_or(0.0);
                opts.speed = cell.speed.unwrap_or(opts.speed);
                opts.reverse = cell.reverse.unwrap_or(opts.reverse);
                self.launch_with(ctx, i, path, opts, None, true);
            }
            None => self.stop_layer(i),
        }
        // Même média gardé à l'écran : on lui applique la vitesse et le sens de la case.
        for p in self.layers[i].players() {
            if let Some(s) = cell.speed {
                p.set_speed(s);
            }
            if let Some(r) = cell.reverse {
                p.set_reverse(r);
            }
            if let Some(paused) = cell.paused {
                p.set_paused(paused);
            }
        }
        let cfg = &mut self.project.layers[i];
        // Mix auto de la case (une ancienne case n'en a pas : le mix changerait de vidéo derrière elle).
        match cell.mix {
            Some(mix) => cfg.mix = mix,
            None => cfg.mix.enabled = false,
        }
        if let Some(w) = cell.warp {
            cfg.warp = w;
        }
        if let Some(m) = cell.mask {
            cfg.mask = m;
        }
        if let Some(r) = cell.media_rotate {
            cfg.media_rotate = r;
        }
        if let Some(r) = cell.react {
            cfg.react = r;
        }
        cfg.generator = cell.generator;
        cfg.auto = cell.auto;
        cfg.fit = cell.fit;
        cfg.opacity = cell.opacity;
        cfg.blend = cell.blend;
        self.apply_preset_to(i, cell.preset, cell.effects, self.project.preset_secs);
        // `apply_preset_to` garde les animations de forme, contours et destruction du calque :
        // ici ce sont celles de la case qu'on veut.
        let fx = &mut self.project.layers[i].effects;
        *fx = fx.with_shape_of(&cell.effects);
        let rt = &mut self.layers[i];
        rt.auto_timer = 0.0;
        rt.cell = Some(k);
    }

    /// Pendant un glisser-déposer de média : miniature et nom sous le pointeur.
    pub(super) fn media_drag_preview(&mut self, ctx: &egui::Context) {
        let Some(drag) = egui::DragAndDrop::payload::<MediaDrag>(ctx) else {
            return;
        };
        let Some(pos) = ctx.pointer_interact_pos() else {
            return;
        };
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::NULL.with("media_drag")));
        let rect = Rect::from_min_size(pos + Vec2::new(12.0, 12.0), Vec2::new(78.0, 44.0));
        painter.rect_filled(rect.expand(3.0), 6.0, theme::PANEL);
        let name = match &*drag {
            MediaDrag::File(path) => {
                match self.thumbs.get(ctx, path) {
                    Some(t) => {
                        let full = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        painter.image(t.id(), rect, full, Color32::WHITE);
                    }
                    None => {
                        painter.rect_filled(rect, 4.0, theme::WIDGET);
                    }
                }
                file_name(path)
            }
            MediaDrag::Pattern(p) => {
                let palette = self.project.layers[self.sel()].generator.palette;
                paint_pattern_swatch(&painter, rect, *p, palette);
                p.label().to_owned()
            }
        };
        painter.rect_stroke(rect.expand(3.0), 6.0, Stroke::new(1.5, WARN), egui::StrokeKind::Outside);
        painter.text(
            rect.left_bottom() + Vec2::new(0.0, 8.0),
            egui::Align2::LEFT_TOP,
            name,
            egui::FontId::proportional(11.0),
            TEXT,
        );
    }

    /// Média déposé sur la case `k` du calque `i` : nouvelle case avec ce média (sans effets),
    /// ou, sur une case pleine, remplacement de son média en gardant ses effets et réglages.
    pub(super) fn drop_media_in_cell(&mut self, ctx: &egui::Context, i: usize, k: usize, media: MediaDrag) {
        let stem = |p: &Path| p.file_stem().map_or_else(|| file_name(p), |s| s.to_string_lossy().into_owned());
        // Nom automatique d'une case : celui de son média (fichier ou motif).
        let auto_name = |c: &Cell| match &c.clip {
            Some(p) => stem(p),
            None => c.generator.pattern.label().to_owned(),
        };
        let fit = self.project.layers[i].fit;
        let cells = &mut self.project.layers[i].cells;
        if cells.len() <= k {
            cells.resize(k + 1, None);
        }
        let cell = cells[k].get_or_insert_with(|| Cell { fit, ..Cell::default() });
        // Nom gardé s'il a été changé à la main ; sinon il suit le média.
        let renamed = !cell.name.is_empty() && cell.name != auto_name(cell);
        match &media {
            MediaDrag::File(path) => {
                cell.clip = Some(path.clone());
                // Une vidéo remplace le visuel généré.
                cell.generator.pattern = Pattern::Off;
            }
            MediaDrag::Pattern(p) => {
                cell.clip = None;
                cell.generator.pattern = *p;
                cell.generator.mix = 1.0;
            }
        }
        if !renamed {
            cell.name = auto_name(cell);
        }
        self.layers[i].cell_pos.remove(&k);
        // Case en cours de lecture : le nouveau média passe tout de suite à l'écran
        // (sinon la case reprendrait l'ancien en gardant l'état du calque).
        if self.layers[i].cell == Some(k) {
            self.launch_any(ctx, i, &media);
        }
    }

    /// Envoie un média (fichier ou visuel généré) sur le calque `i`.
    pub(super) fn launch_any(&mut self, ctx: &egui::Context, i: usize, media: &MediaDrag) {
        match media {
            MediaDrag::File(path) => {
                // Une vidéo envoyée remplace le visuel généré du calque.
                self.project.layers[i].generator.pattern = Pattern::Off;
                self.launch_on(ctx, i, path);
            }
            MediaDrag::Pattern(p) => {
                // Un visuel généré remplace la vidéo du calque, comme un autre média.
                self.stop_layer(i);
                self.project.layers[i].mix.enabled = false;
                let g = &mut self.project.layers[i].generator;
                g.pattern = *p;
                g.mix = 1.0;
            }
        }
    }

    pub(super) fn column_filled(&self, k: usize) -> bool {
        self.project.layers.iter().any(|l| l.cells.get(k).is_some_and(Option::is_some))
    }

    /// Joue toute la colonne `k` : chaque calque qui a une case dans cette colonne la lance.
    pub(super) fn play_column(&mut self, ctx: &egui::Context, k: usize) {
        for i in 0..self.project.layers.len() {
            self.play_cell(ctx, i, k);
        }
        self.chain_col = Some(k);
        self.chain_timer = 0.0;
    }

    /// Enchaînement : colonne remplie suivante (en boucle).
    pub(super) fn chain_next(&mut self, ctx: &egui::Context) {
        let n = self.project.columns;
        let start = self.chain_col.map_or(0, |c| c + 1);
        if let Some(k) = (0..n).map(|d| (start + d) % n).find(|&k| self.column_filled(k)) {
            self.play_column(ctx, k);
        } else {
            self.chain_timer = 0.0;
        }
    }
}

/// Action demandée depuis le tableau.
pub(super) enum CellAction {
    Play(usize, usize),
    Record(usize, usize),
    Clear(usize, usize),
    Column(usize),
    AddColumn,
    /// Média glissé depuis la liste sur la case (calque, colonne).
    Drop(usize, usize, MediaDrag),
}

/// Média en cours de glisser-déposer depuis la liste des médias : un fichier ou un visuel généré.
#[derive(Clone)]
pub(super) enum MediaDrag {
    File(PathBuf),
    Pattern(Pattern),
}

/// Vignette d'un visuel généré : dégradé de sa palette, décalé selon le motif (pour les distinguer).
pub(super) fn paint_pattern_swatch(painter: &egui::Painter, rect: Rect, pattern: Pattern, palette: Palette) {
    const STRIPS: usize = 24;
    let shift = pattern as usize as f32 * 0.137;
    let w = rect.width() / STRIPS as f32;
    for s in 0..STRIPS {
        let t = s as f32 / STRIPS as f32;
        let [r, g, b] = palette.color(t * 0.9 + shift);
        let strip = Rect::from_min_size(egui::pos2(rect.min.x + s as f32 * w, rect.min.y), Vec2::new(w + 0.5, rect.height()));
        painter.rect_filled(strip, 0.0, Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
    }
}

/// Case du tableau : miniature du média, nom en bas ; « + » si vide.
pub(super) fn cell_widget(
    ui: &mut egui::Ui,
    size: Vec2,
    thumb: Option<&egui::TextureHandle>,
    swatch: Option<(Pattern, Palette)>,
    name: Option<&str>,
    active: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let hover = resp.hovered();
    let radius = 4.0;
    match name {
        None => {
            p.rect_filled(rect, radius, if hover { theme::WIDGET } else { CARD });
            let c = if hover { TEXT } else { theme::BORDER };
            p.text(rect.center(), egui::Align2::CENTER_CENTER, ic::PLUS, egui::FontId::proportional(13.0), c);
        }
        Some(name) => {
            p.rect_filled(rect, radius, theme::WIDGET_ACTIVE);
            if let Some((pattern, palette)) = swatch {
                paint_pattern_swatch(p, rect.shrink(1.0), pattern, palette);
            }
            if let Some(t) = thumb {
                // Miniature recadrée pour remplir la case sans déformation.
                let [tw, th] = t.size();
                let (ta, ra) = (tw as f32 / th.max(1) as f32, rect.width() / rect.height());
                let uv = if ta > ra {
                    let w = ra / ta;
                    Rect::from_min_max(egui::pos2(0.5 - w / 2.0, 0.0), egui::pos2(0.5 + w / 2.0, 1.0))
                } else {
                    let h = ta / ra;
                    Rect::from_min_max(egui::pos2(0.0, 0.5 - h / 2.0), egui::pos2(1.0, 0.5 + h / 2.0))
                };
                let tint = if hover { Color32::WHITE } else { Color32::from_gray(190) };
                p.image(t.id(), rect.shrink(1.0), uv, tint);
            }
            let band = Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - 14.0), rect.max);
            p.rect_filled(band.shrink(1.0), 2.0, Color32::from_black_alpha(170));
            let short: String = if name.chars().count() > 13 {
                name.chars().take(12).chain(['…']).collect()
            } else {
                name.to_owned()
            };
            p.text(
                band.left_center() + Vec2::new(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                short,
                egui::FontId::proportional(10.5),
                TEXT,
            );
        }
    }
    let stroke = if active {
        Stroke::new(2.0, ACCENT)
    } else if hover {
        Stroke::new(1.0, TEXT_WEAK)
    } else {
        Stroke::new(1.0, theme::BORDER)
    };
    p.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
    resp
}
