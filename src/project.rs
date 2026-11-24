//! Fichier projet `.ozmap` (JSON) : médias, calques, presets et réglages.
//!
//! Les chemins des médias situés à côté du projet sont enregistrés en relatif,
//! pour pouvoir déplacer ou copier le dossier complet.

use crate::rhythm::React;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::automix::AutoMix;
use crate::effects::{AutoFx, BlendMode, Effects, Preset};
use crate::generator::Generator;
use crate::surface::{Fit, Mask, Warp};

pub const EXTENSION: &str = "ozmap";
const VERSION: u32 = 1;
pub const MAX_COLUMNS: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerConfig {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    /// Cadrage de l'image dans la surface (étirer, ajuster, remplir).
    pub fit: Fit,
    /// Rotation du média dans la surface, en degrés (le cadrage en tient compte).
    pub media_rotate: f32,
    pub effects: Effects,
    /// Navigation automatique entre les effets.
    pub auto: AutoFx,
    /// Mix auto entre plusieurs vidéos.
    pub mix: AutoMix,
    /// Réaction au rythme (son ou tempo tapé).
    pub react: React,
    /// Visuel généré (utilisable sans vidéo).
    pub generator: Generator,
    /// Photo plaquée sur le visuel « Visage qui parle » (le visage de quelqu'un, vu de face).
    pub face_photo: Option<PathBuf>,
    pub warp: Warp,
    pub mask: Mask,
    /// Vidéo chargée sur ce calque (relancée à l'ouverture du projet).
    pub clip: Option<PathBuf>,
    /// Le calque affiche le dessin fait à distance (iPad, téléphone) à la place de sa vidéo.
    pub sketch: bool,
    /// Sortie (index dans `Project::outputs`) où ce calque est affiché.
    pub output: usize,
    /// Ligne du tableau : cases enregistrées pour ce calque (index = colonne).
    pub cells: Vec<Option<Cell>>,
    /// Groupe de calques (id dans `Project::groups`, 0 : aucun).
    pub group: u32,
    /// Bassin de chimie de ce calque : contenu enregistré (image) et réglages.
    pub chem: Option<ChemSaved>,
}

/// Bassin de chimie enregistré.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChemSaved {
    /// Contenu du bassin (PNG 16 bits, voir `chem::Basin`).
    pub file: Option<PathBuf>,
    pub settings: crate::chem::ChemSettings,
}

/// Groupe de calques : ses calques se suivent dans la liste ; on les montre, cache, atténue,
/// déplace et joue ensemble.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerGroup {
    pub id: u32,
    pub name: String,
    pub visible: bool,
    /// Multiplie l'opacité de chacun de ses calques.
    pub opacity: f32,
    /// Replié dans la liste des calques.
    pub collapsed: bool,
    /// Effet de groupe : les calques s'allument tour à tour, chacun de sa couleur.
    pub fx: crate::group_fx::GroupEffect,
}

impl Default for LayerGroup {
    fn default() -> Self {
        Self { id: 1, name: "Groupe 1".into(), visible: true, opacity: 1.0, collapsed: false, fx: Default::default() }
    }
}

/// Case du tableau : un média avec ses effets et ses réglages, rejoué d'un clic sur son calque.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cell {
    pub name: String,
    /// Média joué (aucun : seulement le visuel généré, ou rien).
    pub clip: Option<PathBuf>,
    pub effects: Effects,
    /// Nom du preset d'origine, pour le surligner une fois la case jouée.
    pub preset: Option<String>,
    pub generator: Generator,
    pub auto: AutoFx,
    pub fit: Fit,
    pub opacity: f32,
    pub blend: BlendMode,
    // Réglages ajoutés ensuite : absents (None) dans une case plus ancienne, le calque garde alors les siens.
    pub warp: Option<Warp>,
    pub mask: Option<Mask>,
    pub react: Option<React>,
    pub mix: Option<AutoMix>,
    pub media_rotate: Option<f32>,
    /// Lecture : vitesse et sens.
    pub speed: Option<f32>,
    pub reverse: Option<bool>,
    pub paused: Option<bool>,
    /// La case montre le dessin de l'iPad (absent dans une case plus ancienne : non).
    pub drawing: Option<bool>,
}

impl Default for Cell {
    fn default() -> Self {
        let layer = LayerConfig::default();
        Self {
            name: String::new(),
            clip: None,
            effects: layer.effects,
            preset: None,
            generator: layer.generator,
            auto: layer.auto,
            fit: layer.fit,
            opacity: layer.opacity,
            blend: layer.blend,
            warp: None,
            mask: None,
            react: None,
            mix: None,
            media_rotate: None,
            speed: None,
            reverse: None,
            paused: None,
            drawing: None,
        }
    }
}

/// Enchaînement automatique des colonnes du tableau.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chain {
    pub enabled: bool,
    /// Durée d'une colonne, en secondes.
    pub secs: f32,
}

impl Default for Chain {
    fn default() -> Self {
        Self { enabled: false, secs: 8.0 }
    }
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            name: "Calque 1".into(),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            fit: Fit::Stretch,
            media_rotate: 0.0,
            effects: Effects::default(),
            auto: AutoFx::default(),
            mix: AutoMix::default(),
            react: React::default(),
            generator: Generator::default(),
            face_photo: None,
            warp: Warp::default(),
            mask: Mask::default(),
            clip: None,
            sketch: false,
            output: 0,
            cells: Vec::new(),
            group: 0,
            chem: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub media: Vec<PathBuf>,
    /// Du bas vers le haut.
    pub layers: Vec<LayerConfig>,
    pub presets: Vec<Preset>,
    /// Presets mémorisés des animations sur la forme du masque.
    pub mask_presets: Vec<Preset>,
    pub fade_secs: f32,
    pub preset_secs: f32,
    pub looping: bool,
    pub master: f32,
    /// Nombre de colonnes du tableau.
    pub columns: usize,
    pub chain: Chain,
    /// LEDs pilotées par des contrôleurs WLED (installation modélisée en 3D).
    pub leds: crate::leds::LedSetup,
    /// Sorties (chacune sa fenêtre, son écran, ses calques). Au moins une.
    pub outputs: Vec<OutputDef>,
    /// Groupes de calques.
    pub groups: Vec<LayerGroup>,
    /// Dossiers de médias ajoutés par référence (sous-dossiers compris, relus régulièrement).
    pub media_folders: Vec<PathBuf>,
    /// Dessin fait sur l'iPad (traits et photos posées dedans).
    pub drawing: Option<crate::sketch::DrawingDoc>,
}

/// Une sortie : sa fenêtre affiche les calques qui lui sont attribués.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputDef {
    pub name: String,
    /// Écran choisi (sorties 2 et suivantes ; la sortie 1 garde le sien dans la session).
    pub screen: Option<String>,
    /// Orientation du vidéoprojecteur : rotation de l'image (degrés, sens horaire), n'importe
    /// quel angle. Vers 90° ou 270°, l'image devient verticale (VP posé sur le côté).
    pub rotate: f32,
    /// Miroir gauche-droite (rétroprojection) et haut-bas, dans l'image du VP.
    pub mirror_x: bool,
    pub mirror_y: bool,
    /// VP qui n'est pas en face de la surface (degrés) : décalé sur le côté (+ : à gauche,
    /// tourné vers la droite) ou en hauteur (+ : plus bas, tourné vers le haut). L'image est
    /// pré-déformée pour arriver rectangulaire sur la surface.
    pub tilt_h: f32,
    pub tilt_v: f32,
    /// Rapport de projection de l'objectif (distance / largeur de l'image) : plus il est petit
    /// (courte focale), plus un même angle déforme l'image.
    pub throw: f32,
    /// Réglage des 4 coins (haut-gauche, haut-droit, bas-droit, bas-gauche) : décalage de
    /// chaque coin de l'image dans l'image du VP, en fraction de sa taille, après tout le reste.
    pub corners: [[f32; 2]; 4],
}

impl Default for OutputDef {
    fn default() -> Self {
        Self {
            name: "Sortie 1".into(),
            screen: None,
            rotate: 0.0,
            mirror_x: false,
            mirror_y: false,
            tilt_h: 0.0,
            tilt_v: 0.0,
            throw: 1.5,
            corners: [[0.0; 2]; 4],
        }
    }
}

/// Angle maximal de biais du VP pris en compte (degrés).
pub const MAX_TILT: f32 = 60.0;

impl OutputDef {
    /// L'image est tournée, retournée ou pré-déformée avant d'arriver au VP.
    pub fn oriented(&self) -> bool {
        let r = self.rotate.rem_euclid(360.0);
        (r > 1e-3 && r < 360.0 - 1e-3) || self.mirror_x || self.mirror_y || self.tilt_h.abs() > 1e-3 || self.tilt_v.abs() > 1e-3 || self.corners_moved()
    }

    /// Un coin au moins a été déplacé à la main.
    pub fn corners_moved(&self) -> bool {
        self.corners.iter().flatten().any(|v| v.abs() > 1e-6)
    }

    /// Taille de l'image composée (masques, déformations, aperçu) pour un VP de taille `phys` :
    /// largeur et hauteur échangées quand le VP est plutôt sur le côté (entre 45° et 135°).
    pub fn logical(&self, phys: [f32; 2]) -> [f32; 2] {
        let a = self.rotate.rem_euclid(180.0);
        if a > 45.0 && a < 135.0 { [phys[1], phys[0]] } else { phys }
    }

    /// Placement de l'image composée dans l'image d'un VP de taille `phys` (à calculer une fois,
    /// puis `Projection::place` pour chaque point).
    pub fn projection(&self, phys: [f32; 2]) -> Projection {
        Projection::new(self, phys)
    }

    /// Coins de l'image composée (haut-gauche, haut-droit, bas-droit, bas-gauche) dans l'image
    /// du VP, en fraction de sa taille.
    pub fn corners(&self, phys: [f32; 2]) -> [[f32; 2]; 4] {
        let pr = self.projection(phys);
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|c| {
            let q = pr.place(c);
            [q[0] / phys[0].max(1.0), q[1] / phys[1].max(1.0)]
        })
    }
}

/// Image composée → image du VP. On suit les rayons du VP (rotation sur lui-même, biais
/// horizontal et vertical) jusqu'à la surface, supposée plane et face au public : l'image
/// composée y est le plus grand rectangle droit (à ses proportions) que le VP peut éclairer,
/// et chacun de ses points revient dans l'image du VP par le même rayon. Sans biais, c'est une
/// simple rotation, réduite si l'angle n'est pas droit.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    /// Repère du VP → repère de la surface (colonnes : axes du VP).
    rot: [[f32; 3]; 3],
    /// Focale en pixels de l'image du VP.
    focal: f32,
    phys: [f32; 2],
    /// Rectangle de l'image composée sur la surface : centre et demi-taille.
    center: [f32; 2],
    half: [f32; 2],
    mirror: [bool; 2],
    /// Image composée (0..1) → pixels du VP, coins réglés à la main compris, et l'inverse.
    to_vp: [[f32; 3]; 3],
    from_vp: [[f32; 3]; 3],
}

impl Projection {
    fn new(o: &OutputDef, phys: [f32; 2]) -> Self {
        let phys = [phys[0].max(1.0), phys[1].max(1.0)];
        let (sr, cr) = (-o.rotate.to_radians()).sin_cos();
        let (sy, cy) = o.tilt_h.clamp(-MAX_TILT, MAX_TILT).to_radians().sin_cos();
        let (sp, cp) = o.tilt_v.clamp(-MAX_TILT, MAX_TILT).to_radians().sin_cos();
        let roll = [[cr, -sr, 0.0], [sr, cr, 0.0], [0.0, 0.0, 1.0]];
        // Haut de l'image vers -y : un VP plus bas, tourné vers le haut, envoie son axe vers -y.
        let pitch = [[1.0, 0.0, 0.0], [0.0, cp, -sp], [0.0, sp, cp]];
        let yaw = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
        let rot = mat_mul(yaw, mat_mul(pitch, roll));
        let mut pr = Self {
            rot,
            focal: o.throw.clamp(0.2, 10.0) * phys[0],
            phys,
            center: [0.0, 0.0],
            half: [0.0, 0.0],
            mirror: [o.mirror_x, o.mirror_y],
            to_vp: IDENTITY,
            from_vp: IDENTITY,
        };
        // Zone éclairée sur la surface : l'image des quatre coins du VP.
        let lit: Vec<[f32; 2]> = [[0.0, 0.0], [phys[0], 0.0], [phys[0], phys[1]], [0.0, phys[1]]].iter().filter_map(|&r| pr.wall(r)).collect();
        if lit.len() < 4 {
            pr.set_corners(o);
            return pr;
        }
        let center = [lit.iter().map(|p| p[0]).sum::<f32>() / 4.0, lit.iter().map(|p| p[1]).sum::<f32>() / 4.0];
        let [lw, lh] = o.logical(phys);
        let aspect = lw / lh.max(1.0);
        let inside = |s: f32| {
            [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].iter().all(|c| {
                let p = [center[0] + c[0] * s * aspect, center[1] + c[1] * s];
                (0..4).all(|k| {
                    let (a, b) = (lit[k], lit[(k + 1) % 4]);
                    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-9
                })
            })
        };
        // Le plus grand rectangle centré qui tient dans la zone éclairée.
        let (mut lo, mut hi) = (0.0f32, lit.iter().map(|p| (p[0] - center[0]).abs().max((p[1] - center[1]).abs())).fold(0.0, f32::max));
        for _ in 0..60 {
            let mid = (lo + hi) * 0.5;
            if inside(mid) { lo = mid } else { hi = mid }
        }
        pr.center = center;
        pr.half = [lo * aspect, lo];
        pr.set_corners(o);
        pr
    }

    /// Coins de l'image dans le VP (calculés, puis décalés à la main) : la correspondance
    /// devient la perspective qui envoie le carré de l'image sur ces quatre points.
    fn set_corners(&mut self, o: &OutputDef) {
        let base = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|c| self.place_computed(c));
        let quad: [[f32; 2]; 4] = std::array::from_fn(|k| [base[k][0] + o.corners[k][0] * self.phys[0], base[k][1] + o.corners[k][1] * self.phys[1]]);
        self.to_vp = square_to_quad(quad);
        self.from_vp = invert3(self.to_vp);
    }

    /// Pixel du VP → point de la surface (None : le rayon ne la touche pas).
    fn wall(&self, r: [f32; 2]) -> Option<[f32; 2]> {
        let v = [r[0] - self.phys[0] * 0.5, r[1] - self.phys[1] * 0.5, self.focal];
        let d = mat_vec(self.rot, v);
        (d[2] > 1e-6).then(|| [d[0] / d[2], d[1] / d[2]])
    }

    /// Point de la surface → pixel du VP.
    fn raster(&self, w: [f32; 2]) -> [f32; 2] {
        let t = [[self.rot[0][0], self.rot[1][0], self.rot[2][0]], [self.rot[0][1], self.rot[1][1], self.rot[2][1]], [self.rot[0][2], self.rot[1][2], self.rot[2][2]]];
        let d = mat_vec(t, [w[0], w[1], 1.0]);
        let z = if d[2].abs() < 1e-6 { 1e-6 } else { d[2] };
        [self.focal * d[0] / z + self.phys[0] * 0.5, self.focal * d[1] / z + self.phys[1] * 0.5]
    }

    /// Pixel du VP (y vers le bas) du point `p` (0..1) de l'image composée.
    pub fn place(&self, p: [f32; 2]) -> [f32; 2] {
        apply(self.to_vp, p).unwrap_or(p)
    }

    /// Point (0..1) de l'image composée sous le pixel `q` du VP.
    pub fn unplace(&self, q: [f32; 2]) -> Option<[f32; 2]> {
        apply(self.from_vp, q)
    }

    /// Placement calculé (rotation, biais, miroirs), avant le réglage des coins.
    fn place_computed(&self, p: [f32; 2]) -> [f32; 2] {
        let w = [self.center[0] + (2.0 * p[0] - 1.0) * self.half[0], self.center[1] + (2.0 * p[1] - 1.0) * self.half[1]];
        let mut q = self.raster(w);
        if self.mirror[0] {
            q[0] = self.phys[0] - q[0];
        }
        if self.mirror[1] {
            q[1] = self.phys[1] - q[1];
        }
        q
    }

}

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Applique une perspective (matrice en lignes) à un point ; None derrière l'horizon.
fn apply(m: [[f32; 3]; 3], p: [f32; 2]) -> Option<[f32; 2]> {
    let v = [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2]);
    (v[2].abs() > 1e-9).then(|| [v[0] / v[2], v[1] / v[2]])
}

/// Perspective du carré unité vers le quadrilatère `q` (coins dans l'ordre (0,0), (1,0),
/// (1,1), (0,1)), en lignes : (x, y, w) = M · (u, v, 1).
pub fn square_to_quad(q: [[f32; 2]; 4]) -> [[f32; 3]; 3] {
    let [p0, p1, p2, p3] = q;
    let (dx1, dx2, dx3) = (p1[0] - p2[0], p3[0] - p2[0], p0[0] - p1[0] + p2[0] - p3[0]);
    let (dy1, dy2, dy3) = (p1[1] - p2[1], p3[1] - p2[1], p0[1] - p1[1] + p2[1] - p3[1]);
    let den = dx1 * dy2 - dx2 * dy1;
    let (g, h) = if den.abs() < 1e-12 { (0.0, 0.0) } else { ((dx3 * dy2 - dx2 * dy3) / den, (dx1 * dy3 - dx3 * dy1) / den) };
    [
        [p1[0] - p0[0] + g * p1[0], p3[0] - p0[0] + h * p3[0], p0[0]],
        [p1[1] - p0[1] + g * p1[1], p3[1] - p0[1] + h * p3[1], p0[1]],
        [g, h, 1.0],
    ]
}

/// Inverse d'une matrice 3×3 (en lignes) ; l'identité si elle n'est pas inversible.
pub fn invert3(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let c = |r: usize, k: usize| {
        let (r1, r2, k1, k2) = ((r + 1) % 3, (r + 2) % 3, (k + 1) % 3, (k + 2) % 3);
        m[r1][k1] * m[r2][k2] - m[r1][k2] * m[r2][k1]
    };
    let det = m[0][0] * c(0, 0) + m[0][1] * c(0, 1) + m[0][2] * c(0, 2);
    if det.abs() < 1e-12 {
        return IDENTITY;
    }
    // Inverse = transposée des cofacteurs / déterminant.
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| c(j, i) / det))
}

fn mat_mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

fn mat_vec(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: VERSION,
            media: Vec::new(),
            layers: vec![LayerConfig::default()],
            presets: Vec::new(),
            mask_presets: Vec::new(),
            fade_secs: 1.0,
            preset_secs: 0.5,
            looping: true,
            master: 1.0,
            columns: 8,
            chain: Chain::default(),
            leds: crate::leds::LedSetup::default(),
            outputs: vec![OutputDef::default()],
            groups: Vec::new(),
            media_folders: Vec::new(),
            drawing: None,
        }
    }
}

impl Project {
    /// Groupe d'un calque.
    pub fn group_of(&self, i: usize) -> Option<&LayerGroup> {
        let g = self.layers.get(i)?.group;
        (g != 0).then(|| self.groups.iter().find(|x| x.id == g)).flatten()
    }

    /// Le calque se voit : lui et son groupe sont affichés.
    pub fn layer_visible(&self, i: usize) -> bool {
        self.layers.get(i).is_some_and(|l| l.visible) && self.group_of(i).is_none_or(|g| g.visible)
    }

    /// Opacité réelle du calque (celle de son groupe comprise).
    pub fn layer_opacity(&self, i: usize) -> f32 {
        self.layers.get(i).map_or(0.0, |l| l.opacity) * self.group_of(i).map_or(1.0, |g| g.opacity)
    }

    /// Calques d'un groupe (rangs, du bas vers le haut).
    pub fn group_members(&self, id: u32) -> Vec<usize> {
        (0..self.layers.len()).filter(|&i| id != 0 && self.layers[i].group == id).collect()
    }

    pub fn next_group_id(&self) -> u32 {
        self.groups.iter().map(|g| g.id).max().unwrap_or(0) + 1
    }

    /// Groupes sans calque retirés, calques d'un groupe disparu détachés.
    pub fn prune_groups(&mut self) {
        let used: std::collections::HashSet<u32> = self.layers.iter().map(|l| l.group).collect();
        self.groups.retain(|g| used.contains(&g.id));
        let ids: std::collections::HashSet<u32> = self.groups.iter().map(|g| g.id).collect();
        for l in &mut self.layers {
            if !ids.contains(&l.group) {
                l.group = 0;
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let base = path.parent().unwrap_or(Path::new(""));
        let mut p = self.clone();
        p.version = VERSION;
        for m in p.media.iter_mut().chain(p.media_folders.iter_mut()) {
            *m = relative_to(m, base);
        }
        for f in p.drawing.iter_mut().flat_map(|d| d.files_mut()) {
            *f = relative_to(f, base);
        }
        for l in &mut p.layers {
            if let Some(f) = l.chem.as_mut().and_then(|c| c.file.as_mut()) {
                *f = relative_to(f, base);
            }
            if let Some(c) = &mut l.clip {
                *c = relative_to(c, base);
            }
            for c in l.cells.iter_mut().flatten().filter_map(|c| c.clip.as_mut()) {
                *c = relative_to(c, base);
            }
        }
        let json = serde_json::to_string_pretty(&p)?;
        std::fs::write(path, json).with_context(|| format!("écriture de {}", path.display()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))?;
        let mut p: Project = serde_json::from_str(&text).context("fichier projet invalide")?;
        let base = path.parent().unwrap_or(Path::new(""));
        for m in p.media.iter_mut().chain(p.media_folders.iter_mut()) {
            *m = resolve(m, base);
        }
        for f in p.drawing.iter_mut().flat_map(|d| d.files_mut()) {
            *f = resolve(f, base);
        }
        for l in &mut p.layers {
            if let Some(f) = l.chem.as_mut().and_then(|c| c.file.as_mut()) {
                *f = resolve(f, base);
            }
            if let Some(c) = &mut l.clip {
                *c = resolve(c, base);
            }
            for c in l.cells.iter_mut().flatten().filter_map(|c| c.clip.as_mut()) {
                *c = resolve(c, base);
            }
        }
        p.sanitize();
        Ok(p)
    }

    /// Corrige les valeurs incohérentes (fichier édité à la main, ancienne version…).
    pub fn sanitize(&mut self) {
        if self.layers.is_empty() {
            self.layers.push(LayerConfig::default());
        }
        self.columns = self.columns.clamp(1, MAX_COLUMNS);
        self.leds.sanitize();
        if self.outputs.is_empty() {
            self.outputs.push(OutputDef::default());
        }
        let n = self.outputs.len();
        for l in &mut self.layers {
            l.output = l.output.min(n - 1);
        }
        for s in &mut self.leds.surfaces {
            s.output = s.output.min(n - 1);
        }
        for l in &mut self.layers {
            l.warp.sanitize();
            l.mask.sanitize();
            // Une case au-delà de la dernière colonne agrandit le tableau.
            if let Some(last) = l.cells.iter().rposition(Option::is_some) {
                self.columns = self.columns.max((last + 1).min(MAX_COLUMNS));
            }
            l.cells.truncate(MAX_COLUMNS);
        }
    }
}

fn resolve(p: &Path, base: &Path) -> PathBuf {
    if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
}

/// Chemin relatif si le média est dans le dossier du projet (ou un sous-dossier).
/// Toujours avec des « / », compris par macOS comme par Windows : un projet reste
/// utilisable d'un système à l'autre.
fn relative_to(p: &Path, base: &Path) -> PathBuf {
    match p.strip_prefix(base) {
        Ok(rel) if !base.as_os_str().is_empty() && rel.components().all(|c| matches!(c, Component::Normal(_))) => {
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            PathBuf::from(parts.join("/"))
        }
        _ => p.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn layer_groups() {
        let mut p = Project::default();
        p.layers = (0..3).map(|k| LayerConfig { name: format!("C{k}"), ..LayerConfig::default() }).collect();
        p.groups.push(LayerGroup { id: 4, opacity: 0.5, ..LayerGroup::default() });
        p.layers[1].group = 4;
        p.layers[2].group = 4;
        p.layers[2].opacity = 0.5;
        assert_eq!(p.layer_opacity(2), 0.25);
        assert_eq!(p.group_members(4), vec![1, 2]);
        p.groups[0].visible = false;
        assert!(p.layer_visible(0) && !p.layer_visible(1));
        // Groupe vidé : retiré ; calque d'un groupe disparu : détaché.
        p.layers[1].group = 0;
        p.layers[2].group = 9;
        p.prune_groups();
        assert!(p.groups.is_empty());
        assert_eq!(p.layers[2].group, 0);
        assert!(p.layer_visible(2));
    }

    use super::*;

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01
    }

    #[test]
    fn output_orientation() {
        let phys = [1920.0, 1080.0];
        let mut o = OutputDef::default();
        assert!(!o.oriented());
        assert_eq!(o.logical(phys), phys);
        assert!(near(o.projection(phys).place([0.0, 0.0]), [0.0, 0.0]) && near(o.projection(phys).place([1.0, 1.0]), phys));

        // VP sur le côté : image verticale, qui remplit tout le VP ; le haut de l'image à droite.
        o.rotate = 90.0;
        assert!(o.oriented());
        assert_eq!(o.logical(phys), [1080.0, 1920.0]);
        assert!(near(o.projection(phys).place([0.0, 0.0]), [1920.0, 0.0]), "{:?}", o.projection(phys).place([0.0, 0.0]));
        assert!(near(o.projection(phys).place([1.0, 1.0]), [0.0, 1080.0]));
        o.rotate = -90.0;
        assert_eq!(o.logical(phys), [1080.0, 1920.0]);
        assert!(near(o.projection(phys).place([0.0, 0.0]), [0.0, 1080.0]));

        // Angle quelconque : l'image entière reste dans le VP.
        for deg in [3.0, 30.0, 45.0, 60.0, 137.0, 200.0, 315.0] {
            o.rotate = deg;
            for c in o.corners(phys) {
                assert!((-1e-3..=1.0 + 1e-3).contains(&c[0]) && (-1e-3..=1.0 + 1e-3).contains(&c[1]), "{deg}° : {c:?}");
            }
            assert!(near(o.projection(phys).place([0.5, 0.5]), [960.0, 540.0]));
        }

        // Rétroprojection : gauche et droite échangées dans l'image du VP ; plafond : haut et bas.
        o.rotate = 0.0;
        o.mirror_x = true;
        assert!(o.oriented());
        assert!(near(o.projection(phys).place([0.0, 0.0]), [1920.0, 0.0]));
        o.mirror_y = true;
        assert!(near(o.projection(phys).place([0.0, 0.0]), [1920.0, 1080.0]));

        // VP plus bas que la surface, tourné vers le haut : l'image envoyée est plus étroite en
        // haut (elle s'élargit en montant) ; elle reste dans le VP, et chaque point y revient.
        o.mirror_x = false;
        o.mirror_y = false;
        o.tilt_v = 20.0;
        assert!(o.oriented());
        let c = o.corners(phys);
        let (top, bottom) = (c[1][0] - c[0][0], c[2][0] - c[3][0]);
        assert!(top < bottom * 0.97, "haut {top} / bas {bottom}");
        for c in c {
            assert!((-1e-3..=1.0 + 1e-3).contains(&c[0]) && (-1e-3..=1.0 + 1e-3).contains(&c[1]), "{c:?}");
        }
        o.tilt_v = 0.0;
        o.tilt_h = 25.0;
        let c = o.corners(phys);
        // VP à gauche, tourné vers la droite : le côté droit de la surface est plus loin, donc
        // plus court dans l'image envoyée.
        assert!(c[2][1] - c[1][1] < (c[3][1] - c[0][1]) * 0.97);
        let pr = o.projection(phys);
        for p in [[0.2, 0.3], [0.9, 0.1], [0.5, 0.5]] {
            assert!(near(pr.unplace(pr.place(p)).unwrap(), p));
        }

        // Coins réglés à la main : le coin bougé arrive où on l'a mis, les autres ne bougent pas,
        // et l'aller-retour reste juste au milieu de l'image.
        let mut o = OutputDef::default();
        o.corners[2] = [-0.05, -0.1];
        assert!(o.oriented() && o.corners_moved());
        let c = o.corners(phys);
        assert!(near(c[2], [0.95, 0.9]) && near(c[0], [0.0, 0.0]) && near(c[1], [1.0, 0.0]) && near(c[3], [0.0, 1.0]), "{c:?}");
        let pr = o.projection(phys);
        let q = pr.place([0.5, 0.5]);
        // Le milieu de l'image est au croisement des diagonales du quadrilatère.
        assert!((q[0] - 985.946).abs() < 0.05 && (q[1] - 525.405).abs() < 0.05, "le milieu suit la perspective : {q:?}");
        assert!(near(pr.unplace(q).unwrap(), [0.5, 0.5]));

        // Anciens projets : sans orientation.
        let old: OutputDef = serde_json::from_str(r#"{"name": "Sortie 1", "screen": null}"#).unwrap();
        assert!(!old.oriented());
    }

    #[test]
    fn media_next_to_project_are_relative() {
        let dir = std::env::temp_dir().join(format!("ozmap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("show.ozmap");
        let inside = dir.join("clips").join("a.mp4");
        let outside = std::env::temp_dir().join("ailleurs").join("b.mov");

        let mut p = Project::default();
        p.media = vec![inside.clone(), outside.clone()];
        p.layers[0].clip = Some(inside.clone());
        p.layers[0].cells = vec![None, Some(Cell { name: "A".into(), clip: Some(inside.clone()), ..Cell::default() })];
        p.save(&file).unwrap();

        let raw = std::fs::read_to_string(&file).unwrap();
        assert!(raw.contains("\"clips/a.mp4\""), "{raw}");
        assert!(!raw.contains("\"b.mov\""), "le média hors du dossier doit rester absolu : {raw}");

        let loaded = Project::load(&file).unwrap();
        // Comparaison composant par composant (sous Windows, « / » et « \\ » sont équivalents).
        let same = |a: &Path, b: &Path| a.components().eq(b.components());
        assert!(same(&loaded.media[0], &inside), "{:?}", loaded.media[0]);
        assert!(same(&loaded.media[1], &outside));
        assert!(same(loaded.layers[0].clip.as_deref().unwrap(), &inside));
        let cell = loaded.layers[0].cells[1].as_ref().expect("case enregistrée");
        assert!(same(cell.clip.as_deref().unwrap(), &inside), "{:?}", cell.clip);
        assert!(loaded.layers[0].cells[0].is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
