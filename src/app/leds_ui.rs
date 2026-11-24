//! Onglet LEDs : installation modélisée en 3D, simulateur, contrôleurs WLED et envoi en direct.

use std::collections::HashMap;

use super::*;
use crate::leds::{self, Fixture, FixtureKind, Led, LedSetup, Projection, Source};

/// Caméra du simulateur : elle tourne autour d'un point de la salle.
#[derive(Clone, Copy)]
pub(super) struct Orbit {
    yaw: f32,
    pitch: f32,
    dist: f32,
    target: [f32; 3],
}

impl Orbit {
    fn home(room: [f32; 2]) -> Self {
        Self { yaw: 0.5, pitch: 0.42, dist: room[0].max(room[1]) * 0.85 + 1.5, target: [0.0, 0.8, room[1] * 0.5] }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LedView {
    /// Sortie au-dessus, simulateur en dessous.
    Both,
    Sim,
    Output,
    /// Éditeur des tracés 2D, sur l'image de la sortie ou une photo.
    Plan,
}

/// Planche à dessin 2D : la forme d'un bandeau, en mètres (y vers le haut), avant de l'ajouter
/// à la scène (ou celle d'un tracé plat de la scène, modifié en direct).
pub(super) struct Sketch {
    points: Vec<[f32; 2]>,
    closed: bool,
    /// Élément de la scène modifié (id) ; aucun : nouvelle forme, pas encore ajoutée.
    editing: Option<u32>,
    /// On pose des points (sinon on modifie la forme).
    drawing: bool,
    /// LEDs par mètre.
    density: f32,
}

impl Default for Sketch {
    fn default() -> Self {
        Self { points: Vec::new(), closed: false, editing: None, drawing: true, density: 60.0 }
    }
}

/// Ce qu'on fait glisser sur la planche : un point, ou toute la forme.
#[derive(Clone, Copy)]
enum SketchGrab {
    Point(usize),
    All,
}

/// État des LEDs propre à la session (non enregistré).
pub(super) struct LedRuntime {
    /// Les couleurs partent vers les WLED. Coupé au lancement : on teste d'abord dans le simulateur.
    pub(super) live: bool,
    pub(super) sender: leds::Sender,
    /// Image de la sortie lue sur le GPU (remplie par le rendu).
    pub(super) frames: HashMap<usize, Arc<Mutex<leds::Frame>>>,
    /// LEDs calculées pour l'installation `cache_of`.
    cache_of: Option<LedSetup>,
    leds: Vec<Led>,
    colors: Vec<[u8; 3]>,
    last_send: f64,
    pub(super) selected: Option<usize>,
    /// Élément qui clignote (repérage) et heure de début.
    flash: Option<(usize, f64)>,
    /// Objet du décor choisi (exclusif avec `selected`).
    pub(super) selected_surface: Option<usize>,
    /// Groupe d'objets choisi : les déplacements s'appliquent à tous ses membres.
    selected_set: Option<u32>,
    /// Clic dans une liste du panneau (objet, Maj enfoncée), traité à l'image suivante.
    pending_click: Option<(Target, bool)>,
    /// Bandeaux choisis ensemble (id) : Cmd+clic, Maj+clic ; et dernier rang cliqué (plages).
    multi: Vec<u32>,
    /// Objets du décor et appareils DMX choisis ensemble (id), par le rectangle de sélection.
    multi_surf: Vec<u32>,
    multi_dmx: Vec<u32>,
    /// Rectangle de sélection en cours (coin de départ).
    marquee: Option<egui::Pos2>,
    anchor: Option<usize>,
    /// Glisser-déposer en cours dans une liste (0 : bandeaux, 1 : groupes), rangs glissés.
    list_drag: Option<(u8, Vec<usize>)>,
    /// Contrôleurs (id) des bandes des chips : côté gauche, côté droit.
    chips_ctrl: [u32; 2],
    /// Planche 2D : forme en cours, demande venue du panneau (Some(None) : nouvelle forme,
    /// Some(Some(i)) : modifier l'élément i), glissement, vue (centre en m, pixels par m),
    /// photo de fond chargée (chemin, texture).
    sketch: Sketch,
    sketch_req: Option<Option<usize>>,
    sketch_grab: Option<SketchGrab>,
    sketch_view: ([f32; 2], f32),
    plan_tex: Option<(std::path::PathBuf, egui::TextureHandle)>,
    /// Réglages du générateur de tunnel d'arches.
    tunnel: leds::Tunnel,
    /// Appareil DMX choisi ; envoi DMX en direct ; rafales en cours (id → fin, infini : tant
    /// qu'on appuie) ; dernier envoi.
    pub(super) selected_dmx: Option<usize>,
    pub(super) dmx_live: bool,
    dmx_sender: crate::dmx::Sender,
    pub(super) bursts: HashMap<u32, f64>,
    dmx_last: f64,
    serial_ports: Vec<String>,
    /// Couleur dominante de la vidéo de chaque sortie, lissée.
    video_avg: HashMap<usize, [f32; 3]>,
    camera: Option<Orbit>,
    /// Vue vers laquelle la caméra glisse en douceur (boutons de vue, double-clic, tout voir).
    cam_goal: Option<Orbit>,
    /// Point d'ajout : les éléments ajoutés apparaissent ici (clic droit dans la scène).
    pub(super) cursor3d: Option<[f32; 3]>,
    /// Souris sur la vue 3D à l'image précédente (le clavier pilote alors le drone) ;
    /// mode drone : glisser regarde autour de soi au lieu de tourner autour d'un point.
    hover_3d: bool,
    drone: bool,
    drag: Option<Drag>,
    /// Image de la sortie sur les écrans de la scène, et numéro de l'image affichée.
    /// Image de chaque sortie sur les écrans de la scène, et numéro de l'image affichée.
    screen_tex: HashMap<usize, (egui::TextureHandle, u64)>,
    /// Tracé libre en cours de dessin dans la vue 3D.
    drawing: Option<Drawing>,
    /// Montre l'ordre des LEDs dans la vue 3D (début, sens, fil d'un bandeau au suivant).
    show_wiring: bool,
    /// Câblage à la souris en cours, et sortie proposée (contrôleur, sortie).
    wiring: Option<WireMode>,
    wire_target: (u32, usize),
    /// Vue de la zone centrale choisie (aucune : automatique, voir `shown_view`).
    pub(super) view: Option<LedView>,
    /// Messages des opérations sur les contrôleurs (lecture, configuration), par id.
    jobs: Arc<Mutex<HashMap<u32, String>>>,
    /// Lectures terminées, à reporter dans les réglages.
    reads: Arc<Mutex<Vec<(u32, leds::WledInfo)>>>,
    infos: HashMap<u32, String>,
    confirm_push: Option<u32>,
}

impl Default for LedRuntime {
    fn default() -> Self {
        Self {
            live: false,
            sender: leds::Sender::new(),
            frames: HashMap::new(),
            cache_of: None,
            leds: Vec::new(),
            colors: Vec::new(),
            last_send: 0.0,
            selected: None,
            flash: None,
            selected_surface: None,
            selected_set: None,
            pending_click: None,
            multi: Vec::new(),
            multi_surf: Vec::new(),
            multi_dmx: Vec::new(),
            marquee: None,
            anchor: None,
            list_drag: None,
            chips_ctrl: [0, 0],
            sketch: Sketch::default(),
            sketch_req: None,
            sketch_grab: None,
            sketch_view: ([0.0, 1.0], 120.0),
            plan_tex: None,
            tunnel: leds::Tunnel::default(),
            selected_dmx: None,
            dmx_live: false,
            dmx_sender: crate::dmx::Sender::default(),
            bursts: HashMap::new(),
            dmx_last: 0.0,
            serial_ports: Vec::new(),
            video_avg: HashMap::new(),
            camera: None,
            cam_goal: None,
            cursor3d: None,
            hover_3d: false,
            drone: false,
            drag: None,
            screen_tex: HashMap::new(),
            drawing: None,
            show_wiring: false,
            wiring: None,
            wire_target: (0, 0),
            view: None,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            reads: Arc::new(Mutex::new(Vec::new())),
            infos: HashMap::new(),
            confirm_push: None,
        }
    }
}

/// Objet de la scène qu'on peut choisir et déplacer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Fixture(usize),
    Surface(usize),
    /// Appareil DMX (index dans `dmx`).
    Dmx(usize),
    /// Laser ILDA (index dans `lasers`).
    Laser(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragMode {
    /// Au sol.
    Move,
    /// En hauteur.
    Lift,
    /// Rotation autour de la verticale.
    Rotate,
}

#[derive(Clone, Copy)]
struct Drag {
    target: Target,
    mode: DragMode,
    /// Écart entre l'objet et le point du sol saisi.
    offset: [f32; 2],
}

/// Tracé libre en cours : points posés (dans la salle) et élément redessiné (sinon : nouveau).
struct Drawing {
    points: Vec<[f32; 3]>,
    fixture: Option<usize>,
}

/// Câblage à la souris : les bandeaux cliqués se suivent, dans l'ordre, sur cette sortie.
struct WireMode {
    controller: u32,
    output: usize,
    /// Bandeaux déjà câblés (id), dans l'ordre.
    chain: Vec<u32>,
}

/// LEDs par mètre d'un nouveau tracé.
const PATH_DENSITY: f32 = 30.0;

/// Ce que dessine le simulateur (trié par distance).
enum Item {
    /// Morceau d'une face (les grandes faces sont découpées : le tri par distance reste juste).
    Tile { pts: [egui::Pos2; 4], z: [f32; 4], mesh: egui::Mesh, surface: usize, quad: [[f32; 3]; 4] },
    /// Trait (arête, mât) ; `z` : sa vraie distance, `hidden` : caché par une face.
    Wire { a: egui::Pos2, b: egui::Pos2, stroke: Stroke, z: f32, hidden: bool },
    /// Faisceau lumineux (laser, derby, projecteur), en lumière additive.
    Beam { a: egui::Pos2, b: egui::Pos2, c: [u8; 3], w: f32 },
    /// Bouffée de fumée (disque) ou bulle (cercle).
    Puff { p: egui::Pos2, r: f32, c: Color32, ring: bool },
    /// `hidden` : cachée par une face (seul son halo se voit, comme la lumière qui déborde).
    Led { p: egui::Pos2, r: f32, c: [u8; 3], i: usize, hidden: bool },
}

/// Durée du clignotement de repérage.
const FLASH_SECS: f64 = 2.5;

impl OzApp {
    /// LEDs de la scène recalculées si la scène a changé (et choix d'objets disparus retirés).
    pub(super) fn refresh_led_cache(&mut self) {
        if self.led.cache_of.as_ref() != Some(&self.project.leds) {
            self.led.leds = self.project.leds.leds();
            self.led.cache_of = Some(self.project.leds.clone());
            if self.led.selected.is_some_and(|s| s >= self.project.leds.fixtures.len()) {
                self.led.selected = None;
            }
            if self.led.selected_surface.is_some_and(|s| s >= self.project.leds.surfaces.len()) {
                self.led.selected_surface = None;
            }
            if self.led.selected_dmx.is_some_and(|s| s >= self.project.leds.dmx.len()) {
                self.led.selected_dmx = None;
            }
        }
    }

    /// LEDs utiles en ce moment (l'image de la sortie doit alors être lue pour elles).
    /// Sorties dont il faut lire l'image, et à quelle largeur : celle que suivent les LEDs, et
    /// celles affichées sur les écrans de la scène 3D (plus fines).
    fn frames_needed(&self) -> Vec<(usize, u32)> {
        let setup = &self.project.leds;
        let tab = self.right_tab == RightTab::Leds;
        let n = self.project.outputs.len().max(1);
        let video = |s: Source| matches!(s, Source::Video | Source::VideoAverage);
        let uses_video = video(setup.source) || setup.groups.iter().any(|g| g.effect.as_ref().is_some_and(|e| video(e.source)));
        let mut need: Vec<(usize, u32)> = Vec::new();
        let mut add = |k: usize, w: u32| match need.iter_mut().find(|e| e.0 == k) {
            Some(e) => e.1 = e.1.max(w),
            None => need.push((k, w)),
        };
        if !setup.fixtures.is_empty() && uses_video && (self.led.live || tab || self.shown_view() != LedView::Output) {
            for k in self.video_outputs() {
                add(k, 192);
            }
        }
        if self.shown_view() != LedView::Output {
            for s in setup.surfaces.iter().filter(|s| s.show_output) {
                add(s.output.min(n - 1), 448);
            }
        }
        // Lasers qui tracent les contours d'une sortie.
        for l in setup.lasers.iter().filter(|l| l.cue.shape == crate::laser::Shape::Video) {
            add(l.cue.video_output.min(n - 1), 192);
        }
        need
    }

    /// Sorties suivies par les LEDs : celle de l'effet général, et celles choisies par des groupes.
    fn video_outputs(&self) -> Vec<usize> {
        let n = self.project.outputs.len().max(1);
        let l = &self.project.leds;
        let mut v = vec![l.video_output.min(n - 1)];
        for e in l.groups.iter().filter_map(|g| g.effect.as_ref()) {
            if let Some(k) = e.output {
                if !v.contains(&k.min(n - 1)) {
                    v.push(k.min(n - 1));
                }
            }
        }
        v
    }

    /// Image lue d'une sortie (créée à la demande).
    pub(super) fn frame_of(&mut self, k: usize) -> Arc<Mutex<leds::Frame>> {
        self.led.frames.entry(k).or_insert_with(|| Arc::new(Mutex::new(leds::Frame::default()))).clone()
    }

    /// Vue de la zone centrale : celle choisie, sinon la sortie seule tant que la scène est vide,
    /// et sortie + 3D dès qu'elle contient quelque chose. Le plan 2D n'existe que dans l'onglet LEDs.
    pub(super) fn shown_view(&self) -> LedView {
        let l = &self.project.leds;
        let empty = l.fixtures.is_empty() && l.surfaces.is_empty() && l.dmx.is_empty() && l.lasers.is_empty();
        match self.led.view {
            Some(LedView::Plan) if self.right_tab != RightTab::Leds => LedView::Both,
            Some(v) => v,
            None if empty => LedView::Output,
            None => LedView::Both,
        }
    }


    /// Calcule les couleurs des LEDs et les envoie aux contrôleurs (à chaque image).
    pub(super) fn leds_tick(&mut self, ctx: &egui::Context) {
        if let Some((t, shift)) = self.led.pending_click.take() {
            if shift { self.add_to_group(t) } else { self.select(t) }
        }
        if self.led.selected_set.is_some_and(|id| !self.project.leds.sets.iter().any(|g| g.id == id)) {
            self.led.selected_set = None;
        }
        // Lectures de contrôleurs terminées : pins et ordre des couleurs reportés dans les réglages.
        for (id, info) in std::mem::take(&mut *self.led.reads.lock().unwrap()) {
            if let Some(c) = self.project.leds.controllers.iter_mut().find(|c| c.id == id) {
                if !info.outputs.is_empty() {
                    c.pins = info.outputs.iter().map(|o| o.0).collect();
                }
                if let Some(o) = info.order {
                    c.order = o;
                }
            }
            self.led.infos.insert(id, info.summary());
        }

        self.refresh_led_cache();
        let now = self.clock();
        if self.led.flash.is_some_and(|(_, t)| now - t > FLASH_SECS) {
            self.led.flash = None;
        }
        let flash = self.led.flash.map(|(f, t)| (f, (now - t) as f32));
        let level = 1.0 - self.blackout_level;
        self.led.colors = {
            // Sorties suivies : celle de l'effet général et celles des groupes.
            let main = self.project.leds.video_output.min(self.project.outputs.len().max(1) - 1);
            let outs = self.video_outputs();
            let srcs: Vec<(usize, Arc<Mutex<leds::Frame>>)> = outs.iter().map(|&k| (k, self.frame_of(k))).collect();
            let guards: Vec<(usize, std::sync::MutexGuard<leds::Frame>)> = srcs.iter().map(|(k, a)| (*k, a.lock().unwrap())).collect();
            let mut video = leds::VideoIn { main, ..leds::VideoIn::default() };
            for (k, g) in &guards {
                // Couleur dominante de la vidéo, lissée (pas de scintillement d'une image à l'autre).
                let avg = g.average();
                let sm = self.led.video_avg.entry(*k).or_insert(avg);
                for c in 0..3 {
                    sm[c] += (avg[c] - sm[c]) * 0.15;
                }
                video.avg.insert(*k, *sm);
                video.frames.insert(*k, &**g);
            }
            let music = leds::Music {
                beat: self.rhythm.tempo.beat_at(now) as f32,
                level: self.rhythm.pulse.level,
                env: self.rhythm.pulse.env,
                bass: self.rhythm.pulse.bass,
            };
            leds::colors(&self.project.leds, &self.led.leds, &video, now as f32, level, flash, music)
        };

        // DMX : envoi à sa cadence, rafales terminées retirées.
        self.led.bursts.retain(|_, until| *until > now);
        if self.led.dmx_live && !self.project.leds.dmx.is_empty() {
            ctx.request_repaint();
            let fps = f64::from(self.project.leds.dmx_out.fps.max(1));
            if now - self.led.dmx_last >= 1.0 / fps {
                self.led.dmx_last = now;
                let universes = crate::dmx::universes(&self.project.leds.dmx, &|d| self.dmx_live_state(d));
                self.led.dmx_sender.send(&self.project.leds.dmx_out, universes);
            }
        }
        if !self.led.bursts.is_empty() {
            ctx.request_repaint();
        }
        let sending = self.led.live || flash.is_some();
        if !sending || self.project.leds.fixtures.is_empty() {
            return;
        }
        ctx.request_repaint();
        if now - self.led.last_send < 1.0 / f64::from(self.project.leds.fps.max(1)) {
            return;
        }
        self.led.last_send = now;
        let setup = &self.project.leds;
        for (ci, c) in setup.controllers.iter().enumerate() {
            if c.host.trim().is_empty() || !(c.enabled || flash.is_some()) {
                continue;
            }
            // Repérage hors direct : seul le contrôleur de l'élément qui clignote reçoit l'image.
            if !self.led.live && flash.is_some_and(|(f, _)| setup.fixtures.get(f).is_none_or(|f| f.controller != c.id)) {
                continue;
            }
            let total = setup.layout(ci).total;
            let mut rgb = vec![0u8; total * 3];
            for (led, col) in self.led.leds.iter().zip(&self.led.colors) {
                if led.controller == Some(ci) && led.address < total {
                    rgb[led.address * 3..led.address * 3 + 3].copy_from_slice(col);
                }
            }
            self.led.sender.send(&c.host, rgb);
        }
    }

    /// Fait lire l'image de la sortie pour les LEDs (petite définition), si besoin.
    pub(super) fn leds_capture(&mut self, ui: &egui::Ui) {
        let Some(r) = self.renderer.clone() else { return };
        for (k, w) in self.frames_needed() {
            let o = self.out_size(k);
            let aspect = (o.x / o.y.max(1.0)).clamp(0.2, 5.0);
            let size = (w, ((w as f32 / aspect) as u32).clamp(8, 1024));
            let sink = self.frame_of(k);
            let params = self.draw_params_for(k);
            ui.painter().add(renderer::led_capture_callback(ui.max_rect(), r.clone(), params, k, size, sink));
        }
    }

    /// Points de l'image où les LEDs prennent leur couleur, dessinés sur l'aperçu de la sortie.
    pub(super) fn leds_overlay(&self, painter: &egui::Painter, rect: Rect) {
        if self.right_tab != RightTab::Leds || self.project.leds.source != Source::Video {
            return;
        }
        for led in &self.led.leds {
            let p = rect.min + Vec2::new(led.uv[0] * rect.width(), led.uv[1] * rect.height());
            let sel = self.led.selected == Some(led.fixture);
            let color = if sel { ACCENT } else { Color32::from_white_alpha(150) };
            painter.circle_stroke(p, if sel { 2.5 } else { 1.8 }, Stroke::new(1.0, color));
        }
    }

    /// Zone centrale quand l'onglet LEDs est ouvert : sortie et/ou simulateur 3D.
    pub(super) fn leds_central(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let bar = Rect::from_min_size(full.min + Vec2::new(14.0, 6.0), Vec2::new(full.width() - 28.0, 24.0));
        let shown = self.shown_view();
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            ui.horizontal(|ui| {
                for (v, name) in [(LedView::Both, "Sortie + 3D"), (LedView::Sim, "Scène 3D"), (LedView::Plan, "Plan 2D"), (LedView::Output, "Sortie")] {
                    if v == LedView::Plan && self.right_tab != RightTab::Leds {
                        continue;
                    }
                    if ui.selectable_label(shown == v, name).clicked() {
                        self.led.view = Some(v);
                    }
                }
                let n = self.led.leds.len();
                if n == 0 {
                    return;
                }
                ui.separator();
                let state = if self.led.live {
                    egui::RichText::new(with_icon(ic::WIFI_HIGH, &format!("{n} LEDs · envoi aux WLED"))).color(ACCENT)
                } else {
                    egui::RichText::new(format!("{n} LEDs · simulation seulement")).color(TEXT_WEAK)
                };
                ui.label(state);
            });
        });
        let body = Rect::from_min_max(egui::pos2(full.min.x, bar.max.y + 14.0), full.max);
        match shown {
            LedView::Output => {
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| self.preview(ui));
            }
            LedView::Sim => self.led_sim(ui, body.shrink(14.0)),
            LedView::Plan => self.led_plan(ui, body.shrink(14.0)),
            LedView::Both => {
                let split = body.min.y + body.height() * 0.4;
                let top = Rect::from_min_max(body.min, egui::pos2(body.max.x, split));
                let bottom = Rect::from_min_max(egui::pos2(body.min.x, split), body.max).shrink(14.0);
                ui.scope_builder(egui::UiBuilder::new().max_rect(top), |ui| self.preview(ui));
                self.led_sim(ui, bottom);
            }
        }
        ui.allocate_rect(full, Sense::hover());
    }

    /// Simulateur 3D : la salle, le décor (écrans, volumes) et les LEDs allumées.
    fn led_sim(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let room = self.project.leds.room;
        // Glissement en douceur vers la vue demandée.
        if let Some(goal) = self.led.cam_goal {
            let cam = self.led.camera.get_or_insert_with(|| Orbit::home(room));
            let k = (ui.input(|i| i.stable_dt) * 8.0).clamp(0.05, 1.0);
            let dyaw = (goal.yaw - cam.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            cam.yaw += dyaw * k;
            cam.pitch += (goal.pitch - cam.pitch) * k;
            cam.dist += (goal.dist - cam.dist) * k;
            for i in 0..3 {
                cam.target[i] += (goal.target[i] - cam.target[i]) * k;
            }
            let close = dyaw.abs() < 0.002 && (goal.pitch - cam.pitch).abs() < 0.002 && (goal.dist - cam.dist).abs() < 0.01;
            if close {
                *cam = goal;
                self.led.cam_goal = None;
            }
            ui.ctx().request_repaint();
        }
        let cam = *self.led.camera.get_or_insert_with(|| Orbit::home(room));
        let resp = ui.interact(rect, ui.scope_id().with("led_sim"), Sense::click_and_drag());
        self.led.hover_3d = resp.hovered() || resp.dragged();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::from_rgb(8, 9, 12));
        self.update_screen_texture(ui.ctx());

        let view = View::new(cam, rect);
        // Sol de la salle et quadrillage d'un mètre.
        let (hw, depth) = (room[0] * 0.5, room[1]);
        let floor: Vec<_> =
            [[-hw, 0.0, 0.0], [hw, 0.0, 0.0], [hw, 0.0, depth], [-hw, 0.0, depth]].iter().filter_map(|p| view.project(*p)).collect();
        if floor.len() == 4 {
            painter.add(egui::Shape::convex_polygon(
                floor.iter().map(|p| p.0).collect(),
                Color32::from_rgb(17, 20, 26),
                Stroke::new(1.0, theme::BORDER),
            ));
        }
        let grid = Stroke::new(1.0, Color32::from_white_alpha(14));
        let mut x = -hw.floor();
        while x <= hw {
            view.line(&painter, [x, 0.0, 0.0], [x, 0.0, depth], grid);
            x += 1.0;
        }
        let mut z = 0.0;
        while z <= depth {
            view.line(&painter, [-hw, 0.0, z], [hw, 0.0, z], grid);
            z += 1.0;
        }
        if let Some((p, _)) = view.project([0.0, 0.0, -0.25]) {
            painter.text(p, egui::Align2::CENTER_TOP, "avant", egui::FontId::proportional(11.0), TEXT_WEAK);
        }

        // Tout ce qui se dessine, trié du plus loin au plus proche.
        let mut items: Vec<(f32, Item)> = Vec::new();
        let light = norm([0.35, 0.8, -0.5]);
        let n_out = self.project.outputs.len().max(1);
        let tex_of = |k: usize| self.led.screen_tex.get(&k.min(n_out - 1)).map(|t| t.0.id());
        // Les personnages dansent sur le tempo de l'onglet Rythme.
        let beat = self.rhythm.tempo.beat_at(self.clock()) as f32;
        if self.project.leds.surfaces.iter().any(|s| matches!(s.kind, leds::SurfaceKind::Crowd | leds::SurfaceKind::Sub | leds::SurfaceKind::SuperScoop | leds::SurfaceKind::CuboKick) || (s.kind == leds::SurfaceKind::Booth && s.dj)) {
            ui.ctx().request_repaint();
        }
        let set = self.led.selected_set.unwrap_or(u32::MAX);
        // Boîtiers des appareils DMX, dessinés comme des volumes (index après les objets du décor).
        let n_surf = self.project.leds.surfaces.len();
        let bodies: Vec<leds::Surface> = self
            .project
            .leds
            .dmx
            .iter()
            .map(|d| {
                let (color, size) = d.kind.body();
                leds::Surface {
                    kind: leds::SurfaceKind::Box,
                    pos: d.pos,
                    rot: d.rot,
                    size,
                    color,
                    show_output: false,
                    ..leds::Surface::default()
                }
            })
            .chain(self.project.leds.lasers.iter().map(|l| leds::Surface {
                kind: leds::SurfaceKind::Box,
                pos: l.pos,
                rot: l.rot,
                size: [0.3, 0.14, 0.34],
                color: [40, 30, 50],
                show_output: false,
                ..leds::Surface::default()
            }))
            .collect();
        let n_dmx = self.project.leds.dmx.len();
        // Vidéoprojecteurs allumés dont l'image est prête.
        let n_out = self.project.outputs.len().max(1);
        let beamers: Vec<Beamer> = self
            .project
            .leds
            .surfaces
            .iter()
            .filter(|s| s.kind == leds::SurfaceKind::Projector && s.show_output)
            .filter_map(|s| {
                let k = s.output.min(n_out - 1);
                let tex = self.led.screen_tex.get(&k).map(|t| t.0.id())?;
                let o = self.out_size(k);
                Some(Beamer { surface: s, tex, aspect: (o.x / o.y.max(1.0)).clamp(0.2, 5.0) })
            })
            .collect();
        let all_bodies = self.project.leds.surfaces.iter().enumerate().map(|(i, s)| (i, s)).chain(bodies.iter().enumerate().map(|(k, s)| (n_surf + k, s)));
        for (si, s) in all_bodies {
            let sel = if si < n_surf {
                self.led.selected_surface == Some(si) || s.set == set || self.led.multi_surf.contains(&s.id)
            } else if si < n_surf + n_dmx {
                let d = &self.project.leds.dmx[si - n_surf];
                self.led.selected_dmx == Some(si - n_surf) || d.set == set || self.led.multi_dmx.contains(&d.id)
            } else {
                let k = si - n_surf - n_dmx;
                (self.right_tab == RightTab::Laser && self.laser.selected == Some(k)) || self.project.leds.lasers[k].set == set
            };
            let edge = Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { ACCENT } else { Color32::from_black_alpha(200) });
            for f3 in s.faces(beat) {
                let face = f3.quad;
                let n = norm(cross(sub(face[1], face[0]), sub(face[3], face[0])));
                let center = [0, 1, 2].map(|i| face.iter().map(|p| p[i]).sum::<f32>() / 4.0);
                let facing = dot(n, sub(view.eye, center)) > 0.0;
                // Un volume ne montre que ses faces tournées vers nous ; un écran montre aussi son dos.
                if !facing && !matches!(s.kind, leds::SurfaceKind::Screen | leds::SurfaceKind::Panels | leds::SurfaceKind::Sun) {
                    continue;
                }
                let shade = 0.55 + 0.45 * dot(if facing { n } else { n.map(|v| -v) }, light).max(0.0);
                let base = if facing { f3.color } else { [60, 62, 68] };
                let fill = Color32::from_rgb(
                    (f32::from(base[0]) * shade) as u8,
                    (f32::from(base[1]) * shade) as u8,
                    (f32::from(base[2]) * shade) as u8,
                );
                let textured = (f3.output && facing && s.kind != leds::SurfaceKind::Projector).then(|| tex_of(s.output)).flatten();
                // Un projecteur n'éclaire pas son propre boîtier.
                let lights: Vec<Beamer> = beamers
                    .iter()
                    .filter(|b| !std::ptr::eq(b.surface, s))
                    .map(|b| Beamer { surface: b.surface, tex: b.tex, aspect: b.aspect })
                    .collect();
                face_tiles(&view, face, fill, textured.map(|t| (t, f3.uv)), edge, f3.edges, si, &mut items, &lights);
            }
        }
        self.dmx_effects(&view, &mut items, beat);
        let targets = self.laser_targets(beat);
        self.ilda_beams(&view, &mut items, &targets);
        for (i, l) in self.led.leds.iter().enumerate() {
            if let Some((p, z)) = view.project(l.pos) {
                let c = self.led.colors.get(i).copied().unwrap_or_default();
                let r = (0.022 * view.focal / z).clamp(1.2, 7.0);
                items.push((z, Item::Led { p, r, c, i, hidden: false }));
            }
        }
        // Corps des éléments LED : mât des colonnes, fil des autres.
        for (fi, f) in self.project.leds.fixtures.iter().enumerate() {
            let sel = self.led.selected == Some(fi) || f.set == set || self.led.multi.contains(&f.id);
            let color = if sel { ACCENT.gamma_multiply(0.7) } else { Color32::from_gray(60) };
            let width = if f.kind == FixtureKind::Column { if sel { 4.0 } else { 3.0 } } else if sel { 2.0 } else { 1.0 };
            let pts: Vec<[f32; 3]> = if f.kind == FixtureKind::Column {
                vec![f.pos, [f.pos[0], f.pos[1] + f.size[0], f.pos[2]]]
            } else {
                self.led.leds.iter().filter(|l| l.fixture == fi).map(|l| l.pos).collect()
            };
            for w in pts.windows(2) {
                if let (Some(a), Some(b)) = (view.project(w[0]), view.project(w[1])) {
                    // Un peu derrière les LEDs qu'il porte.
                    let z = (a.1 + b.1) * 0.5;
                    let stroke = Stroke::new(width, color);
                    items.push((z + 0.01, Item::Wire { a: a.0, b: b.0, stroke, z, hidden: false }));
                }
            }
        }
        hide_covered_leds(&mut items);
        items.sort_by(|a, b| b.0.total_cmp(&a.0));

        let hover = resp.hover_pos();
        let mut hovered_led: Option<(usize, f32)> = None;
        let mut hovered_surface: Option<usize> = None;
        let mut hovered_quad: Option<[[f32; 3]; 4]> = None;
        for (_, item) in &items {
            match item {
                Item::Tile { pts, mesh, surface, quad, .. } => {
                    painter.add(egui::Shape::mesh(mesh.clone()));
                    // Le dernier dessiné sous le pointeur est le plus proche.
                    if hover.is_some_and(|h| inside(pts, h)) {
                        hovered_surface = Some(*surface);
                        hovered_quad = Some(*quad);
                    }
                }
                Item::Beam { a, b, c, w } => {
                    painter.line_segment([*a, *b], Stroke::new(*w * 4.0, glow(*c, 0.10)));
                    painter.line_segment([*a, *b], Stroke::new(*w, glow(*c, 0.75)));
                }
                Item::Puff { p, r, c, ring } => {
                    if *ring {
                        painter.circle_stroke(*p, *r, Stroke::new(1.0, *c));
                    } else {
                        painter.circle_filled(*p, *r, *c);
                    }
                }
                Item::Wire { hidden: true, .. } => {}
                Item::Wire { a, b, stroke, .. } => {
                    painter.line_segment([*a, *b], *stroke);
                }
                Item::Led { p, r, c, i, hidden } => {
                    let lum = c.iter().map(|&v| u32::from(v)).max().unwrap_or(0);
                    if *hidden {
                        if lum > 8 {
                            // Lumière qui éclaire le panneau de derrière et déborde entre les panneaux.
                            painter.circle_filled(*p, r * 5.0, glow(*c, 0.10));
                            painter.circle_filled(*p, r * 2.2, glow(*c, 0.22));
                        }
                    } else if lum > 8 {
                        painter.circle_filled(*p, r * 3.0, glow(*c, 0.15));
                        painter.circle_filled(*p, *r, Color32::from_rgb(c[0], c[1], c[2]));
                    } else {
                        painter.circle_filled(*p, r * 0.8, Color32::from_gray(38));
                    }
                    if let Some(h) = hover {
                        let d = h.distance(*p);
                        if d < 10.0 && hovered_led.is_none_or(|(_, best)| d < best) {
                            hovered_led = Some((*i, d));
                        }
                    }
                }
            }
        }
        // Une LED sous le pointeur passe avant l'objet derrière elle.
        let drawing = self.led.drawing.is_some();
        let hovered = if drawing {
            // En dessin, les objets ne se sélectionnent pas et ne bougent pas.
            None
        } else {
            match hovered_led {
            Some((i, _)) => Some(Target::Fixture(self.led.leds[i].fixture)),
                None => hovered_surface.map(|s| {
                    if s < n_surf {
                        Target::Surface(s)
                    } else if s < n_surf + self.project.leds.dmx.len() {
                        Target::Dmx(s - n_surf)
                    } else {
                        Target::Laser(s - n_surf - self.project.leds.dmx.len())
                    }
                }),
            }
        };

        // Cotes de l'objet choisi, comme dans un logiciel d'architecture.
        if let Some(s) = self.led.selected_surface.and_then(|i| self.project.leds.surfaces.get(i)) {
            let f = s.front();
            let mid = |a: [f32; 3], b: [f32; 3]| [0, 1, 2].map(|i| (a[i] + b[i]) * 0.5);
            for (p, text) in [(mid(f[2], f[3]), format!("{:.2} m", s.size[0])), (mid(f[0], f[3]), format!("{:.2} m", s.size[1]))] {
                if let Some((q, _)) = view.project(p) {
                    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.0), BG);
                    let r = Rect::from_center_size(q, galley.size()).expand2(Vec2::new(4.0, 2.0));
                    painter.rect_filled(r, 3.0, ACCENT);
                    painter.galley(r.min + Vec2::new(4.0, 2.0), galley, BG);
                }
            }
        }

        // Souris : glisser un objet le déplace au sol (Maj : en hauteur, Alt : le tourne),
        // glisser ailleurs tourne autour de la salle.
        let shift_held = ui.input(|i| i.modifiers.shift);
        if resp.drag_started_by(egui::PointerButton::Primary) && hovered.is_none() && shift_held && !drawing && self.led.wiring.is_none() {
            // Maj + glisser dans le vide : rectangle de sélection.
            self.led.marquee = resp.interact_pointer_pos();
        } else if resp.drag_started_by(egui::PointerButton::Primary) {
            self.led.drag = hovered.and_then(|t| {
                let pos = self.target_pos(t)?;
                let mods = ui.input(|i| i.modifiers);
                let mode = if mods.alt {
                    DragMode::Rotate
                } else if mods.shift {
                    DragMode::Lift
                } else {
                    DragMode::Move
                };
                let hit = view.ray_floor(resp.interact_pointer_pos()?, pos[1])?;
                Some(Drag { target: t, mode, offset: [pos[0] - hit[0], pos[2] - hit[2]] })
            });
            if let Some(d) = self.led.drag {
                // Un objet de la sélection multiple garde la sélection (ils bougeront ensemble).
                if !(self.multi_count() > 1 && self.in_multi(d.target)) {
                    self.select(d.target);
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) {
            if let Some(d) = self.led.drag {
                let delta = resp.drag_delta();
                let pointer = resp.interact_pointer_pos();
                let snap = |v: f32| (v * 20.0).round() / 20.0;
                let before = self.target_pos(d.target).zip(self.target_rot(d.target));
                if let Some((pos, rot)) = self.target_mut(d.target) {
                    match d.mode {
                        DragMode::Move => {
                            if let Some(hit) = pointer.and_then(|p| view.ray_floor(p, pos[1])) {
                                pos[0] = snap(hit[0] + d.offset[0]);
                                pos[2] = snap(hit[2] + d.offset[1]);
                            }
                        }
                        DragMode::Lift => {
                            let z = view.project(*pos).map_or(cam.dist, |p| p.1);
                            pos[1] = (pos[1] - delta.y * z / view.focal).max(0.0);
                            pos[1] = (pos[1] * 100.0).round() / 100.0;
                        }
                        DragMode::Rotate => {
                            rot[0] = (rot[0] + delta.x * 0.5).rem_euclid(360.0);
                            if !ui.input(|i| i.modifiers.shift) {
                                // Sans Maj, la rotation s'arrête tous les 15°.
                                rot[0] = rot[0].round();
                            }
                        }
                    }
                }
                if let Some(b) = before {
                    self.spread_to_set(d.target, b);
                }
            } else if self.led.marquee.is_none() {
                let d = resp.drag_delta();
                let drone = self.led.drone;
                let cam = self.led.camera.as_mut().unwrap();
                if drone {
                    // Drone : on tourne la tête, l'œil reste en place.
                    let eye = view.eye;
                    cam.yaw += d.x * 0.006;
                    cam.pitch = (cam.pitch - d.y * 0.006).clamp(-1.55, 1.55);
                    let (cp, sp) = (cam.pitch.cos(), cam.pitch.sin());
                    let off = [cam.yaw.sin() * cp, sp, -cam.yaw.cos() * cp];
                    cam.target = [0, 1, 2].map(|k| eye[k] - off[k] * cam.dist);
                } else {
                    // Glisser dans le vide : tourner autour de la scène, dans tous les sens.
                    cam.yaw -= d.x * 0.008;
                    cam.pitch = (cam.pitch + d.y * 0.008).clamp(-1.55, 1.55);
                }
            }
        }
        // Rectangle de sélection : dessiné pendant le glissement, appliqué au lâcher.
        if let (Some(a), Some(b)) = (self.led.marquee, resp.interact_pointer_pos()) {
            let r = Rect::from_two_pos(a, b);
            painter.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.12));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
            if resp.drag_stopped() {
                let add = ui.input(|i| i.modifiers.shift);
                self.select_in_rect(r, &view, add);
                self.led.marquee = None;
            }
        }
        if !resp.dragged() && !resp.drag_stopped() {
            self.led.marquee = None;
        }
        if resp.drag_stopped() {
            if let Some(Drag { target, mode: DragMode::Rotate, .. }) = self.led.drag.filter(|_| self.led.selected_set.is_none()) {
                if let Some((_, rot)) = self.target_mut(target) {
                    if !ui.input(|i| i.modifiers.shift) {
                        rot[0] = (rot[0] / 15.0).round() * 15.0 % 360.0;
                    }
                }
            }
            self.led.drag = None;
        }
        // Clic droit glissé, ou molette enfoncée : déplacer la vue.
        if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
            let d = resp.drag_delta();
            let cam = self.led.camera.as_mut().unwrap();
            let k = cam.dist / view.focal;
            for i in 0..3 {
                cam.target[i] += (-view.right[i] * d.x + view.up[i] * d.y) * k;
            }
        }
        if resp.hovered() {
            // Molette ou pincement du trackpad : zoom vers le point visé (pas vers le centre).
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (-scroll * 0.002).exp() / pinch.max(0.01);
            if (factor - 1.0).abs() > 1e-4 {
                self.led.cam_goal = None;
                let aim = resp.hover_pos().and_then(|h| match hovered_quad {
                    Some(q) => view.ray_plane(h, q[0], norm(cross(sub(q[1], q[0]), sub(q[3], q[0])))),
                    None => view.ray_floor(h, 0.0),
                });
                let cam = self.led.camera.as_mut().unwrap();
                let new_dist = (cam.dist * factor).clamp(0.3, 120.0);
                let f = new_dist / cam.dist;
                if let Some(p) = aim.filter(|p| dot(sub(*p, view.eye), sub(*p, view.eye)).sqrt() < 200.0) {
                    for i in 0..3 {
                        cam.target[i] += (p[i] - cam.target[i]) * (1.0 - f);
                    }
                }
                cam.dist = new_dist;
            }
        }
        // Clic droit (sans glisser) : poser le point d'ajout, au sol ou sur la face visée.
        if resp.secondary_clicked() && !drawing {
            if let Some(h) = resp.interact_pointer_pos() {
                let hit = match hovered_quad {
                    Some(q) => {
                        let n = norm(cross(sub(q[1], q[0]), sub(q[3], q[0])));
                        view.ray_plane(h, q[0], n)
                    }
                    None => view.ray_floor(h, 0.0),
                };
                if let Some(p) = hit {
                    let snap = |v: f32| (v * 20.0).round() / 20.0;
                    self.led.cursor3d = Some([snap(p[0]), snap(p[1]).max(0.0), snap(p[2])]);
                }
            }
        }
        if resp.clicked() && !drawing && self.led.wiring.is_some() {
            if let Some((i, _)) = hovered_led {
                self.wire_click(i);
            }
        } else if resp.clicked() && !drawing {
            let shift = ui.input(|i| i.modifiers.shift);
            let cmd = ui.input(|i| i.modifiers.command);
            match hovered {
                Some(Target::Fixture(i)) if cmd => self.toggle_multi(i),
                Some(t) if shift => self.add_to_group(t),
                Some(t) => {
                    self.select(t);
                    if !matches!(t, Target::Dmx(_) | Target::Laser(_)) {
                        self.right_tab = RightTab::Leds;
                    }
                }
                None => {
                    self.led.selected = None;
                    self.led.selected_surface = None;
                    self.led.selected_dmx = None;
                    self.clear_multi();
                }
            }
        }
        if resp.double_clicked() && !drawing {
            match hovered {
                // Double-clic sur un objet : la vue se centre sur lui.
                Some(t) => {
                    if let Some(p) = self.target_pos(t) {
                        let cam = self.led.camera.unwrap();
                        let h = match t {
                            Target::Surface(i) => self.project.leds.surfaces.get(i).map_or(0.0, |s| s.size[1] * 0.5),
                            _ => 0.3,
                        };
                        self.led.cam_goal = Some(Orbit { target: [p[0], p[1] + h, p[2]], dist: cam.dist.min(5.0), ..cam });
                    }
                }
                // Dans le vide : tout voir.
                None => self.frame_all(None),
            }
        }
        if resp.dragged() || resp.hovered() {
            ui.ctx().request_repaint();
        }
        if hovered.is_some() && self.led.drag.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        if self.led.show_wiring || self.led.wiring.is_some() {
            self.draw_wiring(&painter, &view);
        }

        // Dessin d'un tracé libre : la souris pose des points au sol ou sur la face visée.
        if drawing {
            self.draw_path(ui, &resp, &painter, &view, rect, hovered_quad);
            return;
        }

        // Cône des vidéoprojecteurs allumés (pointillés), jusqu'à 6 m.
        for s in self.project.leds.surfaces.iter().filter(|s| s.kind == leds::SurfaceKind::Projector && s.show_output) {
            let o = self.out_size(s.output.min(self.project.outputs.len().max(1) - 1));
            let aspect = (o.x / o.y.max(1.0)).clamp(0.2, 5.0);
            let l = 6.0;
            let (hx, hy) = (l / (2.0 * s.throw.max(0.2)), l / (2.0 * s.throw.max(0.2) * aspect));
            let lens = s.lens();
            let z = -s.size[2] * 0.5 - 0.04 - l;
            let corners: Vec<[f32; 3]> = [[-hx, hy], [hx, hy], [hx, -hy], [-hx, -hy]]
                .iter()
                .map(|c| leds::place(s.pos, s.rot, [c[0], s.size[1] * 0.5 + c[1], z]))
                .collect();
            let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 200, 255, 90));
            let px: Vec<Option<egui::Pos2>> = corners.iter().map(|c| view.project(*c).map(|p| p.0)).collect();
            if let Some((a, _)) = view.project(lens) {
                for p in px.iter().flatten() {
                    painter.extend(egui::Shape::dashed_line(&[a, *p], stroke, 5.0, 4.0));
                }
            }
            for k in 0..4 {
                if let (Some(a), Some(b)) = (px[k], px[(k + 1) % 4]) {
                    painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 5.0, 4.0));
                }
            }
        }

        // Point d'ajout.
        if let Some(c) = self.led.cursor3d {
            if let Some((p, _)) = view.project(c) {
                painter.circle_stroke(p, 9.0, Stroke::new(2.0, WARN));
                painter.line_segment([p - Vec2::new(14.0, 0.0), p + Vec2::new(14.0, 0.0)], Stroke::new(1.5, WARN));
                painter.line_segment([p - Vec2::new(0.0, 14.0), p + Vec2::new(0.0, 14.0)], Stroke::new(1.5, WARN));
                // Repère vertical jusqu'au sol, pour lire la hauteur.
                if c[1] > 0.01 {
                    view.line(&painter, c, [c[0], 0.0, c[2]], Stroke::new(1.0, WARN.gamma_multiply(0.5)));
                }
            }
        }

        // Infos de la LED survolée.
        // (Liste des LEDs d'une image en retard sur la scène — un bandeau vient d'être supprimé,
        // depuis l'iPad par exemple : pas d'infos plutôt qu'un plantage.)
        if let (Some((i, _)), Some(h)) = (hovered_led, hover)
            && let Some(l) = self.led.leds.get(i)
            && let Some(f) = self.project.leds.fixtures.get(l.fixture)
        {
            let ctrl = l.controller.and_then(|c| self.project.leds.controllers.get(c));
            let addr = ctrl.map_or("non branchée".to_owned(), |c| format!("{} · adresse {}", c.name, l.address));
            let c = self.led.colors.get(i).copied().unwrap_or_default();
            let text = format!("{} · LED {}\n{addr}\nRVB {} {} {}", f.name, l.index + 1, c[0], c[1], c[2]);
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.5), TEXT);
            let pos = h + Vec2::new(14.0, 10.0);
            painter.rect_filled(Rect::from_min_size(pos, galley.size()).expand(5.0), 5.0, theme::CARD);
            painter.galley(pos, galley, TEXT);
        }

        // Vues toutes prêtes et aide.
        let buttons = Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::new(rect.width() - 16.0, 24.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(buttons), |ui| {
            ui.horizontal(|ui| {
                for (name, yaw, pitch) in [("Face", 0.0, 0.12), ("Dessus", 0.0, 1.55), ("Côté", -1.5708, 0.12), ("3D", 0.5, 0.42)] {
                    if ui.small_button(name).clicked() {
                        self.frame_all(Some((yaw, pitch)));
                    }
                }
                if ui.small_button(with_icon(ic::CORNERS_OUT, "Tout voir")).on_hover_text("Cadrer toute la scène (double-clic dans le vide)").clicked() {
                    self.frame_all(None);
                }
                if self.led.cursor3d.is_some() {
                    ui.separator();
                    ui.label(egui::RichText::new(with_icon(ic::CROSSHAIR, "Point d'ajout posé")).small().color(WARN))
                        .on_hover_text("Les éléments ajoutés apparaissent ici. Clic droit ailleurs : le déplacer.");
                    if ui.small_button(ic::X).on_hover_text("Retirer le point d'ajout").clicked() {
                        self.led.cursor3d = None;
                    }
                }
            });
        });
        // Aide en bas de la vue : retour à la ligne plutôt que coupée, sur un fond qui la détache de la scène.
        let help = painter.layout(
            "ZQSD : drone (Espace / C : monter, descendre ; Maj : vite) · flèches : déplacer la vue · glisser : tourner · clic droit glissé : déplacer · molette : zoom · double-clic : centrer (vide : tout voir) · clic droit : point d'ajout · Maj + glisser : sélection".into(),
            egui::FontId::proportional(11.0),
            TEXT_WEAK,
            rect.width() - 16.0,
        );
        let at = rect.left_bottom() + Vec2::new(8.0, -6.0 - help.size().y);
        painter.rect_filled(Rect::from_min_size(at, help.size()).expand(4.0), 4.0, Color32::from_black_alpha(170));
        painter.galley(at, help, TEXT_WEAK);
        if self.project.leds.fixtures.is_empty() && self.project.leds.surfaces.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Posez un écran, du décor (onglet Scène) ou des LEDs (onglet LEDs)",
                egui::FontId::proportional(14.0),
                TEXT_WEAK,
            );
        }
    }

    /// Drone : ZQSD (ou WASD) pour avancer, reculer, se décaler, Espace / C pour monter et
    /// descendre, tant que la souris est sur la vue 3D. Ces touches ne lancent alors pas leurs
    /// raccourcis habituels. Appelé avant les raccourcis.
    pub(super) fn drone_keys(&mut self, ctx: &egui::Context) {
        let hover = std::mem::take(&mut self.led.hover_3d);
        if !hover || ctx.egui_wants_keyboard_input() || ctx.input(|i| i.modifiers.command) {
            return;
        }
        // Flèches : déplacer la vue (haut, bas, gauche, droite), quand aucun objet n'est choisi
        // (sinon elles déplacent l'objet).
        let nothing_selected = self.current_target().is_none() && self.multi_count() < 2;
        let (fwd, side, up, dt) = ctx.input_mut(|i| {
            let down = |k: Key| i.key_down(k);
            let arrows = nothing_selected;
            let f = (down(Key::Z) || down(Key::W)) as i32 - down(Key::S) as i32;
            let r = (down(Key::D) || arrows && down(Key::ArrowRight)) as i32 - (down(Key::Q) || down(Key::A) || arrows && down(Key::ArrowLeft)) as i32;
            let u = (down(Key::Space) || arrows && down(Key::ArrowUp)) as i32 - (down(Key::C) || arrows && down(Key::ArrowDown)) as i32;
            if arrows {
                for k in [Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight] {
                    i.consume_key(Modifiers::NONE, k);
                    i.consume_key(Modifiers::SHIFT, k);
                }
            }
            // Les appuis ne vont pas aux raccourcis de l'app.
            for k in [Key::Z, Key::W, Key::S, Key::D, Key::Q, Key::A, Key::Space, Key::C] {
                i.consume_key(Modifiers::NONE, k);
                i.consume_key(Modifiers::SHIFT, k);
            }
            (f as f32, r as f32, u as f32, i.stable_dt.min(0.1))
        });
        if fwd == 0.0 && side == 0.0 && up == 0.0 {
            return;
        }
        let room = self.project.leds.room;
        // Maj : plus vite.
        let speed = if ctx.input(|i| i.modifiers.shift) { 6.0 } else { 2.5 };
        self.led.cam_goal = None;
        self.led.drone = true;
        let cam = self.led.camera.get_or_insert_with(|| Orbit::home(room));
        // Le regard s'approche de l'œil : on vole, on ne tourne plus autour de la scène.
        cam.dist = cam.dist.min(2.0);
        let (f, r) = ([-cam.yaw.sin(), 0.0, cam.yaw.cos()], [cam.yaw.cos(), 0.0, cam.yaw.sin()]);
        for k in 0..3 {
            cam.target[k] += (f[k] * fwd + r[k] * side + if k == 1 { up } else { 0.0 }) * speed * dt;
        }
        cam.target[1] = cam.target[1].max(-1.0);
        ctx.request_repaint();
    }

    /// Raccourcis de la scène 3D (onglet LEDs ouvert). Appelé avant les raccourcis généraux :
    /// les touches utilisées ici ne lancent pas aussi leur action habituelle.
    pub(super) fn scene_shortcuts(&mut self, ctx: &egui::Context) {
        if !matches!(self.right_tab, RightTab::Leds | RightTab::Dmx) || ctx.egui_wants_keyboard_input() || self.led.confirm_push.is_some() {
            return;
        }
        if self.led.wiring.is_some() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.led.wiring = None;
            return;
        }
        // Pendant un tracé, Entrée, Retour arrière et Échap appartiennent au dessin.
        if self.led.drawing.is_some() || (self.shown_view() == LedView::Plan && self.led.sketch.drawing) {
            return;
        }
        let key = |mods: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(mods, k));
        if key(Modifiers::NONE, Key::D) {
            self.start_drawing(None);
            return;
        }
        // Vues toutes prêtes : V passe de l'une à l'autre.
        if key(Modifiers::NONE, Key::V) {
            let room = self.project.leds.room;
            let views = [(0.5, 0.42), (0.0, 0.12), (0.0, 1.55), (-1.5708, 0.12)];
            let cam = self.led.camera.get_or_insert_with(|| Orbit::home(room));
            let cur = views.iter().position(|v| (v.0 - cam.yaw).abs() < 0.01 && (v.1 - cam.pitch).abs() < 0.01);
            let (yaw, pitch) = views[cur.map_or(0, |c| (c + 1) % views.len())];
            let _ = cam;
            self.frame_all(Some((yaw, pitch)));
        }
        // Plusieurs objets choisis (rectangle, Cmd+clic) : les touches agissent sur tous.
        let multi = self.multi_count() > 1;
        let target = match self.current_target() {
            Some(t) => t,
            None if multi => Target::Fixture(usize::MAX),
            None => return,
        };
        if key(Modifiers::NONE, Key::Escape) {
            self.led.selected = None;
            self.led.selected_surface = None;
            self.led.selected_dmx = None;
            self.led.selected_set = None;
            self.clear_multi();
            return;
        }
        if key(Modifiers::NONE, Key::Delete) || key(Modifiers::NONE, Key::Backspace) {
            if multi {
                self.delete_multi();
                return;
            }
            if let Some(id) = self.led.selected_set.take() {
                // Tout le groupe disparaît.
                let setup = &mut self.project.leds;
                setup.fixtures.retain(|f| f.set != id);
                let gone: Vec<u32> = setup.surfaces.iter().filter(|s| s.set == id).map(|s| s.id).collect();
                setup.surfaces.retain(|s| s.set != id);
                if setup.image_on.is_some_and(|i| gone.contains(&i)) {
                    setup.image_on = None;
                }
                setup.prune_sets();
                self.led.selected = None;
                self.led.selected_surface = None;
                return;
            }
            let setup = &mut self.project.leds;
            match target {
                Target::Fixture(i) if i < setup.fixtures.len() => {
                    setup.fixtures.remove(i);
                }
                Target::Surface(i) if i < setup.surfaces.len() => {
                    let id = setup.surfaces.remove(i).id;
                    if setup.image_on == Some(id) {
                        setup.image_on = None;
                    }
                }
                Target::Dmx(i) if i < setup.dmx.len() => {
                    setup.dmx.remove(i);
                }
                Target::Laser(i) if i < setup.lasers.len() => {
                    setup.lasers.remove(i);
                }
                _ => {}
            }
            self.led.selected = None;
            self.led.selected_surface = None;
            self.led.selected_dmx = None;
            return;
        }
        if !multi && key(Modifiers::COMMAND, Key::D) {
            let setup = &mut self.project.leds;
            match target {
                Target::Fixture(i) if i < setup.fixtures.len() => {
                    let mut f = setup.fixtures[i].clone();
                    f.id = setup.next_fixture_id();
                    f.name = format!("{} (copie)", f.name);
                    f.pos[0] += 0.5;
                    setup.fixtures.insert(i + 1, f);
                    self.select(Target::Fixture(i + 1));
                }
                Target::Surface(i) if i < setup.surfaces.len() => {
                    let mut o = setup.surfaces[i].clone();
                    o.id = setup.next_surface_id();
                    o.name = format!("{} (copie)", o.name);
                    o.pos[0] += o.size[0] + 0.2;
                    setup.surfaces.insert(i + 1, o);
                    self.select(Target::Surface(i + 1));
                }
                _ => {}
            }
            return;
        }
        // Flèches : déplacer au sol (5 cm, Maj : 50 cm), dans le sens de la vue ;
        // Page haut/bas : monter, descendre ; [ et ] (ou Q et E) : tourner de 15°.
        let fine = ctx.input(|i| i.modifiers.shift);
        let step = if fine { 0.5 } else { 0.05 };
        let mods = if fine { Modifiers::SHIFT } else { Modifiers::NONE };
        let yaw = self.led.camera.map_or(0.0, |c| c.yaw);
        // Axes de la salle les plus proches de « vers la droite » et « vers le fond » à l'écran.
        let axis = |x: f32, z: f32| if x.abs() >= z.abs() { [x.signum(), 0.0] } else { [0.0, z.signum()] };
        let right = axis(yaw.cos(), yaw.sin());
        let fwd = axis(-yaw.sin(), yaw.cos());
        let mut mv = [0.0f32; 3];
        let mut turn = 0.0;
        for (k, dir) in [
            (Key::ArrowRight, [right[0], 0.0, right[1]]),
            (Key::ArrowLeft, [-right[0], 0.0, -right[1]]),
            // Haut / bas : en hauteur ; PgHaut / PgBas : en profondeur (s'éloigner, se rapprocher).
            (Key::ArrowUp, [0.0, 1.0, 0.0]),
            (Key::ArrowDown, [0.0, -1.0, 0.0]),
            (Key::PageUp, [fwd[0], 0.0, fwd[1]]),
            (Key::PageDown, [-fwd[0], 0.0, -fwd[1]]),
        ] {
            if key(mods, k) {
                for i in 0..3 {
                    mv[i] += dir[i] * step;
                }
            }
        }
        if key(Modifiers::NONE, Key::CloseBracket) || key(Modifiers::NONE, Key::E) {
            turn += 15.0;
        }
        if key(Modifiers::NONE, Key::OpenBracket) || key(Modifiers::NONE, Key::Q) {
            turn -= 15.0;
        }
        if mv != [0.0; 3] || turn != 0.0 {
            if multi {
                // Les objets choisis bougent ensemble, comme un groupe provisoire.
                self.with_multi(|l, id| {
                    l.move_set(id, mv);
                    if turn != 0.0 {
                        l.rotate_set(id, turn);
                    }
                });
            } else if let Some(id) = self.led.selected_set {
                // Tout le groupe.
                self.project.leds.move_set(id, mv);
                if turn != 0.0 {
                    self.project.leds.rotate_set(id, turn);
                }
            } else if let Some((pos, rot)) = self.target_mut(target) {
                for i in 0..3 {
                    pos[i] = ((pos[i] + mv[i]) * 100.0).round() / 100.0;
                }
                pos[1] = pos[1].max(0.0);
                rot[0] = (rot[0] + turn).rem_euclid(360.0);
            }
        }
    }

    /// Générateur de tunnel d'arches LED.
    fn tunnel_ui(&mut self, ui: &mut egui::Ui) {
        let mut create = false;
        egui::CollapsingHeader::new(egui::RichText::new(with_icon(ic::SPIRAL, "Tunnel d'arches")).strong())
            .id_salt("led_tunnel")
            .show(ui, |ui| {
                let t = &mut self.led.tunnel;
                ui.horizontal(|ui| {
                    ui.label("Arches");
                    ui.add(egui::DragValue::new(&mut t.arches).range(1..=50));
                    ui.label("tous les");
                    ui.add(egui::DragValue::new(&mut t.spacing).range(0.1..=20.0).speed(0.01).suffix(" m"));
                    ui.label("dès");
                    ui.add(egui::DragValue::new(&mut t.start).range(0.0..=100.0).speed(0.01).suffix(" m"))
                        .on_hover_text("Profondeur de la première arche");
                });
                ui.horizontal(|ui| {
                    ui.label("Largeur");
                    ui.add(egui::DragValue::new(&mut t.width).range(0.3..=30.0).speed(0.01).suffix(" m"));
                    ui.label("Murs");
                    ui.add(egui::DragValue::new(&mut t.wall).range(0.0..=20.0).speed(0.01).suffix(" m"));
                    ui.label("Sommet");
                    ui.add(egui::DragValue::new(&mut t.height).range(0.2..=20.0).speed(0.01).suffix(" m"));
                });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut t.round, false, "Voûte à pans");
                    ui.selectable_value(&mut t.round, true, "Voûte arrondie");
                    ui.checkbox(&mut t.floor, "Bandes au sol");
                });
                ui.horizontal(|ui| {
                    ui.label("Densité");
                    for d in [30.0, 60.0, 96.0, 144.0] {
                        ui.selectable_value(&mut t.density, d, format!("{d:.0}/m"));
                    }
                });
                create = ui
                    .add(theme::primary_button(with_icon(ic::PLUS, "Créer le tunnel")))
                    .on_hover_text("Remplace le tunnel déjà créé. Groupes de bandeaux de l'avant vers le fond : Sortie › Scénario « Allumage » ou « Vague » font courir la lumière dans le tunnel.")
                    .clicked();
            });
        if !create {
            return;
        }
        let paths = self.led.tunnel.paths();
        let density = self.led.tunnel.density;
        let setup = &mut self.project.leds;
        const PREFIX: &str = "Tunnel · ";
        setup.fixtures.retain(|f| !f.name.starts_with(PREFIX));
        let (ctrl, outputs) = self
            .led
            .selected
            .and_then(|s| setup.fixtures.get(s))
            .and_then(|f| setup.controllers.iter().find(|c| c.id == f.controller))
            .or(setup.controllers.first())
            .map_or((0, 1), |c| (c.id, c.pins.len().max(1)));
        let set = setup.next_set_id();
        setup.sets.push(leds::LedGroup { id: set, name: "Tunnel".into(), effect: None });
        for (k, (name, pts)) in paths.iter().enumerate() {
            let id = setup.next_fixture_id();
            let mut f = Fixture::new(FixtureKind::Path, id, format!("{PREFIX}{name}"));
            f.set_path(pts, false);
            f.count = ((f.path_length() * density).round() as u32).clamp(2, leds::MAX_COUNT);
            f.controller = ctrl;
            f.output = k % outputs;
            f.set = set;
            setup.fixtures.push(f);
        }
        setup.prune_sets();
        // L'effet tunnel : les groupes vont de l'avant vers le fond.
        setup.auto_groups(leds::GroupOrder::FrontBack);
        self.led.selected_set = Some(set).filter(|id| setup.sets.iter().any(|g| g.id == *id));
        if self.shown_view() == LedView::Output {
            self.led.view = Some(LedView::Both);
        }
    }

    /// Ordre des LEDs dans la vue 3D : numéro et sortie au début de chaque bandeau, flèche du sens,
    /// pointillés du fil qui relie la fin d'un bandeau au début du suivant.
    fn draw_wiring(&self, painter: &egui::Painter, view: &View) {
        let setup = &self.project.leds;
        // Première, deuxième (pour le sens) et dernière LED de chaque élément.
        let mut ends: HashMap<usize, ([f32; 3], [f32; 3], [f32; 3])> = HashMap::new();
        for l in &self.led.leds {
            let n = setup.fixtures.get(l.fixture).map_or(1, Fixture::led_count);
            let e = ends.entry(l.fixture).or_insert((l.pos, l.pos, l.pos));
            if l.index == 0 {
                e.0 = l.pos;
            }
            if l.index == (n / 8).clamp(1, 6).min(n - 1) {
                e.1 = l.pos;
            }
            if l.index + 1 == n {
                e.2 = l.pos;
            }
        }
        let font = egui::FontId::proportional(11.0);
        for (ci, c) in setup.controllers.iter().enumerate() {
            let outputs = c.pins.len().max(1);
            let mut prev: HashMap<usize, (usize, usize)> = HashMap::new();
            for (fi, start, _) in setup.layout(ci).fixtures {
                let o = setup.fixtures[fi].output.min(outputs - 1);
                let rank = prev.get(&o).map_or(1, |p| p.1 + 1);
                let Some(&(first, second, last)) = ends.get(&fi) else { continue };
                // Fil depuis le bandeau précédent de la même sortie.
                if let Some(&(pf, _)) = prev.get(&o) {
                    if let (Some(a), Some(b)) = (ends.get(&pf).and_then(|e| view.project(e.2)), view.project(first)) {
                        painter.extend(egui::Shape::dashed_line(&[a.0, b.0], Stroke::new(1.5, WARN), 6.0, 4.0));
                    }
                }
                prev.insert(o, (fi, rank));
                if let (Some(a), Some(b)) = (view.project(first), view.project(second)) {
                    // Flèche du sens.
                    let dir = (b.0 - a.0).normalized();
                    let tip = a.0 + dir * 22.0;
                    painter.line_segment([a.0, tip], Stroke::new(2.0, TEXT));
                    let side = egui::vec2(-dir.y, dir.x) * 4.0;
                    painter.add(egui::Shape::convex_polygon(vec![tip + dir * 6.0, tip + side, tip - side], TEXT, Stroke::NONE));
                    painter.circle_filled(a.0, 4.0, TEXT);
                    let text = if setup.controllers.len() > 1 {
                        format!("{} S{}·{}  #{}", c.name, o + 1, rank, start)
                    } else {
                        format!("S{}·{}  #{}", o + 1, rank, start)
                    };
                    let galley = painter.layout_no_wrap(text, font.clone(), BG);
                    let r = Rect::from_min_size(a.0 + egui::vec2(6.0, 4.0), galley.size()).expand2(egui::vec2(3.0, 1.0));
                    painter.rect_filled(r, 3.0, WARN);
                    painter.galley(r.min + egui::vec2(3.0, 1.0), galley, BG);
                }
                let _ = last;
            }
        }
        // Éléments non branchés : leur sens seulement.
        for (fi, f) in setup.fixtures.iter().enumerate() {
            if setup.controller_index(f.controller).is_some() {
                continue;
            }
            if let Some(&(first, _, _)) = ends.get(&fi) {
                if let Some(a) = view.project(first) {
                    painter.circle_stroke(a.0, 4.0, Stroke::new(1.5, TEXT_WEAK));
                    painter.text(a.0 + egui::vec2(6.0, 4.0), egui::Align2::LEFT_TOP, "non branché", font.clone(), TEXT_WEAK);
                }
            }
        }
    }

    /// Clic sur un bandeau en mode câblage : il suit le précédent sur la sortie choisie, et son
    /// signal arrive par le bout le plus proche du clic.
    fn wire_click(&mut self, led: usize) {
        let Some(wm) = &mut self.led.wiring else { return };
        let Some(l) = self.led.leds.get(led) else { return };
        let fi = l.fixture;
        let setup = &mut self.project.leds;
        let Some(f) = setup.fixtures.get_mut(fi) else { return };
        if l.index >= f.led_count() / 2 {
            f.reverse = !f.reverse;
        }
        f.controller = wm.controller;
        f.output = wm.output;
        let id = f.id;
        if !wm.chain.contains(&id) {
            let item = setup.fixtures.remove(fi);
            // Juste après le bandeau câblé précédent, sinon avant tous ceux de la même sortie.
            let at = match wm.chain.last().and_then(|p| setup.fixtures.iter().position(|f| f.id == *p)) {
                Some(p) => p + 1,
                None => setup
                    .fixtures
                    .iter()
                    .position(|f| f.controller == wm.controller && f.output == wm.output)
                    .unwrap_or(setup.fixtures.len()),
            };
            setup.fixtures.insert(at, item);
            wm.chain.push(id);
        }
        self.led.selected = setup.fixtures.iter().position(|f| f.id == id);
        self.led.selected_surface = None;
        self.led.selected_set = None;
    }

    /// Ordre des LEDs : affichage du câblage et câblage à la souris.
    fn wiring_ui(&mut self, ui: &mut egui::Ui) {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(with_icon(ic::LIST, "Ordre des LEDs")).strong());
                ui.checkbox(&mut self.led.show_wiring, "Voir le câblage")
                    .on_hover_text("Dans la vue 3D : début et sens de chaque bandeau, ordre sur sa sortie, fil qui relie les bandeaux");
            });
            let setup = &self.project.leds;
            if let Some(wm) = &mut self.led.wiring {
                ui.label(
                    egui::RichText::new(format!(
                        "Cliquez les bandeaux dans l'ordre du fil, près du bout par où arrive le signal. {} câblé(s).",
                        wm.chain.len()
                    ))
                    .small()
                    .color(WARN),
                );
                if ui.add(theme::primary_button("Terminer le câblage")).clicked() {
                    self.led.wiring = None;
                }
                return;
            }
            let mut start = None;
            let outputs: Vec<((u32, usize), String)> = setup
                .controllers
                .iter()
                .flat_map(|c| {
                    (0..c.pins.len().max(1)).map(move |o| {
                        let pin = c.pins.get(o).copied().flatten().map_or(String::new(), |p| format!(" (pin {p})"));
                        ((c.id, o), format!("{} · sortie {}{pin}", c.name, o + 1))
                    })
                })
                .collect();
            let chosen = &mut self.led.wire_target;
            if !outputs.iter().any(|o| o.0 == *chosen) {
                *chosen = outputs.first().map_or((0, 0), |o| o.0);
            }
            ui.horizontal(|ui| {
                let label = outputs.iter().find(|o| o.0 == *chosen).map_or(String::new(), |o| o.1.clone());
                egui::ComboBox::from_id_salt("wire_output").selected_text(label).width(190.0).show_ui(ui, |ui| {
                    for (key, name) in &outputs {
                        ui.selectable_value(chosen, *key, name);
                    }
                });
                if ui.button(with_icon(ic::PENCIL_SIMPLE, "Câbler à la souris")).clicked() {
                    start = Some(*chosen);
                }
            });
            ui.label(
                egui::RichText::new("Choisissez la sortie, puis cliquez les bandeaux dans l'ordre du fil (Échap : terminer). Dans la liste, les boutons monter, descendre et inverser font la même chose à la main.")
                    .small()
                    .color(TEXT_WEAK),
            );
            if let Some((controller, output)) = start {
                self.led.wiring = Some(WireMode { controller, output, chain: Vec::new() });
                self.led.show_wiring = true;
                if self.shown_view() == LedView::Output {
                    self.led.view = Some(LedView::Both);
                }
            }
        });
        ui.add_space(4.0);
    }

    /// Ouvre la planche 2D : nouvelle forme (`None`) ou forme d'un tracé plat de la scène.
    fn open_sketch(&mut self, target: Option<usize>) {
        self.led.view = Some(LedView::Plan);
        self.led.sketch_grab = None;
        match target.and_then(|i| self.project.leds.fixtures.get(i)).filter(|f| f.is_flat()) {
            Some(f) => {
                let len = f.path_length();
                self.led.sketch = Sketch {
                    points: f.points.iter().map(|p| [p[0], p[1]]).collect(),
                    closed: f.closed,
                    editing: Some(f.id),
                    drawing: false,
                    density: if len > 0.01 { f.count as f32 / len } else { 60.0 },
                };
                self.frame_sketch();
            }
            None => {
                let density = self.led.sketch.density;
                self.led.sketch = Sketch { density, ..Sketch::default() };
                self.led.sketch_view = ([0.0, 1.0], self.led.sketch_view.1);
            }
        }
    }

    /// Cadre la vue sur la forme.
    fn frame_sketch(&mut self) {
        let pts = &self.led.sketch.points;
        if pts.is_empty() {
            return;
        }
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in pts {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let size = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(0.5);
        self.led.sketch_view = ([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0], (500.0 / size).clamp(20.0, 2000.0));
    }

    /// Planche à dessin 2D : on dessine la forme d'un bandeau en mètres, puis on l'ajoute à la
    /// scène (où on la place et l'oriente) ; ou on modifie la forme d'un tracé plat de la scène.
    fn led_plan(&mut self, ui: &mut egui::Ui, rect: Rect) {
        // Tracé modifié supprimé entre-temps : la planche redevient une nouvelle forme.
        if let Some(id) = self.led.sketch.editing {
            if !self.project.leds.fixtures.iter().any(|f| f.id == id) {
                self.led.sketch.editing = None;
            }
        }
        let editing_name = self.led.sketch.editing.and_then(|id| self.project.leds.fixtures.iter().find(|f| f.id == id)).map(|f| f.name.clone());
        let len = sketch_length(&self.led.sketch.points, self.led.sketch.closed);
        let count = ((len * self.led.sketch.density).round() as u32).clamp(2, leds::MAX_COUNT);

        // Outils (1re ligne) et forme (2e ligne).
        let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 56.0));
        let mut pick_photo = false;
        let mut add = false;
        let mut new_shape = false;
        let mut done = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            ui.horizontal(|ui| {
                if ui.button(with_icon(ic::PLUS, "Nouvelle forme")).clicked() {
                    new_shape = true;
                }
                let sk = &mut self.led.sketch;
                if ui.selectable_label(!sk.drawing, with_icon(ic::CROSSHAIR, "Modifier")).clicked() {
                    sk.drawing = false;
                }
                if ui.selectable_label(sk.drawing, with_icon(ic::PENCIL_SIMPLE, "Tracer")).clicked() {
                    sk.drawing = true;
                }
                ui.separator();
                let setup = &mut self.project.leds;
                let name = setup.plan_photo.as_deref().map_or("Photo de référence…".to_owned(), |p| elide_name(&file_name(p)));
                if ui.selectable_label(setup.plan_photo.is_some(), name).on_hover_text("Photo de l'élément réel, pour décalquer sa forme").clicked() {
                    pick_photo = true;
                }
                if setup.plan_photo.is_some() {
                    ui.add(egui::DragValue::new(&mut setup.plan_photo_w).range(0.1..=100.0).speed(0.01).prefix("largeur ").suffix(" m"))
                        .on_hover_text("Largeur réelle de ce que montre la photo : elle se met à l'échelle de la planche");
                    ui.add(egui::Slider::new(&mut setup.plan_bg, 0.0..=1.0).show_value(false)).on_hover_text("Opacité de la photo");
                    if ui.small_button(ic::X).on_hover_text("Retirer la photo").clicked() {
                        setup.plan_photo = None;
                    }
                }
            });
            ui.horizontal(|ui| {
                let sk = &mut self.led.sketch;
                ui.label(egui::RichText::new(format!("{len:.2} m · {count} LEDs")).strong());
                for d in [30.0, 60.0, 96.0, 144.0] {
                    ui.selectable_value(&mut sk.density, d, format!("{d:.0}/m"));
                }
                ui.checkbox(&mut sk.closed, "Boucle fermée");
                ui.separator();
                match &editing_name {
                    Some(name) => {
                        ui.label(egui::RichText::new(format!("Modifie « {name} » en direct")).color(ACCENT));
                        if ui.button("Terminé").clicked() {
                            done = true;
                        }
                    }
                    None => {
                        add = ui
                            .add_enabled(sk.points.len() >= 2, theme::primary_button(with_icon(ic::PLUS, "Ajouter à la scène")))
                            .on_hover_text("Le bandeau apparaît debout au milieu de la salle : placez-le et tournez-le dans la vue 3D")
                            .clicked();
                    }
                }
            });
        });
        if pick_photo {
            if let Some(p) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"]).pick_file() {
                self.project.leds.plan_photo = Some(p);
            }
        }
        if new_shape {
            self.open_sketch(None);
        }

        // Planche : quadrillage en mètres, photo à l'échelle, forme.
        let canvas = Rect::from_min_max(egui::pos2(rect.min.x, bar.max.y + 4.0), egui::pos2(rect.max.x, rect.max.y - 18.0));
        let resp = ui.interact(canvas, ui.scope_id().with("led_sketch"), Sense::click_and_drag());
        let (mut center, mut scale) = self.led.sketch_view;
        // Clic droit ou molette enfoncée : déplacer la vue ; molette : zoom sous le pointeur.
        if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
            let d = resp.drag_delta();
            center = [center[0] - d.x / scale, center[1] + d.y / scale];
        }
        if let (true, Some(h)) = (resp.hovered(), resp.hover_pos()) {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let before = [center[0] + (h.x - canvas.center().x) / scale, center[1] - (h.y - canvas.center().y) / scale];
                scale = (scale * (scroll * 0.002).exp()).clamp(10.0, 4000.0);
                center = [before[0] - (h.x - canvas.center().x) / scale, before[1] + (h.y - canvas.center().y) / scale];
            }
        }
        self.led.sketch_view = (center, scale);
        let to_px = |p: [f32; 2]| canvas.center() + Vec2::new((p[0] - center[0]) * scale, -(p[1] - center[1]) * scale);
        let to_m = |q: egui::Pos2| [center[0] + (q.x - canvas.center().x) / scale, center[1] - (q.y - canvas.center().y) / scale];
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 6.0, Color32::from_rgb(14, 16, 20));
        if let Some(path) = self.project.leds.plan_photo.clone() {
            if self.led.plan_tex.as_ref().is_none_or(|t| t.0 != path) {
                self.led.plan_tex = load_photo(ui.ctx(), &path).map(|t| (path.clone(), t));
                if self.led.plan_tex.is_none() {
                    self.status = Some(format!("Photo illisible : {}", path.display()));
                    self.project.leds.plan_photo = None;
                }
            }
            if let Some((_, tex)) = &self.led.plan_tex {
                // Photo centrée sur l'origine, posée au sol (y = 0 en bas).
                let w = self.project.leds.plan_photo_w;
                let [tw, th] = tex.size().map(|v| v as f32);
                let h = w * th / tw.max(1.0);
                let r = Rect::from_two_pos(to_px([-w / 2.0, h]), to_px([w / 2.0, 0.0]));
                let a = (self.project.leds.plan_bg * 255.0) as u8;
                painter.image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::from_white_alpha(a));
            }
        }
        // Quadrillage : 10 cm (si assez grand), 1 m, axes.
        let (lo, hi) = (to_m(canvas.left_bottom()), to_m(canvas.right_top()));
        for (step, alpha) in [(0.1f32, 10u8), (1.0, 28)] {
            if step * scale < 8.0 {
                continue;
            }
            let mut x = (lo[0] / step).floor() * step;
            while x <= hi[0] {
                painter.vline(to_px([x, 0.0]).x, canvas.y_range(), Stroke::new(1.0, Color32::from_white_alpha(alpha)));
                x += step;
            }
            let mut y = (lo[1] / step).floor() * step;
            while y <= hi[1] {
                painter.hline(canvas.x_range(), to_px([0.0, y]).y, Stroke::new(1.0, Color32::from_white_alpha(alpha)));
                y += step;
            }
        }
        painter.hline(canvas.x_range(), to_px([0.0, 0.0]).y, Stroke::new(1.0, Color32::from_white_alpha(60)));
        painter.vline(to_px([0.0, 0.0]).x, canvas.y_range(), Stroke::new(1.0, Color32::from_white_alpha(40)));
        painter.text(egui::pos2(canvas.min.x + 4.0, to_px([0.0, 0.0]).y - 2.0), egui::Align2::LEFT_BOTTOM, "sol", egui::FontId::proportional(10.0), TEXT_WEAK);
        let mut m = (lo[0]).ceil();
        while m <= hi[0] && scale >= 25.0 {
            painter.text(egui::pos2(to_px([m, 0.0]).x + 2.0, canvas.max.y - 2.0), egui::Align2::LEFT_BOTTOM, format!("{m:.0} m"), egui::FontId::proportional(10.0), TEXT_WEAK);
            m += 1.0;
        }

        // La forme, ses points et l'aperçu de ses LEDs.
        let sk = &self.led.sketch;
        let mut pts_px: Vec<egui::Pos2> = sk.points.iter().map(|&p| to_px(p)).collect();
        if sk.closed && pts_px.len() > 2 {
            pts_px.push(pts_px[0]);
        }
        painter.add(egui::Shape::line(pts_px.clone(), Stroke::new(2.0, ACCENT)));
        if sk.points.len() >= 2 {
            let preview = Fixture {
                kind: FixtureKind::Path,
                points: sk.points.iter().map(|p| [p[0], p[1], 0.0]).collect(),
                closed: sk.closed,
                count,
                pos: [0.0; 3],
                ..Fixture::default()
            };
            let r = (0.012 * scale).clamp(1.2, 4.0);
            for (k, p) in preview.positions().into_iter().enumerate() {
                let q = to_px([p[0], p[1]]);
                painter.circle_filled(q, r, if k == 0 { Color32::WHITE } else { Color32::from_rgb(255, 170, 60) });
            }
        }
        let hover = resp.hover_pos();
        let mut on_point = None;
        for (k, &p) in sk.points.iter().enumerate() {
            let q = to_px(p);
            let rr = Rect::from_center_size(q, Vec2::splat(9.0));
            painter.rect_filled(rr, 2.0, TEXT);
            painter.rect_stroke(rr, 2.0, Stroke::new(1.0, BG), egui::StrokeKind::Inside);
            if hover.is_some_and(|h| h.distance(q) < 8.0) {
                on_point = Some(k);
            }
        }
        let on_line = hover.is_some_and(|h| pts_px.windows(2).any(|w| seg_dist(h, w[0], w[1]) < 7.0));
        // Magnétisme : tous les 5 cm (Alt : libre).
        let alt = ui.input(|i| i.modifiers.alt);
        let snap = |p: [f32; 2]| if alt { p } else { p.map(|v| (v / 0.05).round() * 0.05) };

        let ctx = ui.ctx().clone();
        if self.led.sketch.drawing {
            let (enter, escape, back, shift) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                    i.consume_key(Modifiers::NONE, Key::Backspace),
                    i.modifiers.shift,
                )
            });
            let sk = &mut self.led.sketch;
            if back {
                sk.points.pop();
            }
            let first = sk.points.first().map(|&p| to_px(p));
            let mut close = false;
            let target = hover.map(|h| {
                if sk.points.len() > 2 && first.is_some_and(|f| f.distance(h) < 10.0) {
                    close = true;
                    return sk.points[0];
                }
                let mut p = snap(to_m(h));
                // Maj : horizontale, verticale ou 45° depuis le point précédent.
                if let (true, Some(&last)) = (shift, sk.points.last()) {
                    let (dx, dy) = (p[0] - last[0], p[1] - last[1]);
                    let a = (dy.atan2(dx) / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
                    let l = (dx * dx + dy * dy).sqrt();
                    p = [last[0] + a.cos() * l, last[1] + a.sin() * l];
                }
                p
            });
            if let (Some(&last), Some(t)) = (sk.points.last(), target) {
                painter.line_segment([to_px(last), to_px(t)], Stroke::new(1.5, ACCENT.gamma_multiply(0.6)));
                let l = ((t[0] - last[0]).powi(2) + (t[1] - last[1]).powi(2)).sqrt();
                painter.text(to_px(t) + Vec2::new(10.0, -10.0), egui::Align2::LEFT_BOTTOM, format!("{l:.2} m"), egui::FontId::proportional(11.5), ACCENT);
            }
            if let Some(t) = target {
                painter.circle_stroke(to_px(t), if close { 8.0 } else { 5.0 }, Stroke::new(1.5, if close { WARN } else { TEXT }));
            }
            if resp.clicked() && !resp.double_clicked() {
                if close {
                    sk.closed = true;
                    sk.drawing = false;
                } else if let Some(t) = target {
                    sk.points.push(t);
                }
            }
            if enter || resp.double_clicked() || escape {
                sk.drawing = false;
                // Double-clic : son second clic a posé un point en double.
                if sk.points.len() >= 2 {
                    let n = sk.points.len();
                    let (a, b) = (sk.points[n - 1], sk.points[n - 2]);
                    if (a[0] - b[0]).abs() + (a[1] - b[1]).abs() < 1e-4 {
                        sk.points.pop();
                    }
                }
            }
            if resp.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        } else {
            // Modifier : glisser un point ou toute la forme, double-clic pour ajouter un point,
            // clic droit sur un point pour le retirer.
            if resp.drag_started_by(egui::PointerButton::Primary) {
                self.led.sketch_grab = match (on_point, on_line) {
                    (Some(k), _) => Some(SketchGrab::Point(k)),
                    (None, true) => Some(SketchGrab::All),
                    _ => None,
                };
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                let d = resp.drag_delta();
                let sk = &mut self.led.sketch;
                match self.led.sketch_grab {
                    Some(SketchGrab::Point(k)) => {
                        if let (Some(p), Some(h)) = (sk.points.get_mut(k), resp.interact_pointer_pos()) {
                            *p = snap(to_m(h));
                        }
                    }
                    Some(SketchGrab::All) => {
                        for p in &mut sk.points {
                            *p = [p[0] + d.x / scale, p[1] - d.y / scale];
                        }
                    }
                    None => {}
                }
            }
            if resp.drag_stopped() {
                self.led.sketch_grab = None;
            }
            let sk = &mut self.led.sketch;
            if resp.double_clicked() {
                if let Some(h) = hover {
                    let n = sk.points.len();
                    let segs = if sk.closed && n > 2 { n } else { n.saturating_sub(1) };
                    let best = (0..segs).min_by(|&a, &b| {
                        let da = seg_dist(h, to_px(sk.points[a]), to_px(sk.points[(a + 1) % n]));
                        let db = seg_dist(h, to_px(sk.points[b]), to_px(sk.points[(b + 1) % n]));
                        da.total_cmp(&db)
                    });
                    if let Some(k) = best.filter(|&k| seg_dist(h, to_px(sk.points[k]), to_px(sk.points[(k + 1) % n])) < 10.0) {
                        sk.points.insert(k + 1, snap(to_m(h)));
                    }
                }
            }
            if resp.secondary_clicked() {
                if let Some(k) = on_point.filter(|_| sk.points.len() > 2) {
                    sk.points.remove(k);
                }
            }
            if on_point.is_some() || on_line {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }
        }
        let hint = if self.led.sketch.drawing {
            "Clic : poser un point (tous les 5 cm, Alt : libre)  ·  Maj : horizontale, verticale ou 45°  ·  clic sur le premier point : boucle fermée  ·  Entrée ou double-clic : terminer  ·  Retour arrière : retirer"
        } else {
            "Glisser un point ou la forme  ·  double-clic sur la forme : ajouter un point  ·  clic droit sur un point : le retirer  ·  clic droit glissé : déplacer la vue  ·  molette : zoom"
        };
        ui.painter().text(egui::pos2(canvas.min.x, rect.max.y), egui::Align2::LEFT_BOTTOM, hint, egui::FontId::proportional(11.0), TEXT_WEAK);

        // Tracé de la scène modifié en direct.
        if let Some(id) = self.led.sketch.editing {
            let sk = &self.led.sketch;
            if let Some(f) = self.project.leds.fixtures.iter_mut().find(|f| f.id == id) {
                let pts: Vec<[f32; 3]> = sk.points.iter().map(|p| [p[0], p[1], 0.0]).collect();
                if f.points != pts || f.closed != sk.closed || f.count != count {
                    f.points = pts;
                    f.closed = sk.closed;
                    f.count = count;
                }
            }
        }
        if done {
            self.led.view = Some(LedView::Both);
        }
        if add {
            self.add_sketch(count);
        }
    }

    /// Ajoute la forme dessinée à la scène : debout, au milieu de la salle, face à l'avant.
    fn add_sketch(&mut self, count: u32) {
        let sk = &self.led.sketch;
        let (mut minx, mut maxx, mut miny) = (f32::MAX, f32::MIN, f32::MAX);
        for p in &sk.points {
            minx = minx.min(p[0]);
            maxx = maxx.max(p[0]);
            miny = miny.min(p[1]);
        }
        // Origine de l'élément : milieu du bas de la forme.
        let (cx, by) = ((minx + maxx) / 2.0, miny);
        let local: Vec<[f32; 2]> = sk.points.iter().map(|p| [p[0] - cx, p[1] - by]).collect();
        let setup = &mut self.project.leds;
        let id = setup.next_fixture_id();
        let n = setup.fixtures.iter().filter(|f| f.kind == FixtureKind::Path).count() + 1;
        let mut f = Fixture::new(FixtureKind::Path, id, format!("Tracé {n}"));
        f.points = local.iter().map(|p| [p[0], p[1], 0.0]).collect();
        f.closed = sk.closed;
        f.count = count;
        f.pos = self.led.cursor3d.unwrap_or([0.0, by.max(0.0), setup.room[1] * 0.5]);
        f.rot = [0.0; 3];
        if let Some(prev) = self.led.selected.and_then(|s| setup.fixtures.get(s)).or(setup.fixtures.last()) {
            f.controller = prev.controller;
            f.output = prev.output;
        } else if let Some(c) = setup.controllers.first() {
            f.controller = c.id;
        }
        setup.fixtures.push(f);
        let index = setup.fixtures.len() - 1;
        // La planche reste liée au nouveau bandeau : la modifier le modifie dans la scène.
        self.led.sketch.points = local;
        self.led.sketch.editing = Some(id);
        self.led.sketch.drawing = false;
        self.frame_sketch();
        self.select(Target::Fixture(index));
        // Place au bandeau dans la scène.
        self.led.view = Some(LedView::Both);
    }

    /// Cadre toute la scène (en gardant l'angle de vue, ou celui donné), en douceur.
    fn frame_all(&mut self, angle: Option<(f32, f32)>) {
        self.led.drone = false;
        let room = self.project.leds.room;
        let l = &self.project.leds;
        let mut pts: Vec<[f32; 3]> = self.led.leds.iter().map(|x| x.pos).collect();
        for s in &l.surfaces {
            pts.push(s.pos);
            pts.push(leds::place(s.pos, s.rot, [0.0, s.size[1], 0.0]));
        }
        pts.extend(l.dmx.iter().map(|d| d.pos));
        pts.extend(l.lasers.iter().map(|x| x.pos));
        let (target, radius) = if pts.is_empty() {
            ([0.0, 1.0, room[1] * 0.5], room[0].max(room[1]) * 0.6)
        } else {
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &pts {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            let c = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
            let r = (0..3).map(|k| (hi[k] - lo[k]) / 2.0).map(|v| v * v).sum::<f32>().sqrt();
            (c, r.max(0.8))
        };
        let cam = self.led.camera.unwrap_or_else(|| Orbit::home(room));
        let (yaw, pitch) = angle.unwrap_or((cam.yaw, cam.pitch));
        // Champ de 50° : la sphère englobante tient dans la vue.
        let dist = radius / (25f32.to_radians()).sin() * 1.05;
        self.led.cam_goal = Some(Orbit { yaw, pitch, dist, target });
    }

    /// Valeurs en direct d'un appareil DMX : rafale en cours, temps écoulés, enveloppe du temps.
    pub(super) fn dmx_live_state(&self, d: &crate::dmx::Fixture) -> crate::dmx::Live {
        let now = self.clock();
        let beat = self.rhythm.tempo.beat_at(now);
        crate::dmx::Live {
            burst: self.led.bursts.get(&d.id).is_some_and(|&until| until > now),
            beats: beat.max(0.0).floor() as u64,
            env: (-(beat.fract() as f32) * 4.0).exp(),
        }
    }

    /// Ce que font les appareils DMX dans le simulateur : faisceaux, fumée, bulles.
    fn dmx_effects(&self, view: &View, items: &mut Vec<(f32, Item)>, beat: f32) {
        let targets = self.laser_targets(beat);
        use crate::dmx::{Kind, Role};
        let t = self.clock() as f32;
        for d in &self.project.leds.dmx {
            let live = self.dmx_live_state(d);
            let get = |r: Role| d.live_role(r, live).map_or(0.0, |v| f32::from(v) / 255.0);
            let on = d.on || live.burst;
            let (_, size) = d.kind.body();
            let world = |p: [f32; 3]| leds::place(d.pos, d.rot, p);
            let front = [0.0, size[1] * 0.5, -size[2] * 0.5];
            // Stroboscope : allumé une demi-période sur deux.
            let strobe = get(Role::Strobe);
            if strobe > 0.03 && (t * (1.0 + strobe * 14.0)).fract() > 0.5 {
                continue;
            }
            let beam = |items: &mut Vec<(f32, Item)>, from: [f32; 3], dir: [f32; 3], len: f32, c: [u8; 3], w: f32| {
                let to = [from[0] + dir[0] * len, from[1] + dir[1] * len, from[2] + dir[2] * len];
                let (a, mut b) = (world(from), world(to));
                // Le faisceau s'arrête sur le premier objet touché (et y laisse un point lumineux).
                let wd = sub(b, a);
                let len_w = dot(wd, wd).sqrt().max(1e-4);
                let dn = wd.map(|v| v / len_w);
                if let Some(h) = first_hit(a, dn, &targets).filter(|&h| h < len_w) {
                    b = [a[0] + dn[0] * h, a[1] + dn[1] * h, a[2] + dn[2] * h];
                    if let Some((p, z)) = view.project(b) {
                        items.push((z - 0.01, Item::Puff { p, r: 2.5, c: glow(c, 0.9), ring: false }));
                    }
                }
                if let Some((pa, pb)) = view.segment(a, b) {
                    items.push(((pa.1 + pb.1) * 0.5, Item::Beam { a: pa.0, b: pb.0, c, w }));
                }
            };
            let color = |v: f32, j: usize, n: usize| -> [u8; 3] {
                // Couleur 0 : multicolore ; sinon une teinte.
                let h = if v < 0.03 { j as f32 / n.max(1) as f32 } else { v };
                leds::hue(h).map(|x| x as u8)
            };
            match d.kind {
                Kind::Laser if on && get(Role::Mode) > 0.0 => {
                    let n = 3 + (get(Role::Pattern) * 8.0) as usize;
                    let spread = (5.0 + get(Role::Size) * 55.0).to_radians();
                    let spin = (get(Role::Rotation) - 0.0) * 4.0 * t;
                    let (yaw, pitch) = ((get(Role::PosX) - 0.5) * 0.7, (get(Role::PosY) - 0.5) * 0.5);
                    for j in 0..n {
                        let a = -spread / 2.0 + spread * j as f32 / (n - 1) as f32;
                        let (ox, oy) = (a.sin() * spin.cos(), a.sin() * spin.sin());
                        let dir = norm([ox + yaw, oy + pitch, -1.0]);
                        beam(items, front, dir, 9.0, color(get(Role::Color), j, n), 1.2);
                    }
                }
                Kind::Derby if on && get(Role::Mode) > 0.0 => {
                    let phase = t * (0.3 + get(Role::Rotation) * 3.0);
                    const RGBW: [[u8; 3]; 4] = [[255, 40, 40], [40, 255, 60], [60, 90, 255], [255, 255, 255]];
                    for j in 0..8 {
                        let az = phase + j as f32 * std::f32::consts::TAU / 8.0;
                        let dir = norm([az.sin() * 0.8, -0.7, az.cos() * 0.8]);
                        let c = if get(Role::Color) < 0.03 { RGBW[j % 4] } else { color(get(Role::Color), j, 8) };
                        beam(items, [0.0, 0.0, 0.0], dir, 4.0, c, 2.0);
                    }
                }
                Kind::Par | Kind::Generic if on => {
                    let dim = if d.kind == Kind::Par { get(Role::Dimmer) } else { 1.0 };
                    let c = [get(Role::Red), get(Role::Green), get(Role::Blue)].map(|v| (v * dim * 255.0) as u8);
                    if c.iter().any(|&v| v > 4) {
                        for j in 0..5 {
                            let a = (j as f32 - 2.0) * 0.06;
                            beam(items, front, norm([a, -0.25, -1.0]), 3.5, c, 4.0);
                        }
                    }
                }
                Kind::Smoke | Kind::Bubble => {
                    let out = get(Role::Output);
                    if out < 0.02 {
                        continue;
                    }
                    let bubbles = d.kind == Kind::Bubble;
                    let n = if bubbles { 28 } else { 16 };
                    for k in 0..n {
                        let r1 = crate::leds::hash01(d.id, k as u32, 1);
                        let r2 = crate::leds::hash01(d.id, k as u32, 2);
                        let age = (t * if bubbles { 0.22 } else { 0.35 } + k as f32 / n as f32 + r1 * 0.3).fract();
                        let local = if bubbles {
                            [(r2 - 0.5) * age * 2.0, front[1] + age * 1.4 - age * age * 0.5, front[2] - age * (1.0 + r1) * 1.2]
                        } else {
                            [(r2 - 0.5) * age, front[1] + age * 0.5, front[2] - age * 2.5]
                        };
                        let Some((p, z)) = view.project(world(local)) else { continue };
                        if bubbles {
                            let r = (0.035 * view.focal / z).clamp(1.5, 10.0);
                            let a = ((1.0 - age) * 220.0 * out) as u8;
                            items.push((z, Item::Puff { p, r, c: Color32::from_white_alpha(a), ring: true }));
                        } else {
                            let r = ((0.15 + age * 0.9) * view.focal / z).clamp(2.0, 400.0);
                            let a = ((1.0 - age).powf(1.5) * 60.0 * out) as u8;
                            items.push((z, Item::Puff { p, r, c: Color32::from_rgba_unmultiplied(200, 200, 210, a), ring: false }));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Onglet latéral DMX : appareils, puis sortie.
    pub(super) fn dmx_ui(&mut self, ui: &mut egui::Ui) {
        let sec = self.section();
        sec.show(ui, 0, |ui| self.dmx_scene_ui(ui));
        sec.show(ui, 1, |ui| self.dmx_output_ui(ui));
    }

    /// Faces du décor et le sol, pour arrêter les faisceaux laser sur ce qu'ils touchent.
    fn laser_targets(&self, beat: f32) -> Vec<[[f32; 3]; 4]> {
        let room = self.project.leds.room;
        let mut faces: Vec<[[f32; 3]; 4]> = self
            .project
            .leds
            .surfaces
            .iter()
            .filter(|s| s.kind != leds::SurfaceKind::Projector)
            .flat_map(|s| s.faces(beat).into_iter().map(|f| f.quad))
            .collect();
        let (hw, d) = (room[0] * 0.5 + 20.0, room[1] + 20.0);
        faces.push([[-hw, 0.0, -20.0], [hw, 0.0, -20.0], [hw, 0.0, d], [-hw, 0.0, d]]);
        faces
    }

    /// Faisceaux des lasers ILDA : un rayon vers chaque point de la forme, arrêté sur le premier
    /// objet touché ; la forme se dessine sur ce qu'elle touche.
    fn ilda_beams(&self, view: &View, items: &mut Vec<(f32, Item)>, targets: &[[[f32; 3]; 4]]) {
        let t = self.clock() as f32;
        for l in &self.project.leds.lasers {
            let frame = crate::laser::frame(l, t, self.rhythm.pulse.env, 360, &self.laser_input(&l.cue));
            let spread = (l.fov.clamp(5.0, 120.0) * 0.5).to_radians().tan();
            let from = leds::place(l.pos, l.rot, [0.0, 0.07, -0.17]);
            let mut prev: Option<([f32; 3], [u8; 3])> = None;
            for (k, p) in frame.iter().step_by(2).enumerate() {
                if !p.lit() {
                    prev = None;
                    continue;
                }
                let local = norm([p.x * spread, p.y * spread, -1.0]);
                let dir = sub(leds::place(l.pos, l.rot, [local[0], 0.07 + local[1], -0.17 + local[2]]), from);
                let dist = first_hit(from, dir, targets).unwrap_or(9.0).min(30.0);
                let hit = [from[0] + dir[0] * dist, from[1] + dir[1] * dist, from[2] + dir[2] * dist];
                let c = [p.r, p.g, p.b].map(|v| (v * 255.0) as u8);
                // Un rayon sur trois, pour voir les faisceaux sans tout noyer.
                if k % 4 == 0 {
                    if let Some((a, b)) = view.segment(from, hit) {
                        items.push(((a.1 + b.1) * 0.5, Item::Beam { a: a.0, b: b.0, c: c.map(|v| v / 2), w: 0.8 }));
                    }
                }
                // Le dessin sur la surface touchée.
                if let Some((q, _)) = prev {
                    if let Some((pa, pb)) = view.segment(q, hit) {
                        if pa.0.distance(pb.0) < 80.0 {
                            // Nettement devant la surface touchée : jamais recouvert par elle.
                            items.push(((pa.1 + pb.1) * 0.5 - 0.15, Item::Beam { a: pa.0, b: pb.0, c, w: 2.0 }));
                        }
                    }
                }
                prev = Some((hit, c));
            }
        }
    }

    /// Appareils DMX de la scène : ajout, liste, réglages.
    fn dmx_scene_ui(&mut self, ui: &mut egui::Ui) {
        use crate::dmx::{self, Kind};
        theme::section(ui, ic::LIGHTNING, "Appareils DMX");
        ui.label(
            egui::RichText::new("Lasers, derbys, machines à bulles et à fumée, projecteurs : posés dans la scène, pilotés en DMX (envoi : DMX › Sortie).")
                .small()
                .color(TEXT_WEAK),
        );
        let setup = &mut self.project.leds;
        ui.horizontal_wrapped(|ui| {
            for kind in Kind::ALL {
                if ui.button(with_icon(ic::PLUS, kind.name())).clicked() {
                    let channels = dmx::profile(kind);
                    let id = setup.next_dmx_id();
                    let n = setup.dmx.iter().filter(|d| d.kind == kind).count() + 1;
                    let depth = setup.room[1];
                    // Lasers et derbys en hauteur au fond, face à la salle ; fumée et bulles au sol.
                    let pos = match kind {
                        Kind::Laser | Kind::Derby | Kind::Par => [0.0, 2.2, (depth - 1.0).max(0.5)],
                        _ => [0.0, 0.0, (depth - 1.5).max(0.5)],
                    };
                    let pos = self.led.cursor3d.unwrap_or(pos);
                    let address = setup.next_dmx_address(channels.len());
                    setup.dmx.push(dmx::Fixture {
                        id,
                        name: format!("{} {n}", kind.name()),
                        kind,
                        address,
                        channels,
                        pos,
                        ..dmx::Fixture::default()
                    });
                    self.led.selected_dmx = Some(setup.dmx.len() - 1);
                    self.led.selected = None;
                    self.led.selected_surface = None;
                }
            }
        });
        let mut action: Option<(usize, u8)> = None;
        for (i, d) in setup.dmx.iter_mut().enumerate() {
            let sel = self.led.selected_dmx == Some(i);
            ui.horizontal(|ui| {
                let text = egui::RichText::new(format!("{}  ·  U{} @{}", d.name, d.universe, d.address));
                if ui.selectable_label(sel, if sel { text.strong() } else { text }).clicked() {
                    action = Some((i, 9));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(theme::icon_button(ic::TRASH).small().frame(false)).on_hover_text("Supprimer").clicked() {
                        action = Some((i, 0));
                    }
                    if ui.add(theme::icon_button(ic::COPY).small().frame(false)).on_hover_text("Dupliquer").clicked() {
                        action = Some((i, 1));
                    }
                    let icon = if d.on { ic::SUN } else { ic::MOON };
                    if ui.add(theme::icon_button(icon).small().frame(false)).on_hover_text(if d.on { "Éteindre" } else { "Allumer" }).clicked() {
                        d.on = !d.on;
                    }
                });
            });
        }
        match action {
            Some((i, 0)) => {
                setup.dmx.remove(i);
                self.led.selected_dmx = None;
            }
            Some((i, 1)) => {
                let mut d = setup.dmx[i].clone();
                d.id = setup.next_dmx_id();
                d.name = format!("{} (copie)", d.name);
                d.pos[0] += 0.6;
                d.address = setup.next_dmx_address(d.channels.len());
                setup.dmx.insert(i + 1, d);
                self.led.selected_dmx = Some(i + 1);
            }
            Some((i, _)) => {
                self.select(Target::Dmx(i));
            }
            None => {}
        }
        let setup = &mut self.project.leds;
        let Some(di) = self.led.selected_dmx.filter(|&i| i < setup.dmx.len()) else { return };
        let now = self.start.elapsed().as_secs_f64();
        let d = &mut setup.dmx[di];
        let mut hold: Option<bool> = None;
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label("Nom");
                ui.text_edit_singleline(&mut d.name);
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.on, "Allumé");
                let label = match d.kind {
                    Kind::Smoke => "Fumée !",
                    Kind::Bubble => "Bulles !",
                    _ => "Flash !",
                };
                let r = ui.add(egui::Button::new(egui::RichText::new(with_icon(ic::LIGHTNING, label)).strong()).sense(Sense::click_and_drag()))
                    .on_hover_text("À fond tant qu'on appuie");
                if r.is_pointer_button_down_on() {
                    hold = Some(true);
                } else if r.drag_stopped() || r.clicked() {
                    hold = Some(false);
                }
                if matches!(d.kind, Kind::Laser | Kind::Par) {
                    ui.checkbox(&mut d.beat, "Sur les temps").on_hover_text(if d.kind == Kind::Laser {
                        "Le motif change à chaque temps (onglet Rythme)"
                    } else {
                        "L'intensité flashe sur les temps (onglet Rythme)"
                    });
                }
            });
            ui.horizontal(|ui| {
                ui.label("Univers");
                ui.add(egui::DragValue::new(&mut d.universe).range(0..=32767));
                ui.label("Adresse");
                ui.add(egui::DragValue::new(&mut d.address).range(1..=512));
                ui.label(egui::RichText::new(format!("→ {}", d.last_address())).small().color(TEXT_WEAK));
            });
            ui.horizontal(|ui| {
                ui.label("Profil");
                egui::ComboBox::from_id_salt("dmx_profile").selected_text("Remettre un profil…").show_ui(ui, |ui| {
                    for k in Kind::ALL {
                        if ui.selectable_label(false, k.name()).clicked() {
                            d.kind = k;
                            d.channels = dmx::profile(k);
                        }
                    }
                });
            });
            ui.label(egui::RichText::new("Canaux (vérifiez l'ordre dans la notice de l'appareil)").small().color(TEXT_WEAK));
            let mut remove = None;
            for (k, c) in d.channels.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{}", d.address as usize + k)).small().color(TEXT_WEAK));
                    ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(100.0));
                    egui::ComboBox::from_id_salt(("dmx_role", k)).selected_text(c.role.name()).width(80.0).show_ui(ui, |ui| {
                        for r in dmx::Role::ALL {
                            ui.selectable_value(&mut c.role, r, r.name());
                        }
                    });
                    ui.add(egui::Slider::new(&mut c.value, 0..=255));
                    if ui.small_button(ic::X).on_hover_text("Retirer ce canal").clicked() {
                        remove = Some(k);
                    }
                });
            }
            if let Some(k) = remove {
                d.channels.remove(k);
            }
            if ui.small_button(with_icon(ic::PLUS, "Canal")).clicked() {
                let n = d.channels.len() + 1;
                d.channels.push(dmx::Channel { name: format!("Canal {n}"), ..dmx::Channel::default() });
            }
            ui.horizontal(|ui| {
                ui.label("Position");
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.add(egui::DragValue::new(&mut d.pos[k]).speed(0.01).prefix(format!("{axis} ")).suffix(" m"));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Rotation");
                for (k, name) in ["cap", "incl.", "roulis"].iter().enumerate() {
                    ui.add(egui::DragValue::new(&mut d.rot[k]).speed(1.0).range(-360.0..=360.0).prefix(format!("{name} ")).suffix("°"));
                }
            });
        });
        match hold {
            Some(true) => {
                self.led.bursts.insert(self.project.leds.dmx[di].id, f64::INFINITY);
            }
            Some(false) => {
                self.led.bursts.insert(self.project.leds.dmx[di].id, now + 0.2);
            }
            None => {}
        }
    }

    /// Branchements : chaque bandeau, son contrôleur et sa sortie (pin), modifiables d'un coup d'œil.
    fn plugs_ui(&mut self, ui: &mut egui::Ui) {
        let setup = &mut self.project.leds;
        if setup.fixtures.is_empty() {
            return;
        }
        theme::section(ui, ic::LIST, "Branchements");
        ui.label(egui::RichText::new("Quel contrôleur et quel pin pilotent chaque bandeau. Renvoyez la configuration au contrôleur après un changement.").small().color(TEXT_WEAK));
        let ctrls: Vec<(u32, String, Vec<Option<u8>>)> = setup.controllers.iter().map(|c| (c.id, c.name.clone(), c.pins.clone())).collect();
        egui::Grid::new("led_plugs").striped(true).num_columns(4).spacing([8.0, 4.0]).show(ui, |ui| {
            for f in &mut setup.fixtures {
                ui.label(elide_name(&f.name)).on_hover_text(&f.name);
                let cur = ctrls.iter().find(|c| c.0 == f.controller);
                egui::ComboBox::from_id_salt(("plug_ctrl", f.id)).selected_text(cur.map_or("aucun".to_owned(), |c| c.1.clone())).width(100.0).show_ui(ui, |ui| {
                    for (id, name, _) in &ctrls {
                        ui.selectable_value(&mut f.controller, *id, name);
                    }
                    ui.selectable_value(&mut f.controller, 0, "aucun");
                });
                let pins = cur.map_or(Vec::new(), |c| c.2.clone());
                let out_name = |o: usize| match pins.get(o).copied().flatten() {
                    Some(p) => format!("S{} · pin {p}", o + 1),
                    None => format!("S{}", o + 1),
                };
                egui::ComboBox::from_id_salt(("plug_out", f.id)).selected_text(out_name(f.output)).width(90.0).show_ui(ui, |ui| {
                    for o in 0..pins.len().max(1) {
                        ui.selectable_value(&mut f.output, o, out_name(o));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{} LEDs", f.led_count())).small().color(TEXT_WEAK));
                    ui.checkbox(&mut f.reverse, "").on_hover_text("Câblé à l'envers");
                });
                ui.end_row();
            }
        });
    }

    /// Sortie DMX (onglet Contrôleurs).
    fn dmx_output_ui(&mut self, ui: &mut egui::Ui) {
        use crate::dmx::Protocol;
        theme::section(ui, ic::LIGHTNING, "Sortie DMX");
        let out = &mut self.project.leds.dmx_out;
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label("Protocole");
                egui::ComboBox::from_id_salt("dmx_proto").selected_text(out.protocol.name()).width(180.0).show_ui(ui, |ui| {
                    for p in Protocol::ALL {
                        ui.selectable_value(&mut out.protocol, p, p.name());
                    }
                });
            });
            match out.protocol {
                Protocol::ArtNet | Protocol::Sacn => {
                    ui.horizontal(|ui| {
                        ui.label("Boîtier");
                        ui.add(egui::TextEdit::singleline(&mut out.host).hint_text("vide : tout le réseau").desired_width(150.0))
                            .on_hover_text("Adresse IP du boîtier Art-Net / sACN (ou du laser s'il le reçoit directement). Vide : diffusion Art-Net, ou multicast sACN.");
                    });
                }
                Protocol::EnttecPro => {
                    ui.horizontal(|ui| {
                        ui.label("Port");
                        let ports = &mut self.led.serial_ports;
                        let shown = if out.serial.is_empty() { "Choisir…".to_owned() } else { out.serial.clone() };
                        let r = egui::ComboBox::from_id_salt("dmx_serial").selected_text(shown).width(200.0).show_ui(ui, |ui| {
                            for p in ports.iter() {
                                ui.selectable_value(&mut out.serial, p.clone(), p);
                            }
                        });
                        if r.response.clicked() || ports.is_empty() {
                            *ports = crate::dmx::serial_ports();
                        }
                    });
                }
            }
            fit_slider(ui, "Images/s", egui::Slider::new(&mut out.fps, 10..=44));
            let live = self.led.dmx_live;
            let text = if live { with_icon(ic::WIFI_HIGH, "ENVOI DMX : ACTIF") } else { with_icon(ic::CUBE, "DMX : simulation seulement") };
            let mut b = egui::Button::new(egui::RichText::new(text).strong().color(if live { BG } else { TEXT_WEAK }))
                .min_size(Vec2::new(ui.available_width(), 30.0));
            if live {
                b = b.fill(ACCENT);
            }
            if ui.add(b).clicked() {
                self.led.dmx_live = !live;
            }
            if live {
                let (msg, color) = match self.led.dmx_sender.status.lock().unwrap().clone() {
                    Some(Ok(t)) if t.elapsed().as_secs_f32() < 1.0 => ("envoi en cours".to_owned(), ACCENT),
                    Some(Err(e)) => (e, theme::DANGER),
                    _ => ("en attente…".to_owned(), TEXT_WEAK),
                };
                ui.label(egui::RichText::new(msg).small().color(color));
            }
            ui.label(
                egui::RichText::new("Laserworld : par l'entrée DMX du laser (leur réseau ShowNET est réservé à leur logiciel). Il faut un boîtier Art-Net / sACN ou une interface Enttec DMX USB Pro.")
                    .small()
                    .color(TEXT_WEAK),
            );
        });
    }

    /// Mode dessin du tracé libre (clics : points ; Entrée ou double-clic : terminer).
    fn draw_path(
        &mut self,
        ui: &mut egui::Ui,
        resp: &egui::Response,
        painter: &egui::Painter,
        view: &View,
        rect: Rect,
        face: Option<[[f32; 3]; 4]>,
    ) {
        let ctx = ui.ctx().clone();
        let (enter, escape, back, shift) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::Backspace),
                i.modifiers.shift,
            )
        });
        let Some(drawing) = &mut self.led.drawing else { return };
        if back {
            drawing.points.pop();
        }
        let first_px = drawing.points.first().and_then(|p| view.project(*p)).map(|p| p.0);
        // Point visé : sur la face d'un objet (un peu devant), sinon au sol, ou (Maj) à la hauteur
        // du point précédent.
        let mut close = false;
        let target = resp.hover_pos().and_then(|h| {
            if drawing.points.len() > 2 && first_px.is_some_and(|f| f.distance(h) < 12.0) {
                close = true;
                return drawing.points.first().copied();
            }
            if let (Some(q), false) = (face, shift) {
                let n = norm(cross(sub(q[1], q[0]), sub(q[3], q[0])));
                let n = if dot(n, sub(view.eye, q[0])) < 0.0 { n.map(|v| -v) } else { n };
                return view.ray_plane(h, q[0], n).map(|p| [0, 1, 2].map(|k| p[k] + n[k] * 0.01));
            }
            let y = if shift { drawing.points.last().map_or(0.0, |p| p[1]) } else { 0.0 };
            view.ray_floor(h, y).map(|p| [(p[0] * 20.0).round() / 20.0, y, (p[2] * 20.0).round() / 20.0])
        });

        // Tracé posé et segment en cours.
        let stroke = Stroke::new(2.0, ACCENT);
        let pts: Vec<egui::Pos2> = drawing.points.iter().filter_map(|p| view.project(*p)).map(|p| p.0).collect();
        for w in pts.windows(2) {
            painter.line_segment([w[0], w[1]], stroke);
        }
        for (k, p) in pts.iter().enumerate() {
            painter.circle_filled(*p, if k == 0 { 5.0 } else { 3.5 }, ACCENT);
        }
        if let (Some(last), Some(t)) = (drawing.points.last(), target) {
            if let (Some(a), Some(b)) = (view.project(*last), view.project(t)) {
                painter.line_segment([a.0, b.0], Stroke::new(1.5, ACCENT.gamma_multiply(0.6)));
                let len = sub(t, *last);
                let text = format!("{:.2} m", dot(len, len).sqrt());
                painter.text(b.0 + Vec2::new(10.0, -10.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::proportional(11.5), ACCENT);
            }
        }
        if let Some(t) = target.and_then(|t| view.project(t)) {
            painter.circle_stroke(t.0, if close { 8.0 } else { 5.0 }, Stroke::new(1.5, if close { WARN } else { TEXT }));
        }

        let mut finish = enter || resp.double_clicked();
        let mut closed = false;
        if resp.clicked() && !resp.double_clicked() {
            if close {
                finish = true;
                closed = true;
            } else if let Some(t) = target {
                drawing.points.push(t);
            }
        }
        if resp.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            ctx.request_repaint();
        }

        // Bandeau du mode dessin.
        let n = drawing.points.len();
        let bar = Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::new(rect.width() - 16.0, 26.0));
        let mut cancel = escape;
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(with_icon(ic::PENCIL_SIMPLE, &format!("Tracé : {n} point(s)"))).color(ACCENT).strong());
                if ui.add_enabled(n >= 2, theme::primary_button("Terminer")).clicked() {
                    finish = true;
                }
                if ui.button("Annuler").clicked() {
                    cancel = true;
                }
            });
        });
        painter.text(
            rect.left_bottom() + Vec2::new(8.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            "Clic : poser un point (au sol, ou sur la face d'un mur, d'un écran, d'une boîte)  ·  Maj : à la hauteur du point précédent  ·  \
             clic sur le premier point : boucle fermée  ·  Entrée ou double-clic : terminer  ·  Retour arrière : retirer  ·  Échap : annuler",
            egui::FontId::proportional(11.0),
            TEXT_WEAK,
        );

        if cancel {
            self.led.drawing = None;
            return;
        }
        if finish {
            let Some(d) = self.led.drawing.take() else { return };
            // Les clics d'un double-clic posent des points en double : on les retire.
            let mut pts: Vec<[f32; 3]> = Vec::new();
            for p in d.points {
                if pts.last().is_none_or(|q| dot(sub(p, *q), sub(p, *q)) > 1e-4) {
                    pts.push(p);
                }
            }
            if pts.len() < 2 {
                return;
            }
            self.finish_path(&pts, closed, d.fixture);
        }
    }

    /// Crée (ou remplace) l'élément tracé : LEDs réparties à la même densité.
    fn finish_path(&mut self, pts: &[[f32; 3]], closed: bool, fixture: Option<usize>) {
        let setup = &mut self.project.leds;
        let index = match fixture.filter(|&i| i < setup.fixtures.len()) {
            Some(i) => {
                let f = &mut setup.fixtures[i];
                let density = if f.kind == FixtureKind::Path && f.path_length() > 0.01 {
                    f.count as f32 / f.path_length()
                } else {
                    PATH_DENSITY
                };
                f.kind = FixtureKind::Path;
                f.set_path(pts, closed);
                f.count = ((f.path_length() * density).round() as u32).clamp(2, leds::MAX_COUNT);
                i
            }
            None => {
                let id = setup.next_fixture_id();
                let n = setup.fixtures.iter().filter(|f| f.kind == FixtureKind::Path).count() + 1;
                let mut f = Fixture::new(FixtureKind::Path, id, format!("Tracé {n}"));
                if let Some(prev) = setup.fixtures.last() {
                    f.controller = prev.controller;
                    f.output = prev.output;
                } else if let Some(c) = setup.controllers.first() {
                    f.controller = c.id;
                }
                f.set_path(pts, closed);
                f.count = ((f.path_length() * PATH_DENSITY).round() as u32).clamp(2, leds::MAX_COUNT);
                setup.fixtures.push(f);
                setup.fixtures.len() - 1
            }
        };
        self.select(Target::Fixture(index));
    }

    /// Lance le dessin d'un tracé libre dans la vue 3D.
    fn start_drawing(&mut self, fixture: Option<usize>) {
        self.led.drawing = Some(Drawing { points: Vec::new(), fixture });
        if self.shown_view() == LedView::Output {
            self.led.view = Some(LedView::Both);
        }
    }

    /// Met à jour l'image de chaque sortie affichée sur les écrans de la scène.
    fn update_screen_texture(&mut self, ctx: &egui::Context) {
        let n = self.project.outputs.len().max(1);
        let mut outs: Vec<usize> = self.project.leds.surfaces.iter().filter(|s| s.show_output).map(|s| s.output.min(n - 1)).collect();
        outs.sort_unstable();
        outs.dedup();
        for k in outs {
            let src = self.frame_of(k);
            let frame = src.lock().unwrap();
            let shown = self.led.screen_tex.get(&k).map(|t| t.1);
            if shown == Some(frame.serial) || frame.size.0 == 0 {
                continue;
            }
            let (w, h) = (frame.size.0 as usize, frame.size.1 as usize);
            if frame.pixels.len() < w * h * 4 {
                continue;
            }
            // Lignes lues sur le GPU du bas vers le haut : on les remet dans l'ordre.
            let mut rgba = Vec::with_capacity(w * h * 4);
            for row in (0..h).rev() {
                rgba.extend_from_slice(&frame.pixels[row * w * 4..(row + 1) * w * 4]);
            }
            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            let serial = frame.serial;
            match self.led.screen_tex.get_mut(&k) {
                Some(t) => {
                    t.0.set(image, egui::TextureOptions::LINEAR);
                    t.1 = serial;
                }
                None => {
                    let tex = ctx.load_texture(format!("led_scene_output_{k}"), image, egui::TextureOptions::LINEAR);
                    self.led.screen_tex.insert(k, (tex, serial));
                }
            }
        }
    }

    fn select(&mut self, t: Target) {
        self.led.multi_surf.clear();
        self.led.multi_dmx.clear();
        let set = self.target_set(t);
        self.led.selected_set = (set != 0).then_some(set);
        self.led.multi = match t {
            Target::Fixture(i) => self.project.leds.fixtures.get(i).map(|f| vec![f.id]).unwrap_or_default(),
            Target::Surface(_) | Target::Dmx(_) | Target::Laser(_) => Vec::new(),
        };
        if let Target::Fixture(i) = t {
            self.led.anchor = Some(i);
        }
        match t {
            Target::Fixture(i) => {
                self.led.selected = Some(i);
                self.led.selected_surface = None;
                self.sub_tab[RightTab::Leds as usize] = 1;
            }
            Target::Surface(i) => {
                self.led.selected_surface = Some(i);
                self.led.selected = None;
                self.sub_tab[RightTab::Leds as usize] = 0;
            }
            Target::Dmx(i) => {
                self.led.selected_dmx = Some(i);
                self.led.selected = None;
                self.led.selected_surface = None;
                self.right_tab = RightTab::Dmx;
                self.sub_tab[RightTab::Dmx as usize] = 0;
            }
            Target::Laser(i) => {
                self.laser.selected = Some(i);
                self.led.selected = None;
                self.led.selected_surface = None;
                self.right_tab = RightTab::Laser;
                self.sub_tab[RightTab::Laser as usize] = 0;
            }
        }
        if !matches!(t, Target::Dmx(_)) {
            self.led.selected_dmx = None;
        }
    }

    fn target_pos(&self, t: Target) -> Option<[f32; 3]> {
        match t {
            Target::Fixture(i) => self.project.leds.fixtures.get(i).map(|f| f.pos),
            Target::Surface(i) => self.project.leds.surfaces.get(i).map(|s| s.pos),
            Target::Dmx(i) => self.project.leds.dmx.get(i).map(|d| d.pos),
            Target::Laser(i) => self.project.leds.lasers.get(i).map(|l| l.pos),
        }
    }

    /// Cmd+clic : ajoute ou retire un bandeau de la sélection multiple.
    fn toggle_multi(&mut self, i: usize) {
        let Some(id) = self.project.leds.fixtures.get(i).map(|f| f.id) else { return };
        if self.led.multi.is_empty() {
            if let Some(cur) = self.led.selected.and_then(|s| self.project.leds.fixtures.get(s)) {
                self.led.multi.push(cur.id);
            }
        }
        if let Some(k) = self.led.multi.iter().position(|&m| m == id) {
            self.led.multi.remove(k);
        } else {
            self.led.multi.push(id);
            self.led.selected = Some(i);
        }
        self.led.selected_surface = None;
        self.led.selected_set = None;
        self.led.anchor = Some(i);
    }

    /// Nombre d'objets choisis ensemble (bandeaux, décor, appareils DMX).
    fn multi_count(&self) -> usize {
        self.led.multi.len() + self.led.multi_surf.len() + self.led.multi_dmx.len()
    }

    /// L'objet fait partie de la sélection multiple.
    fn in_multi(&self, t: Target) -> bool {
        let l = &self.project.leds;
        match t {
            Target::Fixture(i) => l.fixtures.get(i).is_some_and(|f| self.led.multi.contains(&f.id)),
            Target::Surface(i) => l.surfaces.get(i).is_some_and(|s| self.led.multi_surf.contains(&s.id)),
            Target::Dmx(i) => l.dmx.get(i).is_some_and(|d| self.led.multi_dmx.contains(&d.id)),
            Target::Laser(_) => false,
        }
    }

    /// Applique `op` à tous les objets choisis, réunis le temps de l'opération dans un groupe
    /// provisoire (les opérations de groupe : déplacer, tourner, redimensionner).
    fn with_multi(&mut self, op: impl FnOnce(&mut LedSetup, u32)) {
        const TEMP: u32 = u32::MAX;
        let (mf, ms, md) = (self.led.multi.clone(), self.led.multi_surf.clone(), self.led.multi_dmx.clone());
        let l = &mut self.project.leds;
        let saved_f: Vec<(usize, u32)> = l.fixtures.iter().enumerate().filter(|(_, f)| mf.contains(&f.id)).map(|(i, f)| (i, f.set)).collect();
        let saved_s: Vec<(usize, u32)> = l.surfaces.iter().enumerate().filter(|(_, s)| ms.contains(&s.id)).map(|(i, s)| (i, s.set)).collect();
        let saved_d: Vec<(usize, u32)> = l.dmx.iter().enumerate().filter(|(_, d)| md.contains(&d.id)).map(|(i, d)| (i, d.set)).collect();
        for &(i, _) in &saved_f {
            l.fixtures[i].set = TEMP;
        }
        for &(i, _) in &saved_s {
            l.surfaces[i].set = TEMP;
        }
        for &(i, _) in &saved_d {
            l.dmx[i].set = TEMP;
        }
        op(l, TEMP);
        for (i, v) in saved_f {
            l.fixtures[i].set = v;
        }
        for (i, v) in saved_s {
            l.surfaces[i].set = v;
        }
        for (i, v) in saved_d {
            l.dmx[i].set = v;
        }
    }

    fn clear_multi(&mut self) {
        self.led.multi.clear();
        self.led.multi_surf.clear();
        self.led.multi_dmx.clear();
    }

    /// Supprime tous les objets choisis.
    fn delete_multi(&mut self) {
        let (mf, ms, md) = (std::mem::take(&mut self.led.multi), std::mem::take(&mut self.led.multi_surf), std::mem::take(&mut self.led.multi_dmx));
        let l = &mut self.project.leds;
        l.fixtures.retain(|f| !mf.contains(&f.id));
        l.surfaces.retain(|s| !ms.contains(&s.id));
        l.dmx.retain(|d| !md.contains(&d.id));
        if l.image_on.is_some_and(|i| ms.contains(&i)) {
            l.image_on = None;
        }
        l.prune_sets();
        self.led.selected = None;
        self.led.selected_surface = None;
        self.led.selected_dmx = None;
    }

    /// Rectangle de sélection : tout ce dont le centre (ou une LED) tombe dedans.
    fn select_in_rect(&mut self, r: Rect, view: &View, add: bool) {
        if !add {
            self.clear_multi();
        }
        let l = &self.project.leds;
        let inside = |p: [f32; 3]| view.project(p).is_some_and(|(q, _)| r.contains(q));
        for (fi, f) in l.fixtures.iter().enumerate() {
            if self.led.leds.iter().any(|x| x.fixture == fi && inside(x.pos)) && !self.led.multi.contains(&f.id) {
                self.led.multi.push(f.id);
            }
        }
        for s in &l.surfaces {
            if inside(leds::place(s.pos, s.rot, [0.0, s.size[1] * 0.5, 0.0])) && !self.led.multi_surf.contains(&s.id) {
                self.led.multi_surf.push(s.id);
            }
        }
        for d in &l.dmx {
            if inside(leds::place(d.pos, d.rot, [0.0, 0.1, 0.0])) && !self.led.multi_dmx.contains(&d.id) {
                self.led.multi_dmx.push(d.id);
            }
        }
        self.led.selected = None;
        self.led.selected_surface = None;
        self.led.selected_dmx = None;
        self.led.selected_set = None;
    }

    /// Barre d'actions sur plusieurs objets choisis (rectangle de sélection).
    fn multi_scene_bar(&mut self, ui: &mut egui::Ui) {
        let n = self.multi_count();
        if n < 2 {
            return;
        }
        let mut action = 0;
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(format!(
                    "{n} objets choisis  ·  {} bandeau(x), {} décor, {} DMX",
                    self.led.multi.len(),
                    self.led.multi_surf.len(),
                    self.led.multi_dmx.len()
                ))
                .strong()
                .color(ACCENT),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.small_button(with_icon(ic::STACK, "Grouper")).on_hover_text("En faire un groupe d'objets").clicked() {
                    action = 1;
                }
                if ui.small_button(with_icon(ic::TRASH, "Supprimer")).clicked() {
                    action = 2;
                }
                if ui.small_button("Désélectionner").clicked() {
                    action = 3;
                }
            });
            ui.label(egui::RichText::new("Glisser l'un d'eux, ou flèches / Q E / PgHaut PgBas : tous bougent ensemble.").small().color(TEXT_WEAK));
        });
        match action {
            1 => {
                let id = self.project.leds.next_set_id();
                self.project.leds.sets.push(leds::LedGroup { id, name: format!("Groupe d'objets {id}"), effect: None });
                self.with_multi(|l, temp| {
                    for f in l.fixtures.iter_mut().filter(|f| f.set == temp) {
                        f.set = id;
                    }
                    for s in l.surfaces.iter_mut().filter(|s| s.set == temp) {
                        s.set = id;
                    }
                    for d in l.dmx.iter_mut().filter(|d| d.set == temp) {
                        d.set = id;
                    }
                });
                // `with_multi` remet les anciens groupes : on refait l'affectation après.
                let (mf, ms, md) = (self.led.multi.clone(), self.led.multi_surf.clone(), self.led.multi_dmx.clone());
                let l = &mut self.project.leds;
                for f in l.fixtures.iter_mut().filter(|f| mf.contains(&f.id)) {
                    f.set = id;
                }
                for s in l.surfaces.iter_mut().filter(|s| ms.contains(&s.id)) {
                    s.set = id;
                }
                for d in l.dmx.iter_mut().filter(|d| md.contains(&d.id)) {
                    d.set = id;
                }
                l.prune_sets();
                self.clear_multi();
                self.led.selected_set = Some(id);
            }
            2 => self.delete_multi(),
            3 => self.clear_multi(),
            _ => {}
        }
        ui.add_space(4.0);
    }

    /// Groupe d'objets d'un objet (0 : aucun).
    fn target_set(&self, t: Target) -> u32 {
        match t {
            Target::Fixture(i) => self.project.leds.fixtures.get(i).map_or(0, |f| f.set),
            Target::Surface(i) => self.project.leds.surfaces.get(i).map_or(0, |s| s.set),
            Target::Dmx(i) => self.project.leds.dmx.get(i).map_or(0, |d| d.set),
            Target::Laser(i) => self.project.leds.lasers.get(i).map_or(0, |l| l.set),
        }
    }

    fn put_in_set(&mut self, t: Target, id: u32) {
        match t {
            Target::Fixture(i) => {
                if let Some(f) = self.project.leds.fixtures.get_mut(i) {
                    f.set = id;
                }
            }
            Target::Surface(i) => {
                if let Some(s) = self.project.leds.surfaces.get_mut(i) {
                    s.set = id;
                }
            }
            Target::Dmx(i) => {
                if let Some(d) = self.project.leds.dmx.get_mut(i) {
                    d.set = id;
                }
            }
            Target::Laser(i) => {
                if let Some(l) = self.project.leds.lasers.get_mut(i) {
                    l.set = id;
                }
            }
        }
    }

    /// L'objet choisi (ou son groupe), sous forme de cible.
    fn current_target(&self) -> Option<Target> {
        match (self.led.selected, self.led.selected_surface, self.led.selected_dmx) {
            (Some(i), _, _) => Some(Target::Fixture(i)),
            (_, Some(i), _) => Some(Target::Surface(i)),
            (_, _, Some(i)) => Some(Target::Dmx(i)),
            _ => None,
        }
    }

    /// Maj+clic : ajoute `t` au groupe de l'objet choisi (créé s'il n'en a pas).
    fn add_to_group(&mut self, t: Target) {
        let Some(cur) = self.current_target() else {
            self.select(t);
            return;
        };
        if cur == t {
            return;
        }
        let mut id = self.target_set(cur);
        if id == 0 {
            id = self.project.leds.next_set_id();
            self.project.leds.sets.push(leds::LedGroup { id, name: format!("Groupe d'objets {id}"), effect: None });
            self.put_in_set(cur, id);
        }
        // Un objet déjà dans un autre groupe y amène tout son groupe.
        let other = self.target_set(t);
        if other != 0 && other != id {
            for f in &mut self.project.leds.fixtures {
                if f.set == other {
                    f.set = id;
                }
            }
            for s in &mut self.project.leds.surfaces {
                if s.set == other {
                    s.set = id;
                }
            }
        }
        self.put_in_set(t, id);
        self.project.leds.prune_sets();
        self.led.selected_set = Some(id);
    }

    /// Reporte sur tout le groupe choisi le déplacement et la rotation qu'on vient de faire subir
    /// à l'un de ses membres (`before` : sa position et sa rotation avant).
    fn spread_to_set(&mut self, t: Target, before: ([f32; 3], [f32; 3])) {
        // Sélection multiple : tous les objets choisis suivent.
        if self.multi_count() > 1 && self.in_multi(t) {
            let Some((p1, r1)) = self.target_pos(t).zip(self.target_rot(t)) else { return };
            if let Some((pos, rot)) = self.target_mut(t) {
                *pos = before.0;
                *rot = before.1;
            }
            let d = [p1[0] - before.0[0], p1[1] - before.0[1], p1[2] - before.0[2]];
            let turn = (r1[0] - before.1[0] + 180.0).rem_euclid(360.0) - 180.0;
            self.with_multi(|l, id| {
                if d != [0.0; 3] {
                    l.move_set(id, d);
                }
                if turn.abs() > 1e-4 {
                    l.rotate_set(id, turn);
                }
            });
            return;
        }
        let Some(id) = self.led.selected_set.filter(|&id| id == self.target_set(t)) else { return };
        let Some((p1, r1)) = self.target_pos(t).zip(self.target_rot(t)) else { return };
        if let Some((pos, rot)) = self.target_mut(t) {
            *pos = before.0;
            *rot = before.1;
        }
        let d = [p1[0] - before.0[0], p1[1] - before.0[1], p1[2] - before.0[2]];
        if d != [0.0; 3] {
            self.project.leds.move_set(id, d);
        }
        let turn = (r1[0] - before.1[0] + 180.0).rem_euclid(360.0) - 180.0;
        if turn.abs() > 1e-4 {
            self.project.leds.rotate_set(id, turn);
        }
    }

    fn target_rot(&self, t: Target) -> Option<[f32; 3]> {
        match t {
            Target::Fixture(i) => self.project.leds.fixtures.get(i).map(|f| f.rot),
            Target::Surface(i) => self.project.leds.surfaces.get(i).map(|s| s.rot),
            Target::Dmx(i) => self.project.leds.dmx.get(i).map(|d| d.rot),
            Target::Laser(i) => self.project.leds.lasers.get(i).map(|l| l.rot),
        }
    }

    fn target_mut(&mut self, t: Target) -> Option<(&mut [f32; 3], &mut [f32; 3])> {
        match t {
            Target::Fixture(i) => self.project.leds.fixtures.get_mut(i).map(|f| (&mut f.pos, &mut f.rot)),
            Target::Surface(i) => self.project.leds.surfaces.get_mut(i).map(|s| (&mut s.pos, &mut s.rot)),
            Target::Dmx(i) => self.project.leds.dmx.get_mut(i).map(|d| (&mut d.pos, &mut d.rot)),
            Target::Laser(i) => self.project.leds.lasers.get_mut(i).map(|l| (&mut l.pos, &mut l.rot)),
        }
    }

    /// Onglet latéral LEDs.
    pub(super) fn leds_ui(&mut self, ui: &mut egui::Ui) {
        self.live_toggle(ui);
        let sec = self.section();
        sec.show(ui, 0, |ui| self.scene_ui(ui));
        sec.show(ui, 1, |ui| self.leds_install_ui(ui));
        sec.show(ui, 2, |ui| self.leds_controllers_ui(ui));
        sec.show(ui, 3, |ui| self.leds_output_ui(ui));

        self.push_confirm(ui.ctx());
    }

    /// Gros interrupteur de l'envoi en direct aux WLED.
    fn live_toggle(&mut self, ui: &mut egui::Ui) {
        let live = self.led.live;
        let (text, color) = if live {
            (with_icon(ic::WIFI_HIGH, "ENVOI AUX WLED : ACTIF"), ACCENT)
        } else {
            (with_icon(ic::CUBE, "SIMULATION (rien n'est envoyé)"), TEXT_WEAK)
        };
        let mut button = egui::Button::new(egui::RichText::new(text).size(14.0).strong().color(if live { BG } else { color }))
            .min_size(Vec2::new(ui.available_width(), 34.0));
        if live {
            button = button.fill(ACCENT);
        }
        let hover = if live {
            "Clic : couper l'envoi (les WLED reprennent leurs propres effets après quelques secondes)"
        } else {
            "Clic : envoyer les couleurs aux contrôleurs WLED (DDP). Testez d'abord dans le simulateur."
        };
        if ui.add(button).on_hover_text(hover).clicked() {
            self.led.live = !live;
        }
        // État de chaque contrôleur en direct.
        if live {
            let status = self.led.sender.status.lock().unwrap().clone();
            for c in &self.project.leds.controllers {
                let (msg, color) = match status.get(c.host.trim()) {
                    _ if !c.enabled => ("envoi coupé".to_owned(), TEXT_WEAK),
                    _ if c.host.trim().is_empty() => ("adresse IP à renseigner".to_owned(), WARN),
                    Some(Ok(t)) if t.elapsed().as_secs_f32() < 1.0 => ("reçoit les couleurs".to_owned(), ACCENT),
                    Some(Err(e)) => (e.clone(), theme::DANGER),
                    _ => ("en attente…".to_owned(), TEXT_WEAK),
                };
                ui.label(egui::RichText::new(format!("{} : {msg}", c.name)).small().color(color));
            }
        }
        ui.add_space(6.0);
    }

    /// Salle et décor : écrans (qui affichent la sortie) et volumes.
    /// Fiche du groupe d'objets choisi : position, rotation, échelle de tout le groupe.
    fn set_card(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.led.selected_set else {
            ui.label(
                egui::RichText::new("Maj+clic sur un deuxième objet (vue 3D ou liste) pour les grouper.").small().color(TEXT_WEAK),
            );
            return;
        };
        let setup = &mut self.project.leds;
        let (fs, ss) = setup.set_members(id);
        let pivot = setup.set_pivot(id);
        let mut ungroup = false;
        let mut single = false;
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(ic::STACK).color(ACCENT));
                if let Some(g) = setup.sets.iter_mut().find(|g| g.id == id) {
                    ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(150.0));
                }
                ui.label(egui::RichText::new(format!("{} objet(s)", fs.len() + ss.len())).small().color(TEXT_WEAK));
            });
            ui.horizontal(|ui| {
                ui.label("Position");
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    let mut v = pivot[k];
                    if ui.add(egui::DragValue::new(&mut v).speed(0.01).prefix(format!("{axis} ")).suffix(" m")).changed() {
                        let mut d = [0.0; 3];
                        d[k] = v - pivot[k];
                        setup.move_set(id, d);
                    }
                }
            })
            .response
            .on_hover_text("Milieu du groupe, au niveau de son point le plus bas");
            ui.horizontal(|ui| {
                ui.label("Tourner");
                if ui.small_button("−15°").clicked() {
                    setup.rotate_set(id, -15.0);
                }
                if ui.small_button("+15°").clicked() {
                    setup.rotate_set(id, 15.0);
                }
                let mut deg = 0.0f32;
                if ui.add(egui::DragValue::new(&mut deg).speed(0.5).suffix("°")).on_hover_text("Glisser pour tourner").changed() {
                    setup.rotate_set(id, deg);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Taille");
                if ui.small_button("−10 %").clicked() {
                    setup.scale_set(id, 1.0 / 1.1);
                }
                if ui.small_button("+10 %").clicked() {
                    setup.scale_set(id, 1.1);
                }
                let mut pct = 100.0f32;
                if ui
                    .add(egui::DragValue::new(&mut pct).speed(0.3).suffix(" %"))
                    .on_hover_text("Glisser pour agrandir ou rétrécir tout le groupe (le nombre de LEDs ne change pas)")
                    .changed()
                {
                    setup.scale_set(id, pct / 100.0);
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Modifier un objet seul").on_hover_text("L'objet choisi bouge seul, le groupe reste").clicked() {
                    single = true;
                }
                if ui.button("Dégrouper").clicked() {
                    ungroup = true;
                }
            });
        });
        if single {
            self.led.selected_set = None;
        }
        if ungroup {
            let setup = &mut self.project.leds;
            setup.sets.retain(|g| g.id != id);
            setup.prune_sets();
            self.led.selected_set = None;
        }
        ui.add_space(4.0);
    }

    fn scene_ui(&mut self, ui: &mut egui::Ui) {
        let cursor3d = self.led.cursor3d;
        self.multi_scene_bar(ui);
        self.set_card(ui);
        let layers: Vec<(String, [f32; 4])> = self
            .project
            .layers
            .iter()
            .map(|l| {
                // Zone de la sortie occupée par le calque : sa déformation, réduite à son masque.
                let mut pts = l.warp.outline();
                if l.mask.enabled && !l.mask.invert {
                    pts = l.mask.all_rings().flatten().copied().collect();
                }
                let (mut lo, mut hi) = ([1.0f32; 2], [0.0f32; 2]);
                for p in &pts {
                    for k in 0..2 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
                let crop = [lo[0], lo[1], hi[0], hi[1]].map(|v| v.clamp(0.0, 1.0));
                (l.name.clone(), if crop[2] > crop[0] && crop[3] > crop[1] { crop } else { [0.0, 0.0, 1.0, 1.0] })
            })
            .collect();
        let setup = &mut self.project.leds;
        theme::section(ui, ic::CUBE, "Salle");
        ui.horizontal(|ui| {
            ui.label("Largeur");
            ui.add(egui::DragValue::new(&mut setup.room[0]).range(1.0..=100.0).speed(0.1).suffix(" m"));
            ui.label("Profondeur");
            ui.add(egui::DragValue::new(&mut setup.room[1]).range(1.0..=100.0).speed(0.1).suffix(" m"));
        });
        ui.add_space(6.0);
        theme::section(ui, ic::PROJECTOR_SCREEN, "Écrans et décor");
        ui.label(
            egui::RichText::new("Un écran affiche l'image de la sortie (ou la partie d'un calque mappé). Les volumes posent le décor.")
                .small()
                .color(TEXT_WEAK),
        );
        ui.horizontal_wrapped(|ui| {
            let presets = leds::Surface::presets(setup);
            for (name, mut s) in presets {
                if ui.button(with_icon(ic::PLUS, name)).clicked() {
                    // Au point d'ajout, s'il est posé.
                    if let Some(c) = cursor3d {
                        s.pos = c;
                    }
                    s.id = setup.next_surface_id();
                    let n = setup.surfaces.iter().filter(|o| o.name.starts_with(name)).count() + 1;
                    s.name = format!("{name} {n}");
                    setup.surfaces.push(s);
                    self.led.selected_surface = Some(setup.surfaces.len() - 1);
                    self.led.selected = None;
                }
            }
        });

        let mut action: Option<(usize, bool)> = None;
        for (i, s) in setup.surfaces.iter().enumerate() {
            let sel = self.led.selected_surface == Some(i);
            ui.horizontal(|ui| {
                let icon = match s.kind {
                    leds::SurfaceKind::Screen => ic::PROJECTOR_SCREEN,
                    leds::SurfaceKind::Booth => ic::DISC,
                    leds::SurfaceKind::Crowd => ic::USERS,
                    leds::SurfaceKind::Panels => ic::STACK,
                    leds::SurfaceKind::Sub | leds::SurfaceKind::Top => ic::WAVES,
                    leds::SurfaceKind::Truss => ic::GRID_FOUR,
                    leds::SurfaceKind::Sun => ic::SUN,
                    leds::SurfaceKind::Projector => ic::PROJECTOR_SCREEN,
                    _ => ic::CUBE,
                };
                let text = egui::RichText::new(with_icon(icon, &s.name));
                if ui.selectable_label(sel, if sel { text.strong() } else { text }).on_hover_text("Maj+clic : grouper avec l'objet choisi").clicked() {
                    self.led.pending_click = Some((Target::Surface(i), ui.input(|i| i.modifiers.shift)));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(theme::icon_button(ic::TRASH).small().frame(false)).on_hover_text("Supprimer").clicked() {
                        action = Some((i, false));
                    }
                    if ui.add(theme::icon_button(ic::COPY).small().frame(false)).on_hover_text("Dupliquer").clicked() {
                        action = Some((i, true));
                    }
                });
            });
        }
        match action {
            Some((i, false)) => {
                let id = setup.surfaces.remove(i).id;
                if setup.image_on == Some(id) {
                    setup.image_on = None;
                }
                self.led.selected_surface = None;
            }
            Some((i, true)) => {
                let mut s = setup.surfaces[i].clone();
                s.id = setup.next_surface_id();
                s.name = format!("{} (copie)", s.name);
                s.pos[0] += s.size[0] + 0.2;
                setup.surfaces.insert(i + 1, s);
                self.led.selected_surface = Some(i + 1);
            }
            None => {}
        }

        let Some(si) = self.led.selected_surface.filter(|&i| i < setup.surfaces.len()) else {
            if !setup.surfaces.is_empty() {
                ui.label(egui::RichText::new("Choisissez un objet (ici ou dans la vue 3D) pour le régler.").small().color(TEXT_WEAK));
            }
            return;
        };
        ui.add_space(6.0);
        let out_names: Vec<String> = self.project.outputs.iter().map(|o| o.name.clone()).collect();
        let mut panel_leds: Option<(String, Vec<(f32, Vec<[f32; 3]>)>)> = None;
        let mut detach_sun = false;
        let controllers: Vec<(u32, String)> = setup.controllers.iter().map(|c| (c.id, c.name.clone())).collect();
        let chips_ctrl = &mut self.led.chips_ctrl;
        let image_on = &mut setup.image_on;
        // Dessus de la scène sous l'objet choisi (pour l'y poser).
        let here = setup.surfaces[si].pos;
        let stage_top = setup
            .surfaces
            .iter()
            .enumerate()
            .filter(|(i, o)| *i != si && o.kind == leds::SurfaceKind::Stage && o.covers(here[0], here[2]))
            .map(|(_, o)| o.top())
            .reduce(f32::max);
        let s = &mut setup.surfaces[si];
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label("Nom");
                ui.text_edit_singleline(&mut s.name);
            });
            ui.horizontal(|ui| {
                ui.label("Type");
                egui::ComboBox::from_id_salt("scene_kind").selected_text(s.kind.name()).show_ui(ui, |ui| {
                    for k in leds::SurfaceKind::ALL {
                        ui.selectable_value(&mut s.kind, k, k.name());
                    }
                });
                ui.color_edit_button_srgb(&mut s.color).on_hover_text("Couleur");
            });
            if s.kind == leds::SurfaceKind::Projector {
                ui.horizontal(|ui| {
                    ui.label("Rapport de projection");
                    ui.add(egui::DragValue::new(&mut s.throw).range(0.2..=10.0).speed(0.01))
                        .on_hover_text("Distance / largeur de l'image (voir la notice du projecteur : 1,5 = à 3 m, image de 2 m)");
                    ui.label(egui::RichText::new(format!("à 5 m : {:.2} m de large", 5.0 / s.throw.max(0.2))).small().color(TEXT_WEAK));
                });
            }
            if s.kind == leds::SurfaceKind::Top {
                ui.horizontal(|ui| {
                    ui.label("Hauteur du pied");
                    ui.add(egui::DragValue::new(&mut s.stand).range(0.0..=5.0).speed(0.01).suffix(" m"));
                    if let Some(top) = stage_top {
                        if ui.small_button("Poser sur la scène").clicked() {
                            s.pos[1] = top;
                        }
                    }
                });
            }
            if s.kind == leds::SurfaceKind::Truss {
                ui.label(egui::RichText::new("Taille : L = portée, H = hauteur, P = section du treillis. Accrochez-y lasers, derbys ou bandeaux (Maj+clic pour les grouper avec le pont).").small().color(TEXT_WEAK));
            }
            if s.kind == leds::SurfaceKind::Crowd {
                ui.horizontal(|ui| {
                    ui.label("Personnes");
                    ui.add(egui::DragValue::new(&mut s.people).range(1..=400));
                });
                percent_slider(ui, &mut s.dance, "Énergie de la danse");
                ui.label(
                    egui::RichText::new("Ils dansent sur le tempo de l'onglet Rythme (tapez T pour le donner).").small().color(TEXT_WEAK),
                );
            }
            if s.kind == leds::SurfaceKind::Panels {
                let p = &mut s.panels;
                ui.horizontal(|ui| {
                    ui.label("Panneaux");
                    ui.add(egui::DragValue::new(&mut p.count).range(1..=60));
                    ui.label("Écart");
                    ui.add(egui::DragValue::new(&mut p.gap).range(0.0..=0.5).speed(0.005).suffix(" m"))
                        .on_hover_text("Écart entre deux couches de panneaux : la place des LEDs");
                });
                percent_slider(ui, &mut p.overlap, "Recouvrement");
                percent_slider(ui, &mut p.low, "Hauteur au centre");
                fit_slider(ui, "Ondulation", egui::Slider::new(&mut p.wave, 0.0..=0.2).suffix(" m"));
                ui.horizontal(|ui| {
                    ui.label("Dégradé");
                    ui.color_edit_button_srgb(&mut p.inner).on_hover_text("Couleur du centre");
                    ui.label("→");
                    ui.color_edit_button_srgb(&mut p.outer).on_hover_text("Couleur des bords");
                });
                if p.sun > 0.01 && ui
                    .button(with_icon(ic::SUN, "Séparer le soleil"))
                    .on_hover_text("Le soleil devient un objet à part (bouton « Soleil » de la scène), au même endroit")
                    .clicked()
                {
                    detach_sun = true;
                }
                // Contrôleur de chaque côté (les bandes peuvent aller sur des contrôleurs différents).
                for side in 0..2 {
                    if !controllers.iter().any(|c| c.0 == chips_ctrl[side]) {
                        chips_ctrl[side] = controllers.first().map_or(0, |c| c.0);
                    }
                }
                ui.horizontal(|ui| {
                    for (side, label) in [(0, "Gauche"), (1, "Droite")] {
                        ui.label(label);
                        let shown = controllers.iter().find(|c| c.0 == chips_ctrl[side]).map_or("aucun".to_owned(), |c| c.1.clone());
                        egui::ComboBox::from_id_salt(("chips_ctrl", side)).selected_text(shown).width(110.0).show_ui(ui, |ui| {
                            for (id, name) in &controllers {
                                ui.selectable_value(&mut chips_ctrl[side], *id, name);
                            }
                        });
                    }
                })
                .response
                .on_hover_text("Contrôleur des bandes de chaque côté : chaque bande prend la sortie suivante, du centre vers le bord");
                if ui
                    .button(with_icon(ic::LIGHTBULB, "Poser les LEDs entre les panneaux"))
                    .on_hover_text("Une bande derrière le bord de chaque panneau, qui éclaire le panneau de derrière (60 LEDs/m). Remplace les bandes déjà posées sur cet objet.")
                    .clicked()
                {
                    panel_leds = Some((s.name.clone(), s.panel_led_paths()));
                }
            }
            if s.kind == leds::SurfaceKind::Booth {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.dj, "DJ derrière la cabine");
                    if let Some(top) = stage_top {
                        if ui.small_button("Poser sur la scène").clicked() {
                            s.pos[1] = top;
                        }
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("Taille");
                ui.add(egui::DragValue::new(&mut s.size[0]).range(0.01..=100.0).speed(0.01).prefix("L ").suffix(" m"))
                    .on_hover_text("Largeur");
                ui.add(egui::DragValue::new(&mut s.size[1]).range(0.01..=100.0).speed(0.01).prefix("H ").suffix(" m"))
                    .on_hover_text(if s.kind == leds::SurfaceKind::Stage { "Hauteur du plateau" } else { "Hauteur" });
                if s.kind != leds::SurfaceKind::Screen {
                    ui.add(egui::DragValue::new(&mut s.size[2]).range(0.01..=100.0).speed(0.01).prefix("P ").suffix(" m"));
                }
            });
            if s.kind == leds::SurfaceKind::Screen && ui.small_button("Proportions de la sortie (16:9)").clicked() {
                let crop_aspect = (s.crop[2] - s.crop[0]).max(0.01) / (s.crop[3] - s.crop[1]).max(0.01);
                s.size[1] = s.size[0] / (16.0 / 9.0 * crop_aspect);
            }
            ui.horizontal(|ui| {
                ui.label("Position");
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.add(egui::DragValue::new(&mut s.pos[k]).speed(0.01).prefix(format!("{axis} ")).suffix(" m"));
                }
            })
            .response
            .on_hover_text("Milieu du bas de l'objet. X : gauche/droite · Y : hauteur · Z : de l'avant vers le fond");
            ui.horizontal(|ui| {
                ui.label("Rotation");
                for (k, name) in ["cap", "incl.", "roulis"].iter().enumerate() {
                    ui.add(egui::DragValue::new(&mut s.rot[k]).speed(1.0).range(-360.0..=360.0).prefix(format!("{name} ")).suffix("°"));
                }
            });
            ui.separator();
            if s.kind == leds::SurfaceKind::Crowd {
                s.show_output = false;
            } else {
                ui.horizontal(|ui| {
                    let label = if s.kind == leds::SurfaceKind::Projector { "Projette" } else { "Affiche l'image de" };
                    ui.checkbox(&mut s.show_output, label);
                    let shown = out_names.get(s.output).cloned().unwrap_or_else(|| "la sortie".into());
                    egui::ComboBox::from_id_salt("surface_out").selected_text(shown).width(110.0).show_ui(ui, |ui| {
                        for (k, name) in out_names.iter().enumerate() {
                            ui.selectable_value(&mut s.output, k, name);
                        }
                    });
                })
                .response
                .on_hover_text("Quelle sortie cet écran (ou cet élément mappé) montre, face avant");
            }
            if s.show_output {
                ui.horizontal(|ui| {
                    ui.label("Partie montrée");
                    egui::ComboBox::from_id_salt("scene_crop").selected_text("Choisir…").show_ui(ui, |ui| {
                        if ui.selectable_label(false, "Toute la sortie").clicked() {
                            s.crop = [0.0, 0.0, 1.0, 1.0];
                        }
                        for (name, crop) in &layers {
                            if ui.selectable_label(false, format!("Zone du calque « {name} »")).clicked() {
                                s.crop = *crop;
                            }
                        }
                    });
                });
                ui.horizontal(|ui| {
                    for (k, name) in ["G", "H", "D", "B"].iter().enumerate() {
                        let mut v = s.crop[k] * 100.0;
                        if ui
                            .add(egui::DragValue::new(&mut v).range(0.0..=100.0).speed(0.5).prefix(format!("{name} ")).suffix(" %"))
                            .changed()
                        {
                            s.crop[k] = v / 100.0;
                        }
                    }
                })
                .response
                .on_hover_text("Bords de la partie de la sortie affichée (gauche, haut, droite, bas)");
            }
            let on = *image_on == Some(s.id);
            let mut cal = on;
            ui.checkbox(&mut cal, "Caler l'image des LEDs sur cet écran")
                .on_hover_text("Les LEDs prolongent l'image de l'écran : chacune prend le point de l'image qui lui fait face");
            if cal != on {
                *image_on = cal.then_some(s.id);
            }
        });
        if let Some((name, paths)) = panel_leds {
            self.place_panel_leds(&name, &paths);
        }
        if detach_sun {
            // Même place qu'avant : devant le panneau du centre, à la hauteur du soleil des chips.
            let setup = &mut self.project.leds;
            let c = setup.surfaces[si].clone();
            let (d, h, low, gap) = (c.panels.sun, c.size[1], c.panels.low, c.panels.gap);
            let r = d / 2.0;
            let cy = (h * low - r * 0.75).max(r);
            let id = setup.next_surface_id();
            setup.surfaces.push(leds::Surface {
                id,
                name: "Soleil".into(),
                kind: leds::SurfaceKind::Sun,
                pos: leds::place(c.pos, c.rot, [0.0, cy - r, -gap]),
                rot: c.rot,
                size: [d, d, 0.02],
                color: c.panels.sun_color,
                show_output: false,
                set: c.set,
                ..leds::Surface::default()
            });
            setup.surfaces[si].panels.sun = 0.0;
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new(with_icon(ic::LIGHTNING, "Lasers, derbys, bulles, fumée : onglet DMX (à droite).")).small().color(TEXT_WEAK));
    }

    /// Bandes LED des panneaux superposés : une par tracé, nommées G1, G2… et D1, D2… du centre
    /// vers les bords ; chaque côté sur son contrôleur, une sortie par bande (dans l'ordre).
    /// Les anciennes bandes du même objet sont remplacées.
    fn place_panel_leds(&mut self, name: &str, paths: &[(f32, Vec<[f32; 3]>)]) {
        let surface = self.led.selected_surface;
        let ctrls = self.led.chips_ctrl;
        let setup = &mut self.project.leds;
        let prefix = format!("{name} · LED ");
        setup.fixtures.retain(|f| !f.name.starts_with(&prefix));
        // Prochaine sortie libre de chaque contrôleur.
        let mut next_out: HashMap<u32, usize> = HashMap::new();
        for (side, letter, ctrl) in [(-1.0f32, "G", ctrls[0]), (1.0, "D", ctrls[1])] {
            let ctrl = setup.controller_index(ctrl).map_or(setup.controllers.first().map_or(0, |c| c.id), |_| ctrl);
            let outputs = setup.controllers.iter().find(|c| c.id == ctrl).map_or(1, |c| c.pins.len().max(1));
            // Du centre vers le bord : à gauche, les tracés arrivent du bord vers le centre.
            let mut strips: Vec<&Vec<[f32; 3]>> = paths.iter().filter(|p| p.0 == side).map(|p| &p.1).collect();
            if side < 0.0 {
                strips.reverse();
            }
            for (k, pts) in strips.into_iter().enumerate() {
                let id = setup.next_fixture_id();
                let mut f = Fixture::new(FixtureKind::Path, id, format!("{prefix}{letter}{}", k + 1));
                f.set_path(pts, false);
                f.count = ((f.path_length() * 60.0).round() as u32).clamp(2, leds::MAX_COUNT);
                f.controller = ctrl;
                let o = next_out.entry(ctrl).or_insert(0);
                f.output = *o % outputs;
                *o += 1;
                setup.fixtures.push(f);
            }
        }
        // Les panneaux et leurs bandes forment un groupe d'objets : ils bougent ensemble.
        if let Some(si) = surface.filter(|&i| i < setup.surfaces.len()) {
            let mut id = setup.surfaces[si].set;
            if id == 0 {
                id = setup.next_set_id();
                setup.sets.push(leds::LedGroup { id, name: name.to_owned(), effect: None });
                setup.surfaces[si].set = id;
            }
            for f in setup.fixtures.iter_mut().filter(|f| f.name.starts_with(&prefix)) {
                f.set = id;
            }
            self.led.selected_set = Some(id);
        }
        // Premières bandes : groupes en paires gauche/droite, du centre vers les bords (pour les scénarios).
        if setup.groups.is_empty() {
            setup.auto_groups(leds::GroupOrder::CenterOut);
        }
        self.led.selected = None;
    }

    fn leds_install_ui(&mut self, ui: &mut egui::Ui) {
        if self.led.selected_set.is_some() {
            self.set_card(ui);
        }
        if !self.project.leds.fixtures.is_empty() {
            self.wiring_ui(ui);
        }
        self.tunnel_ui(ui);
        let mut start = false;
        let mut redraw = None;
        self.leds_install_body(ui, &mut start, &mut redraw);
        if start {
            self.start_drawing(None);
        }
        if redraw.is_some() {
            self.start_drawing(redraw);
        }
        if let Some(target) = self.led.sketch_req.take() {
            self.open_sketch(target);
        }
    }

    fn leds_install_body(&mut self, ui: &mut egui::Ui, start: &mut bool, redraw: &mut Option<usize>) {
        let setup = &mut self.project.leds;
        theme::section(ui, ic::LIGHTBULB, "Éléments LED");
        ui.horizontal_wrapped(|ui| {
            for kind in FixtureKind::ALL {
                if kind == FixtureKind::Path {
                    if ui
                        .button(with_icon(ic::PENCIL_SIMPLE, "Dessiner en 2D"))
                        .on_hover_text("Dessinez la forme du bandeau à plat, en mètres, sur la planche 2D, puis ajoutez-la à la scène")
                        .clicked()
                    {
                        self.led.sketch_req = Some(None);
                    }
                    if ui
                        .button(with_icon(ic::PENCIL_SIMPLE, "Dessiner en 3D"))
                        .on_hover_text("Dessinez le parcours point par point dans la vue 3D, au sol ou sur les murs")
                        .clicked()
                    {
                        *start = true;
                    }
                    continue;
                }

                if ui.button(with_icon(ic::PLUS, kind.name())).clicked() {
                    let id = setup.next_fixture_id();
                    let n = setup.fixtures.iter().filter(|f| f.kind == kind).count() + 1;
                    let mut f = Fixture::new(kind, id, format!("{} {n}", kind.name()));
                    // Branché sur le contrôleur et la sortie de l'élément choisi (ou du dernier).
                    if let Some(prev) = self.led.selected.and_then(|s| setup.fixtures.get(s)).or(setup.fixtures.last()) {
                        f.controller = prev.controller;
                        f.output = prev.output;
                        if kind == FixtureKind::Column && prev.kind == FixtureKind::Column {
                            f.pos = [prev.pos[0] + 0.5, prev.pos[1], prev.pos[2]];
                        }
                    } else if let Some(c) = setup.controllers.first() {
                        f.controller = c.id;
                    }
                    if let Some(c) = self.led.cursor3d {
                        f.pos = c;
                    }
                    setup.fixtures.push(f);
                    self.led.selected = Some(setup.fixtures.len() - 1);
                    self.led.selected_surface = None;
                }
            }
        });
        if setup.fixtures.is_empty() {
            ui.label(egui::RichText::new("Pour commencer vite :").small().color(TEXT_WEAK));
            if ui.button("Exemple : 8 colonnes de 30 LEDs").clicked() {
                setup.columns_example();
            }
            return;
        }

        // Sélection multiple : actions sur tous les bandeaux choisis.
        self.led.multi.retain(|id| setup.fixtures.iter().any(|f| f.id == *id));
        if self.led.multi.len() > 1 {
            multi_bar(ui, setup, &mut self.led.multi);
        }
        // Liste des éléments (glisser la poignée pour changer l'ordre du fil).
        let mut action: Option<(usize, u8)> = None;
        let mut rows: Vec<Rect> = Vec::new();
        let mut click: Option<(usize, Modifiers)> = None;
        for (i, f) in setup.fixtures.iter().enumerate() {
            let sel = self.led.selected == Some(i) || self.led.multi.contains(&f.id);
            let row = ui.horizontal(|ui| {
                let grip = ui
                    .add(egui::Label::new(egui::RichText::new(ic::LIST).color(TEXT_WEAK)).sense(Sense::drag()))
                    .on_hover_text("Glisser pour changer l'ordre (les bandeaux choisis glissent ensemble)")
                    .on_hover_cursor(egui::CursorIcon::Grab);
                if grip.drag_started() {
                    let mut items: Vec<usize> = if self.led.multi.contains(&f.id) && self.led.multi.len() > 1 {
                        (0..setup.fixtures.len()).filter(|&k| self.led.multi.contains(&setup.fixtures[k].id)).collect()
                    } else {
                        vec![i]
                    };
                    items.sort_unstable();
                    self.led.list_drag = Some((0, items));
                }
                let ctrl = setup.controller_index(f.controller);
                let plug = ctrl.map_or("non branché".to_owned(), |c| format!("{} · sortie {}", setup.controllers[c].name, f.output + 1));
                let text = egui::RichText::new(format!("{}  ({} LEDs)", f.name, f.led_count()));
                let several = setup.controllers.len() > 1;
                if ui
                    .selectable_label(sel, if sel { text.strong() } else { text })
                    .on_hover_text(format!("{plug}\nCmd+clic : ajouter à la sélection · Maj+clic : sélectionner une plage"))
                    .clicked()
                {
                    click = Some((i, ui.input(|i| i.modifiers)));
                }
                // Plusieurs contrôleurs : on voit tout de suite où chaque bandeau est branché.
                if several {
                    let tag = ctrl.map_or("—".to_owned(), |c| format!("{} S{}", setup.controllers[c].name, f.output + 1));
                    ui.label(egui::RichText::new(tag).small().color(TEXT_WEAK));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(theme::icon_button(ic::TRASH).small().frame(false)).on_hover_text("Supprimer").clicked() {
                        action = Some((i, 0));
                    }
                    if ui.add(theme::icon_button(ic::COPY).small().frame(false)).on_hover_text("Dupliquer").clicked() {
                        action = Some((i, 1));
                    }
                    if ui
                        .add(theme::icon_button(ic::LIGHTNING).small().frame(false))
                        .on_hover_text("Faire clignoter (dans le simulateur, et sur la vraie installation si le contrôleur a une adresse)")
                        .clicked()
                    {
                        action = Some((i, 2));
                    }
                    if ui
                        .add(theme::icon_button(ic::ARROWS_CLOCKWISE).small().frame(false))
                        .on_hover_text("Inverser le sens (le signal arrive par l'autre bout)")
                        .clicked()
                    {
                        action = Some((i, 5));
                    }
                    if ui.add(theme::icon_button(ic::CARET_DOWN).small().frame(false)).on_hover_text("Plus loin sur le fil").clicked() {
                        action = Some((i, 4));
                    }
                    if ui.add(theme::icon_button(ic::CARET_UP).small().frame(false)).on_hover_text("Plus tôt sur le fil").clicked() {
                        action = Some((i, 3));
                    }
                });
            });
            rows.push(row.response.rect);
        }
        // Clic : seul, Cmd (ajouter / retirer), Maj (plage depuis le dernier clic).
        if let Some((i, mods)) = click {
            let id = setup.fixtures[i].id;
            if mods.command {
                if self.led.multi.is_empty() {
                    if let Some(cur) = self.led.selected.and_then(|s| setup.fixtures.get(s)) {
                        self.led.multi.push(cur.id);
                    }
                }
                if let Some(k) = self.led.multi.iter().position(|&m| m == id) {
                    self.led.multi.remove(k);
                } else {
                    self.led.multi.push(id);
                }
                self.led.selected = Some(i);
                self.led.anchor = Some(i);
            } else if mods.shift {
                let a = self.led.anchor.or(self.led.selected).unwrap_or(i).min(setup.fixtures.len() - 1);
                let (lo, hi) = (a.min(i), a.max(i));
                self.led.multi = setup.fixtures[lo..=hi].iter().map(|f| f.id).collect();
                self.led.selected = Some(i);
            } else {
                self.led.pending_click = Some((Target::Fixture(i), false));
            }
            self.led.selected_surface = None;
        }
        // Glisser-déposer : ligne d'insertion, puis déplacement au lâcher.
        if let Some((0, items)) = self.led.list_drag.clone() {
            if let Some(at) = drop_slot(ui, &rows, &items) {
                let ids: Vec<u32> = items.iter().filter_map(|&k| setup.fixtures.get(k).map(|f| f.id)).collect();
                let primary = self.led.selected.and_then(|s| setup.fixtures.get(s)).map(|f| f.id);
                leds::move_block(&mut setup.fixtures, &items, at);
                self.led.selected = primary.and_then(|p| setup.fixtures.iter().position(|f| f.id == p));
                if ids.len() > 1 {
                    self.led.multi = ids;
                }
                self.led.list_drag = None;
            }
        }
        match action {
            Some((i, 0)) => {
                setup.fixtures.remove(i);
                self.led.selected = None;
            }
            Some((i, 1)) => {
                let mut f = setup.fixtures[i].clone();
                f.id = setup.next_fixture_id();
                f.name = format!("{} (copie)", f.name);
                f.pos[0] += 0.5;
                setup.fixtures.insert(i + 1, f);
                self.led.selected = Some(i + 1);
            }
            Some((i, 3)) if i > 0 => {
                setup.fixtures.swap(i, i - 1);
                self.led.selected = Some(i - 1);
            }
            Some((i, 4)) if i + 1 < setup.fixtures.len() => {
                setup.fixtures.swap(i, i + 1);
                self.led.selected = Some(i + 1);
            }
            Some((i, 5)) => {
                setup.fixtures[i].reverse = !setup.fixtures[i].reverse;
                self.led.selected = Some(i);
            }
            Some((_, 3 | 4)) => {}
            Some((i, _)) => {
                self.led.flash = Some((i, self.start.elapsed().as_secs_f64()));
                self.led.selected = Some(i);
            }
            None => {}
        }

        // Réglages de l'élément choisi.
        let Some(si) = self.led.selected.filter(|&s| s < setup.fixtures.len()) else {
            ui.label(egui::RichText::new("Choisissez un élément (ici ou dans le simulateur) pour le régler.").small().color(TEXT_WEAK));
            return;
        };
        ui.add_space(6.0);
        let groups: Vec<(u32, String)> = setup.groups.iter().map(|g| (g.id, g.name.clone())).collect();
        let controllers: Vec<(u32, String, usize)> =
            setup.controllers.iter().map(|c| (c.id, c.name.clone(), c.pins.len())).collect();
        let pins: HashMap<u32, Vec<Option<u8>>> = setup.controllers.iter().map(|c| (c.id, c.pins.clone())).collect();
        let f = &mut setup.fixtures[si];
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label("Nom");
                ui.text_edit_singleline(&mut f.name);
            });
            ui.horizontal(|ui| {
                ui.label("Type");
                egui::ComboBox::from_id_salt("led_kind").selected_text(f.kind.name()).show_ui(ui, |ui| {
                    for k in FixtureKind::ALL {
                        ui.selectable_value(&mut f.kind, k, k.name());
                    }
                });
                ui.checkbox(&mut f.reverse, "Câblé à l'envers").on_hover_text("Le fil arrive par l'autre bout");
            });
            if !groups.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("Groupe");
                    let current = groups.iter().find(|g| g.0 == f.group).map_or("aucun".to_owned(), |g| g.1.clone());
                    egui::ComboBox::from_id_salt("led_group").selected_text(current).show_ui(ui, |ui| {
                        ui.selectable_value(&mut f.group, 0, "aucun");
                        for (id, name) in &groups {
                            ui.selectable_value(&mut f.group, *id, name);
                        }
                    });
                })
                .response
                .on_hover_text("Groupe de bandeaux pour les scénarios (onglet Sortie)");
            }
            ui.horizontal(|ui| {
                ui.label("Contrôleur");
                let current = controllers.iter().find(|c| c.0 == f.controller).map_or("aucun".to_owned(), |c| c.1.clone());
                egui::ComboBox::from_id_salt("led_ctrl").selected_text(current).show_ui(ui, |ui| {
                    for (id, name, _) in &controllers {
                        ui.selectable_value(&mut f.controller, *id, name);
                    }
                    ui.selectable_value(&mut f.controller, 0, "aucun (simulation)");
                });
                let outputs = controllers.iter().find(|c| c.0 == f.controller).map_or(1, |c| c.2);
                let pin_of = |o: usize| pins.get(&f.controller).and_then(|p| p.get(o).copied().flatten());
                let out_name = |o: usize| match pin_of(o) {
                    Some(p) => format!("Sortie {} (pin {p})", o + 1),
                    None => format!("Sortie {}", o + 1),
                };
                egui::ComboBox::from_id_salt("led_out").selected_text(out_name(f.output)).show_ui(ui, |ui| {
                    for o in 0..outputs {
                        ui.selectable_value(&mut f.output, o, out_name(o));
                    }
                });
            });
            ui.horizontal(|ui| match f.kind {
                FixtureKind::Matrix => {
                    ui.label("LEDs");
                    ui.add(egui::DragValue::new(&mut f.cols).range(1..=128).suffix(" col."));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut f.rows).range(1..=128).suffix(" lignes"));
                    ui.checkbox(&mut f.serpentine, "Zigzag");
                }
                _ => {
                    ui.label("LEDs");
                    ui.add(egui::DragValue::new(&mut f.count).range(1..=leds::MAX_COUNT));
                }
            });
            fn m(v: &mut f32, lo: f32) -> egui::DragValue<'_> {
                egui::DragValue::new(v).range(lo..=100.0).speed(0.01).suffix(" m")
            }
            ui.horizontal(|ui| match f.kind {
                FixtureKind::Column => {
                    ui.label("Hauteur");
                    ui.add(m(&mut f.size[0], 0.05));
                }
                FixtureKind::Strip => {
                    ui.label("Longueur");
                    ui.add(m(&mut f.size[0], 0.05));
                }
                FixtureKind::Ring => {
                    ui.label("Rayon");
                    ui.add(m(&mut f.size[0], 0.02));
                }
                FixtureKind::Matrix => {
                    ui.label("Taille");
                    ui.add(m(&mut f.size[0], 0.02));
                    ui.label("×");
                    ui.add(m(&mut f.size[1], 0.02));
                }
                FixtureKind::Spiral => {
                    ui.label("Rayon");
                    ui.add(m(&mut f.size[0], 0.02));
                    ui.label("Hauteur");
                    ui.add(m(&mut f.size[1], 0.05));
                    ui.label("Tours");
                    ui.add(egui::DragValue::new(&mut f.turns).range(0.1..=100.0).speed(0.05));
                }
                FixtureKind::Path => {
                    let len = f.path_length();
                    ui.label(format!("{:.2} m · {} points", len, f.points.len()));
                    if len > 0.01 {
                        ui.label(egui::RichText::new(format!("{:.0} LEDs/m", f.count as f32 / len)).color(TEXT_WEAK));
                    }
                }
            });
            if f.kind == FixtureKind::Path {
                ui.horizontal_wrapped(|ui| {
                    let len = f.path_length();
                    for d in [30, 60, 96, 144] {
                        if ui.small_button(format!("{d}/m")).on_hover_text("Densité du bandeau (LEDs par mètre)").clicked() && len > 0.01 {
                            f.count = ((len * d as f32).round() as u32).clamp(2, leds::MAX_COUNT);
                        }
                    }
                    ui.checkbox(&mut f.closed, "Boucle fermée");
                    if f.is_flat() {
                        if ui.button(with_icon(ic::PENCIL_SIMPLE, "Modifier en 2D")).on_hover_text("Ouvre sa forme sur la planche 2D").clicked() {
                            self.led.sketch_req = Some(Some(si));
                        }
                    } else if ui.button(with_icon(ic::PENCIL_SIMPLE, "Redessiner")).clicked() {
                        *redraw = Some(si);
                    }
                });
                if f.is_flat() {
                    // Orientation de la forme dessinée en 2D.
                    ui.horizontal(|ui| {
                        ui.label("Poser");
                        if ui.small_button("Debout, face à la salle").clicked() {
                            f.rot = [0.0, 0.0, 0.0];
                        }
                        if ui.small_button("Au sol").clicked() {
                            f.rot = [0.0, -90.0, 0.0];
                            f.pos[1] = 0.02;
                        }
                        if ui.small_button("Au plafond").clicked() {
                            f.rot = [0.0, 90.0, 0.0];
                            f.pos[1] = 2.5;
                        }
                    });
                }
                egui::CollapsingHeader::new(format!("Points du tracé ({})", f.points.len())).id_salt("path_pts").show(ui, |ui| {
                    let mut remove = None;
                    for (k, p) in f.points.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{}", k + 1)).small().color(TEXT_WEAK));
                            for (a, axis) in ["X", "Y", "Z"].iter().enumerate() {
                                ui.add(egui::DragValue::new(&mut p[a]).speed(0.01).prefix(format!("{axis} ")).suffix(" m"));
                            }
                            if ui.small_button(ic::X).on_hover_text("Retirer ce point").clicked() {
                                remove = Some(k);
                            }
                        });
                    }
                    if let Some(k) = remove.filter(|_| f.points.len() > 2) {
                        f.points.remove(k);
                    }
                    ui.label(egui::RichText::new("Coordonnées relatives à la position de l'élément.").small().color(TEXT_WEAK));
                });
            }
            let in_room = true;
            if in_room {
                ui.horizontal(|ui| {
                    ui.label("Position");
                    for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                        ui.add(egui::DragValue::new(&mut f.pos[k]).speed(0.01).prefix(format!("{axis} ")).suffix(" m"));
                    }
                })
                .response
                .on_hover_text("X : gauche/droite · Y : hauteur · Z : de l'avant vers le fond de la salle");
            }
            if in_room && f.kind != FixtureKind::Column {
                ui.horizontal(|ui| {
                    ui.label("Rotation");
                    for (k, name) in ["cap", "incl.", "roulis"].iter().enumerate() {
                        ui.add(egui::DragValue::new(&mut f.rot[k]).speed(1.0).range(-360.0..=360.0).prefix(format!("{name} ")).suffix("°"));
                    }
                });
                if f.kind == FixtureKind::Strip {
                    ui.horizontal(|ui| {
                        if ui.small_button("Horizontale").clicked() {
                            f.rot = [0.0, 0.0, 0.0];
                        }
                        if ui.small_button("Verticale").clicked() {
                            f.rot = [0.0, 0.0, 90.0];
                        }
                        if ui.small_button("Vers le fond").clicked() {
                            f.rot = [-90.0, 0.0, 0.0];
                        }
                    });
                }
                if matches!(f.kind, FixtureKind::Ring | FixtureKind::Matrix) && ui.small_button("À plat au sol / au plafond").clicked() {
                    f.rot = [0.0, 90.0, 0.0];
                }
            }
        });
    }

    fn leds_controllers_ui(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, ic::WIFI_HIGH, "Contrôleurs WLED");
        ui.label(
            egui::RichText::new(
                "Un ESP32 avec WLED par contrôleur. Adresse IP : visible dans l'appli WLED ou sur la box. \
                 Les couleurs partent en DDP (port 4048) ; WLED doit être sur le même réseau.",
            )
            .small()
            .color(TEXT_WEAK),
        );
        let jobs = self.led.jobs.lock().unwrap().clone();
        let mut remove = None;
        let n_ctrl = self.project.leds.controllers.len();
        for ci in 0..n_ctrl {
            let layout = self.project.leds.layout(ci);
            let attached = self.project.leds.fixtures.iter().filter(|f| f.controller == self.project.leds.controllers[ci].id).count();
            let c = &mut self.project.leds.controllers[ci];
            let id = c.id;
            theme::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut c.name);
                    ui.checkbox(&mut c.enabled, "Direct").on_hover_text("Reçoit les couleurs quand l'envoi aux WLED est actif");
                });
                ui.horizontal(|ui| {
                    ui.label("Adresse");
                    ui.add(egui::TextEdit::singleline(&mut c.host).hint_text("192.168.1.50").desired_width(140.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Modèle");
                    egui::ComboBox::from_id_salt(("led_model", id)).selected_text("Choisir…").show_ui(ui, |ui| {
                        for (name, pins) in leds::MODELS {
                            if ui.selectable_label(false, name).clicked() {
                                c.pins = pins.to_vec();
                            }
                        }
                    });
                    ui.label("Couleurs");
                    egui::ComboBox::from_id_salt(("led_order", id)).width(60.0).selected_text(leds::ORDERS[c.order]).show_ui(ui, |ui| {
                        for (k, o) in leds::ORDERS.iter().enumerate() {
                            ui.selectable_value(&mut c.order, k, *o);
                        }
                    });
                });
                // Pins des sorties (-1 : inconnue).
                ui.horizontal_wrapped(|ui| {
                    ui.label("Pins");
                    for (o, pin) in c.pins.iter_mut().enumerate() {
                        let mut v = pin.map_or(-1, i32::from);
                        ui.label(egui::RichText::new(format!("S{}", o + 1)).small().color(TEXT_WEAK));
                        if ui
                            .add(egui::DragValue::new(&mut v).range(-1..=48).custom_formatter(|v, _| if v < 0.0 { "?".into() } else { format!("{v}") }))
                            .changed()
                        {
                            *pin = u8::try_from(v).ok();
                        }
                    }
                    if ui.small_button(ic::PLUS).on_hover_text("Ajouter une sortie").clicked() && c.pins.len() < 10 {
                        c.pins.push(None);
                    }
                    if c.pins.len() > 1 && ui.small_button("−").on_hover_text("Retirer la dernière sortie").clicked() {
                        c.pins.pop();
                    }
                });
                // Répartition des LEDs.
                if layout.outputs.is_empty() {
                    ui.label(egui::RichText::new("Aucun élément branché").small().color(TEXT_WEAK));
                } else {
                    let outs: Vec<String> =
                        layout.outputs.iter().map(|(o, s, n)| format!("S{} : {}–{}", o + 1, s, s + n - 1)).collect();
                    ui.label(egui::RichText::new(format!("{} LEDs  ·  {}", layout.total, outs.join("  ·  "))).small().color(TEXT_WEAK));
                }
                if let Some(info) = self.led.infos.get(&id) {
                    ui.label(egui::RichText::new(with_icon(ic::INFO, info)).small().color(ACCENT));
                }
                if let Some(msg) = jobs.get(&id) {
                    let color = if msg.starts_with("Erreur") { theme::DANGER } else { WARN };
                    ui.label(egui::RichText::new(msg).small().color(color));
                }
                ui.horizontal(|ui| {
                    let has_host = !c.host.trim().is_empty();
                    if ui
                        .add_enabled(has_host, egui::Button::new(with_icon(ic::DOWNLOAD_SIMPLE, "Lire")))
                        .on_hover_text("Lit le nom, la version, les sorties et les pins du contrôleur")
                        .clicked()
                    {
                        let host = c.host.clone();
                        let (jobs, reads) = (self.led.jobs.clone(), self.led.reads.clone());
                        jobs.lock().unwrap().insert(id, "Lecture…".into());
                        std::thread::spawn(move || match leds::read_controller(&host) {
                            Ok(info) => {
                                jobs.lock().unwrap().remove(&id);
                                reads.lock().unwrap().push((id, info));
                            }
                            Err(e) => {
                                jobs.lock().unwrap().insert(id, format!("Erreur : {e:#}"));
                            }
                        });
                    }
                    if ui
                        .add_enabled(has_host && !layout.outputs.is_empty(), egui::Button::new(with_icon(ic::UPLOAD_SIMPLE, "Envoyer la configuration…")))
                        .on_hover_text("Écrit dans WLED les sorties (pins, nombre de LEDs, ordre des couleurs), redémarre le contrôleur et crée un segment par élément")
                        .clicked()
                    {
                        self.led.confirm_push = Some(id);
                    }
                    if attached == 0
                        && n_ctrl > 1
                        && ui.add(theme::icon_button(ic::TRASH).small().frame(false)).on_hover_text("Supprimer ce contrôleur").clicked()
                    {
                        remove = Some(ci);
                    }
                });
            });
            ui.add_space(4.0);
        }
        if let Some(ci) = remove {
            self.project.leds.controllers.remove(ci);
        }
        if ui.button(with_icon(ic::PLUS, "Ajouter un contrôleur")).clicked() {
            let id = self.project.leds.next_controller_id();
            self.project.leds.controllers.push(leds::Controller::new(id, &format!("ESP32 {id}"), 0));
        }
        ui.add_space(8.0);
        self.plugs_ui(ui);
    }

    /// Confirmation avant d'écrire la configuration dans un WLED (il redémarre).
    fn push_confirm(&mut self, ctx: &egui::Context) {
        let Some(id) = self.led.confirm_push else { return };
        let Some(ci) = self.project.leds.controller_index(id) else {
            self.led.confirm_push = None;
            return;
        };
        let setup = &self.project.leds;
        let c = setup.controllers[ci].clone();
        let layout = setup.layout(ci);
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("led_push_confirm")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            theme::section(ui, ic::UPLOAD_SIMPLE, &format!("Configurer {} ?", c.name));
            ui.label(format!(
                "Le contrôleur {} va recevoir {} sortie(s) et {} LEDs, ordre des couleurs {}, puis redémarrer. \
                 Ses sorties actuelles seront remplacées.",
                c.host.trim(),
                layout.outputs.len(),
                layout.total,
                leds::ORDERS[c.order]
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button("Envoyer")).clicked() {
                    answer = Some(true);
                }
                if ui.button("Annuler").clicked() {
                    answer = Some(false);
                }
            });
        });
        let Some(ok) = answer else { return };
        self.led.confirm_push = None;
        if !ok {
            return;
        }
        let names: Vec<(String, bool)> =
            layout.fixtures.iter().map(|&(fi, _, _)| (setup.fixtures[fi].name.clone(), setup.fixtures[fi].reverse)).collect();
        let jobs = self.led.jobs.clone();
        jobs.lock().unwrap().insert(id, "Démarrage…".into());
        std::thread::spawn(move || {
            let progress = |m: String| {
                jobs.lock().unwrap().insert(id, m);
            };
            let res = leds::push_config(c.host.trim(), &c, &layout, &names, &progress);
            jobs.lock().unwrap().insert(id, res.unwrap_or_else(|e| format!("Erreur : {e:#}")));
        });
    }

    fn leds_output_ui(&mut self, ui: &mut egui::Ui) {
        let out_names: Vec<String> = self.project.outputs.iter().map(|o| o.name.clone()).collect();
        let setup = &mut self.project.leds;
        theme::section(ui, ic::PLAY, "Ce que montrent les LEDs");
        // Sortie suivie (image ou couleur de la vidéo), quand il y en a plusieurs.
        if out_names.len() > 1 {
            ui.horizontal(|ui| {
                ui.label(with_icon(ic::MONITOR, "Sortie suivie"));
                let shown = out_names.get(setup.video_output).cloned().unwrap_or_default();
                egui::ComboBox::from_id_salt("led_video_out_main").selected_text(shown).show_ui(ui, |ui| {
                    for (k, name) in out_names.iter().enumerate() {
                        ui.selectable_value(&mut setup.video_output, k, name);
                    }
                });
            })
            .response
            .on_hover_text("Sortie dont l'image (ou la couleur) est suivie par les LEDs ; chaque groupe peut en suivre une autre");
        }
        for s in Source::ALL {
            ui.radio_value(&mut setup.source, s, s.name());
            if s == Source::Video && setup.source == Source::Video {
                ui.indent("led_proj", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Calée sur");
                        let current = setup
                            .image_on
                            .and_then(|id| setup.surfaces.iter().find(|s| s.id == id))
                            .map_or("l'installation des LEDs".to_owned(), |s| format!("l'écran « {} »", s.name));
                        egui::ComboBox::from_id_salt("led_image_on").selected_text(current).show_ui(ui, |ui| {
                            ui.selectable_value(&mut setup.image_on, None, "l'installation des LEDs");
                            for s in setup.surfaces.iter().filter(|s| s.kind == leds::SurfaceKind::Screen) {
                                ui.selectable_value(&mut setup.image_on, Some(s.id), format!("l'écran « {} »", s.name));
                            }
                        });
                    });
                    if setup.image_on.is_none() {
                        ui.horizontal(|ui| {
                            ui.label("Image vue");
                            for p in Projection::ALL {
                                ui.selectable_value(&mut setup.projection, p, p.name());
                            }
                        });
                        ui.checkbox(&mut setup.stretch, "Étirer l'image sur toute l'installation");
                    }
                    ui.label(
                        egui::RichText::new("Les cercles sur l'aperçu de la sortie montrent où chaque LED prend sa couleur.")
                            .small()
                            .color(TEXT_WEAK),
                    );
                });
            }
            if s == Source::Scenario && setup.source == Source::Scenario {
                ui.indent("led_scenario", |ui| scenario_ui(ui, setup));
            }
            if s == Source::Solid && setup.source == Source::Solid {
                ui.indent("led_color", |ui| {
                    ui.color_edit_button_srgb(&mut setup.test_color);
                });
            }
        }
        ui.add_space(6.0);
        ui.add_space(6.0);
        // Réaction au son : par-dessus n'importe quel effet.
        theme::section(ui, ic::WAVES, "Réaction au son");
        let pulse = &self.rhythm.pulse;
        let lv = if setup.sound.bass { pulse.bass } else { pulse.level }.clamp(0.0, 1.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut setup.sound.bass, true, "Basses (kick)");
            ui.selectable_value(&mut setup.sound.bass, false, "Tout le son");
            // Niveau en direct.
            let (r, _) = ui.allocate_exact_size(Vec2::new(90.0, 8.0), Sense::hover());
            ui.painter().rect_filled(r, 4.0, theme::WIDGET);
            ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width() * lv, r.height())), 4.0, ACCENT);
        });
        percent_slider(ui, &mut setup.sound.follow, "Suit le volume");
        percent_slider(ui, &mut setup.sound.flash, "Flash sur les temps");
        ui.label(
            egui::RichText::new("Le son vient de l'onglet Rythme (micro, carte son ; ou le tempo tapé avec T). Marche avec tous les effets, image de la vidéo comprise.")
                .small()
                .color(TEXT_WEAK),
        );
        if setup.sound.follow > 0.0 || setup.sound.flash > 0.0 {
            ui.ctx().request_repaint();
        }
        ui.add_space(6.0);
        groups_ui(ui, setup, &mut self.led.list_drag, &out_names);
        ui.add_space(6.0);
        theme::section(ui, ic::SLIDERS_HORIZONTAL, "Réglages");
        percent_slider(ui, &mut setup.brightness, "Luminosité");
        fit_slider(ui, "Images/s", egui::Slider::new(&mut setup.fps, 5..=60));
        ui.label(
            egui::RichText::new("Le blackout (B) éteint aussi les LEDs. Si vous coupez l'envoi, WLED reprend ses propres effets au bout de quelques secondes.")
                .small()
                .color(TEXT_WEAK),
        );
    }
}

/// Nom court d'un effet, pour les listes de groupes.
fn short_source(s: Source) -> &'static str {
    match s {
        Source::Video => "Image de la sortie",
        Source::VideoAverage => "Couleur de la vidéo",
        Source::Scenario => "Scénario",
        Source::Solid => "Couleur unie",
        Source::Rainbow => "Arc-en-ciel",
        Source::Off => "Éteint",
        other => other.name(),
    }
}

/// Glisser-déposer dans une liste : dessine la ligne d'insertion sous le pointeur et renvoie
/// le rang d'insertion quand on lâche (`None` tant qu'on glisse).
fn drop_slot(ui: &mut egui::Ui, rows: &[Rect], items: &[usize]) -> Option<usize> {
    if rows.is_empty() {
        return Some(0);
    }
    let pointer = ui.ctx().pointer_interact_pos();
    let at = pointer.map_or(rows.len(), |p| rows.iter().position(|r| p.y < r.center().y).unwrap_or(rows.len()));
    let painter = ui.painter();
    for &k in items {
        if let Some(r) = rows.get(k) {
            painter.rect_filled(r.expand(1.0), 4.0, ACCENT.gamma_multiply(0.18));
        }
    }
    let y = if at < rows.len() { rows[at].top() - 2.0 } else { rows[rows.len() - 1].bottom() + 2.0 };
    let x = rows[0].x_range();
    painter.line_segment([egui::pos2(x.min, y), egui::pos2(x.max, y)], Stroke::new(2.0, ACCENT));
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    ui.ctx().request_repaint();
    (!ui.input(|i| i.pointer.primary_down())).then_some(at)
}

/// Barre d'actions sur plusieurs bandeaux choisis ensemble.
fn multi_bar(ui: &mut egui::Ui, setup: &mut LedSetup, multi: &mut Vec<u32>) {
    let ids = multi.clone();
    let picked = |f: &Fixture| ids.contains(&f.id);
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} bandeaux choisis", ids.len())).strong().color(ACCENT));
            if ui.small_button("Désélectionner").clicked() {
                multi.clear();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.small_button(with_icon(ic::ARROWS_CLOCKWISE, "Inverser le sens")).clicked() {
                for f in setup.fixtures.iter_mut().filter(|f| picked(f)) {
                    f.reverse = !f.reverse;
                }
            }
            // Brancher tous les bandeaux choisis sur une même sortie (ils s'y suivent dans l'ordre de la liste).
            let outputs: Vec<((u32, usize), String)> = setup
                .controllers
                .iter()
                .flat_map(|c| (0..c.pins.len().max(1)).map(move |o| ((c.id, o), format!("{} · sortie {}", c.name, o + 1))))
                .collect();
            let mut plug = None;
            egui::ComboBox::from_id_salt("multi_plug").selected_text("Brancher sur…").width(150.0).show_ui(ui, |ui| {
                for (key, name) in &outputs {
                    if ui.selectable_label(false, name).clicked() {
                        plug = Some(*key);
                    }
                }
            });
            if let Some((c, o)) = plug {
                for f in setup.fixtures.iter_mut().filter(|f| picked(f)) {
                    f.controller = c;
                    f.output = o;
                }
            }
            if ui
                .small_button(with_icon(ic::PLUS, "Groupe de bandeaux"))
                .on_hover_text("Un nouveau groupe pour les scénarios : ces bandeaux s'allumeront ensemble")
                .clicked()
            {
                let id = setup.next_group_id();
                setup.groups.push(leds::LedGroup { id, name: format!("Groupe {id}"), effect: None });
                for f in setup.fixtures.iter_mut().filter(|f| picked(f)) {
                    f.group = id;
                }
            }
            if ui
                .small_button(with_icon(ic::STACK, "Grouper dans la scène"))
                .on_hover_text("Ils se déplacent, tournent et se redimensionnent ensemble")
                .clicked()
            {
                let id = setup.next_set_id();
                setup.sets.push(leds::LedGroup { id, name: format!("Groupe d'objets {id}"), effect: None });
                for f in setup.fixtures.iter_mut().filter(|f| picked(f)) {
                    f.set = id;
                }
                setup.prune_sets();
            }
            if ui.small_button(with_icon(ic::TRASH, "Supprimer")).clicked() {
                setup.fixtures.retain(|f| !picked(f));
                multi.clear();
            }
        });
    });
}

/// Réglages du scénario et groupes de bandeaux (dans l'ordre d'animation).
fn scenario_ui(ui: &mut egui::Ui, setup: &mut LedSetup) {
    let sc = &mut setup.scenario;
    ui.horizontal(|ui| {
        ui.label("Scénario");
        ui.add_enabled_ui(!sc.chain, |ui| {
            egui::ComboBox::from_id_salt("led_scenario_kind").selected_text(sc.kind.name()).show_ui(ui, |ui| {
                for k in leds::ScenarioKind::ALL {
                    ui.selectable_value(&mut sc.kind, k, k.name());
                }
            });
        });
    });
    fit_slider(ui, "Vitesse", egui::Slider::new(&mut sc.speed, 0.1..=4.0).logarithmic(true).suffix("×"));
    ui.checkbox(&mut sc.sync, "Sur le tempo (onglet Rythme)")
        .on_hover_text("Les battements, vagues et allumages suivent le tempo tapé ou détecté. Sinon : 120 BPM.");
    if sc.kind == leds::ScenarioKind::Comets && !sc.chain {
        ui.horizontal(|ui| {
            ui.label("Comètes");
            ui.add(egui::DragValue::new(&mut sc.comets).range(1..=32));
        });
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut sc.chain, "Enchaîner au hasard, toutes les");
        ui.add_enabled(sc.chain, egui::DragValue::new(&mut sc.chain_secs).range(3.0..=600.0).suffix(" s"));
    });

}

/// Groupes de bandeaux : ordre d'animation des scénarios, et effet propre à chaque groupe.
fn groups_ui(ui: &mut egui::Ui, setup: &mut LedSetup, drag: &mut Option<(u8, Vec<usize>)>, out_names: &[String]) {
    theme::section(ui, ic::STACK, "Groupes de bandeaux");
    ui.label(
        egui::RichText::new(
            "Les bandeaux d'un groupe s'allument ensemble ; les scénarios passent d'un groupe au suivant dans cet ordre. \
             Chaque groupe peut avoir son propre effet. Sans groupe, chaque bandeau est son propre groupe.",
        )
        .small()
        .color(TEXT_WEAK),
    );
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Du centre vers les bords").on_hover_text("Paires symétriques gauche/droite, du centre vers l'extérieur").clicked() {
            setup.auto_groups(leds::GroupOrder::CenterOut);
        }
        if ui.small_button("De gauche à droite").clicked() {
            setup.auto_groups(leds::GroupOrder::LeftRight);
        }
        if ui.small_button("De l'avant vers le fond").on_hover_text("Rangée par rangée en profondeur : l'effet tunnel").clicked() {
            setup.auto_groups(leds::GroupOrder::FrontBack);
        }
        if !setup.groups.is_empty() && ui.small_button("Aucun groupe").clicked() {
            setup.groups.clear();
            for f in &mut setup.fixtures {
                f.group = 0;
            }
        }
        if ui.small_button(with_icon(ic::PLUS, "Groupe")).clicked() {
            let id = setup.next_group_id();
            setup.groups.push(leds::LedGroup { id, name: format!("Groupe {id}"), effect: None });
        }
    });
    let mut action: Option<(usize, i32)> = None;
    let n = setup.groups.len();
    let mut rows = Vec::new();
    for (i, g) in setup.groups.iter_mut().enumerate() {
        let members = setup.fixtures.iter().filter(|f| f.group == g.id).count();
        let row = ui.horizontal(|ui| {
            let grip = ui
                .add(egui::Label::new(egui::RichText::new(ic::LIST).color(TEXT_WEAK)).sense(Sense::drag()))
                .on_hover_text("Glisser pour changer l'ordre d'animation")
                .on_hover_cursor(egui::CursorIcon::Grab);
            if grip.drag_started() {
                *drag = Some((1, vec![i]));
            }
            ui.label(egui::RichText::new(format!("{}", i + 1)).small().color(TEXT_WEAK));
            ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(110.0));
            ui.label(egui::RichText::new(format!("{members} élém.")).small().color(TEXT_WEAK));
            // Effet du groupe : l'effet général, ou le sien.
            let shown = g.effect.as_ref().map_or("Effet général", |e| short_source(e.source));
            egui::ComboBox::from_id_salt(("group_fx", g.id)).selected_text(shown).width(120.0).show_ui(ui, |ui| {
                if ui.selectable_label(g.effect.is_none(), "Effet général").clicked() {
                    g.effect = None;
                }
                for src in [Source::Video, Source::VideoAverage, Source::Scenario, Source::Solid, Source::Rainbow, Source::Off] {
                    let on = g.effect.as_ref().is_some_and(|e| e.source == src);
                    if ui.selectable_label(on, short_source(src)).clicked() {
                        let e = g.effect.get_or_insert_with(leds::GroupEffect::default);
                        e.source = src;
                    }
                }
            })
            .response
            .on_hover_text("Ce groupe peut suivre l'effet général, ou en avoir un à lui");
            if i > 0 && ui.small_button(ic::CARET_UP).on_hover_text("Plus tôt").clicked() {
                action = Some((i, -1));
            }
            if i + 1 < n && ui.small_button(ic::CARET_DOWN).on_hover_text("Plus tard").clicked() {
                action = Some((i, 1));
            }
            if ui.small_button(ic::X).on_hover_text("Supprimer le groupe").clicked() {
                action = Some((i, 0));
            }
        });
        rows.push(row.response.rect);
        // Réglages de l'effet propre au groupe.
        if let Some(e) = &mut g.effect {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                if matches!(e.source, Source::Video | Source::VideoAverage) && out_names.len() > 1 {
                    let shown = e.output.and_then(|k| out_names.get(k).cloned()).unwrap_or_else(|| "Sortie suivie".into());
                    egui::ComboBox::from_id_salt(("group_out", g.id)).selected_text(shown).width(110.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut e.output, None, "Sortie suivie (générale)");
                        for (k, name) in out_names.iter().enumerate() {
                            ui.selectable_value(&mut e.output, Some(k), name);
                        }
                    });
                }
                match e.source {
                    Source::Scenario => {
                        egui::ComboBox::from_id_salt(("group_sc", g.id)).selected_text(e.scenario.name()).width(150.0).show_ui(ui, |ui| {
                            for k in leds::ScenarioKind::ALL {
                                ui.selectable_value(&mut e.scenario, k, k.name());
                            }
                        });
                    }
                    Source::Solid => {
                        ui.color_edit_button_srgb(&mut e.color);
                    }
                    _ => {}
                }
                ui.add(egui::Slider::new(&mut e.brightness, 0.0..=1.0).show_value(false)).on_hover_text("Luminosité du groupe");
                ui.checkbox(&mut e.sound, "Son").on_hover_text("Réagit au son (réglages « Réaction au son »)");
            });
        }
    }
    if let Some((1, items)) = drag.clone() {
        if let Some(at) = drop_slot(ui, &rows, &items) {
            leds::move_block(&mut setup.groups, &items, at);
            *drag = None;
        }
    }
    match action {
        Some((i, 0)) => {
            let id = setup.groups.remove(i).id;
            for f in &mut setup.fixtures {
                if f.group == id {
                    f.group = 0;
                }
            }
        }
        Some((i, d)) => setup.groups.swap(i, (i as i32 + d) as usize),
        None => {}
    }
    if !setup.groups.is_empty() {
        let loose = setup.fixtures.iter().filter(|f| !setup.groups.iter().any(|g| g.id == f.group)).count();
        if loose > 0 {
            ui.label(egui::RichText::new(format!("{loose} élément(s) sans groupe : éteints pendant le scénario")).small().color(WARN));
        }
    }
}

/// Projection perspective de la caméra du simulateur.
struct View {
    eye: [f32; 3],
    fwd: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    center: egui::Pos2,
    focal: f32,
}

impl View {
    fn new(cam: Orbit, rect: Rect) -> Self {
        let (cp, sp) = (cam.pitch.cos(), cam.pitch.sin());
        let eye = [
            cam.target[0] + cam.dist * cam.yaw.sin() * cp,
            cam.target[1] + cam.dist * sp,
            cam.target[2] - cam.dist * cam.yaw.cos() * cp,
        ];
        let fwd = norm(sub(cam.target, eye));
        let right = norm(cross([0.0, 1.0, 0.0], fwd));
        let up = cross(fwd, right);
        // Champ vertical de 50°.
        let focal = rect.height() * 0.5 / (25f32.to_radians()).tan();
        Self { eye, fwd, right, up, center: rect.center(), focal }
    }

    /// Point de la salle → point de l'écran et distance (None : derrière la caméra).
    fn project(&self, p: [f32; 3]) -> Option<(egui::Pos2, f32)> {
        let d = sub(p, self.eye);
        let z = dot(d, self.fwd);
        if z < 0.05 {
            return None;
        }
        let (x, y) = (dot(d, self.right), dot(d, self.up));
        Some((self.center + Vec2::new(x / z * self.focal, -y / z * self.focal), z))
    }

    fn line(&self, painter: &egui::Painter, a: [f32; 3], b: [f32; 3], stroke: Stroke) {
        if let Some(((pa, _), (pb, _))) = self.segment(a, b) {
            painter.line_segment([pa, pb], stroke);
        }
    }

    /// Segment de la salle à l'écran, coupé là où il passe derrière la caméra : un faisceau
    /// qui file vers la caméra (et finit dans son dos) reste visible jusqu'au bord de la vue.
    fn segment(&self, a: [f32; 3], b: [f32; 3]) -> Option<((egui::Pos2, f32), (egui::Pos2, f32))> {
        const NEAR: f32 = 0.08;
        let (za, zb) = (dot(sub(a, self.eye), self.fwd), dot(sub(b, self.eye), self.fwd));
        if za < NEAR && zb < NEAR {
            return None;
        }
        let cut = |p: [f32; 3], q: [f32; 3], zp: f32, zq: f32| {
            let t = (NEAR - zp) / (zq - zp);
            [0, 1, 2].map(|k| p[k] + (q[k] - p[k]) * t)
        };
        let a2 = if za < NEAR { cut(a, b, za, zb) } else { a };
        let b2 = if zb < NEAR { cut(b, a, zb, za) } else { b };
        Some((self.project(a2)?, self.project(b2)?))
    }

    /// Point d'un plan (passant par `p0`, de normale `n`) sous le pointeur.
    fn ray_plane(&self, screen: egui::Pos2, p0: [f32; 3], n: [f32; 3]) -> Option<[f32; 3]> {
        let s = (screen - self.center) / self.focal;
        let dir = [0, 1, 2].map(|i| self.fwd[i] + self.right[i] * s.x - self.up[i] * s.y);
        let den = dot(dir, n);
        if den.abs() < 1e-5 {
            return None;
        }
        let t = dot(sub(p0, self.eye), n) / den;
        (t > 0.0).then(|| [0, 1, 2].map(|i| self.eye[i] + dir[i] * t))
    }

    /// Point du plan horizontal de hauteur `y` sous le pointeur.
    fn ray_floor(&self, screen: egui::Pos2, y: f32) -> Option<[f32; 3]> {
        let s = (screen - self.center) / self.focal;
        let dir = [0, 1, 2].map(|i| self.fwd[i] + self.right[i] * s.x - self.up[i] * s.y);
        if dir[1].abs() < 1e-4 {
            return None;
        }
        let t = (y - self.eye[1]) / dir[1];
        (t > 0.0).then(|| [self.eye[0] + dir[0] * t, y, self.eye[2] + dir[2] * t])
    }
}

/// Halo lumineux qui s'ajoute à ce qu'il y a dessous (alpha prémultiplié nul : mélange additif),
/// au lieu de le recouvrir : une LED faible n'assombrit jamais une surface.
fn glow(c: [u8; 3], k: f32) -> Color32 {
    let v = |x: u8| (f32::from(x) * k).round() as u8;
    Color32::from_rgba_premultiplied(v(c[0]), v(c[1]), v(c[2]), 0)
}

/// Longueur d'une forme de la planche 2D (m).
fn sketch_length(pts: &[[f32; 2]], closed: bool) -> f32 {
    let mut l: f32 = pts.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum();
    if closed && pts.len() > 2 {
        let (a, b) = (pts[0], pts[pts.len() - 1]);
        l += ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    }
    l
}

/// Distance du premier quadrilatère touché par le rayon (origine `o`, direction unitaire `d`).
fn first_hit(o: [f32; 3], d: [f32; 3], quads: &[[[f32; 3]; 4]]) -> Option<f32> {
    let mut best: Option<f32> = None;
    for q in quads {
        let n = cross(sub(q[1], q[0]), sub(q[3], q[0]));
        let den = dot(n, d);
        if den.abs() < 1e-6 {
            continue;
        }
        let t = dot(n, sub(q[0], o)) / den;
        if t <= 0.05 || best.is_some_and(|b| t >= b) {
            continue;
        }
        let p = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
        // Dans le quadrilatère (convexe) : du même côté de ses quatre bords.
        let inside = (0..4).all(|k| dot(cross(sub(q[(k + 1) % 4], q[k]), sub(p, q[k])), n) >= -1e-6);
        if inside {
            best = Some(t);
        }
    }
    best
}

/// Distance d'un point à un segment (pixels).
fn seg_dist(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn elide_name(s: &str) -> String {
    if s.chars().count() <= 18 { s.to_owned() } else { s.chars().take(17).collect::<String>() + "…" }
}

/// Photo de fond du plan 2D (réduite à 2048 px au plus).
fn load_photo(ctx: &egui::Context, path: &std::path::Path) -> Option<egui::TextureHandle> {
    let img = image::open(path).ok()?.thumbnail(2048, 2048).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture("led_plan_photo", color, egui::TextureOptions::LINEAR))
}

/// Marque les LEDs cachées par une face : la face passe, à l'endroit précis de la LED à l'écran,
/// devant elle (le tri par morceau ne suffit pas pour des LEDs glissées entre deux panneaux).
fn hide_covered_leds(items: &mut [(f32, Item)]) {
    const CELL: f32 = 24.0;
    let tiles: Vec<([egui::Pos2; 4], [f32; 4])> = items
        .iter()
        .filter_map(|(_, it)| if let Item::Tile { pts, z, .. } = it { Some((*pts, *z)) } else { None })
        .collect();
    if tiles.is_empty() {
        return;
    }
    // Grille de cases à plat sur la zone des LEDs (seules elles sont testées), au plus 128 × 128
    // cases, rangée d'un seul tenant : rien à allouer case par case à chaque image.
    let mut lo = egui::pos2(f32::MAX, f32::MAX);
    let mut hi = egui::pos2(f32::MIN, f32::MIN);
    for (_, it) in items.iter() {
        let p = match it {
            Item::Led { p, .. } => *p,
            Item::Wire { a, b, .. } => egui::pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5),
            _ => continue,
        };
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if lo.x > hi.x || lo.y > hi.y {
        return;
    }
    let size = CELL.max((hi.x - lo.x).max(hi.y - lo.y) / 128.0);
    let gw = ((hi.x - lo.x) / size) as usize + 1;
    let gh = ((hi.y - lo.y) / size) as usize + 1;
    let span = |r: Rect| -> Option<(usize, usize, usize, usize)> {
        if r.max.x < lo.x || r.max.y < lo.y || r.min.x > hi.x || r.min.y > hi.y {
            return None;
        }
        let c = |v: f32, o: f32, n: usize| (((v - o) / size).max(0.0) as usize).min(n - 1);
        Some((c(r.min.x, lo.x, gw), c(r.min.y, lo.y, gh), c(r.max.x, lo.x, gw), c(r.max.y, lo.y, gh)))
    };
    let spans: Vec<Option<(usize, usize, usize, usize)>> = tiles.iter().map(|(pts, _)| span(Rect::from_points(pts))).collect();
    // Comptage, puis rangement (tableau d'index par case, comme une matrice creuse).
    let mut start = vec![0u32; gw * gh + 1];
    for (x0, y0, x1, y1) in spans.iter().flatten() {
        for cy in *y0..=*y1 {
            for cx in *x0..=*x1 {
                start[cy * gw + cx + 1] += 1;
            }
        }
    }
    for i in 1..start.len() {
        start[i] += start[i - 1];
    }
    let mut fill = start.clone();
    let mut list = vec![0u32; start[gw * gh] as usize];
    for (k, sp) in spans.iter().enumerate() {
        let Some((x0, y0, x1, y1)) = sp else { continue };
        for cy in *y0..=*y1 {
            for cx in *x0..=*x1 {
                let c = cy * gw + cx;
                list[fill[c] as usize] = k as u32;
                fill[c] += 1;
            }
        }
    }
    let covered = |p: egui::Pos2, z: f32, margin: f32| {
        let (x, y) = ((p.x - lo.x) / size, (p.y - lo.y) / size);
        if x < 0.0 || y < 0.0 || x as usize >= gw || y as usize >= gh {
            return false;
        }
        let c = y as usize * gw + x as usize;
        list[start[c] as usize..start[c + 1] as usize].iter().any(|&k| {
            let (pts, zs) = &tiles[k as usize];
            depth_at(pts, zs, p).is_some_and(|d| d < z - margin)
        })
    };
    for (z, it) in items.iter_mut() {
        match it {
            Item::Led { p, hidden, .. } => *hidden = covered(*p, *z, 0.005),
            // Un trait est caché si son milieu l'est (sa propre face, à la même distance, ne compte pas).
            Item::Wire { a, b, z: wz, hidden, .. } => *hidden = covered(egui::pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5), *wz, 0.015),
            Item::Tile { .. } | Item::Beam { .. } | Item::Puff { .. } => {}
        }
    }
}

/// Distance d'un morceau de face au point `p` de l'écran (interpolée en perspective), s'il le couvre.
fn depth_at(pts: &[egui::Pos2; 4], z: &[f32; 4], p: egui::Pos2) -> Option<f32> {
    for [a, b, c] in [[0, 1, 2], [0, 2, 3]] {
        let (pa, pb, pc) = (pts[a], pts[b], pts[c]);
        let det = (pb.y - pc.y) * (pa.x - pc.x) + (pc.x - pb.x) * (pa.y - pc.y);
        if det.abs() < 1e-6 {
            continue;
        }
        let u = ((pb.y - pc.y) * (p.x - pc.x) + (pc.x - pb.x) * (p.y - pc.y)) / det;
        let v = ((pc.y - pa.y) * (p.x - pc.x) + (pa.x - pc.x) * (p.y - pc.y)) / det;
        let w = 1.0 - u - v;
        if u >= -1e-4 && v >= -1e-4 && w >= -1e-4 {
            return Some(1.0 / (u / z[a] + v / z[b] + w / z[c]));
        }
    }
    None
}

/// Taille des morceaux de face (m).
const TILE: f32 = 0.4;

/// Découpe une face en morceaux (maillages sans anticrénelage : pas de pointes sur les faces
/// vues par la tranche, pas de jointures visibles) et ajoute ses arêtes, un peu en avant.
/// Vidéoprojecteur qui éclaire la scène : l'objet, l'image de sa sortie, ses proportions.
struct Beamer<'a> {
    surface: &'a leds::Surface,
    tex: egui::TextureId,
    aspect: f32,
}

#[allow(clippy::too_many_arguments)]
fn face_tiles(
    view: &View,
    q: [[f32; 3]; 4],
    fill: Color32,
    texture: Option<(egui::TextureId, [[f32; 2]; 4])>,
    edge: Stroke,
    edges: u8,
    surface: usize,
    items: &mut Vec<(f32, Item)>,
    beamers: &[Beamer],
) {
    let len = |a: [f32; 3], b: [f32; 3]| dot(sub(a, b), sub(a, b)).sqrt();
    // Vidéoprojecteurs tournés vers cette face (les autres ne l'éclairent pas).
    let n = norm(cross(sub(q[1], q[0]), sub(q[3], q[0])));
    let center = [0, 1, 2].map(|k| q.iter().map(|p| p[k]).sum::<f32>() / 4.0);
    let lit: Vec<&Beamer> = beamers
        .iter()
        .filter(|b| {
            let to = sub(b.surface.lens(), center);
            dot(n, to) > 0.0
        })
        .collect();
    // Une image (de la face ou projetée) garde des morceaux plus petits : la perspective reste juste.
    let tile = if texture.is_some() || !lit.is_empty() { TILE * 0.6 } else { TILE };
    let nu = ((len(q[0], q[1]) / tile).ceil() as usize).clamp(1, 48);
    let nv = ((len(q[0], q[3]) / tile).ceil() as usize).clamp(1, 48);
    let mut grid = Vec::with_capacity((nu + 1) * (nv + 1));
    for j in 0..=nv {
        for i in 0..=nu {
            let (s, t) = (i as f32 / nu as f32, j as f32 / nv as f32);
            let p = [0, 1, 2].map(|k| {
                let top = q[0][k] + (q[1][k] - q[0][k]) * s;
                let bottom = q[3][k] + (q[2][k] - q[3][k]) * s;
                top + (bottom - top) * t
            });
            let Some(pp) = view.project(p) else { return };
            grid.push((pp.0, pp.1, s, t, p));
        }
    }
    let at = |i: usize, j: usize| grid[j * (nu + 1) + i];
    for j in 0..nv {
        for i in 0..nu {
            let c = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
            let pts = c.map(|v| v.0);
            // Morceau vu par la tranche : rien à dessiner.
            let area: f32 = (0..4).map(|k| pts[k].x * pts[(k + 1) % 4].y - pts[(k + 1) % 4].x * pts[k].y).sum::<f32>() * 0.5;
            if area.abs() < 0.3 {
                continue;
            }
            let mut mesh = match texture {
                Some((t, _)) => egui::Mesh::with_texture(t),
                None => egui::Mesh::default(),
            };
            for v in c {
                match texture {
                    Some((_, uv)) => {
                        // Point de l'image : interpolé entre les coins de la face.
                        let (s, t) = (v.2, v.3);
                        let k = |i: usize| {
                            let top = uv[0][i] + (uv[1][i] - uv[0][i]) * s;
                            let bottom = uv[3][i] + (uv[2][i] - uv[3][i]) * s;
                            top + (bottom - top) * t
                        };
                        mesh.vertices.push(egui::epaint::Vertex { pos: v.0, uv: egui::pos2(k(0), k(1)), color: Color32::WHITE });
                    }
                    None => mesh.colored_vertex(v.0, fill),
                }
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            let depth = c.iter().map(|v| v.1).sum::<f32>() / 4.0;
            items.push((depth, Item::Tile { pts, z: c.map(|v| v.1), mesh, surface, quad: q }));
            // Lumière des vidéoprojecteurs : leur image, ajoutée à la couleur de la face.
            for b in &lit {
                let uvs = c.map(|v| b.surface.projected_uv(v.4, b.aspect));
                if uvs.iter().all(|u| u.is_none_or(|u| !(-0.05..=1.05).contains(&u[0]) || !(-0.05..=1.05).contains(&u[1]))) {
                    continue;
                }
                let mut over = egui::Mesh::with_texture(b.tex);
                for (v, uv) in c.iter().zip(uvs) {
                    let inside = uv.is_some_and(|u| (0.0..=1.0).contains(&u[0]) && (0.0..=1.0).contains(&u[1]));
                    let u = uv.unwrap_or([0.5, 0.5]);
                    // Mélange additif : la projection éclaire, elle ne recouvre pas.
                    let color = if inside { Color32::from_rgba_premultiplied(235, 235, 235, 0) } else { Color32::TRANSPARENT };
                    over.vertices.push(egui::epaint::Vertex { pos: v.0, uv: egui::pos2(u[0].clamp(0.0, 1.0), u[1].clamp(0.0, 1.0)), color });
                }
                over.add_triangle(0, 1, 2);
                over.add_triangle(0, 2, 3);
                items.push((depth - 0.002, Item::Tile { pts, z: c.map(|v| v.1), mesh: over, surface, quad: q }));
            }
        }
    }
    // Arêtes, découpées comme la face et un peu en avant d'elle.
    let mut lines = Vec::new();
    for i in 0..nu {
        if edges & 0b0001 != 0 {
            lines.push((at(i, 0), at(i + 1, 0)));
        }
        if edges & 0b0100 != 0 {
            lines.push((at(i, nv), at(i + 1, nv)));
        }
    }
    for j in 0..nv {
        if edges & 0b1000 != 0 {
            lines.push((at(0, j), at(0, j + 1)));
        }
        if edges & 0b0010 != 0 {
            lines.push((at(nu, j), at(nu, j + 1)));
        }
    }
    for (a, b) in lines {
        let z = (a.1 + b.1) * 0.5;
        items.push((z - 0.03, Item::Wire { a: a.0, b: b.0, stroke: edge, z, hidden: false }));
    }
}

/// Point dans un polygone convexe ou non (règle pair-impair).
fn inside(poly: &[egui::Pos2], p: egui::Pos2) -> bool {
    let mut odd = false;
    let mut j = poly.len().wrapping_sub(1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            odd = !odd;
        }
        j = i;
    }
    odd
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-6);
    a.map(|v| v / l)
}
