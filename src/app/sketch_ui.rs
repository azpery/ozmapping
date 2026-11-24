//! Dessin depuis un iPad ou un téléphone : onglet « Dessin » et source des calques qui l'affichent.

use super::*;
use crate::midi::ShowMode;
use crate::sketch::{Remote, Sketch};

/// Image de la sortie envoyée aux pages : cible de lecture à part, et cadence.
const SCREEN_SLOT: usize = usize::MAX - 64;
const SCREEN_EVERY: f64 = 1.0 / 20.0;
const SCREEN_W: u32 = 1280;

/// Taille d'un module du QR code, en pixels de texture.
const QR_PX: usize = 6;

#[derive(Default)]
pub(super) struct SketchRuntime {
    pub(super) server: Option<Sketch>,
    error: Option<String>,
    /// QR code de l'adresse, avec l'adresse qui l'a produit.
    qr: Option<(String, egui::TextureHandle)>,
    /// Dernier envoi de l'état à l'onglet Live (horloge de l'app).
    live_at: f64,
    /// Image de la sortie lue pour les pages, numéro de la dernière transmise, heure de la demande.
    screen: Arc<Mutex<crate::leds::Frame>>,
    screen_serial: u64,
    screen_at: f64,
    /// Fond et renouvellement choisis sur la page, appliqués au bassin quand il est créé.
    chem_settings: Vec<crate::chem::ChemMsg>,
    /// Sortie choisie sur l'iPad pour le dessin et la chimie (None : celle du calque choisi).
    output: Option<usize>,
    /// Dernier envoi de la salle 3D (horloge de l'app).
    room_at: f64,
    /// Dernier message envoyé à l'iPad (horloge de l'app).
    notice_at: f64,
    /// Le serveur a reçu le dessin du projet ; dernière version du dessin reportée dans le projet.
    pub(super) doc_synced: bool,
    doc_version: u64,
    /// Depuis quand (horloge de l'app) le serveur ne sert plus à rien : onglet fermé, aucun
    /// appareil connecté, rien de dessiné.
    idle_since: Option<f64>,
}

/// Délai avant d'arrêter un serveur inutile (le temps qu'un iPad qui vient de scanner se connecte).
const IDLE_STOP_SECS: f64 = 30.0;

impl OzApp {
    /// Serveur lancé à l'ouverture de l'onglet Dessin (c'est là qu'on scanne le QR code), et
    /// arrêté quand il ne sert plus : onglet fermé, aucun appareil connecté, rien de dessiné.
    pub(super) fn sketch_tick(&mut self, ctx: &egui::Context) {
        // OZ_SKETCH=1 : serveur toujours lancé (installation sans écran de contrôle, essais).
        // Dessin enregistré qu'un calque montre : le serveur le dessine (même sans iPad).
        let has_drawing = self.project.drawing.as_ref().is_some_and(|d| !d.is_empty()) && self.project.layers.iter().any(|l| l.sketch);
        let wanted = self.right_tab == RightTab::Sketch || std::env::var_os("OZ_SKETCH").is_some() || has_drawing;
        if let Some(server) = &self.sketch.server {
            let useless = !wanted && server.clients() == 0 && server.strokes() == 0 && server.stickers() == 0;
            let now = self.clock();
            match (useless, self.sketch.idle_since) {
                (false, _) => self.sketch.idle_since = None,
                (true, None) => self.sketch.idle_since = Some(now),
                (true, Some(since)) if now - since >= IDLE_STOP_SECS => {
                    self.sketch.server = None;
                    self.sketch.idle_since = None;
                    self.sketch.qr = None;
                }
                _ => {}
            }
            if self.sketch.idle_since.is_some() {
                // Pour revenir vérifier sans attendre une autre raison de redessiner.
                ctx.request_repaint_after(std::time::Duration::from_secs(5));
            }
        }
        if wanted && self.sketch.server.is_none() && self.sketch.error.is_none() {
            let repaint = ctx.clone();
            match Sketch::start(move || repaint.request_repaint()) {
                Ok(s) => {
                    self.sketch.server = Some(s);
                    // Nouveau serveur : il reprend le dessin du projet.
                    self.sketch.doc_synced = false;
                    self.sketch.doc_version = 0;
                }
                Err(e) => self.sketch.error = Some(format!("{e:#}")),
            }
        }
        let Some(server) = &self.sketch.server else { return };
        let p = &self.project;
        server.set_shown((0..p.layers.len()).any(|i| p.layers[i].sketch && p.layer_visible(i) && p.layer_opacity(i) > 0.0));
        server.set_beat(self.beat_env());
        server.set_media(&self.media_folders.all);
        // Dessin : celui du projet donné au serveur (ouverture, redémarrage), puis chaque
        // modification faite sur l'iPad reportée dans le projet (enregistrée avec lui).
        if !self.sketch.doc_synced {
            self.sketch.doc_synced = true;
            self.sketch.doc_version = server.drawing().0;
            if let Some(doc) = self.project.drawing.clone().filter(|d| !d.is_empty()) {
                server.load_drawing(doc);
            }
        } else {
            let (version, doc) = server.drawing();
            if version > self.sketch.doc_version {
                self.sketch.doc_version = version;
                let doc = doc.map(|d| (*d).clone()).filter(|d| !d.is_empty());
                if self.project.drawing != doc {
                    self.project.drawing = doc;
                }
            }
        }
        let remote = server.take_remote();
        for r in remote {
            match r {
                Remote::Trigger(a, on) => self.midi_trigger(ctx, &a, on),
                Remote::Value(a, v) => self.midi_value(&a, v),
                Remote::Photo { path } => {
                    self.add_media([path.clone()]);
                    let layer = self.content_layer();
                    self.launch_any(ctx, layer, &MediaDrag::File(path));
                    self.status = Some("Photo reçue de l'iPad, affichée sur le calque".into());
                }
                Remote::Object { photo, mask, with_photo, calibrate } => self.map_ipad_object(ctx, &photo, &mask, with_photo, calibrate),
                Remote::DrawingUsed => {
                    self.sketch_layer();
                }
                Remote::Room(m) => {
                    crate::room::apply(&mut self.project.leds, &m);
                    // LEDs recalculées tout de suite : la vue 3D de cette image ne doit pas
                    // chercher un bandeau supprimé.
                    self.refresh_led_cache();
                    // Scène renvoyée tout de suite (l'iPad voit le résultat).
                    self.sketch.room_at = f64::MIN;
                }
                Remote::Chem(m) => {
                    use crate::chem::ChemMsg;
                    // Seul un ingrédient versé met la chimie sur un calque (s'il n'y en a pas) ; le
                    // fond, le renouvellement, le son (renvoyés à chaque connexion de la page)
                    // attendent qu'un calque la montre.
                    if matches!(m, ChemMsg::Base { .. } | ChemMsg::Renew { .. } | ChemMsg::Sound { .. } | ChemMsg::Listen { .. }) {
                        self.sketch.chem_settings.retain(|s| std::mem::discriminant(s) != std::mem::discriminant(&m));
                        self.sketch.chem_settings.push(m.clone());
                    }
                    let existing = self.project.layers.iter().position(|l| l.generator.pattern == Pattern::Chemistry);
                    let layer = if matches!(m, ChemMsg::Drop { .. }) { self.chem_layer() } else { existing };
                    if let Some(i) = layer {
                        if existing.is_none() {
                            for s in self.sketch.chem_settings.clone() {
                                self.layers[i].chem.apply(&s);
                            }
                        }
                        self.layers[i].chem.apply(&m);
                    }
                }
            }
        }
        // Nouvelle image de la sortie : aux pages.
        let frame = {
            let f = self.sketch.screen.lock().unwrap();
            (f.serial != self.sketch.screen_serial && !f.pixels.is_empty()).then(|| (f.serial, f.pixels.clone(), f.size))
        };
        if let Some((serial, pixels, size)) = frame {
            self.sketch.screen_serial = serial;
            if let Some(server) = &self.sketch.server {
                server.set_screen(pixels, size);
            }
        }
        let now = self.clock();
        if now - self.sketch.live_at >= 0.1 {
            self.sketch.live_at = now;
            let state = self.live_state().to_string();
            if let Some(server) = &self.sketch.server {
                server.set_live(state);
            }
        }
        if now - self.sketch.room_at >= 0.25 && self.sketch.server.as_ref().is_some_and(|s| s.wants_room()) {
            self.sketch.room_at = now;
            let room = crate::room::room_json(&self.project.leds);
            if let Some(server) = &self.sketch.server {
                server.set_room(room);
            }
        }
        let Some(server) = &self.sketch.server else { return };
        // Proportions de la sortie du premier calque qui affiche le dessin.
        let size = self.out_size(self.sketch_output());
        server.set_aspect(size.x / size.y.max(1.0));
    }

    /// On dessine depuis la page : le calque qui montre le dessin, rendu visible ; s'il n'y en a
    /// pas, le dessin va sur un calque vide (aucun calque n'est ajouté : le dessin se pose dans
    /// une case, comme un média).
    pub(super) fn sketch_layer(&mut self) -> Option<usize> {
        if let Some(i) = self.project.layers.iter().position(|l| l.sketch) {
            self.reveal(i);
            return Some(i);
        }
        let Some(i) = self.free_layer() else {
            self.ipad_notice("Rien n'affiche le dessin : posez « Dessin (iPad) » sur une case du tableau (liste des visuels).");
            return None;
        };
        self.stop_layer(i);
        let cfg = &mut self.project.layers[i];
        cfg.mix.enabled = false;
        cfg.generator.pattern = Pattern::Off;
        cfg.sketch = true;
        self.reveal(i);
        self.status = Some(format!("Dessin de l'iPad sur le calque « {} » (il était vide)", self.project.layers[i].name));
        Some(i)
    }

    /// Calque libre pour un contenu de l'iPad : le calque choisi s'il ne montre rien, sinon le
    /// premier calque vide (de la sortie de l'iPad d'abord).
    fn free_layer(&self) -> Option<usize> {
        let empty = |i: usize| {
            let l = &self.project.layers[i];
            !l.sketch && l.generator.pattern == Pattern::Off && self.layers[i].active().is_none()
        };
        let n = self.project.layers.len();
        let sel = self.sel();
        if sel < n && empty(sel) {
            return Some(sel);
        }
        let out = self.sketch_output();
        (0..n).find(|&i| empty(i) && self.project.layers[i].output == out).or_else(|| (0..n).find(|&i| empty(i)))
    }

    /// Calque rendu visible (on agit dessus depuis l'iPad, on veut le voir).
    fn reveal(&mut self, i: usize) {
        let l = &mut self.project.layers[i];
        if !l.visible || l.opacity <= 0.0 {
            l.visible = true;
            l.opacity = l.opacity.max(1.0);
        }
    }

    /// Message à l'iPad (au plus un toutes les 5 s).
    fn ipad_notice(&mut self, text: &str) {
        let now = self.clock();
        if now - self.sketch.notice_at < 5.0 {
            return;
        }
        self.sketch.notice_at = now;
        self.status = Some(text.to_owned());
        if let Some(server) = &self.sketch.server {
            server.notice(text.to_owned());
        }
    }

    /// Bassins de chimie : celui du projet repris à l'ouverture, puis une photo du bassin
    /// toutes les 20 s, enregistrée à part (PNG) et notée dans le calque.
    pub(super) fn chem_persist_tick(&mut self) {
        const EVERY: f64 = 20.0;
        let now = self.clock();
        for i in 0..self.project.layers.len() {
            let cfg = &mut self.project.layers[i];
            let rt = &mut self.layers[i];
            let chem = cfg.generator.pattern == Pattern::Chemistry || rt.gen_shown.0 == Pattern::Chemistry;
            if !chem {
                continue;
            }
            if !rt.chem_restored {
                rt.chem_restored = true;
                rt.chem_saved_at = now;
                if let Some(saved) = &cfg.chem {
                    rt.chem.apply_settings(&saved.settings);
                    if let Some(f) = saved.file.as_ref().filter(|f| f.exists()) {
                        match crate::chem::Basin::load_png(f) {
                            Ok(b) => rt.chem.restore(b),
                            Err(e) => eprintln!("chimie : bassin illisible ({e:#})"),
                        }
                    }
                }
            }
            if now - rt.chem_saved_at >= EVERY {
                rt.chem_saved_at = now;
                rt.chem.request_snapshot();
            }
            if let Some(basin) = rt.chem.take_snapshot() {
                let file = cfg.chem.as_ref().and_then(|c| c.file.clone()).unwrap_or_else(|| {
                    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
                    crate::sketch::data_dir().unwrap_or_else(std::env::temp_dir).join("Chimie").join(format!("bassin-{stamp}.png"))
                });
                let settings = rt.chem.settings();
                let saved = project::ChemSaved { file: Some(file.clone()), settings };
                if cfg.chem.as_ref() != Some(&saved) {
                    cfg.chem = Some(saved);
                }
                // Écrit à part (quelques Mo) : l'image n'attend pas.
                std::thread::spawn(move || {
                    let tmp = file.with_extension("tmp.png");
                    if basin.save_png(&tmp).is_ok() {
                        let _ = std::fs::rename(&tmp, &file);
                    }
                });
            }
        }
    }

    /// Genre d'un calque, pour les modes de la sortie : dessin, chimie, visuel généré (sans
    /// vidéo dessous), sinon médias.
    pub(super) fn layer_kind(&self, i: usize) -> ShowMode {
        let cfg = &self.project.layers[i];
        if cfg.sketch {
            ShowMode::Drawing
        } else if cfg.generator.pattern == Pattern::Chemistry {
            ShowMode::Chemistry
        } else if cfg.generator.pattern != Pattern::Off && self.layers[i].active().is_none() {
            ShowMode::Visual
        } else {
            ShowMode::Media
        }
    }

    /// Mode de la sortie : les calques de ce genre seuls (créés pour le dessin et la chimie
    /// s'il n'y en a pas), ou tous.
    pub(super) fn show_mode(&mut self, mode: ShowMode) {
        match mode {
            ShowMode::Drawing if !self.project.layers.iter().any(|l| l.sketch) => {
                if self.sketch_layer().is_none() {
                    return;
                }
            }
            ShowMode::Chemistry if !self.project.layers.iter().any(|l| l.generator.pattern == Pattern::Chemistry) => {
                if self.chem_layer().is_none() {
                    return;
                }
            }
            _ => {}
        }
        let n = self.project.layers.len();
        if mode != ShowMode::All && !(0..n).any(|i| self.layer_kind(i) == mode) {
            self.status = Some(format!("Mode « {} » : aucun calque de ce genre", mode.name()));
            return;
        }
        for i in 0..n {
            let show = mode == ShowMode::All || self.layer_kind(i) == mode;
            let cfg = &mut self.project.layers[i];
            cfg.visible = show;
            if show && cfg.opacity <= 0.0 {
                cfg.opacity = 1.0;
            }
            // Un calque montré dans un groupe caché : le groupe se montre aussi.
            let g = cfg.group;
            if show && g != 0 {
                if let Some(grp) = self.project.groups.iter_mut().find(|x| x.id == g) {
                    grp.visible = true;
                    if grp.opacity <= 0.0 {
                        grp.opacity = 1.0;
                    }
                }
            }
        }
        self.status = Some(format!("Sortie : {}", mode.name()));
    }

    /// Mode actuel : un seul genre de calque visible, sinon « Tout ».
    fn current_mode(&self) -> ShowMode {
        let mut seen = None;
        for i in 0..self.project.layers.len() {
            if !self.project.layer_visible(i) || self.project.layer_opacity(i) <= 0.0 {
                continue;
            }
            let k = self.layer_kind(i);
            match seen {
                None => seen = Some(k),
                Some(s) if s != k => return ShowMode::All,
                _ => {}
            }
        }
        seen.unwrap_or(ShowMode::All)
    }

    /// Calque du bassin : le premier qui montre la chimie, rendu visible ; sinon la chimie va
    /// sur un calque vide (aucun calque n'est ajouté : la chimie se pose dans une case).
    fn chem_layer(&mut self) -> Option<usize> {
        if let Some(i) = self.project.layers.iter().position(|l| l.generator.pattern == Pattern::Chemistry) {
            self.reveal(i);
            return Some(i);
        }
        let Some(i) = self.free_layer() else {
            self.ipad_notice("Rien n'affiche la chimie : posez « Chimie (iPad) » sur une case du tableau (liste des visuels).");
            return None;
        };
        self.stop_layer(i);
        let cfg = &mut self.project.layers[i];
        cfg.mix.enabled = false;
        cfg.sketch = false;
        cfg.generator.pattern = Pattern::Chemistry;
        cfg.generator.mix = 1.0;
        cfg.generator.auto = false;
        self.reveal(i);
        self.status = Some(format!("Chimie de l'iPad sur le calque « {} » (il était vide)", self.project.layers[i].name));
        Some(i)
    }

    /// Sortie du dessin : celle du premier calque qui l'affiche.
    fn sketch_output(&self) -> usize {
        let n_out = self.project.outputs.len().max(1);
        let layer = |chem: bool| {
            self.project.layers.iter().find(|l| if chem { l.generator.pattern == Pattern::Chemistry } else { l.sketch }).map(|l| l.output)
        };
        layer(false).or(self.sketch.output).or(layer(true)).unwrap_or(0).min(n_out - 1)
    }

    /// Sortie où l'iPad projette : le dessin et la chimie y vont, son aperçu la montre.
    pub(super) fn set_ipad_output(&mut self, k: usize) {
        if k >= self.project.outputs.len() {
            return;
        }
        self.sketch.output = Some(k);
        for l in &mut self.project.layers {
            if l.sketch || l.generator.pattern == Pattern::Chemistry {
                l.output = k;
            }
        }
        let name = self.project.outputs[k].name.clone();
        self.status = Some(format!("iPad : dessin et chimie sur « {name} »"));
    }

    /// Fait lire l'image de la sortie (30 fois par seconde) tant qu'une page veut la voir
    /// (option « Toute la sortie » ; sinon la page ne montre que le dessin).
    /// `painted` : sortie affichée par la fenêtre qui appelle (None : fenêtre principale).
    /// La lecture se fait dans la fenêtre de cette sortie quand elle est ouverte : la fenêtre
    /// principale, cachée par une sortie en plein écran, n'est plus redessinée par macOS.
    pub(super) fn sketch_capture(&mut self, ui: &egui::Ui, painted: Option<usize>) {
        let Some(server) = &self.sketch.server else { return };
        let Some(r) = self.renderer.clone() else { return };
        let out = self.sketch_output();
        let shown = if out == 0 { self.output_open } else { self.outs.get(out).is_some_and(|o| o.open) };
        let here = match painted {
            Some(k) => k == out,
            None => !shown,
        };
        let now = self.clock();
        if !here || !server.wants_screen() || now - self.sketch.screen_at < SCREEN_EVERY {
            return;
        }
        self.sketch.screen_at = now;
        let o = self.out_size(out);
        let aspect = (o.x / o.y.max(1.0)).clamp(0.2, 5.0);
        let size = (SCREEN_W, ((SCREEN_W as f32 / aspect) as u32).clamp(8, 2048));
        let params = self.draw_params_for(out);
        let sink = self.sketch.screen.clone();
        ui.painter().add(renderer::led_capture_callback(ui.max_rect(), r, params, SCREEN_SLOT + out, size, sink));
    }

    /// État de l'app pour l'onglet Live de la page.
    fn live_state(&self) -> serde_json::Value {
        use serde_json::json;
        let sel = self.sel();
        // Effets et visuels : le calque choisi (dessin compris). Médias et lecture : le calque
        // de contenu (celui du dessin n'affiche pas de vidéo).
        let cfg = &self.project.layers[sel];
        let rt = &self.layers[sel];
        let content = self.content_layer();
        let ccfg = &self.project.layers[content];
        let crt = &self.layers[content];
        let player = crt.active();
        let playing = player.and_then(|p| self.media_folders.all.iter().position(|m| *m == p.path));
        let layers: Vec<_> = self
            .project
            .layers
            .iter()
            .map(|l| json!({ "name": l.name, "opacity": l.opacity, "visible": l.visible, "sketch": l.sketch, "group": l.group }))
            .collect();
        let columns: Vec<_> = (0..self.project.columns)
            .map(|k| {
                let name = self
                    .project
                    .layers
                    .iter()
                    .filter_map(|l| l.cells.get(k).and_then(Option::as_ref))
                    .map(|c| c.name.clone())
                    .find(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("Colonne {}", k + 1));
                json!({
                    "name": name,
                    "filled": self.column_filled(k),
                    "active": self.layers.iter().any(|r| r.cell == Some(k)),
                })
            })
            .collect();
        // Rangés par famille, comme dans l'app ; les presets mémorisés à la fin.
        let mut presets: Vec<_> = Vec::new();
        for (k, family) in crate::effects::PRESET_FAMILIES.iter().enumerate() {
            for p in self.builtin_presets.iter().filter(|p| crate::effects::preset_family(&p.name) == Some(k)) {
                presets.push(json!({ "name": p.name, "family": family }));
            }
        }
        let others = self.builtin_presets.iter().filter(|p| crate::effects::preset_family(&p.name).is_none());
        presets.extend(others.map(|p| json!({ "name": p.name, "family": "Autres" })));
        presets.extend(self.project.presets.iter().map(|p| json!({ "name": p.name, "family": "Mes presets" })));
        let visuals: Vec<_> = Pattern::by_family()
            .into_iter()
            .flat_map(|(family, patterns)| patterns.into_iter().map(move |p| json!({ "id": p, "name": p.label(), "family": family })))
            .collect();
        // Médias, ceux des dossiers suivis compris (avec leur dossier, pour les ranger sur l'iPad).
        // L'état part 10 fois par seconde : au plus 500 médias.
        let folder_of = |m: &Path| -> String {
            let root = self.project.media_folders.iter().filter(|d| m.starts_with(d)).max_by_key(|d| d.as_os_str().len());
            match (root, m.parent()) {
                (Some(r), Some(dir)) => {
                    let rel = dir.strip_prefix(r).unwrap_or(Path::new(""));
                    let base = file_name(r);
                    if rel.as_os_str().is_empty() { base } else { format!("{base}/{}", rel.to_string_lossy()) }
                }
                _ => String::new(),
            }
        };
        let media: Vec<_> = self
            .media_folders
            .all
            .iter()
            .take(500)
            .map(|m| json!({ "name": file_name(m), "photo": video::is_still_image(m), "folder": folder_of(m) }))
            .collect();
        let mut fx = cfg.effects;
        let params: Vec<_> = crate::midi::FX_PARAMS
            .iter()
            .enumerate()
            .filter_map(|(k, &(name, lo, hi))| {
                let v = *crate::midi::fx_param(&mut fx, k)?;
                Some(json!({ "name": name, "v": ((v - lo) / (hi - lo)).clamp(0.0, 1.0) }))
            })
            .collect();
        let leds = &self.project.leds;
        let scenario = (leds.source == crate::leds::Source::Scenario).then_some(leds.scenario.kind);
        let scenarios: Vec<_> =
            crate::leds::ScenarioKind::ALL.iter().map(|k| json!({ "id": k, "name": k.name().split(" (").next().unwrap_or("") })).collect();
        let dmx: Vec<_> = leds.dmx.iter().map(|d| json!({ "id": d.id, "name": d.name, "kind": d.kind, "on": d.on })).collect();
        let tempo = &self.rhythm.tempo;
        json!({
            "t": "live",
            "master": self.project.master,
            "blackout": self.blackout,
            "white": self.white_screen,
            "bypass": self.bypass_fx,
            "grid": self.grid,
            "recording": self.recorder.is_some(),
            "chain": self.project.chain.enabled,
            "bpm": tempo.bpm,
            "phase": tempo.beat_at(self.clock()).rem_euclid(1.0),
            "sel": sel,
            "layers": layers,
            "columns": columns,
            "presets": presets,
            "preset": rt.preset,
            "auto": cfg.auto.enabled,
            "mix": ccfg.mix.enabled,
            "react": cfg.react.enabled,
            "visual": (cfg.generator.pattern != Pattern::Off).then_some(cfg.generator.pattern),
            "visual_mix": cfg.generator.mix,
            "content": content,
            "visuals": visuals,
            "media": media,
            "playing": playing,
            "paused": player.is_some_and(VideoPlayer::is_paused),
            "reverse": player.is_some_and(VideoPlayer::is_reverse),
            "speed": player.map_or(1.0, VideoPlayer::speed),
            "fx": params,
            "leds": {
                "has": !leds.fixtures.is_empty(),
                "live": self.led.live,
                "scenario": scenario,
                "chain": leds.scenario.chain,
                "brightness": leds.brightness,
                "scenarios": scenarios,
            },
            "dmx": dmx,
            "mode": self.current_mode(),
            "groups": self.project.groups.iter().map(|g| json!({ "id": g.id, "name": g.name, "visible": g.visible, "fx": g.fx.kind })).collect::<Vec<_>>(),
            "outputs": self.project.outputs.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
            "output": self.sketch_output(),
            "kinds": (0..self.project.layers.len()).map(|i| self.layer_kind(i)).collect::<std::collections::HashSet<_>>(),
            "lasers": { "has": !leds.lasers.is_empty(), "armed": self.laser.armed,
                        "cues": leds.laser_cues.iter().map(|c| c.name.as_str()).collect::<Vec<_>>() },
                    })
    }

    pub(super) fn sketch_source(&self) -> Option<SourceRef> {
        self.sketch.server.as_ref().map(|s| {
            let (key, frame) = s.source();
            SourceRef { key, frame }
        })
    }

    pub(super) fn sketch_ui(&mut self, ui: &mut egui::Ui) {
        let i = self.sel();
        ui.label(egui::RichText::new(with_icon(ic::PENCIL_SIMPLE, "Dessiner depuis un iPad ou un téléphone")).strong().color(TEXT));
        ui.label(
            egui::RichText::new(
                "On dessine au doigt ou à l'Apple Pencil dans Safari, le trait apparaît en direct sur le calque. \
                 Pinceaux, symétrie, animations et disparitions se choisissent sur l'iPad ; les effets du calque \
                 s'appliquent au dessin. La même page a un onglet Live (commandes du spectacle) et un onglet \
                 Médias (lancer un média, envoyer une photo de l'iPad, détourée ou non).",
            )
            .small()
            .color(TEXT_WEAK),
        );
        ui.add_space(6.0);

        let name = self.project.layers[i].name.clone();
        let mut on = self.project.layers[i].sketch;
        if ui.checkbox(&mut on, format!("Le calque « {name} » affiche le dessin")).changed() {
            self.project.layers[i].sketch = on;
        }
        ui.label(
            egui::RichText::new(
                "Le dessin remplace la vidéo de ce calque ; ce qui n'est pas dessiné reste transparent. \
                 Le plus simple : glisser « Dessin (iPad) » (liste des visuels, à côté de « Chimie (iPad) ») \
                 sur une case du tableau, comme un média. Si rien ne montre le dessin quand on dessine, \
                 il va sur un calque vide (aucun calque n'est ajouté).",
            )
            .small()
            .color(TEXT_WEAK),
        );
        ui.add_space(10.0);

        if let Some(e) = &self.sketch.error {
            ui.colored_label(WARN, with_icon(ic::WARNING, &format!("Serveur de dessin indisponible : {e}")));
            if ui.button("Réessayer").clicked() {
                self.sketch.error = None;
            }
            return;
        }
        let Some(server) = &self.sketch.server else {
            ui.label("Démarrage…");
            return;
        };
        let url = server.urls().into_iter().next();
        let (clients, strokes, port) = (server.clients(), server.strokes(), server.port());

        theme::card().show(ui, |ui| {
            ui.label(egui::RichText::new(with_icon(ic::WIFI_HIGH, "Sur l'iPad, même Wi-Fi")).strong().color(TEXT));
            match &url {
                Some(url) => {
                    ui.label(egui::RichText::new("Scannez avec l'appareil photo, ou tapez l'adresse dans Safari :").small().color(TEXT_WEAK));
                    ui.add_space(4.0);
                    if let Some(tex) = self.sketch_qr(ui.ctx(), url) {
                        let side = ui.available_width().min(220.0);
                        ui.add(egui::Image::new(&tex).fit_to_exact_size(Vec2::splat(side)).corner_radius(6.0));
                    }
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(url).monospace().size(15.0).color(ACCENT));
                }
                None => {
                    ui.colored_label(WARN, "Pas de réseau : branchez cet ordinateur au Wi-Fi ou au câble.");
                }
            }
            ui.add_space(4.0);
            let status = match clients {
                0 => "Aucun appareil connecté".to_string(),
                1 => "1 appareil connecté".to_string(),
                n => format!("{n} appareils connectés"),
            };
            let strokes = match strokes {
                0 => String::new(),
                1 => " · 1 trait".to_string(),
                n => format!(" · {n} traits"),
            };
            ui.label(egui::RichText::new(format!("{status}{strokes}")).color(if clients > 0 { ACCENT } else { TEXT_WEAK }));
            ui.horizontal_wrapped(|ui| {
                if ui.button(with_icon(ic::TRASH, "Tout effacer")).clicked()
                    && let Some(s) = &self.sketch.server
                {
                    s.clear();
                }
                if ui
                    .button(with_icon(ic::MONITOR, "Ouvrir ici"))
                    .on_hover_text("Ouvre la page de dessin dans le navigateur de cet ordinateur (souris ou tablette graphique)")
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(format!("http://localhost:{port}/")));
                }
            });
        });

        ui.add_space(8.0);
        for tip in [
            "Pour l'ouvrir en plein écran comme une app : dans Safari, Partager › Sur l'écran d'accueil.",
            "Avec l'Apple Pencil, la main posée sur l'écran ne dessine pas.",
            "Plusieurs appareils peuvent dessiner en même temps : ils voient tous le même dessin.",
            "Au premier lancement, macOS peut demander d'autoriser les connexions entrantes : acceptez.",
        ] {
            ui.label(egui::RichText::new(format!("• {tip}")).small().color(TEXT_WEAK));
        }
    }

    /// QR code de l'adresse (recalculé seulement si elle change).
    fn sketch_qr(&mut self, ctx: &egui::Context, url: &str) -> Option<egui::TextureHandle> {
        if let Some((u, tex)) = &self.sketch.qr
            && u == url
        {
            return Some(tex.clone());
        }
        let code = qrcode::QrCode::new(url.as_bytes()).ok()?;
        let n = code.width();
        let quiet = 2;
        let side = (n + quiet * 2) * QR_PX;
        let colors = code.to_colors();
        let mut image = egui::ColorImage::new([side, side], vec![Color32::WHITE; side * side]);
        for y in 0..n {
            for x in 0..n {
                if colors[y * n + x] == qrcode::Color::Dark {
                    for dy in 0..QR_PX {
                        for dx in 0..QR_PX {
                            let (px, py) = ((x + quiet) * QR_PX + dx, (y + quiet) * QR_PX + dy);
                            image.pixels[py * side + px] = Color32::BLACK;
                        }
                    }
                }
            }
        }
        let tex = ctx.load_texture("sketch-qr", image, egui::TextureOptions::NEAREST);
        self.sketch.qr = Some((url.to_string(), tex.clone()));
        Some(tex)
    }
}
