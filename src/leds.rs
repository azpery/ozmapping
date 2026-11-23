//! LEDs pilotées par des contrôleurs WLED (ESP32).
//!
//! On modélise l'installation en 3D (colonnes, bandes, anneaux, matrices, spirales posés dans
//! la salle), on la teste dans le simulateur, puis on envoie les couleurs aux WLED en DDP
//! (UDP, port 4048). Les LEDs reprennent l'image de la sortie : chaque LED, vue de face, de
//! dessus ou de côté, prend la couleur du point de l'image où elle tombe. Des mires de test
//! (couleur unie, chenillard, balayage, identification) servent à vérifier le câblage.
//!
//! La répartition des LEDs suit WLED : pour chaque contrôleur, sortie par sortie, les
//! éléments branchés sur une même sortie se suivent dans l'ordre de la liste.
//! « Envoyer la configuration » écrit ces sorties dans WLED (pins, longueurs, ordre des
//! couleurs), redémarre le contrôleur et crée un segment par élément.

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Port DDP de WLED.
pub const DDP_PORT: u16 = 4048;
/// Données RGB par paquet DDP (480 LEDs).
const DDP_CHUNK: usize = 1440;
/// LEDs par élément au plus.
pub const MAX_COUNT: u32 = 2000;
/// Ordres des couleurs, dans l'ordre des codes WLED (0 = GRB…).
pub const ORDERS: [&str; 6] = ["GRB", "RGB", "BRG", "RBG", "BGR", "GBR"];

/// Modèles de contrôleurs proposés : nom, pins des sorties (None : à lire sur le contrôleur).
pub const MODELS: [(&str, &[Option<u8>]); 4] = [
    (
        "ESP32 DevKitC (8 sorties)",
        &[Some(16), Some(17), Some(18), Some(19), Some(21), Some(22), Some(23), Some(27)],
    ),
    ("ESP32 (1 sortie, pin 16)", &[Some(16)]),
    ("Gledopto 4 sorties", &[None, None, None, None]),
    ("Gledopto 6 sorties", &[None, None, None, None, None, None]),
];

// ---------------------------------------------------------------- modèle (enregistré dans le projet)

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LedSetup {
    /// Salle : largeur (X, centrée sur 0) et profondeur (Z, de 0 vers le fond), en mètres.
    pub room: [f32; 2],
    pub controllers: Vec<Controller>,
    pub fixtures: Vec<Fixture>,
    /// Décor de la scène : écrans et surfaces mappées, volumes.
    pub surfaces: Vec<Surface>,
    /// Écran (id) sur lequel l'image des LEDs est calée : les LEDs prolongent son image.
    /// Aucun : l'image couvre l'installation des LEDs, vue selon `projection`.
    pub image_on: Option<u32>,
    /// Groupes d'objets de la scène (déplacés, tournés, redimensionnés ensemble).
    pub sets: Vec<LedGroup>,
    /// Groupes de bandeaux, dans l'ordre où les scénarios les animent.
    pub groups: Vec<LedGroup>,
    /// Scénario joué quand `source` vaut `Source::Scenario`.
    pub scenario: ScenarioSettings,
    /// Ce qu'affichent les LEDs.
    pub source: Source,
    pub test_color: [u8; 3],
    /// Luminosité générale (0-1).
    pub brightness: f32,
    /// Direction selon laquelle l'image de la sortie est plaquée sur les LEDs.
    pub projection: Projection,
    /// L'image couvre toute l'installation, quitte à être déformée (sinon : proportions gardées).
    pub stretch: bool,
    /// Images par seconde envoyées aux WLED.
    pub fps: u32,
}

impl Default for LedSetup {
    fn default() -> Self {
        Self {
            room: [6.0, 9.0],
            controllers: vec![Controller::new(1, "ESP32 principal", 0)],
            fixtures: Vec::new(),
            surfaces: Vec::new(),
            image_on: None,
            sets: Vec::new(),
            groups: Vec::new(),
            scenario: ScenarioSettings::default(),
            source: Source::Video,
            test_color: [255, 120, 0],
            brightness: 1.0,
            projection: Projection::Front,
            stretch: false,
            fps: 40,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Controller {
    pub id: u32,
    pub name: String,
    /// Adresse IP ou nom (« wled-salon.local »).
    pub host: String,
    /// Pin de chaque sortie (None : inconnu, à lire sur le contrôleur).
    pub pins: Vec<Option<u8>>,
    /// Ordre des couleurs des bandeaux (index dans `ORDERS`).
    pub order: usize,
    /// Reçoit les couleurs en direct.
    pub enabled: bool,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new(1, "WLED", 0)
    }
}

impl Controller {
    pub fn new(id: u32, name: &str, model: usize) -> Self {
        Self {
            id,
            name: name.into(),
            host: String::new(),
            pins: MODELS[model.min(MODELS.len() - 1)].1.to_vec(),
            order: 1,
            enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixtureKind {
    /// Colonne verticale posée au sol (hauteur = `size[0]`).
    #[default]
    Column,
    /// Bande droite (longueur = `size[0]`), le long de l'axe X local.
    Strip,
    /// Anneau (rayon = `size[0]`), à plat face à la salle.
    Ring,
    /// Matrice (largeur × hauteur = `size`, `cols` × `rows` LEDs).
    Matrix,
    /// Spirale autour d'un axe vertical (rayon = `size[0]`, hauteur = `size[1]`, `turns` tours).
    Spiral,
    /// Tracé libre dessiné point par point (`points`), LEDs réparties régulièrement dessus.
    Path,
}

impl FixtureKind {
    pub const ALL: [FixtureKind; 6] = [
        FixtureKind::Column,
        FixtureKind::Strip,
        FixtureKind::Ring,
        FixtureKind::Matrix,
        FixtureKind::Spiral,
        FixtureKind::Path,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FixtureKind::Column => "Colonne",
            FixtureKind::Strip => "Bande",
            FixtureKind::Ring => "Anneau",
            FixtureKind::Matrix => "Matrice",
            FixtureKind::Spiral => "Spirale",
            FixtureKind::Path => "Tracé libre",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fixture {
    pub id: u32,
    pub name: String,
    pub kind: FixtureKind,
    /// Position (m) : pied de la colonne, début de la bande, centre de l'anneau, de la matrice,
    /// base de la spirale.
    pub pos: [f32; 3],
    /// Orientation en degrés : rotation autour de Y (cap), puis X (inclinaison), puis Z (roulis).
    pub rot: [f32; 3],
    /// Nombre de LEDs (sauf matrice : `cols` × `rows`).
    pub count: u32,
    pub size: [f32; 2],
    pub cols: u32,
    pub rows: u32,
    /// Matrice câblée en zigzag (une ligne sur deux à l'envers).
    pub serpentine: bool,
    pub turns: f32,
    /// Points du tracé libre, relatifs à `pos` (avant rotation).
    pub points: Vec<[f32; 3]>,
    /// Le tracé revient à son premier point.
    pub closed: bool,
    /// Contrôleur (id) et sortie (index) où l'élément est branché.
    pub controller: u32,
    pub output: usize,
    /// Câblé depuis l'autre bout.
    pub reverse: bool,
    /// Groupe de bandeaux (id, 0 : aucun) pour les scénarios.
    pub group: u32,
    /// Groupe d'objets de la scène (id, 0 : aucun).
    pub set: u32,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            id: 1,
            name: "Colonne 1".into(),
            kind: FixtureKind::Column,
            pos: [0.0, 0.0, 2.0],
            rot: [0.0; 3],
            count: 30,
            size: [1.5, 1.0],
            cols: 16,
            rows: 16,
            serpentine: true,
            turns: 4.0,
            points: Vec::new(),
            closed: false,
            controller: 1,
            output: 0,
            reverse: false,
            group: 0,
            set: 0,
        }
    }
}

impl Fixture {
    /// Nouvel élément d'un type, avec des dimensions raisonnables.
    pub fn new(kind: FixtureKind, id: u32, name: String) -> Self {
        let base = Self { id, name, kind, ..Self::default() };
        match kind {
            FixtureKind::Column => base,
            FixtureKind::Strip => Self { pos: [-1.0, 1.0, 2.0], size: [2.0, 1.0], count: 60, ..base },
            FixtureKind::Ring => Self { pos: [0.0, 1.5, 2.0], size: [0.5, 1.0], count: 48, ..base },
            FixtureKind::Matrix => Self { pos: [0.0, 1.5, 2.0], size: [1.0, 1.0], ..base },
            FixtureKind::Spiral => Self { size: [0.4, 2.0], count: 150, ..base },
            FixtureKind::Path => Self { count: 60, ..base },
        }
    }

    pub fn led_count(&self) -> usize {
        match self.kind {
            FixtureKind::Matrix => (self.cols.max(1) * self.rows.max(1)).min(MAX_COUNT * 8) as usize,
            _ => self.count.clamp(1, MAX_COUNT) as usize,
        }
    }

    /// Positions des LEDs dans la salle, dans l'ordre du câblage.
    pub fn positions(&self) -> Vec<[f32; 3]> {
        let n = self.led_count();
        let step = |i: usize| if n > 1 { i as f32 / (n - 1) as f32 } else { 0.5 };
        let mut local: Vec<[f32; 3]> = match self.kind {
            FixtureKind::Column => (0..n).map(|i| [0.0, (i as f32 + 0.5) / n as f32 * self.size[0], 0.0]).collect(),
            FixtureKind::Strip => (0..n).map(|i| [step(i) * self.size[0], 0.0, 0.0]).collect(),
            FixtureKind::Ring => (0..n)
                .map(|i| {
                    let a = i as f32 / n as f32 * std::f32::consts::TAU;
                    [a.sin() * self.size[0], a.cos() * self.size[0], 0.0]
                })
                .collect(),
            FixtureKind::Matrix => {
                let (cols, rows) = (self.cols.max(1) as usize, self.rows.max(1) as usize);
                let mut v = Vec::with_capacity(cols * rows);
                // Départ en haut à gauche, ligne par ligne.
                for r in 0..rows {
                    for c in 0..cols {
                        let c = if self.serpentine && r % 2 == 1 { cols - 1 - c } else { c };
                        let x = if cols > 1 { c as f32 / (cols - 1) as f32 - 0.5 } else { 0.0 };
                        let y = if rows > 1 { 0.5 - r as f32 / (rows - 1) as f32 } else { 0.0 };
                        v.push([x * self.size[0], y * self.size[1], 0.0]);
                    }
                }
                v
            }
            FixtureKind::Spiral => (0..n)
                .map(|i| {
                    let t = step(i);
                    let a = t * self.turns * std::f32::consts::TAU;
                    [a.sin() * self.size[0], t * self.size[1], a.cos() * self.size[0]]
                })
                .collect(),
            FixtureKind::Path => {
                let pts = self.path();
                let total = path_length(&pts);
                if pts.len() < 2 || total < 1e-4 {
                    vec![pts.first().copied().unwrap_or_default(); n]
                } else {
                    // Boucle fermée : pas de LED en double sur le point de départ.
                    let span = if self.closed { n as f32 } else { (n.max(2) - 1) as f32 };
                    (0..n).map(|i| point_at(&pts, total * if n > 1 { i as f32 / span } else { 0.5 })).collect()
                }
            }
        };
        if self.reverse {
            local.reverse();
        }
        let m = rotation(self.rot);
        local
            .into_iter()
            .map(|p| {
                let q = mat_mul(&m, p);
                [q[0] + self.pos[0], q[1] + self.pos[1], q[2] + self.pos[2]]
            })
            .collect()
    }
}

impl Fixture {
    /// Points du tracé libre (le premier répété à la fin si la boucle est fermée).
    pub fn path(&self) -> Vec<[f32; 3]> {
        let mut pts = self.points.clone();
        if self.closed && pts.len() > 2 {
            pts.push(pts[0]);
        }
        pts
    }

    /// Longueur du tracé libre (m).
    pub fn path_length(&self) -> f32 {
        path_length(&self.path())
    }

    /// Tracé libre à partir de points de la salle : la position devient le premier point.
    pub fn set_path(&mut self, world: &[[f32; 3]], closed: bool) {
        let origin = world.first().copied().unwrap_or_default();
        self.pos = origin;
        self.rot = [0.0; 3];
        self.points = world.iter().map(|p| [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]]).collect();
        self.closed = closed;
    }
}

fn path_length(pts: &[[f32; 3]]) -> f32 {
    pts.windows(2).map(|w| dist(w[0], w[1])).sum()
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

type Block = ([f32; 3], [f32; 3], [u8; 3], bool);

/// Silhouette d'un DJ (1,80 m) debout en `z`, tourné vers l'avant, les mains sur les platines ;
/// il hoche la tête et plie un peu les genoux sur les temps.
fn dj_blocks(z: f32, beat: f32) -> Vec<Block> {
    let (cloth, pants, skin) = ([30, 30, 36], [22, 24, 40], [196, 150, 120]);
    let bob = 0.03 * (beat * std::f32::consts::PI).sin().abs();
    let nod = 0.04 * (beat * std::f32::consts::TAU).sin();
    let y = -bob;
    vec![
        ([-0.2, 0.0, z - 0.09], [-0.04, 0.88 + y, z + 0.09], pants, false),
        ([0.04, 0.0, z - 0.09], [0.2, 0.88 + y, z + 0.09], pants, false),
        ([-0.23, 0.88 + y, z - 0.12], [0.23, 1.5 + y, z + 0.12], cloth, false),
        ([-0.08, 1.5 + y, z - 0.06], [0.08, 1.56 + y, z + 0.06], skin, false),
        ([-0.11, 1.56 + y, z - 0.12 + nod], [0.11, 1.8 + y, z + 0.11 + nod], skin, false),
        // Bras tendus vers la cabine.
        ([-0.33, 1.2 + y, z - 0.48], [-0.23, 1.3 + y, z + 0.05], cloth, false),
        ([0.23, 1.2 + y, z - 0.48], [0.33, 1.3 + y, z + 0.05], cloth, false),
        // Casque.
        ([-0.13, 1.66 + y, z - 0.04 + nod], [0.13, 1.83 + y, z + 0.04 + nod], [15, 15, 18], false),
    ]
}

const CLOTHES: [[u8; 3]; 8] = [
    [200, 40, 60],
    [40, 90, 200],
    [240, 200, 40],
    [30, 30, 34],
    [235, 235, 235],
    [60, 170, 90],
    [150, 60, 190],
    [240, 120, 30],
];
const SKINS: [[u8; 3]; 4] = [[236, 196, 164], [198, 150, 118], [150, 104, 74], [96, 64, 44]];

/// Nombre pseudo-aléatoire stable (0-1) : la foule garde la même allure d'une image à l'autre.
fn hash01(a: u32, b: u32, c: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B) ^ c.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    (h & 0xFF_FFFF) as f32 / 0x100_0000 as f32
}

/// Une personne du public, tournée vers le fond de la salle (vers la scène).
struct Dancer {
    x: f32,
    z: f32,
    scale: f32,
    /// Décalage dans le temps (0-1 temps) et énergie (0-1).
    phase: f32,
    energy: f32,
    shirt: [u8; 3],
    skin: [u8; 3],
    /// Danse les bras en l'air (sinon les coudes pliés).
    arms_up: bool,
}

impl Dancer {
    fn blocks(&self, beat: f32) -> Vec<Block> {
        use std::f32::consts::PI;
        let t = beat + self.phase * 0.35;
        let e = self.energy;
        // Rebond sur chaque temps, balancement sur deux temps.
        let jump = 0.09 * e * (t * PI).sin().abs();
        let sway = 0.06 * e * (t * PI * 0.5).sin();
        let s = self.scale;
        let (x, z) = (self.x + sway, self.z);
        let pants = [40, 44, 60];
        let y0 = jump;
        let p = |lo: [f32; 3], hi: [f32; 3], color: [u8; 3]| -> Block {
            (
                [x + lo[0] * s, y0 + lo[1] * s, z + lo[2] * s],
                [x + hi[0] * s, y0 + hi[1] * s, z + hi[2] * s],
                color,
                false,
            )
        };
        let mut b = vec![
            // Jambes (la première touche toujours le sol).
            ([x - 0.17 * s, 0.0, z - 0.08 * s], [x - 0.03 * s, y0 + 0.85 * s, z + 0.08 * s], pants, false),
            p([0.03, 0.0, -0.08], [0.17, 0.85, 0.08], pants),
            p([-0.21, 0.85, -0.11], [0.21, 1.45, 0.11], self.shirt),
            p([-0.07, 1.45, -0.06], [0.07, 1.51, 0.06], self.skin),
            p([-0.1, 1.51, -0.1], [0.1, 1.73, 0.1], self.skin),
        ];
        // Bras : levés en rythme, ou coudes pliés qui battent la mesure.
        let lift = (0.5 + 0.5 * (t * PI).cos()) * e;
        for side in [-1.0f32, 1.0] {
            let (x0, x1) = if side < 0.0 { (-0.31, -0.21) } else { (0.21, 0.31) };
            if self.arms_up {
                let top = 1.45 + 0.15 + 0.45 * lift;
                b.push(p([x0, 1.2, -0.05], [x1, top, 0.05], self.skin));
            } else {
                let fore = 1.05 + 0.25 * lift;
                b.push(p([x0, 1.05, -0.05], [x1, 1.42, 0.05], self.shirt));
                b.push(p([x0, fore, 0.05], [x1, fore + 0.09, 0.38], self.skin));
            }
        }
        b
    }
}

/// Point situé à la distance `d` le long du tracé.
fn point_at(pts: &[[f32; 3]], mut d: f32) -> [f32; 3] {
    for w in pts.windows(2) {
        let l = dist(w[0], w[1]);
        if d <= l && l > 0.0 {
            let t = d / l;
            return [0, 1, 2].map(|k| w[0][k] + (w[1][k] - w[0][k]) * t);
        }
        d -= l;
    }
    *pts.last().unwrap()
}

/// Déplace les éléments `items` (index, dans n'importe quel ordre) d'un bloc, pour qu'ils
/// viennent juste avant l'élément qui était au rang `at` (`at` = longueur : à la fin).
/// Renvoie le rang du premier élément déplacé.
pub fn move_block<T>(v: &mut Vec<T>, items: &[usize], at: usize) -> usize {
    let mut items: Vec<usize> = items.iter().copied().filter(|&i| i < v.len()).collect();
    items.sort_unstable();
    items.dedup();
    let before = items.iter().filter(|&&i| i < at).count();
    let mut block = Vec::with_capacity(items.len());
    for &i in items.iter().rev() {
        block.push(v.remove(i));
    }
    block.reverse();
    let target = at.min(v.len() + before) - before;
    let target = target.min(v.len());
    for (k, item) in block.into_iter().enumerate() {
        v.insert(target + k, item);
    }
    target
}

/// Matrice de rotation (degrés) : Y, puis X, puis Z.
fn rotation(deg: [f32; 3]) -> [[f32; 3]; 3] {
    let [y, x, z] = deg.map(f32::to_radians);
    let (sy, cy, sx, cx, sz, cz) = (y.sin(), y.cos(), x.sin(), x.cos(), z.sin(), z.cos());
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mat_mat(&mat_mat(&ry, &rx), &rz)
}

fn mat_mat(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn mat_mul(m: &[[f32; 3]; 3], p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2])
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurfaceKind {
    /// Surface plane (écran, toile, élément mappé) qui affiche l'image de la sortie.
    #[default]
    Screen,
    /// Volume (boîte, mur), avec l'image sur sa face avant si on le souhaite.
    Box,
    /// Scène (estrade) : largeur, hauteur du plateau, profondeur.
    Stage,
    /// Cabine DJ : façade (qui peut afficher l'image), plateau, platines, et le DJ si on veut.
    Booth,
    /// Public qui danse : zone (largeur × profondeur), `people` personnes de hauteur `size[1]`.
    Crowd,
    /// Panneaux ondulés superposés (comme « les chips ») : largeur totale × hauteur maximale.
    Panels,
}

impl SurfaceKind {
    pub const ALL: [SurfaceKind; 6] = [
        SurfaceKind::Screen,
        SurfaceKind::Box,
        SurfaceKind::Stage,
        SurfaceKind::Booth,
        SurfaceKind::Crowd,
        SurfaceKind::Panels,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SurfaceKind::Screen => "Écran",
            SurfaceKind::Box => "Volume",
            SurfaceKind::Stage => "Scène",
            SurfaceKind::Booth => "Cabine DJ",
            SurfaceKind::Crowd => "Public",
            SurfaceKind::Panels => "Panneaux superposés",
        }
    }
}

/// Face d'un objet du décor, prête à dessiner.
pub struct Face3 {
    /// Coins : haut gauche, haut droit, bas droit, bas gauche vus de l'extérieur.
    pub quad: [[f32; 3]; 4],
    pub color: [u8; 3],
    /// Affiche l'image de la sortie.
    pub output: bool,
    /// Point de l'image de sortie sur chaque coin.
    pub uv: [[f32; 2]; 4],
    /// Arêtes à dessiner : haut, droite, bas, gauche (bits 0 à 3).
    pub edges: u8,
}

/// Réglages des panneaux ondulés superposés (« les chips »).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Panels {
    pub count: u32,
    /// Recouvrement d'un panneau sur son voisin (part de sa largeur).
    pub overlap: f32,
    /// Hauteur des panneaux du centre, rapportée à celle des bords.
    pub low: f32,
    /// Écart entre deux couches (m) : la place des LEDs.
    pub gap: f32,
    /// Amplitude de l'ondulation des bords (m).
    pub wave: f32,
    /// Couleurs du centre et des bords (dégradé).
    pub inner: [u8; 3],
    pub outer: [u8; 3],
    /// Soleil au centre : diamètre (0 : aucun) et couleur.
    pub sun: f32,
    pub sun_color: [u8; 3],
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            count: 15,
            overlap: 0.45,
            low: 0.5,
            gap: 0.05,
            wave: 0.035,
            inner: [244, 186, 40],
            outer: [214, 36, 48],
            sun: 1.3,
            sun_color: [242, 200, 30],
        }
    }
}

/// Un panneau : rangées (hauteur, bord gauche, bord droit), profondeur, couleur, côté (-1, 0, 1)
/// et rang depuis le centre (0-1).
struct Panel {
    rows: Vec<(f32, f32, f32)>,
    z: f32,
    color: [u8; 3],
    side: f32,
    rank: f32,
}


/// Objet du décor : écran ou volume, posé dans la salle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Surface {
    pub id: u32,
    pub name: String,
    pub kind: SurfaceKind,
    /// Milieu du bas de l'objet (m).
    pub pos: [f32; 3],
    /// Orientation en degrés (comme les éléments LED).
    pub rot: [f32; 3],
    /// Largeur, hauteur, profondeur (m ; profondeur ignorée pour un écran).
    pub size: [f32; 3],
    pub color: [u8; 3],
    /// La face avant affiche l'image de la sortie.
    pub show_output: bool,
    /// Partie de la sortie affichée : gauche, haut, droite, bas (0-1).
    pub crop: [f32; 4],
    /// Cabine DJ : un DJ se tient derrière.
    pub dj: bool,
    /// Public : nombre de personnes et énergie de la danse (0 : immobiles, 1 : à fond).
    pub people: u32,
    pub dance: f32,
    /// Panneaux superposés.
    pub panels: Panels,
    /// Groupe d'objets de la scène (id, 0 : aucun).
    pub set: u32,
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            id: 1,
            name: "Écran 1".into(),
            kind: SurfaceKind::Screen,
            pos: [0.0, 0.8, 8.0],
            rot: [0.0; 3],
            size: [3.2, 1.8, 0.4],
            color: [200, 200, 205],
            show_output: true,
            crop: [0.0, 0.0, 1.0, 1.0],
            dj: true,
            people: 24,
            dance: 0.7,
            panels: Panels::default(),
            set: 0,
        }
    }
}

impl Surface {
    /// Coins de la face avant (vers l'avant de la salle) : haut gauche, haut droit, bas droit, bas gauche.
    pub fn front(&self) -> [[f32; 3]; 4] {
        let [w, h, _] = self.size;
        let z = self.front_z();
        [[-w / 2.0, h, z], [w / 2.0, h, z], [w / 2.0, 0.0, z], [-w / 2.0, 0.0, z]].map(|p| self.world(p))
    }

    fn front_z(&self) -> f32 {
        if self.kind == SurfaceKind::Screen { 0.0 } else { -self.size[2] / 2.0 }
    }

    /// Hauteur du dessus (pour poser un objet sur une scène).
    pub fn top(&self) -> f32 {
        self.pos[1] + self.size[1]
    }

    /// Le point (x, z) de la salle est au-dessus de l'objet (sans tenir compte de sa rotation
    /// autour de X et Z).
    pub fn covers(&self, x: f32, z: f32) -> bool {
        let a = -self.rot[0].to_radians();
        let (dx, dz) = (x - self.pos[0], z - self.pos[2]);
        let (lx, lz) = (dx * a.cos() + dz * a.sin(), -dx * a.sin() + dz * a.cos());
        lx.abs() <= self.size[0] / 2.0 && lz.abs() <= self.size[2] / 2.0
    }

    /// Faces visibles de l'objet (pavés qui le composent).
    /// `beat` : numéro du temps musical (fractionnaire), pour faire danser les personnages.
    pub fn faces(&self, beat: f32) -> Vec<Face3> {
        let [u0, v0, u1, v1] = self.crop;
        let full = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
        if self.kind == SurfaceKind::Screen {
            return vec![Face3 { quad: self.front(), color: self.color, output: self.show_output, uv: full, edges: 0b1111 }];
        }
        if self.kind == SurfaceKind::Panels {
            return self.panel_faces();
        }
        let mut faces = Vec::new();
        for (lo, hi, color, output) in self.blocks(beat) {
            let c = |x: f32, y: f32, z: f32| self.world([x, y, z]);
            let ([x0, y0, z0], [x1, y1, z1]) = (lo, hi);
            let quads = [
                [c(x0, y1, z0), c(x1, y1, z0), c(x1, y0, z0), c(x0, y0, z0)],
                [c(x1, y1, z1), c(x0, y1, z1), c(x0, y0, z1), c(x1, y0, z1)],
                [c(x0, y1, z1), c(x0, y1, z0), c(x0, y0, z0), c(x0, y0, z1)],
                [c(x1, y1, z0), c(x1, y1, z1), c(x1, y0, z1), c(x1, y0, z0)],
                [c(x0, y1, z1), c(x1, y1, z1), c(x1, y1, z0), c(x0, y1, z0)],
                [c(x0, y0, z0), c(x1, y0, z0), c(x1, y0, z1), c(x0, y0, z1)],
            ];
            for (k, quad) in quads.into_iter().enumerate() {
                faces.push(Face3 { quad, color, output: output && k == 0, uv: full, edges: 0b1111 });
            }
        }
        faces
    }

    /// Panneaux en coordonnées locales : le centre devant, les bords derrière, en V.
    fn panel_shapes(&self) -> Vec<Panel> {
        let p = &self.panels;
        let n = p.count.clamp(1, 60) as usize;
        let [w, h, _] = self.size;
        let step = w / n as f32;
        let pw = step * (1.0 + p.overlap.clamp(0.0, 2.0));
        let c = (n as f32 - 1.0) / 2.0;
        let lerp = |a: u8, b: u8, t: f32| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        (0..n)
            .map(|i| {
                let side = (i as f32 - c).signum() * if (i as f32 - c).abs() < 0.01 { 0.0 } else { 1.0 };
                let rank = if c > 0.0 { (i as f32 - c).abs() / c } else { 0.0 };
                let height = h * (p.low + (1.0 - p.low) * rank.powf(1.5));
                let cx = -w / 2.0 + step * (i as f32 + 0.5);
                let phase = i as f32 * 1.7;
                // Sommet arrondi, qui part en flamme vers l'extérieur sur les panneaux des bords.
                let cap = (pw * 0.6).min(height * 0.5);
                let rows_n = ((height / 0.08).ceil() as usize).max(4);
                let rows = (0..=rows_n)
                    .map(|k| {
                        let y = height * k as f32 / rows_n as f32;
                        let wave = p.wave * (y / 0.55 * std::f32::consts::TAU + phase).sin();
                        let (f, shift) = if y > height - cap {
                            let t = (y - (height - cap)) / cap;
                            ((1.0 - t * t).max(0.0).sqrt(), side * pw * 0.35 * rank * rank * t)
                        } else {
                            (1.0, 0.0)
                        };
                        (y, cx - pw / 2.0 * f + wave + shift, cx + pw / 2.0 * f + wave + shift)
                    })
                    .collect();
                Panel {
                    rows,
                    z: (i as f32 - c).abs().round() * p.gap,
                    // Le jaune du centre tient plus longtemps avant de virer au rouge.
                    color: [0, 1, 2].map(|k| lerp(p.inner[k], p.outer[k], rank.powf(1.4))),
                    side,
                    rank,
                }
            })
            .collect()
    }

    fn panel_faces(&self) -> Vec<Face3> {
        let [w, h, _] = self.size;
        let [u0, v0, u1, v1] = self.crop;
        let uv = |x: f32, y: f32| {
            let (s, t) = ((x / w.max(0.01) + 0.5).clamp(0.0, 1.0), (1.0 - y / h.max(0.01)).clamp(0.0, 1.0));
            [u0 + s * (u1 - u0), v0 + t * (v1 - v0)]
        };
        let mut faces = Vec::new();
        for panel in self.panel_shapes() {
            let last = panel.rows.len() - 2;
            for (k, r) in panel.rows.windows(2).enumerate() {
                let ((ya, la, ra), (yb, lb, rb)) = (r[0], r[1]);
                let local = [[lb, yb, panel.z], [rb, yb, panel.z], [ra, ya, panel.z], [la, ya, panel.z]];
                let mut edges = 0b1010;
                if k == last {
                    edges |= 0b0001;
                }
                if k == 0 {
                    edges |= 0b0100;
                }
                faces.push(Face3 {
                    quad: local.map(|q| self.world(q)),
                    color: panel.color,
                    output: self.show_output,
                    uv: local.map(|q| uv(q[0], q[1])),
                    edges,
                });
            }
        }
        // Soleil à rayons, posé devant le panneau du centre.
        let p = &self.panels;
        if p.sun > 0.01 {
            let r = p.sun / 2.0;
            let center_h = h * p.low;
            let (cx, cy, z) = (0.0, (center_h - r * 0.75).max(r), -p.gap);
            let n = 32;
            let pt = |k: usize| {
                let a = k as f32 / n as f32 * std::f32::consts::TAU;
                let rr = if k % 2 == 0 { r } else { r * 0.8 };
                [cx + a.sin() * rr, cy + a.cos() * rr, z]
            };
            for k in 0..n {
                let local = [pt(k), pt(k + 1), [cx, cy, z], [cx, cy, z]];
                faces.push(Face3 {
                    quad: local.map(|q| self.world(q)),
                    color: p.sun_color,
                    output: false,
                    uv: local.map(|q| uv(q[0], q[1])),
                    edges: 0b0001,
                });
            }
        }
        faces
    }

    /// Tracés des bandes LED entre les panneaux : derrière le bord extérieur de chaque panneau
    /// (deux bords pour celui du centre), au milieu de l'écart avec la couche suivante.
    /// Chaque tracé vient avec son côté (-1 : gauche, 1 : droite) ; de gauche à droite.
    pub fn panel_led_paths(&self) -> Vec<(f32, Vec<[f32; 3]>)> {
        let gap = self.panels.gap;
        let mut paths = Vec::new();
        for panel in self.panel_shapes() {
            if panel.rank >= 0.999 {
                continue;
            }
            let sides: &[f32] = if panel.side == 0.0 { &[-1.0, 1.0] } else { std::slice::from_ref(&panel.side) };
            for &side in sides {
                let pts: Vec<[f32; 3]> = panel
                    .rows
                    .iter()
                    // On s'arrête avant que le sommet ne se referme.
                    .take_while(|r| (r.2 - r.1) > 0.4 * (panel.rows[0].2 - panel.rows[0].1))
                    .map(|&(y, l, r)| {
                        let x = if side > 0.0 { r - 0.03 } else { l + 0.03 };
                        self.world([x, y.max(0.03), panel.z + gap / 2.0])
                    })
                    .collect();
                if pts.len() >= 2 {
                    paths.push((side, pts));
                }
            }
        }
        paths
    }

    /// Pavés (coin bas, coin haut, couleur, façade qui affiche l'image) en coordonnées locales.
    fn blocks(&self, beat: f32) -> Vec<([f32; 3], [f32; 3], [u8; 3], bool)> {
        let [w, h, d] = self.size;
        let (hw, hd) = (w / 2.0, d / 2.0);
        match self.kind {
            SurfaceKind::Screen | SurfaceKind::Panels => Vec::new(),
            SurfaceKind::Box | SurfaceKind::Stage => vec![([-hw, 0.0, -hd], [hw, h, hd], self.color, self.show_output)],
            SurfaceKind::Booth => {
                let t = 0.04;
                let dark = [28, 28, 32];
                let mut b = vec![
                    // Façade, côtés, plateau.
                    ([-hw, 0.0, -hd], [hw, h, -hd + t], self.color, self.show_output),
                    ([-hw, 0.0, -hd + t], [-hw + t, h, hd], self.color, false),
                    ([hw - t, 0.0, -hd + t], [hw, h, hd], self.color, false),
                    ([-hw - 0.03, h, -hd - 0.03], [hw + 0.03, h + t, hd], [45, 45, 50], false),
                ];
                // Deux platines et une table de mixage.
                let top = h + t;
                for x in [-w * 0.28, w * 0.28] {
                    b.push(([x - 0.17, top, -0.12], [x + 0.17, top + 0.05, 0.18], dark, false));
                }
                b.push(([-0.13, top, -0.12], [0.13, top + 0.08, 0.2], [55, 55, 62], false));
                if self.dj {
                    b.extend(dj_blocks(hd + 0.35, beat));
                }
                b
            }
            SurfaceKind::Crowd => {
                // Grille irrégulière : chacun sa place, sa taille, ses couleurs et son décalage.
                let n = self.people.clamp(1, 400) as usize;
                let cols = ((n as f32 * w / d.max(0.1)).sqrt().ceil() as usize).max(1);
                let rows = n.div_ceil(cols);
                let mut b = Vec::new();
                for k in 0..n {
                    let r = |salt: u32| hash01(self.id, k as u32, salt);
                    let (c, row) = (k % cols, k / cols);
                    let x = -hw + w * (c as f32 + 0.25 + 0.5 * r(1)) / cols as f32;
                    let z = -hd + d * (row as f32 + 0.25 + 0.5 * r(2)) / rows as f32;
                    let dancer = Dancer {
                        x,
                        z,
                        scale: h / 1.75 * (0.9 + 0.18 * r(3)),
                        phase: r(4),
                        energy: self.dance * (0.6 + 0.4 * r(5)),
                        shirt: CLOTHES[(r(6) * CLOTHES.len() as f32) as usize % CLOTHES.len()],
                        skin: SKINS[(r(7) * SKINS.len() as f32) as usize % SKINS.len()],
                        arms_up: r(8) < 0.45,
                    };
                    b.extend(dancer.blocks(beat));
                }
                b
            }
        }
    }

    fn world(&self, p: [f32; 3]) -> [f32; 3] {
        let q = mat_mul(&rotation(self.rot), p);
        [q[0] + self.pos[0], q[1] + self.pos[1], q[2] + self.pos[2]]
    }

    /// Point de l'image de sortie (0-1) qui tombe, de face, sur un point de la salle.
    pub fn image_uv(&self, p: [f32; 3]) -> [f32; 2] {
        let m = rotation(self.rot);
        let d = [p[0] - self.pos[0], p[1] - self.pos[1], p[2] - self.pos[2]];
        // Rotation inverse (transposée).
        let l = [0, 1, 2].map(|i| m[0][i] * d[0] + m[1][i] * d[1] + m[2][i] * d[2]);
        let s = l[0] / self.size[0].max(1e-3) + 0.5;
        let t = 1.0 - l[1] / self.size[1].max(1e-3);
        let [u0, v0, u1, v1] = self.crop;
        [(u0 + s * (u1 - u0)).clamp(0.0, 1.0), (v0 + t * (v1 - v0)).clamp(0.0, 1.0)]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// L'image de la sortie, plaquée sur les LEDs.
    #[default]
    Video,
    /// Scénario qui fait jouer les groupes de bandeaux les uns avec les autres.
    Scenario,
    Solid,
    Rainbow,
    /// Un point qui parcourt chaque élément du début à la fin (sens du câblage).
    Chase,
    /// Un plan lumineux qui traverse la salle.
    SweepX,
    SweepY,
    SweepZ,
    /// Chaque élément d'une couleur, sa première LED en blanc.
    Identify,
    Off,
}

impl Source {
    pub const ALL: [Source; 10] = [
        Source::Video,
        Source::Scenario,
        Source::Solid,
        Source::Rainbow,
        Source::Chase,
        Source::SweepX,
        Source::SweepY,
        Source::SweepZ,
        Source::Identify,
        Source::Off,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Source::Video => "Image de la sortie",
            Source::Scenario => "Scénario (les bandeaux jouent ensemble)",
            Source::Solid => "Couleur unie",
            Source::Rainbow => "Arc-en-ciel",
            Source::Chase => "Chenillard (sens du câblage)",
            Source::SweepX => "Balayage gauche → droite",
            Source::SweepY => "Balayage bas → haut",
            Source::SweepZ => "Balayage avant → fond",
            Source::Identify => "Identifier les éléments",
            Source::Off => "Noir",
        }
    }
}

/// Groupe de bandeaux : les éléments d'un groupe s'allument ensemble (ex. les deux chips
/// symétriques, gauche et droite).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LedGroup {
    pub id: u32,
    pub name: String,
}

impl Default for LedGroup {
    fn default() -> Self {
        Self { id: 1, name: "Groupe 1".into() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScenarioKind {
    /// Les groupes s'allument l'un après l'autre.
    #[default]
    Sequence,
    /// Double battement de tous les groupes, et de temps en temps une vague groupe par groupe.
    HeartBeat,
    /// La luminosité ondule, décalée d'un groupe à l'autre.
    Wave,
    /// Même vague, couleurs qui tournent.
    WaveColors,
    /// Arc-en-ciel qui défile le long des bandeaux.
    Rainbow,
    /// Points lumineux qui serpentent d'un bandeau à l'autre.
    Comets,
    /// Étincelles blanches au hasard.
    Sparkles,
    /// Vumètre arc-en-ciel depuis le milieu des bandeaux, au son.
    Sound,
}

impl ScenarioKind {
    pub const ALL: [ScenarioKind; 8] = [
        ScenarioKind::Sequence,
        ScenarioKind::HeartBeat,
        ScenarioKind::Wave,
        ScenarioKind::WaveColors,
        ScenarioKind::Rainbow,
        ScenarioKind::Comets,
        ScenarioKind::Sparkles,
        ScenarioKind::Sound,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ScenarioKind::Sequence => "Allumage (l'un après l'autre)",
            ScenarioKind::HeartBeat => "Battement de cœur",
            ScenarioKind::Wave => "Vague",
            ScenarioKind::WaveColors => "Vague de couleurs",
            ScenarioKind::Rainbow => "Arc-en-ciel",
            ScenarioKind::Comets => "Comètes (de bandeau en bandeau)",
            ScenarioKind::Sparkles => "Étincelles",
            ScenarioKind::Sound => "Vumètre (son)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScenarioSettings {
    pub kind: ScenarioKind,
    /// Vitesse (1 : normale).
    pub speed: f32,
    /// Calé sur le tempo de l'onglet Rythme (sinon : 120 BPM).
    pub sync: bool,
    /// Enchaîne les scénarios au hasard, avec un fondu, toutes les `chain_secs` secondes.
    pub chain: bool,
    pub chain_secs: f32,
    /// Comètes en même temps.
    pub comets: u32,
}

impl Default for ScenarioSettings {
    fn default() -> Self {
        Self { kind: ScenarioKind::Sequence, speed: 1.0, sync: true, chain: false, chain_secs: 30.0, comets: 4 }
    }
}

/// Couleurs des groupes, du centre vers l'extérieur (celles d'organic_tears_drops).
const GROUP_COLORS: [[u8; 3]; 26] = [
    [255, 255, 175],
    [255, 255, 102],
    [255, 204, 0],
    [255, 153, 0],
    [255, 102, 0],
    [255, 51, 0],
    [255, 0, 0],
    [255, 0, 51],
    [255, 0, 102],
    [255, 0, 153],
    [255, 0, 204],
    [255, 0, 255],
    [204, 0, 255],
    [153, 0, 255],
    [102, 0, 255],
    [51, 0, 255],
    [0, 0, 255],
    [0, 51, 255],
    [0, 102, 255],
    [0, 153, 255],
    [0, 204, 255],
    [0, 255, 255],
    [0, 255, 204],
    [0, 255, 153],
    [0, 255, 102],
    [0, 255, 51],
];

/// Musique au moment de l'image : numéro du temps (fractionnaire) et niveau du son (0-1).
#[derive(Clone, Copy, Default)]
pub struct Music {
    pub beat: f32,
    pub level: f32,
}

/// Couleur d'une LED dans un scénario : groupe `g` sur `n`, position `u` (0-1) le long de son
/// bandeau, numéro de LED `id` (pour le hasard), temps `t` (en temps musicaux), heure `secs`.
fn scenario_color(sc: &ScenarioSettings, kind: ScenarioKind, g: usize, n: usize, u: f32, id: u32, t: f32, secs: f32, level: f32) -> [f32; 3] {
    use std::f32::consts::TAU;
    let gf = g as f32;
    let nf = n.max(1) as f32;
    let pal = GROUP_COLORS[g % GROUP_COLORS.len()].map(f32::from);
    let scale = |c: [f32; 3], b: f32| c.map(|v| v * b.clamp(0.0, 1.0));
    // Éclat bref : montée rapide juste avant `c`, puis retombée.
    let pulse = |x: f32, c: f32, a: f32| {
        if x >= c {
            a * (-(x - c) * 5.0).exp()
        } else if x > c - 0.06 {
            a * (x - (c - 0.06)) / 0.06
        } else {
            0.0
        }
    };
    match kind {
        ScenarioKind::Sequence => {
            // Deux groupes par temps ; chacun s'éteint doucement derrière le suivant.
            let phase = (t * 2.0 - gf).rem_euclid(nf);
            let b = if phase < 1.0 { 1.0 } else { (1.0 - (phase - 1.0) / 2.0).max(0.0).powi(2) };
            scale(pal, b)
        }
        ScenarioKind::HeartBeat => {
            // « Boum-boum » sur deux temps ; un cycle sur quatre, la vague passe de groupe en groupe.
            let cycle = (t / 2.0).floor() as i64;
            let x = t.rem_euclid(2.0);
            let b = if cycle.rem_euclid(4) == 3 {
                pulse(x, gf / nf * 1.6, 1.0).max(0.05)
            } else {
                pulse(x, 0.0, 1.0).max(pulse(x, 0.45, 0.75)).max(0.08)
            };
            scale(pal, b)
        }
        ScenarioKind::Wave => {
            let b = (0.5 + 0.5 * ((t * 0.5 - gf / nf) * TAU).sin()).powf(1.5);
            scale(pal, b)
        }
        ScenarioKind::WaveColors => {
            let b = (0.5 + 0.5 * ((t * 0.5 - gf / nf) * TAU).sin()).powf(1.5);
            scale(hue(gf / nf * 0.4 + t * 0.05), b)
        }
        ScenarioKind::Rainbow => hue(u * 0.5 + gf * 0.07 - t * 0.25),
        ScenarioKind::Comets => {
            // Chaque comète parcourt les groupes dans l'ordre, un bandeau dans un sens, le suivant dans l'autre.
            let k_n = sc.comets.clamp(1, 32);
            let mut best = [0.0f32; 3];
            for k in 0..k_n {
                let pos = (t * 0.35 / nf * 4.0 + k as f32 / k_n as f32).rem_euclid(1.0) * nf;
                let cg = pos.floor() as usize;
                if cg != g {
                    continue;
                }
                let cu = if cg % 2 == 1 { 1.0 - pos.fract() } else { pos.fract() };
                let b = (-((u - cu) / 0.035).powi(2)).exp();
                let c = scale(hue(k as f32 / k_n as f32), b);
                for i in 0..3 {
                    best[i] = best[i].max(c[i]);
                }
            }
            best
        }
        ScenarioKind::Sparkles => {
            let frame = (secs * 20.0 * sc.speed.max(0.1)) as u32;
            if hash01(id, frame, 77) < 0.08 { [255.0; 3] } else { [0.0; 3] }
        }
        ScenarioKind::Sound => {
            // Sans son, le tempo donne le niveau.
            let lv = level.max((-(t.fract()) * 4.0).exp() * 0.8);
            let h = lv * (0.4 + 0.6 * (gf + 1.0) / nf);
            let d = (u - 0.5).abs() * 2.0;
            if d <= h { hue(d * 0.6 + secs * 0.1) } else { [0.0; 3] }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Projection {
    /// Vue de face : X → largeur de l'image, Y → hauteur.
    #[default]
    Front,
    /// Vue de dessus : X → largeur, Z → hauteur (le fond de la salle en haut).
    Top,
    /// Vue de côté : Z → largeur, Y → hauteur.
    Side,
}

impl Projection {
    pub const ALL: [Projection; 3] = [Projection::Front, Projection::Top, Projection::Side];

    pub fn name(self) -> &'static str {
        match self {
            Projection::Front => "De face",
            Projection::Top => "De dessus",
            Projection::Side => "De côté",
        }
    }

    /// Point de l'image (x vers la droite, y vers le haut) où tombe un point de la salle.
    fn flatten(self, p: [f32; 3]) -> [f32; 2] {
        match self {
            Projection::Front => [p[0], p[1]],
            Projection::Top => [p[0], p[2]],
            Projection::Side => [p[2], p[1]],
        }
    }
}

/// Une LED de l'installation, prête à être affichée et envoyée.
#[derive(Clone, Debug)]
pub struct Led {
    pub pos: [f32; 3],
    /// Point de l'image de sortie (0-1, y vers le bas) dont elle prend la couleur.
    pub uv: [f32; 2],
    /// Élément (index dans `fixtures`) et rang de la LED dans l'élément.
    pub fixture: usize,
    pub index: usize,
    /// Contrôleur (index dans `controllers`) et adresse de la LED chez lui.
    pub controller: Option<usize>,
    pub address: usize,
}

/// Sorties d'un contrôleur, au sens de WLED : (sortie, première LED, nombre de LEDs).
#[derive(Clone, Debug, Default)]
pub struct ControllerLayout {
    pub outputs: Vec<(usize, usize, usize)>,
    pub total: usize,
    /// Plage d'adresses de chaque élément branché : (élément, début, nombre).
    pub fixtures: Vec<(usize, usize, usize)>,
}

impl LedSetup {
    pub fn sanitize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        for c in &mut self.controllers {
            while !seen.insert(c.id) {
                c.id += 1;
            }
            c.order = c.order.min(ORDERS.len() - 1);
            if c.pins.is_empty() {
                c.pins.push(None);
            }
        }
        let mut seen = std::collections::HashSet::new();
        for f in &mut self.fixtures {
            while !seen.insert(f.id) {
                f.id += 1;
            }
            f.count = f.count.clamp(1, MAX_COUNT);
        }
        let mut seen = std::collections::HashSet::new();
        for s in &mut self.surfaces {
            while !seen.insert(s.id) {
                s.id += 1;
            }
            s.size = s.size.map(|v| v.max(0.01));
            s.crop = s.crop.map(|v| v.clamp(0.0, 1.0));
        }
        self.brightness = self.brightness.clamp(0.0, 1.0);
        self.fps = self.fps.clamp(5, 60);
    }

    pub fn controller_index(&self, id: u32) -> Option<usize> {
        self.controllers.iter().position(|c| c.id == id)
    }

    pub fn next_fixture_id(&self) -> u32 {
        self.fixtures.iter().map(|f| f.id).max().unwrap_or(0) + 1
    }

    pub fn next_set_id(&self) -> u32 {
        self.sets.iter().map(|g| g.id).max().unwrap_or(0) + 1
    }

    /// Membres d'un groupe d'objets : éléments LED et objets du décor (index).
    pub fn set_members(&self, id: u32) -> (Vec<usize>, Vec<usize>) {
        let f = (0..self.fixtures.len()).filter(|&i| id != 0 && self.fixtures[i].set == id).collect();
        let s = (0..self.surfaces.len()).filter(|&i| id != 0 && self.surfaces[i].set == id).collect();
        (f, s)
    }

    /// Pivot d'un groupe : milieu en largeur et profondeur, au niveau de son point le plus bas.
    pub fn set_pivot(&self, id: u32) -> [f32; 3] {
        let (fs, ss) = self.set_members(id);
        let pts: Vec<[f32; 3]> = fs.iter().map(|&i| self.fixtures[i].pos).chain(ss.iter().map(|&i| self.surfaces[i].pos)).collect();
        if pts.is_empty() {
            return [0.0; 3];
        }
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &pts {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        [(lo[0] + hi[0]) / 2.0, lo[1], (lo[2] + hi[2]) / 2.0]
    }

    /// Déplace tout le groupe.
    pub fn move_set(&mut self, id: u32, d: [f32; 3]) {
        let (fs, ss) = self.set_members(id);
        for i in fs {
            let p = &mut self.fixtures[i].pos;
            *p = [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
        }
        for i in ss {
            let p = &mut self.surfaces[i].pos;
            *p = [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
        }
    }

    /// Tourne tout le groupe autour de la verticale passant par son pivot.
    pub fn rotate_set(&mut self, id: u32, deg: f32) {
        let c = self.set_pivot(id);
        let a = deg.to_radians();
        let (sa, ca) = (a.sin(), a.cos());
        // Même sens que la rotation « cap » des objets (autour de Y).
        let turn = |p: &mut [f32; 3]| {
            let (dx, dz) = (p[0] - c[0], p[2] - c[2]);
            p[0] = c[0] + ca * dx + sa * dz;
            p[2] = c[2] - sa * dx + ca * dz;
        };
        let (fs, ss) = self.set_members(id);
        for i in fs {
            let f = &mut self.fixtures[i];
            turn(&mut f.pos);
            f.rot[0] = (f.rot[0] + deg).rem_euclid(360.0);
        }
        for i in ss {
            let s = &mut self.surfaces[i];
            turn(&mut s.pos);
            s.rot[0] = (s.rot[0] + deg).rem_euclid(360.0);
        }
    }

    /// Agrandit ou rétrécit tout le groupe (facteur `k`) autour de son pivot, le bas restant en place.
    /// Le nombre de LEDs ne change pas.
    pub fn scale_set(&mut self, id: u32, k: f32) {
        let k = k.clamp(0.05, 20.0);
        let c = self.set_pivot(id);
        let sc = |p: &mut [f32; 3]| {
            for i in 0..3 {
                p[i] = c[i] + (p[i] - c[i]) * k;
            }
        };
        let (fs, ss) = self.set_members(id);
        for i in fs {
            let f = &mut self.fixtures[i];
            sc(&mut f.pos);
            f.size = f.size.map(|v| v * k);
            for p in &mut f.points {
                *p = p.map(|v| v * k);
            }
        }
        for i in ss {
            let s = &mut self.surfaces[i];
            sc(&mut s.pos);
            s.size = s.size.map(|v| v * k);
            s.panels.gap *= k;
            s.panels.wave *= k;
            s.panels.sun *= k;
        }
    }

    /// Retire les groupes d'objets vides ou réduits à un seul membre.
    pub fn prune_sets(&mut self) {
        let keep: Vec<u32> = self
            .sets
            .iter()
            .map(|g| g.id)
            .filter(|&id| {
                let (f, s) = self.set_members(id);
                f.len() + s.len() > 1
            })
            .collect();
        self.sets.retain(|g| keep.contains(&g.id));
        for f in &mut self.fixtures {
            if !keep.contains(&f.set) {
                f.set = 0;
            }
        }
        for s in &mut self.surfaces {
            if !keep.contains(&s.set) {
                s.set = 0;
            }
        }
    }

    pub fn next_group_id(&self) -> u32 {
        self.groups.iter().map(|g| g.id).max().unwrap_or(0) + 1
    }

    /// Groupes automatiques : `center_out` = paires symétriques du centre vers les bords
    /// (comme les chips gauche et droite), sinon de gauche à droite, un élément par groupe.
    pub fn auto_groups(&mut self, center_out: bool) {
        let centers: Vec<f32> = self
            .fixtures
            .iter()
            .map(|f| {
                let p = f.positions();
                p.iter().map(|q| q[0]).sum::<f32>() / p.len().max(1) as f32
            })
            .collect();
        let mid = if centers.is_empty() {
            0.0
        } else {
            let (lo, hi) = centers.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
            (lo + hi) / 2.0
        };
        // Clé de tri de chaque élément ; des clés proches (10 cm) forment un même groupe.
        let key = |x: f32| if center_out { (x - mid).abs() } else { x };
        let mut order: Vec<usize> = (0..self.fixtures.len()).collect();
        order.sort_by(|&a, &b| key(centers[a]).total_cmp(&key(centers[b])));
        self.groups.clear();
        let mut last: Option<f32> = None;
        for i in order {
            let k = key(centers[i]);
            if !center_out || last.is_none_or(|l| (k - l).abs() > 0.1) {
                let id = self.next_group_id();
                self.groups.push(LedGroup { id, name: format!("Groupe {id}") });
            }
            last = Some(k);
            self.fixtures[i].group = self.groups.last().unwrap().id;
        }
    }

    pub fn next_surface_id(&self) -> u32 {
        self.surfaces.iter().map(|s| s.id).max().unwrap_or(0) + 1
    }

    pub fn next_controller_id(&self) -> u32 {
        self.controllers.iter().map(|c| c.id).max().unwrap_or(0) + 1
    }

    /// Répartition des LEDs d'un contrôleur (comme WLED : sortie après sortie).
    pub fn layout(&self, ctrl: usize) -> ControllerLayout {
        let Some(c) = self.controllers.get(ctrl) else {
            return ControllerLayout::default();
        };
        let mut l = ControllerLayout::default();
        // Un élément branché sur une sortie qui n'existe plus est rangé sur la dernière.
        let last = c.pins.len().max(1) - 1;
        for o in 0..=last {
            let start = l.total;
            for (i, f) in self.fixtures.iter().enumerate() {
                if f.controller == c.id && f.output.min(last) == o {
                    let n = f.led_count();
                    l.fixtures.push((i, l.total, n));
                    l.total += n;
                }
            }
            if l.total > start {
                l.outputs.push((o, start, l.total - start));
            }
        }
        l
    }

    /// Toutes les LEDs, élément par élément, avec leur point dans l'image et leur adresse.
    pub fn leds(&self) -> Vec<Led> {
        let mut addr: HashMap<usize, (usize, usize)> = HashMap::new();
        for ci in 0..self.controllers.len() {
            for (fi, start, _) in self.layout(ci).fixtures {
                addr.insert(fi, (ci, start));
            }
        }
        let mut leds = Vec::new();
        for (fi, f) in self.fixtures.iter().enumerate() {
            let a = addr.get(&fi).copied();
            for (i, pos) in f.positions().into_iter().enumerate() {
                leds.push(Led {
                    pos,
                    uv: [0.5, 0.5],
                    fixture: fi,
                    index: i,
                    controller: a.map(|a| a.0),
                    address: a.map_or(0, |a| a.1 + i),
                });
            }
        }
        // Image calée sur un écran : chaque LED prend le point de l'écran qui lui fait face.
        if let Some(screen) = self.image_on.and_then(|id| self.surfaces.iter().find(|s| s.id == id)) {
            for led in &mut leds {
                led.uv = screen.image_uv(led.pos);
            }
            return leds;
        }
        // L'image couvre l'installation vue selon la projection choisie.
        let flat: Vec<[f32; 2]> = leds.iter().map(|l| self.projection.flatten(l.pos)).collect();
        if !flat.is_empty() {
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in &flat {
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            let mut size = [(hi[0] - lo[0]).max(1e-3), (hi[1] - lo[1]).max(1e-3)];
            let mut center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
            if !self.stretch {
                // Proportions gardées (image en 16:9 environ) : la plus grande dimension remplit l'image.
                let aspect = 16.0 / 9.0;
                if size[0] / size[1] > aspect {
                    size[1] = size[0] / aspect;
                } else {
                    size[0] = size[1] * aspect;
                }
            }
            // Une installation plate (une seule colonne vue de face…) prend une bande centrale.
            for k in 0..2 {
                if hi[k] - lo[k] < 1e-3 {
                    center[k] = lo[k];
                }
            }
            for (led, p) in leds.iter_mut().zip(flat) {
                let u = (p[0] - center[0]) / size[0] + 0.5;
                let v = 0.5 - (p[1] - center[1]) / size[1];
                led.uv = [u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)];
            }
        }
        leds
    }

    /// Exemple d'installation : deux rangées de quatre colonnes, de part et d'autre de la salle.
    pub fn columns_example(&mut self) {
        let ctrl = self.controllers.first().map_or(1, |c| c.id);
        let depth = self.room[1];
        self.fixtures = (0..8)
            .map(|i| {
                let left = i < 4;
                Fixture {
                    id: i as u32 + 1,
                    name: format!("{}{}", if left { "G" } else { "D" }, i % 4 + 1),
                    pos: [
                        if left { -0.37 } else { 0.37 } * self.room[0],
                        0.0,
                        (1.0 + 2.0 * (i % 4) as f32).min(depth - 0.5),
                    ],
                    controller: ctrl,
                    output: i,
                    ..Fixture::default()
                }
            })
            .collect();
    }
}

// ---------------------------------------------------------------- couleurs

/// Image de la sortie lue sur le GPU pour les LEDs (RGBA, lignes du bas vers le haut).
#[derive(Default)]
pub struct Frame {
    pub pixels: Vec<u8>,
    pub size: (u32, u32),
    /// Numéro de l'image (change à chaque lecture).
    pub serial: u64,
}

impl Frame {
    /// Couleur moyenne autour d'un point (u, v : 0-1, v vers le bas).
    fn sample(&self, uv: [f32; 2]) -> [f32; 3] {
        let (w, h) = (self.size.0 as i32, self.size.1 as i32);
        if w == 0 || h == 0 || self.pixels.len() < (w * h * 4) as usize {
            return [0.0; 3];
        }
        let x = (uv[0] * w as f32) as i32;
        // Les lignes lues sur le GPU commencent en bas.
        let y = ((1.0 - uv[1]) * h as f32) as i32;
        let mut acc = [0.0f32; 3];
        let mut n = 0.0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (px, py) = ((x + dx).clamp(0, w - 1), (y + dy).clamp(0, h - 1));
                let i = ((py * w + px) * 4) as usize;
                for k in 0..3 {
                    acc[k] += self.pixels[i + k] as f32;
                }
                n += 1.0;
            }
        }
        acc.map(|c| c / n)
    }
}

/// Teinte (0-1) → couleur pure.
pub fn hue(h: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [r * 255.0, g * 255.0, b * 255.0]
}

/// Couleur de chaque LED (même ordre que `leds`).
pub fn colors(
    setup: &LedSetup,
    leds: &[Led],
    frame: &Frame,
    time: f32,
    level: f32,
    flash: Option<(usize, f32)>,
    music: Music,
) -> Vec<[u8; 3]> {
    // Scénarios : rang du groupe de chaque élément (sans groupes : un groupe par élément).
    let group_of: Vec<Option<usize>> = if setup.groups.is_empty() {
        (0..setup.fixtures.len()).map(Some).collect()
    } else {
        setup.fixtures.iter().map(|f| setup.groups.iter().position(|g| g.id == f.group)).collect()
    };
    let n_groups = if setup.groups.is_empty() { setup.fixtures.len() } else { setup.groups.len() };
    let sc = &setup.scenario;
    let t_sc = if sc.sync { music.beat } else { time * 2.0 } * sc.speed;
    // Enchaînement : un scénario tiré au hasard par tranche de temps, avec un fondu d'une seconde.
    let (kind, fade) = if sc.chain {
        let secs = sc.chain_secs.max(3.0);
        let block = (time / secs).floor() as u32;
        let k = ScenarioKind::ALL[(hash01(block, 3, 9) * ScenarioKind::ALL.len() as f32) as usize % ScenarioKind::ALL.len()];
        let tin = time - block as f32 * secs;
        (k, tin.min(secs - tin).min(1.0).max(0.0))
    } else {
        (sc.kind, 1.0)
    };
    // Boîte de l'installation, pour les balayages.
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for l in leds {
        for k in 0..3 {
            lo[k] = lo[k].min(l.pos[k]);
            hi[k] = hi[k].max(l.pos[k]);
        }
    }
    let sweep = |l: &Led, k: usize| {
        let span = (hi[k] - lo[k]).max(0.05);
        let x = (l.pos[k] - lo[k]) / span;
        let front = (time * 0.4).rem_euclid(1.3) - 0.15;
        let d = ((x - front) / 0.08).abs();
        let a = (1.0 - d).max(0.0);
        [a * 255.0, a * 255.0, a * 255.0]
    };
    let n_fix = setup.fixtures.len().max(1);
    leds.iter()
        .map(|l| {
            let c = match setup.source {
                Source::Video => frame.sample(l.uv),
                Source::Scenario => match group_of.get(l.fixture).copied().flatten() {
                    Some(g) => {
                        let len = setup.fixtures.get(l.fixture).map_or(1, Fixture::led_count);
                        let u = if len > 1 { l.index as f32 / (len - 1) as f32 } else { 0.5 };
                        let id = (l.fixture as u32) << 16 | l.index as u32;
                        scenario_color(sc, kind, g, n_groups, u, id, t_sc, time, music.level).map(|v| v * fade)
                    }
                    None => [0.0; 3],
                },
                Source::Solid => setup.test_color.map(f32::from),
                Source::Rainbow => hue(l.uv[0] * 0.8 + l.uv[1] * 0.3 - time * 0.25),
                Source::Chase => {
                    let n = setup.fixtures.get(l.fixture).map_or(1, Fixture::led_count) as f32;
                    // Un tour en 3 s quelle que soit la longueur, avec une traîne.
                    let head = (time / 3.0).rem_euclid(1.0) * n;
                    let behind = (head - l.index as f32).rem_euclid(n);
                    let a = if behind < 1.0 { 1.0 } else { (1.0 - behind / (n * 0.15).max(3.0)).max(0.0) * 0.6 };
                    let h = hue(l.fixture as f32 / n_fix as f32);
                    if behind < 1.0 { [255.0; 3] } else { h.map(|v| v * a) }
                }
                Source::SweepX => sweep(l, 0),
                Source::SweepY => sweep(l, 1),
                Source::SweepZ => sweep(l, 2),
                Source::Identify => {
                    if l.index == 0 {
                        [255.0; 3]
                    } else {
                        hue(l.fixture as f32 / n_fix as f32).map(|v| v * 0.7)
                    }
                }
                Source::Off => [0.0; 3],
            };
            // Élément qui clignote (repérage) : blanc / noir, les autres éteints.
            let c = match flash {
                Some((f, t)) => {
                    if f == l.fixture && (t * 4.0).fract() < 0.5 {
                        [200.0; 3]
                    } else {
                        [0.0; 3]
                    }
                }
                None => c.map(|v| v * setup.brightness * level),
            };
            c.map(|v| v.round().clamp(0.0, 255.0) as u8)
        })
        .collect()
}

// ---------------------------------------------------------------- envoi DDP

/// Paquets DDP (en-tête de 10 octets) pour une image RGB complète.
pub fn ddp_packets(rgb: &[u8], seq: u8) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let chunks: Vec<&[u8]> = if rgb.is_empty() { vec![&[][..]] } else { rgb.chunks(DDP_CHUNK).collect() };
    let last = chunks.len() - 1;
    for (i, chunk) in chunks.into_iter().enumerate() {
        let mut p = Vec::with_capacity(10 + chunk.len());
        // Version 1, « push » sur le dernier paquet : WLED affiche l'image complète.
        p.push(if i == last { 0x41 } else { 0x40 });
        p.push(seq & 0x0F);
        p.push(0x0B); // RGB, 8 bits par canal
        p.push(0x01); // sortie par défaut
        p.extend_from_slice(&((i * DDP_CHUNK) as u32).to_be_bytes());
        p.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
        p.extend_from_slice(chunk);
        out.push(p);
    }
    out
}

/// Image à envoyer à un contrôleur.
struct Packet {
    host: String,
    rgb: Vec<u8>,
    seq: u8,
}

/// Envoi en arrière-plan : la résolution des noms et l'UDP ne bloquent jamais l'interface.
pub struct Sender {
    tx: SyncSender<Packet>,
    /// Dernier état de chaque contrôleur (adresse) : Ok(heure du dernier envoi) ou erreur.
    pub status: Arc<Mutex<HashMap<String, Result<Instant, String>>>>,
    seq: u8,
}

impl Default for Sender {
    fn default() -> Self {
        Self::new()
    }
}

impl Sender {
    pub fn new() -> Self {
        // Assez de place pour une image de chaque contrôleur, même nombreux.
        let (tx, rx) = mpsc::sync_channel::<Packet>(64);
        let status = Arc::new(Mutex::new(HashMap::new()));
        let st = status.clone();
        std::thread::spawn(move || send_loop(rx, st));
        Self { tx, status, seq: 0 }
    }

    /// Envoie une image (RGB) à un contrôleur ; perdue si l'envoi a du retard.
    pub fn send(&mut self, host: &str, rgb: Vec<u8>) {
        self.seq = self.seq % 15 + 1;
        let _ = self.tx.try_send(Packet { host: host.trim().to_owned(), rgb, seq: self.seq });
    }
}

/// Adresse d'un contrôleur : résolue (ou non) à tel moment, résolution en cours.
#[derive(Clone, Copy)]
struct Resolved {
    addr: Option<SocketAddr>,
    at: Instant,
    busy: bool,
}

fn send_loop(rx: Receiver<Packet>, status: Arc<Mutex<HashMap<String, Result<Instant, String>>>>) {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            status.lock().unwrap().insert(String::new(), Err(format!("socket UDP : {e}")));
            return;
        }
    };
    // Les noms se résolvent à part (un « .local » ou un contrôleur éteint peut prendre des
    // secondes) : les autres contrôleurs continuent de recevoir pendant ce temps.
    let addrs: Arc<Mutex<HashMap<String, Resolved>>> = Arc::new(Mutex::new(HashMap::new()));
    for p in rx {
        if p.host.is_empty() || p.rgb.is_empty() {
            continue;
        }
        let entry = addrs.lock().unwrap().get(&p.host).copied();
        let addr = match entry {
            Some(r) if r.addr.is_some() => r.addr,
            Some(r) if r.busy || r.at.elapsed() < Duration::from_secs(5) => None,
            _ => {
                addrs.lock().unwrap().insert(p.host.clone(), Resolved { addr: None, at: Instant::now(), busy: true });
                let (addrs, status, host) = (addrs.clone(), status.clone(), p.host.clone());
                std::thread::spawn(move || {
                    let a = resolve(&host, DDP_PORT);
                    if let Err(e) = &a {
                        status.lock().unwrap().insert(host.clone(), Err(e.to_string()));
                    }
                    addrs.lock().unwrap().insert(host, Resolved { addr: a.ok(), at: Instant::now(), busy: false });
                });
                None
            }
        };
        let Some(addr) = addr else { continue };
        let mut res = Ok(Instant::now());
        for pkt in ddp_packets(&p.rgb, p.seq) {
            if let Err(e) = socket.send_to(&pkt, addr) {
                res = Err(format!("envoi : {e}"));
                break;
            }
        }
        status.lock().unwrap().insert(p.host, res);
    }
}

/// « 192.168.1.20 », « wled.local » ou « hôte:port » → adresse (port par défaut `port`).
fn resolve(host: &str, port: u16) -> Result<SocketAddr> {
    let with_port = if host.contains(':') { host.to_owned() } else { format!("{host}:{port}") };
    with_port
        .to_socket_addrs()
        .with_context(|| format!("adresse {host} introuvable"))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| anyhow!("adresse {host} introuvable"))
}

// ---------------------------------------------------------------- API JSON de WLED

/// Requête HTTP vers WLED (GET sans corps, POST avec), réponse JSON.
pub fn http_json(host: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value> {
    let host = host.trim().trim_start_matches("http://").trim_end_matches('/');
    if host.is_empty() {
        bail!("adresse du contrôleur non renseignée");
    }
    let addr = resolve(host, 80)?;
    let mut s = TcpStream::connect_timeout(&addr, timeout).with_context(|| format!("{host} ne répond pas"))?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let req = format!(
        "{} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json\r\n{}\r\n",
        if body.is_some() { "POST" } else { "GET" },
        if body.is_some() {
            format!("Content-Type: application/json\r\nContent-Length: {}\r\n", payload.len())
        } else {
            String::new()
        }
    );
    s.write_all(req.as_bytes())?;
    s.write_all(payload.as_bytes())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).with_context(|| format!("réponse de {host}"))?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| anyhow!("réponse invalide"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_lowercase();
    let mut body = raw[split + 4..].to_vec();
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if !status.starts_with('2') {
        bail!("{host} : erreur HTTP {status}");
    }
    if head.contains("transfer-encoding: chunked") {
        body = dechunk(&body);
    }
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body).context("réponse JSON invalide")
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(eol) = b.windows(2).position(|w| w == b"\r\n") else { break };
        let len = std::str::from_utf8(&b[..eol])
            .ok()
            .and_then(|l| usize::from_str_radix(l.split(';').next().unwrap_or("").trim(), 16).ok())
            .unwrap_or(0);
        b = &b[eol + 2..];
        if len == 0 || b.len() < len {
            break;
        }
        out.extend_from_slice(&b[..len]);
        b = b.get(len + 2..).unwrap_or(&[]);
    }
    out
}

/// Ce que dit un contrôleur WLED de lui-même.
#[derive(Clone, Debug, Default)]
pub struct WledInfo {
    pub name: String,
    pub version: String,
    pub arch: String,
    pub leds: u64,
    /// Pin et longueur de chaque sortie configurée.
    pub outputs: Vec<(Option<u8>, u64)>,
    pub order: Option<usize>,
}

impl WledInfo {
    pub fn summary(&self) -> String {
        format!(
            "{} · WLED {} · {} · {} LEDs sur {} sortie(s)",
            self.name,
            self.version,
            self.arch,
            self.leds,
            self.outputs.len()
        )
    }
}

pub fn read_controller(host: &str) -> Result<WledInfo> {
    let t = Duration::from_secs(4);
    let info = http_json(host, "/json/info", None, t)?;
    let cfg = http_json(host, "/json/cfg", None, t).unwrap_or(Value::Null);
    let ins = cfg["hw"]["led"]["ins"].as_array().cloned().unwrap_or_default();
    Ok(WledInfo {
        name: info["name"].as_str().unwrap_or("WLED").to_owned(),
        version: info["ver"].as_str().unwrap_or("?").to_owned(),
        arch: info["arch"].as_str().unwrap_or("?").to_owned(),
        leds: info["leds"]["count"].as_u64().unwrap_or(0),
        outputs: ins
            .iter()
            .map(|i| (i["pin"][0].as_u64().and_then(|p| u8::try_from(p).ok()), i["len"].as_u64().unwrap_or(0)))
            .collect(),
        order: ins.first().and_then(|i| i["order"].as_u64()).map(|o| (o & 15) as usize).filter(|o| *o < ORDERS.len()),
    })
}

/// Écrit les sorties dans WLED, redémarre le contrôleur, puis crée un segment par élément.
/// `progress` reçoit l'étape en cours.
pub fn push_config(
    host: &str,
    ctrl: &Controller,
    layout: &ControllerLayout,
    names: &[(String, bool)],
    progress: &dyn Fn(String),
) -> Result<String> {
    let t = Duration::from_secs(5);
    if layout.outputs.is_empty() {
        bail!("aucun élément n'est branché sur ce contrôleur");
    }
    for (o, _, _) in &layout.outputs {
        if ctrl.pins.get(*o).copied().flatten().is_none() {
            bail!("pin de la sortie {} inconnue : « Lire le contrôleur » ou saisissez-la", o + 1);
        }
    }
    progress("Lecture de la configuration actuelle…".into());
    let cur = http_json(host, "/json/cfg", None, t)?;
    let template = cur["hw"]["led"]["ins"][0].clone();
    let template = if template.is_object() { template } else { json!({"type": 22, "skip": 0, "ref": false}) };
    let ins: Vec<Value> = layout
        .outputs
        .iter()
        .map(|&(o, start, len)| {
            let mut e = template.clone();
            e["start"] = json!(start);
            e["len"] = json!(len);
            e["pin"] = json!([ctrl.pins[o].unwrap()]);
            e["order"] = json!(ctrl.order);
            e["rev"] = json!(false);
            if e.get("type").is_none() {
                e["type"] = json!(22);
            }
            e
        })
        .collect();
    progress("Envoi des sorties LED…".into());
    http_json(host, "/json/cfg", Some(&json!({"hw": {"led": {"total": layout.total, "ins": ins}}})), t)?;
    progress("Redémarrage du contrôleur…".into());
    let _ = http_json(host, "/json/state", Some(&json!({"rb": true})), t);
    std::thread::sleep(Duration::from_secs(4));
    let mut back = false;
    for _ in 0..30 {
        if http_json(host, "/json/info", None, Duration::from_secs(2)).is_ok() {
            back = true;
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    if !back {
        bail!("le contrôleur ne répond plus après le redémarrage");
    }
    progress("Création des segments…".into());
    let mut segs: Vec<Value> = layout
        .fixtures
        .iter()
        .zip(names)
        .enumerate()
        .map(|(i, (&(_, start, n), (name, rev)))| {
            json!({"id": i, "start": start, "stop": start + n, "n": name, "rev": rev, "on": true})
        })
        .collect();
    let used = segs.len();
    segs.extend((used..16).map(|i| json!({"id": i, "stop": 0})));
    http_json(host, "/json/state", Some(&json!({"seg": segs})), t)?;
    Ok(format!("OK : {} sortie(s), {} LEDs, {} segment(s)", layout.outputs.len(), layout.total, used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddp_splits_and_pushes_last() {
        let rgb = vec![7u8; 500 * 3];
        let p = ddp_packets(&rgb, 3);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][0], 0x40);
        assert_eq!(p[1][0], 0x41);
        assert_eq!(u32::from_be_bytes(p[1][4..8].try_into().unwrap()), 1440);
        assert_eq!(u16::from_be_bytes(p[1][8..10].try_into().unwrap()), 60);
        assert_eq!(p[1].len(), 70);
    }

    #[test]
    fn layout_follows_outputs() {
        let mut s = LedSetup::default();
        s.columns_example();
        // Deux éléments sur la même sortie se suivent.
        s.fixtures[1].output = 0;
        let l = s.layout(0);
        assert_eq!(l.total, 240);
        assert_eq!(l.outputs[0], (0, 0, 60));
        assert_eq!(l.fixtures[1], (1, 30, 30));
        let leds = s.leds();
        assert_eq!(leds.len(), 240);
        assert_eq!(leds[30].address, 30);
        assert!(leds.iter().all(|l| (0.0..=1.0).contains(&l.uv[0])));
    }

    #[test]
    fn column_goes_up_and_reverses() {
        let f = Fixture { count: 10, reverse: true, ..Fixture::default() };
        let p = f.positions();
        assert!(p[0][1] > p[9][1]);
        let r = Fixture { kind: FixtureKind::Strip, rot: [0.0, 0.0, 90.0], ..Fixture::default() };
        let p = r.positions();
        assert!((p.last().unwrap()[1] - r.size[0]).abs() < 1e-4);
    }

    #[test]
    fn sender_reaches_a_udp_listener() {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let port = rx.local_addr().unwrap().port();
        let mut tx = Sender::new();
        // La première image lance la résolution de l'adresse ; les suivantes partent.
        let mut buf = [0u8; 64];
        rx.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let mut n = 0;
        for _ in 0..30 {
            tx.send(&format!("127.0.0.1:{port}"), vec![1, 2, 3, 4, 5, 6]);
            if let Ok(k) = rx.recv(&mut buf) {
                n = k;
                break;
            }
        }
        buf[1] = 1; // numéro de séquence : dépend du nombre d'essais
        assert_eq!(&buf[..n], &[0x41, 1, 0x0B, 1, 0, 0, 0, 0, 0, 6, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn screen_maps_its_corners() {
        let s = Surface { pos: [1.0, 0.5, 4.0], rot: [30.0, 0.0, 0.0], ..Surface::default() };
        let f = s.front();
        let uv = |p| s.image_uv(p);
        let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        assert!(close(uv(f[0]), [0.0, 0.0]));
        assert!(close(uv(f[2]), [1.0, 1.0]));
    }

    #[test]
    fn path_spreads_leds_evenly() {
        let mut f = Fixture { kind: FixtureKind::Path, count: 5, ..Fixture::default() };
        f.set_path(&[[1.0, 0.0, 1.0], [3.0, 0.0, 1.0], [3.0, 2.0, 1.0]], false);
        assert!((f.path_length() - 4.0).abs() < 1e-5);
        let p = f.positions();
        assert_eq!(p[0], [1.0, 0.0, 1.0]);
        assert_eq!(p[2], [3.0, 0.0, 1.0]);
        assert_eq!(p[4], [3.0, 2.0, 1.0]);
    }

    #[test]
    fn panels_make_leds_between_layers() {
        let s = Surface { kind: SurfaceKind::Panels, pos: [0.0; 3], size: [5.5, 2.6, 0.5], ..Surface::default() };
        // 15 panneaux : 7 de chaque côté + celui du centre ; les deux du bout n'ont pas de bande.
        let paths = s.panel_led_paths();
        assert_eq!(paths.len(), 14);
        assert_eq!(paths.iter().filter(|p| p.0 < 0.0).count(), 7);
        assert!(!s.faces(0.0).is_empty());
    }

    #[test]
    fn center_out_groups_pair_left_and_right() {
        let mut s = LedSetup::default();
        s.columns_example();
        s.auto_groups(true);
        // 8 colonnes face à face (x = ±2,22 m) : toutes à la même distance du centre, mais
        // en profondeur différente → un seul groupe en largeur.
        assert_eq!(s.groups.len(), 1);
        s.auto_groups(false);
        assert_eq!(s.groups.len(), 8);
        let leds = s.leds();
        let c = colors(&s, &leds, &Frame::default(), 0.0, 1.0, None, Music::default());
        assert_eq!(c.len(), leds.len());
    }

    #[test]
    fn sets_move_turn_and_scale_together() {
        let mut s = LedSetup::default();
        s.columns_example();
        s.sets.push(LedGroup { id: 1, name: "A".into() });
        s.fixtures[0].set = 1;
        s.fixtures[4].set = 1;
        let (a, b) = (s.fixtures[0].pos, s.fixtures[4].pos);
        s.move_set(1, [1.0, 0.0, 0.0]);
        assert_eq!(s.fixtures[0].pos[0], a[0] + 1.0);
        assert_eq!(s.fixtures[1].pos, LedSetup { ..s.clone() }.fixtures[1].pos);
        s.rotate_set(1, 180.0);
        // Un demi-tour échange les deux colonnes autour de leur milieu.
        assert!((s.fixtures[0].pos[0] - (b[0] + 1.0)).abs() < 1e-4);
        let before = s.fixtures[0].size[0];
        s.scale_set(1, 2.0);
        assert!((s.fixtures[0].size[0] - before * 2.0).abs() < 1e-5);
        s.fixtures[4].set = 0;
        s.prune_sets();
        assert!(s.sets.is_empty() && s.fixtures[0].set == 0);
    }

    #[test]
    fn move_block_keeps_order() {
        let mut v: Vec<u8> = (0..6).collect();
        // 1 et 4 glissés avant le 3.
        assert_eq!(move_block(&mut v, &[4, 1], 3), 2);
        assert_eq!(v, [0, 2, 1, 4, 3, 5]);
        let mut v: Vec<u8> = (0..4).collect();
        assert_eq!(move_block(&mut v, &[0], 4), 3);
        assert_eq!(v, [1, 2, 3, 0]);
        let mut v: Vec<u8> = (0..4).collect();
        move_block(&mut v, &[2, 3], 0);
        assert_eq!(v, [2, 3, 0, 1]);
    }

    #[test]
    fn dechunks() {
        assert_eq!(dechunk(b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"), b"abcde");
    }
}
