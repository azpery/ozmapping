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
    fn midi_trigger(&mut self, ctx: &egui::Context, a: &Action, on: bool) {
        if !on {
            if *a == Action::BlackoutHold {
                self.blackout = false;
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
            // Commandes de potentiomètre sur un pad : la valeur maximale.
            other => self.midi_value(other, 1.0),
        }
    }

    /// Commande d'un potentiomètre (`v` : 0 à 1).
    fn midi_value(&mut self, a: &Action, v: f32) {
        let sel = self.sel();
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

    /// Le MPD218 dessiné : potentiomètres en haut, banques, 16 pads. Clic : choisir la commande.
    fn midi_device(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width().min(440.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, w * 0.98), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 14.0, Color32::from_rgb(30, 31, 35));
        painter.rect_stroke(rect, 14.0, Stroke::new(1.0, Color32::from_rgb(58, 60, 66)), egui::StrokeKind::Inside);
        let m = w * 0.045;
        let now = self.clock();
        let mut clicked = None;

        // Potentiomètres : 2 rangées de 3, en haut à gauche.
        let knob_area = Rect::from_min_size(rect.min + Vec2::splat(m), Vec2::new(w * 0.52, w * 0.27));
        let kr = (knob_area.width() / 3.0).min(knob_area.height() / 2.0) * 0.27;
        for index in 0..midi::KNOBS as u8 {
            let c = Control::Knob { bank: self.midi.knob_bank, index };
            let (col, row) = (index % 3, index / 3);
            let center = egui::pos2(
                knob_area.min.x + knob_area.width() * (col as f32 + 0.5) / 3.0,
                knob_area.min.y + knob_area.height() * (row as f32 + 0.38) / 2.0,
            );
            let sel = self.midi.selected == Some(c);
            let bound = self.midi_setup.action(c).clone();
            painter.circle_filled(center, kr * 1.18, Color32::from_rgb(20, 20, 23));
            painter.circle_filled(center, kr, Color32::from_rgb(52, 54, 60));
            // Repère : de -135° à +135° selon la dernière valeur reçue.
            let v = self.midi.values.get(&c).copied().map_or(0.0, |v| f32::from(v) / 127.0);
            let a = (-135.0 + 270.0 * v).to_radians();
            let dir = Vec2::new(a.sin(), -a.cos());
            painter.line_segment([center + dir * kr * 0.25, center + dir * kr * 0.95], Stroke::new(2.0, Color32::WHITE));
            if sel {
                painter.circle_stroke(center, kr * 1.35, Stroke::new(2.0, ACCENT));
            } else if bound != Action::None {
                painter.circle_stroke(center, kr * 1.3, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)));
            }
            let label = if bound == Action::None { format!("K{}", c_num(c)) } else { bound.short() };
            painter.text(
                center + Vec2::new(0.0, kr * 1.45),
                egui::Align2::CENTER_TOP,
                elide(&label, 13),
                egui::FontId::proportional(10.0),
                if bound == Action::None { TEXT_WEAK } else { ACCENT },
            );
            let hit = Rect::from_center_size(center + Vec2::new(0.0, kr * 0.3), Vec2::new(kr * 2.8, kr * 3.2));
            let r = ui.interact(hit, ui.scope_id().with(("midi", c)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            if r.clicked() {
                clicked = Some(c);
            }
        }

        // Marque, banques (clic : changer de banque sur le dessin).
        let right = Rect::from_min_max(egui::pos2(rect.min.x + w * 0.62, rect.min.y + m), egui::pos2(rect.max.x - m, rect.min.y + w * 0.3));
        painter.text(right.left_top(), egui::Align2::LEFT_TOP, "AKAI", egui::FontId::proportional(w * 0.035), Color32::from_rgb(220, 60, 50));
        painter.text(
            right.left_top() + Vec2::new(0.0, w * 0.04),
            egui::Align2::LEFT_TOP,
            "MPD218",
            egui::FontId::proportional(w * 0.06),
            TEXT,
        );
        let bank_row = |y: f32, name: &str, current: u8, ui: &mut egui::Ui| -> Option<u8> {
            painter.text(egui::pos2(right.min.x, y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(10.0), TEXT_WEAK);
            let mut pick = None;
            for b in 0..midi::BANKS as u8 {
                let r = Rect::from_center_size(egui::pos2(right.min.x + w * 0.16 + b as f32 * w * 0.065, y), Vec2::splat(w * 0.05));
                let on = b == current;
                painter.rect_filled(r, 4.0, if on { Color32::from_rgb(230, 80, 60) } else { Color32::from_rgb(52, 54, 60) });
                painter.text(r.center(), egui::Align2::CENTER_CENTER, (b'A' + b) as char, egui::FontId::proportional(10.0), TEXT);
                if ui.interact(r, ui.scope_id().with((name, b)), Sense::click()).on_hover_text(format!("Banque {}", (b'A' + b) as char)).clicked() {
                    pick = Some(b);
                }
            }
            pick
        };
        if let Some(b) = bank_row(right.min.y + w * 0.15, "PAD BANK", self.midi.pad_bank, ui) {
            self.midi.pad_bank = b;
        }
        if let Some(b) = bank_row(right.min.y + w * 0.22, "CTRL BANK", self.midi.knob_bank, ui) {
            self.midi.knob_bank = b;
        }

        // Pads : 4 × 4, le pad 1 en bas à gauche.
        let pads = Rect::from_min_max(egui::pos2(rect.min.x + m, rect.min.y + w * 0.33), rect.max - Vec2::splat(m));
        let gap = w * 0.02;
        let pw = (pads.width() - 3.0 * gap) / 4.0;
        let ph = (pads.height() - 3.0 * gap) / 4.0;
        for index in 0..midi::PADS as u8 {
            let c = Control::Pad { bank: self.midi.pad_bank, index };
            let (col, row_from_bottom) = (index % 4, index / 4);
            let r = Rect::from_min_size(
                egui::pos2(pads.min.x + col as f32 * (pw + gap), pads.max.y - (row_from_bottom as f32 + 1.0) * ph - row_from_bottom as f32 * gap),
                Vec2::new(pw, ph),
            );
            // Pad frappé : il s'allume en rouge, selon la vélocité, puis s'éteint.
            let glow = self.midi.hits.get(&c).map_or(0.0, |(t, vel)| {
                let age = (now - t) as f32;
                (1.0 - age / 0.45).max(0.0) * (0.35 + 0.65 * f32::from(*vel) / 127.0)
            });
            if glow > 0.0 {
                ui.ctx().request_repaint();
            }
            let base = Color32::from_rgb(58, 60, 66);
            let lit = Color32::from_rgb(235, 60, 40);
            let fill = lerp_color(base, lit, glow);
            painter.rect_filled(r, 6.0, fill);
            let sel = self.midi.selected == Some(c);
            let bound = self.midi_setup.action(c).clone();
            if sel {
                painter.rect_stroke(r, 6.0, Stroke::new(2.5, ACCENT), egui::StrokeKind::Inside);
            } else if bound != Action::None {
                painter.rect_stroke(r, 6.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), egui::StrokeKind::Inside);
            }
            painter.text(
                r.left_top() + Vec2::new(5.0, 4.0),
                egui::Align2::LEFT_TOP,
                format!("PAD {}", c_num(c)),
                egui::FontId::proportional(9.0),
                TEXT_WEAK,
            );
            if bound != Action::None {
                let galley = painter.layout(
                    bound.short(),
                    egui::FontId::proportional(11.0),
                    ACCENT,
                    r.width() - 8.0,
                );
                painter.galley(egui::pos2(r.min.x + 4.0, r.max.y - galley.size().y - 4.0), galley, ACCENT);
            }
            let resp = ui.interact(r, ui.scope_id().with(("midi", c)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                clicked = Some(c);
            }
        }
        if let Some(c) = clicked {
            self.midi.selected = Some(c);
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
        let media: Vec<String> = self.project.media.iter().map(|p| file_name(p)).collect();
        theme::card().show(ui, |ui| {
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
