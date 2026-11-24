//! Dessin à distance : une page web (iPad, téléphone, ordinateur) où l'on dessine au doigt ou au
//! stylet. Les traits arrivent en direct par WebSocket ; OZ Mapping les dessine (pinceaux, symétrie,
//! animations) dans une image qui sert de source à un calque. Les pages reçoivent en retour un
//! aperçu de cette image : tout le monde voit le même dessin.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Transform};
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

use crate::video::{Frame, FrameSlot};

/// Port de la page (sinon un port libre, affiché dans l'app).
pub const PORT: u16 = 47823;
const PAGE: &str = include_str!("sketch.html");
/// Hauteur de l'image dessinée (la largeur suit les proportions de la sortie).
const HEIGHT: u32 = 1080;
/// Au-delà, les traits les plus anciens disparaissent.
const MAX_STROKES: usize = 400;
const MAX_POINTS: usize = 5000;
/// Durée de l'effet de disparition d'un trait éphémère, en secondes.
const FADE_SECS: f32 = 1.2;
/// Mémoires de dessins (rappelées depuis la page).
pub const SLOTS: usize = 8;
/// Aperçu renvoyé aux pages : largeur tant que la page ne l'a pas donnée, intervalle minimal.
const PREVIEW_W: u32 = 1024;
const PREVIEW_EVERY: Duration = Duration::from_millis(33);
/// Aperçu de toute la sortie : 20 images par seconde suffisent sur l'iPad (chaque image est
/// lue sur la carte graphique puis compressée en JPEG).
const SCREEN_PREVIEW_EVERY: Duration = Duration::from_millis(50);
/// Intervalle minimal entre deux rendus pendant qu'on dessine (60 images par seconde).
const FRAME_EVERY: Duration = Duration::from_millis(16);
/// Intervalle entre deux rendus quand seules les animations bougent (30 images par seconde) :
/// chaque image est recopiée puis envoyée à la carte graphique, c'est ce qui coûte.
const ANIM_EVERY: Duration = Duration::from_millis(33);

/// Pinceau d'un trait.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Brush {
    #[default]
    Line,
    /// Tube lumineux : halo coloré et cœur presque blanc.
    Neon,
    /// Couleurs qui défilent le long du trait.
    Rainbow,
    /// Étoiles qui scintillent le long du trait.
    Sparkle,
    /// Aérographe : un nuage de fines particules.
    Spray,
    /// Pointillés lumineux.
    Dots,
}

/// Animation d'un trait.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Anim {
    #[default]
    None,
    /// Grossit et s'illumine sur les temps (son ou tempo).
    Pulse,
    /// Ondule.
    Wave,
    /// Tourne autour du centre de l'image.
    Spin,
    /// Monte doucement en flottant.
    Float,
    /// Se dessine tout seul, en boucle.
    Trace,
    /// Clignote sur les temps.
    Blink,
}

/// Façon dont disparaît un trait éphémère.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Vanish {
    #[default]
    Fade,
    /// S'amincit jusqu'à disparaître.
    Shrink,
    /// S'efface depuis son début, comme s'il se rembobinait.
    Tail,
    /// S'envole vers le haut.
    Rise,
    /// Explose en étincelles.
    Burst,
    /// Coule vers le bas.
    Melt,
    /// Comète : chaque morceau disparaît après la durée, même pendant qu'on dessine.
    Trail,
}

/// Message d'une page.
#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum In {
    /// Début d'un trait : position (0..1 de l'image), pression (0..1) et réglages du pinceau.
    Down {
        id: u32,
        x: f32,
        y: f32,
        #[serde(default = "half")]
        p: f32,
        #[serde(default)]
        color: String,
        /// Épaisseur, en hauteur de l'image.
        #[serde(default = "default_width")]
        w: f32,
        #[serde(default)]
        brush: Brush,
        #[serde(default)]
        anim: Anim,
        /// 1 (aucune), 2 (miroir), 4 (quatre coins), 6 ou 8 (kaléidoscope).
        #[serde(default = "one")]
        sym: u8,
        /// Durée avant disparition, en secondes (0 = reste).
        #[serde(default)]
        life: f32,
        #[serde(default)]
        out: Vanish,
    },
    Move {
        id: u32,
        pts: Vec<[f32; 3]>,
    },
    Up {
        id: u32,
    },
    /// Retire le dernier trait de cette page.
    Undo,
    /// Largeur de la toile de la page, en pixels d'écran (taille de l'aperçu renvoyé).
    View {
        w: u32,
        /// La page veut voir toute la sortie (effets, visuels…) plutôt que le dessin seul.
        #[serde(default)]
        screen: bool,
    },
    Clear,
    /// Gomme : retire les traits qui passent près du point (rayon en hauteur de l'image).
    Erase {
        x: f32,
        y: f32,
        r: f32,
    },
    /// Mémoires : garder le dessin actuel, ou le remplacer par une mémoire.
    Save {
        slot: usize,
    },
    Recall {
        slot: usize,
    },
    /// Photo du dessin : position (centre, 0..1), taille (part de la hauteur), rotation (degrés).
    #[serde(rename = "stmove")]
    StickerMove {
        id: u32,
        x: f32,
        y: f32,
        s: f32,
        r: f32,
    },
    /// Photo du dessin : animation, contour lumineux ("" : aucun), symétrie, retournement.
    #[serde(rename = "stset")]
    StickerSet {
        id: u32,
        #[serde(default)]
        anim: Option<Anim>,
        #[serde(default)]
        outline: Option<String>,
        #[serde(default)]
        sym: Option<u8>,
        #[serde(default)]
        flip: Option<bool>,
    },
    #[serde(rename = "stdel")]
    StickerDel {
        id: u32,
    },
    #[serde(rename = "stdup")]
    StickerDup {
        id: u32,
    },
    /// Onglet Live : commande (appui ou relâchement), ou réglage (0..1).
    Act {
        a: crate::midi::Action,
        #[serde(default = "yes")]
        on: bool,
    },
    Val {
        a: crate::midi::Action,
        v: f32,
    },
    /// Onglet Chimie : verser, touiller, pencher, secouer (voir `chem`).
    Chem(crate::chem::ChemMsg),
    /// Salle 3D : voir la scène des LEDs, y déplacer, ajouter, supprimer (voir `room`).
    Room(crate::room::RoomMsg),
}

/// Commande de l'onglet Live, exécutée par l'app comme celle d'un contrôleur MIDI.
pub enum Remote {
    Trigger(crate::midi::Action, bool),
    Value(crate::midi::Action, f32),
    /// Photo envoyée depuis la page (enregistrée sur cet ordinateur), à poser sur le calque.
    Photo { path: std::path::PathBuf },
    /// Une photo vient d'entrer dans le dessin : il faut qu'un calque l'affiche.
    DrawingUsed,
    /// Onglet Chimie : pour le bassin (posé sur un calque vide si rien ne montre la chimie).
    Chem(crate::chem::ChemMsg),
    /// Salle 3D : changement de la scène des LEDs.
    Room(crate::room::RoomMsg),
    /// Objet photographié et détouré au stylet sur l'iPad : photo et masque (blanc = objet),
    /// à mapper sur le calque choisi. `with_photo` : la photo s'affiche (sinon le masque seul) ;
    /// `calibrate` : la photo montre l'image projetée (écran blanc), qui sert à caler le masque.
    Object { photo: std::path::PathBuf, mask: std::path::PathBuf, with_photo: bool, calibrate: bool },
}

/// Taille maximale d'une photo envoyée.
const UPLOAD_MAX: usize = 40 << 20;
/// Miniatures des médias, pour l'onglet Live.
const THUMB: (u32, u32) = (320, 180);

fn yes() -> bool {
    true
}

fn half() -> f32 {
    0.5
}
fn one() -> u8 {
    1
}
fn default_width() -> f32 {
    0.012
}

enum Ev {
    Msg { client: u32, msg: In },
    /// Dessin enregistré à reprendre (ouverture du projet) : il remplace l'actuel.
    Load(Box<DrawingDoc>),
    /// Photo prête à entrer dans le dessin (détourage fait).
    Sticker(Box<Sticker>),
    /// Nouvelle image de la sortie : réveille la boucle pour l'envoyer tout de suite.
    Screen,
    /// Message pour les pages (photo reçue, ajoutée, ou pourquoi elle ne l'a pas été).
    Notice(String),
    Join { client: u32, tx: Sender<Message> },
    Leave { client: u32 },
    Clear,
}

/// Un trait : points en coordonnées de l'image (0..1) avec leur pression.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Stroke {
    #[serde(skip)]
    client: u32,
    #[serde(default)]
    id: u32,
    color: [f32; 3],
    width: f32,
    brush: Brush,
    anim: Anim,
    sym: u8,
    life: f32,
    #[serde(default)]
    out: Vanish,
    pts: Vec<[f32; 3]>,
    /// Instant d'arrivée de chaque point (comète).
    #[serde(skip)]
    times: Vec<f32>,
    /// Instant de création et de fin (secondes depuis le lancement).
    #[serde(skip)]
    born: f32,
    #[serde(skip)]
    ended: Option<f32>,
    /// Déjà dessiné dans l'image de base (trait terminé et immobile).
    #[serde(skip)]
    baked: bool,
    /// Trait terminé dessiné une fois pour toutes dans une petite image, posée à chaque rendu
    /// avec son opacité et sa rotation (clignote, tourne, s'efface en fondu).
    #[serde(skip)]
    sprite: Option<Arc<Sprite>>,
}

/// Image d'un trait (symétrie comprise) et sa position dans le dessin, en pixels.
#[derive(Debug)]
struct Sprite {
    pix: Pixmap,
    x: f32,
    y: f32,
}

impl Stroke {
    /// Le trait change d'une image à l'autre (il faut redessiner en continu).
    fn animated(&self) -> bool {
        self.anim != Anim::None || matches!(self.brush, Brush::Rainbow | Brush::Sparkle) || self.life > 0.0
    }

    /// Terminé et immobile : il peut rester dans l'image de base.
    fn settled(&self) -> bool {
        self.ended.is_some() && !self.animated()
    }

    /// Trait éphémère sans autre animation : immobile jusqu'au début de sa disparition
    /// (instant renvoyé), rien à redessiner d'ici là.
    fn still_until(&self) -> Option<f32> {
        let plain = self.anim == Anim::None && !matches!(self.brush, Brush::Rainbow | Brush::Sparkle);
        (plain && self.life > 0.0 && self.out != Vanish::Trail).then_some(self.ended? + self.life)
    }

    /// Le trait change à l'instant `t`.
    fn moving(&self, t: f32) -> bool {
        self.animated() && self.still_until().is_none_or(|u| t >= u)
    }

    /// Terminé, et son animation ne fait que déplacer ou estomper le trait entier : il peut
    /// être dessiné une fois dans une image (`Sprite`), posée ensuite à chaque rendu.
    fn spriteable(&self, t: f32) -> bool {
        self.ended.is_some()
            && matches!(self.brush, Brush::Line | Brush::Neon | Brush::Spray | Brush::Dots)
            && matches!(self.anim, Anim::None | Anim::Blink | Anim::Spin)
            && self.out != Vanish::Trail
            && (self.out == Vanish::Fade || self.gone(t) <= 0.0)
    }

    /// Avancement de la disparition (0 : intact, 1 : disparu).
    fn gone(&self, t: f32) -> f32 {
        if self.life <= 0.0 {
            return 0.0;
        }
        if self.out == Vanish::Trail {
            // Comète : disparu quand le dernier point a fait son temps.
            let last = self.times.last().copied().unwrap_or(self.born);
            return if self.ended.is_some() && t - last > self.life { 1.0 } else { 0.0 };
        }
        let secs = match self.out {
            Vanish::Rise => 2.0,
            Vanish::Melt => 1.8,
            Vanish::Burst => 1.0,
            _ => FADE_SECS,
        };
        match self.ended {
            Some(end) => ((t - end - self.life) / secs).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    /// Opacité (fondu de disparition, clignotement).
    fn alpha(&self, t: f32) -> f32 {
        let q = self.gone(t);
        match self.out {
            Vanish::Trail | Vanish::Tail if q < 1.0 => 1.0,
            _ => 1.0 - q,
        }
    }
}

/// Photo posée dans le dessin (détourée ou entière) : on la déplace, la tourne, l'anime depuis la page.
struct Sticker {
    id: u32,
    /// Image (alpha prémultiplié, transparente hors du sujet), avec une marge pour le halo.
    img: Arc<Pixmap>,
    /// Alpha flouté du sujet (contour lumineux), et le halo teinté de la dernière couleur demandée.
    halo: Arc<Vec<u8>>,
    halo_tex: Option<([u8; 3], Arc<Pixmap>)>,
    /// Centre (0..1), hauteur (part de la hauteur de l'image), rotation (degrés).
    x: f32,
    y: f32,
    size: f32,
    rot: f32,
    anim: Anim,
    outline: Option<[u8; 3]>,
    sym: u8,
    flip: bool,
    born: f32,
    /// Fichier d'origine (photo reçue) et détourage demandé : pour la recréer à l'ouverture.
    source: Option<(std::path::PathBuf, bool)>,
}

/// Le dessin de l'iPad, enregistré avec le projet : traits et photos posées dedans.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingDoc {
    strokes: Vec<Stroke>,
    stickers: Vec<StickerDoc>,
}

impl PartialEq for DrawingDoc {
    fn eq(&self, other: &Self) -> bool {
        // Comparaison par le contenu enregistré (les traits n'ont pas `PartialEq`).
        serde_json::to_string(self).ok() == serde_json::to_string(other).ok()
    }
}

impl DrawingDoc {
    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty() && self.stickers.is_empty()
    }

    /// Fichiers des photos du dessin (pour les ranger avec le projet).
    pub fn files_mut(&mut self) -> impl Iterator<Item = &mut std::path::PathBuf> {
        self.stickers.iter_mut().map(|s| &mut s.file)
    }
}

/// Photo posée dans le dessin, telle qu'enregistrée.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StickerDoc {
    file: std::path::PathBuf,
    #[serde(default)]
    cut: bool,
    x: f32,
    y: f32,
    size: f32,
    rot: f32,
    #[serde(default)]
    anim: Anim,
    #[serde(default)]
    outline: Option<[u8; 3]>,
    #[serde(default = "one")]
    sym: u8,
    #[serde(default)]
    flip: bool,
}

impl Clone for Sticker {
    fn clone(&self) -> Self {
        Self { halo_tex: self.halo_tex.clone(), img: self.img.clone(), halo: self.halo.clone(), source: self.source.clone(), ..*self }
    }
}

impl Sticker {
    fn animated(&self) -> bool {
        self.anim != Anim::None
    }

    fn info(&self) -> serde_json::Value {
        let anim = serde_json::to_value(self.anim).unwrap_or_default();
        serde_json::json!({
            "id": self.id, "x": self.x, "y": self.y, "s": self.size, "r": self.rot,
            "ar": self.img.width() as f32 / self.img.height() as f32,
            "anim": anim, "outline": self.outline.is_some(), "sym": self.sym, "flip": self.flip,
        })
    }

    /// Halo teinté de la couleur du contour (gardé tant que la couleur ne change pas).
    fn halo_for(&mut self, c: [u8; 3]) -> Option<Arc<Pixmap>> {
        if let Some((k, tex)) = &self.halo_tex
            && *k == c
        {
            return Some(tex.clone());
        }
        let (w, h) = (self.img.width(), self.img.height());
        let mut data = Vec::with_capacity(self.halo.len() * 4);
        for &a in self.halo.iter() {
            let a = (a as u32 * 2).min(255);
            data.extend([c[0] as u32 * a / 255, c[1] as u32 * a / 255, c[2] as u32 * a / 255, a].map(|v| v as u8));
        }
        let tex = Arc::new(Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h)?)?);
        self.halo_tex = Some((c, tex.clone()));
        Some(tex)
    }
}

/// Prépare une photo pour le dessin : réduite, détourée si demandé (sujet trouvé automatiquement),
/// recadrée sur le sujet, avec une marge et l'alpha flouté pour le contour lumineux.
fn make_sticker(data: &[u8], ext: &str, cut: bool) -> Result<(Pixmap, Vec<u8>)> {
    use image::imageops::{self, FilterType};
    let img = decode_photo(data, ext)?;
    let img = if img.width().max(img.height()) > 1024 { img.resize(1024, 1024, FilterType::Triangle) } else { img };
    let mut rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if cut {
        let c = crate::cutout::Cutout::new(&img);
        if c.mask.iter().any(|&m| m) {
            // Masque de travail agrandi à la taille de la photo : bord adouci.
            let small = image::GrayImage::from_fn(c.width as u32, c.height as u32, |x, y| {
                image::Luma([if c.mask[y as usize * c.width + x as usize] { 255 } else { 0 }])
            });
            let big = imageops::blur(&imageops::resize(&small, w, h, FilterType::Triangle), 1.2);
            for (p, m) in rgba.pixels_mut().zip(big.pixels()) {
                p[3] = ((p[3] as u32 * m[0] as u32) / 255) as u8;
            }
        }
    }
    // Recadrage sur ce qui est visible.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in rgba.enumerate_pixels() {
        if p[3] > 10 {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    anyhow::ensure!(x1 >= x0 && y1 >= y0, "rien trouvé à détourer sur la photo");
    let crop = imageops::crop_imm(&rgba, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
    // Marge autour du sujet, pour que le halo ne soit pas coupé.
    let pad = ((crop.width().max(crop.height()) as f32) * 0.08) as u32 + 4;
    let (pw, ph) = (crop.width() + pad * 2, crop.height() + pad * 2);
    let mut padded = image::RgbaImage::new(pw, ph);
    imageops::overlay(&mut padded, &crop, pad as i64, pad as i64);
    let alpha = image::GrayImage::from_fn(pw, ph, |x, y| image::Luma([padded.get_pixel(x, y)[3]]));
    let halo = imageops::blur(&alpha, pad as f32 * 0.45).into_raw();
    let mut premul = padded.into_raw();
    for p in premul.chunks_exact_mut(4) {
        let a = p[3] as u32;
        for c in &mut p[..3] {
            *c = (*c as u32 * a / 255) as u8;
        }
    }
    let pix = Pixmap::from_vec(premul, tiny_skia::IntSize::from_wh(pw, ph).context("taille")?).context("image")?;
    Ok((pix, halo))
}

/// Dessine une photo du dessin, avec son animation, son contour et ses copies.
fn draw_sticker(pix: &mut Pixmap, st: &mut Sticker, t: f32, beat: f32) {
    let (w, h) = (pix.width() as f32, pix.height() as f32);
    let age = t - st.born;
    let (mut x, mut y, mut k, mut rot, mut alpha) = (st.x * w, st.y * h, 1.0, st.rot, 1.0);
    let seed = (st.id % 31) as f32;
    match st.anim {
        Anim::Pulse => k *= 1.0 + 0.18 * beat,
        Anim::Spin => rot += age * 45.0,
        Anim::Float => {
            y += (age * 1.1 + seed).sin() * h * 0.03;
            x += (age * 0.7 + seed).sin() * h * 0.02;
        }
        // « Danse » : se balance et saute sur les temps.
        Anim::Wave => {
            rot += (age * 4.0).sin() * 12.0;
            y -= beat * h * 0.05;
        }
        Anim::Blink => alpha = 0.12 + 0.88 * beat,
        _ => {}
    }
    let scale = st.size * h / st.img.height() as f32 * k;
    let (iw, ih) = (st.img.width() as f32, st.img.height() as f32);
    let local = Transform::from_translate(-iw * 0.5, -ih * 0.5)
        .post_scale(if st.flip { -scale } else { scale }, scale)
        .post_concat(Transform::from_rotate(rot))
        .post_translate(x, y);
    let halo = st.outline.and_then(|c| st.halo_for(c));
    let paint = PixmapPaint { opacity: alpha, quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
    let glow = PixmapPaint { blend_mode: tiny_skia::BlendMode::Plus, ..paint };
    for copy in sym_copies(st.sym, 0.0, w, h) {
        let tr = local.post_concat(copy);
        if let Some(halo) = &halo {
            pix.draw_pixmap(0, 0, halo.as_ref().as_ref(), &glow, tr, None);
        }
        pix.draw_pixmap(0, 0, st.img.as_ref().as_ref(), &paint, tr, None);
    }
}

struct Shared {
    stop: AtomicBool,
    /// Enveloppe des temps (0..1) et proportions de l'image, en bits de f32.
    beat: AtomicU32,
    aspect: AtomicU32,
    clients: AtomicUsize,
    strokes: AtomicUsize,
    stickers: AtomicUsize,
    /// Un calque visible affiche le dessin (sinon, sans page ouverte, rien n'est calculé).
    shown: AtomicBool,
    slot: FrameSlot,
    /// État de l'app pour l'onglet Live (JSON), avec son numéro de version.
    live: Mutex<(u64, String)>,
    /// Salle 3D (JSON, version), et dernière demande d'une page qui la montre.
    room: Mutex<(u64, String)>,
    room_watch: Mutex<Option<Instant>>,
    /// Dessin actuel, à enregistrer avec le projet (version, contenu).
    doc: Mutex<(u64, Option<Arc<DrawingDoc>>)>,
    /// Commandes de l'onglet Live, en attente de l'app.
    remote: Mutex<Vec<Remote>>,
    /// Médias de l'app (miniatures servies à la page) et miniatures déjà faites.
    media: Mutex<Vec<std::path::PathBuf>>,
    thumbs: Mutex<std::collections::HashMap<std::path::PathBuf, Arc<Vec<u8>>>>,
    repaint: Box<dyn Fn() + Send + Sync>,
    /// Image de la sortie lue par l'app (RGBA, lignes du bas vers le haut) : c'est elle que
    /// voient les pages, tant qu'elle arrive. Son numéro change à chaque nouvelle image.
    screen: Mutex<Option<(Arc<Vec<u8>>, (u32, u32))>>,
    screen_serial: AtomicU64,
    /// Une page veut voir toute la sortie.
    want_screen: AtomicBool,
    /// Certificat de la page en https (None : http seulement).
    tls: Option<Arc<rustls::ServerConfig>>,
}

/// Serveur du dessin à distance (arrêté quand on le lâche).
pub struct Sketch {
    shared: Arc<Shared>,
    tx: Sender<Ev>,
    uid: u64,
    port: u16,
}

impl Sketch {
    pub fn start(repaint: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, PORT))
            .or_else(|_| TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)))
            .context("impossible d'ouvrir le serveur de dessin")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            beat: AtomicU32::new(0f32.to_bits()),
            aspect: AtomicU32::new((16.0f32 / 9.0).to_bits()),
            clients: AtomicUsize::new(0),
            strokes: AtomicUsize::new(0),
            stickers: AtomicUsize::new(0),
            shown: AtomicBool::new(false),
            slot: Arc::new(Mutex::new(None)),
            live: Mutex::new((0, String::new())),
            room: Mutex::new((0, String::new())),
            room_watch: Mutex::new(None),
            doc: Mutex::new((0, None)),
            remote: Mutex::new(Vec::new()),
            media: Mutex::new(Vec::new()),
            thumbs: Mutex::default(),
            repaint: Box::new(repaint),
            screen: Mutex::new(None),
            screen_serial: AtomicU64::new(0),
            want_screen: AtomicBool::new(false),
            tls: tls_config(),
        });
        let (tx, rx) = mpsc::channel();
        {
            let shared = shared.clone();
            let tx = tx.clone();
            std::thread::Builder::new().name("dessin-serveur".into()).spawn(move || accept(listener, shared, tx))?;
        }
        {
            let shared = shared.clone();
            std::thread::Builder::new().name("dessin-rendu".into()).spawn(move || render_loop(shared, rx))?;
        }
        Ok(Self { shared, tx, uid: crate::video::new_source_uid(), port })
    }

    /// Clé de texture et image du dessin (source d'un calque).
    pub fn source(&self) -> (u64, FrameSlot) {
        (self.uid, self.shared.slot.clone())
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Adresses à ouvrir depuis l'iPad (https quand le certificat est prêt : l'inclinaison et
    /// les secousses n'arrivent que sur une page sécurisée).
    pub fn urls(&self) -> Vec<String> {
        let scheme = if self.shared.tls.is_some() { "https" } else { "http" };
        crate::net::local_ips().into_iter().map(|ip| format!("{scheme}://{ip}:{}/", self.port)).collect()
    }

    /// Pages connectées.
    pub fn clients(&self) -> usize {
        self.shared.clients.load(Ordering::Relaxed)
    }

    /// Traits affichés.
    pub fn strokes(&self) -> usize {
        self.shared.strokes.load(Ordering::Relaxed)
    }

    /// Photos posées dans le dessin.
    pub fn stickers(&self) -> usize {
        self.shared.stickers.load(Ordering::Relaxed)
    }

    /// Un calque visible affiche-t-il le dessin ?
    pub fn set_shown(&self, shown: bool) {
        self.shared.shown.store(shown, Ordering::Relaxed);
    }

    /// Enveloppe des temps (0..1), pour l'animation « bat au tempo ».
    pub fn set_beat(&self, beat: f32) {
        self.shared.beat.store(beat.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Proportions de la sortie où le dessin est affiché.
    pub fn set_aspect(&self, aspect: f32) {
        self.shared.aspect.store(aspect.clamp(0.25, 4.0).to_bits(), Ordering::Relaxed);
    }

    pub fn clear(&self) {
        let _ = self.tx.send(Ev::Clear);
    }

    /// État de l'app pour l'onglet Live (envoyé aux pages s'il a changé).
    pub fn set_live(&self, json: String) {
        let mut live = self.shared.live.lock().unwrap();
        if live.1 != json {
            *live = (live.0 + 1, json);
        }
    }

    /// Dessin actuel (traits et photos), avec son numéro de version (change à chaque modification).
    pub fn drawing(&self) -> (u64, Option<Arc<DrawingDoc>>) {
        self.shared.doc.lock().unwrap().clone()
    }

    /// Reprend un dessin enregistré (il remplace l'actuel).
    pub fn load_drawing(&self, doc: DrawingDoc) {
        let _ = self.tx.send(Ev::Load(Box::new(doc)));
    }

    /// Message pour les pages (affiché en haut de l'iPad).
    pub fn notice(&self, text: String) {
        let _ = self.tx.send(Ev::Notice(text));
    }

    /// Salle 3D (envoyée aux pages si elle a changé).
    pub fn set_room(&self, json: String) {
        let mut room = self.shared.room.lock().unwrap();
        if room.1 != json {
            *room = (room.0 + 1, json);
        }
    }

    /// Une page montre la salle 3D (demande de moins de 5 s).
    pub fn wants_room(&self) -> bool {
        self.shared.room_watch.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_secs(5))
    }

    /// Image de la sortie (RGBA, lignes du bas vers le haut) : les pages voient ce qui est
    /// vraiment projeté (vidéos, visuels, effets, dessin compris).
    pub fn set_screen(&self, pixels: Vec<u8>, size: (u32, u32)) {
        *self.shared.screen.lock().unwrap() = Some((Arc::new(pixels), size));
        self.shared.screen_serial.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(Ev::Screen);
    }

    /// Une page veut voir toute la sortie : l'app doit lui envoyer son image (`set_screen`).
    pub fn wants_screen(&self) -> bool {
        self.shared.want_screen.load(Ordering::Relaxed)
    }

    /// Médias de l'app, dans l'ordre de sa liste (miniatures de l'onglet Live).
    pub fn set_media(&self, media: &[std::path::PathBuf]) {
        let mut m = self.shared.media.lock().unwrap();
        if m.as_slice() != media {
            *m = media.to_vec();
        }
    }

    /// Commandes de l'onglet Live arrivées depuis le dernier appel.
    pub fn take_remote(&self) -> Vec<Remote> {
        std::mem::take(&mut *self.shared.remote.lock().unwrap())
    }
}

/// Certificat https pour les adresses de cet ordinateur (None si impossible : http seulement).
fn tls_config() -> Option<Arc<rustls::ServerConfig>> {
    // Essais : un dossier à part (pas celui de l'app).
    let dir = if cfg!(test) { Some(std::env::temp_dir().join(format!("ozmapping-tls-test-{}", std::process::id()))) } else { data_dir() }?;
    let mut names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    names.extend(crate::net::local_ips().into_iter().map(|ip| ip.to_string()));
    crate::tls::server_config(&dir, &names).ok()
}

/// Fichier des mémoires de dessins.
fn memory_path() -> Option<std::path::PathBuf> {
    Some(data_dir()?.join("dessins.json"))
}

/// Dossier des photos envoyées depuis l'iPad.
pub fn photos_dir() -> Option<std::path::PathBuf> {
    Some(data_dir()?.join("Photos iPad"))
}

/// Dossier des données de l'app (mémoires de dessins, photos reçues).
pub fn data_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from);
    let dir = if cfg!(target_os = "macos") {
        home?.join("Library/Application Support/OZ-Mapping")
    } else if cfg!(windows) {
        std::path::PathBuf::from(std::env::var_os("APPDATA")?).join("OZ-Mapping")
    } else {
        home?.join(".local/share/oz-mapping")
    };
    Some(dir)
}

fn load_memories() -> Vec<Option<Vec<Stroke>>> {
    let mut slots: Vec<Option<Vec<Stroke>>> = memory_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    slots.resize(SLOTS, None);
    slots
}

fn save_memories(slots: &[Option<Vec<Stroke>>]) {
    let Some(path) = memory_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec(slots) {
        let _ = std::fs::write(path, json);
    }
}

impl Drop for Sketch {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

fn load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

// ------------------------------------------------------------------ serveur web

fn accept(listener: TcpListener, shared: Arc<Shared>, tx: Sender<Ev>) {
    let mut next = 1u32;
    while !shared.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let (shared, tx, client) = (shared.clone(), tx.clone(), next);
                next += 1;
                let _ = std::thread::Builder::new()
                    .name("dessin-page".into())
                    .spawn(move || {
                        let _ = serve(stream, &shared, &tx, client);
                    });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Une connexion : la page elle-même, ou le WebSocket des traits.
fn serve(stream: TcpStream, shared: &Shared, tx: &Sender<Ev>, client: u32) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    // Une page qui ne lit plus (iPad en veille, Wi-Fi coupé) : au bout de 2 s, on la lâche,
    // plutôt que de rester bloqué à lui écrire pendant que ses aperçus s'accumulent.
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    // https (iPad : inclinaison et secousses) ou http, sur le même port.
    let stream = crate::tls::Conn::accept(stream, shared.tls.as_ref())?;
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut key = None;
    let mut upgrade = false;
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "sec-websocket-key" => key = Some(value.trim().to_string()),
                "upgrade" => upgrade = value.trim().eq_ignore_ascii_case("websocket"),
                "content-length" => length = value.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    let rest = reader.buffer().to_vec();
    let mut stream = reader.into_inner();
    let path = request.split_whitespace().nth(1).unwrap_or("/");

    if let (true, Some(key)) = (upgrade, key) {
        let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
        write!(
            stream,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        )?;
        stream.set_read_timeout(Some(Duration::from_millis(10)))?;
        let ws = WebSocket::from_partially_read(stream, rest, Role::Server, None);
        shared.clients.fetch_add(1, Ordering::Relaxed);
        let result = talk(ws, shared, tx, client);
        shared.clients.fetch_sub(1, Ordering::Relaxed);
        let _ = tx.send(Ev::Leave { client });
        return result;
    }

    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let mut thumb = None;
    let mask_png: Option<Vec<u8>>;
    let (status, kind, body): (&str, &str, &[u8]) = match route {
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", PAGE.as_bytes()),
        "/upload" if request.starts_with("POST") => {
            if length == 0 || length > UPLOAD_MAX {
                ("413 Payload Too Large", "text/plain; charset=utf-8", "Photo trop grande".as_bytes())
            } else {
                // Début du corps déjà lu avec les en-têtes, puis le reste.
                let have = rest.len().min(length);
                let mut body = rest;
                body.resize(length, 0);
                stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                std::io::Read::read_exact(&mut stream, &mut body[have..])?;
                let has = |kv: &str| query.split('&').any(|q| q == kv);
                let ext = photo_ext(&body, query);
                if has("to=drawing") {
                    // Photo pour le dessin : détourée (ou non) à part, elle arrive quand elle est prête.
                    let cut = has("cut=1");
                    let tx = tx.clone();
                    let _ = tx.send(Ev::Notice(if cut { "Photo reçue : détourage…".into() } else { "Photo reçue".into() }));
                    // La photo d'origine est gardée : le dessin enregistré la retrouve.
                    let source = save_photo(&body, ext).ok().map(|p| (p, cut));
                    std::thread::spawn(move || {
                        match make_sticker(&body, ext, cut) {
                            Ok((img, halo)) => {
                                let h = img.height() as f32;
                                let _ = tx.send(Ev::Sticker(Box::new(Sticker {
                                    id: 0,
                                    img: Arc::new(img),
                                    halo: Arc::new(halo),
                                    halo_tex: None,
                                    x: 0.5,
                                    y: 0.5,
                                    size: (h / 1080.0).clamp(0.35, 0.7),
                                    rot: 0.0,
                                    anim: Anim::None,
                                    outline: None,
                                    sym: 1,
                                    flip: false,
                                    born: 0.0,
                                    source,
                                })));
                            }
                            Err(e) => {
                                let _ = tx.send(Ev::Notice(format!("Photo non ajoutée : {e:#}")));
                            }
                        }
                    });
                    ("200 OK", "text/plain; charset=utf-8", "OK".as_bytes())
                } else {
                    match save_photo(&body, ext) {
                        Ok(path) => {
                            let _ = tx.send(Ev::Notice("Photo envoyée sur le calque ✓".into()));
                            shared.remote.lock().unwrap().push(Remote::Photo { path });
                            (shared.repaint)();
                            ("200 OK", "text/plain; charset=utf-8", "OK".as_bytes())
                        }
                        Err(_) => ("500 Internal Server Error", "text/plain; charset=utf-8", "Enregistrement impossible".as_bytes()),
                    }
                }
            }
        }
        // Objet détouré sur l'iPad : longueur de la photo (4 octets), photo JPEG, puis masque PNG.
        "/object" if request.starts_with("POST") => {
            if length < 8 || length > UPLOAD_MAX {
                ("413 Payload Too Large", "text/plain; charset=utf-8", "Photo trop grande".as_bytes())
            } else {
                let have = rest.len().min(length);
                let mut body = rest;
                body.resize(length, 0);
                stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                std::io::Read::read_exact(&mut stream, &mut body[have..])?;
                let has = |kv: &str| query.split('&').any(|q| q == kv);
                let n = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
                if n == 0 || 4 + n >= body.len() {
                    ("400 Bad Request", "text/plain; charset=utf-8", "Envoi incomplet".as_bytes())
                } else {
                    match (save_photo(&body[4..4 + n], "jpg"), save_photo(&body[4 + n..], "png")) {
                        (Ok(photo), Ok(mask)) => {
                            let _ = tx.send(Ev::Notice("Objet reçu : mapping…".into()));
                            shared.remote.lock().unwrap().push(Remote::Object {
                                photo,
                                mask,
                                with_photo: !has("photo=0"),
                                calibrate: has("calibrate=1"),
                            });
                            (shared.repaint)();
                            ("200 OK", "text/plain; charset=utf-8", "OK".as_bytes())
                        }
                        _ => ("500 Internal Server Error", "text/plain; charset=utf-8", "Enregistrement impossible".as_bytes()),
                    }
                }
            }
        }
        // Détourage automatique d'une photo, point de départ du détourage au stylet sur la page :
        // renvoie le masque (PNG en niveaux de gris, blanc = sujet).
        "/cutout" if request.starts_with("POST") => {
            if length == 0 || length > UPLOAD_MAX {
                ("413 Payload Too Large", "text/plain; charset=utf-8", "Photo trop grande".as_bytes())
            } else {
                let have = rest.len().min(length);
                let mut body = rest;
                body.resize(length, 0);
                stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                std::io::Read::read_exact(&mut stream, &mut body[have..])?;
                mask_png = auto_mask(&body, photo_ext(&body, query)).ok();
                match &mask_png {
                    Some(png) => ("200 OK", "image/png", png.as_slice()),
                    None => ("500 Internal Server Error", "text/plain; charset=utf-8", "Détourage impossible".as_bytes()),
                }
            }
        }
        _ if route.starts_with("/thumb/") => {
            let index: Option<usize> = route["/thumb/".len()..].parse().ok();
            let media = index.and_then(|i| shared.media.lock().unwrap().get(i).cloned());
            thumb = media.and_then(|m| thumbnail(shared, &m));
            match &thumb {
                Some(jpeg) => ("200 OK", "image/jpeg", jpeg.as_slice()),
                None => ("404 Not Found", "text/plain; charset=utf-8", "Introuvable".as_bytes()),
            }
        }
        _ => ("404 Not Found", "text/plain; charset=utf-8", "Introuvable".as_bytes()),
    };
    let cache = if thumb.is_some() { "max-age=600" } else { "no-store" };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: {cache}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

/// Enregistre une photo reçue (JPEG préparé par la page) ; renvoie son chemin.
fn save_photo(data: &[u8], ext: &str) -> Result<std::path::PathBuf> {
    let dir = photos_dir().context("dossier des photos introuvable")?;
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let path = dir.join(format!("ipad-{stamp}.{ext}"));
    std::fs::write(&path, data)?;
    Ok(path)
}

/// Masque du détourage automatique (sujet en blanc), en PNG.
fn auto_mask(data: &[u8], ext: &str) -> Result<Vec<u8>> {
    let img = decode_photo(data, ext)?;
    let c = crate::cutout::Cutout::new(&img);
    let mask = image::GrayImage::from_fn(c.width as u32, c.height as u32, |x, y| {
        image::Luma([if c.mask[y as usize * c.width + x as usize] { 255 } else { 0 }])
    });
    let mut png = Vec::new();
    image::DynamicImage::ImageLuma8(mask).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(png)
}

/// Extension d'une photo reçue : celle annoncée par la page (fichier d'origine envoyé tel quel,
/// HEIC par exemple), sinon d'après son contenu.
fn photo_ext(data: &[u8], query: &str) -> &'static str {
    let asked = query.split('&').find_map(|q| q.strip_prefix("ext=")).unwrap_or("").to_ascii_lowercase();
    match asked.as_str() {
        "heic" => "heic",
        "heif" => "heif",
        "png" => "png",
        "webp" => "webp",
        "gif" => "gif",
        "jpg" | "jpeg" => "jpg",
        _ if data.starts_with(&[0x89, b'P', b'N', b'G']) => "png",
        _ if data.get(4..12).is_some_and(|b| b.starts_with(b"ftyphei") || b.starts_with(b"ftypmif")) => "heic",
        _ => "jpg",
    }
}

/// Décode une photo : directement si possible, sinon via un fichier (HEIC : converti par macOS).
fn decode_photo(data: &[u8], ext: &str) -> Result<image::DynamicImage> {
    if ext != "heic" && ext != "heif"
        && let Ok(img) = image::load_from_memory(data)
    {
        return Ok(img);
    }
    static N: AtomicUsize = AtomicUsize::new(0);
    let tmp = std::env::temp_dir().join(format!("ozmapping-photo-{}-{}.{ext}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&tmp, data)?;
    let img = crate::cutout::load_image(&tmp);
    let _ = std::fs::remove_file(&tmp);
    img.context("photo illisible")
}

/// Miniature JPEG d'un média (gardée ensuite).
fn thumbnail(shared: &Shared, path: &std::path::Path) -> Option<Arc<Vec<u8>>> {
    if let Some(t) = shared.thumbs.lock().unwrap().get(path) {
        return Some(t.clone());
    }
    let rgba = crate::video::thumbnail(path, THUMB.0, THUMB.1).ok()?;
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 78)
        .encode(&rgb, THUMB.0, THUMB.1, image::ExtendedColorType::Rgb8)
        .ok()?;
    let out = Arc::new(out);
    shared.thumbs.lock().unwrap().insert(path.to_path_buf(), out.clone());
    Some(out)
}

/// Échanges avec une page : ses traits arrivent, l'aperçu repart.
fn talk(mut ws: WebSocket<crate::tls::Conn>, shared: &Shared, tx: &Sender<Ev>, client: u32) -> Result<()> {
    let (out_tx, out_rx) = mpsc::channel::<Message>();
    let _ = tx.send(Ev::Join { client, tx: out_tx });
    while !shared.stop.load(Ordering::Relaxed) {
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let Ok(msg) = serde_json::from_str::<In>(text.as_str()) {
                    let _ = tx.send(Ev::Msg { client, msg });
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
        // Un seul aperçu à la fois (le plus récent) : une page lente ne prend pas de retard.
        let mut pending: Vec<Message> = out_rx.try_iter().collect();
        if let Some(last) = pending.iter().rposition(Message::is_binary) {
            let mut i = 0;
            pending.retain(|m| {
                i += 1;
                !m.is_binary() || i - 1 == last
            });
        }
        for m in pending {
            ws.write(m)?;
        }
        ws.flush()?;
    }
    Ok(())
}

// ------------------------------------------------------------------ rendu

/// Page connectée : où lui écrire, et la largeur de sa toile en pixels d'écran.
struct Client {
    id: u32,
    tx: Sender<Message>,
    width: u32,
    /// Veut voir toute la sortie (sinon : le dessin seul, plus léger).
    screen: bool,
}

/// Image à encoder pour les pages, sur un fil à part (l'encodage ne ralentit pas le dessin).
struct Shot {
    data: Arc<Vec<u8>>,
    size: (u32, u32),
    /// Largeur voulue de l'aperçu.
    width: u32,
    /// Image lue sur la carte graphique : lignes du bas vers le haut, alpha quelconque.
    gl: bool,
}

fn render_loop(shared: Arc<Shared>, rx: Receiver<Ev>) {
    let repaint = || (shared.repaint)();
    let start = Instant::now();
    let mut strokes: Vec<Stroke> = Vec::new();
    let clients: Arc<Mutex<Vec<Client>>> = Arc::default();
    let (shot_tx, shot_rx) = mpsc::sync_channel::<Shot>(1);
    {
        let clients = clients.clone();
        let _ = std::thread::Builder::new().name("dessin-apercu".into()).spawn(move || encode_loop(shot_rx, clients));
    }
    let mut aspect = load(&shared.aspect);
    let mut pix = canvas(aspect);
    // Traits terminés et immobiles, déjà dessinés (sur fond transparent, posés au-dessus des
    // photos) : seuls les autres sont redessinés à chaque image.
    let mut base = canvas(aspect);
    base.fill(Color::TRANSPARENT);
    let mut baked = 0;
    let mut rebuild = true;
    let mut dirty = true;
    let mut doc_dirty = false;
    let mut last_doc = Instant::now();
    let mut preview_dirty = true;
    let mut last_frame = Instant::now() - FRAME_EVERY;
    let mut last_preview = Instant::now();
    let hello = |aspect: f32| Message::text(format!("{{\"t\":\"hello\",\"aspect\":{aspect}}}"));
    let mut memories = load_memories();
    let mut live_version = 0;
    let mut room_version = 0;
    // Photos posées dans le dessin (sous les traits).
    let mut stickers: Vec<Sticker> = Vec::new();
    let mut next_sticker = 1u32;
    let stickers_msg = |st: &[Sticker]| {
        let list: Vec<_> = st.iter().map(Sticker::info).collect();
        Message::text(serde_json::json!({ "t": "stickers", "list": list }).to_string())
    };
    // Image de la sortie : dernière envoyée aux pages, et quand elle est arrivée.
    let mut screen_seen = 0u64;
    let mut screen_at: Option<Instant> = None;

    while !shared.stop.load(Ordering::Relaxed) {
        let now = start.elapsed().as_secs_f32();
        let animated = strokes.iter().any(|s| s.moving(now)) || stickers.iter().any(Sticker::animated);
        // Prochain trait éphémère qui commence à disparaître.
        let wake = strokes.iter().filter_map(Stroke::still_until).filter(|&u| u > now).fold(f32::MAX, f32::min);
        let has_clients = !clients.lock().unwrap().is_empty();
        // Personne ne voit le dessin (aucun calque visible ne l'affiche, aucune page ouverte) :
        // rien à calculer, il reprend dès qu'on le regarde.
        let seen = has_clients || shared.shown.load(Ordering::Relaxed);
        let every = if dirty { FRAME_EVERY } else { ANIM_EVERY };
        let wait = if !seen {
            Duration::from_millis(250)
        } else if dirty || animated {
            every.saturating_sub(last_frame.elapsed())
        } else if preview_dirty && has_clients {
            Duration::from_millis(10)
        } else {
            Duration::from_millis(200)
        };
        let wait = wait.min(Duration::from_secs_f32((wake - now).clamp(0.0, 1.0)));
        let first = match rx.recv_timeout(wait) {
            Ok(ev) => Some(ev),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let t = start.elapsed().as_secs_f32();
        for ev in first.into_iter().chain(rx.try_iter()) {
            match ev {
                Ev::Join { client, tx } => {
                    let _ = tx.send(hello(aspect));
                    let _ = tx.send(slots_msg(&memories));
                    let live = shared.live.lock().unwrap().1.clone();
                    if !live.is_empty() {
                        let _ = tx.send(Message::text(live));
                    }
                    let _ = tx.send(stickers_msg(&stickers));
                    clients.lock().unwrap().push(Client { id: client, tx, width: PREVIEW_W, screen: false });
                    preview_dirty = true;
                }
                Ev::Leave { client } => {
                    clients.lock().unwrap().retain(|c| c.id != client);
                    // Ses traits en cours sont terminés (page fermée en plein geste).
                    for s in strokes.iter_mut().filter(|s| s.client == client && s.ended.is_none()) {
                        s.ended = Some(t);
                    }
                    dirty = true;
                }
                Ev::Load(doc) => {
                    // Dessin d'un projet : traits fixés tels qu'enregistrés, photos recréées depuis
                    // leurs fichiers (celles introuvables sont laissées).
                    strokes = doc.strokes;
                    for s in &mut strokes {
                        s.client = 0;
                        s.born = t;
                        s.ended = Some(t);
                        s.times = vec![t; s.pts.len()];
                        s.baked = false;
                        s.sprite = None;
                    }
                    stickers.clear();
                    for sd in doc.stickers {
                        let Ok(data) = std::fs::read(&sd.file) else { continue };
                        let ext = sd.file.extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_ascii_lowercase();
                        let Ok((img, halo)) = make_sticker(&data, &ext, sd.cut) else { continue };
                        stickers.push(Sticker {
                            id: next_sticker,
                            img: Arc::new(img),
                            halo: Arc::new(halo),
                            halo_tex: None,
                            x: sd.x,
                            y: sd.y,
                            size: sd.size,
                            rot: sd.rot,
                            anim: sd.anim,
                            outline: sd.outline,
                            sym: sd.sym,
                            flip: sd.flip,
                            born: t,
                            source: Some((sd.file, sd.cut)),
                        });
                        next_sticker += 1;
                    }
                    send_all(&clients, stickers_msg(&stickers));
                    rebuild = true;
                    dirty = true;
                }
                Ev::Clear => {
                    strokes.clear();
                    stickers.clear();
                    send_all(&clients, stickers_msg(&stickers));
                    dirty = true;
                }
                Ev::Screen => {}
                Ev::Notice(text) => send_all(&clients, Message::text(serde_json::json!({ "t": "notice", "text": text }).to_string())),
                Ev::Sticker(mut st) => {
                    st.id = next_sticker;
                    next_sticker += 1;
                    st.born = t;
                    stickers.push(*st);
                    send_all(&clients, stickers_msg(&stickers));
                    send_all(&clients, Message::text(r#"{"t":"notice","text":"Photo ajoutée au dessin ✓"}"#));
                    shared.remote.lock().unwrap().push(Remote::DrawingUsed);
                    repaint();
                    dirty = true;
                }
                Ev::Msg { msg: In::StickerMove { id, x, y, s: size, r }, .. } => {
                    if let Some(st) = stickers.iter_mut().find(|st| st.id == id) {
                        let ok = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
                        st.x = ok(x, -0.5, 1.5, st.x);
                        st.y = ok(y, -0.5, 1.5, st.y);
                        st.size = ok(size, 0.03, 3.0, st.size);
                        st.rot = ok(r, -36_000.0, 36_000.0, st.rot);
                        dirty = true;
                    }
                }
                Ev::Msg { msg: In::StickerSet { id, anim, outline, sym, flip }, .. } => {
                    if let Some(st) = stickers.iter_mut().find(|st| st.id == id) {
                        if let Some(a) = anim {
                            st.anim = a;
                            st.born = t;
                        }
                        if let Some(c) = outline {
                            st.outline = (!c.is_empty()).then(|| parse_color(&c).map(to8));
                        }
                        if let Some(n) = sym.filter(|n| [1, 2, 4, 6, 8].contains(n)) {
                            st.sym = n;
                        }
                        if let Some(f) = flip {
                            st.flip = f;
                        }
                        send_all(&clients, stickers_msg(&stickers));
                        dirty = true;
                    }
                }
                Ev::Msg { msg: In::StickerDel { id }, .. } => {
                    stickers.retain(|st| st.id != id);
                    send_all(&clients, stickers_msg(&stickers));
                    dirty = true;
                }
                Ev::Msg { msg: In::StickerDup { id }, .. } => {
                    if let Some(st) = stickers.iter().find(|st| st.id == id) {
                        let mut copy = st.clone();
                        copy.id = next_sticker;
                        next_sticker += 1;
                        copy.x += 0.05;
                        copy.y += 0.05;
                        stickers.push(copy);
                        send_all(&clients, stickers_msg(&stickers));
                        dirty = true;
                    }
                }
                Ev::Msg { client, msg: In::View { w, screen } } => {
                    if let Some(c) = clients.lock().unwrap().iter_mut().find(|c| c.id == client) {
                        c.width = w.clamp(160, 4096);
                        c.screen = screen;
                    }
                    preview_dirty = true;
                }
                Ev::Msg { msg: In::Erase { x, y, r }, .. } => {
                    dirty |= erase(&mut strokes, [x, y], r, (pix.width() as f32, pix.height() as f32), t);
                }
                Ev::Msg { msg: In::Save { slot }, .. } if slot < SLOTS => {
                    memories[slot] = (!strokes.is_empty()).then(|| strokes.clone());
                    save_memories(&memories);
                    send_all(&clients, slots_msg(&memories));
                }
                Ev::Msg { msg: In::Recall { slot }, .. } if slot < SLOTS => {
                    if let Some(saved) = &memories[slot] {
                        strokes = saved.clone();
                        for s in &mut strokes {
                            s.client = 0;
                            s.born = t;
                            s.ended = Some(t);
                            s.times = vec![t; s.pts.len()];
                            s.baked = false;
                            s.sprite = None;
                        }
                        rebuild = true;
                        dirty = true;
                    }
                }
                Ev::Msg { msg: In::Act { a, on }, .. } => {
                    shared.remote.lock().unwrap().push(Remote::Trigger(a, on));
                    repaint();
                }
                Ev::Msg { client, msg: In::Room(m) } => {
                    if m == crate::room::RoomMsg::Watch {
                        // Première demande : la salle déjà prête part tout de suite à cette page.
                        let first = shared.room_watch.lock().unwrap().replace(Instant::now()).is_none_or(|t| t.elapsed() > Duration::from_secs(5));
                        let room = shared.room.lock().unwrap().1.clone();
                        if first && !room.is_empty() {
                            if let Some(c) = clients.lock().unwrap().iter().find(|c| c.id == client) {
                                let _ = c.tx.send(Message::text(room));
                            }
                        }
                    } else {
                        shared.remote.lock().unwrap().push(Remote::Room(m));
                    }
                    repaint();
                }
                Ev::Msg { msg: In::Chem(m), .. } => {
                    let mut remote = shared.remote.lock().unwrap();
                    // iPad penché : seule la dernière position compte.
                    if matches!(m, crate::chem::ChemMsg::Motion { .. }) {
                        remote.retain(|r| !matches!(r, Remote::Chem(crate::chem::ChemMsg::Motion { .. })));
                    }
                    remote.push(Remote::Chem(m));
                    drop(remote);
                    repaint();
                }
                Ev::Msg { msg: In::Val { a, v }, .. } => {
                    let mut remote = shared.remote.lock().unwrap();
                    // Un curseur qu'on glisse : seule la dernière valeur compte.
                    remote.retain(|r| !matches!(r, Remote::Value(b, _) if *b == a));
                    remote.push(Remote::Value(a, if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }));
                    drop(remote);
                    repaint();
                }
                Ev::Msg { client, msg } => {
                    if matches!(msg, In::Down { .. }) && strokes.is_empty() {
                        shared.remote.lock().unwrap().push(Remote::DrawingUsed);
                        repaint();
                    }
                    dirty |= apply(&mut strokes, client, msg, t);
                }
            }
        }

        shared.strokes.store(strokes.len(), Ordering::Relaxed);
        shared.stickers.store(stickers.len(), Ordering::Relaxed);

        let live = {
            let live = shared.live.lock().unwrap();
            (live.0 != live_version).then(|| (live.0, live.1.clone()))
        };
        if let Some((version, json)) = live {
            live_version = version;
            send_all(&clients, Message::text(json));
        }
        let room = {
            let room = shared.room.lock().unwrap();
            (room.0 != room_version).then(|| (room.0, room.1.clone()))
        };
        if let Some((version, json)) = room {
            room_version = version;
            send_all(&clients, Message::text(json));
        }

        let a = load(&shared.aspect);
        if (a - aspect).abs() > 0.005 {
            aspect = a;
            pix = canvas(aspect);
            base = canvas(aspect);
            base.fill(Color::TRANSPARENT);
            rebuild = true;
            clients.lock().unwrap().retain(|c| c.tx.send(hello(aspect)).is_ok());
            dirty = true;
        }

        // Dessin à enregistrer avec le projet : refait au plus une fois par seconde s'il a changé
        // (traits permanents terminés, photos avec leur fichier d'origine).
        doc_dirty |= dirty;
        if doc_dirty && last_doc.elapsed() >= Duration::from_secs(1) {
            doc_dirty = false;
            last_doc = Instant::now();
            let doc = DrawingDoc {
                strokes: strokes.iter().filter(|s| s.ended.is_some() && s.life <= 0.0).cloned().collect(),
                stickers: stickers
                    .iter()
                    .filter_map(|st| {
                        let (file, cut) = st.source.clone()?;
                        Some(StickerDoc {
                            file,
                            cut,
                            x: st.x,
                            y: st.y,
                            size: st.size,
                            rot: st.rot,
                            anim: st.anim,
                            outline: st.outline,
                            sym: st.sym,
                            flip: st.flip,
                        })
                    })
                    .collect(),
            };
            let mut cur = shared.doc.lock().unwrap();
            if cur.1.as_deref() != Some(&doc) {
                *cur = (cur.0 + 1, Some(Arc::new(doc)));
            }
        }

        // Même condition que l'attente plus haut (photos animées comprises) : sinon, avec une
        // photo animée et aucun trait animé, l'attente tombe à zéro et la boucle tourne à vide.
        let animated = strokes.iter().any(|s| s.moving(t)) || stickers.iter().any(Sticker::animated);
        let every = if dirty { FRAME_EVERY } else { ANIM_EVERY };
        if seen && (dirty || animated) && last_frame.elapsed() >= every {
            last_frame = Instant::now();
            let beat = load(&shared.beat);
            let size = (pix.width() as f32, pix.height() as f32);
            strokes.retain(|s| s.alpha(t) > 0.0);
            // Un trait déjà dessiné a disparu (annuler, effacer, trop de traits) : on refait la base.
            if rebuild || strokes.iter().filter(|s| s.baked).count() < baked {
                base.fill(Color::TRANSPARENT);
                for s in &mut strokes {
                    s.baked = false;
                    if rebuild {
                        s.sprite = None;
                    }
                }
                rebuild = false;
            }
            for s in strokes.iter_mut().filter(|s| !s.baked && s.settled()) {
                draw_stroke(&mut base, s, t, beat);
                s.baked = true;
                s.sprite = None;
            }
            baked = strokes.iter().filter(|s| s.baked).count();
            // Traits animés terminés : dessinés une fois dans leur image.
            for s in strokes.iter_mut().filter(|s| !s.baked) {
                if !s.spriteable(t) {
                    s.sprite = None;
                } else if s.sprite.is_none() {
                    s.sprite = make_sprite(s, t, size).map(Arc::new);
                }
            }
            if stickers.is_empty() {
                // Rien sous les traits : une simple copie de la base.
                pix.data_mut().copy_from_slice(base.data());
            } else {
                pix.fill(Color::TRANSPARENT);
                for st in &mut stickers {
                    draw_sticker(&mut pix, st, t, beat);
                }
                // Traits fixés par-dessus les photos (rien à poser s'il n'y en a pas : 8 Mo de moins
                // à mélanger à chaque image d'une photo animée).
                if baked > 0 {
                    pix.draw_pixmap(0, 0, base.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                }
            }
            for s in strokes.iter().filter(|s| !s.baked) {
                match &s.sprite {
                    Some(sp) => draw_sprite(&mut pix, s, sp, t, beat),
                    None => draw_stroke(&mut pix, s, t, beat),
                }
            }
            shared.strokes.store(strokes.len(), Ordering::Relaxed);
            let mut slot = shared.slot.lock().unwrap();
            // Le tampon de l'image précédente resert (pas de nouvelle allocation de 8 Mo).
            let mut data = slot.take().map(|f| f.data).filter(|d| d.len() == pix.data().len()).unwrap_or_default();
            data.clear();
            data.extend_from_slice(pix.data());
            *slot = Some(Frame {
                data,
                width: pix.width(),
                height: pix.height(),
                id: crate::video::new_frame_id(),
            });
            drop(slot);
            repaint();
            dirty = false;
            preview_dirty = true;
        }

        // Aperçu des pages : l'image de la sortie quand l'app l'envoie (ce qui est vraiment
        // projeté), sinon le dessin seul.
        let serial = shared.screen_serial.load(Ordering::Relaxed);
        if serial != screen_seen {
            screen_seen = serial;
            screen_at = Some(Instant::now());
            preview_dirty = true;
        }
        // Une page au moins veut voir la sortie : l'app la lit (sinon elle ne la lit pas du tout).
        let want = clients.lock().unwrap().iter().any(|c| c.screen);
        shared.want_screen.store(want, Ordering::Relaxed);
        let screen_live = want && screen_at.is_some_and(|at| at.elapsed() < Duration::from_secs(1));
        let preview_every = if screen_live { SCREEN_PREVIEW_EVERY } else { PREVIEW_EVERY };
        if seen && preview_dirty && last_preview.elapsed() >= preview_every {
            let width = clients.lock().unwrap().iter().map(|c| c.width).max();
            let screen = if screen_live { shared.screen.lock().unwrap().clone() } else { None };
            if let Some(width) = width {
                let shot = match screen {
                    Some((data, size)) => Shot { data, size, width, gl: true },
                    None => Shot { data: Arc::new(pix.data().to_vec()), size: (pix.width(), pix.height()), width, gl: false },
                };
                // Encodeur occupé : on réessaie au tour suivant.
                if shot_tx.try_send(shot).is_ok() {
                    last_preview = Instant::now();
                    preview_dirty = false;
                }
            }
        }
    }
}

fn send_all(clients: &Mutex<Vec<Client>>, msg: Message) {
    clients.lock().unwrap().retain(|c| c.tx.send(msg.clone()).is_ok());
}

/// Mémoires occupées, pour la page.
fn slots_msg(memories: &[Option<Vec<Stroke>>]) -> Message {
    let full: Vec<bool> = memories.iter().map(Option::is_some).collect();
    Message::text(serde_json::json!({ "t": "slots", "full": full }).to_string())
}

/// Gomme : retire les traits dont un point (ou son reflet) passe à moins de `r` de `at`.
fn erase(strokes: &mut Vec<Stroke>, at: [f32; 2], r: f32, (w, h): (f32, f32), t: f32) -> bool {
    let (ex, ey, er) = (at[0] * w, at[1] * h, r.clamp(0.002, 0.5) * h);
    let before = strokes.len();
    strokes.retain(|s| {
        let reach = er + s.width * h * 0.6;
        let copies = copies(s, w, h, t);
        !s.pts.iter().any(|&[x, y, _]| {
            copies.iter().any(|tr| {
                let mut p = [tiny_skia::Point::from_xy(x * w, y * h)];
                tr.map_points(&mut p);
                (p[0].x - ex).hypot(p[0].y - ey) < reach
            })
        })
    });
    strokes.len() != before
}

fn encode_loop(rx: Receiver<Shot>, clients: Arc<Mutex<Vec<Client>>>) {
    for shot in rx {
        let Some(jpeg) = preview(shot) else { continue };
        let msg = Message::binary(jpeg);
        clients.lock().unwrap().retain(|c| c.tx.send(msg.clone()).is_ok());
    }
}

fn canvas(aspect: f32) -> Pixmap {
    let w = ((HEIGHT as f32 * aspect).round() as u32).clamp(64, 4096);
    let mut pix = Pixmap::new(w, HEIGHT).expect("image du dessin");
    pix.fill(Color::BLACK);
    pix
}

/// Aperçu JPEG de l'image, à la largeur de la toile de la page (sans dépasser l'image).
fn preview(shot: Shot) -> Option<Vec<u8>> {
    let (w, h) = shot.size;
    let data = if shot.gl {
        // Remise à l'endroit, opaque.
        let row = w as usize * 4;
        let mut out = Vec::with_capacity(shot.data.len());
        for line in shot.data.chunks_exact(row).rev() {
            out.extend(line.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2], 255]));
        }
        out
    } else {
        shot.data.as_ref().clone()
    };
    let full = Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h)?)?;
    let pw = shot.width.clamp(160, w);
    let small = if pw < w {
        let ph = ((pw as f32 * h as f32 / w as f32).round() as u32).max(1);
        let mut small = Pixmap::new(pw, ph)?;
        let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
        small.draw_pixmap(0, 0, full.as_ref(), &paint, Transform::from_scale(pw as f32 / w as f32, ph as f32 / h as f32), None);
        small
    } else {
        full
    };
    let rgb: Vec<u8> = small.data().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 78)
        .encode(&rgb, small.width(), small.height(), image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(out)
}

/// Applique un message ; vrai si l'image change.
fn apply(strokes: &mut Vec<Stroke>, client: u32, msg: In, t: f32) -> bool {
    let clean = |v: f32| if v.is_finite() { v.clamp(-0.5, 1.5) } else { 0.0 };
    match msg {
        In::Down { id, x, y, p, color, w, brush, anim, sym, life, out } => {
            if strokes.len() >= MAX_STROKES {
                strokes.remove(0);
            }
            strokes.push(Stroke {
                client,
                id,
                color: parse_color(&color),
                width: if w.is_finite() { w.clamp(0.001, 0.2) } else { default_width() },
                brush,
                anim,
                sym: if [1, 2, 4, 6, 8].contains(&sym) { sym } else { 1 },
                life: if life.is_finite() { life.clamp(0.0, 600.0) } else { 0.0 },
                out,
                pts: vec![[clean(x), clean(y), clean(p).clamp(0.0, 1.0)]],
                times: vec![t],
                born: t,
                ended: None,
                baked: false,
                sprite: None,
            });
            true
        }
        In::Move { id, pts } => {
            let Some(s) = strokes.iter_mut().rev().find(|s| s.client == client && s.id == id && s.ended.is_none()) else {
                return false;
            };
            for [x, y, p] in pts {
                if s.pts.len() >= MAX_POINTS {
                    break;
                }
                s.pts.push([clean(x), clean(y), clean(p).clamp(0.0, 1.0)]);
                s.times.push(t);
            }
            true
        }
        In::Up { id } => {
            if let Some(s) = strokes.iter_mut().rev().find(|s| s.client == client && s.id == id) {
                s.ended.get_or_insert(t);
            }
            true
        }
        In::Undo => match strokes.iter().rposition(|s| s.client == client) {
            Some(i) => {
                strokes.remove(i);
                true
            }
            None => false,
        },
        In::Clear => {
            strokes.clear();
            true
        }
        // Traités par la boucle de rendu.
        In::View { .. }
        | In::Erase { .. }
        | In::Save { .. }
        | In::Recall { .. }
        | In::Act { .. }
        | In::Val { .. }
        | In::StickerMove { .. }
        | In::StickerSet { .. }
        | In::StickerDel { .. }
        | In::StickerDup { .. }
        | In::Chem(_)
        | In::Room(_) => false,
    }
}

/// "#rrggbb" → couleur (0..1) ; blanc si illisible.
fn parse_color(s: &str) -> [f32; 3] {
    let hex = s.trim_start_matches('#');
    let channel = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|v| v as f32 / 255.0);
    match (hex.len(), channel(0), channel(2), channel(4)) {
        (6, Some(r), Some(g), Some(b)) => [r, g, b],
        _ => [1.0, 1.0, 1.0],
    }
}

fn hue(h: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let f = |n: f32| {
        let k = (n + h) % 6.0;
        1.0 - (k.min(4.0 - k).clamp(0.0, 1.0))
    };
    [f(5.0), f(3.0), f(1.0)]
}

/// Point dessiné : position en pixels et rayon.
type Dot = (f32, f32, f32);

/// Opacité du trait à l'instant `t` (disparition et clignotement).
fn stroke_alpha(s: &Stroke, t: f32, beat: f32) -> f32 {
    let a = s.alpha(t);
    if s.anim == Anim::Blink { a * (0.12 + 0.88 * beat) } else { a }
}

/// Dessine un trait.
fn draw_stroke(pix: &mut Pixmap, s: &Stroke, t: f32, beat: f32) {
    let a = stroke_alpha(s, t, beat);
    if a <= 0.003 || s.pts.is_empty() {
        return;
    }
    let size = (pix.width() as f32, pix.height() as f32);
    let copies = copies(s, size.0, size.1, t);
    paint_stroke(pix, s, t, beat, a, size, &copies);
}

/// Dessine une fois le trait dans une image à sa taille (symétrie comprise, sans rotation ni
/// opacité : elles sont appliquées en posant l'image). `size` : taille du dessin.
fn make_sprite(s: &Stroke, t: f32, (w, h): (f32, f32)) -> Option<Sprite> {
    let dots = dots(s, w, h, t, 0.0);
    let reach = dots.iter().fold(0.0f32, |m, d| m.max(d.2)) * 4.0 + 4.0;
    let sym = sym_copies(s.sym, 0.0, w, h);
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for tr in &sym {
        for &(x, y, _) in &dots {
            let mut p = [tiny_skia::Point::from_xy(x, y)];
            tr.map_points(&mut p);
            lo = [lo[0].min(p[0].x - reach), lo[1].min(p[0].y - reach)];
            hi = [hi[0].max(p[0].x + reach), hi[1].max(p[0].y + reach)];
        }
    }
    // Ce qui peut entrer dans l'image en tournant autour du centre, pas plus.
    let r = (w * w + h * h).sqrt() * 0.5;
    let (x0, y0) = ((lo[0].max(w * 0.5 - r)).floor(), (lo[1].max(h * 0.5 - r)).floor());
    let (x1, y1) = ((hi[0].min(w * 0.5 + r)).ceil(), (hi[1].min(h * 0.5 + r)).ceil());
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut pix = Pixmap::new((x1 - x0) as u32, (y1 - y0) as u32)?;
    let copies: Vec<Transform> = sym.iter().map(|tr| tr.post_translate(-x0, -y0)).collect();
    paint_stroke(&mut pix, s, t, 0.0, 1.0, (w, h), &copies);
    Some(Sprite { pix, x: x0, y: y0 })
}

/// Pose l'image d'un trait : opacité (fondu, clignotement) et rotation (« tourne »).
fn draw_sprite(pix: &mut Pixmap, s: &Stroke, sp: &Sprite, t: f32, beat: f32) {
    let a = stroke_alpha(s, t, beat);
    if a <= 0.003 {
        return;
    }
    let spin = if s.anim == Anim::Spin { (t - s.born) * 35.0 } else { 0.0 };
    let mut tr = Transform::from_translate(sp.x, sp.y);
    if spin != 0.0 {
        tr = tr.post_concat(Transform::from_rotate_at(spin, pix.width() as f32 * 0.5, pix.height() as f32 * 0.5));
    }
    let paint = PixmapPaint {
        opacity: a.min(1.0),
        quality: if spin != 0.0 { tiny_skia::FilterQuality::Bilinear } else { tiny_skia::FilterQuality::Nearest },
        ..PixmapPaint::default()
    };
    pix.draw_pixmap(0, 0, sp.pix.as_ref(), &paint, tr, None);
}

/// Dessine le trait sur `pix`, une fois par copie. `(w, h)` : taille du dessin (les points du
/// trait y sont exprimés), qui peut différer de celle de `pix` (image d'un trait).
fn paint_stroke(pix: &mut Pixmap, s: &Stroke, t: f32, beat: f32, a: f32, (w, h): (f32, f32), copies: &[Transform]) {
    let q = s.gone(t);
    let mut dots = dots(s, w, h, t, beat);
    if s.anim == Anim::Trace && s.ended.is_some() {
        // Se dessine en boucle : apparaît du début à la fin, reste un instant, recommence.
        let period = (dots.len() as f32 * 0.012).clamp(1.2, 6.0);
        let k = ((t - s.born) / period).rem_euclid(1.35).min(1.0);
        dots.truncate(((dots.len() as f32 * k).ceil() as usize).max(1));
    }
    if s.out == Vanish::Tail && q > 0.0 {
        let cut = ((dots.len() as f32 * q) as usize).min(dots.len().saturating_sub(1));
        dots.drain(..cut);
    }
    if dots.is_empty() {
        return;
    }
    let glow = if s.anim == Anim::Pulse { 1.0 + beat * 0.8 } else { 1.0 };
    let mut pen = Pen { pix, copies };
    if s.out == Vanish::Burst && q > 0.0 {
        // Explose : le trait part en étincelles depuis son centre.
        let n = dots.len() as f32;
        let (cx, cy) = dots.iter().fold((0.0, 0.0), |(x, y), d| (x + d.0 / n, y + d.1 / n));
        let color = if s.brush == Brush::Rainbow { [1.0, 1.0, 1.0] } else { s.color };
        for (i, &(x, y, r)) in spaced(&dots).iter().enumerate() {
            let ang = rnd(s.id, i as u32) * std::f32::consts::TAU;
            let (dx, dy) = (x - cx + ang.cos() * h * 0.05, y - cy + ang.sin() * h * 0.05);
            let go = q * (0.6 + rnd(s.id, i as u32 + 7919));
            let (px, py) = (x + dx * go * 1.5, y + dy * go * 1.5 + q * q * h * 0.08);
            let size = (r * (1.0 - q * 0.7)).max(0.8);
            pen.fill(&PathBuilder::from_circle(px, py, size * 2.0), color, 0.2 * a, true);
            pen.fill(&PathBuilder::from_circle(px, py, size * 0.7), [1.0, 1.0, 1.0], a, true);
        }
        return;
    }
    match s.brush {
        Brush::Line => pen.line(&dots, 1.0, s.color, a, false, true),
        Brush::Neon => {
            pen.line(&dots, 3.6, s.color, 0.09 * a * glow, true, false);
            pen.line(&dots, 2.0, s.color, 0.22 * a * glow, true, false);
            pen.line(&dots, 1.0, s.color, 0.85 * a, false, true);
            let core = [0, 1, 2].map(|i| s.color[i] + (1.0 - s.color[i]) * 0.75);
            pen.line(&dots, 0.42, core, a, false, true);
        }
        Brush::Rainbow => {
            // Morceaux de quelques points, chacun sa teinte ; les couleurs défilent.
            const RUN: usize = 6;
            let mut i = 0;
            while i < dots.len() {
                let run = &dots[i..(i + RUN + 1).min(dots.len())];
                let c = hue(i as f32 * 0.012 - (t - s.born) * 0.35);
                pen.line(run, 2.2, c, 0.14 * a * glow, true, false);
                pen.line(run, 1.0, c, a, false, false);
                i += RUN;
            }
        }
        Brush::Sparkle => {
            for (k, &(x, y, r)) in spaced(&dots).iter().enumerate() {
                let tw = 0.5 + 0.5 * (t * 7.0 + k as f32 * 2.39).sin();
                let size = r * (0.8 + 1.2 * tw) * glow;
                pen.fill(&PathBuilder::from_circle(x, y, size * 2.2), s.color, 0.16 * a * tw, true);
                let core = [0, 1, 2].map(|i| s.color[i] + (1.0 - s.color[i]) * 0.6);
                pen.fill(&star(x, y, size), core, a * (0.4 + 0.6 * tw), true);
            }
        }
        Brush::Spray => {
            // Particules semées autour du trait (toujours les mêmes : le trait reste immobile).
            let mut pb = PathBuilder::new();
            let mut k = 0u32;
            for &(x, y, r) in &dots {
                for _ in 0..9 {
                    k += 1;
                    let ang = rnd(s.id, k) * std::f32::consts::TAU;
                    let dist = rnd(s.id, k + 104_729).powf(0.7) * r * 2.4;
                    let size = (r * 0.2 * (0.4 + rnd(s.id, k + 15_485_863))).max(0.9) * glow;
                    pb.push_circle(x + ang.cos() * dist, y + ang.sin() * dist, size);
                }
            }
            pen.fill(&pb.finish(), s.color, a, true);
        }
        Brush::Dots => {
            let mut glow_pb = PathBuilder::new();
            let mut core_pb = PathBuilder::new();
            for &(x, y, r) in &spaced_by(&dots, 2.6) {
                glow_pb.push_circle(x, y, r * 1.9 * glow);
                core_pb.push_circle(x, y, r * 0.8 * glow);
            }
            pen.fill(&glow_pb.finish(), s.color, 0.2 * a, true);
            let core = [0, 1, 2].map(|i| s.color[i] + (1.0 - s.color[i]) * 0.5);
            pen.fill(&core_pb.finish(), core, a, false);
        }
    }
}

/// Nombre pseudo-aléatoire stable (0..1) pour le trait `id` et l'indice `k`.
fn rnd(id: u32, k: u32) -> f32 {
    let mut x = id.wrapping_mul(0x9E37_79B9) ^ k.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Dessine sur l'image, une fois par copie (symétrie, rotation).
struct Pen<'a> {
    pix: &'a mut Pixmap,
    copies: &'a [Transform],
}

impl Pen<'_> {
    fn paint(c: [f32; 3], alpha: f32, add: bool) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        paint.set_color_rgba8(to8(c[0]), to8(c[1]), to8(c[2]), to8(alpha));
        if add {
            paint.blend_mode = tiny_skia::BlendMode::Plus;
        }
        paint
    }

    fn fill(&mut self, path: &Option<Path>, c: [f32; 3], alpha: f32, add: bool) {
        let Some(path) = path else { return };
        let paint = Self::paint(c, alpha, add);
        for tr in self.copies {
            self.pix.fill_path(path, &paint, FillRule::Winding, *tr, None);
        }
    }

    /// Trait passant par les points, `k` fois leur rayon. Avec `pressure`, l'épaisseur suit la
    /// pression par petits morceaux (sinon : épaisseur moyenne, d'un seul tenant).
    fn line(&mut self, dots: &[Dot], k: f32, c: [f32; 3], alpha: f32, add: bool, pressure: bool) {
        let Some(&(x0, y0, r0)) = dots.first() else { return };
        let span = dots.iter().fold(0.0f32, |m, d| m.max((d.0 - x0).abs() + (d.1 - y0).abs()));
        if dots.len() == 1 || span < 0.5 {
            self.fill(&PathBuilder::from_circle(x0, y0, (r0 * k).max(0.5)), c, alpha, add);
            return;
        }
        let paint = Self::paint(c, alpha, add);
        let (lo, hi) = dots.iter().fold((f32::MAX, 0.0f32), |(lo, hi), d| (lo.min(d.2), hi.max(d.2)));
        const PIECE: usize = 6;
        let step = if pressure && hi > lo * 1.2 { PIECE } else { dots.len() };
        let mut i = 0;
        while i + 1 < dots.len() {
            let run = &dots[i..(i + step + 1).min(dots.len())];
            i += step;
            let r = run.iter().map(|d| d.2).sum::<f32>() / run.len() as f32;
            let Some(path) = curve(run) else { continue };
            let stroke = tiny_skia::Stroke {
                width: (2.0 * r * k).max(1.0),
                line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round,
                ..tiny_skia::Stroke::default()
            };
            for tr in self.copies {
                self.pix.stroke_path(&path, &paint, &stroke, *tr, None);
            }
        }
    }
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Points du trait en pixels, animation et disparition comprises.
fn dots(s: &Stroke, w: f32, h: f32, t: f32, beat: f32) -> Vec<Dot> {
    let age = t - s.born;
    let base = s.width * h * 0.5;
    let pulse = if s.anim == Anim::Pulse { 1.0 + beat * 1.1 } else { 1.0 };
    let seed = (s.id % 97) as f32 * 0.71;
    let q = s.gone(t);
    let n = s.pts.len();
    let mut out: Vec<(f32, f32, f32, usize)> = Vec::with_capacity(n);
    for (i, &[x, y, p]) in s.pts.iter().enumerate() {
        let mut r = (base * (0.35 + 1.3 * p) * pulse).max(0.6);
        if s.out == Vanish::Trail && s.life > 0.0 {
            // Comète : le bout le plus ancien s'affine puis disparaît.
            let left = 1.0 - (t - s.times.get(i).copied().unwrap_or(s.born)) / s.life;
            if left <= 0.0 {
                continue;
            }
            r *= (left / 0.5).min(1.0);
        }
        let (px, py) = (x * w, y * h);
        // Points trop serrés (dessin lent) : un sur quelques-uns suffit, le dernier est toujours gardé.
        if let Some(&(lx, ly, _, _)) = out.last()
            && i + 1 < n
            && (px - lx).abs() + (py - ly).abs() < (r * 0.3).max(1.5)
        {
            continue;
        }
        out.push((px, py, r, i));
    }
    out.into_iter()
        .map(|(x, y, mut r, i)| {
            let (mut px, mut py) = (x, y);
            let (u, v) = (x / w, y / h);
            match s.anim {
                Anim::Wave => {
                    px += (v * 9.0 + t * 3.1).sin() * h * 0.012;
                    py += (u * 13.0 + t * 4.3).sin() * h * 0.025;
                }
                Anim::Float => {
                    py -= age * h * 0.05;
                    px += (age * 1.3 + seed + v * 4.0).sin() * h * 0.02;
                }
                _ => {}
            }
            if q > 0.0 {
                match s.out {
                    Vanish::Shrink => r *= 1.0 - q,
                    Vanish::Rise => {
                        py -= q * q * h * 0.35;
                        px += (q * 6.0 + v * 9.0 + seed).sin() * q * h * 0.03;
                    }
                    Vanish::Melt => {
                        // Chaque goutte coule à sa vitesse.
                        let speed = 0.4 + rnd(s.id, (i / 3) as u32);
                        py += q.powf(1.6) * h * 0.4 * speed;
                        r *= 1.0 - q * 0.5;
                    }
                    _ => {}
                }
            }
            (px, py, r.max(0.3))
        })
        .collect()
}

/// Copies du trait : symétrie, et rotation de l'animation « tourne ».
fn copies(s: &Stroke, w: f32, h: f32, t: f32) -> Vec<Transform> {
    let spin = if s.anim == Anim::Spin { (t - s.born) * 35.0 } else { 0.0 };
    sym_copies(s.sym, spin, w, h)
}

/// Copies d'un élément : symétrie (miroir, quatre coins, kaléidoscope), puis rotation `spin`
/// (degrés) autour du centre de l'image.
fn sym_copies(sym: u8, spin: f32, w: f32, h: f32) -> Vec<Transform> {
    let (cx, cy) = (w * 0.5, h * 0.5);
    let spin = if spin != 0.0 { Transform::from_rotate_at(spin, cx, cy) } else { Transform::identity() };
    let mirror_x = Transform::from_row(-1.0, 0.0, 0.0, 1.0, w, 0.0);
    let mirror_y = Transform::from_row(1.0, 0.0, 0.0, -1.0, 0.0, h);
    let base: Vec<Transform> = match sym {
        2 => vec![Transform::identity(), mirror_x],
        4 => vec![Transform::identity(), mirror_x, mirror_y, mirror_x.post_concat(mirror_y)],
        n @ (6 | 8) => (0..n)
            .map(|k| {
                // Kaléidoscope : une copie sur deux est retournée, comme dans un miroir.
                let rot = Transform::from_rotate_at(360.0 / n as f32 * k as f32, cx, cy);
                if k % 2 == 1 {
                    Transform::from_row(-1.0, 0.0, 0.0, 1.0, w, 0.0).post_concat(rot)
                } else {
                    rot
                }
            })
            .collect(),
        _ => vec![Transform::identity()],
    };
    base.into_iter().map(|tr| tr.post_concat(spin)).collect()
}

/// Courbe lissée passant au milieu des segments (les angles du tracé à main levée s'arrondissent).
fn curve(run: &[Dot]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(run[0].0, run[0].1);
    if run.len() > 2 {
        for k in 1..run.len() - 1 {
            let (a, b) = (run[k], run[k + 1]);
            pb.quad_to(a.0, a.1, (a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
        }
    }
    let last = run[run.len() - 1];
    pb.line_to(last.0, last.1);
    pb.finish()
}

/// Points régulièrement espacés le long du trait (étoiles du pinceau « étincelles »).
fn spaced(dots: &[Dot]) -> Vec<Dot> {
    spaced_by(dots, 3.0)
}

/// Points espacés d'environ `gap` fois leur rayon.
fn spaced_by(dots: &[Dot], gap: f32) -> Vec<Dot> {
    let mut out = Vec::new();
    let Some(&first) = dots.first() else { return out };
    out.push(first);
    let mut since = 0.0;
    for pair in dots.windows(2) {
        let ((x0, y0, _), (x1, y1, r1)) = (pair[0], pair[1]);
        since += ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        if since >= (r1 * gap).max(6.0) {
            since = 0.0;
            out.push(pair[1]);
        }
    }
    out
}

/// Étoile à quatre branches.
fn star(x: f32, y: f32, s: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let (long, thin) = (s * 2.2, s * 0.32);
    pb.move_to(x, y - long);
    pb.line_to(x + thin, y - thin);
    pb.line_to(x + long, y);
    pb.line_to(x + thin, y + thin);
    pb.line_to(x, y + long);
    pb.line_to(x - thin, y + thin);
    pb.line_to(x - long, y);
    pb.line_to(x - thin, y - thin);
    pb.close();
    pb.push_circle(x, y, s * 0.55);
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(pix: &mut Pixmap, strokes: &[Stroke], t: f32, beat: f32) {
        pix.fill(Color::BLACK);
        for s in strokes {
            draw_stroke(pix, s, t, beat);
        }
    }

    fn down(id: u32, brush: Brush, sym: u8) -> In {
        In::Down { id, x: 0.2, y: 0.5, p: 0.5, color: "#ff0000".into(), w: 0.02, brush, anim: Anim::None, sym, life: 0.0, out: Vanish::Fade }
    }

    fn lit(pix: &Pixmap, x: u32, y: u32) -> bool {
        let p = pix.pixel(x, y).unwrap();
        p.red() > 60 || p.green() > 60 || p.blue() > 60
    }

    #[test]
    fn messages_parse() {
        let m: In = serde_json::from_str(
            r##"{"t":"down","id":3,"x":0.1,"y":0.2,"p":0.7,"color":"#00ff80","w":0.01,"brush":"neon","anim":"pulse","sym":4,"life":5}"##,
        )
        .unwrap();
        assert!(matches!(m, In::Down { id: 3, brush: Brush::Neon, anim: Anim::Pulse, sym: 4, .. }));
        let m: In = serde_json::from_str(r#"{"t":"move","id":3,"pts":[[0.2,0.2,0.5],[0.3,0.2,0.6]]}"#).unwrap();
        assert!(matches!(m, In::Move { id: 3, ref pts } if pts.len() == 2));
        assert!(matches!(serde_json::from_str::<In>(r#"{"t":"undo"}"#).unwrap(), In::Undo));
        assert_eq!(parse_color("#00ff80"), [0.0, 1.0, 128.0 / 255.0]);
        assert_eq!(parse_color("n'importe quoi"), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn strokes_draw_with_symmetry() {
        let mut strokes = Vec::new();
        for brush in [Brush::Line, Brush::Neon, Brush::Rainbow, Brush::Sparkle] {
            strokes.clear();
            apply(&mut strokes, 1, down(1, brush, 2), 0.0);
            apply(&mut strokes, 1, In::Move { id: 1, pts: vec![[0.3, 0.5, 0.5], [0.35, 0.5, 0.5]] }, 0.0);
            apply(&mut strokes, 1, In::Up { id: 1 }, 0.1);
            let mut pix = canvas(16.0 / 9.0);
            draw(&mut pix, &strokes, 0.2, 0.0);
            let (w, h) = (pix.width(), pix.height());
            // Le trait et son reflet (miroir gauche-droite) sont dessinés, le reste est noir.
            assert!(lit(&pix, (0.3 * w as f32) as u32, h / 2), "{brush:?}");
            assert!(lit(&pix, w - 1 - (0.3 * w as f32) as u32, h / 2), "{brush:?}");
            assert!(!lit(&pix, w / 2, h / 10), "{brush:?}");
        }
    }

    #[test]
    fn ephemeral_strokes_fade_and_undo() {
        let mut strokes = Vec::new();
        apply(
            &mut strokes,
            1,
            In::Down { id: 1, x: 0.5, y: 0.5, p: 0.5, color: String::new(), w: 0.02, brush: Brush::Line, anim: Anim::None, sym: 1, life: 2.0, out: Vanish::Fade },
            0.0,
        );
        apply(&mut strokes, 1, In::Up { id: 1 }, 1.0);
        assert_eq!(strokes[0].alpha(2.5), 1.0);
        assert!(strokes[0].alpha(3.5) > 0.0 && strokes[0].alpha(3.5) < 1.0);
        assert_eq!(strokes[0].alpha(5.0), 0.0);
        apply(&mut strokes, 2, down(1, Brush::Line, 1), 0.0);
        // Annuler ne retire que le dernier trait de la page qui le demande.
        apply(&mut strokes, 2, In::Undo, 0.0);
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].client, 1);
    }

    #[test]
    fn sprite_matches_direct_drawing() {
        // Image d'un trait (kaléidoscope, néon) posée sur le dessin = trait dessiné directement.
        for (brush, sym, anim) in [(Brush::Neon, 6, Anim::Spin), (Brush::Line, 2, Anim::Blink), (Brush::Dots, 4, Anim::None)] {
            let mut strokes = Vec::new();
            apply(&mut strokes, 1, In::Down { id: 1, x: 0.3, y: 0.4, p: 0.5, color: "#33ccff".into(), w: 0.02, brush, anim, sym, life: 0.0, out: Vanish::Fade }, 0.0);
            apply(&mut strokes, 1, In::Move { id: 1, pts: vec![[0.35, 0.45, 0.6], [0.45, 0.42, 0.7], [0.5, 0.3, 0.4]] }, 0.1);
            apply(&mut strokes, 1, In::Up { id: 1 }, 0.2);
            let s = &strokes[0];
            let t = 1.7;
            assert!(s.spriteable(t));
            let (mut direct, mut posed) = (Pixmap::new(320, 180).unwrap(), Pixmap::new(320, 180).unwrap());
            draw_stroke(&mut direct, s, t, 0.6);
            let sp = make_sprite(s, t, (320.0, 180.0)).unwrap();
            assert!(sp.pix.width() < 320 * 2 && sp.pix.height() < 180 * 2);
            draw_sprite(&mut posed, s, &sp, t, 0.6);
            let diff = direct.data().iter().zip(posed.data()).filter(|(a, b)| a.abs_diff(**b) > 40).count();
            let lit = direct.data().iter().filter(|&&v| v > 40).count();
            assert!(lit > 200, "{brush:?} : rien de dessiné");
            // Rotation : rééchantillonnage, quelques pixels de bord diffèrent.
            assert!(diff * 20 < lit, "{brush:?} {anim:?} : {diff} pixels différents sur {lit}");
        }
    }

    #[test]
    fn ephemeral_stroke_sleeps_until_it_fades() {
        let mut strokes = Vec::new();
        apply(&mut strokes, 1, In::Down { id: 1, x: 0.5, y: 0.5, p: 0.5, color: String::new(), w: 0.02, brush: Brush::Line, anim: Anim::None, sym: 1, life: 2.0, out: Vanish::Shrink }, 0.0);
        assert!(strokes[0].moving(0.5), "en cours de tracé : il change");
        apply(&mut strokes, 1, In::Up { id: 1 }, 1.0);
        assert_eq!(strokes[0].still_until(), Some(3.0));
        assert!(!strokes[0].moving(2.0));
        assert!(strokes[0].moving(3.2));
        assert!(strokes[0].spriteable(2.0) && !strokes[0].spriteable(3.2), "rétrécit : dessiné en direct");
    }

    #[test]
    fn chemistry_messages_parse() {
        let m: In = serde_json::from_str(r##"{"t":"chem","op":"drop","ing":"ink","x":0.5,"y":0.4,"color":"#ff00ff"}"##).unwrap();
        assert!(matches!(m, In::Chem(crate::chem::ChemMsg::Drop { .. })));
        let m: In = serde_json::from_str(r#"{"t":"chem","op":"motion","gx":0.1,"gy":0.9}"#).unwrap();
        assert!(matches!(m, In::Chem(crate::chem::ChemMsg::Motion { .. })));
        let m: In = serde_json::from_str(r#"{"t":"chem","op":"clear"}"#).unwrap();
        assert!(matches!(m, In::Chem(crate::chem::ChemMsg::Clear)));
    }

    #[test]
    fn live_commands_parse() {
        use crate::midi::Action;
        let m: In = serde_json::from_str(r#"{"t":"act","a":{"PlayColumn":{"column":2}},"on":true}"#).unwrap();
        assert!(matches!(m, In::Act { a: Action::PlayColumn { column: 2 }, on: true }));
        let m: In = serde_json::from_str(r#"{"t":"act","a":"Tap"}"#).unwrap();
        assert!(matches!(m, In::Act { a: Action::Tap, on: true }));
        let m: In = serde_json::from_str(r#"{"t":"val","a":{"Opacity":{"layer":1}},"v":0.25}"#).unwrap();
        assert!(matches!(m, In::Val { a: Action::Opacity { layer: Some(1) }, .. }));
        let m: In = serde_json::from_str(r#"{"t":"act","a":{"Visual":{"pattern":"Plasma"}}}"#).unwrap();
        assert!(matches!(m, In::Act { a: Action::Visual { pattern: crate::generator::Pattern::Plasma }, .. }));
        let m: In = serde_json::from_str(r#"{"t":"act","a":{"Mode":{"mode":"Chemistry"}}}"#).unwrap();
        assert!(matches!(m, In::Act { a: crate::midi::Action::Mode { mode: crate::midi::ShowMode::Chemistry }, .. }));
        let m: In = serde_json::from_str(r#"{"t":"act","a":{"LedScenario":{"kind":"Wave"}}}"#).unwrap();
        assert!(matches!(m, In::Act { a: Action::LedScenario { .. }, .. }));
        // Les noms envoyés par l'app (état Live) sont ceux que la page renvoie.
        assert_eq!(serde_json::to_value(crate::generator::Pattern::Plasma).unwrap(), "Plasma");
        assert_eq!(serde_json::to_value(crate::dmx::Kind::Smoke).unwrap(), "Smoke");
    }

    #[test]
    fn eraser_and_vanishing() {
        let mut strokes = Vec::new();
        apply(&mut strokes, 1, down(1, Brush::Line, 2), 0.0);
        apply(&mut strokes, 1, In::Up { id: 1 }, 0.0);
        apply(&mut strokes, 1, down(2, Brush::Line, 1), 0.0);
        if let Some(s) = strokes.last_mut() {
            s.pts = vec![[0.5, 0.1, 0.5]];
        }
        // La gomme passe sur le reflet (miroir) du premier trait : lui seul disparaît.
        assert!(erase(&mut strokes, [0.8, 0.5], 0.01, (1920.0, 1080.0), 0.0));
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].id, 2);
        assert!(!erase(&mut strokes, [0.9, 0.9], 0.01, (1920.0, 1080.0), 0.0));

        // Chaque disparition se dessine et finit par tout effacer.
        for out in [Vanish::Fade, Vanish::Shrink, Vanish::Tail, Vanish::Rise, Vanish::Burst, Vanish::Melt] {
            let mut st = Vec::new();
            apply(&mut st, 1, In::Down { id: 1, x: 0.3, y: 0.5, p: 0.5, color: "#ffffff".into(), w: 0.03, brush: Brush::Neon, anim: Anim::None, sym: 1, life: 1.0, out }, 0.0);
            apply(&mut st, 1, In::Move { id: 1, pts: (1..40).map(|k| [0.3 + k as f32 * 0.01, 0.5, 0.5]).collect() }, 0.0);
            apply(&mut st, 1, In::Up { id: 1 }, 0.0);
            let mut pix = canvas(16.0 / 9.0);
            draw(&mut pix, &st, 0.5, 0.0);
            assert!(lit(&pix, (0.45 * pix.width() as f32) as u32, pix.height() / 2), "{out:?} avant");
            draw(&mut pix, &st, 1.4, 0.0);
            assert!(st[0].alpha(1.4) > 0.0, "{out:?} pendant");
            assert_eq!(st[0].alpha(10.0), 0.0, "{out:?} après");
        }

        // Comète : le début du trait s'efface pendant qu'on dessine encore.
        let mut st = Vec::new();
        apply(&mut st, 1, In::Down { id: 1, x: 0.1, y: 0.5, p: 0.5, color: "#ffffff".into(), w: 0.03, brush: Brush::Line, anim: Anim::None, sym: 1, life: 1.0, out: Vanish::Trail }, 0.0);
        apply(&mut st, 1, In::Move { id: 1, pts: vec![[0.9, 0.5, 0.5]] }, 2.0);
        let mut pix = canvas(16.0 / 9.0);
        draw(&mut pix, &st, 2.2, 0.0);
        let (w, h) = (pix.width(), pix.height());
        assert!(!lit(&pix, (0.1 * w as f32) as u32, h / 2));
        assert!(lit(&pix, (0.9 * w as f32) as u32, h / 2));
        assert!(st[0].gone(2.2) < 1.0);
    }

    #[test]
    fn new_brushes_draw() {
        for brush in [Brush::Spray, Brush::Dots] {
            let mut st = Vec::new();
            apply(&mut st, 1, down(1, brush, 1), 0.0);
            apply(&mut st, 1, In::Move { id: 1, pts: (1..30).map(|k| [0.2 + k as f32 * 0.01, 0.5, 0.5]).collect() }, 0.0);
            apply(&mut st, 1, In::Up { id: 1 }, 0.0);
            assert!(st[0].settled(), "{brush:?} reste en mémoire d'image");
            let mut pix = canvas(16.0 / 9.0);
            draw(&mut pix, &st, 0.1, 0.0);
            let n = pix.pixels().iter().filter(|p| p.red() > 60).count();
            assert!(n > 200, "{brush:?} : {n} pixels");
        }
    }

    #[test]
    fn photo_in_drawing() {
        // Disque rouge sur fond blanc : le détourage garde le disque, recadré, avec sa marge.
        let img = image::RgbaImage::from_fn(400, 300, |x, y| {
            let d = ((x as f32 - 200.0).powi(2) + (y as f32 - 150.0).powi(2)).sqrt();
            if d < 80.0 { image::Rgba([220, 30, 40, 255]) } else { image::Rgba([250, 250, 250, 255]) }
        });
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let (pix, halo) = make_sticker(&png, "png", true).unwrap();
        assert!(pix.width() < 300 && pix.height() < 260, "recadré : {}×{}", pix.width(), pix.height());
        let center = pix.pixel(pix.width() / 2, pix.height() / 2).unwrap();
        assert!(center.alpha() > 240 && center.red() > 180);
        assert_eq!(pix.pixel(1, 1).unwrap().alpha(), 0, "le fond est transparent");
        assert_eq!(halo.len(), (pix.width() * pix.height()) as usize);

        // Posée au milieu de l'image du dessin, avec un contour : le reste reste transparent.
        let mut st = Sticker {
            id: 1, img: Arc::new(pix), halo: Arc::new(halo), halo_tex: None, x: 0.5, y: 0.5, size: 0.5, rot: 30.0,
            anim: Anim::Pulse, outline: Some([0, 255, 255]), sym: 2, flip: true, born: 0.0, source: None,
        };
        let mut canvas = Pixmap::new(640, 360).unwrap();
        draw_sticker(&mut canvas, &mut st, 0.1, 1.0);
        assert!(canvas.pixel(320, 180).unwrap().alpha() > 200);
        assert_eq!(canvas.pixel(5, 5).unwrap().alpha(), 0);
        let info = st.info();
        assert_eq!(info["anim"], "pulse");
        assert_eq!(info["outline"], true);
    }

    #[test]
    fn heic_photo_from_ipad() {
        // Une photo HEIC envoyée telle quelle (le navigateur n'a pas su la convertir) est lue.
        let dir = std::env::temp_dir().join(format!("oz-sketch-heic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("p.png");
        let heic = dir.join("p.heic");
        image::RgbaImage::from_pixel(64, 48, image::Rgba([200, 40, 40, 255])).save(&png).unwrap();
        let made = std::process::Command::new("sips").args(["-s", "format", "heic"]).arg(&png).arg("--out").arg(&heic).status();
        if made.is_ok_and(|s| s.success()) {
            let data = std::fs::read(&heic).unwrap();
            assert_eq!(photo_ext(&data, "to=drawing&cut=0"), "heic");
            let img = decode_photo(&data, "heic").unwrap();
            assert_eq!((img.width(), img.height()), (64, 48));
            let (pix, _) = make_sticker(&data, "heic", false).unwrap();
            assert!(pix.width() > 64);
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(photo_ext(b"\x89PNG....", ""), "png");
        assert_eq!(photo_ext(b"whatever", "to=layer&ext=HEIC"), "heic");
    }

    #[test]
    fn sticker_messages_parse() {
        let m: In = serde_json::from_str(r#"{"t":"stmove","id":2,"x":0.4,"y":0.6,"s":0.3,"r":45}"#).unwrap();
        assert!(matches!(m, In::StickerMove { id: 2, .. }));
        let m: In = serde_json::from_str(r##"{"t":"stset","id":2,"anim":"wave","outline":"#ff0000"}"##).unwrap();
        assert!(matches!(m, In::StickerSet { id: 2, anim: Some(Anim::Wave), sym: None, .. }));
        assert!(matches!(serde_json::from_str::<In>(r#"{"t":"stdel","id":3}"#).unwrap(), In::StickerDel { id: 3 }));
    }

    #[test]
    fn drawing_saved_and_reloaded() {
        // Un trait dessiné : le dessin publié pour le projet le contient…
        let a = Sketch::start(|| {}).unwrap();
        let (mut ws, _) = tungstenite::client::connect(format!("ws://127.0.0.1:{}/ws", a.port())).unwrap();
        ws.send(Message::text(r##"{"t":"down","id":1,"x":0.2,"y":0.5,"color":"#ff0000","w":0.03}"##)).unwrap();
        ws.send(Message::text(r#"{"t":"move","id":1,"pts":[[0.4,0.5,0.5],[0.6,0.6,0.5]]}"#)).unwrap();
        ws.send(Message::text(r#"{"t":"up","id":1}"#)).unwrap();
        let t0 = Instant::now();
        let doc = loop {
            if let (_, Some(d)) = a.drawing()
                && d.strokes.len() == 1
            {
                break d;
            }
            assert!(t0.elapsed() < Duration::from_secs(4), "dessin non publié");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(doc.strokes[0].pts.len(), 3);
        // … passe par le fichier projet (JSON) et revient dans un autre serveur.
        let json = serde_json::to_string(&*doc).unwrap();
        let back: DrawingDoc = serde_json::from_str(&json).unwrap();
        let b = Sketch::start(|| {}).unwrap();
        b.load_drawing(back);
        let t0 = Instant::now();
        while b.strokes() != 1 {
            assert!(t0.elapsed() < Duration::from_secs(3), "dessin non repris");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn page_and_socket() {
        use std::io::Read;
        let sketch = Sketch::start(|| {}).unwrap();
        let port = sketch.port();
        // La page.
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut body = String::new();
        s.read_to_string(&mut body).unwrap();
        assert!(body.starts_with("HTTP/1.1 200") && body.contains("<canvas"));
        // Le WebSocket : bonjour, puis un trait qui apparaît dans l'image.
        let (mut ws, _) = tungstenite::client::connect(format!("ws://127.0.0.1:{port}/ws")).unwrap();
        let hello = ws.read().unwrap();
        assert!(hello.to_text().unwrap().contains("hello"));
        ws.send(Message::text(r##"{"t":"down","id":1,"x":0.5,"y":0.5,"color":"#ffffff","w":0.05}"##)).unwrap();
        ws.send(Message::text(r#"{"t":"up","id":1}"#)).unwrap();
        let t0 = Instant::now();
        loop {
            let (_, slot) = sketch.source();
            let on = slot.lock().unwrap().as_ref().is_some_and(|f| {
                let i = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
                f.data[i] > 200
            });
            if on && sketch.strokes() == 1 {
                break;
            }
            assert!(t0.elapsed() < Duration::from_secs(3), "le trait n'est pas arrivé");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(sketch.clients(), 1);
        // Un aperçu JPEG revient à la page.
        let t0 = Instant::now();
        loop {
            if let Ok(Message::Binary(b)) = ws.read() {
                assert_eq!(&b[..2], &[0xFF, 0xD8]);
                break;
            }
            assert!(t0.elapsed() < Duration::from_secs(3));
        }
    }
}
