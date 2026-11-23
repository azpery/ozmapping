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
}

/// État des LEDs propre à la session (non enregistré).
pub(super) struct LedRuntime {
    /// Les couleurs partent vers les WLED. Coupé au lancement : on teste d'abord dans le simulateur.
    pub(super) live: bool,
    pub(super) sender: leds::Sender,
    /// Image de la sortie lue sur le GPU (remplie par le rendu).
    pub(super) frame: Arc<Mutex<leds::Frame>>,
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
    anchor: Option<usize>,
    /// Glisser-déposer en cours dans une liste (0 : bandeaux, 1 : groupes), rangs glissés.
    list_drag: Option<(u8, Vec<usize>)>,
    /// Contrôleurs (id) des bandes des chips : côté gauche, côté droit.
    chips_ctrl: [u32; 2],
    camera: Option<Orbit>,
    drag: Option<Drag>,
    /// Image de la sortie sur les écrans de la scène, et numéro de l'image affichée.
    screen_tex: Option<egui::TextureHandle>,
    tex_serial: u64,
    /// Tracé libre en cours de dessin dans la vue 3D.
    drawing: Option<Drawing>,
    /// Montre l'ordre des LEDs dans la vue 3D (début, sens, fil d'un bandeau au suivant).
    show_wiring: bool,
    /// Câblage à la souris en cours, et sortie proposée (contrôleur, sortie).
    wiring: Option<WireMode>,
    wire_target: (u32, usize),
    pub(super) view: LedView,
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
            frame: Arc::new(Mutex::new(leds::Frame::default())),
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
            anchor: None,
            list_drag: None,
            chips_ctrl: [0, 0],
            camera: None,
            drag: None,
            screen_tex: None,
            tex_serial: 0,
            drawing: None,
            show_wiring: false,
            wiring: None,
            wire_target: (0, 0),
            view: LedView::Both,
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
    /// `hidden` : cachée par une face (seul son halo se voit, comme la lumière qui déborde).
    Led { p: egui::Pos2, r: f32, c: [u8; 3], i: usize, hidden: bool },
}

/// Durée du clignotement de repérage.
const FLASH_SECS: f64 = 2.5;

impl OzApp {
    /// LEDs utiles en ce moment (l'image de la sortie doit alors être lue pour elles).
    fn leds_need_frame(&self) -> bool {
        let setup = &self.project.leds;
        let tab = self.right_tab == RightTab::Leds;
        (!setup.fixtures.is_empty() && setup.source == Source::Video && (self.led.live || tab))
            || (tab && self.led.view != LedView::Output && self.screens_show_output())
    }

    fn screens_show_output(&self) -> bool {
        self.project.leds.surfaces.iter().any(|s| s.show_output)
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

        if self.led.cache_of.as_ref() != Some(&self.project.leds) {
            self.led.leds = self.project.leds.leds();
            self.led.cache_of = Some(self.project.leds.clone());
            if self.led.selected.is_some_and(|s| s >= self.project.leds.fixtures.len()) {
                self.led.selected = None;
            }
            if self.led.selected_surface.is_some_and(|s| s >= self.project.leds.surfaces.len()) {
                self.led.selected_surface = None;
            }
        }
        let now = self.clock();
        if self.led.flash.is_some_and(|(_, t)| now - t > FLASH_SECS) {
            self.led.flash = None;
        }
        let flash = self.led.flash.map(|(f, t)| (f, (now - t) as f32));
        let level = 1.0 - self.blackout_level;
        self.led.colors = {
            let frame = self.led.frame.lock().unwrap();
            let music = leds::Music { beat: self.rhythm.tempo.beat_at(now) as f32, level: self.rhythm.pulse.level };
            leds::colors(&self.project.leds, &self.led.leds, &frame, now as f32, level, flash, music)
        };

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
        if !self.leds_need_frame() {
            return;
        }
        let Some(r) = self.renderer.clone() else { return };
        let aspect = (self.output_size.x / self.output_size.y.max(1.0)).clamp(0.2, 5.0);
        // Plus fine quand elle s'affiche sur les écrans du simulateur.
        let w = if self.right_tab == RightTab::Leds && self.screens_show_output() { 448.0 } else { 192.0 };
        let size = (w as u32, ((w / aspect) as u32).clamp(8, 1024));
        let rect = ui.max_rect();
        ui.painter().add(renderer::led_capture_callback(rect, r, self.draw_params(), size, self.led.frame.clone()));
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
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            ui.horizontal(|ui| {
                for (v, name) in [(LedView::Both, "Sortie + LEDs"), (LedView::Sim, "LEDs 3D"), (LedView::Output, "Sortie")] {
                    ui.selectable_value(&mut self.led.view, v, name);
                }
                ui.separator();
                let n = self.led.leds.len();
                let state = if self.led.live {
                    egui::RichText::new(with_icon(ic::WIFI_HIGH, &format!("{n} LEDs · envoi aux WLED"))).color(ACCENT)
                } else {
                    egui::RichText::new(format!("{n} LEDs · simulation seulement")).color(TEXT_WEAK)
                };
                ui.label(state);
            });
        });
        let body = Rect::from_min_max(egui::pos2(full.min.x, bar.max.y + 14.0), full.max);
        match self.led.view {
            LedView::Output => {
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| self.preview(ui));
            }
            LedView::Sim => self.led_sim(ui, body.shrink(14.0)),
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
        let cam = *self.led.camera.get_or_insert_with(|| Orbit::home(room));
        let resp = ui.interact(rect, ui.scope_id().with("led_sim"), Sense::click_and_drag());
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
        let tex = self.led.screen_tex.as_ref().map(|t| t.id());
        // Les personnages dansent sur le tempo de l'onglet Rythme.
        let beat = self.rhythm.tempo.beat_at(self.clock()) as f32;
        if self.project.leds.surfaces.iter().any(|s| s.kind == leds::SurfaceKind::Crowd || (s.kind == leds::SurfaceKind::Booth && s.dj)) {
            ui.ctx().request_repaint();
        }
        let set = self.led.selected_set.unwrap_or(u32::MAX);
        for (si, s) in self.project.leds.surfaces.iter().enumerate() {
            let sel = self.led.selected_surface == Some(si) || s.set == set;
            let edge = Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { ACCENT } else { Color32::from_black_alpha(200) });
            for f3 in s.faces(beat) {
                let face = f3.quad;
                let n = norm(cross(sub(face[1], face[0]), sub(face[3], face[0])));
                let center = [0, 1, 2].map(|i| face.iter().map(|p| p[i]).sum::<f32>() / 4.0);
                let facing = dot(n, sub(view.eye, center)) > 0.0;
                // Un volume ne montre que ses faces tournées vers nous ; un écran montre aussi son dos.
                if !facing && !matches!(s.kind, leds::SurfaceKind::Screen | leds::SurfaceKind::Panels) {
                    continue;
                }
                let shade = 0.55 + 0.45 * dot(if facing { n } else { n.map(|v| -v) }, light).max(0.0);
                let base = if facing { f3.color } else { [60, 62, 68] };
                let fill = Color32::from_rgb(
                    (f32::from(base[0]) * shade) as u8,
                    (f32::from(base[1]) * shade) as u8,
                    (f32::from(base[2]) * shade) as u8,
                );
                let textured = (f3.output && facing).then_some(tex).flatten();
                face_tiles(&view, face, fill, textured.map(|t| (t, f3.uv)), edge, f3.edges, si, &mut items);
            }
        }
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
                None => hovered_surface.map(Target::Surface),
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
        if resp.drag_started_by(egui::PointerButton::Primary) {
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
                self.select(d.target);
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
            } else {
                let d = resp.drag_delta();
                let cam = self.led.camera.as_mut().unwrap();
                cam.yaw -= d.x * 0.008;
                cam.pitch = (cam.pitch + d.y * 0.008).clamp(-0.2, 1.55);
            }
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
        if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
            let d = resp.drag_delta();
            let cam = self.led.camera.as_mut().unwrap();
            let k = cam.dist / view.focal;
            for i in 0..3 {
                cam.target[i] += (-view.right[i] * d.x + view.up[i] * d.y) * k;
            }
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let cam = self.led.camera.as_mut().unwrap();
                cam.dist = (cam.dist * (-scroll * 0.002).exp()).clamp(0.5, 80.0);
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
                Some(t) => self.select(t),
                None => {
                    self.led.selected = None;
                    self.led.selected_surface = None;
                }
            }
        }
        if resp.double_clicked() && hovered.is_none() && !drawing {
            self.led.camera = Some(Orbit::home(room));
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

        // Infos de la LED survolée.
        if let (Some((i, _)), Some(h)) = (hovered_led, hover) {
            let l = &self.led.leds[i];
            let f = &self.project.leds.fixtures[l.fixture];
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
                let cam = self.led.camera.as_mut().unwrap();
                for (name, yaw, pitch) in [("Face", 0.0, 0.12), ("Dessus", 0.0, 1.55), ("Côté", -1.5708, 0.12), ("3D", 0.5, 0.42)] {
                    if ui.small_button(name).clicked() {
                        *cam = Orbit { yaw, pitch, ..Orbit::home(room) };
                    }
                }
            });
        });
        painter.text(
            rect.left_bottom() + Vec2::new(8.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            "Glisser : tourner · clic droit : déplacer la vue · molette : zoom · objet : glisser (Maj : hauteur, Alt : tourner) · flèches, PgHaut PgBas : déplacer · Q E : tourner · Suppr · Cmd+D : dupliquer · D : dessiner · V : vue",
            egui::FontId::proportional(11.0),
            TEXT_WEAK,
        );
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

    /// Raccourcis de la scène 3D (onglet LEDs ouvert). Appelé avant les raccourcis généraux :
    /// les touches utilisées ici ne lancent pas aussi leur action habituelle.
    pub(super) fn scene_shortcuts(&mut self, ctx: &egui::Context) {
        if self.right_tab != RightTab::Leds || ctx.egui_wants_keyboard_input() || self.led.confirm_push.is_some() {
            return;
        }
        if self.led.wiring.is_some() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.led.wiring = None;
            return;
        }
        // Pendant un tracé, Entrée, Retour arrière et Échap appartiennent au dessin.
        if self.led.drawing.is_some() {
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
            *cam = Orbit { yaw, pitch, ..Orbit::home(room) };
        }
        let target = match (self.led.selected, self.led.selected_surface) {
            (Some(i), _) => Target::Fixture(i),
            (_, Some(i)) => Target::Surface(i),
            _ => return,
        };
        if key(Modifiers::NONE, Key::Escape) {
            self.led.selected = None;
            self.led.selected_surface = None;
            self.led.selected_set = None;
            self.led.multi.clear();
            return;
        }
        if key(Modifiers::NONE, Key::Delete) || key(Modifiers::NONE, Key::Backspace) {
            if self.led.multi.len() > 1 {
                // Tous les bandeaux choisis.
                let multi = std::mem::take(&mut self.led.multi);
                self.project.leds.fixtures.retain(|f| !multi.contains(&f.id));
                self.project.leds.prune_sets();
                self.led.selected = None;
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
                _ => {}
            }
            self.led.selected = None;
            self.led.selected_surface = None;
            return;
        }
        if key(Modifiers::COMMAND, Key::D) {
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
            (Key::ArrowUp, [fwd[0], 0.0, fwd[1]]),
            (Key::ArrowDown, [-fwd[0], 0.0, -fwd[1]]),
            (Key::PageUp, [0.0, 1.0, 0.0]),
            (Key::PageDown, [0.0, -1.0, 0.0]),
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
            if self.led.multi.len() > 1 {
                // Les bandeaux choisis bougent ensemble, comme un groupe provisoire.
                const TEMP: u32 = u32::MAX;
                let setup = &mut self.project.leds;
                let saved: Vec<(usize, u32)> = setup
                    .fixtures
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| self.led.multi.contains(&f.id))
                    .map(|(i, f)| (i, f.set))
                    .collect();
                for &(i, _) in &saved {
                    setup.fixtures[i].set = TEMP;
                }
                setup.move_set(TEMP, mv);
                if turn != 0.0 {
                    setup.rotate_set(TEMP, turn);
                }
                for (i, set) in saved {
                    setup.fixtures[i].set = set;
                }
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
                if self.led.view == LedView::Output {
                    self.led.view = LedView::Both;
                }
            }
        });
        ui.add_space(4.0);
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
        if self.led.view == LedView::Output {
            self.led.view = LedView::Both;
        }
    }

    /// Met à jour la texture de l'image de sortie affichée sur les écrans de la scène.
    fn update_screen_texture(&mut self, ctx: &egui::Context) {
        if !self.project.leds.surfaces.iter().any(|s| s.show_output) {
            return;
        }
        let frame = self.led.frame.lock().unwrap();
        if frame.serial == self.led.tex_serial || frame.size.0 == 0 {
            return;
        }
        self.led.tex_serial = frame.serial;
        let (w, h) = (frame.size.0 as usize, frame.size.1 as usize);
        if frame.pixels.len() < w * h * 4 {
            return;
        }
        // Lignes lues sur le GPU du bas vers le haut : on les remet dans l'ordre.
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in (0..h).rev() {
            rgba.extend_from_slice(&frame.pixels[row * w * 4..(row + 1) * w * 4]);
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
        match &mut self.led.screen_tex {
            Some(t) => t.set(image, egui::TextureOptions::LINEAR),
            None => self.led.screen_tex = Some(ctx.load_texture("led_scene_output", image, egui::TextureOptions::LINEAR)),
        }
    }

    fn select(&mut self, t: Target) {
        let set = self.target_set(t);
        self.led.selected_set = (set != 0).then_some(set);
        self.led.multi = match t {
            Target::Fixture(i) => self.project.leds.fixtures.get(i).map(|f| vec![f.id]).unwrap_or_default(),
            Target::Surface(_) => Vec::new(),
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
        }
    }

    fn target_pos(&self, t: Target) -> Option<[f32; 3]> {
        match t {
            Target::Fixture(i) => self.project.leds.fixtures.get(i).map(|f| f.pos),
            Target::Surface(i) => self.project.leds.surfaces.get(i).map(|s| s.pos),
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

    /// Groupe d'objets d'un objet (0 : aucun).
    fn target_set(&self, t: Target) -> u32 {
        match t {
            Target::Fixture(i) => self.project.leds.fixtures.get(i).map_or(0, |f| f.set),
            Target::Surface(i) => self.project.leds.surfaces.get(i).map_or(0, |s| s.set),
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
        }
    }

    /// L'objet choisi (ou son groupe), sous forme de cible.
    fn current_target(&self) -> Option<Target> {
        match (self.led.selected, self.led.selected_surface) {
            (Some(i), _) => Some(Target::Fixture(i)),
            (_, Some(i)) => Some(Target::Surface(i)),
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
            self.project.leds.sets.push(leds::LedGroup { id, name: format!("Groupe d'objets {id}") });
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
        }
    }

    fn target_mut(&mut self, t: Target) -> Option<(&mut [f32; 3], &mut [f32; 3])> {
        match t {
            Target::Fixture(i) => self.project.leds.fixtures.get_mut(i).map(|f| (&mut f.pos, &mut f.rot)),
            Target::Surface(i) => self.project.leds.surfaces.get_mut(i).map(|s| (&mut s.pos, &mut s.rot)),
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
            let room = setup.room;
            let presets: [(&str, leds::Surface); 7] = [
                ("Écran", leds::Surface { pos: [0.0, 0.8, (room[1] - 0.3).max(0.5)], ..leds::Surface::default() }),
                (
                    "Boîte",
                    leds::Surface {
                        kind: leds::SurfaceKind::Box,
                        pos: [0.0, 0.0, room[1] * 0.5],
                        size: [1.0, 1.0, 1.0],
                        show_output: false,
                        ..leds::Surface::default()
                    },
                ),
                (
                    "Mur",
                    leds::Surface {
                        kind: leds::SurfaceKind::Box,
                        pos: [0.0, 0.0, room[1]],
                        size: [room[0], 3.0, 0.15],
                        color: [120, 124, 132],
                        show_output: false,
                        ..leds::Surface::default()
                    },
                ),
                (
                    "Scène",
                    leds::Surface {
                        kind: leds::SurfaceKind::Stage,
                        pos: [0.0, 0.0, (room[1] - 2.3).max(2.0)],
                        size: [(room[0] - 1.0).clamp(1.0, 8.0), 0.8, 4.0],
                        color: [48, 48, 56],
                        show_output: false,
                        ..leds::Surface::default()
                    },
                ),
                (
                    "Public",
                    leds::Surface {
                        kind: leds::SurfaceKind::Crowd,
                        pos: [0.0, 0.0, (room[1] * 0.3).max(1.5)],
                        size: [(room[0] - 1.5).clamp(1.0, 6.0), 1.75, (room[1] * 0.35).clamp(1.0, 5.0)],
                        show_output: false,
                        ..leds::Surface::default()
                    },
                ),
                (
                    "Chips",
                    leds::Surface {
                        kind: leds::SurfaceKind::Panels,
                        pos: [0.0, 0.5, (room[1] - 1.0).max(1.0)],
                        size: [5.5, 2.6, 0.5],
                        show_output: false,
                        ..leds::Surface::default()
                    },
                ),
                ("Cabine DJ", {
                    // Posée au milieu de la scène s'il y en a une.
                    let stage = setup.surfaces.iter().find(|s| s.kind == leds::SurfaceKind::Stage);
                    let (y, z) = stage.map_or((0.0, room[1] * 0.6), |st| (st.top(), st.pos[2] - 0.4));
                    leds::Surface {
                        kind: leds::SurfaceKind::Booth,
                        pos: [stage.map_or(0.0, |st| st.pos[0]), y, z],
                        size: [1.8, 1.05, 0.7],
                        color: [30, 30, 34],
                        show_output: true,
                        ..leds::Surface::default()
                    }
                }),
            ];
            for (name, mut s) in presets {
                if ui.button(with_icon(ic::PLUS, name)).clicked() {
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
        let mut panel_leds: Option<(String, Vec<(f32, Vec<[f32; 3]>)>)> = None;
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
                ui.horizontal(|ui| {
                    ui.label("Soleil");
                    ui.add(egui::DragValue::new(&mut p.sun).range(0.0..=5.0).speed(0.01).suffix(" m"))
                        .on_hover_text("Diamètre du soleil au centre (0 : aucun)");
                    ui.color_edit_button_srgb(&mut p.sun_color);
                });
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
                ui.checkbox(&mut s.show_output, "Affiche l'image de la sortie (face avant)");
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
                setup.sets.push(leds::LedGroup { id, name: name.to_owned() });
                setup.surfaces[si].set = id;
            }
            for f in setup.fixtures.iter_mut().filter(|f| f.name.starts_with(&prefix)) {
                f.set = id;
            }
            self.led.selected_set = Some(id);
        }
        // Premières bandes : groupes en paires gauche/droite, du centre vers les bords (pour les scénarios).
        if setup.groups.is_empty() {
            setup.auto_groups(true);
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
        let mut start = false;
        let mut redraw = None;
        self.leds_install_body(ui, &mut start, &mut redraw);
        if start {
            self.start_drawing(None);
        }
        if redraw.is_some() {
            self.start_drawing(redraw);
        }
    }

    fn leds_install_body(&mut self, ui: &mut egui::Ui, start: &mut bool, redraw: &mut Option<usize>) {
        let setup = &mut self.project.leds;
        theme::section(ui, ic::LIGHTBULB, "Éléments LED");
        ui.horizontal_wrapped(|ui| {
            for kind in FixtureKind::ALL {
                if kind == FixtureKind::Path {
                    if ui
                        .button(with_icon(ic::PENCIL_SIMPLE, "Dessiner un tracé"))
                        .on_hover_text("Dessinez le parcours du bandeau point par point dans la vue 3D")
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
                    if ui.button(with_icon(ic::PENCIL_SIMPLE, "Redessiner")).clicked() {
                        *redraw = Some(si);
                    }
                });
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
            ui.horizontal(|ui| {
                ui.label("Position");
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.add(egui::DragValue::new(&mut f.pos[k]).speed(0.01).prefix(format!("{axis} ")).suffix(" m"));
                }
            })
            .response
            .on_hover_text("X : gauche/droite · Y : hauteur · Z : de l'avant vers le fond de la salle");
            if f.kind != FixtureKind::Column {
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
        let setup = &mut self.project.leds;
        theme::section(ui, ic::PLAY, "Ce que montrent les LEDs");
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
                ui.indent("led_scenario", |ui| scenario_ui(ui, setup, &mut self.led.list_drag));
            }
            if s == Source::Solid && setup.source == Source::Solid {
                ui.indent("led_color", |ui| {
                    ui.color_edit_button_srgb(&mut setup.test_color);
                });
            }
        }
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
                setup.groups.push(leds::LedGroup { id, name: format!("Groupe {id}") });
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
                setup.sets.push(leds::LedGroup { id, name: format!("Groupe d'objets {id}") });
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
fn scenario_ui(ui: &mut egui::Ui, setup: &mut LedSetup, drag: &mut Option<(u8, Vec<usize>)>) {
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

    ui.add_space(4.0);
    ui.label(egui::RichText::new("Groupes de bandeaux").strong());
    ui.label(
        egui::RichText::new(
            "Les éléments d'un groupe s'allument ensemble ; les scénarios passent d'un groupe au suivant dans cet ordre. \
             Sans groupe, chaque élément est son propre groupe.",
        )
        .small()
        .color(TEXT_WEAK),
    );
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Du centre vers les bords").on_hover_text("Paires symétriques gauche/droite, du centre vers l'extérieur").clicked() {
            setup.auto_groups(true);
        }
        if ui.small_button("De gauche à droite").clicked() {
            setup.auto_groups(false);
        }
        if !setup.groups.is_empty() && ui.small_button("Aucun groupe").clicked() {
            setup.groups.clear();
            for f in &mut setup.fixtures {
                f.group = 0;
            }
        }
        if ui.small_button(with_icon(ic::PLUS, "Groupe")).clicked() {
            let id = setup.next_group_id();
            setup.groups.push(leds::LedGroup { id, name: format!("Groupe {id}") });
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
        if let (Some((pa, _)), Some((pb, _))) = (self.project(a), self.project(b)) {
            painter.line_segment([pa, pb], stroke);
        }
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
    let cell = |v: f32| (v / CELL).floor() as i32;
    let mut bins: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (k, (pts, _)) in tiles.iter().enumerate() {
        let r = Rect::from_points(pts);
        for cx in cell(r.min.x)..=cell(r.max.x) {
            for cy in cell(r.min.y)..=cell(r.max.y) {
                bins.entry((cx, cy)).or_default().push(k);
            }
        }
    }
    let covered = |p: egui::Pos2, z: f32, margin: f32| {
        bins.get(&(cell(p.x), cell(p.y)))
            .is_some_and(|ks| ks.iter().any(|&k| depth_at(&tiles[k].0, &tiles[k].1, p).is_some_and(|d| d < z - margin)))
    };
    for (z, it) in items.iter_mut() {
        match it {
            Item::Led { p, hidden, .. } => *hidden = covered(*p, *z, 0.005),
            // Un trait est caché si son milieu l'est (sa propre face, à la même distance, ne compte pas).
            Item::Wire { a, b, z: wz, hidden, .. } => *hidden = covered(egui::pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5), *wz, 0.015),
            Item::Tile { .. } => {}
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
fn face_tiles(
    view: &View,
    q: [[f32; 3]; 4],
    fill: Color32,
    texture: Option<(egui::TextureId, [[f32; 2]; 4])>,
    edge: Stroke,
    edges: u8,
    surface: usize,
    items: &mut Vec<(f32, Item)>,
) {
    let len = |a: [f32; 3], b: [f32; 3]| dot(sub(a, b), sub(a, b)).sqrt();
    // Une image garde des morceaux plus petits, pour que la perspective reste juste.
    let tile = if texture.is_some() { TILE * 0.6 } else { TILE };
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
            grid.push((pp.0, pp.1, s, t));
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
