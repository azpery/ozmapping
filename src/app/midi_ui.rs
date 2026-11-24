//! Onglet MIDI : l'Akai MPD218 dessiné, pour assigner d'un clic une commande à chaque pad
//! et potentiomètre ; exécution des commandes reçues.

use std::collections::HashMap;

use super::*;
use crate::midi::{self, Action, Control, FX_PARAMS};

/// État MIDI de la session (non enregistré ; les assignations sont dans `OzApp::midi_setup`).
#[derive(Default)]
pub(super) struct MidiRuntime {
    input: Option<midi::Input>,
    ports: Vec<String>,
    /// Dernière recherche des ports (s).
    ports_at: f64,
    error: Option<String>,
    /// Commande choisie sur le dessin.
    selected: Option<Control>,
    /// Le prochain message reçu devient celui de la commande choisie.
    learning: bool,
    /// La commande choisie vient de changer : faire défiler jusqu'à sa fiche.
    reveal: bool,
    /// Après « Disposition live » : ce qui reste à assigner à la main.
    layout_note: Option<String>,
    /// Toucher le contrôleur choisit la commande sur le dessin.
    follow: Option<bool>,
    /// Banques affichées (pads, potentiomètres).
    pad_bank: u8,
    knob_bank: u8,
    /// Dernier appui de chaque pad (heure, vélocité) et dernière valeur de chaque potentiomètre.
    hits: HashMap<Control, (f64, u8)>,
    values: HashMap<Control, u8>,
    /// Dernier message reçu, reconnu ou non.
    last: Option<(midi::Event, Option<Control>)>,
}

impl MidiRuntime {
    fn follow(&self) -> bool {
        self.follow.unwrap_or(true)
    }
}

impl OzApp {
    /// Connexion au contrôleur et exécution des messages reçus (à chaque image).
    pub(super) fn midi_tick(&mut self, ctx: &egui::Context) {
        let now = self.clock();
        // Branchement et débranchement : les ports sont relus régulièrement.
        if now - self.midi.ports_at > 2.0 {
            self.midi.ports_at = now;
            self.midi.ports = midi::ports();
            if self.midi.input.as_ref().is_some_and(|i| !self.midi.ports.contains(&i.port)) {
                self.midi.input = None;
            }
            if self.midi.input.is_none() {
                if let Some(port) = midi::pick_port(&self.midi.ports, self.midi_setup.port.as_deref()) {
                    match midi::Input::open(&port) {
                        Ok(i) => {
                            self.midi.input = Some(i);
                            self.midi.error = None;
                        }
                        Err(e) => self.midi.error = Some(format!("{e:#}")),
                    }
                }
            }
        }
        let Some(events) = self.midi.input.as_ref().map(midi::Input::drain) else { return };
        if events.is_empty() {
            return;
        }
        ctx.request_repaint();
        for e in events {
            // Apprentissage : le message touché devient celui de la commande choisie.
            if self.midi.learning && e.on {
                if let Some(c) = self.midi.selected {
                    if c.is_pad() == (e.key.kind == midi::Kind::Note) {
                        self.midi_setup.learn(c, e.key);
                        self.midi.learning = false;
                    }
                }
            }
            let control = self.midi_setup.control_of(e.key);
            self.midi.last = Some((e, control));
            let Some(c) = control else { continue };
            match c {
                Control::Pad { bank, .. } => {
                    self.midi.pad_bank = bank;
                    if e.on {
                        self.midi.hits.insert(c, (now, e.value));
                    }
                }
                Control::Knob { bank, .. } => {
                    self.midi.knob_bank = bank;
                    self.midi.values.insert(c, e.value);
                }
            }
            // Sélection sur le dessin, sans faire défiler le panneau (on joue, on ne règle pas).
            if self.midi.follow() && e.on {
                self.midi.selected = Some(c);
            }
            let action = self.midi_setup.action(c).clone();
            if c.is_pad() {
                self.midi_trigger(ctx, &action, e.on);
            } else {
                self.midi_value(&action, f32::from(e.value) / 127.0);
            }
        }
    }

    /// Commande d'un pad (`on` : appui, sinon relâchement).
    pub(super) fn midi_trigger(&mut self, ctx: &egui::Context, a: &Action, on: bool) {
        // Médias et lecture : sur le calque de contenu (celui du dessin n'affiche pas de vidéo).
        if on && a.targets_content() {
            let keep = self.selected;
            self.selected = self.content_layer();
            self.midi_trigger_on(ctx, a);
            self.selected = keep;
            return;
        }
        self.midi_trigger_inner(ctx, a, on);
    }

    fn midi_trigger_on(&mut self, ctx: &egui::Context, a: &Action) {
        self.midi_trigger_inner(ctx, a, true);
    }

    /// Calque où vont les médias et visuels : le calque choisi, ou s'il affiche le dessin,
    /// le premier calque sans dessin en dessous (même sortie), sinon au-dessus.
    pub(super) fn content_layer(&self) -> usize {
        let sel = self.sel();
        let layers = &self.project.layers;
        if !layers[sel].sketch {
            return sel;
        }
        let out = layers[sel].output;
        (0..sel)
            .rev()
            .chain(sel + 1..layers.len())
            .find(|&i| !layers[i].sketch && layers[i].output == out)
            .unwrap_or(sel)
    }

    fn midi_trigger_inner(&mut self, ctx: &egui::Context, a: &Action, on: bool) {
        if !on {
            match a {
                Action::BlackoutHold => self.blackout = false,
                Action::DmxHold { fixture } => {
                    self.led.bursts.insert(*fixture, self.clock() + 0.15);
                }
                _ => {}
            }
            return;
        }
        let sel = self.sel();
        match a {
            Action::None => {}
            Action::PlayCell { layer, column } => {
                if *layer < self.project.layers.len() {
                    self.play_cell(ctx, *layer, *column);
                }
            }
            Action::PlayColumn { column } => self.play_column(ctx, *column),
            Action::Preset { name } => {
                let fx = self
                    .builtin_presets
                    .iter()
                    .chain(&self.project.presets)
                    .find(|p| &p.name == name)
                    .map(|p| p.effects);
                if let Some(fx) = fx {
                    self.apply_preset(Some(name.clone()), fx);
                }
            }
            Action::RandomEffect => {
                let fx = Effects::random(self.next_random());
                self.apply_preset(None, fx);
            }
            Action::ClearEffects => self.clear_effects(),
            Action::LaunchMedia { index } => self.launch_media(ctx, *index),
            Action::NextMedia => self.launch_relative(ctx, 1),
            Action::PrevMedia => self.launch_relative(ctx, -1),
            Action::SelectLayer { layer } => {
                if *layer < self.project.layers.len() {
                    self.selected = *layer;
                }
            }
            Action::NextLayer => self.selected = (sel + 1) % self.project.layers.len(),
            Action::Blackout => self.blackout = !self.blackout,
            Action::BlackoutHold => self.blackout = true,
            Action::White => self.white_screen = !self.white_screen,
            Action::Bypass => self.bypass_fx = !self.bypass_fx,
            Action::Grid => self.grid = !self.grid,
            Action::Tap => self.tap(),
            Action::Pause => self.toggle_pause(),
            Action::Reverse => self.toggle_reverse(),
            Action::AutoFx => {
                let auto = &mut self.project.layers[sel].auto;
                auto.enabled = !auto.enabled;
                if auto.enabled {
                    self.auto_next(sel);
                }
            }
            Action::Shatter => self.layers[sel].shatter_shot = Some(0.0),
            Action::LedLive => self.led.live = !self.led.live,
            Action::LedScenario { kind } => {
                self.project.leds.source = crate::leds::Source::Scenario;
                self.project.leds.scenario.kind = *kind;
                self.project.leds.scenario.chain = false;
            }
            Action::DmxToggle { fixture } => {
                if let Some(d) = self.project.leds.dmx.iter_mut().find(|d| d.id == *fixture) {
                    d.on = !d.on;
                }
            }
            Action::DmxHold { fixture } => {
                self.led.bursts.insert(*fixture, f64::INFINITY);
            }
            Action::TempoHalf | Action::TempoDouble => {
                let k = if *a == Action::TempoHalf { 0.5 } else { 2.0 };
                let bpm = (self.rhythm.tempo.bpm * k).clamp(30.0, 300.0);
                let now = self.clock();
                self.rhythm.tempo.set_bpm(bpm, now);
            }
            Action::LedSource { source } => self.project.leds.source = *source,
            Action::LedChain => {
                let sc = &mut self.project.leds.scenario;
                sc.chain = !sc.chain;
                self.project.leds.source = crate::leds::Source::Scenario;
            }
            Action::DrawingPlain => {
                for i in 0..self.project.layers.len() {
                    if self.project.layers[i].sketch {
                        self.project.layers[i].auto.enabled = false;
                        self.project.layers[i].mix.enabled = false;
                        self.project.layers[i].generator.pattern = Pattern::Off;
                        self.apply_preset_to(i, None, Effects::default(), 0.0);
                        self.layers[i].phases = Phases::default();
                    }
                }
            }
            Action::Mode { mode } => self.show_mode(*mode),
            Action::IpadOutput { output } => self.set_ipad_output(*output),
            Action::GroupVisible { group } => {
                if let Some(g) = self.project.groups.iter_mut().find(|g| g.id == *group) {
                    g.visible = !g.visible;
                }
            }
            Action::GroupFx { group, fx } => {
                if let Some(g) = self.project.groups.iter_mut().find(|g| g.id == *group) {
                    g.fx.kind = if g.fx.kind == *fx { crate::group_fx::GroupFx::Off } else { *fx };
                }
            }
            Action::Visual { pattern } => {
                if *pattern == Pattern::Off {
                    self.project.layers[sel].generator.pattern = Pattern::Off;
                } else {
                    self.launch_any(ctx, sel, &MediaDrag::Pattern(*pattern));
                }
            }
            Action::LayerVisible { layer } => {
                if let Some(cfg) = self.project.layers.get_mut(*layer) {
                    cfg.visible = !cfg.visible;
                }
            }
            Action::Chain => {
                let chain = &mut self.project.chain;
                chain.enabled = !chain.enabled;
                self.chain_timer = 0.0;
            }
            Action::Record => {
                if self.recorder.is_some() {
                    self.stop_recording();
                } else {
                    self.start_recording();
                }
            }
            Action::MixAuto => {
                let mix = &mut self.project.layers[sel].mix;
                mix.enabled = !mix.enabled;
            }
            Action::React => {
                let react = &mut self.project.layers[sel].react;
                react.enabled = !react.enabled;
            }
            Action::LaserArm => self.laser.armed = !self.laser.armed,
            Action::LaserStop => self.laser_stop(),
            Action::LaserCue { index } => {
                if let Some(c) = self.project.leds.laser_cues.get(*index).cloned() {
                    for l in &mut self.project.leds.lasers {
                        l.cue = c.clone();
                    }
                }
            }
            // Commandes de potentiomètre sur un pad : la valeur maximale.
            other => self.midi_value(other, 1.0),
        }
    }

    /// Commande d'un potentiomètre (`v` : 0 à 1).
    pub(super) fn midi_value(&mut self, a: &Action, v: f32) {
        let sel = if a.targets_content() { self.content_layer() } else { self.sel() };
        match a {
            Action::Master => self.project.master = v,
            Action::Opacity { layer } => {
                let l = layer.unwrap_or(sel);
                if let Some(cfg) = self.project.layers.get_mut(l) {
                    cfg.opacity = v;
                }
            }
            Action::Speed => {
                // ×0,25 à ×4, ×1 au milieu.
                let speed = 4f32.powf(2.0 * v - 1.0);
                if let Some(p) = self.layers[sel].active() {
                    p.set_speed(speed);
                }
            }
            Action::PresetFade => self.project.preset_secs = v * 3.0,
            Action::LedBrightness => self.project.leds.brightness = v,
            Action::LedSpeed => self.project.leds.scenario.speed = 4f32.powf(2.0 * v - 1.0),
            Action::LedFlash => self.project.leds.sound.flash = v,
            Action::LedFollow => self.project.leds.sound.follow = v,
            Action::LedHue => {
                let c = crate::leds::hue(v);
                self.project.leds.test_color = c.map(|x| (x.clamp(0.0, 1.0) * 255.0) as u8);
            }
            Action::LedChainSecs => self.project.leds.scenario.chain_secs = 2.0 + v * 58.0,
            Action::VisualMix => self.project.layers[sel].generator.mix = v,
            Action::Effect { param } => {
                if let Some(&(_, lo, hi)) = FX_PARAMS.get(*param) {
                    // Le potentiomètre prend la main : une transition de preset en cours s'arrête.
                    self.layers[sel].fx_anim = None;
                    if let Some(f) = midi::fx_param(&mut self.project.layers[sel].effects, *param) {
                        *f = lo + v * (hi - lo);
                    }
                }
            }
            _ => {}
        }
    }

    /// Onglet latéral MIDI.
    pub(super) fn midi_ui(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, ic::SQUARES_FOUR, "Contrôleur MIDI");
        // Port et état de la connexion.
        ui.horizontal(|ui| {
            let current = self.midi.input.as_ref().map(|i| i.port.clone());
            let (text, color) = match &current {
                Some(p) => (format!("{} {p}", ic::CHECK_SQUARE), ACCENT),
                None => (format!("{} Aucun contrôleur branché", ic::PROHIBIT), WARN),
            };
            ui.label(egui::RichText::new(text).color(color));
        });
        if self.midi.ports.len() > 1 || self.midi.input.is_none() {
            ui.horizontal(|ui| {
                ui.label("Port");
                let shown = self.midi_setup.port.clone().unwrap_or_else(|| "automatique (MPD218)".into());
                let mut choice = None;
                egui::ComboBox::from_id_salt("midi_port").selected_text(shown).width(200.0).show_ui(ui, |ui| {
                    if ui.selectable_label(self.midi_setup.port.is_none(), "automatique (MPD218)").clicked() {
                        choice = Some(None);
                    }
                    for p in &self.midi.ports {
                        if ui.selectable_label(self.midi_setup.port.as_ref() == Some(p), p).clicked() {
                            choice = Some(Some(p.clone()));
                        }
                    }
                });
                if let Some(c) = choice {
                    self.midi_setup.port = c;
                    self.midi.input = None;
                    self.midi.ports_at = 0.0;
                }
            });
        }
        if let Some(e) = &self.midi.error {
            ui.label(egui::RichText::new(e).small().color(theme::DANGER));
        }
        let mut follow = self.midi.follow();
        if ui
            .checkbox(&mut follow, "Toucher un pad le choisit sur le dessin")
            .on_hover_text("Frappez un pad ou tournez un potentiomètre : il se sélectionne ici, il ne reste qu'à lui donner une commande")
            .changed()
        {
            self.midi.follow = Some(follow);
        }
        ui.add_space(4.0);
        self.midi_device(ui);
        ui.add_space(6.0);
        self.midi_editor(ui);
        ui.add_space(6.0);
        self.midi_presets(ui);
    }

    /// Le MPD218 dessiné comme l'appareil : à gauche le logo, les 6 potentiomètres (2 par rangée),
    /// les voyants de banque et les boutons ; à droite les 16 pads, le pad 1 en bas à gauche.
    /// Clic : choisir la commande ; CTRL BANK et PAD BANK passent à la banque suivante.
    fn midi_device(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width().min(460.0);
        let h = w * 0.86;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
        let painter = ui.painter_at(rect);
        let at = |x: f32, y: f32| rect.min + Vec2::new(x * w, y * h);
        painter.rect_filled(rect, 12.0, Color32::from_rgb(24, 24, 27));
        painter.rect_stroke(rect, 12.0, Stroke::new(1.0, Color32::from_rgb(58, 60, 66)), egui::StrokeKind::Inside);
        let now = self.clock();
        let mut clicked = None;
        let print = Color32::from_rgb(215, 215, 220);
        // Battement du tempo (rythme actif, sinon tempo tapé) : le pad Tap s'allume sur chaque temps.
        let beat = self.beat_env();

        // Logo et nom.
        painter.text(at(0.05, 0.035), egui::Align2::LEFT_TOP, "AKAI", egui::FontId::proportional(w * 0.042), print);
        painter.text(at(0.05, 0.035) + Vec2::new(0.0, w * 0.045), egui::Align2::LEFT_TOP, "PROFESSIONAL", egui::FontId::proportional(w * 0.014), TEXT_WEAK);
        painter.text(at(0.965, 0.03), egui::Align2::RIGHT_TOP, "MPD218", egui::FontId::proportional(w * 0.05), print);

        // Potentiomètres : 2 colonnes, 3 rangées, numérotés 1 à 6 comme sur l'appareil.
        let kr = w * 0.036;
        for index in 0..midi::KNOBS as u8 {
            let c = Control::Knob { bank: self.midi.knob_bank, index };
            let (col, row) = (index % 2, index / 2);
            let center = at(0.1 + col as f32 * 0.14, 0.235 + row as f32 * 0.185);
            let sel = self.midi.selected == Some(c);
            let bound = self.midi_setup.action(c).clone();
            painter.text(center - Vec2::new(0.0, kr * 1.55), egui::Align2::CENTER_BOTTOM, (index + 1).to_string(), egui::FontId::proportional(9.0), print);
            painter.circle_filled(center, kr * 1.15, Color32::from_rgb(10, 10, 12));
            painter.circle_filled(center, kr, Color32::from_rgb(40, 41, 46));
            // Repère : de -135° à +135° selon la dernière valeur reçue.
            let v = self.midi.values.get(&c).copied().map_or(0.0, |v| f32::from(v) / 127.0);
            let a = (-135.0 + 270.0 * v).to_radians();
            let dir = Vec2::new(a.sin(), -a.cos());
            painter.line_segment([center + dir * kr * 0.25, center + dir * kr * 0.95], Stroke::new(2.0, Color32::WHITE));
            if sel {
                painter.circle_stroke(center, kr * 1.4, Stroke::new(2.0, ACCENT));
            } else if bound != Action::None {
                painter.circle_stroke(center, kr * 1.3, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)));
            }
            if bound != Action::None {
                painter.text(
                    center + Vec2::new(0.0, kr * 1.3),
                    egui::Align2::CENTER_TOP,
                    elide(&bound.short(), 11),
                    egui::FontId::proportional(9.5),
                    ACCENT,
                );
            }
            let hit = Rect::from_center_size(center + Vec2::new(0.0, kr * 0.2), Vec2::new(w * 0.13, kr * 3.4));
            let r = ui
                .interact(hit, ui.scope_id().with(("midi", c)), Sense::click())
                .on_hover_text(format!("K{} · {}", c_num(c), if bound == Action::None { "libre".into() } else { bound.short() }))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if r.clicked() {
                clicked = Some(c);
            }
        }

        // Voyants de banque (A, B ; les deux allumés : C) au-dessus de CTRL BANK et PAD BANK.
        let leds = |x: f32, bank: u8| {
            for (k, name) in ["A", "B"].into_iter().enumerate() {
                let on = bank == k as u8 || bank == 2;
                let p = at(x, 0.74) + Vec2::new((k as f32 - 0.5) * w * 0.036, 0.0);
                painter.text(p - Vec2::new(0.0, w * 0.013), egui::Align2::CENTER_BOTTOM, name, egui::FontId::proportional(8.5), print);
                painter.circle_filled(p, w * 0.009, if on { Color32::from_rgb(255, 150, 40) } else { Color32::from_rgb(70, 30, 26) });
            }
        };
        leds(0.085, self.midi.knob_bank);
        leds(0.265, self.midi.pad_bank);

        // Boutons : la banque change d'un clic, les autres sont dessinés pour s'y retrouver.
        let bs = w * 0.045;
        let buttons = [
            ("CTRL\nBANK", 0.085, 0.815),
            ("PROG\nSELECT", 0.175, 0.815),
            ("PAD\nBANK", 0.265, 0.815),
            ("FULL\nLEVEL", 0.085, 0.935),
            ("NR\nCONFIG", 0.175, 0.935),
            ("NOTE\nREPEAT", 0.265, 0.935),
        ];
        for (name, x, y) in buttons {
            let r = Rect::from_center_size(at(x, y) - Vec2::new(0.0, w * 0.02), Vec2::splat(bs));
            let bank = matches!(name, "CTRL\nBANK" | "PAD\nBANK");
            let resp = ui.interact(r, ui.scope_id().with(("midi_btn", name)), if bank { Sense::click() } else { Sense::hover() });
            painter.rect_filled(r, 3.0, if bank && resp.hovered() { Color32::from_rgb(56, 58, 64) } else { Color32::from_rgb(14, 14, 16) });
            painter.rect_stroke(r, 3.0, Stroke::new(1.0, Color32::from_rgb(60, 62, 68)), egui::StrokeKind::Inside);
            painter.text(r.center_bottom() + Vec2::new(0.0, 1.0), egui::Align2::CENTER_TOP, name, egui::FontId::proportional(7.0), print);
            if bank {
                let resp = resp.on_hover_text("Banque suivante (A → B → C), comme sur l'appareil").on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    let b = if name == "CTRL\nBANK" { &mut self.midi.knob_bank } else { &mut self.midi.pad_bank };
                    *b = (*b + 1) % midi::BANKS as u8;
                }
            }
        }

        // Pads : 4 × 4 dans leur cadre, le pad 1 en bas à gauche, « PAD n » sous chaque pad.
        let pads = Rect::from_min_max(at(0.355, 0.13), at(0.97, 0.965));
        painter.rect_filled(pads, 6.0, Color32::from_rgb(16, 16, 18));
        let inner = pads.shrink(w * 0.015);
        let gap = w * 0.016;
        let cw = (inner.width() - 3.0 * gap) / 4.0;
        let ch = (inner.height() - 3.0 * gap) / 4.0;
        let label_h = 11.0;
        for index in 0..midi::PADS as u8 {
            let c = Control::Pad { bank: self.midi.pad_bank, index };
            let (col, row_from_bottom) = (index % 4, index / 4);
            let cell = Rect::from_min_size(
                egui::pos2(inner.min.x + col as f32 * (cw + gap), inner.max.y - (row_from_bottom as f32 + 1.0) * ch - row_from_bottom as f32 * gap),
                Vec2::new(cw, ch),
            );
            let r = Rect::from_min_max(cell.min, egui::pos2(cell.max.x, cell.max.y - label_h));
            // Pad frappé : il s'allume en rouge, selon la vélocité, puis s'éteint.
            let glow = self.midi.hits.get(&c).map_or(0.0, |(t, vel)| {
                let age = (now - t) as f32;
                (1.0 - age / 0.45).max(0.0) * (0.35 + 0.65 * f32::from(*vel) / 127.0)
            });
            let bound_now = self.midi_setup.action(c);
            let glow = if *bound_now == Action::Tap { glow.max(beat * 0.85) } else { glow };
            if glow > 0.0 || *bound_now == Action::Tap {
                ui.ctx().request_repaint();
            }
            let fill = lerp_color(Color32::from_rgb(38, 39, 43), Color32::from_rgb(235, 60, 40), glow);
            painter.rect_filled(r, 4.0, fill);
            let sel = self.midi.selected == Some(c);
            let bound = self.midi_setup.action(c).clone();
            if sel {
                painter.rect_stroke(r, 4.0, Stroke::new(2.5, ACCENT), egui::StrokeKind::Inside);
            } else if bound != Action::None {
                painter.rect_stroke(r, 4.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), egui::StrokeKind::Inside);
            }
            painter.text(
                egui::pos2(cell.min.x + 2.0, cell.max.y),
                egui::Align2::LEFT_BOTTOM,
                format!("PAD {}", index + 1),
                egui::FontId::proportional(8.5),
                print,
            );
            if bound != Action::None {
                // Police réduite jusqu'à ce que le mot le plus long tienne sur la largeur du pad.
                let text = bound.short();
                let mut size = 10.5;
                while size > 7.5 {
                    let widest = text
                        .split_whitespace()
                        .map(|wd| painter.layout_no_wrap(wd.to_string(), egui::FontId::proportional(size), ACCENT).size().x)
                        .fold(0.0, f32::max);
                    if widest <= r.width() - 6.0 {
                        break;
                    }
                    size -= 0.5;
                }
                let galley = painter.layout(text, egui::FontId::proportional(size), ACCENT, r.width() - 6.0);
                painter.galley(r.center() - galley.size() * 0.5, galley, ACCENT);
            }
            let resp = ui
                .interact(cell, ui.scope_id().with(("midi", c)), Sense::click())
                .on_hover_text(format!("Pad {} (banque {})", c_num(c), (b'A' + self.midi.pad_bank) as char))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                clicked = Some(c);
            }
        }
        if let Some(c) = clicked {
            self.midi.selected = Some(c);
            self.midi.reveal = true;
            self.midi.learning = false;
        }
    }

    /// Commande assignée à la commande physique choisie.
    fn midi_editor(&mut self, ui: &mut egui::Ui) {
        let Some(c) = self.midi.selected else {
            ui.label(
                egui::RichText::new("Cliquez un pad ou un potentiomètre sur le dessin (ou touchez-le sur le contrôleur) pour lui donner une commande.")
                    .small()
                    .color(TEXT_WEAK),
            );
            return;
        };
        let key = self.midi_setup.key_of(c);
        let mut action = self.midi_setup.action(c).clone();
        let layers = self.project.layers.len();
        let columns = self.project.columns;
        let presets: Vec<String> =
            self.builtin_presets.iter().chain(&self.project.presets).map(|p| p.name.clone()).collect();
        let media: Vec<String> = self.media_folders.all.iter().map(|p| file_name(p)).collect();
        let dmx: Vec<(u32, String)> = self.project.leds.dmx.iter().map(|d| (d.id, d.name.clone())).collect();
        let cues: Vec<String> = self.project.leds.laser_cues.iter().map(|c| c.name.clone()).collect();
        let card = theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(c.name()).strong());
                ui.label(egui::RichText::new(key.label()).small().color(TEXT_WEAK));
            });
            ui.horizontal(|ui| {
                if self.midi.learning {
                    ui.label(egui::RichText::new(if c.is_pad() { "Frappez le pad sur le contrôleur…" } else { "Tournez le potentiomètre…" }).color(WARN));
                    if ui.small_button("Annuler").clicked() {
                        self.midi.learning = false;
                    }
                } else if ui
                    .small_button("Apprendre")
                    .on_hover_text("Si le contrôleur envoie un autre message que celui d'usine : le prochain reçu sera celui de cette commande")
                    .clicked()
                {
                    self.midi.learning = true;
                }
            });
            // Type de commande.
            let choices = if c.is_pad() { Action::triggers() } else { Action::continuous() };
            ui.horizontal(|ui| {
                ui.label("Commande");
                egui::ComboBox::from_id_salt("midi_action").selected_text(action.title()).width(200.0).show_ui(ui, |ui| {
                    if ui.selectable_label(action == Action::None, "Rien").clicked() {
                        action = Action::None;
                    }
                    for a in choices {
                        if ui.selectable_label(action.same_kind(&a), a.title()).clicked() && !action.same_kind(&a) {
                            action = a;
                        }
                    }
                });
            });
            // Paramètres.
            match &mut action {
                Action::PlayCell { layer, column } => {
                    ui.horizontal(|ui| {
                        ui.label("Calque");
                        let mut l = *layer + 1;
                        ui.add(egui::DragValue::new(&mut l).range(1..=layers.max(1)));
                        *layer = l - 1;
                        ui.label("Colonne");
                        let mut k = *column + 1;
                        ui.add(egui::DragValue::new(&mut k).range(1..=columns.max(1)));
                        *column = k - 1;
                    });
                }
                Action::PlayColumn { column } => {
                    ui.horizontal(|ui| {
                        ui.label("Colonne");
                        let mut k = *column + 1;
                        ui.add(egui::DragValue::new(&mut k).range(1..=columns.max(1)));
                        *column = k - 1;
                    });
                }
                Action::SelectLayer { layer } => {
                    ui.horizontal(|ui| {
                        ui.label("Calque");
                        let mut l = *layer + 1;
                        ui.add(egui::DragValue::new(&mut l).range(1..=layers.max(1)));
                        *layer = l - 1;
                    });
                }
                Action::Preset { name } => {
                    egui::ComboBox::from_id_salt("midi_preset")
                        .selected_text(if name.is_empty() { "Choisir un preset…" } else { name.as_str() })
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            for p in &presets {
                                ui.selectable_value(name, p.clone(), p);
                            }
                        });
                }
                Action::LaunchMedia { index } => {
                    let shown = media.get(*index).cloned().unwrap_or_else(|| format!("Média {}", *index + 1));
                    egui::ComboBox::from_id_salt("midi_media").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                        for (k, m) in media.iter().enumerate() {
                            ui.selectable_value(index, k, format!("{}. {m}", k + 1));
                        }
                    });
                }
                Action::DmxToggle { fixture } | Action::DmxHold { fixture } => {
                    let shown = dmx.iter().find(|d| d.0 == *fixture).map_or("Choisir un appareil…".to_owned(), |d| d.1.clone());
                    egui::ComboBox::from_id_salt("midi_dmx").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                        for (id, name) in &dmx {
                            ui.selectable_value(fixture, *id, name);
                        }
                    });
                }
                Action::LaserCue { index } => {
                    let shown = cues.get(*index).cloned().unwrap_or_else(|| format!("Réglage {}", *index + 1));
                    egui::ComboBox::from_id_salt("midi_laser_cue").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                        for (k, name) in cues.iter().enumerate() {
                            ui.selectable_value(index, k, name);
                        }
                    });
                }
                Action::LedSource { source } => {
                    egui::ComboBox::from_id_salt("midi_led_source").selected_text(source.name()).width(220.0).show_ui(ui, |ui| {
                        for k in crate::leds::Source::ALL {
                            ui.selectable_value(source, k, k.name());
                        }
                    });
                }
                Action::GroupFx { group, fx } => {
                    ui.add(egui::DragValue::new(group).range(1..=999).prefix("groupe n° "));
                    egui::ComboBox::from_id_salt("midi_group_fx").selected_text(fx.name()).show_ui(ui, |ui| {
                        for k in crate::group_fx::GroupFx::ALL {
                            ui.selectable_value(fx, k, k.name());
                        }
                    });
                }
                Action::GroupVisible { group } => {
                    ui.add(egui::DragValue::new(group).range(1..=999).prefix("groupe n° "))
                        .on_hover_text("Numéro du groupe (dans l'ordre de leur création)");
                }
                Action::IpadOutput { output } => {
                    egui::ComboBox::from_id_salt("midi_ipad_out").selected_text(format!("Sortie {}", *output + 1)).show_ui(ui, |ui| {
                        for k in 0..8 {
                            ui.selectable_value(output, k, format!("Sortie {}", k + 1));
                        }
                    });
                }
                Action::Mode { mode } => {
                    egui::ComboBox::from_id_salt("midi_mode").selected_text(mode.name()).width(220.0).show_ui(ui, |ui| {
                        for m in crate::midi::ShowMode::ALL {
                            ui.selectable_value(mode, m, m.name());
                        }
                    });
                }
                Action::LedScenario { kind } => {
                    egui::ComboBox::from_id_salt("midi_scenario").selected_text(kind.name()).width(220.0).show_ui(ui, |ui| {
                        for k in crate::leds::ScenarioKind::ALL {
                            ui.selectable_value(kind, k, k.name());
                        }
                    });
                }
                Action::Opacity { layer } => {
                    let shown = layer.map_or("Calque sélectionné".to_owned(), |l| format!("Calque {}", l + 1));
                    egui::ComboBox::from_id_salt("midi_opacity").selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(layer, None, "Calque sélectionné");
                        for l in 0..layers {
                            ui.selectable_value(layer, Some(l), format!("Calque {}", l + 1));
                        }
                    });
                }
                Action::Effect { param } => {
                    egui::ComboBox::from_id_salt("midi_fx")
                        .selected_text(FX_PARAMS.get(*param).map_or("", |p| p.0))
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for (k, p) in FX_PARAMS.iter().enumerate() {
                                ui.selectable_value(param, k, p.0);
                            }
                        });
                    ui.label(egui::RichText::new("Sur le calque sélectionné.").small().color(TEXT_WEAK));
                }
                _ => {}
            }
        });
        // Commande choisie à l'instant : sa fiche, sous le dessin, vient à l'écran.
        if std::mem::take(&mut self.midi.reveal) {
            card.response.scroll_to_me(Some(egui::Align::Center));
        }
        if &action != self.midi_setup.action(c) {
            self.midi_setup.set_action(c, action);
        }
        // Dernier message reçu.
        if let Some((e, ctrl)) = &self.midi.last {
            let what = ctrl.map_or("inconnu (Apprendre pour l'associer)".to_owned(), |c| c.name());
            ui.label(
                egui::RichText::new(format!("Reçu : {} · valeur {}  →  {what}", e.key.label(), e.value)).small().color(TEXT_WEAK),
            );
        }
    }

    /// Remplissages rapides de la banque affichée.
    fn midi_presets(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Remplir d'un coup (banque affichée)").strong());
        if ui
            .button(with_icon(ic::SQUARES_FOUR, "Disposition live (3 banques)"))
            .on_hover_text("A : effets et tempo · B : médias et colonnes du tableau · C : LEDs, fumée et bulles (pads et potentiomètres).\nPad 16 = Tap partout (« TAP TEMPO » sur l'appareil) ; pad 13 = noir tant qu'on appuie.\nRemplace les commandes des trois banques.")
            .clicked()
        {
            let ok = self.midi_setup.bindings.is_empty()
                || rfd::MessageDialog::new()
                    .set_title("Disposition live")
                    .set_description("Remplacer les commandes des trois banques du MPD218 ?")
                    .set_buttons(rfd::MessageButtons::OkCancel)
                    .show()
                    == rfd::MessageDialogResult::Ok;
            if ok {
                let missing = self.midi_live_layout();
                self.midi.layout_note = Some(if missing.is_empty() {
                    "Disposition live en place.".into()
                } else {
                    format!("Disposition live en place. Ajoutez dans l'onglet DMX : {} (puis relancez la disposition).", missing.join(", "))
                });
            }
        }
        if let Some(note) = &self.midi.layout_note {
            ui.label(egui::RichText::new(note).small().color(TEXT_WEAK));
        }
        let (pb, kb) = (self.midi.pad_bank, self.midi.knob_bank);
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("Pads = colonnes du tableau").clicked() {
                for i in 0..midi::PADS as u8 {
                    let column = pb as usize * midi::PADS + i as usize;
                    self.midi_setup.set_action(Control::Pad { bank: pb, index: i }, Action::PlayColumn { column });
                }
            }
            if ui.small_button("Pads = presets").clicked() {
                let names: Vec<String> = self.builtin_presets.iter().map(|p| p.name.clone()).collect();
                for i in 0..midi::PADS as u8 {
                    if let Some(name) = names.get(pb as usize * midi::PADS + i as usize) {
                        self.midi_setup.set_action(Control::Pad { bank: pb, index: i }, Action::Preset { name: name.clone() });
                    }
                }
            }
            if ui.small_button("Pads = médias").clicked() {
                for i in 0..midi::PADS as u8 {
                    let index = pb as usize * midi::PADS + i as usize;
                    self.midi_setup.set_action(Control::Pad { bank: pb, index: i }, Action::LaunchMedia { index });
                }
            }
            if ui.small_button("Potentiomètres = réglages courants").clicked() {
                let set = [
                    Action::Master,
                    Action::Opacity { layer: None },
                    Action::Speed,
                    Action::Effect { param: 3 },
                    Action::Effect { param: 11 },
                    Action::Effect { param: 8 },
                ];
                for (i, a) in set.into_iter().enumerate() {
                    self.midi_setup.set_action(Control::Knob { bank: kb, index: i as u8 }, a);
                }
            }
            if ui.small_button("Tout effacer").on_hover_text("Retire toutes les commandes (toutes banques)").clicked() {
                self.midi_setup.bindings.clear();
            }
        });
    }
}

impl OzApp {
    /// Disposition proposée pour le live, sur les trois banques. Renvoie ce qui n'a pas pu être
    /// assigné (appareil DMX ou réglage laser absent du projet).
    fn midi_live_layout(&mut self) -> Vec<&'static str> {
        use crate::dmx::Kind;
        use crate::leds::ScenarioKind as S;
        let fx = |name: &str| Action::Effect { param: midi::FX_PARAMS.iter().position(|p| p.0 == name).unwrap_or(0) };
        let preset = |name: &str| Action::Preset { name: name.into() };
        let col = |column: usize| Action::PlayColumn { column };
        let dmx = &self.project.leds.dmx;
        let first = |k: Kind, n: usize| dmx.iter().filter(|d| d.kind == k).nth(n).map(|d| d.id);
        let mut missing = Vec::new();
        let mut need = |v: Option<Action>, what: &'static str| {
            v.unwrap_or_else(|| {
                missing.push(what);
                Action::None
            })
        };
        // Pads 1 à 16 de chaque banque (le pad 1 en bas à gauche). Partout : pad 13 = noir tant
        // qu'on appuie, pad 16 = Tap (« TAP TEMPO » est imprimé sous ce pad).
        // A : effets et tempo.
        let a = vec![
            preset("Néon"), preset("Kaléido"), preset("Tunnel"), preset("Glitch"),
            preset("Miroir 4"), preset("Psyché"), preset("Rétro VHS"), preset("Strobe"),
            Action::RandomEffect, Action::AutoFx, Action::Shatter, Action::ClearEffects,
            Action::BlackoutHold, Action::TempoHalf, Action::TempoDouble, Action::Tap,
        ];
        // B : médias et colonnes du tableau.
        let b = vec![
            col(0), col(1), col(2), col(3),
            col(4), col(5), col(6), col(7),
            Action::SelectLayer { layer: 0 }, Action::SelectLayer { layer: 1 }, Action::PrevMedia, Action::NextMedia,
            Action::BlackoutHold, Action::Pause, Action::Reverse, Action::Tap,
        ];
        // C : LEDs, fumée et bulles.
        let src = |source| Action::LedSource { source };
        let c = vec![
            Action::LedScenario { kind: S::HeartBeat }, Action::LedScenario { kind: S::Wave },
            Action::LedScenario { kind: S::Comets }, Action::LedScenario { kind: S::Rainbow },
            Action::LedScenario { kind: S::Sequence }, Action::LedScenario { kind: S::Sparkles },
            Action::LedScenario { kind: S::WaveColors }, Action::LedScenario { kind: S::Sound },
            src(crate::leds::Source::Video), src(crate::leds::Source::VideoAverage), src(crate::leds::Source::Solid), Action::LedChain,
            Action::BlackoutHold,
            need(first(Kind::Smoke, 0).map(|fixture| Action::DmxHold { fixture }), "machine à fumée"),
            need(first(Kind::Bubble, 0).map(|fixture| Action::DmxHold { fixture }), "machine à bulles"),
            Action::Tap,
        ];
        // Potentiomètres 1 à 6 (deux par rangée, 1 et 2 en haut).
        let ka = [fx("Teinte"), fx("Kaléidoscope"), fx("Tourbillon"), fx("Zoom"), fx("Glitch"), fx("Halo lumineux")];
        let kb = [Action::Master, Action::Opacity { layer: None }, Action::Speed, Action::PresetFade, Action::Opacity { layer: Some(0) }, Action::Opacity { layer: Some(1) }];
        let kc = [Action::LedBrightness, Action::LedSpeed, Action::LedFlash, Action::LedFollow, Action::LedHue, Action::LedChainSecs];
        for (bank, pads) in [a, b, c].into_iter().enumerate() {
            for (i, act) in pads.into_iter().enumerate() {
                self.midi_setup.set_action(Control::Pad { bank: bank as u8, index: i as u8 }, act);
            }
        }
        for (bank, knobs) in [ka, kb, kc].into_iter().enumerate() {
            for (i, act) in knobs.into_iter().enumerate() {
                self.midi_setup.set_action(Control::Knob { bank: bank as u8, index: i as u8 }, act);
            }
        }
        missing
    }
}

/// Numéro imprimé sur le contrôleur (pads 1 à 48, potentiomètres 1 à 18).
fn c_num(c: Control) -> usize {
    match c {
        Control::Pad { bank, index } => bank as usize * midi::PADS + index as usize + 1,
        Control::Knob { bank, index } => bank as usize * midi::KNOBS + index as usize + 1,
    }
}

fn elide(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_owned() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
