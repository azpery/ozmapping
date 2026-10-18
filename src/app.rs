use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui::{
    self, Color32, Key, KeyboardShortcut, Modifiers, Rect, Sense, Stroke, Vec2, ViewportBuilder, ViewportCommand,
    ViewportId,
};
use eframe::glow;
use serde::{Deserialize, Serialize};

use crate::cpu::{self, CpuStats};
use crate::effects::{self, BlendMode, Effects, Mirror, Preset};
use crate::icons as ic;
use crate::project::{self, LayerConfig, Project};
use crate::renderer::{self, DrawParams, LayerDraw, MaskDraw, Phases, Renderer, SourceRef};
use crate::surface::{self, ACCENT, MAX_GRID, MaskShape, Warp};
use crate::theme::{self, BG, CARD, TEXT, TEXT_WEAK, WARN, with_icon};
use crate::thumbs::Thumbnails;
use crate::video::{self, VideoPlayer};

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mkv", "mov", "avi", "webm", "mpg", "mpeg", "wmv", "flv", "ts", "mts", "m2ts", "ogv", "gif", "hap",
];
const MAX_LAYERS: usize = 8;
/// Touche de commande affichée dans les libellés (les raccourcis utilisent Ctrl hors macOS).
const CMD: &str = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };

fn output_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("oz_output")
}

/// Session restaurée au lancement (le projet courant, même non enregistré).
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Session {
    project: Option<Project>,
    project_path: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    Warp,
    Mask,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RightTab {
    Effects,
    Shape,
}

/// Transition douce entre deux jeux d'effets (rappel de preset).
struct FxAnim {
    from: Effects,
    to: Effects,
    t: f32,
    secs: f32,
}

/// État d'exécution d'un calque (sa configuration est dans `Project::layers`).
#[derive(Default)]
struct LayerRuntime {
    current: Option<VideoPlayer>,
    next: Option<VideoPlayer>,
    fade: f32,
    fx_anim: Option<FxAnim>,
    /// Temps écoulé depuis le dernier changement du mode Auto.
    auto_timer: f32,
    /// Nom du dernier preset appliqué (pour le surligner, même pendant la transition).
    preset: Option<String>,
    phases: Phases,
    mesh: Option<(Warp, Arc<Vec<f32>>)>,
}

impl LayerRuntime {
    /// Le clip principal (celui qui arrive s'il y a un fondu).
    fn active(&self) -> Option<&VideoPlayer> {
        self.next.as_ref().or(self.current.as_ref())
    }

    fn players(&self) -> impl Iterator<Item = &VideoPlayer> {
        [&self.current, &self.next].into_iter().flatten()
    }

    fn stop(&mut self) {
        self.current = None;
        self.next = None;
        self.fade = 0.0;
    }

    fn mesh(&mut self, warp: &Warp) -> Arc<Vec<f32>> {
        match &self.mesh {
            Some((w, m)) if w == warp => m.clone(),
            _ => {
                let m = Arc::new(warp.mesh());
                self.mesh = Some((warp.clone(), m.clone()));
                m
            }
        }
    }
}

fn source(p: &VideoPlayer) -> SourceRef {
    SourceRef {
        key: p.uid,
        frame: p.slot.clone(),
    }
}

pub struct OzApp {
    project: Project,
    /// Parallèle à `project.layers`.
    layers: Vec<LayerRuntime>,
    selected: usize,
    project_path: Option<PathBuf>,
    /// Dernier état enregistré, pour détecter les modifications.
    saved: Project,

    renderer: Option<Arc<Mutex<Renderer>>>,
    thumbs: Thumbnails,

    /// Pré-visualisation (visible uniquement dans la fenêtre de contrôle).
    cue: Option<VideoPlayer>,
    cue_tex: Option<egui::TextureHandle>,
    cue_frame: u64,

    output_open: bool,
    output_fullscreen: bool,
    output_size: Vec2,

    blackout: bool,
    blackout_level: f32,
    grid: bool,
    /// Contournement global : affiche les vidéos sans effets, sans perdre les réglages.
    bypass_fx: bool,
    edit_mapping: bool,
    edit_target: EditTarget,
    right_tab: RightTab,

    preset_name: String,
    builtin_presets: Vec<Preset>,
    random_seed: u64,
    status: Option<String>,
    title: String,
    start: Instant,
    fps: f32,
    cpu: Arc<Mutex<CpuStats>>,
}

impl OzApp {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Vec<PathBuf>) -> Self {
        let mut status = None;
        let renderer = match cc.gl.as_ref().map(|gl| Renderer::new(gl)) {
            Some(Ok(r)) => Some(Arc::new(Mutex::new(r))),
            Some(Err(e)) => {
                eprintln!("Erreur OpenGL : {e}");
                status = Some(format!("Erreur OpenGL : {e}"));
                None
            }
            None => {
                status = Some("OpenGL indisponible".into());
                None
            }
        };
        if video::tool_command("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            status = Some(if cfg!(windows) {
                "ffmpeg introuvable : placez ffmpeg.exe et ffprobe.exe à côté de l'app, ou installez ffmpeg (winget install ffmpeg)".into()
            } else {
                "ffmpeg introuvable : installez-le (brew install ffmpeg)".into()
            });
        }
        theme::apply(&cc.egui_ctx);

        let mut app = Self {
            project: Project::default(),
            layers: vec![LayerRuntime::default()],
            selected: 0,
            project_path: None,
            saved: Project::default(),
            renderer,
            thumbs: Thumbnails::new(&cc.egui_ctx),
            cue: None,
            cue_tex: None,
            cue_frame: 0,
            output_open: true,
            output_fullscreen: false,
            output_size: Vec2::new(1920.0, 1080.0),
            blackout: false,
            blackout_level: 0.0,
            grid: false,
            bypass_fx: false,
            edit_mapping: false,
            edit_target: EditTarget::Warp,
            right_tab: RightTab::Effects,
            preset_name: String::new(),
            builtin_presets: effects::builtin_presets(),
            random_seed: 0x9E37_79B9_7F4A_7C15,
            status,
            title: String::new(),
            start: Instant::now(),
            fps: 0.0,
            cpu: cpu::spawn_monitor(),
        };

        // Restaure la session précédente.
        let session: Session = cc
            .storage
            .and_then(|st| eframe::get_value(st, eframe::APP_KEY))
            .unwrap_or_default();
        if let Some(mut project) = session.project {
            project.sanitize();
            let saved = session
                .project_path
                .as_deref()
                .and_then(|p| Project::load(p).ok())
                .unwrap_or_default();
            app.set_project(&cc.egui_ctx, project, session.project_path);
            app.saved = saved;
        }

        // Arguments : un projet .ozmap, ou des vidéos (la première est lancée).
        let ctx = &cc.egui_ctx;
        if let Some(proj) = args.iter().find(|p| is_project_file(p)) {
            app.load_project_file(ctx, proj);
        } else if let Some(first) = args.first() {
            let first = std::path::absolute(first).unwrap_or_else(|_| first.clone());
            app.add_media(args);
            app.launch_on(ctx, app.sel(), &first);
        }
        app
    }

    fn sel(&self) -> usize {
        self.selected.min(self.project.layers.len() - 1)
    }

    fn is_dirty(&self) -> bool {
        self.project != self.saved
    }

    // ---------------------------------------------------------------- projet

    fn set_project(&mut self, ctx: &egui::Context, project: Project, path: Option<PathBuf>) {
        self.layers = project.layers.iter().map(|_| LayerRuntime::default()).collect();
        self.project = project;
        self.saved = self.project.clone();
        self.project_path = path;
        self.selected = 0;
        self.cue = None;
        // Relance les vidéos des calques.
        for i in 0..self.project.layers.len() {
            if let Some(clip) = self.project.layers[i].clip.clone() {
                if clip.exists() {
                    self.launch_on(ctx, i, &clip);
                } else {
                    self.status = Some(format!("Média introuvable : {}", clip.display()));
                }
            }
        }
        // Relancer les vidéos ne compte pas comme une modification.
        self.saved = self.project.clone();
    }

    /// Propose d'enregistrer si besoin. Renvoie false si l'utilisateur annule.
    fn confirm_discard(&mut self) -> bool {
        if !self.is_dirty() {
            return true;
        }
        let answer = rfd::MessageDialog::new()
            .set_title("Modifications non enregistrées")
            .set_description("Enregistrer les modifications du projet en cours ?")
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();
        match answer {
            rfd::MessageDialogResult::Yes => self.save(false),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    fn new_project(&mut self, ctx: &egui::Context) {
        if self.confirm_discard() {
            self.set_project(ctx, Project::default(), None);
        }
    }

    fn open_project(&mut self, ctx: &egui::Context) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Projet OZ Mapping", &[project::EXTENSION])
            .pick_file()
        {
            self.load_project_file(ctx, &path);
        }
    }

    fn load_project_file(&mut self, ctx: &egui::Context, path: &Path) {
        match Project::load(path) {
            Ok(p) => {
                self.status = None;
                self.set_project(ctx, p, Some(path.to_path_buf()));
            }
            Err(e) => self.status = Some(format!("Ouverture impossible : {e:#}")),
        }
    }

    /// Enregistre (ou « enregistrer sous » si demandé / pas encore de fichier).
    fn save(&mut self, save_as: bool) -> bool {
        let path = match (&self.project_path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("Projet OZ Mapping", &[project::EXTENSION])
                    .set_file_name(format!("{}.{}", self.project_name(), project::EXTENSION));
                if let Some(dir) = self.project_path.as_ref().and_then(|p| p.parent()) {
                    dialog = dialog.set_directory(dir);
                }
                let Some(mut p) = dialog.save_file() else {
                    return false;
                };
                if p.extension().is_none() {
                    p.set_extension(project::EXTENSION);
                }
                p
            }
        };
        match self.project.save(&path) {
            Ok(()) => {
                self.saved = self.project.clone();
                self.project_path = Some(path);
                self.status = None;
                true
            }
            Err(e) => {
                self.status = Some(format!("Enregistrement impossible : {e:#}"));
                false
            }
        }
    }

    fn project_name(&self) -> String {
        self.project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "Sans titre".into(), |s| s.to_string_lossy().into_owned())
    }

    // ---------------------------------------------------------------- médias / lecture

    fn add_media(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            // `absolute` plutôt que `canonicalize` : sous Windows ce dernier produit des chemins
            // « \\?\C:\… » que ffmpeg gère mal.
            let p = std::path::absolute(&p).unwrap_or(p);
            if is_video_file(&p) && !self.project.media.contains(&p) {
                self.project.media.push(p);
            }
        }
    }

    fn launch_on(&mut self, ctx: &egui::Context, layer: usize, path: &Path) {
        let Some(rt) = self.layers.get_mut(layer) else {
            return;
        };
        // Un fondu est déjà en cours : on le termine net.
        if let Some(next) = rt.next.take() {
            rt.current = Some(next);
        }
        rt.fade = 0.0;
        let repaint_ctx = ctx.clone();
        match VideoPlayer::open(path, self.project.looping, video::OUTPUT_MAX, move || {
            repaint_ctx.request_repaint()
        }) {
            Ok(player) => {
                rt.next = Some(player);
                self.project.layers[layer].clip = Some(path.to_path_buf());
                self.status = None;
            }
            Err(e) => self.status = Some(format!("{} : {e:#}", file_name(path))),
        }
    }

    fn launch_media(&mut self, ctx: &egui::Context, index: usize) {
        if let Some(path) = self.project.media.get(index).cloned() {
            self.launch_on(ctx, self.sel(), &path);
        }
    }

    fn launch_relative(&mut self, ctx: &egui::Context, delta: isize) {
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

    fn stop_layer(&mut self, i: usize) {
        self.layers[i].stop();
        self.project.layers[i].clip = None;
    }

    fn toggle_pause(&mut self) {
        let rt = &self.layers[self.sel()];
        if let Some(active) = rt.active() {
            let paused = !active.is_paused();
            for p in rt.players() {
                p.set_paused(paused);
            }
        }
    }

    fn cue_media(&mut self, ctx: &egui::Context, path: &Path) {
        if self.cue.as_ref().is_some_and(|c| c.path == path) {
            return;
        }
        let repaint_ctx = ctx.clone();
        match VideoPlayer::open(path, true, video::PREVIEW_MAX, move || repaint_ctx.request_repaint()) {
            Ok(p) => self.cue = Some(p),
            Err(e) => self.status = Some(format!("{} : {e:#}", file_name(path))),
        }
    }

    // ---------------------------------------------------------------- calques

    fn add_layer(&mut self) {
        if self.project.layers.len() >= MAX_LAYERS {
            return;
        }
        let n = self.project.layers.len() + 1;
        let cfg = LayerConfig {
            name: format!("Calque {n}"),
            ..LayerConfig::default()
        };
        // Nouveau calque juste au-dessus du calque sélectionné.
        let at = self.sel() + 1;
        self.project.layers.insert(at, cfg);
        self.layers.insert(at, LayerRuntime::default());
        self.selected = at;
    }

    fn remove_layer(&mut self, i: usize) {
        if self.project.layers.len() <= 1 {
            return;
        }
        self.project.layers.remove(i);
        self.layers.remove(i);
        self.selected = self.selected.min(self.project.layers.len() - 1);
    }

    fn move_layer(&mut self, i: usize, up: bool) {
        let j = if up { i + 1 } else { i.wrapping_sub(1) };
        if j >= self.project.layers.len() {
            return;
        }
        self.project.layers.swap(i, j);
        self.layers.swap(i, j);
        if self.selected == i {
            self.selected = j;
        } else if self.selected == j {
            self.selected = i;
        }
    }

    fn apply_preset(&mut self, name: Option<String>, fx: Effects) {
        let i = self.sel();
        self.apply_preset_to(i, name, fx, self.project.preset_secs);
        // Un choix manuel reste affiché un intervalle complet avant que le mode Auto ne reprenne la main.
        self.layers[i].auto_timer = 0.0;
    }

    /// Retour à l'image brute sur le calque sélectionné : plus d'effets et mode AUTO coupé.
    fn clear_effects(&mut self) {
        let i = self.sel();
        self.project.layers[i].auto.enabled = false;
        // Immédiat et complet : effets neutres et animations (rotation, teinte…) remises à zéro,
        // pour retrouver tout de suite l'image d'origine, dans le bon sens.
        self.apply_preset_to(i, None, Effects::default(), 0.0);
        self.layers[i].phases = Phases::default();
        self.layers[i].auto_timer = 0.0;
    }

    fn apply_preset_to(&mut self, i: usize, name: Option<String>, fx: Effects, secs: f32) {
        self.layers[i].preset = name;
        if secs <= 0.01 {
            self.project.layers[i].effects = fx;
            self.layers[i].fx_anim = None;
        } else {
            self.layers[i].fx_anim = Some(FxAnim {
                from: self.project.layers[i].effects,
                to: fx,
                t: 0.0,
                secs,
            });
        }
    }

    /// Générateur pseudo-aléatoire (splitmix64), alimenté aussi par l'horloge.
    fn next_random(&mut self) -> u64 {
        self.random_seed = self
            .random_seed
            .wrapping_add(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(self.start.elapsed().as_nanos() as u64 & 0xFFFF);
        let mut z = self.random_seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Mode Auto : passe le calque `i` sur un effet tiré au hasard.
    fn auto_next(&mut self, i: usize) {
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
        let choice = if use_random || candidates.is_empty() {
            (None, Effects::random(roll))
        } else {
            let p = &candidates[(roll % candidates.len() as u64) as usize];
            (Some(p.name.clone()), p.effects)
        };
        self.apply_preset_to(i, choice.0, choice.1, auto.transition);
        self.layers[i].auto_timer = 0.0;
    }

    // ---------------------------------------------------------------- boucle

    fn tick(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if dt > 0.0 {
            self.fps = self.fps * 0.95 + (1.0 / dt) * 0.05;
        }

        for (rt, cfg) in self.layers.iter_mut().zip(&mut self.project.layers) {
            // Fondu enchaîné : démarre dès que la nouvelle vidéo a sa première image.
            if rt.next.as_ref().is_some_and(|n| n.has_frame()) {
                rt.fade += if self.project.fade_secs <= 0.01 { 1.0 } else { dt / self.project.fade_secs };
                if rt.fade >= 1.0 {
                    rt.current = rt.next.take();
                    rt.fade = 0.0;
                }
            }
            if let Some(anim) = &mut rt.fx_anim {
                anim.t += dt / anim.secs.max(0.01);
                cfg.effects = Effects::lerp(&anim.from, &anim.to, anim.t);
                if anim.t >= 1.0 {
                    cfg.effects = anim.to;
                    rt.fx_anim = None;
                }
            }
            rt.phases.advance(&cfg.effects, dt);
            for p in rt.players() {
                if let Some(e) = p.error() {
                    self.status = Some(format!("{} : {e}", file_name(&p.path)));
                }
            }
        }

        // Mode Auto : changement d'effet à intervalle régulier.
        for i in 0..self.layers.len() {
            let auto = self.project.layers[i].auto;
            if auto.enabled {
                self.layers[i].auto_timer += dt;
                if self.layers[i].auto_timer >= auto.interval.max(0.5) {
                    self.auto_next(i);
                }
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

    fn draw_params(&mut self) -> DrawParams {
        let mut layers = Vec::new();
        for (rt, cfg) in self.layers.iter_mut().zip(&self.project.layers) {
            if !cfg.visible || cfg.opacity <= 0.0 {
                continue;
            }
            layers.push(LayerDraw {
                a: rt.current.as_ref().map(source),
                b: rt.next.as_ref().map(source),
                mix: rt.fade.clamp(0.0, 1.0),
                mesh: rt.mesh(&cfg.warp),
                mask: cfg.mask.is_active().then(|| MaskDraw {
                    points: cfg.mask.points.clone(),
                    invert: cfg.mask.invert,
                    feather: cfg.mask.feather,
                }),
                effects: if self.bypass_fx { Effects::default() } else { cfg.effects },
                opacity: cfg.opacity,
                blend: cfg.blend,
                grid: self.grid,
                phases: if self.bypass_fx { Phases::default() } else { rt.phases },
            });
        }
        DrawParams {
            layers,
            time: self.start.elapsed().as_secs_f32(),
            master: self.project.master * (1.0 - self.blackout_level),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |shift: bool, key: Key| {
            let mods = if shift { Modifiers::COMMAND | Modifiers::SHIFT } else { Modifiers::COMMAND };
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)))
        };
        if cmd(true, Key::S) {
            self.save(true);
        }
        if cmd(false, Key::S) {
            self.save(false);
        }
        if cmd(false, Key::O) {
            self.open_project(ctx);
        }
        if cmd(false, Key::N) {
            self.new_project(ctx);
        }

        if ctx.egui_wants_keyboard_input() || ctx.input(|i| i.modifiers.command) {
            return;
        }
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if pressed(Key::Space) {
            self.toggle_pause();
        }
        if pressed(Key::B) {
            self.blackout = !self.blackout;
        }
        if pressed(Key::G) {
            self.grid = !self.grid;
        }
        if pressed(Key::E) {
            self.bypass_fx = !self.bypass_fx;
        }
        if pressed(Key::Num0) {
            self.clear_effects();
        }
        if pressed(Key::A) {
            let i = self.sel();
            let auto = &mut self.project.layers[i].auto;
            auto.enabled = !auto.enabled;
            if auto.enabled {
                self.auto_next(i);
            }
        }
        if pressed(Key::M) {
            self.edit_mapping = !self.edit_mapping;
        }
        if pressed(Key::Tab) {
            self.selected = (self.sel() + 1) % self.project.layers.len();
        }
        if pressed(Key::F) {
            self.set_fullscreen(ctx, !self.output_fullscreen);
        }
        if pressed(Key::Escape) && self.output_fullscreen {
            self.set_fullscreen(ctx, false);
        }
        if pressed(Key::ArrowRight) {
            self.launch_relative(ctx, 1);
        }
        if pressed(Key::ArrowLeft) {
            self.launch_relative(ctx, -1);
        }
        let digits = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (i, k) in digits.into_iter().enumerate() {
            if pressed(k) {
                self.launch_media(ctx, i);
            }
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.output_open = true;
        self.output_fullscreen = on;
        ctx.send_viewport_cmd_to(output_viewport_id(), ViewportCommand::Fullscreen(on));
    }

    // ---------------------------------------------------------------- UI

    fn top_bar(&mut self, ui: &mut egui::Ui) {
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
            let mut fs = self.output_fullscreen;
            if ui
                .toggle_value(&mut fs, with_icon(ic::CORNERS_OUT, "Plein écran"))
                .on_hover_text("Sortie en plein écran (F)")
                .changed()
            {
                self.set_fullscreen(&ctx, fs);
            }
            ui.separator();
            let bo = ui
                .toggle_value(&mut self.blackout, with_icon(ic::MOON, "Blackout"))
                .on_hover_text("Fondu au noir (B)");
            if self.blackout {
                bo.highlight();
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
                self.right_tab = RightTab::Shape;
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

    fn media_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            theme::section(ui, ic::FILM_STRIP, "Médias");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(with_icon(ic::PLUS, "Ajouter")).on_hover_text("Ajouter des vidéos (ou glisser-déposer)").clicked()
                && let Some(files) = rfd::FileDialog::new()
                    .add_filter("Vidéos", VIDEO_EXTENSIONS)
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
                            ui.label(egui::RichText::new("Glissez-déposez vos vidéos ici").color(TEXT_WEAK));
                            ui.label(egui::RichText::new("mp4, mkv, mov, webm…").small().color(TEXT_WEAK));
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
                                    .sense(Sense::click()),
                            ),
                            None => {
                                let (r, resp) = ui.allocate_exact_size(size, Sense::click());
                                ui.painter().rect_filled(r, 4.0, theme::WIDGET);
                                ui.painter().text(
                                    r.center(),
                                    egui::Align2::CENTER_CENTER,
                                    ic::FILM_SLATE,
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
                                .add(egui::Label::new(name).truncate().sense(Sense::click()))
                                .on_hover_text(path.display().to_string());
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
        if let Some(i) = to_remove {
            if cue_path.as_deref() == Some(media[i].as_path()) {
                self.cue = None;
            }
            self.project.media.remove(i);
        }

        ui.add_space(6.0);
        self.cue_ui(ui);
    }

    fn cue_ui(&mut self, ui: &mut egui::Ui) {
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
        ui.label(
            egui::RichText::new(format!(
                "{}×{}  ·  {:.2} i/s  ·  {}",
                cue.info.width,
                cue.info.height,
                cue.info.fps,
                fmt_time(cue.info.duration)
            ))
            .small()
            .color(TEXT_WEAK),
        );

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

    fn layers_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let total = ui.available_width();
        ui.horizontal_top(|ui| {
            ui.allocate_ui(Vec2::new(total * 0.64, ui.available_height()), |ui| {
                ui.vertical(|ui| self.layer_list(ui));
            });
            ui.separator();
            ui.vertical(|ui| self.transport(ui));
        });
    }

    fn layer_list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::section(ui, ic::STACK, "Calques");
            ui.add_enabled_ui(self.project.layers.len() < MAX_LAYERS, |ui| {
                if ui
                    .button(with_icon(ic::PLUS, "Calque"))
                    .on_hover_text("Ajouter un calque au-dessus du calque sélectionné")
                    .clicked()
                {
                    self.add_layer();
                }
            });
            ui.label(
                egui::RichText::new("Tab : calque suivant · en haut de la liste = au premier plan")
                    .small()
                    .color(TEXT_WEAK),
            );
        });

        let n = self.project.layers.len();
        let sel = self.sel();
        let mut action: Option<(usize, &str)> = None;
        egui::ScrollArea::vertical().id_salt("layers").show(ui, |ui| {
            for i in (0..n).rev() {
                let selected = i == sel;
                let clip = self.layers[i].active().map(|p| file_name(&p.path));
                let cfg = &mut self.project.layers[i];
                let frame = egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .corner_radius(6.0)
                    .fill(if selected { theme::ACCENT_DIM.gamma_multiply(0.55) } else { CARD })
                    .stroke(Stroke::new(1.0, if selected { ACCENT.gamma_multiply(0.7) } else { theme::BORDER }));
                let row = frame
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (eye, eye_color) =
                                if cfg.visible { (ic::EYE, TEXT) } else { (ic::EYE_SLASH, TEXT_WEAK) };
                            if ui
                                .add(egui::Button::new(egui::RichText::new(eye).size(16.0).color(eye_color)).frame(false))
                                .on_hover_text(if cfg.visible { "Masquer le calque" } else { "Afficher le calque" })
                                .clicked()
                            {
                                cfg.visible = !cfg.visible;
                            }
                            if selected {
                                ui.add(egui::TextEdit::singleline(&mut cfg.name).desired_width(100.0));
                            } else if ui
                                .add_sized(
                                    [100.0, 20.0],
                                    egui::Button::new(egui::RichText::new(cfg.name.as_str()).color(TEXT)).frame(false),
                                )
                                .clicked()
                            {
                                action = Some((i, "select"));
                            }
                            let clip_text = match &clip {
                                Some(c) => with_icon(ic::FILM_SLATE, c),
                                None => "—".into(),
                            };
                            ui.add_sized(
                                [140.0, 20.0],
                                egui::Label::new(egui::RichText::new(clip_text).small().color(TEXT_WEAK)).truncate(),
                            );
                            let badge = egui::RichText::new(ic::MAGIC_WAND).size(15.0);
                            let badge = if cfg.auto.enabled { badge.color(ACCENT) } else { badge.color(Color32::TRANSPARENT) };
                            ui.label(badge).on_hover_text("Mode AUTO actif sur ce calque");
                            ui.label(egui::RichText::new(ic::DROP).color(TEXT_WEAK)).on_hover_text("Opacité");
                            ui.add(egui::Slider::new(&mut cfg.opacity, 0.0..=1.0).show_value(false))
                                .on_hover_text("Opacité");
                            egui::ComboBox::from_id_salt(("blend", i))
                                .width(90.0)
                                .selected_text(cfg.blend.label())
                                .show_ui(ui, |ui| {
                                    for m in BlendMode::ALL {
                                        ui.selectable_value(&mut cfg.blend, m, m.label());
                                    }
                                });
                            if ui.add(theme::icon_button(ic::CARET_UP).small()).on_hover_text("Monter").clicked() {
                                action = Some((i, "up"));
                            }
                            if ui.add(theme::icon_button(ic::CARET_DOWN).small()).on_hover_text("Descendre").clicked() {
                                action = Some((i, "down"));
                            }
                            if ui
                                .add(theme::icon_button(ic::STOP).small())
                                .on_hover_text("Arrêter la vidéo du calque")
                                .clicked()
                            {
                                action = Some((i, "stop"));
                            }
                            if n > 1
                                && ui
                                    .add(theme::icon_button(ic::TRASH).small())
                                    .on_hover_text("Supprimer le calque")
                                    .clicked()
                            {
                                action = Some((i, "remove"));
                            }
                        });
                    })
                    .response;
                // Un clic n'importe où sur la ligne sélectionne le calque, sans capturer le clic :
                // les boutons et curseurs de la ligne continuent de fonctionner.
                let clicked_in_row = ui.input(|inp| {
                    inp.pointer.primary_clicked()
                        && inp.pointer.interact_pos().is_some_and(|p| row.rect.contains(p))
                });
                if clicked_in_row && action.is_none() {
                    action = Some((i, "select"));
                }
            }
        });
        match action {
            Some((i, "select")) => self.selected = i,
            Some((i, "up")) => self.move_layer(i, true),
            Some((i, "down")) => self.move_layer(i, false),
            Some((i, "stop")) => self.stop_layer(i),
            Some((i, "remove")) => self.remove_layer(i),
            _ => {}
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        let i = self.sel();
        theme::section(ui, ic::PLAY, &format!("Lecture · {}", self.project.layers[i].name));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(ic::TIMER).color(TEXT_WEAK)).on_hover_text("Fondu entre deux vidéos");
            ui.add(egui::Slider::new(&mut self.project.fade_secs, 0.0..=5.0).suffix(" s").text("Fondu"));
        });
        if ui.checkbox(&mut self.project.looping, with_icon(ic::REPEAT, "Lecture en boucle")).changed() {
            for p in self.layers.iter().flat_map(|rt| rt.players()) {
                p.set_looping(self.project.looping);
            }
        }
        let Some(player) = self.layers[i].active() else {
            ui.label(egui::RichText::new("Aucune vidéo sur ce calque.").color(TEXT_WEAK));
            return;
        };
        let info = player.info.clone();
        ui.label(egui::RichText::new(with_icon(ic::FILM_SLATE, &file_name(&player.path))).strong().color(TEXT));

        let paused = player.is_paused();
        let mut speed = player.speed();
        let mut pos = player.position() as f32;
        let mut seek_to = None;
        let mut toggle = false;
        ui.horizontal(|ui| {
            toggle = ui
                .add(theme::icon_button(if paused { ic::PLAY } else { ic::PAUSE }))
                .on_hover_text(if paused { "Lecture (Espace)" } else { "Pause (Espace)" })
                .clicked();
            if player.ended() {
                ui.label(egui::RichText::new("terminé").small().color(TEXT_WEAK));
            }
            if info.duration > 0.0 {
                let label = format!("{} / {}", fmt_time(pos as f64), fmt_time(info.duration));
                let resp = ui.add(
                    egui::Slider::new(&mut pos, 0.0..=info.duration as f32)
                        .show_value(false)
                        .text(label),
                );
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                    seek_to = Some(pos as f64);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(ic::SPEEDOMETER).color(TEXT_WEAK)).on_hover_text("Vitesse de lecture");
            ui.add(egui::Slider::new(&mut speed, 0.1..=4.0).logarithmic(true).suffix("×").text("Vitesse"));
            if ui.small_button("1×").clicked() {
                speed = 1.0;
            }
        });

        player.set_speed(speed);
        if let Some(t) = seek_to {
            player.seek(t);
        }
        if toggle {
            self.toggle_pause();
        }
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (tab, icon, name) in [(RightTab::Effects, ic::SPARKLE, "Effets"), (RightTab::Shape, ic::POLYGON, "Forme")] {
                let text = egui::RichText::new(with_icon(icon, name)).size(15.0).strong();
                ui.selectable_value(&mut self.right_tab, tab, text);
            }
        });
        ui.label(
            egui::RichText::new(with_icon(ic::STACK, &self.project.layers[self.sel()].name))
                .color(ACCENT)
                .small(),
        )
        .on_hover_text("Calque sélectionné : les réglages ci-dessous s'appliquent à lui");
        ui.add_space(2.0);
        egui::ScrollArea::vertical()
            .id_salt("right")
            .auto_shrink([false, false])
            .show(ui, |ui| match self.right_tab {
                RightTab::Effects => self.effects_ui(ui),
                RightTab::Shape => self.shape_ui(ui),
            });
    }

    fn effects_ui(&mut self, ui: &mut egui::Ui) {
        self.presets_ui(ui);

        let i = self.sel();
        let fx = &mut self.project.layers[i].effects;
        // Vrai seulement si l'utilisateur a touché un réglage (pas sur une simple mise à jour).
        let mut touched = false;
        let d = Effects::default();
        egui::CollapsingHeader::new(with_icon(ic::PALETTE, "Couleur")).default_open(true).show(ui, |ui| {
            touched |= fx_slider(ui, "Luminosité", &mut fx.brightness, -1.0..=1.0, d.brightness);
            touched |= fx_slider(ui, "Contraste", &mut fx.contrast, 0.0..=3.0, d.contrast);
            touched |= fx_slider(ui, "Saturation", &mut fx.saturation, 0.0..=3.0, d.saturation);
            touched |= fx_slider(ui, "Teinte", &mut fx.hue, 0.0..=1.0, d.hue);
            touched |= fx_slider(ui, "Cycle teinte /s", &mut fx.hue_speed, -1.0..=1.0, d.hue_speed);
            touched |= fx_slider(ui, "Négatif", &mut fx.invert, 0.0..=1.0, d.invert);
            touched |= fx_slider(ui, "Postérisation", &mut fx.posterize, 0.0..=16.0, d.posterize);
            ui.horizontal(|ui| {
                // Copie locale : le sélecteur réécrit sinon la couleur arrondie à chaque image.
                let mut tint = fx.tint;
                if ui.color_edit_button_rgb(&mut tint).changed() {
                    fx.tint = tint;
                    touched = true;
                }
                touched |= fx_slider(ui, "Colorisation", &mut fx.tint_amount, 0.0..=1.0, d.tint_amount);
            });
            touched |= fx_slider(ui, "Contours", &mut fx.edge, 0.0..=1.0, d.edge);
            touched |= fx_slider(ui, "Vignette", &mut fx.vignette, 0.0..=1.0, d.vignette);
        });
        egui::CollapsingHeader::new(with_icon(ic::FRAME_CORNERS, "Géométrie")).default_open(true).show(ui, |ui| {
            touched |= fx_slider(ui, "Zoom", &mut fx.zoom, 0.2..=5.0, d.zoom);
            touched |= fx_slider(ui, "Rotation °", &mut fx.rotate, -180.0..=180.0, d.rotate);
            touched |= fx_slider(ui, "Rotation auto °/s", &mut fx.rotate_speed, -360.0..=360.0, d.rotate_speed);
            ui.horizontal(|ui| {
                touched |= ui.checkbox(&mut fx.flip_x, "Retourner H").changed();
                touched |= ui.checkbox(&mut fx.flip_y, "Retourner V").changed();
            });
            egui::ComboBox::from_label("Miroir")
                .selected_text(fx.mirror.label())
                .show_ui(ui, |ui| {
                    for m in Mirror::ALL {
                        touched |= ui.selectable_value(&mut fx.mirror, m, m.label()).changed();
                    }
                });
            touched |= fx_slider(ui, "Kaléidoscope", &mut fx.kaleido, 0.0..=16.0, d.kaleido);
            touched |= fx_slider(ui, "Rotation kaléido °/s", &mut fx.kaleido_spin, -180.0..=180.0, d.kaleido_spin);
        });
        egui::CollapsingHeader::new(with_icon(ic::WAVES, "Distorsions")).default_open(true).show(ui, |ui| {
            touched |= fx_slider(ui, "Pixelisation", &mut fx.pixelate, 0.0..=1.0, d.pixelate);
            touched |= fx_slider(ui, "Décalage RVB", &mut fx.rgb_shift, 0.0..=0.05, d.rgb_shift);
            touched |= fx_slider(ui, "Ondulation", &mut fx.wave_amp, 0.0..=0.1, d.wave_amp);
            touched |= fx_slider(ui, "Fréquence onde", &mut fx.wave_freq, 0.5..=30.0, d.wave_freq);
            touched |= fx_slider(ui, "Vitesse onde", &mut fx.wave_speed, 0.0..=20.0, d.wave_speed);
        });
        egui::CollapsingHeader::new(with_icon(ic::LIGHTNING, "Rythme")).default_open(true).show(ui, |ui| {
            touched |= fx_slider(ui, "Stroboscope Hz", &mut fx.strobe, 0.0..=20.0, d.strobe);
        });
        // Un réglage manuel interrompt une transition de preset en cours.
        if touched {
            self.layers[i].fx_anim = None;
            self.layers[i].preset = None;
        }
    }

    fn presets_ui(&mut self, ui: &mut egui::Ui) {
        let sel = self.sel();
        let current = self.project.layers[sel].effects;
        let active = self.layers[sel].preset.clone();
        // Surligné : le preset appliqué en dernier sur ce calque, sinon celui qui correspond aux réglages.
        let is_active = |p: &Preset| match &active {
            Some(name) => *name == p.name,
            None => p.effects == current,
        };
        let mut apply: Option<(Option<String>, Effects)> = None;
        let mut auto_next = false;
        let mut clear = false;
        egui::CollapsingHeader::new(with_icon(ic::STAR, "Presets")).default_open(true).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for p in &self.builtin_presets {
                    if ui.selectable_label(is_active(p), &p.name).clicked() {
                        apply = Some((Some(p.name.clone()), p.effects));
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .button(with_icon(ic::SHUFFLE, "Aléatoire"))
                    .on_hover_text("Combinaison d'effets au hasard")
                    .clicked()
                {
                    apply = Some((None, Effects::random(self.next_random())));
                }
                if ui
                    .button(with_icon(ic::PROHIBIT, "Aucun"))
                    .on_hover_text("Revenir à l'image d'origine : retire tous les effets du calque et coupe le mode AUTO (0)")
                    .clicked()
                {
                    clear = true;
                }
                ui.label(egui::RichText::new(ic::TIMER).color(TEXT_WEAK)).on_hover_text("Durée de transition entre presets");
                ui.add(egui::Slider::new(&mut self.project.preset_secs, 0.0..=3.0).suffix(" s"));
            });

            ui.add_space(4.0);
            auto_ui(
                ui,
                &mut self.project.layers[sel].auto,
                self.layers[sel].auto_timer,
                active.as_deref(),
                &mut auto_next,
            );

            ui.add_space(4.0);
            ui.label(egui::RichText::new(with_icon(ic::BOOKMARK_SIMPLE, "Mes presets")).strong().color(TEXT));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.preset_name).hint_text("Nom").desired_width(130.0));
                if ui
                    .button(with_icon(ic::FLOPPY_DISK, "Mémoriser"))
                    .on_hover_text("Enregistrer les effets actuels comme preset")
                    .clicked()
                {
                    let name = if self.preset_name.trim().is_empty() {
                        format!("Preset {}", self.project.presets.len() + 1)
                    } else {
                        self.preset_name.trim().to_owned()
                    };
                    self.project.presets.push(Preset { name: name.clone(), effects: current });
                    self.layers[sel].preset = Some(name);
                    self.preset_name.clear();
                }
            });
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                if self.project.presets.is_empty() {
                    ui.label(egui::RichText::new("Réglez des effets puis « Mémoriser ».").small().color(TEXT_WEAK));
                }
                for (k, p) in self.project.presets.iter().enumerate() {
                    let resp = ui
                        .selectable_label(is_active(p), &p.name)
                        .on_hover_text("Clic : appliquer · Clic droit : supprimer");
                    if resp.clicked() {
                        apply = Some((Some(p.name.clone()), p.effects));
                    }
                    if resp.secondary_clicked() {
                        remove = Some(k);
                    }
                }
            });
            if let Some(k) = remove {
                self.project.presets.remove(k);
            }
        });
        if let Some((name, fx)) = apply {
            self.apply_preset(name, fx);
        }
        if auto_next {
            self.auto_next(sel);
        }
        if clear {
            self.clear_effects();
        }
    }

    fn shape_ui(&mut self, ui: &mut egui::Ui) {
        let aspect = (self.output_size.x / self.output_size.y.max(1.0)).clamp(0.2, 5.0);
        ui.toggle_value(&mut self.edit_mapping, with_icon(ic::PENCIL_SIMPLE, "Éditer sur l'aperçu (M)"));
        ui.horizontal(|ui| {
            ui.label("Éditer :");
            ui.radio_value(&mut self.edit_target, EditTarget::Warp, "Déformation");
            ui.radio_value(&mut self.edit_target, EditTarget::Mask, "Masque");
        });
        ui.label(
            egui::RichText::new(
                "Glissez les points sur l'aperçu ou sur la sortie. Maj = déplacement précis. La croix déplace toute la forme.",
            )
            .small()
            .color(TEXT_WEAK),
        );

        let sel = self.sel();
        let cfg = &mut self.project.layers[sel];
        egui::CollapsingHeader::new(with_icon(ic::GRID_FOUR, "Déformation")).default_open(true).show(ui, |ui| {
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
        egui::CollapsingHeader::new(with_icon(ic::SHAPES, "Masque")).default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut cfg.mask.enabled, "Activer");
                ui.checkbox(&mut cfg.mask.invert, "Inverser");
            });
            ui.add(egui::Slider::new(&mut cfg.mask.feather, 0.0..=200.0).text("Adoucissement").suffix(" px"));
            ui.label("Formes :");
            ui.horizontal_wrapped(|ui| {
                for shape in MaskShape::ALL {
                    if ui.button(shape.label()).clicked() {
                        cfg.mask.points = shape.points(aspect);
                        cfg.mask.enabled = true;
                        self.edit_target = EditTarget::Mask;
                        self.edit_mapping = true;
                    }
                }
            });
            ui.label(
                egui::RichText::new(format!(
                    "{} points · double-clic sur un bord : ajouter un point · clic droit sur un point : le supprimer",
                    cfg.mask.points.len()
                ))
                .small()
                .weak(),
            );
        });
        ui.add_space(4.0);
        if ui.button(with_icon(ic::STACK, "Copier forme et masque vers tous les calques")).clicked() {
            let (warp, mask) = (cfg.warp.clone(), cfg.mask.clone());
            for l in &mut self.project.layers {
                l.warp = warp.clone();
                l.mask = mask.clone();
            }
        }
    }

    /// Poignées d'édition du calque sélectionné (et contours des autres calques).
    fn shape_editor(&mut self, ui: &mut egui::Ui, rect: Rect, salt: &str) {
        if !self.edit_mapping {
            return;
        }
        let sel = self.sel();
        for (i, l) in self.project.layers.iter().enumerate() {
            if i != sel && l.visible {
                surface::draw_warp_outline(ui, rect, &l.warp, Color32::from_white_alpha(60));
            }
        }
        let cfg = &mut self.project.layers[sel];
        match self.edit_target {
            EditTarget::Warp => surface::edit_warp(ui, rect, &mut cfg.warp, salt),
            EditTarget::Mask => surface::edit_mask(ui, rect, &mut cfg.mask, salt),
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
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
            painter.add(renderer::paint_callback(rect, r, self.draw_params()));
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

    fn output_window(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) {
            self.output_open = false;
            self.output_fullscreen = false;
            return;
        }
        if let Some(fs) = ctx.input(|i| i.viewport().fullscreen) {
            self.output_fullscreen = fs;
        }
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

impl eframe::App for OzApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.tick(&ctx);
        self.handle_shortcuts(&ctx);

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(proj) = dropped.iter().find(|p| is_project_file(p)) {
            if self.confirm_discard() {
                self.load_project_file(&ctx, proj);
            }
        } else if !dropped.is_empty() {
            self.add_media(dropped);
        }

        egui::Panel::top("top")
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(12, 4)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(s) = &self.status {
                        ui.label(egui::RichText::new(with_icon(ic::PROHIBIT, s)).color(theme::DANGER));
                    } else {
                        ui.label(
                            egui::RichText::new(format!(
                                "Espace pause  ·  0 sans effet  ·  E tout sans effets  ·  A auto  ·  B blackout  ·  M forme  ·  F plein écran  ·  {CMD}+S enregistrer"
                            ))
                            .small()
                            .color(TEXT_WEAK),
                        );
                    }
                });
            });
        let panel_frame = egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(12, 4));
        egui::Panel::bottom("layers")
            .resizable(true)
            .default_size(200.0)
            .frame(panel_frame)
            .show(ui, |ui| self.layers_panel(ui));
        egui::Panel::left("media")
            .resizable(true)
            .default_size(330.0)
            .frame(panel_frame)
            .show(ui, |ui| self.media_panel(ui));
        egui::Panel::right("effects")
            .resizable(true)
            .default_size(360.0)
            .frame(panel_frame)
            .show(ui, |ui| self.right_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ui, |ui| self.preview(ui));

        if self.output_open {
            ctx.show_viewport_immediate(
                output_viewport_id(),
                ViewportBuilder::default()
                    .with_title("OZ Mapping — Sortie")
                    .with_inner_size([960.0, 540.0]),
                |ui, _class| self.output_window(ui),
            );
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let session = Session {
            project: Some(self.project.clone()),
            project_path: self.project_path.clone(),
        };
        eframe::set_value(storage, eframe::APP_KEY, &session);
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        for rt in &mut self.layers {
            rt.stop();
        }
        self.cue = None;
        if let (Some(gl), Some(r)) = (gl, &self.renderer) {
            r.lock().unwrap().destroy(gl);
        }
    }
}

/// Jauge CPU : l'app avec ses décodeurs ffmpeg, sur fond de charge totale de la machine.
fn cpu_gauge(ui: &mut egui::Ui, stats: CpuStats) {
    // Couleur selon la charge de l'app ; la partie grise de la barre montre le reste de la machine.
    let color = theme::load_color(stats.app);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(96.0, 8.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, theme::WIDGET);
    let w = rect.width();
    let sys = egui::Rect::from_min_size(rect.min, Vec2::new(w * stats.system / 100.0, rect.height()));
    p.rect_filled(sys, 4.0, TEXT_WEAK.gamma_multiply(0.35));
    let app = egui::Rect::from_min_size(rect.min, Vec2::new(w * stats.app / 100.0, rect.height()));
    p.rect_filled(app, 4.0, color);
    resp.on_hover_text(format!(
        "OZ Mapping + {} décodeur(s) ffmpeg : {:.0} % de la machine\nMachine entière : {:.0} %\n{} cœurs",
        stats.decoders, stats.app, stats.system, stats.cores
    ));
    ui.label(egui::RichText::new(format!("{:.0} %", stats.app)).small().color(color));
    ui.label(egui::RichText::new(ic::CPU).size(16.0).color(color));
}

/// Réglages du mode Auto. `next` passe à true si un changement immédiat est demandé.
fn auto_ui(ui: &mut egui::Ui, auto: &mut effects::AutoFx, timer: f32, current: Option<&str>, next: &mut bool) {
    ui.horizontal(|ui| {
        let label = egui::RichText::new(with_icon(ic::MAGIC_WAND, "AUTO")).strong();
        if ui
            .toggle_value(&mut auto.enabled, label)
            .on_hover_text("Enchaîne des effets au hasard sur ce calque (A)")
            .changed()
            && auto.enabled
        {
            *next = true;
        }
        ui.label(egui::RichText::new("Navigation automatique").color(TEXT_WEAK));
        if ui
            .add(theme::icon_button(ic::SKIP_FORWARD))
            .on_hover_text("Passer tout de suite à un autre effet")
            .clicked()
        {
            *next = true;
        }
    });
    if auto.enabled {
        let interval = auto.interval.max(0.5);
        let left = (interval - timer).max(0.0);
        ui.add(
            egui::ProgressBar::new((timer / interval).clamp(0.0, 1.0))
                .desired_height(6.0)
                .fill(ACCENT.gamma_multiply(0.7)),
        );
        let now = current.unwrap_or("effet inédit (« Mémoriser » pour le garder)");
        ui.label(egui::RichText::new(format!("En cours : {now}  ·  prochain dans {left:.0} s")).small().color(TEXT_WEAK));
    }
    ui.add(egui::Slider::new(&mut auto.interval, 1.0..=60.0).text("Changer toutes les").suffix(" s"));
    ui.add(egui::Slider::new(&mut auto.transition, 0.0..=10.0).text("Fondu").suffix(" s"));
    ui.add(
        egui::Slider::new(&mut auto.novelty, 0.0..=1.0)
            .text("Effets inédits")
            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
            .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
    )
    .on_hover_text("Part des changements qui inventent un nouvel effet au lieu de reprendre un preset");
    ui.checkbox(&mut auto.strobe, "Autoriser le stroboscope").on_hover_text("Inclut les presets avec flashs");
}

/// Slider d'effet avec bouton de remise à zéro. Renvoie true si l'utilisateur l'a modifié.
fn fx_slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, default: f32) -> bool {
    ui.horizontal(|ui| {
        // Clamping « Edits » : le slider ne réécrit pas la valeur tant qu'on n'y touche pas
        // (sinon il l'arrondit à chaque image et interrompt les transitions de preset).
        let mut changed = ui
            .add(egui::Slider::new(value, range).text(label).clamping(egui::SliderClamping::Edits))
            .changed();
        if *value != default
            && ui
                .add(theme::icon_button(ic::ARROW_COUNTER_CLOCKWISE).small().frame(false))
                .on_hover_text("Réinitialiser")
                .clicked()
        {
            *value = default;
            changed = true;
        }
        changed
    })
    .inner
}

fn is_video_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

fn is_project_file(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()) == Some(project::EXTENSION)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
