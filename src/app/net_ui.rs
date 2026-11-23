//! Synchronisation de plusieurs OZ Mapping sur le réseau local : le projet (calques, effets,
//! masques, scène 3D, LEDs…), la lecture des vidéos et les boutons du direct se répercutent d'une
//! machine à l'autre ; les médias absents sont copiés.
//!
//! Le projet circule sous forme JSON « de référence » : les chemins des médias y sont ceux de la
//! machine qui les a ajoutés ; chaque machine les remplace par ses fichiers locaux (le même
//! chemin s'il existe, sinon la copie reçue). L'écran choisi pour chaque sortie reste propre à
//! chaque machine.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::*;
use crate::net::{self, Live, LiveLayer, Msg, Net, NetSettings, Ver};

/// Modifications envoyées au plus toutes les 60 ms ; état de la lecture toutes les 200 ms.
const FLUSH_SECS: f32 = 0.06;
const LIVE_SECS: f32 = 0.2;
/// Décalage de lecture toléré avant de recaler une vidéo sur celle de la machine qui la pilote.
const DRIFT_SECS: f64 = 0.35;

#[derive(Default)]
struct Track {
    /// Fichier (chemin de référence), vitesse, sens, pause.
    sig: (Option<String>, u32, bool, bool),
    pos: f64,
    at: Option<Instant>,
    ver: Ver,
    last_fix: Option<Instant>,
}

#[derive(Default)]
pub(super) struct NetRuntime {
    pub(super) settings: NetSettings,
    net: Option<Net>,
    error: Option<String>,
    /// Projet de référence tel que partagé, versions de ses morceaux, horloge.
    base: Option<Value>,
    shadow: Option<Project>,
    vers: HashMap<String, Ver>,
    clock: u64,
    last_flush: Option<Instant>,
    last_live: Option<Instant>,
    /// Le projet des autres a été repris (ou on est la première machine).
    joined: bool,
    /// Prendre telle quelle la prochaine lecture reçue de cette machine (on vient d'arriver).
    adopt_live: Option<u64>,
    tracks: Vec<Track>,
    flags_sig: [bool; 4],
    flags_ver: Ver,
    /// Médias : chemin de référence ↔ fichier local.
    canon_local: HashMap<String, PathBuf>,
    local_canon: HashMap<PathBuf, String>,
    fetching: HashSet<String>,
    failed: HashMap<String, String>,
    manual_input: String,
    /// Message d'information (machine liée, partie…), affiché quelques secondes.
    notice: Option<(String, Instant)>,
}

impl NetRuntime {
    fn reset(&mut self) {
        let settings = std::mem::take(&mut self.settings);
        let manual_input = std::mem::take(&mut self.manual_input);
        *self = NetRuntime { settings, manual_input, ..Default::default() };
    }

    fn tick(&mut self) -> Ver {
        self.clock += 1;
        (self.clock, self.net.as_ref().map_or(0, Net::id))
    }

    fn see(&mut self, v: Ver) {
        self.clock = self.clock.max(v.0);
    }

    fn canon_of(&self, p: &Path) -> String {
        self.local_canon.get(p).cloned().unwrap_or_else(|| p.to_string_lossy().into_owned())
    }

    fn local_of(&self, canon: &str) -> Option<PathBuf> {
        if let Some(p) = self.canon_local.get(canon) {
            return Some(p.clone());
        }
        let p = PathBuf::from(canon);
        p.exists().then_some(p)
    }
}

impl OzApp {
    pub(super) fn net_notice(&self) -> Option<&str> {
        self.net.notice.as_ref().filter(|(_, t)| t.elapsed().as_secs_f32() < 5.0).map(|(s, _)| s.as_str())
    }

    /// Cette machine suit une autre : les automatismes (AUTO, mix auto, enchaînement du tableau,
    /// visuels qui changent seuls) ne tournent que sur la machine arrivée la première.
    pub(super) fn net_follower(&self) -> bool {
        let Some(net) = &self.net.net else { return false };
        let me = (net.since(), net.id());
        net.peers().iter().any(|p| (p.since, p.id) < me)
    }

    /// Projet de référence : chemins des médias de référence, sans les écrans des sorties.
    fn net_canonical(&self) -> Value {
        let mut v = serde_json::to_value(&self.project).unwrap_or(Value::Null);
        let map = &self.net.local_canon;
        net::map_strings(&mut v, &mut |s| map.get(Path::new(s)).cloned());
        if let Some(outs) = v.get_mut("outputs").and_then(Value::as_array_mut) {
            for o in outs {
                o["screen"] = Value::Null;
            }
        }
        v
    }

    pub(super) fn net_tick(&mut self, ctx: &egui::Context) {
        // Marche / arrêt.
        if self.net.settings.enabled && self.net.net.is_none() {
            let repaint = ctx.clone();
            match Net::start(&self.net.settings, move || repaint.request_repaint()) {
                Ok(n) => {
                    self.net.reset();
                    self.net.net = Some(n);
                }
                Err(e) => {
                    self.net.error = Some(format!("{e:#}"));
                    self.net.settings.enabled = false;
                }
            }
        } else if !self.net.settings.enabled && self.net.net.is_some() {
            self.net.reset();
        }
        let Some(net) = &self.net.net else { return };
        let events: Vec<net::Event> = net.events.try_iter().collect();
        let alone = net.peers().is_empty();
        let now = Instant::now();

        // Nos modifications d'abord : elles entrent dans le projet de référence.
        if self.net.last_flush.is_none_or(|t| t.elapsed().as_secs_f32() >= FLUSH_SECS) {
            self.net.last_flush = Some(now);
            self.net_flush();
        }
        if alone && !self.net.joined {
            // Seule machine de la salle : c'est elle la référence.
            self.net.joined = true;
        }

        for e in events {
            match e {
                net::Event::Up { id, since, name } => {
                    self.net.notice = Some((format!("Réseau : {name} est liée"), Instant::now()));
                    let me = self.net.net.as_ref().map_or((0, 0), |n| (n.since(), n.id()));
                    // Arrivée dans une salle déjà active : on reprend le projet de la plus ancienne.
                    if (since, id) < me {
                        self.net.joined = false;
                        if let Some(n) = &self.net.net {
                            n.send_to(id, &Msg::Want);
                        }
                    }
                }
                net::Event::Down { name } => self.net.notice = Some((format!("Réseau : {name} est partie"), Instant::now())),
                net::Event::Msg { from, msg } => self.net_receive(ctx, from, msg),
                net::Event::File { canon, result } => {
                    self.net.fetching.remove(&canon);
                    match result {
                        Ok(local) => {
                            self.net.local_canon.insert(local.clone(), canon.clone());
                            self.net.canon_local.insert(canon, local);
                            self.net_apply(ctx, None, false);
                            // Une lecture attendait peut-être ce fichier : la redemander.
                            for t in &mut self.net.tracks {
                                t.ver = (0, 0);
                            }
                        }
                        Err(e) => {
                            self.net.failed.insert(canon, e);
                        }
                    }
                }
            }
        }

        if self.net.last_live.is_none_or(|t| t.elapsed().as_secs_f32() >= LIVE_SECS) {
            self.net.last_live = Some(now);
            let live = self.net_local_live();
            if let Some(n) = &self.net.net
                && !n.peers().is_empty()
            {
                n.send_all(&Msg::Live(live));
            }
        }
    }

    /// Envoie ce qui a changé dans le projet depuis la dernière fois.
    fn net_flush(&mut self) {
        if self.net.shadow.as_ref() == Some(&self.project) {
            return;
        }
        let cur = self.net_canonical();
        let changes = match &self.net.base {
            Some(base) => net::diff(base, &cur),
            None => Vec::new(),
        };
        if !changes.is_empty() && self.net.joined {
            let mut out = Vec::new();
            for (k, v) in changes {
                let ver = self.net.tick();
                self.net.vers.insert(k.clone(), ver);
                out.push((k, v, ver));
            }
            if let Some(n) = &self.net.net {
                n.send_all(&Msg::Patch { changes: out });
            }
        }
        // Médias que les autres peuvent demander.
        let mut files = HashMap::new();
        net::map_strings(&mut cur.clone(), &mut |s| {
            if net::looks_like_media(s)
                && let Some(local) = self.net.local_of(s)
            {
                files.insert(s.to_string(), local);
            }
            None
        });
        if let Some(n) = &self.net.net {
            n.set_files(files);
        }
        self.net.base = Some(cur);
        self.net.shadow = Some(self.project.clone());
    }

    fn net_receive(&mut self, ctx: &egui::Context, from: u64, msg: Msg) {
        match msg {
            Msg::Want => {
                self.net_flush();
                if let (Some(n), Some(base)) = (&self.net.net, &self.net.base) {
                    n.send_to(from, &Msg::Full { state: base.clone(), vers: self.net.vers.clone(), clock: self.net.clock });
                    let live = Live {
                        layers: self.net.tracks.iter().map(|t| LiveLayer {
                            path: t.sig.0.clone(),
                            pos: t.pos,
                            speed: f32::from_bits(t.sig.1),
                            reverse: t.sig.2,
                            paused: t.sig.3,
                            ver: t.ver,
                        }).collect(),
                        flags: self.net.flags_sig,
                        flags_ver: self.net.flags_ver,
                    };
                    n.send_to(from, &Msg::Live(live));
                }
            }
            Msg::Full { state, vers, clock } => {
                self.net.see((clock, 0));
                self.net.base = Some(state);
                self.net.vers = vers;
                self.net.joined = true;
                self.net.adopt_live = Some(from);
                self.net_apply(ctx, Some(from), true);
                self.net.notice = Some(("Réseau : projet repris de l'autre machine".into(), Instant::now()));
            }
            Msg::Patch { changes } => {
                if !self.net.joined {
                    return;
                }
                let mut applied = false;
                for (k, v, ver) in changes {
                    self.net.see(ver);
                    if ver > net::version_of(&self.net.vers, &k)
                        && let Some(base) = &mut self.net.base
                        && net::set_at(base, &k, v)
                    {
                        self.net.vers.insert(k, ver);
                        applied = true;
                    }
                }
                if applied {
                    self.net_apply(ctx, Some(from), false);
                }
            }
            Msg::Live(live) => self.net_apply_live(ctx, from, live),
            _ => {}
        }
    }

    /// Le projet de référence devient le projet de cette machine (avec ses fichiers locaux).
    fn net_apply(&mut self, ctx: &egui::Context, from: Option<u64>, full: bool) {
        let Some(mut v) = self.net.base.clone() else { return };
        let mut missing = Vec::new();
        {
            let rt = &self.net;
            net::map_strings(&mut v, &mut |s| {
                if let Some(p) = rt.canon_local.get(s) {
                    return Some(p.to_string_lossy().into_owned());
                }
                if net::looks_like_media(s) && !Path::new(s).exists() {
                    missing.push(s.to_string());
                }
                None
            });
        }
        if let Some(outs) = v.get_mut("outputs").and_then(Value::as_array_mut) {
            for (i, o) in outs.iter_mut().enumerate() {
                o["screen"] = serde_json::to_value(self.project.outputs.get(i).and_then(|d| d.screen.clone())).unwrap_or(Value::Null);
            }
        }
        match serde_json::from_value::<Project>(v) {
            Ok(mut p) => {
                p.sanitize();
                let relayout = full || p.layers.len() != self.project.layers.len();
                self.project = p;
                self.net.shadow = Some(self.project.clone());
                // Une transition d'effets en cours réécrirait les effets reçus.
                for rt in &mut self.layers {
                    rt.fx_anim = None;
                }
                if relayout {
                    self.after_history(ctx);
                }
            }
            Err(e) => self.status = Some(format!("Réseau : projet reçu illisible ({e})")),
        }
        // Médias absents : copie depuis la machine qui les a (déjà copiés : réutilisés).
        for canon in missing {
            if self.net.fetching.contains(&canon) || self.net.failed.contains_key(&canon) {
                continue;
            }
            let dest = net::cache_path(&canon);
            if dest.exists() {
                self.net.local_canon.insert(dest.clone(), canon.clone());
                self.net.canon_local.insert(canon, dest);
                continue;
            }
            let source = from.or_else(|| self.net.net.as_ref().and_then(|n| n.peers().first().map(|p| p.id)));
            if let (Some(n), Some(src)) = (&self.net.net, source) {
                self.net.fetching.insert(canon.clone());
                n.fetch(src, canon, dest);
            }
        }
    }

    /// Lecture de cette machine ; une lecture changée ici (média lancé, pause, vitesse, saut)
    /// prend une nouvelle version, que les autres suivront.
    fn net_local_live(&mut self) -> Live {
        let id = self.net.net.as_ref().map_or(0, Net::id);
        let now = Instant::now();
        self.net.tracks.resize_with(self.layers.len(), Track::default);
        let mut layers = Vec::new();
        for i in 0..self.layers.len() {
            let (path, pos, speed, reverse, paused) = match self.layers[i].active() {
                Some(p) => (Some(self.net.canon_of(&p.path)), p.position(), p.speed(), p.is_reverse(), p.is_paused()),
                None => (None, 0.0, 1.0, false, false),
            };
            let sig = (path.clone(), speed.to_bits(), reverse, paused);
            let t = &self.net.tracks[i];
            let expected = match t.at {
                Some(at) if !t.sig.3 => {
                    let dir = if t.sig.2 { -1.0 } else { 1.0 };
                    t.pos + at.elapsed().as_secs_f64() * f64::from(f32::from_bits(t.sig.1)) * dir
                }
                _ => t.pos,
            };
            // Saut dans la vidéo (sauf retour au début d'une boucle).
            let jumped = t.at.is_some() && path.is_some() && sig == t.sig && (pos - expected).abs() > 1.5 && pos > 1.5;
            let changed = sig != t.sig || jumped;
            if changed {
                self.net.clock += 1;
                let ver = (self.net.clock, id);
                self.net.tracks[i].ver = ver;
            }
            let t = &mut self.net.tracks[i];
            t.sig = sig;
            t.pos = pos;
            t.at = Some(now);
            layers.push(LiveLayer { path, pos, speed, reverse, paused, ver: t.ver });
        }
        let flags = [self.blackout, self.white_screen, self.bypass_fx, self.grid];
        if flags != self.net.flags_sig {
            self.net.flags_sig = flags;
            self.net.flags_ver = self.net.tick();
        }
        Live { layers, flags, flags_ver: self.net.flags_ver }
    }

    fn net_apply_live(&mut self, ctx: &egui::Context, from: u64, live: Live) {
        let adopt = self.net.adopt_live == Some(from);
        if adopt {
            self.net.adopt_live = None;
        }
        if !self.net.joined {
            return;
        }
        self.net.see(live.flags_ver);
        if adopt || live.flags_ver > self.net.flags_ver {
            [self.blackout, self.white_screen, self.bypass_fx, self.grid] = live.flags;
            self.net.flags_sig = live.flags;
            self.net.flags_ver = live.flags_ver;
        }
        self.net.tracks.resize_with(self.layers.len(), Track::default);
        let now = Instant::now();
        for (i, l) in live.layers.into_iter().enumerate().take(self.layers.len()) {
            self.net.see(l.ver);
            let mine = &self.net.tracks[i];
            let newer = adopt || l.ver > mine.ver;
            // Même version, envoyée par la machine qui pilote ce calque : simple recalage.
            let owner = !newer && l.ver == mine.ver && l.ver.1 == from && l.ver != (0, 0);
            if !newer && !owner {
                continue;
            }
            if newer {
                let local = l.path.as_deref().map(|c| self.net.local_of(c));
                match local {
                    Some(Some(local)) => {
                        if self.layers[i].active().map(|p| &p.path) != Some(&local) {
                            let opts = video::PlayOptions::new(self.project.looping, video::OUTPUT_MAX);
                            self.launch_with(ctx, i, &local, opts, None, false);
                        }
                    }
                    // Fichier pas encore là (copie en cours) : on réessaiera au prochain envoi.
                    Some(None) => continue,
                    None => {
                        if self.layers[i].active().is_some() {
                            self.layers[i].stop();
                        }
                    }
                }
            }
            let mut pos = l.pos;
            if let Some(p) = self.layers[i].active() {
                p.set_speed(l.speed);
                p.set_reverse(l.reverse);
                p.set_paused(l.paused);
                let t = &self.net.tracks[i];
                let cool = t.last_fix.is_none_or(|f| f.elapsed().as_secs_f32() > 1.5);
                if (p.position() - l.pos).abs() > DRIFT_SECS && (newer || cool) {
                    p.seek(l.pos);
                    self.net.tracks[i].last_fix = Some(now);
                } else {
                    pos = p.position();
                }
            }
            let t = &mut self.net.tracks[i];
            t.sig = (l.path, l.speed.to_bits(), l.reverse, l.paused);
            t.pos = pos;
            t.at = Some(now);
            t.ver = l.ver;
        }
    }

    /// Bouton « Réseau » de la barre du haut.
    pub(super) fn net_widget(&mut self, ui: &mut egui::Ui) {
        let peers = self.net.net.as_ref().map(Net::peers).unwrap_or_default();
        let label = match &self.net.net {
            Some(_) if peers.is_empty() => with_icon(ic::WIFI_HIGH, "Réseau · seul"),
            Some(_) => with_icon(ic::WIFI_HIGH, &format!("Réseau · {}", peers.len() + 1)),
            None => with_icon(ic::WIFI_HIGH, "Réseau"),
        };
        let resp = ui.menu_button(label, |ui| {
            ui.set_min_width(340.0);
            ui.label(egui::RichText::new("Plusieurs OZ Mapping ensemble").strong());
            ui.label(
                egui::RichText::new(
                    "Une modification faite sur une machine se répercute sur les autres de la même salle : calques, effets, masques, tableau, scène 3D, LEDs, lecture des vidéos, blackout. Les médias absents sont copiés.",
                )
                .small()
                .color(TEXT_WEAK),
            );
            ui.add_space(4.0);
            let editable = self.net.net.is_none();
            ui.add_enabled_ui(editable, |ui| {
                egui::Grid::new("net_settings").num_columns(2).show(ui, |ui| {
                    ui.label("Nom de cette machine");
                    ui.text_edit_singleline(&mut self.net.settings.name);
                    ui.end_row();
                    ui.label("Salle");
                    ui.text_edit_singleline(&mut self.net.settings.room)
                        .on_hover_text("Seules les machines de la même salle se synchronisent");
                    ui.end_row();
                });
            });
            ui.checkbox(&mut self.net.settings.enabled, "Synchroniser sur le réseau local");
            if let Some(e) = &self.net.error {
                ui.label(egui::RichText::new(e).small().color(theme::DANGER));
            }
            let Some(n) = &self.net.net else { return };
            ui.separator();
            let ips: Vec<String> = net::local_ips().iter().map(|ip| format!("{ip}:{}", n.port())).collect();
            ui.label(egui::RichText::new(format!("Cette machine : {}", ips.join(", "))).small().color(TEXT_WEAK));
            if peers.is_empty() {
                ui.label(egui::RichText::new(format!("Recherche des autres machines de la salle « {} »…", self.net.settings.room)).color(WARN));
            }
            let me = (n.since(), n.id());
            for p in &peers {
                let lead = peers.iter().all(|q| (q.since, q.id) >= (p.since, p.id)) && (p.since, p.id) < me;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(ic::CIRCLE).color(ACCENT));
                    ui.label(&p.name);
                    ui.label(egui::RichText::new(p.addr.ip().to_string()).small().color(TEXT_WEAK));
                    if lead {
                        ui.label(egui::RichText::new("référence").small().color(TEXT_WEAK))
                            .on_hover_text("Arrivée la première : les automatismes (AUTO, mix auto, enchaînement) tournent sur elle");
                    }
                });
            }
            if !peers.is_empty() && !self.net_follower() {
                ui.label(egui::RichText::new("Cette machine est la référence : les automatismes tournent ici.").small().color(TEXT_WEAK));
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.net.manual_input).hint_text("192.168.1.20").desired_width(150.0));
                if ui.button("Se lier").on_hover_text("Si les machines ne se trouvent pas seules (réseau qui bloque la diffusion)").clicked() {
                    let addr = self.net.manual_input.trim().to_string();
                    if !addr.is_empty() {
                        n.connect(&addr);
                        if !self.net.settings.manual.contains(&addr) {
                            self.net.settings.manual.push(addr);
                        }
                        self.net.manual_input.clear();
                    }
                }
            });
            for (canon, got, size) in n.progress() {
                let name = canon.rsplit(['/', '\\']).next().unwrap_or(&canon).to_string();
                let pct = if size > 0 { got as f32 / size as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(pct).text(format!("Copie de {name}")).desired_width(320.0));
            }
            for (canon, e) in &self.net.failed {
                let name = canon.rsplit(['/', '\\']).next().unwrap_or(canon);
                ui.label(egui::RichText::new(format!("{name} : {e}")).small().color(theme::DANGER));
            }
            ui.separator();
            ui.label(
                egui::RichText::new(
                    "Chaque machine garde son écran de sortie, son contrôleur MIDI et ses envois aux WLED, en DMX et aux lasers : activez ces envois sur une seule machine.",
                )
                .small()
                .color(TEXT_WEAK),
            );
        });
        if self.net.net.is_some() {
            resp.response.highlight();
        }
    }
}
