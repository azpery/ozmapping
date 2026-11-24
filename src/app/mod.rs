//! Application : état, création et boucle principale.
//! Le reste est réparti par thème dans les sous-modules.

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
use crate::recorder;
use crate::rhythm::{self, Rhythm};
use crate::screens;
use crate::renderer::{self, DrawParams, GenDraw, LayerDraw, MaskDraw, Phases, Renderer, ShapeDraw, SourceRef};
use crate::surface::{self, ACCENT, Fit, MAX_GRID, Mask, MaskShape, Warp};
use crate::theme::{self, BG, CARD, TEXT, TEXT_WEAK, WARN, with_icon};
use crate::thumbs::Thumbnails;
use crate::video::{self, VideoPlayer};

mod cutout_ui;
mod effects_ui;
mod files;
mod grid;
mod layer_runtime;
mod layers;
mod laser_ui;
mod layout_ui;
mod media_folders;
mod leds_ui;
mod midi_ui;
mod net_ui;
mod output;
mod panels;
mod playback;
mod project_io;
mod recording;
mod rhythm_ui;
mod shape_ui;
mod shortcuts;
mod sketch_ui;
mod undo;
mod visuals_ui;
mod widgets;

use cutout_ui::*;
use files::*;
use grid::*;
use layer_runtime::*;
use laser_ui::*;
use leds_ui::*;
use midi_ui::*;
use net_ui::*;
use sketch_ui::*;
use output::*;
use widgets::*;

const MAX_LAYERS: usize = 16;
/// Touche de commande affichée dans les libellés (les raccourcis utilisent Ctrl hors macOS).
const CMD: &str = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };

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
    /// Contrôleur MIDI : port et commandes assignées (indépendants du projet).
    midi: crate::midi::MidiSetup,
    /// Envoi aux WLED et en DMX actifs : ils reprennent tout seuls après un redémarrage
    /// (un crash en plein spectacle ne laisse pas les LEDs éteintes).
    leds_live: bool,
    dmx_live: bool,
    /// Synchronisation avec les autres OZ Mapping du réseau local.
    net: crate::net::NetSettings,
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
    Leds,
    Dmx,
    Laser,
    Midi,
    Sketch,
    /// Disposition des vidéoprojecteurs : orientation, écran, masques de chaque sortie.
    Layout,
}

impl RightTab {
    const COUNT: usize = 12;

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
                (ic::SPEEDOMETER, "Route"),
                (ic::EYE, "Visage"),
            ],
            RightTab::Shape => &[
                (ic::FRAME_CORNERS, "Cadrage"),
                (ic::GRID_FOUR, "Déformation"),
                (ic::SHAPES, "Masque"),
            ],
            RightTab::Rhythm => &[(ic::WAVES, "Source"), (ic::SPARKLE, "Réaction")],
            RightTab::Leds => &[(ic::CUBE, "Scène"), (ic::LIGHTBULB, "LEDs"), (ic::WIFI_HIGH, "Contrôleurs"), (ic::PLAY, "Sortie")],
            RightTab::Dmx => &[(ic::LIGHTNING, "Appareils"), (ic::WIFI_HIGH, "Sortie")],
            RightTab::Laser => &[(ic::PENCIL_SIMPLE, "ILDA (formes)"), (ic::SQUARES_FOUR, "DMX (motifs)")],
            RightTab::Photo | RightTab::Mix | RightTab::Midi | RightTab::Sketch | RightTab::Layout => &[],
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
    /// Sorties 2 et suivantes (index = rang de la sortie ; l'entrée 0 n'est pas utilisée : la
    /// sortie 1 a les champs ci-dessus) ; sortie montrée par l'aperçu ; dernier calque choisi.
    outs: Vec<ExtraOut>,
    preview_out: usize,
    last_sel: usize,
    screens: Vec<screens::Screen>,
    screens_at: f64,
    rhythm: Rhythm,
    cutout: Option<CutoutEditor>,
    /// LEDs WLED : simulateur, envoi en direct.
    led: LedRuntime,
    /// Contrôleur MIDI : connexion, dessin ; et ses commandes assignées (gardées entre sessions).
    midi: MidiRuntime,
    midi_setup: crate::midi::MidiSetup,
    /// Lasers ILDA : armement, envoi aux boîtiers.
    laser: LaserRuntime,
    /// Plusieurs OZ Mapping ensemble sur le réseau local.
    net: NetRuntime,
    /// Dessin depuis un iPad ou un téléphone.
    sketch: SketchRuntime,

    blackout: bool,
    blackout_level: f32,
    /// Sortie en blanc plein (pour photographier l'image projetée, régler le projecteur).
    white_screen: bool,
    grid: bool,
    /// Contournement global : affiche les vidéos sans effets, sans perdre les réglages.
    bypass_fx: bool,
    edit_mapping: bool,
    edit_target: EditTarget,
    /// Édition sur l'aperçu de l'ensemble média + masque (cadre à redimensionner).
    edit_both: bool,
    /// Réglage des 4 coins de cette sortie en cours, et le coin choisi (flèches du clavier).
    corner_edit: Option<usize>,
    corner_sel: usize,
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
    /// Calques cochés pour être groupés (Cmd ou Maj + clic sur leur nom).
    group_pick: std::collections::BTreeSet<usize>,
    /// Dossiers de médias suivis : leur contenu et la liste de tous les médias.
    media_folders: media_folders::MediaFolders,
    /// Défilement horizontal commun aux lignes du tableau.
    grid_offset: f32,
    /// Enregistrement de la sortie en cours, et ses réglages (hauteur, images/s, durée max).
    recorder: Option<Arc<Mutex<recorder::Recorder>>>,
    rec_height: u32,
    rec_fps: u32,
    rec_max_secs: f32,
    /// Résultat du dernier enregistrement (fichier écrit, ou erreur), rempli en arrière-plan.
    rec_result: Arc<Mutex<Option<Result<PathBuf, String>>>>,

    /// Annuler / rétablir.
    history: undo::History,
    preset_name: String,
    mask_preset_name: String,
    /// Calage en trois clics de la photo du visage : demandé, puis en cours.
    face_calib_wanted: bool,
    face_calib: Option<visuals_ui::FaceCalib>,
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
            outs: Vec::new(),
            preview_out: 0,
            last_sel: usize::MAX,
            screens: screens::list(),
            screens_at: 0.0,
            rhythm: Rhythm::default(),
            cutout: None,
            led: LedRuntime::default(),
            midi: MidiRuntime::default(),
            laser: LaserRuntime::default(),
            net: NetRuntime::default(),
            sketch: SketchRuntime::default(),
            midi_setup: crate::midi::MidiSetup::default(),
            blackout: false,
            blackout_level: 0.0,
            white_screen: false,
            grid: false,
            bypass_fx: false,
            edit_mapping: false,
            edit_target: EditTarget::Warp,
            edit_both: false,
            corner_edit: None,
            corner_sel: 0,
            right_tab: RightTab::Effects,
            fx_hidden_alert: None,
            sub_tab: [0; RightTab::COUNT],
            chain_col: None,
            chain_timer: 0.0,
            layer_card_w: 0.0,
            group_pick: Default::default(),
            media_folders: Default::default(),
            grid_offset: 0.0,
            recorder: None,
            rec_height: 1080,
            rec_fps: 30,
            rec_max_secs: 0.0,
            rec_result: Arc::new(Mutex::new(None)),
            history: undo::History::default(),
            preset_name: String::new(),
            mask_preset_name: String::new(),
            face_calib_wanted: false,
            face_calib: None,
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
        app.midi_setup = session.midi.clone();
        app.led.live = session.leds_live;
        app.led.dmx_live = session.dmx_live;
        app.net.settings = session.net.clone();
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

    fn section(&self) -> Section {
        Section(self.sub_tab[self.right_tab as usize])
    }

    fn sel(&self) -> usize {
        self.selected.min(self.project.layers.len() - 1)
    }

    fn is_dirty(&self) -> bool {
        self.project != self.saved
    }
}

impl eframe::App for OzApp {
    /// Fenêtre principale cachée (autre app devant, sortie en plein écran par-dessus) : eframe
    /// n'appellerait plus que cette fonction, et la lecture, les sorties, les LEDs et l'iPad
    /// s'arrêteraient. Le spectacle continue : on redemande l'interface complète à chaque image.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::RequestPaintWhileHidden);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.undo_begin(&ctx);
        self.tick(&ctx);
        self.midi_tick(&ctx);
        self.leds_tick(&ctx);
        self.laser_tick(&ctx);
        self.net_tick(&ctx);
        self.media_folders_tick(&ctx);
        self.chem_persist_tick();
        self.sketch_tick(&ctx);
        if self.cutout.is_none() {
            self.drone_keys(&ctx);
            self.scene_shortcuts(&ctx);
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
                    let rec_result = self.rec_result.lock().unwrap().clone();
                    if let Some(s) = &self.status {
                        ui.label(egui::RichText::new(with_icon(ic::PROHIBIT, s)).color(theme::DANGER));
                    } else if let Some(n) = self.net_notice() {
                        ui.label(egui::RichText::new(with_icon(ic::WIFI_HIGH, n)).color(ACCENT));
                    } else if let Some(Ok(path)) = &rec_result {
                        ui.label(egui::RichText::new(with_icon(ic::CHECK_SQUARE, &format!("Vidéo enregistrée : {}", file_name(path)))).color(ACCENT));
                        if ui.small_button("Afficher le fichier").clicked() {
                            reveal_in_file_manager(path);
                        }
                        if ui.small_button(ic::X).clicked() {
                            *self.rec_result.lock().unwrap() = None;
                        }
                    } else if let Some(Err(e)) = &rec_result {
                        ui.label(egui::RichText::new(with_icon(ic::PROHIBIT, &format!("Enregistrement : {e}"))).color(theme::DANGER));
                        if ui.small_button(ic::X).clicked() {
                            *self.rec_result.lock().unwrap() = None;
                        }
                    } else if self.right_tab == RightTab::Leds {
                        ui.label(
                            egui::RichText::new(format!(
                                "Scène 3D :  flèches : haut, bas, gauche, droite (Maj ×10)  ·  PgHaut/PgBas : profondeur  ·  Q/E ou [ ] tourner de 15°  ·  Suppr supprimer  ·  {CMD}+D dupliquer  ·  D dessiner un tracé  ·  V changer de vue  ·  Échap désélectionner"
                            ))
                            .small()
                            .color(TEXT_WEAK),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(format!(
                                "Espace pause  ·  R envers  ·  0 sans effet  ·  E tout sans effets  ·  A auto  ·  B blackout  ·  M forme  ·  F plein écran  ·  {CMD}+Z annuler  ·  {CMD}+S enregistrer"
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
            .show(ui, |ui| {
                self.leds_capture(ui);
                self.sketch_capture(ui, None);
                // Sortie, scène 3D ou les deux, quel que soit l'onglet de droite.
                self.leds_central(ui);
            });

        self.media_drag_preview(&ctx);
        self.cutout_window(&ctx);

        self.undo_end(&ctx);
        // Sorties 2 et suivantes : chacune sa fenêtre.
        self.sync_outputs();
        for k in 1..self.outs.len() {
            if self.outs[k].open {
                let title = format!("OZ Mapping — {}", self.project.outputs[k].name);
                ctx.show_viewport_immediate(
                    extra_viewport_id(k),
                    ViewportBuilder::default().with_title(title).with_inner_size([960.0, 540.0]),
                    |ui, _class| self.extra_output_window(ui, k),
                );
            }
        }
        if self.output_open {
            ctx.show_viewport_immediate(
                output_viewport_id(),
                ViewportBuilder::default()
                    .with_title(format!("OZ Mapping — {}", self.project.outputs.first().map_or("Sortie", |o| o.name.as_str())))
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
            midi: self.midi_setup.clone(),
            leds_live: self.led.live,
            dmx_live: self.led.dmx_live,
            net: self.net.settings.clone(),
        };
        eframe::set_value(storage, eframe::APP_KEY, &session);
    }

    /// Session enregistrée toutes les 5 s : après un crash, on repart de là.
    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(5)
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
