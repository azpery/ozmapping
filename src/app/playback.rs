//! Lecture : lancement des médias sur les calques, fondus, mix et générateur automatiques, boucle d'animation.

use super::*;

impl OzApp {
    pub(super) fn launch_on(&mut self, ctx: &egui::Context, layer: usize, path: &Path) {
        let opts = video::PlayOptions::new(self.project.looping, video::OUTPUT_MAX);
        self.launch_with(ctx, layer, path, opts, None, true);
    }

    /// Lance une vidéo sur un calque, en fondu depuis la vidéo actuelle.
    /// `fade` remplace la durée de fondu générale ; `remember` l'enregistre comme vidéo du calque
    /// dans le projet (pas pour le mix auto, qui change de vidéo en permanence).
    pub(super) fn launch_with(
        &mut self,
        ctx: &egui::Context,
        layer: usize,
        path: &Path,
        opts: video::PlayOptions,
        fade: Option<f32>,
        remember: bool,
    ) {
        let Some(rt) = self.layers.get_mut(layer) else {
            return;
        };
        // Un fondu est déjà en cours : on le termine net.
        if let Some(next) = rt.next.take() {
            rt.current = Some(next);
        }
        rt.fade = 0.0;
        rt.fade_secs = fade;
        let repaint_ctx = ctx.clone();
        match VideoPlayer::open(path, opts, move || repaint_ctx.request_repaint()) {
            Ok(player) => {
                rt.next = Some(player);
                if remember {
                    self.project.layers[layer].clip = Some(path.to_path_buf());
                }
                self.status = None;
            }
            Err(e) => self.status = Some(format!("{} : {e:#}", file_name(path))),
        }
    }

    /// Mix auto : passe le calque `i` à la séquence suivante.
    pub(super) fn mix_next(&mut self, ctx: &egui::Context, i: usize) {
        let mix = self.project.layers[i].mix.clone();
        let current = self.layers[i].active().map(|p| p.path.clone());
        let mut seed = self.next_random();
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32
        };
        let Some(seg) = automix::next_segment(&mix, current.as_deref(), &mut rnd) else {
            return;
        };
        let rt = &mut self.layers[i];
        rt.mix_timer = 0.0;
        rt.mix_len = seg.length;
        let duration = match video::probe(&seg.clip) {
            Ok(info) => info.duration,
            Err(e) => {
                self.status = Some(format!("{} : {e:#}", file_name(&seg.clip)));
                rt.mix_len = 0.5; // on réessaiera avec une autre vidéo
                return;
            }
        };
        // Point de départ : la séquence doit tenir dans la vidéo, dans un sens comme dans l'autre.
        let span = (seg.length * seg.speed) as f64;
        let (lo, hi) = if seg.reverse { (span, duration) } else { (0.0, duration - span) };
        let start = if hi > lo { lo + (hi - lo) * seg.start } else { duration * seg.start };

        match rt.active() {
            Some(p) if seg.same_clip => {
                p.set_reverse(seg.reverse);
                p.set_speed(seg.speed);
                p.seek(start);
            }
            _ => {
                let opts = video::PlayOptions {
                    looping: true,
                    max_size: video::OUTPUT_MAX,
                    start,
                    reverse: seg.reverse,
                    speed: seg.speed,
                };
                let clip = seg.clip.clone();
                self.launch_with(ctx, i, &clip, opts, Some(seg.fade), false);
            }
        }
        self.layers[i].mix_seg = Some(seg);
    }

    pub(super) fn toggle_reverse(&mut self) {
        let rt = &self.layers[self.sel()];
        if let Some(active) = rt.active() {
            let reverse = !active.is_reverse();
            for p in rt.players() {
                p.set_reverse(reverse);
            }
        }
    }

    pub(super) fn launch_media(&mut self, ctx: &egui::Context, index: usize) {
        if let Some(path) = self.project.media.get(index).cloned() {
            // Une vidéo envoyée remplace le visuel généré du calque.
            self.launch_any(ctx, self.sel(), &MediaDrag::File(path));
        }
    }

    pub(super) fn launch_relative(&mut self, ctx: &egui::Context, delta: isize) {
        let n = self.project.media.len() as isize;
        if n == 0 {
            return;
        }
        let current = self.layers[self.sel()]
            .active()
            .and_then(|p| self.project.media.iter().position(|m| *m == p.path));
        let i = current.map_or(0, |i| (i as isize + delta).rem_euclid(n));
        self.launch_media(ctx, i as usize);
    }

    pub(super) fn stop_layer(&mut self, i: usize) {
        self.layers[i].stop();
        self.project.layers[i].clip = None;
    }

    pub(super) fn toggle_pause(&mut self) {
        let rt = &self.layers[self.sel()];
        if let Some(active) = rt.active() {
            let paused = !active.is_paused();
            for p in rt.players() {
                p.set_paused(paused);
            }
        }
    }

    pub(super) fn cue_media(&mut self, ctx: &egui::Context, path: &Path) {
        if self.cue.as_ref().is_some_and(|c| c.path == path) {
            return;
        }
        let repaint_ctx = ctx.clone();
        match VideoPlayer::open(path, video::PlayOptions::new(true, video::PREVIEW_MAX), move || {
            repaint_ctx.request_repaint()
        }) {
            Ok(p) => self.cue = Some(p),
            Err(e) => self.status = Some(format!("{} : {e:#}", file_name(path))),
        }
    }

    /// Générateur pseudo-aléatoire (splitmix64), alimenté aussi par l'horloge.
    pub(super) fn next_random(&mut self) -> u64 {
        self.random_seed = self
            .random_seed
            .wrapping_add(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(self.start.elapsed().as_nanos() as u64 & 0xFFFF);
        let mut z = self.random_seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Générateur : passe le calque `i` sur un autre motif (et souvent une autre palette) au hasard.
    pub(super) fn generator_next(&mut self, i: usize) {
        let g = self.project.layers[i].generator;
        let patterns: Vec<Pattern> = Pattern::ALL.into_iter().filter(|p| *p != Pattern::Off && *p != g.pattern).collect();
        let pattern = patterns[(self.next_random() % patterns.len() as u64) as usize];
        let palette = if self.next_random() % 10 < 7 {
            Palette::ALL[(self.next_random() % Palette::ALL.len() as u64) as usize]
        } else {
            g.palette
        };
        let cfg = &mut self.project.layers[i].generator;
        cfg.pattern = pattern;
        cfg.palette = palette;
        let rt = &mut self.layers[i];
        rt.gen_timer = 0.0;
        rt.gen_next_secs = Some(g.transition);
    }

    /// Mode Auto : passe le calque `i` sur un effet tiré au hasard.
    pub(super) fn auto_next(&mut self, i: usize) {
        let auto = self.project.layers[i].auto;
        // Tirage : effet inédit avec la probabilité `novelty`, sinon un preset.
        let use_random = (self.next_random() % 10_000) as f32 / 10_000.0 < auto.novelty;
        let current = self.layers[i].preset.clone();
        let candidates: Vec<Preset> = self
            .builtin_presets
            .iter()
            .chain(&self.project.presets)
            .filter(|p| auto.strobe || p.effects.strobe <= 0.0)
            .filter(|p| current.as_deref() != Some(p.name.as_str()))
            .cloned()
            .collect();
        let roll = self.next_random();
        let choice = if auto.theme != AutoTheme::All || use_random || candidates.is_empty() {
            (None, Effects::themed(auto.theme, roll, auto.intensity, auto.strobe))
        } else {
            let p = &candidates[(roll % candidates.len() as u64) as usize];
            (Some(p.name.clone()), p.effects)
        };
        self.apply_preset_to(i, choice.0, choice.1, auto.transition);
        self.layers[i].auto_timer = 0.0;
    }

    pub(super) fn tick(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if dt > 0.0 {
            self.fps = self.fps * 0.95 + (1.0 / dt) * 0.05;
        }
        if self.project.chain.enabled {
            self.chain_timer += dt;
            if self.chain_col.is_none() || self.chain_timer >= self.project.chain.secs.max(0.5) {
                self.chain_next(ctx);
            }
        }

        for (rt, cfg) in self.layers.iter_mut().zip(&mut self.project.layers) {
            // Fondu enchaîné : démarre dès que la nouvelle vidéo a sa première image.
            if rt.next.as_ref().is_some_and(|n| n.has_frame()) {
                let secs = rt.fade_secs.unwrap_or(self.project.fade_secs);
                rt.fade += if secs <= 0.01 { 1.0 } else { dt / secs };
                if rt.fade >= 1.0 {
                    rt.current = rt.next.take();
                    rt.fade = 0.0;
                    rt.fade_secs = None;
                }
            }
            if let Some(anim) = &mut rt.fx_anim {
                anim.t += dt / anim.secs.max(0.01);
                // Les animations de forme restent celles du calque (réglables pendant la transition).
                cfg.effects = Effects::lerp(&anim.from, &anim.to, anim.t).with_shape_of(&cfg.effects);
                if anim.t >= 1.0 {
                    cfg.effects = anim.to.with_shape_of(&cfg.effects);
                    rt.fx_anim = None;
                }
            }
            rt.phases.advance(&cfg.effects, dt);

            // Générateur : animation, puis fondu si le motif ou la palette viennent de changer.
            let g = &cfg.generator;
            rt.gen_time = (rt.gen_time + g.speed * dt).rem_euclid(10_000.0);
            rt.gen_hue = (rt.gen_hue + g.color_speed * dt).rem_euclid(1.0);
            if (g.pattern, g.palette) != rt.gen_shown {
                rt.gen_from = rt.gen_shown;
                rt.gen_shown = (g.pattern, g.palette);
                rt.gen_fade = 0.0;
                rt.gen_fade_secs = rt.gen_next_secs.take().unwrap_or(0.8);
            }
            rt.gen_fade = if rt.gen_fade_secs <= 0.01 { 1.0 } else { (rt.gen_fade + dt / rt.gen_fade_secs).min(1.0) };
            if g.auto && g.is_active() {
                rt.gen_timer += dt;
            }
            if let Some(t) = &mut rt.shatter_shot {
                *t += dt / SHATTER_SHOT_SECS;
                if *t >= 1.0 {
                    rt.shatter_shot = None;
                }
            }

            for p in rt.players() {
                if let Some(e) = p.error() {
                    self.status = Some(format!("{} : {e}", file_name(&p.path)));
                }
            }
        }

        for i in 0..self.layers.len() {
            let g = self.project.layers[i].generator;
            if g.auto && g.is_active() && self.layers[i].gen_timer >= g.interval.max(1.0) {
                self.generator_next(i);
            }
        }

        // Rythme : réactions sur les temps.
        let pulse = self.rhythm.update(self.clock());
        let rhythm_on = self.rhythm.active();
        let counts_beats = self.rhythm.source != rhythm::Source::Manual;
        for i in 0..self.layers.len() {
            let (react, auto_on, mix_on) = {
                let cfg = &self.project.layers[i];
                (cfg.react, cfg.auto.enabled, cfg.mix.enabled && !cfg.mix.clips.is_empty())
            };
            let rt = &mut self.layers[i];
            if !react.enabled || !rhythm_on {
                rt.beat = rhythm::BeatState::default();
                continue;
            }
            rt.beat.step(&react, pulse.beat, dt);
            if pulse.beat && counts_beats {
                if auto_on && react.auto_beats > 0 {
                    rt.auto_beats += 1;
                    if rt.auto_beats >= react.auto_beats {
                        rt.auto_beats = 0;
                        self.auto_next(i);
                    }
                }
                let rt = &mut self.layers[i];
                if mix_on && react.mix_beats > 0 {
                    rt.mix_beats += 1;
                    if rt.mix_beats >= react.mix_beats {
                        rt.mix_beats = 0;
                        self.mix_next(ctx, i);
                    }
                }
            }
        }

        // Mode Auto : changement d'effet à intervalle régulier (ou sur les temps, plus haut).
        for i in 0..self.layers.len() {
            let auto = self.project.layers[i].auto;
            if auto.enabled && !self.on_beats(i, |r| r.auto_beats) {
                self.layers[i].auto_timer += dt;
                if self.layers[i].auto_timer >= auto.interval.max(0.5) {
                    self.auto_next(i);
                }
            }
        }

        // Mix auto : séquence suivante quand la durée prévue est écoulée.
        for i in 0..self.layers.len() {
            let mix = &self.project.layers[i].mix;
            if !mix.enabled || mix.clips.is_empty() {
                continue;
            }
            let rt = &mut self.layers[i];
            // Le temps ne compte qu'une fois l'image affichée (pas pendant l'ouverture du fichier).
            if rt.active().is_some_and(VideoPlayer::has_frame) {
                rt.mix_timer += dt;
            }
            let timed_out = rt.mix_timer >= rt.mix_len && !self.on_beats(i, |r| r.mix_beats);
            if self.layers[i].active().is_none() || timed_out {
                self.mix_next(ctx, i);
            }
        }

        // Image de pré-visualisation → texture egui.
        if let Some(cue) = &self.cue {
            let guard = cue.slot.lock().unwrap();
            if let Some(frame) = guard.as_ref()
                && frame.id != self.cue_frame
            {
                self.cue_frame = frame.id;
                let image =
                    egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.data);
                match &mut self.cue_tex {
                    Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                    None => self.cue_tex = Some(ctx.load_texture("cue", image, egui::TextureOptions::LINEAR)),
                }
            }
        }

        let target = if self.blackout { 1.0 } else { 0.0 };
        let step = dt / 0.4;
        self.blackout_level += (target - self.blackout_level).clamp(-step, step);

        let title = format!(
            "OZ Mapping — {}{}",
            self.project_name(),
            if self.is_dirty() { " *" } else { "" }
        );
        if title != self.title {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        // Vidéo en lecture → on redessine en continu.
        ctx.request_repaint();
    }

    /// Le calque `i` change sur les temps (AUTO ou mix, selon `n`) plutôt qu'à la durée.
    /// En mode manuel il n'y a pas de temps réguliers : AUTO et mix gardent leur durée,
    /// les appuis sur T ne pilotent que les réactions.
    pub(super) fn on_beats(&self, i: usize, n: impl Fn(&rhythm::React) -> u32) -> bool {
        let react = &self.project.layers[i].react;
        react.enabled && n(react) > 0 && self.rhythm.active() && self.rhythm.source != rhythm::Source::Manual
    }

    pub(super) fn draw_params(&mut self) -> DrawParams {
        let mut layers = Vec::new();
        // Le masque est rastérisé à 1024 px de large, aux proportions de la sortie.
        let aspect = (self.output_size.x / self.output_size.y.max(1.0)).clamp(0.2, 5.0);
        let mask_size = (1024u32, ((1024.0 / aspect).round() as u32).clamp(64, 4096));
        let to_mask_px = mask_size.0 as f32 / self.output_size.x.max(1.0);
        let rhythm_on = self.rhythm.active();
        let pulse = self.rhythm.pulse;
        // Champ de forme : moitié de la définition du masque (il varie en douceur).
        let shape_size = (mask_size.0 / 2, mask_size.1 / 2);
        for (rt, cfg) in self.layers.iter_mut().zip(&self.project.layers) {
            if !cfg.visible || cfg.opacity <= 0.0 {
                continue;
            }
            let effects = if self.bypass_fx {
                Effects::default()
            } else if cfg.react.enabled && rhythm_on {
                cfg.react.apply(&cfg.effects, &pulse, &rt.beat)
            } else {
                cfg.effects
            };
            let feather = cfg.mask.feather / self.output_size.y.max(1.0) / 2.0;
            layers.push(LayerDraw {
                a: rt.current.as_ref().map(source),
                b: rt.next.as_ref().map(source),
                mix: rt.fade.clamp(0.0, 1.0),
                mesh: rt.mesh(&cfg.warp),
                mask: cfg
                    .mask
                    .is_active()
                    .then(|| rt.mask(&cfg.mask, mask_size, cfg.mask.feather * to_mask_px)),
                shape: (cfg.mask.is_active() && effects.uses_shape())
                    .then(|| rt.shape(&cfg.mask, shape_size, feather)),
                generator: GenDraw {
                    from: rt.gen_from.0,
                    from_palette: rt.gen_from.1,
                    to: rt.gen_shown.0,
                    to_palette: rt.gen_shown.1,
                    fade: rt.gen_fade,
                    mix: cfg.generator.mix,
                    time: rt.gen_time,
                    hue: cfg.generator.hue + rt.gen_hue,
                    scale: cfg.generator.scale,
                    detail: cfg.generator.detail,
                    aspect,
                },
                // Destruction : réglage manuel, boucle ou « Casser ! », le plus avancé des trois.
                shatter: effects
                    .shatter_amount
                    .max(Effects::shatter_cycle(rt.phases.shatter))
                    .max(rt.shatter_shot.map_or(0.0, Effects::shatter_cycle)),
                shatter_center: if cfg.mask.is_active() { cfg.mask.centroid() } else { [0.5, 0.5] },
                effects,
                opacity: cfg.opacity,
                blend: cfg.blend,
                fit: cfg.fit,
                media_rotate: cfg.media_rotate,
                surface_aspect: cfg.warp.aspect((self.output_size.x, self.output_size.y)),
                grid: self.grid,
                phases: if self.bypass_fx { Phases::default() } else { rt.phases },
            });
        }
        DrawParams {
            layers,
            time: self.start.elapsed().as_secs_f32(),
            master: self.project.master * (1.0 - self.blackout_level),
            white: self.white_screen || self.cutout.as_ref().is_some_and(|ed| ed.white),
        }
    }

    /// Horloge du rythme (secondes depuis le lancement).
    pub(super) fn clock(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Tap tempo : passe en mode tempo si aucun rythme n'est actif.
    pub(super) fn tap(&mut self) {
        let now = self.clock();
        self.rhythm.tap(now);
    }
}
