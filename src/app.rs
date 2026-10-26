use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui::{
    self, Color32, Key, KeyboardShortcut, Modifiers, Rect, Sense, Stroke, Vec2, ViewportBuilder, ViewportCommand,
    ViewportId,
};
use eframe::glow;
use serde::{Deserialize, Serialize};

use crate::automix::{self, AutoMix};
use crate::cpu::{self, CpuStats};
use crate::cutout::{self, Cutout};
use crate::effects::{self, AutoTheme, BlendMode, Effects, Mirror, Object3d, Preset, Shatter};
use crate::generator::{Palette, Pattern};
use crate::icons as ic;
use crate::project::{self, Cell, LayerConfig, Project};
use crate::rhythm::{self, Rhythm};
use crate::screens;
use crate::renderer::{self, DrawParams, GenDraw, LayerDraw, MaskDraw, Phases, Renderer, ShapeDraw, SourceRef};
use crate::surface::{self, ACCENT, Fit, MAX_GRID, Mask, MaskShape, Warp};
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
    /// Écran choisi pour la sortie.
    output_screen: Option<String>,
    rhythm_source: rhythm::Source,
    rhythm_band: rhythm::Band,
    bpm: Option<f32>,
}

/// Détourage d'une photo en cours (fenêtre « Masque depuis une photo »).
struct CutoutEditor {
    cut: Cutout,
    name: String,
    tex: Option<egui::TextureHandle>,
    dirty: bool,
    /// Début du cadre en cours de tracé (pixels de l'image de travail).
    drag_from: Option<egui::Pos2>,
    /// Coin de l'image projetée en cours de déplacement.
    drag_corner: Option<usize>,
    /// La sortie affiche un écran blanc (pour photographier l'image projetée).
    white: bool,
}

/// Déplacement de la fenêtre de sortie vers un autre écran, en plusieurs temps
/// (sortir du plein écran, déplacer, repasser en plein écran).
struct OutputMove {
    screen: screens::Screen,
    stage: u8,
    at: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    Warp,
    Mask,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RightTab {
    Effects,
    Visuals,
    Photo,
    Shape,
    Mix,
    Rhythm,
}

impl RightTab {
    const COUNT: usize = 6;

    /// Onglets horizontaux de chaque onglet latéral (vide : une seule page).
    fn sections(self) -> &'static [(&'static str, &'static str)] {
        match self {
            RightTab::Effects => &[
                (ic::STAR, "Presets"),
                (ic::PALETTE, "Couleur"),
                (ic::FRAME_CORNERS, "Géométrie"),
                (ic::WAVES, "Distorsions"),
                (ic::SPIRAL, "Psyché"),
                (ic::MONITOR, "Rétro"),
                (ic::CUBE, "3D"),
                (ic::LIGHTNING, "Strobo"),
            ],
            RightTab::Visuals => &[
                (ic::SPIRAL, "Motif"),
                (ic::PALETTE, "Couleurs"),
                (ic::SLIDERS_HORIZONTAL, "Réglages"),
                (ic::SHUFFLE, "Enchaînement"),
                (ic::SHAPES, "Sur le masque"),
                (ic::LIGHTNING, "Destruction"),
            ],
            RightTab::Shape => &[
                (ic::FRAME_CORNERS, "Cadrage"),
                (ic::GRID_FOUR, "Déformation"),
                (ic::SHAPES, "Masque"),
            ],
            RightTab::Rhythm => &[(ic::WAVES, "Source"), (ic::SPARKLE, "Réaction")],
            RightTab::Photo | RightTab::Mix => &[],
        }
    }
}

/// Onglet horizontal affiché dans l'onglet latéral courant.
#[derive(Clone, Copy)]
struct Section(usize);

impl Section {
    /// N'affiche `add` que si la section `n` est l'onglet choisi.
    fn show(self, ui: &mut egui::Ui, n: usize, add: impl FnOnce(&mut egui::Ui)) {
        if self.0 == n {
            add(ui);
        }
    }
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
    /// Durée du fondu en cours, si elle diffère du réglage général (mix auto).
    fade_secs: Option<f32>,
    fx_anim: Option<FxAnim>,
    /// Mix auto : temps écoulé dans la séquence, durée prévue, séquence en cours.
    mix_timer: f32,
    mix_len: f32,
    mix_seg: Option<automix::Segment>,
    /// Temps écoulé depuis le dernier changement du mode Auto.
    auto_timer: f32,
    /// Rythme : temps comptés pour AUTO et le mix, état des réactions.
    auto_beats: u32,
    mix_beats: u32,
    beat: rhythm::BeatState,
    /// Nom du dernier preset appliqué (pour le surligner, même pendant la transition).
    preset: Option<String>,
    phases: Phases,
    mesh: Option<(Warp, Arc<Vec<f32>>)>,
    /// Dernière image du masque, avec ce qui a servi à la calculer.
    mask_raster: Option<(Mask, (u32, u32), u32, MaskDraw)>,
    /// Dernier champ de forme du masque (animations de forme), avec la forme qui l'a produit.
    shape_field: Option<(Mask, (u32, u32), ShapeDraw)>,
    /// Générateur : temps d'animation et rotation de la palette (intégrés image par image).
    gen_time: f32,
    gen_hue: f32,
    /// Motif affiché, motif d'où part le fondu en cours, avancement du fondu (1 = fini).
    gen_shown: (Pattern, Palette),
    gen_from: (Pattern, Palette),
    gen_fade: f32,
    gen_fade_secs: f32,
    /// Durée imposée au prochain fondu (mode auto) ; sinon fondu court d'un changement manuel.
    gen_next_secs: Option<f32>,
    /// Temps écoulé depuis le dernier changement automatique de motif.
    gen_timer: f32,
    /// Destruction déclenchée à la main (« Casser ! ») : avancement du cycle (0..1).
    shatter_shot: Option<f32>,
    /// Dernière case du tableau jouée sur ce calque.
    cell: Option<usize>,
    /// Position de la vidéo de chaque case quittée (par colonne), pour y reprendre au retour.
    cell_pos: std::collections::HashMap<usize, f64>,
}

/// Durée d'une destruction déclenchée à la main (cassure, pause, reconstruction).
const SHATTER_SHOT_SECS: f32 = 5.0;

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

    /// Image du masque, recalculée seulement si la forme, la taille ou l'adoucissement changent.
    fn mask(&mut self, mask: &Mask, size: (u32, u32), feather_px: f32) -> MaskDraw {
        let feather_key = (feather_px * 4.0).round() as u32;
        if let Some((m, s, f, draw)) = &self.mask_raster
            && m == mask
            && *s == size
            && *f == feather_key
        {
            return draw.clone();
        }
        static NEXT_MASK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let draw = MaskDraw {
            key: NEXT_MASK.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            width: size.0,
            height: size.1,
            data: Arc::new(crate::mask::rasterize(mask, size.0, size.1, feather_px)),
            invert: mask.invert,
        };
        self.mask_raster = Some((mask.clone(), size, feather_key, draw.clone()));
        draw
    }

    /// Champ de forme du masque, recalculé seulement si les contours ou la taille changent.
    fn shape(&mut self, mask: &Mask, size: (u32, u32), feather: f32) -> ShapeDraw {
        // Seuls les contours comptent (l'inversion et l'adoucissement sont appliqués au rendu).
        let outline = Mask { enabled: true, invert: false, feather: 0.0, ..mask.clone() };
        let draw = match &self.shape_field {
            Some((m, s, draw)) if *m == outline && *s == size => draw.clone(),
            _ => {
                static NEXT_SHAPE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                let draw = ShapeDraw {
                    key: NEXT_SHAPE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    width: size.0,
                    height: size.1,
                    data: Arc::new(crate::mask::shape_field(&outline, size.0, size.1)),
                    feather: 0.0,
                };
                self.shape_field = Some((outline, size, draw.clone()));
                draw
            }
        };
        ShapeDraw { feather, ..draw }
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
    output_screen: Option<String>,
    output_move: Option<OutputMove>,
    screens: Vec<screens::Screen>,
    screens_at: f64,
    rhythm: Rhythm,
    cutout: Option<CutoutEditor>,

    blackout: bool,
    blackout_level: f32,
    /// Sortie en blanc plein (pour photographier l'image projetée, régler le projecteur).
    white_screen: bool,
    grid: bool,
    /// Contournement global : affiche les vidéos sans effets, sans perdre les réglages.
    bypass_fx: bool,
    edit_mapping: bool,
    edit_target: EditTarget,
    right_tab: RightTab,
    /// Calque dont on vient de changer les effets alors qu'il ne se voit pas à l'écran.
    fx_hidden_alert: Option<usize>,
    /// Onglet horizontal choisi dans chaque onglet latéral.
    sub_tab: [usize; RightTab::COUNT],
    /// Tableau : dernière colonne lancée, temps écoulé depuis (enchaînement auto),
    /// largeur de la carte d'un calque (pour aligner les en-têtes de colonnes).
    chain_col: Option<usize>,
    chain_timer: f32,
    layer_card_w: f32,
    /// Défilement horizontal commun aux lignes du tableau.
    grid_offset: f32,

    preset_name: String,
    mask_preset_name: String,
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
            output_screen: None,
            output_move: None,
            screens: screens::list(),
            screens_at: 0.0,
            rhythm: Rhythm::default(),
            cutout: None,
            blackout: false,
            blackout_level: 0.0,
            white_screen: false,
            grid: false,
            bypass_fx: false,
            edit_mapping: false,
            edit_target: EditTarget::Warp,
            right_tab: RightTab::Effects,
            fx_hidden_alert: None,
            sub_tab: [0; RightTab::COUNT],
            chain_col: None,
            chain_timer: 0.0,
            layer_card_w: 0.0,
            grid_offset: 0.0,
            preset_name: String::new(),
            mask_preset_name: String::new(),
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
        // La sortie retourne sur son écran (en plein écran s'il ne s'agit pas de l'écran principal).
        if let Some(screen) = app.screens.iter().find(|s| Some(&s.key) == session.output_screen.as_ref()) {
            if !screen.primary {
                app.output_move = Some(OutputMove { screen: screen.clone(), stage: 0, at: 0.0 });
            }
            app.output_screen = session.output_screen.clone();
        }
        if let Some(bpm) = session.bpm {
            app.rhythm.tempo.bpm = bpm;
        }
        app.rhythm.band = session.rhythm_band;
        if session.rhythm_source != rhythm::Source::Off {
            app.rhythm.set_source(session.rhythm_source.clone());
        }
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

    /// Ouvre l'onglet Forme sur la déformation ou le masque.
    fn show_shape_tab(&mut self, target: EditTarget) {
        self.edit_target = target;
        self.right_tab = RightTab::Shape;
        self.sub_tab[RightTab::Shape as usize] = if target == EditTarget::Mask { 2 } else { 1 };
    }

    fn section(&self) -> Section {
        Section(self.sub_tab[self.right_tab as usize])
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
        self.fx_hidden_alert = None;
        self.chain_col = None;
        self.chain_timer = 0.0;
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
        self.store_active_cells();
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
            if (is_video_file(&p) || video::is_still_image(&p)) && !self.project.media.contains(&p) {
                self.project.media.push(p);
            }
        }
    }

    fn launch_on(&mut self, ctx: &egui::Context, layer: usize, path: &Path) {
        let opts = video::PlayOptions::new(self.project.looping, video::OUTPUT_MAX);
        self.launch_with(ctx, layer, path, opts, None, true);
    }

    /// Lance une vidéo sur un calque, en fondu depuis la vidéo actuelle.
    /// `fade` remplace la durée de fondu générale ; `remember` l'enregistre comme vidéo du calque
    /// dans le projet (pas pour le mix auto, qui change de vidéo en permanence).
    fn launch_with(
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
    fn mix_next(&mut self, ctx: &egui::Context, i: usize) {
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

    fn toggle_reverse(&mut self) {
        let rt = &self.layers[self.sel()];
        if let Some(active) = rt.active() {
            let reverse = !active.is_reverse();
            for p in rt.players() {
                p.set_reverse(reverse);
            }
        }
    }

    fn launch_media(&mut self, ctx: &egui::Context, index: usize) {
        if let Some(path) = self.project.media.get(index).cloned() {
            // Une vidéo envoyée remplace le visuel généré du calque.
            self.launch_any(ctx, self.sel(), &MediaDrag::File(path));
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
        match VideoPlayer::open(path, video::PlayOptions::new(true, video::PREVIEW_MAX), move || {
            repaint_ctx.request_repaint()
        }) {
            Ok(p) => self.cue = Some(p),
            Err(e) => self.status = Some(format!("{} : {e:#}", file_name(path))),
        }
    }

    // ---------------------------------------------------------------- tableau

    /// Enregistre l'état du calque `i` (média, effets, réglages) dans la case `k`.
    fn record_cell(&mut self, i: usize, k: usize) {
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
    fn snapshot_cell(&self, i: usize) -> Cell {
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
            speed: player.map(VideoPlayer::speed),
            reverse: player.map(VideoPlayer::is_reverse),
            paused: player.map(VideoPlayer::is_paused),
        }
    }

    /// Garde dans la case jouée sur le calque `i` tout ce qui a été réglé depuis qu'elle est lancée.
    fn store_active_cell(&mut self, i: usize) {
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
    fn store_active_cells(&mut self) {
        for i in 0..self.layers.len() {
            self.store_active_cell(i);
        }
    }

    /// Joue la case `k` sur le calque `i` : son média (en fondu), ses effets (en transition), ses réglages.
    fn play_cell(&mut self, ctx: &egui::Context, i: usize, k: usize) {
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
    fn media_drag_preview(&mut self, ctx: &egui::Context) {
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
    fn drop_media_in_cell(&mut self, ctx: &egui::Context, i: usize, k: usize, media: MediaDrag) {
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
    fn launch_any(&mut self, ctx: &egui::Context, i: usize, media: &MediaDrag) {
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

    fn column_filled(&self, k: usize) -> bool {
        self.project.layers.iter().any(|l| l.cells.get(k).is_some_and(Option::is_some))
    }

    /// Joue toute la colonne `k` : chaque calque qui a une case dans cette colonne la lance.
    fn play_column(&mut self, ctx: &egui::Context, k: usize) {
        for i in 0..self.project.layers.len() {
            self.play_cell(ctx, i, k);
        }
        self.chain_col = Some(k);
        self.chain_timer = 0.0;
    }

    /// Enchaînement : colonne remplie suivante (en boucle).
    fn chain_next(&mut self, ctx: &egui::Context) {
        let n = self.project.columns;
        let start = self.chain_col.map_or(0, |c| c + 1);
        if let Some(k) = (0..n).map(|d| (start + d) % n).find(|&k| self.column_filled(k)) {
            self.play_column(ctx, k);
        } else {
            self.chain_timer = 0.0;
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
        self.fx_hidden_alert = None;
    }

    fn move_layer(&mut self, i: usize, up: bool) {
        let j = if up { i + 1 } else { i.wrapping_sub(1) };
        if j >= self.project.layers.len() {
            return;
        }
        self.project.layers.swap(i, j);
        self.layers.swap(i, j);
        self.fx_hidden_alert = None;
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

    /// Le calque `i` dessine-t-il quelque chose (vidéo, photo ou visuel généré) ?
    fn has_content(&self, i: usize) -> bool {
        let rt = &self.layers[i];
        rt.active().is_some() || rt.gen_shown.0 != Pattern::Off || rt.gen_from.0 != Pattern::Off
    }

    /// Pourquoi le calque `i` ne se voit pas à l'écran (None s'il est visible).
    fn hidden_reason(&self, i: usize) -> Option<String> {
        let cfg = &self.project.layers[i];
        if !cfg.visible {
            return Some("il est masqué".into());
        }
        if cfg.opacity <= 0.0 {
            return Some("son opacité est à 0".into());
        }
        if !self.has_content(i) {
            return Some("il n'a ni vidéo ni visuel".into());
        }
        self.covered_by(i).map(|j| format!("il est recouvert par le calque « {} », placé devant", self.project.layers[j].name))
    }

    /// Calque opaque placé devant qui recouvre entièrement le calque `i` (celui qui en cache le plus).
    /// Test approché sur une grille de points de la sortie : forme déformée et masque de chaque calque.
    fn covered_by(&self, i: usize) -> Option<usize> {
        const NX: usize = 32;
        const NY: usize = 18;
        // Calques devant qui cachent complètement ce qu'il y a derrière eux.
        let front: Vec<(usize, Vec<surface::Point>)> = (i + 1..self.project.layers.len())
            .filter(|&j| {
                let c = &self.project.layers[j];
                c.visible
                    && c.opacity >= 0.99
                    && c.blend == BlendMode::Normal
                    && c.fit != Fit::Contain
                    && c.effects.shatter == Shatter::Off
                    && c.effects.obj3d == Object3d::Off
                    && c.effects.fold <= 0.0
                    && self.has_content(j)
            })
            .map(|j| (j, self.project.layers[j].warp.outline()))
            .collect();
        if front.is_empty() {
            return None;
        }
        let cfg = &self.project.layers[i];
        let outline = cfg.warp.outline();
        let mut hits = vec![0usize; front.len()];
        let mut drawn = 0;
        for y in 0..NY {
            for x in 0..NX {
                let p = [(x as f32 + 0.5) / NX as f32, (y as f32 + 0.5) / NY as f32];
                if surface::winding(&outline, p) == 0 || !cfg.mask.contains(p) {
                    continue;
                }
                drawn += 1;
                let cover = front
                    .iter()
                    .rposition(|(j, o)| surface::winding(o, p) != 0 && self.project.layers[*j].mask.contains(p));
                match cover {
                    Some(k) => hits[k] += 1,
                    None => return None, // ce point du calque reste visible
                }
            }
        }
        if drawn == 0 {
            return None;
        }
        hits.iter().enumerate().max_by_key(|(_, h)| **h).map(|(k, _)| front[k].0)
    }

    /// Effets visés par le calque `i` (la cible d'une transition en cours, sinon les effets actuels).
    fn fx_target(&self, i: usize) -> (Effects, Option<String>) {
        let rt = &self.layers[i];
        let fx = rt.fx_anim.as_ref().map_or(self.project.layers[i].effects, |a| a.to);
        (fx, rt.preset.clone())
    }

    /// Bandeau : l'effet choisi ne se voit pas, proposer de l'appliquer sur un calque visible.
    fn hidden_fx_banner(&mut self, ui: &mut egui::Ui) {
        let sel = self.sel();
        if self.fx_hidden_alert != Some(sel) {
            self.fx_hidden_alert = None;
            return;
        }
        let Some(reason) = self.hidden_reason(sel) else {
            // Le calque est redevenu visible : plus rien à signaler.
            self.fx_hidden_alert = None;
            return;
        };
        // Calques visibles, du premier plan vers le fond (comme la liste).
        let visible: Vec<usize> = (0..self.project.layers.len())
            .rev()
            .filter(|&j| j != sel && self.hidden_reason(j).is_none())
            .collect();
        let can_show = !self.project.layers[sel].visible || self.project.layers[sel].opacity <= 0.0;
        let can_raise = !can_show && self.has_content(sel) && sel + 1 < self.project.layers.len();
        let mut raise = false;
        let mut target = None;
        let mut show = false;
        let mut dismiss = false;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 6))
            .corner_radius(6.0)
            .fill(WARN.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0, WARN.gamma_multiply(0.6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(ic::WARNING).size(16.0).color(WARN));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "L'effet ne se voit pas : le calque « {} » n'est pas visible à l'écran ({reason}).",
                                self.project.layers[sel].name
                            ))
                            .color(TEXT),
                        )
                        .wrap(),
                    );
                });
                if visible.is_empty() {
                    ui.label(
                        egui::RichText::new("Aucun autre calque n'est visible pour l'instant.").small().color(TEXT_WEAK),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("Sélectionner un calque visible et y appliquer l'effet :")
                            .small()
                            .color(TEXT_WEAK),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for &j in &visible {
                            if ui
                                .button(with_icon(ic::ARROW_RIGHT, &self.project.layers[j].name))
                                .on_hover_text("Sélectionner ce calque et lui appliquer les mêmes effets")
                                .clicked()
                            {
                                target = Some(j);
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if can_show
                        && ui
                            .button(with_icon(ic::EYE, "Afficher ce calque"))
                            .on_hover_text("Rendre visible le calque sélectionné")
                            .clicked()
                    {
                        show = true;
                    }
                    if can_raise
                        && ui
                            .button(with_icon(ic::CARET_UP, "Passer au premier plan"))
                            .on_hover_text("Placer le calque sélectionné devant tous les autres")
                            .clicked()
                    {
                        raise = true;
                    }
                    if ui.button(with_icon(ic::X, "Ignorer")).clicked() {
                        dismiss = true;
                    }
                });
            });
        ui.add_space(4.0);
        if let Some(j) = target {
            let (fx, name) = self.fx_target(sel);
            self.apply_preset_to(j, name, fx, self.project.preset_secs);
            self.layers[j].auto_timer = 0.0;
            self.selected = j;
            self.fx_hidden_alert = None;
        } else if show {
            let cfg = &mut self.project.layers[sel];
            cfg.visible = true;
            if cfg.opacity <= 0.0 {
                cfg.opacity = 1.0;
            }
            self.fx_hidden_alert = None;
        } else if raise {
            for k in sel..self.project.layers.len() - 1 {
                self.move_layer(k, true);
            }
            self.fx_hidden_alert = None;
        } else if dismiss {
            self.fx_hidden_alert = None;
        }
    }

    /// Retour à l'image brute sur le calque sélectionné : plus d'effets et mode AUTO coupé.
    fn clear_effects(&mut self) {
        let i = self.sel();
        self.project.layers[i].auto.enabled = false;
        // Immédiat et complet : effets neutres et animations (rotation, teinte…) remises à zéro,
        // pour retrouver tout de suite l'image d'origine, dans le bon sens.
        self.apply_preset_to(i, None, Effects::default(), 0.0);
        let shape = self.layers[i].phases;
        self.layers[i].phases = Phases {
            shape_ripple: shape.shape_ripple,
            shape_trace: shape.shape_trace,
            shape_breathe: shape.shape_breathe,
            shatter: shape.shatter,
            ..Phases::default()
        };
        self.layers[i].auto_timer = 0.0;
    }

    /// Applique des effets au calque `i`, sans toucher à ses animations de forme du masque.
    fn apply_preset_to(&mut self, i: usize, name: Option<String>, fx: Effects, secs: f32) {
        let fx = fx.with_shape_of(&self.project.layers[i].effects);
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

    /// Générateur : passe le calque `i` sur un autre motif (et souvent une autre palette) au hasard.
    fn generator_next(&mut self, i: usize) {
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
        let choice = if auto.theme != AutoTheme::All || use_random || candidates.is_empty() {
            (None, Effects::themed(auto.theme, roll, auto.intensity, auto.strobe))
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
    fn on_beats(&self, i: usize, n: impl Fn(&rhythm::React) -> u32) -> bool {
        let react = &self.project.layers[i].react;
        react.enabled && n(react) > 0 && self.rhythm.active() && self.rhythm.source != rhythm::Source::Manual
    }

    fn draw_params(&mut self) -> DrawParams {
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
        if pressed(Key::W) {
            self.white_screen = !self.white_screen;
        }
        if pressed(Key::G) {
            self.grid = !self.grid;
        }
        if pressed(Key::R) {
            self.toggle_reverse();
        }
        // Sans la répétition automatique : garder T enfoncé ne compte qu'un tap.
        let tapped = ctx.input(|i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::T, pressed: true, repeat: false, .. }))
        });
        if tapped {
            self.tap();
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

    /// Horloge du rythme (secondes depuis le lancement).
    fn clock(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Tap tempo : passe en mode tempo si aucun rythme n'est actif.
    fn tap(&mut self) {
        let now = self.clock();
        self.rhythm.tap(now);
    }

    /// Voyant de temps + bouton TAP (barre du haut).
    fn tap_widget(&mut self, ui: &mut egui::Ui) {
        let pulse = self.rhythm.pulse;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        let on = self.rhythm.active();
        let color = if on { ACCENT.gamma_multiply(0.25 + 0.75 * pulse.env) } else { theme::BORDER };
        ui.painter().circle_filled(rect.center(), 5.0, color);
        let label = match self.rhythm.source {
            rhythm::Source::Audio(_) if on => "SON".to_owned(),
            rhythm::Source::Manual => "MANUEL".to_owned(),
            _ => format!("{:.0} BPM", self.rhythm.tempo.bpm),
        };
        if ui
            .button(with_icon(ic::WAVES, &format!("TAP (T) · {label}")))
            .on_hover_text("Taper en rythme pour donner le tempo (T). Réglages dans l'onglet Rythme.")
            .clicked()
        {
            self.tap();
        }
    }

    /// Onglet Rythme : source (son ou tempo) et réaction du calque sélectionné.
    fn generator_ui(&mut self, ui: &mut egui::Ui) {
        let sec = self.section();
        let sel = self.sel();
        let has_video = self.layers[sel].active().is_some();
        let timer = self.layers[sel].gen_timer;
        let mut next = false;
        let g = &mut self.project.layers[sel].generator;
        let d = crate::generator::Generator::default();
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
            ui.horizontal_wrapped(|ui| {
                for p in Pattern::ALL {
                    let resp = ui.selectable_label(g.pattern == p, p.label()).on_hover_text(p.description());
                    if resp.clicked() {
                        g.pattern = p;
                    }
                }
            });
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
            ui.add(egui::Slider::new(&mut g.interval, 2.0..=120.0).text("Durée d'un motif").suffix(" s"));
            ui.add(egui::Slider::new(&mut g.transition, 0.0..=10.0).text("Fondu").suffix(" s"));
            if g.auto && g.is_active() {
                ui.add(egui::ProgressBar::new((timer / g.interval.max(1.0)).clamp(0.0, 1.0)).desired_height(4.0));
            }
        });
        if next {
            self.generator_next(sel);
        }
        self.shape_fx_ui(ui);
    }

    /// Contours animés de l'image du calque (idéal pour une photo).
    fn lines_fx_ui(&mut self, ui: &mut egui::Ui) {
        let i = self.sel();
        let fx = &mut self.project.layers[i].effects;
        let d = Effects::default();
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
                for (k, (name, desc)) in Effects::LINE_STYLES.iter().enumerate() {
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
    fn shape_fx_ui(&mut self, ui: &mut egui::Ui) {
        let sec = self.section();
        let i = self.sel();
        let has_mask = self.project.layers[i].mask.is_active();
        let fx = &mut self.project.layers[i].effects;
        let d = Effects::default();
        let builtin = effects::mask_anim_presets();
        let user = &mut self.project.mask_presets;
        let preset_name = &mut self.mask_preset_name;
        sec.show(ui, 4, |ui| {
            ui.label(egui::RichText::new(with_icon(ic::STAR, "Presets")).strong().color(TEXT));
            ui.horizontal_wrapped(|ui| {
                for p in &builtin {
                    let on = fx.uses_shape() && fx.same_mask_anim(&p.effects);
                    if ui.selectable_label(on, &p.name).clicked() {
                        *fx = fx.with_mask_anim_of(&p.effects);
                    }
                }
                if ui
                    .button(with_icon(ic::PROHIBIT, "Aucune"))
                    .on_hover_text("Retirer les animations sur la forme du masque")
                    .clicked()
                {
                    *fx = fx.with_mask_anim_of(&d);
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
            fx_slider(ui, "Respiration", &mut fx.shape_breathe, -0.15..=0.15, d.shape_breathe);
            fx_slider(ui, "Fréquence respiration Hz", &mut fx.shape_breathe_rate, 0.05..=4.0, d.shape_breathe_rate);
        });
        let mut shot = false;
        sec.show(ui, 5, |ui| {
            ui.label(
                egui::RichText::new(
                    "La forme se casse depuis le centre du masque (ou du calque, sans masque) et laisse voir \
                     les calques du dessous, puis se reconstruit. Les fissures prennent la couleur des lumières.",
                )
                .small()
                .color(TEXT_WEAK),
            );
            ui.horizontal_wrapped(|ui| {
                for st in Shatter::ALL {
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
    }

    fn rhythm_ui(&mut self, ui: &mut egui::Ui) {
        if self.section().0 == 0 {
            self.rhythm_source_ui(ui);
        } else {
            self.rhythm_react_ui(ui);
        }
    }

    fn rhythm_source_ui(&mut self, ui: &mut egui::Ui) {
        let current = match &self.rhythm.source {
            rhythm::Source::Off => "Aucune".to_owned(),
            rhythm::Source::Audio(None) => match &self.rhythm.input {
                Some(input) => format!("Par défaut ({})", input.device),
                None => "Entrée audio par défaut".to_owned(),
            },
            rhythm::Source::Audio(Some(n)) => n.clone(),
            rhythm::Source::Tempo => "Tempo tapé (BPM)".to_owned(),
            rhythm::Source::Manual => "Manuel (touche T)".to_owned(),
        };
        let mut chosen = None;
        let combo = egui::ComboBox::from_id_salt("rhythm_source").selected_text(current).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(self.rhythm.source == rhythm::Source::Off, "Aucune").clicked() {
                chosen = Some(rhythm::Source::Off);
            }
            if ui.selectable_label(self.rhythm.source == rhythm::Source::Tempo, "Tempo tapé (BPM)").clicked() {
                chosen = Some(rhythm::Source::Tempo);
            }
            if ui
                .selectable_label(self.rhythm.source == rhythm::Source::Manual, "Manuel (touche T)")
                .on_hover_text("Chaque appui sur T donne une pulsation ; sans appui, plus rien ne bouge")
                .clicked()
            {
                chosen = Some(rhythm::Source::Manual);
            }
            ui.separator();
            if ui.selectable_label(self.rhythm.source == rhythm::Source::Audio(None), "Entrée audio par défaut").clicked() {
                chosen = Some(rhythm::Source::Audio(None));
            }
            for name in rhythm::input_devices() {
                let source = rhythm::Source::Audio(Some(name.clone()));
                if ui.selectable_label(self.rhythm.source == source, &name).clicked() {
                    chosen = Some(source);
                }
            }
        });
        combo.response.on_hover_text(
            "Son : micro ou entrée de la carte son (pour la sortie de l'ordinateur, utiliser une entrée virtuelle \
             comme BlackHole sur Mac ou « Mixage stéréo » sous Windows).\nTempo : taper le rythme avec le bouton TAP ou la touche T.\nManuel : une pulsation à chaque appui sur T, rien sans appui.",
        );
        if let Some(source) = chosen {
            self.rhythm.set_source(source);
        }
        if let Some(e) = &self.rhythm.error {
            ui.colored_label(WARN, e);
        }
        if matches!(self.rhythm.source, rhythm::Source::Audio(_)) {
            ui.horizontal(|ui| {
                ui.label("Réagir aux");
                for band in rhythm::Band::ALL {
                    if ui
                        .selectable_label(self.rhythm.band == band, band.label())
                        .on_hover_text(band.description())
                        .clicked()
                    {
                        self.rhythm.set_band(band);
                    }
                }
            });
        }

        let pulse = self.rhythm.pulse;
        match self.rhythm.source {
            rhythm::Source::Audio(_) if self.rhythm.input.is_some() => {
                for (label, v) in [("Intensité", pulse.level), ("Graves", pulse.bass), ("Aigus", pulse.high)] {
                    ui.horizontal(|ui| {
                        ui.add_sized([56.0, 14.0], egui::Label::new(egui::RichText::new(label).small().color(TEXT_WEAK)));
                        ui.add(egui::ProgressBar::new(v).desired_height(8.0).fill(ACCENT.gamma_multiply(0.8)));
                    });
                }
            }
            rhythm::Source::Tempo => {
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button(with_icon(ic::WAVES, "TAP"))).on_hover_text("Touche T").clicked() {
                        self.tap();
                    }
                    let mut bpm = self.rhythm.tempo.bpm;
                    if ui.add(egui::DragValue::new(&mut bpm).range(30.0..=300.0).speed(0.2).max_decimals(1).suffix(" BPM")).changed() {
                        self.rhythm.tempo.set_bpm(bpm, self.clock());
                    }
                    if ui.small_button("÷2").clicked() {
                        let b = self.rhythm.tempo.bpm / 2.0;
                        self.rhythm.tempo.set_bpm(b, self.clock());
                    }
                    if ui.small_button("×2").clicked() {
                        let b = self.rhythm.tempo.bpm * 2.0;
                        self.rhythm.tempo.set_bpm(b, self.clock());
                    }
                });
            }
            rhythm::Source::Manual => {
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button(with_icon(ic::LIGHTNING, "PULSE"))).on_hover_text("Touche T").clicked() {
                        self.tap();
                    }
                    ui.label(
                        egui::RichText::new("Une pulsation par appui sur T ; sans appui, plus rien ne bouge.")
                            .small()
                            .color(TEXT_WEAK),
                    );
                });
            }
            _ => {}
        }
        if self.rhythm.active() {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), Sense::hover());
            ui.painter().rect_filled(rect, 3.0, ACCENT.gamma_multiply(0.1 + 0.9 * pulse.env));
        }

    }

    fn rhythm_react_ui(&mut self, ui: &mut egui::Ui) {
        let sel = self.sel();
        let react = &mut self.project.layers[sel].react;
        ui.toggle_value(&mut react.enabled, with_icon(ic::LIGHTNING, "Réagir au rythme"))
            .on_hover_text("Les effets de ce calque bougent avec le son ou le tempo");
        ui.label(egui::RichText::new("Réactions toutes prêtes").small().color(TEXT_WEAK));
        ui.horizontal_wrapped(|ui| {
            for (name, desc, preset) in rhythm::React::presets() {
                let on = react.enabled && react.same_feel(&preset);
                if ui.selectable_label(on, name).on_hover_text(desc).clicked() {
                    *react = rhythm::React { auto_beats: react.auto_beats, mix_beats: react.mix_beats, ..preset };
                }
            }
        });
        ui.add_space(4.0);
        ui.add_enabled_ui(react.enabled, |ui| {
            percent_slider(ui, &mut react.zoom, "Coup de zoom").on_hover_text("Zoom bref sur chaque temps");
            percent_slider(ui, &mut react.flash, "Flash").on_hover_text("Éclair de lumière sur chaque temps");
            percent_slider(ui, &mut react.shake, "Secousse RVB").on_hover_text("Décalage des couleurs sur les temps et les aigus");
            percent_slider(ui, &mut react.drive, "Suit le volume")
                .on_hover_text("Ondes, liquide, tourbillon et saturation s'amplifient avec le volume (avec le tempo : sur les temps)");
            ui.add(
                egui::Slider::new(&mut react.hue, 0.0..=0.5)
                    .text("Saut de teinte")
                    .custom_formatter(|v, _| format!("{:.0}°", v * 360.0)),
            )
            .on_hover_text("La teinte tourne d'autant à chaque temps");
            percent_slider(ui, &mut react.invert, "Négatif").on_hover_text("Flash négatif sur chaque temps");
            percent_slider(ui, &mut react.gate, "Stroboscope")
                .on_hover_text("Noir entre les temps : l'image ne s'allume que sur le rythme");
            percent_slider(ui, &mut react.glitch, "Glitch").on_hover_text("L'image se casse en pixels sur chaque temps");
            percent_slider(ui, &mut react.shock, "Onde de choc").on_hover_text("Ondulation qui part sur chaque temps");
            percent_slider(ui, &mut react.kaleido, "Kaléido").on_hover_text("Le kaléidoscope s'ouvre sur chaque temps");
            ui.add(
                egui::Slider::new(&mut react.spin, 0.0..=1.0)
                    .text("Rotation par temps")
                    .custom_formatter(|v, _| format!("{:.0}°", v * 90.0)),
            )
            .on_hover_text("L'image tourne d'autant à chaque temps, en glissant");
            ui.checkbox(&mut react.flip, "Miroir un temps sur deux");
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Enchaînements sur les temps (0 = désactivé)").small().color(TEXT_WEAK));
            ui.add(egui::Slider::new(&mut react.auto_beats, 0..=32).text("AUTO : effet suivant tous les").suffix(" temps"))
                .on_hover_text("Remplace l'intervalle en secondes du mode AUTO (onglet Effets)");
            ui.add(egui::Slider::new(&mut react.mix_beats, 0..=32).text("Mix auto : coupe tous les").suffix(" temps"))
                .on_hover_text("Remplace les durées de séquence du mix auto (onglet Mix)");
        });
        if self.rhythm.source == rhythm::Source::Manual {
            ui.label(
                egui::RichText::new("Mode manuel : AUTO et mix gardent leur durée en secondes ; T ne pilote que les réactions.")
                    .small()
                    .color(TEXT_WEAK),
            );
        }
        if !self.rhythm.active() {
            ui.label(egui::RichText::new("Choisir une source du rythme (onglet Source) pour activer les réactions.").small().color(WARN));
        }
    }

    /// Menu de choix de l'écran de sortie.
    fn screen_menu(&mut self, ui: &mut egui::Ui) {
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
    fn drive_output_move(&mut self, ctx: &egui::Context) {
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

    fn media_panel(&mut self, ui: &mut egui::Ui) {
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
    fn generated_media_ui(&mut self, ui: &mut egui::Ui) {
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

    fn layers_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let total = ui.available_width();
        ui.horizontal_top(|ui| {
            ui.allocate_ui(Vec2::new(total * 0.74, ui.available_height()), |ui| {
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
                egui::RichText::new("Tab : calque suivant · en haut = au premier plan")
                    .small()
                    .color(TEXT_WEAK),
            );
            ui.separator();
            ui.label(egui::RichText::new(with_icon(ic::SQUARES_FOUR, "Tableau")).strong().color(TEXT)).on_hover_text(
                "Chaque case garde un média avec ses effets et réglages.\n\
                 Case vide : enregistrer l'état actuel du calque · Case pleine : la jouer · Clic droit : options.\n\
                 Le bouton d'une colonne joue toutes ses cases d'un coup.",
            );
            let chain = &mut self.project.chain;
            ui.toggle_value(&mut chain.enabled, with_icon(ic::REPEAT, "Enchaîner"))
                .on_hover_text("Joue les colonnes remplies l'une après l'autre, en boucle");
            ui.add(egui::DragValue::new(&mut chain.secs).range(0.5..=600.0).speed(0.1).suffix(" s"))
                .on_hover_text("Durée de chaque colonne");
        });

        let n = self.project.layers.len();
        let sel = self.sel();
        let mut action: Option<(usize, &str)> = None;
        let columns = self.project.columns;
        let cell_size = Vec2::new(78.0, 30.0);
        let ctx = ui.ctx().clone();
        let mut cell_action: Option<CellAction> = None;
        // Les cases défilent horizontalement, toutes lignes ensemble ; les cartes des calques restent fixes.
        let offset = self.grid_offset;
        let mut new_offset = offset;
        let cells_scroll = |row: usize| {
            egui::ScrollArea::horizontal()
                .id_salt(("cells", row))
                .horizontal_scroll_offset(offset)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        };
        egui::ScrollArea::vertical().id_salt("layers").show(ui, |ui| {
            // En-têtes de colonnes, alignés sur les cases (à droite des cartes de calque).
            ui.horizontal(|ui| {
                ui.add_space(self.layer_card_w);
                let out = cells_scroll(usize::MAX).show(ui, |ui| {
                ui.horizontal(|ui| {
                for k in 0..columns {
                    let filled = self.project.layers.iter().any(|l| l.cells.get(k).is_some_and(Option::is_some));
                    let current = self.chain_col == Some(k);
                    let text = egui::RichText::new(format!("{} {}", ic::PLAY, k + 1)).small();
                    let text = if current { text.color(ACCENT) } else if filled { text.color(TEXT) } else { text.color(TEXT_WEAK) };
                    let resp = ui
                        .add_enabled(filled, egui::Button::new(text).min_size(Vec2::new(cell_size.x, 18.0)))
                        .on_hover_text("Jouer toute la colonne (une case par calque)");
                    if resp.clicked() {
                        cell_action = Some(CellAction::Column(k));
                    }
                }
                if columns < project::MAX_COLUMNS
                    && ui.add(theme::icon_button(ic::PLUS).small()).on_hover_text("Ajouter une colonne").clicked()
                {
                    cell_action = Some(CellAction::AddColumn);
                }
                });
                });
                if (out.state.clamped_offset().x - offset).abs() > 0.5 {
                    new_offset = out.state.clamped_offset().x;
                }
            });
            for i in (0..n).rev() {
                let selected = i == sel;
                let clip = self.layers[i].active().map(|p| file_name(&p.path));
                let cfg = &mut self.project.layers[i];
                let frame = egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .corner_radius(6.0)
                    .fill(if selected { theme::ACCENT_DIM.gamma_multiply(0.55) } else { CARD })
                    .stroke(Stroke::new(1.0, if selected { ACCENT.gamma_multiply(0.7) } else { theme::BORDER }));
                let row = ui.horizontal(|ui| {
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
                                [110.0, 20.0],
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
                self.layer_card_w = row.rect.width();
                // Cases du tableau pour ce calque.
                let out = cells_scroll(i).show(ui, |ui| {
                ui.horizontal(|ui| {
                for k in 0..columns {
                    let cell = cfg.cells.get(k).and_then(Option::as_ref);
                    let thumb = cell.and_then(|c| c.clip.as_ref()).and_then(|p| self.thumbs.get(&ctx, p));
                    let hover = cell.map(|c| {
                        let mut lines = vec![c.name.clone()];
                        lines.push(match &c.clip {
                            Some(p) => with_icon(ic::FILM_SLATE, &file_name(p)),
                            None if c.generator.pattern != Pattern::Off => with_icon(ic::SUN, c.generator.pattern.label()),
                            None => "Sans média".into(),
                        });
                        if let Some(p) = &c.preset {
                            lines.push(with_icon(ic::SPARKLE, p));
                        }
                        lines.push("Clic : jouer · Clic droit : options".into());
                        lines.join("\n")
                    });
                    let name = cell.map(|c| c.name.clone());
                    let active = self.layers[i].cell == Some(k) && name.is_some();
                    let swatch = cell
                        .filter(|c| c.clip.is_none() && c.generator.pattern != Pattern::Off)
                        .map(|c| (c.generator.pattern, c.generator.palette));
                    let resp = cell_widget(ui, cell_size, thumb.as_ref(), swatch, name.as_deref(), active);
                    if resp.dnd_hover_payload::<MediaDrag>().is_some() {
                        ui.painter().rect_stroke(resp.rect, 4.0, Stroke::new(2.0, WARN), egui::StrokeKind::Inside);
                    }
                    if let Some(m) = resp.dnd_release_payload::<MediaDrag>() {
                        cell_action = Some(CellAction::Drop(i, k, (*m).clone()));
                    }
                    let resp = resp.on_hover_text(
                        hover.unwrap_or_else(|| "Enregistrer ici le média, les effets et les réglages actuels du calque".into()),
                    );
                    if resp.clicked() {
                        cell_action = Some(if name.is_some() { CellAction::Play(i, k) } else { CellAction::Record(i, k) });
                    }
                    if name.is_some() {
                        resp.context_menu(|ui| {
                            if let Some(Some(c)) = cfg.cells.get_mut(k) {
                                ui.horizontal(|ui| {
                                    ui.label(with_icon(ic::PENCIL_SIMPLE, ""));
                                    ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(140.0));
                                });
                            }
                            if ui.button(with_icon(ic::FLOPPY_DISK, "Remplacer par l'état actuel")).clicked() {
                                cell_action = Some(CellAction::Record(i, k));
                                ui.close();
                            }
                            if ui.button(with_icon(ic::TRASH, "Vider la case")).clicked() {
                                cell_action = Some(CellAction::Clear(i, k));
                                ui.close();
                            }
                        });
                    }
                }
                });
                });
                if (out.state.clamped_offset().x - offset).abs() > 0.5 {
                    new_offset = out.state.clamped_offset().x;
                }
                row
                }).inner;
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
        self.grid_offset = new_offset;
        match cell_action {
            Some(CellAction::Play(i, k)) => {
                self.play_cell(&ctx, i, k);
                self.selected = i;
            }
            Some(CellAction::Record(i, k)) => {
                self.record_cell(i, k);
                self.selected = i;
            }
            Some(CellAction::Clear(i, k)) => {
                if let Some(c) = self.project.layers[i].cells.get_mut(k) {
                    *c = None;
                }
                if self.layers[i].cell == Some(k) {
                    self.layers[i].cell = None;
                }
                self.layers[i].cell_pos.remove(&k);
            }
            Some(CellAction::Column(k)) => self.play_column(&ctx, k),
            Some(CellAction::AddColumn) => self.project.columns += 1,
            Some(CellAction::Drop(i, k, media)) => self.drop_media_in_cell(&ctx, i, k, media),
            None => {}
        }
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
            ui.label(egui::RichText::new("Aucune vidéo ni photo sur ce calque.").color(TEXT_WEAK));
            return;
        };
        let info = player.info.clone();
        if info.still {
            // Photo : rien à lire, mais on peut l'animer.
            ui.label(egui::RichText::new(with_icon(ic::IMAGE, &file_name(&player.path))).strong().color(TEXT));
            ui.label(
                egui::RichText::new(format!("Photo  ·  {}×{}", info.width, info.height)).small().color(TEXT_WEAK),
            );
            if ui
                .button(with_icon(ic::SPARKLE, "Animer la photo…"))
                .on_hover_text("Contours qui s'illuminent et bougent (onglet Photo)")
                .clicked()
            {
                self.right_tab = RightTab::Photo;
                if self.project.layers[i].effects.lines_glow <= 0.0 {
                    self.project.layers[i].effects.set_lines_style(0);
                }
            }
            return;
        }
        ui.label(egui::RichText::new(with_icon(ic::FILM_SLATE, &file_name(&player.path))).strong().color(TEXT));

        let paused = player.is_paused();
        let mut speed = player.speed();
        let mut pos = player.position() as f32;
        let mut seek_to = None;
        let mut toggle = false;
        let mut flip = false;
        ui.horizontal(|ui| {
            toggle = ui
                .add(theme::icon_button(if paused { ic::PLAY } else { ic::PAUSE }))
                .on_hover_text(if paused { "Lecture (Espace)" } else { "Pause (Espace)" })
                .clicked();
            let reverse = player.is_reverse();
            if ui
                .add(theme::icon_button(ic::REWIND).selected(reverse))
                .on_hover_text("Lecture à l'envers (R)")
                .clicked()
            {
                flip = true;
            }
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
            ui.add(
                egui::Slider::new(&mut speed, 0.1..=4.0)
                    .logarithmic(true)
                    .max_decimals(2)
                    .suffix("×")
                    .text("Vitesse"),
            );
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
        if flip {
            self.toggle_reverse();
        }
        if self.project.layers[i].mix.enabled {
            ui.label(egui::RichText::new(with_icon(ic::FILM_REEL, "Mix auto actif sur ce calque")).small().color(ACCENT));
        }
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
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
    fn settings_pane(&mut self, ui: &mut egui::Ui) {
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

    fn effects_ui(&mut self, ui: &mut egui::Ui) {
        let sec = self.section();
        self.presets_ui(ui);

        let i = self.sel();
        let fx = &mut self.project.layers[i].effects;
        // Vrai seulement si l'utilisateur a touché un réglage (pas sur une simple mise à jour).
        let mut touched = false;
        let d = Effects::default();
        sec.show(ui, 1, |ui| {
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
            touched |= fx_slider(ui, "Caméra thermique", &mut fx.thermal, 0.0..=1.0, d.thermal);
            touched |= fx_slider(ui, "Flou", &mut fx.blur, 0.0..=1.0, d.blur);
            touched |= fx_slider(ui, "Halo lumineux", &mut fx.bloom, 0.0..=1.5, d.bloom);
        });
        sec.show(ui, 2, |ui| {
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
        sec.show(ui, 3, |ui| {
            touched |= fx_slider(ui, "Pixelisation", &mut fx.pixelate, 0.0..=1.0, d.pixelate);
            touched |= fx_slider(ui, "Décalage RVB", &mut fx.rgb_shift, 0.0..=0.05, d.rgb_shift);
            touched |= fx_slider(ui, "Ondulation", &mut fx.wave_amp, 0.0..=0.1, d.wave_amp);
            touched |= fx_slider(ui, "Fréquence onde", &mut fx.wave_freq, 0.5..=30.0, d.wave_freq);
            touched |= fx_slider(ui, "Vitesse onde", &mut fx.wave_speed, 0.0..=20.0, d.wave_speed);
            touched |= fx_slider(ui, "Fish-eye", &mut fx.fisheye, -1.5..=1.5, d.fisheye);
            touched |= fx_slider(ui, "Ondes de choc", &mut fx.shock, 0.0..=1.0, d.shock);
            touched |= fx_slider(ui, "Ondes de choc /s", &mut fx.shock_speed, 0.05..=3.0, d.shock_speed);
            touched |= fx_slider(ui, "Mosaïque hexagonale", &mut fx.hexagon, 0.0..=1.0, d.hexagon);
            touched |= fx_slider(ui, "Droste (spirale infinie)", &mut fx.droste, 0.0..=1.0, d.droste);
            touched |= fx_slider(ui, "Zoom Droste /s", &mut fx.droste_speed, -1.5..=1.5, d.droste_speed);
            touched |= fx_slider(ui, "Flou de zoom", &mut fx.zoom_blur, 0.0..=1.0, d.zoom_blur);
        });
        sec.show(ui, 4, |ui| {
            touched |= fx_slider(ui, "Tourbillon", &mut fx.swirl, -4.0..=4.0, d.swirl);
            touched |= fx_slider(ui, "Oscillation tourbillon /s", &mut fx.swirl_speed, 0.0..=1.0, d.swirl_speed);
            touched |= fx_slider(ui, "Tunnel", &mut fx.polar, 0.0..=1.0, d.polar);
            touched |= fx_slider(ui, "Vitesse tunnel", &mut fx.tunnel_speed, -2.0..=2.0, d.tunnel_speed);
            touched |= fx_slider(ui, "Mosaïque miroir", &mut fx.tiles, 1.0..=8.0, d.tiles);
            touched |= fx_slider(ui, "Liquide", &mut fx.liquid, 0.0..=0.08, d.liquid);
            touched |= fx_slider(ui, "Échelle liquide", &mut fx.liquid_scale, 1.0..=20.0, d.liquid_scale);
            touched |= fx_slider(ui, "Vitesse liquide", &mut fx.liquid_speed, 0.0..=8.0, d.liquid_speed);
            touched |= fx_slider(ui, "Palette acide", &mut fx.acid, 0.0..=1.0, d.acid);
            touched |= fx_slider(ui, "Rotation acide /s", &mut fx.acid_speed, -1.0..=1.0, d.acid_speed);
            touched |= fx_slider(ui, "Arc-en-ciel spatial", &mut fx.rainbow, 0.0..=3.0, d.rainbow);
            touched |= fx_slider(ui, "Solarisation", &mut fx.solarize, 0.0..=1.0, d.solarize);
            touched |= fx_slider(ui, "Pulsation", &mut fx.pulse, 0.0..=0.3, d.pulse);
            touched |= fx_slider(ui, "Fréquence pulsation Hz", &mut fx.pulse_rate, 0.05..=5.0, d.pulse_rate);
            touched |= fx_slider(ui, "Écho", &mut fx.echo, 0.0..=1.0, d.echo);
        });
        sec.show(ui, 5, |ui| {
            touched |= fx_slider(ui, "Télé cathodique", &mut fx.crt, 0.0..=1.0, d.crt);
            touched |= fx_slider(ui, "Glitch numérique", &mut fx.glitch, 0.0..=1.0, d.glitch);
            touched |= fx_slider(ui, "Grain de pellicule", &mut fx.grain, 0.0..=1.0, d.grain);
            touched |= fx_slider(ui, "Trame BD", &mut fx.halftone, 0.0..=1.0, d.halftone);
            touched |= fx_slider(ui, "ASCII (taille des caractères)", &mut fx.ascii, 0.0..=1.0, d.ascii);
        });
        sec.show(ui, 6, |ui| {
            ui.label(egui::RichText::new("Objet").strong().color(TEXT));
            ui.horizontal_wrapped(|ui| {
                for o in Object3d::ALL {
                    touched |= ui.selectable_value(&mut fx.obj3d, o, o.label()).on_hover_text(o.description()).changed();
                }
            });
            ui.add_enabled_ui(fx.obj3d != Object3d::Off, |ui| {
                touched |= fx_slider(ui, "Inclinaison °", &mut fx.rot3d_x, -180.0..=180.0, d.rot3d_x);
                touched |= fx_slider(ui, "Rotation °", &mut fx.rot3d_y, -180.0..=180.0, d.rot3d_y);
                touched |= fx_slider(ui, "Rotation auto °/s", &mut fx.spin3d_y, -180.0..=180.0, d.spin3d_y);
                touched |= fx_slider(ui, "Bascule auto °/s", &mut fx.spin3d_x, -180.0..=180.0, d.spin3d_x);
                touched |= fx_slider(ui, "Distance caméra", &mut fx.dist3d, 1.0..=8.0, d.dist3d);
                if fx.obj3d.uses_height() {
                    touched |= fx_slider(ui, "Hauteur du relief", &mut fx.height3d, 0.0..=1.0, d.height3d);
                }
                if fx.obj3d == Object3d::Voxels {
                    touched |= fx_slider(ui, "Colonnes (par hauteur)", &mut fx.voxels, 8.0..=120.0, d.voxels);
                }
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Volume").strong().color(TEXT));
            touched |= fx_slider(ui, "Relief", &mut fx.relief, 0.0..=1.5, d.relief);
            touched |= fx_slider(ui, "Anaglyphe (lunettes 3D)", &mut fx.anaglyph, 0.0..=0.04, d.anaglyph);
            touched |= fx_slider(ui, "Drapeau", &mut fx.flag, 0.0..=1.5, d.flag);
            touched |= fx_slider(ui, "Vitesse drapeau", &mut fx.flag_speed, 0.0..=8.0, d.flag_speed);
            touched |= fx_slider(ui, "Accordéon", &mut fx.fold, 0.0..=1.0, d.fold);
            touched |= fx_slider(ui, "Nombre de plis", &mut fx.fold_count, 2.0..=16.0, d.fold_count);
        });
        sec.show(ui, 7, |ui| {
            touched |= fx_slider(ui, "Stroboscope Hz", &mut fx.strobe, 0.0..=20.0, d.strobe);
        });
        // Un réglage manuel interrompt une transition de preset en cours.
        if touched {
            self.layers[i].fx_anim = None;
            self.layers[i].preset = None;
        }
    }

    fn presets_ui(&mut self, ui: &mut egui::Ui) {
        let sec = self.section();
        let sel = self.sel();
        let current = self.project.layers[sel].effects;
        let active = self.layers[sel].preset.clone();
        // Surligné : le preset appliqué en dernier sur ce calque, sinon celui qui correspond aux réglages.
        let is_active = |p: &Preset| match &active {
            Some(name) => *name == p.name,
            None => p.effects.with_shape_of(&current) == current,
        };
        let mut apply: Option<(Option<String>, Effects)> = None;
        let mut auto_next = false;
        let mut clear = false;
        sec.show(ui, 0, |ui| {
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
        let sec = self.section();
        let i = self.sel();
        sec.show(ui, 0, |ui| {
            ui.label(egui::RichText::new("Comment l'image se place dans la surface").small().color(TEXT_WEAK));
            for f in Fit::ALL {
                ui.selectable_value(&mut self.project.layers[i].fit, f, f.label()).on_hover_text(f.hint());
            }
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

    fn mix_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let sel = self.sel();
        let (timer, len, seg) = {
            let rt = &self.layers[sel];
            (rt.mix_timer, rt.mix_len, rt.mix_seg.clone())
        };
        let media = self.project.media.clone();
        let mut next = false;
        let mut started = false;
        let mix = &mut self.project.layers[sel].mix;

        ui.horizontal(|ui| {
            let label = egui::RichText::new(with_icon(ic::FILM_REEL, "MIX AUTO")).strong();
            if ui
                .toggle_value(&mut mix.enabled, label)
                .on_hover_text("Enchaîne au hasard des séquences des vidéos cochées sur ce calque")
                .changed()
                && mix.enabled
            {
                started = true;
            }
            if ui
                .add(theme::icon_button(ic::SKIP_FORWARD))
                .on_hover_text("Séquence suivante tout de suite")
                .clicked()
            {
                next = true;
            }
        });
        ui.label(
            egui::RichText::new(
                "Découpe des séquences dans les vidéos cochées : changements de vidéo, sauts dans le temps, \
                 lecture à l'envers, vitesse variable, coupes franches ou fondus.",
            )
            .small()
            .color(TEXT_WEAK),
        );
        if mix.enabled {
            ui.add(
                egui::ProgressBar::new((timer / len.max(0.01)).clamp(0.0, 1.0))
                    .desired_height(6.0)
                    .fill(ACCENT.gamma_multiply(0.7)),
            );
            if let Some(seg) = &seg {
                let dir = if seg.reverse { with_icon(ic::REWIND, "envers") } else { with_icon(ic::PLAY, "endroit") };
                let cut = if seg.fade > 0.0 { format!("fondu {:.1} s", seg.fade) } else { "coupe".into() };
                ui.label(
                    egui::RichText::new(format!(
                        "{}  ·  {dir}  ·  ×{:.2}  ·  {cut}  ·  {:.1} s",
                        file_name(&seg.clip),
                        seg.speed,
                        seg.length
                    ))
                    .small()
                    .color(TEXT_WEAK),
                );
            }
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("Vidéos du mix : {}", mix.clips.len())).strong().color(TEXT));
            if ui.small_button("Toutes").clicked() {
                mix.clips = media.clone();
            }
            if ui.small_button("Aucune").clicked() {
                mix.clips.clear();
            }
        });
        if media.is_empty() {
            ui.label(egui::RichText::new("Ajoutez d'abord des vidéos dans la liste Médias.").small().color(TEXT_WEAK));
        }
        for path in &media {
            let mut on = mix.clips.contains(path);
            let icon = if on { ic::CHECK_SQUARE } else { ic::SQUARE };
            let text = egui::RichText::new(with_icon(icon, &file_name(path))).color(if on { ACCENT } else { TEXT });
            if ui.add(egui::Button::new(text).frame(false)).clicked() {
                on = !on;
                if on {
                    mix.clips.push(path.clone());
                } else {
                    mix.clips.retain(|p| p != path);
                }
            }
        }
        if mix.enabled && mix.clips.is_empty() {
            ui.label(egui::RichText::new("Cochez au moins une vidéo.").color(WARN));
        }

        ui.add_space(6.0);
        ui.label(egui::RichText::new(with_icon(ic::SCISSORS, "Séquences")).strong().color(TEXT));
        ui.add(egui::Slider::new(&mut mix.seg_min, 0.3..=20.0).logarithmic(true).text("Durée min").suffix(" s"));
        ui.add(egui::Slider::new(&mut mix.seg_max, 0.3..=20.0).logarithmic(true).text("Durée max").suffix(" s"));
        percent_slider(ui, &mut mix.same_clip_chance, "Rester sur la même vidéo")
            .on_hover_text("Sinon on passe à une autre vidéo cochée");
        ui.add_space(4.0);
        ui.label(egui::RichText::new(with_icon(ic::REWIND, "Sens et vitesse")).strong().color(TEXT));
        percent_slider(ui, &mut mix.reverse_chance, "À l'envers");
        percent_slider(ui, &mut mix.speed_chance, "Vitesse variable")
            .on_hover_text("Part des séquences jouées à une autre vitesse que la normale");
        ui.add(egui::Slider::new(&mut mix.speed_min, 0.1..=4.0).logarithmic(true).text("Vitesse min").suffix("×"));
        ui.add(egui::Slider::new(&mut mix.speed_max, 0.1..=4.0).logarithmic(true).text("Vitesse max").suffix("×"));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(with_icon(ic::SHUFFLE, "Transitions")).strong().color(TEXT));
        percent_slider(ui, &mut mix.cut_chance, "Coupes franches").on_hover_text("Sinon fondu enchaîné");
        ui.add(egui::Slider::new(&mut mix.fade, 0.05..=3.0).text("Durée des fondus").suffix(" s"));
        if ui.small_button("Réglages par défaut").clicked() {
            let (enabled, clips) = (mix.enabled, std::mem::take(&mut mix.clips));
            *mix = AutoMix { enabled, clips, ..AutoMix::default() };
        }

        if started {
            // Lit à l'avance la durée des vidéos, pour que les changements soient instantanés.
            let clips = mix.clips.clone();
            std::thread::spawn(move || {
                for c in clips {
                    let _ = video::probe(&c);
                }
            });
        }
        if (started || next) && !mix.clips.is_empty() {
            self.mix_next(&ctx, sel);
        }
    }

    /// Importe un SVG, ou la forme détourée d'une image, comme masque du calque sélectionné.
    fn import_mask(&mut self, path: &Path) {
        let aspect = (self.output_size.x / self.output_size.y.max(1.0)).clamp(0.2, 5.0);
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
            // Photo ou image : détourage automatique, à vérifier et retoucher avant d'appliquer.
            match Cutout::open(path) {
                Ok(cut) => {
                    self.cutout = Some(CutoutEditor { cut, name: file_name(path), tex: None, dirty: true, drag_from: None, drag_corner: None, white: false });
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
    fn apply_mask_rings(&mut self, rings: Vec<Vec<surface::Point>>, nonzero: bool) {
        let sel = self.sel();
        let mask = &mut self.project.layers[sel].mask;
        mask.set_rings(rings, nonzero);
        mask.enabled = true;
        self.show_shape_tab(EditTarget::Mask);
    }

    /// Fenêtre de détourage d'une photo.
    fn cutout_window(&mut self, ctx: &egui::Context) {
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
                if ui.add(theme::primary_button(with_icon(ic::CHECK_SQUARE, "Appliquer au calque"))).clicked() {
                    apply = Some(ed.cut.rings(aspect));
                }
                if ui.button("Fermer").clicked() {
                    close = true;
                }
            });
        });
        match apply {
            Some(Ok((rings, nonzero))) => {
                self.apply_mask_rings(rings, nonzero);
                self.cutout = None;
            }
            Some(Err(e)) => self.status = Some(format!("Détourage : {e:#}")),
            None if close => self.cutout = None,
            None => {}
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

impl eframe::App for OzApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.tick(&ctx);
        if self.cutout.is_none() {
            self.handle_shortcuts(&ctx);
        }

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
        } else if let Some(shape) = dropped
            .iter()
            .find(|p| is_svg_file(p) || (self.right_tab == RightTab::Shape && video::is_still_image(p)))
        {
            // Un SVG déposé (ou une photo déposée quand l'onglet Forme est ouvert) devient le masque
            // du calque sélectionné. Sinon, les photos s'ajoutent aux médias.
            self.import_mask(shape);
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
                                "Espace pause  ·  R envers  ·  0 sans effet  ·  E tout sans effets  ·  A auto  ·  B blackout  ·  M forme  ·  F plein écran  ·  {CMD}+S enregistrer"
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
        // Largeur limitée : l'aperçu garde toujours sa place au centre.
        let win_w = ui.available_width();
        egui::Panel::right("effects")
            .resizable(true)
            .default_size(420.0)
            .size_range(320.0..=(win_w * 0.45).max(320.0))
            .frame(panel_frame)
            .show(ui, |ui| self.right_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ui, |ui| self.preview(ui));

        self.media_drag_preview(&ctx);
        self.cutout_window(&ctx);

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
        self.store_active_cells();
        let session = Session {
            project: Some(self.project.clone()),
            project_path: self.project_path.clone(),
            output_screen: self.output_screen.clone(),
            rhythm_source: self.rhythm.source.clone(),
            rhythm_band: self.rhythm.band,
            bpm: Some(self.rhythm.tempo.bpm),
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
/// Onglet latéral : icône au-dessus du nom, barre d'accent quand il est choisi.
fn side_tab(ui: &mut egui::Ui, icon: &str, name: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 6.0, theme::ACCENT_DIM.gamma_multiply(0.55));
        // Barre d'accent côté bord de la fenêtre (les onglets sont à droite).
        let bar = Rect::from_min_size(egui::pos2(rect.max.x - 3.0, rect.min.y + 8.0), Vec2::new(3.0, rect.height() - 16.0));
        p.rect_filled(bar, 1.5, ACCENT);
    } else if resp.hovered() {
        p.rect_filled(rect, 6.0, theme::WIDGET);
    }
    let color = if selected { ACCENT } else if resp.hovered() { TEXT } else { TEXT_WEAK };
    p.text(rect.center() - Vec2::new(0.0, 8.0), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(20.0), color);
    p.text(
        rect.center() + Vec2::new(0.0, 13.0),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(11.0),
        if selected { TEXT } else { TEXT_WEAK },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Action demandée depuis le tableau.
enum CellAction {
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
enum MediaDrag {
    File(PathBuf),
    Pattern(Pattern),
}

/// Vignette d'un visuel généré : dégradé de sa palette, décalé selon le motif (pour les distinguer).
fn paint_pattern_swatch(painter: &egui::Painter, rect: Rect, pattern: Pattern, palette: Palette) {
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
fn cell_widget(
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
    ui.horizontal(|ui| {
        ui.label("Thème");
        let before = auto.theme;
        egui::ComboBox::from_id_salt("auto_theme")
            .selected_text(auto.theme.label())
            .width(140.0)
            .show_ui(ui, |ui| {
                for t in AutoTheme::ALL {
                    ui.selectable_value(&mut auto.theme, t, t.label()).on_hover_text(t.description());
                }
            });
        // Changer de thème en cours de route : effet du nouveau thème tout de suite.
        if auto.enabled && auto.theme != before {
            *next = true;
        }
    });
    ui.label(egui::RichText::new(auto.theme.description()).small().color(TEXT_WEAK));
    ui.add(egui::Slider::new(&mut auto.interval, 1.0..=60.0).text("Changer toutes les").suffix(" s"));
    ui.add(egui::Slider::new(&mut auto.transition, 0.0..=10.0).text("Fondu").suffix(" s"));
    ui.add(egui::Slider::new(&mut auto.intensity, 0.3..=1.5).text("Intensité"))
        .on_hover_text("Dosage des effets générés : faible = subtil, fort = extrême");
    if auto.theme == AutoTheme::All {
        percent_slider(ui, &mut auto.novelty, "Effets inédits")
            .on_hover_text("Part des changements qui inventent un nouvel effet au lieu de reprendre un preset");
    }
    ui.checkbox(&mut auto.strobe, "Autoriser le stroboscope").on_hover_text("Inclut les presets avec flashs");
}

/// Slider de pourcentage (valeur stockée entre 0 et 1).
fn percent_slider(ui: &mut egui::Ui, value: &mut f32, label: &str) -> egui::Response {
    ui.add(
        egui::Slider::new(value, 0.0..=1.0)
            .text(label)
            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
            .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
    )
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

fn is_svg_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
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
