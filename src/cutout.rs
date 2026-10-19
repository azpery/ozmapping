//! Détourage d'une photo pour en faire un masque : séparation automatique du sujet et du fond,
//! puis retouches à la baguette magique.
//!
//! L'automatique s'inspire de GrabCut : un modèle de couleurs du fond (bords de la photo) et un
//! du sujet (centre), comparés pixel par pixel, lissés, puis réestimés sur le résultat.

use std::collections::VecDeque;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::surface::Point;

/// Plus grand côté de l'image analysée (pixels).
const WORK_MAX: u32 = 640;
/// Couleurs par modèle (fond, sujet).
const CLUSTERS: usize = 6;

/// Méthode du détourage automatique.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Réseau de neurones : le sujet principal de la photo.
    Subject,
    /// Couleurs : ce qui se distingue du fond (bords de l'image). Idéal pour un logo sur fond uni.
    Color,
}

impl Method {
    pub const ALL: [Method; 2] = [Method::Subject, Method::Color];

    pub fn label(self) -> &'static str {
        match self {
            Method::Subject => "Sujet (IA)",
            Method::Color => "Couleur du fond",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Method::Subject => "Trouve le sujet principal de la photo (bâtiment, objet, personne…)",
            Method::Color => "Garde ce qui se distingue de la couleur des bords : logo ou objet sur fond uni",
        }
    }
}

pub struct Cutout {
    pub width: usize,
    pub height: usize,
    rgb: Vec<[u8; 3]>,
    lab: Vec<[f32; 3]>,
    /// Transparence de l'image d'origine (détourage déjà fait).
    alpha: Option<Vec<u8>>,
    /// Pixel dans le sujet.
    pub mask: Vec<bool>,
    undo: Vec<Vec<bool>>,
    /// Écart de couleur accepté par la baguette magique (distance Lab).
    pub tolerance: f32,
    /// Bouche les trous à l'intérieur du sujet.
    pub fill_holes: bool,
    /// Adoucissement du contour final (0–1).
    pub smooth: f32,
    pub method: Method,
    /// Sensibilité de la détection du sujet (0–1) : plus haut, plus large.
    pub sensitivity: f32,
    /// Cadres tracés autour des objets voulus (vide : toute l'image).
    frames: Vec<Region>,
    /// Carte « sujet » du réseau (calculée une fois par jeu de cadres).
    sal: Option<Vec<f32>>,
    /// Le réseau n'a pas pu tourner.
    pub model_error: Option<String>,
    /// Coins de l'image projetée visible sur la photo (pixels de travail) : le masque est alors
    /// placé par rapport à eux (photo prise de biais) au lieu d'ajuster la photo à la sortie.
    pub projection: Option<[[f32; 2]; 4]>,
}

impl Cutout {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self::new(&load_image(path)?))
    }

    pub fn new(img: &image::DynamicImage) -> Self {
        Self::with_method(img, Method::Subject)
    }

    pub fn with_method(img: &image::DynamicImage, method: Method) -> Self {
        let rgba = img.to_rgba8();
        let (w0, h0) = rgba.dimensions();
        let scale = (WORK_MAX as f32 / w0.max(h0).max(1) as f32).min(1.0);
        let (w, h) = (((w0 as f32 * scale).round() as u32).max(2), ((h0 as f32 * scale).round() as u32).max(2));
        let small = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Triangle);
        let transparent = small.pixels().filter(|p| p[3] < 128).count();
        let n = (w * h) as usize;
        let alpha = (transparent > n / 200 && transparent < n).then(|| small.pixels().map(|p| p[3]).collect());
        // Les pixels semi-transparents sont vus sur fond blanc.
        let rgb: Vec<[u8; 3]> = small
            .pixels()
            .map(|p| {
                let a = p[3] as f32 / 255.0;
                [0, 1, 2].map(|k| (p[k] as f32 * a + 255.0 * (1.0 - a)).round() as u8)
            })
            .collect();
        let lab = rgb.iter().map(|&c| to_lab(c)).collect();
        let mut cut = Self {
            width: w as usize,
            height: h as usize,
            rgb,
            lab,
            alpha,
            mask: vec![false; n],
            undo: Vec::new(),
            tolerance: 18.0,
            fill_holes: false,
            smooth: 0.3,
            method,
            sensitivity: 0.5,
            frames: Vec::new(),
            sal: None,
            model_error: None,
            projection: None,
        };
        cut.mask = cut.compute_auto();
        cut.undo.clear();
        cut
    }

    /// Relance le détourage automatique.
    pub fn auto(&mut self) {
        let mask = self.compute_auto();
        self.set(mask);
    }

    /// Recalcule l'automatique après un réglage (méthode, sensibilité). `checkpoint` : garder
    /// l'état actuel dans l'historique (pas à chaque pas d'un curseur qu'on fait glisser).
    pub fn retune(&mut self, checkpoint: bool) {
        let mask = self.compute_auto();
        if checkpoint {
            self.set(mask);
        } else {
            self.mask = mask;
        }
    }

    fn set(&mut self, mask: Vec<bool>) {
        if mask != self.mask {
            self.undo.push(std::mem::replace(&mut self.mask, mask));
            if self.undo.len() > 50 {
                self.undo.remove(0);
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(m) = self.undo.pop() {
            self.mask = m;
        }
    }

    pub fn invert(&mut self) {
        let mask = self.mask.iter().map(|m| !m).collect();
        self.set(mask);
    }

    pub fn set_fill_holes(&mut self, on: bool) {
        self.fill_holes = on;
        if on {
            let mut mask = self.mask.clone();
            fill_holes(&mut mask, self.width, self.height, None);
            self.set(mask);
        }
    }

    /// Baguette magique : ajoute (ou retire) la zone de couleur proche autour du pixel.
    pub fn wand(&mut self, x: usize, y: usize, add: bool) {
        let (w, h) = (self.width, self.height);
        if x >= w || y >= h {
            return;
        }
        let seed = self.lab[y * w + x];
        let tol2 = self.tolerance * self.tolerance;
        let mut region = vec![false; w * h];
        let mut queue = VecDeque::from([y * w + x]);
        region[y * w + x] = true;
        while let Some(i) = queue.pop_front() {
            let (px, py) = (i % w, i / w);
            let mut visit = |j: usize| {
                if !region[j] && dist2(self.lab[j], seed) <= tol2 {
                    region[j] = true;
                    queue.push_back(j);
                }
            };
            if px > 0 {
                visit(i - 1);
            }
            if px + 1 < w {
                visit(i + 1);
            }
            if py > 0 {
                visit(i - w);
            }
            if py + 1 < h {
                visit(i + w);
            }
        }
        // Les bords de la zone sont souvent un peu en dessous du seuil : on l'élargit d'un pixel.
        let region = dilate(&region, w, h);
        let mut mask = self.mask.clone();
        for (m, r) in mask.iter_mut().zip(&region) {
            if *r {
                *m = add;
            }
        }
        if add && self.fill_holes {
            fill_holes(&mut mask, w, h, None);
        }
        self.set(mask);
    }

    /// Image d'aperçu (RGBA) : le sujet tel quel, le fond assombri et teinté, le contour souligné.
    pub fn preview(&self, accent: [u8; 3]) -> Vec<u8> {
        let (w, h) = (self.width, self.height);
        let mut out = Vec::with_capacity(w * h * 4);
        for (i, (&c, &m)) in self.rgb.iter().zip(&self.mask).enumerate() {
            let (x, y) = (i % w, i / w);
            let edge = [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)]
                .iter()
                .any(|&(ok, j)| ok && self.mask[j] != m);
            let px = if edge {
                accent
            } else if m {
                c
            } else {
                [0, 1, 2].map(|k| (c[k] as f32 * 0.25 + [60.0, 10.0, 40.0][k] * 0.5) as u8)
            };
            out.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        out
    }

    /// Part de l'image dans le sujet (0–1).
    pub fn coverage(&self) -> f32 {
        self.mask.iter().filter(|&&m| m).count() as f32 / self.mask.len().max(1) as f32
    }

    /// Contours du sujet en coordonnées de sortie (photo ajustée à la sortie).
    pub fn rings(&self, aspect: f32) -> Result<(Vec<Vec<Point>>, bool)> {
        if !self.mask.iter().any(|&m| m) {
            bail!("rien n'est sélectionné");
        }
        let (w, h) = (self.width, self.height);
        let mut img: Vec<u8> = self.mask.iter().map(|&m| if m { 255 } else { 0 }).collect();
        let radius = (self.smooth * 4.0).round() as usize;
        if radius > 0 {
            for _ in 0..2 {
                crate::mask::box_blur(&mut img, w, h, radius);
            }
        }
        let field: Vec<f32> = img.iter().map(|&v| v as f32 / 255.0).collect();
        match &self.projection {
            Some(q) => {
                let to_output =
                    crate::surface::quad_to_unit(q).context("coins de l'image projetée mal placés (quadrilatère plat)")?;
                // Les contours sont en pixels ; échantillons au centre des pixels, comme les coins.
                crate::mask::field_to_rings_mapped(&field, w, h, |p| to_output([p[0] + 0.5, p[1] + 0.5]))
            }
            None => crate::mask::field_to_rings(&field, w, h, aspect),
        }
    }

    // -------------------------------------------------------------- automatique

    /// Cadre l'objet voulu (en pixels de l'image de travail) : le détourage automatique ne
    /// cherche plus que dans les cadres. `add` : garde les cadres précédents (plusieurs objets).
    pub fn add_frame(&mut self, region: Region, add: bool) {
        let [x0, y0, x1, y1] = region;
        let (x0, x1) = (x0.min(x1), x0.max(x1).min(self.width));
        let (y0, y1) = (y0.min(y1), y0.max(y1).min(self.height));
        if x1 - x0 < 4 || y1 - y0 < 4 {
            return;
        }
        if !add {
            self.frames.clear();
        }
        // Un peu de contexte autour du cadre aide le réseau à voir le contour de l'objet.
        let (mx, my) = ((x1 - x0) / 20 + 2, (y1 - y0) / 20 + 2);
        self.frames.push([
            x0.saturating_sub(mx),
            y0.saturating_sub(my),
            (x1 + mx).min(self.width),
            (y1 + my).min(self.height),
        ]);
        self.sal = None;
        self.retune(true);
    }

    /// Revient à toute l'image.
    pub fn clear_frames(&mut self) {
        self.frames.clear();
        self.sal = None;
        self.retune(true);
    }

    pub fn frames(&self) -> &[Region] {
        &self.frames
    }

    fn in_frames(&self, i: usize) -> bool {
        let (x, y) = (i % self.width, i / self.width);
        self.frames.iter().any(|&[x0, y0, x1, y1]| x >= x0 && x < x1 && y >= y0 && y < y1)
    }

    /// L'image a sa propre transparence (pas de méthode à choisir).
    pub fn has_alpha(&self) -> bool {
        self.alpha.is_some()
    }

    fn compute_auto(&mut self) -> Vec<bool> {
        let (w, h) = (self.width, self.height);
        if let Some(alpha) = &self.alpha {
            return alpha.iter().map(|&a| a >= 128).collect();
        }
        let n = w * h;
        if self.method == Method::Subject {
            if self.sal.is_none() && self.model_error.is_none() {
                let regions = if self.frames.is_empty() { vec![[0, 0, w, h]] } else { self.frames.clone() };
                let mut acc = vec![0.0f32; n];
                for region in regions {
                    // Un cadre allongé est aussi analysé par morceaux presque carrés (le réseau
                    // travaille sur une image carrée) : un objet fin et long et un plus massif au
                    // bout ressortent chacun dans son morceau.
                    let parts = if self.frames.is_empty() { Vec::new() } else { split_long(region) };
                    for part in std::iter::once(region).chain(parts) {
                        match saliency(&self.rgb, w, h, part) {
                            Ok(sal) => acc.iter_mut().zip(sal).for_each(|(a, v)| *a = a.max(v)),
                            Err(e) => {
                                self.model_error = Some(format!("{e:#}"));
                                break;
                            }
                        }
                    }
                }
                if self.model_error.is_none() {
                    self.sal = Some(acc);
                }
            }
            if let Some(sal) = &self.sal {
                let threshold = 0.85 - 0.7 * self.sensitivity.clamp(0.0, 1.0);
                let mask: Vec<bool> = sal.iter().map(|&p| p >= threshold).collect();
                let mut mask = self.keep_regions(&mask);
                fill_holes(&mut mask, w, h, (!self.fill_holes).then_some(n / 200));
                return mask;
            }
        }
        self.color_mask()
    }

    /// Détourage par les couleurs (fond = couleurs des bords).
    fn color_mask(&self) -> Vec<bool> {
        let (w, h) = (self.width, self.height);
        let n = w * h;

        // Départ : le fond est sur les bords, le sujet vers le centre. Avec des cadres : le fond
        // est hors des cadres, le sujet dedans.
        let band = ((w.min(h) as f32) * 0.04).ceil().max(2.0) as usize;
        let framed = !self.frames.is_empty();
        let in_border = |i: usize| {
            let (x, y) = (i % w, i / w);
            if framed { !self.in_frames(i) } else { x < band || y < band || x + band >= w || y + band >= h }
        };
        let in_center = |i: usize| {
            if framed {
                return self.in_frames(i);
            }
            let (x, y) = ((i % w) as f32 / w as f32 - 0.5, (i / w) as f32 / h as f32 - 0.5);
            (x / 0.3).powi(2) + (y / 0.3).powi(2) <= 1.0
        };
        let mut bg: Vec<[f32; 3]> = sample(&self.lab, |i| in_border(i));
        let bg_model = kmeans(&bg, CLUSTERS);
        // Les couleurs du centre déjà présentes au bord sont plutôt du fond. « Présentes » :
        // aussi proches du modèle que la plupart des pixels du bord eux-mêmes (bruit, dégradés).
        let mut spread: Vec<f32> = bg.iter().map(|&c| nearest(&bg_model, c)).collect();
        spread.sort_by(f32::total_cmp);
        let limit = spread[spread.len() * 95 / 100].max(12.0 * 12.0) * 1.5;
        let mut fg: Vec<[f32; 3]> =
            sample(&self.lab, |i| in_center(i)).into_iter().filter(|c| nearest(&bg_model, *c) > limit).collect();
        if fg.len() < 20 {
            // Centre de la même couleur que le bord : rien à séparer.
            return vec![false; n];
        }

        let mut mask = vec![false; n];
        for _ in 0..4 {
            let bg_model = kmeans(&bg, CLUSTERS);
            let fg_model = kmeans(&fg, CLUSTERS);
            // Probabilité « sujet » : plus près des couleurs du sujet que de celles du fond.
            let mut prob: Vec<u8> = self
                .lab
                .iter()
                .map(|&c| {
                    let (db, df) = (nearest(&bg_model, c).sqrt(), nearest(&fg_model, c).sqrt());
                    (255.0 * db / (db + df + 1e-3)) as u8
                })
                .collect();
            // Lissage : un pixel suit ses voisins (régularisation à bon marché).
            for _ in 0..2 {
                crate::mask::box_blur(&mut prob, w, h, 2);
            }
            let next: Vec<bool> = prob.iter().enumerate().map(|(i, &p)| p >= 128 && (!framed || self.in_frames(i))).collect();
            let mut next = self.keep_regions(&next);
            // Petits trous (bruit, reflets) toujours bouchés ; les grands sur demande.
            fill_holes(&mut next, w, h, (!self.fill_holes).then_some(n / 200));
            if next == mask {
                break;
            }
            mask = next;
            // Modèles réestimés sur les pixels sûrs du découpage obtenu (loin du contour, où
            // les couleurs se mélangent).
            bg = sample(&self.lab, |i| !mask[i] && prob[i] < 40);
            fg = sample(&self.lab, |i| mask[i] && prob[i] > 215);
            if bg.len() < 20 || fg.len() < 20 {
                break;
            }
        }
        mask
    }
}

impl Cutout {
    /// Garde les régions du sujet qui comptent (plus petites acceptées dans des cadres).
    fn keep_regions(&self, mask: &[bool]) -> Vec<bool> {
        let min_share = if self.frames.is_empty() { 100 } else { 1000 };
        keep_main_regions(mask, self.width, self.height, min_share)
    }
}

// ------------------------------------------------------------------ cadre de projection

impl Cutout {
    /// Cherche l'image projetée sur la photo (la grande zone éclairée par le projecteur) et
    /// renvoie ses coins : haut-gauche, haut-droit, bas-droit, bas-gauche (pixels de travail).
    /// Active le calage sur l'image projetée : coins détectés, ou un cadre par défaut à ajuster.
    /// Renvoie vrai si la détection a trouvé l'image projetée.
    pub fn enable_projection(&mut self) -> bool {
        let found = self.detect_projection();
        let (w, h) = (self.width as f32, self.height as f32);
        self.projection = Some(found.unwrap_or([
            [w * 0.2, h * 0.2],
            [w * 0.8, h * 0.2],
            [w * 0.8, h * 0.8],
            [w * 0.2, h * 0.8],
        ]));
        found.is_some()
    }

    pub fn detect_projection(&self) -> Option<[[f32; 2]; 4]> {
        let (w, h) = (self.width, self.height);
        let lum: Vec<f32> = self.lab.iter().map(|c| c[0] * 2.55).collect();
        let threshold = otsu(&lum);
        let bright: Vec<bool> = lum.iter().map(|&l| l > threshold).collect();
        // Plus grande zone éclairée.
        let (pixels, _) = regions(&bright, w, h, true).into_iter().max_by_key(|r| r.0.len())?;
        if pixels.len() < w * h / 50 {
            return None;
        }
        // Contour extérieur → enveloppe convexe (les objets devant le faisceau creusent la zone,
        // l'enveloppe passe par-dessus) → quadrilatère le plus grand inscrit dans l'enveloppe.
        let mut pts: Vec<[i64; 2]> = Vec::new();
        let mut row_ends: std::collections::BTreeMap<usize, (usize, usize)> = Default::default();
        for &i in &pixels {
            let (x, y) = (i % w, i / w);
            let e = row_ends.entry(y).or_insert((x, x));
            e.0 = e.0.min(x);
            e.1 = e.1.max(x);
        }
        for (&y, &(x0, x1)) in &row_ends {
            pts.push([x0 as i64, y as i64]);
            pts.push([x1 as i64 + 1, y as i64]);
            pts.push([x0 as i64, y as i64 + 1]);
            pts.push([x1 as i64 + 1, y as i64 + 1]);
        }
        let hull = convex_hull(pts);
        if hull.len() < 4 {
            return None;
        }
        let quad = largest_quad(&hull);
        Some(order_corners(quad.map(|p| [p[0] as f32, p[1] as f32])))
    }
}

/// Seuil séparant au mieux deux populations (méthode d'Otsu), valeurs sur 0–255.
fn otsu(values: &[f32]) -> f32 {
    let mut hist = [0u64; 256];
    for v in values {
        hist[v.clamp(0.0, 255.0) as usize] += 1;
    }
    let total = values.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, &n)| i as f64 * n as f64).sum();
    let (mut sum_b, mut w_b, mut best, mut threshold) = (0.0, 0.0, -1.0, 127.5);
    for (i, &n) in hist.iter().enumerate() {
        w_b += n as f64;
        let w_f = total - w_b;
        if w_b == 0.0 || w_f == 0.0 {
            continue;
        }
        sum_b += i as f64 * n as f64;
        let (m_b, m_f) = (sum_b / w_b, (sum - sum_b) / w_f);
        let between = w_b * w_f * (m_b - m_f).powi(2);
        if between > best {
            best = between;
            // Limite haute de la case : ses valeurs restent du côté sombre.
            threshold = i as f32 + 1.0;
        }
    }
    threshold
}

/// Enveloppe convexe (chaîne monotone d'Andrew), sens trigonométrique.
fn convex_hull(mut pts: Vec<[i64; 2]>) -> Vec<[i64; 2]> {
    pts.sort();
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: [i64; 2], a: [i64; 2], b: [i64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut hull: Vec<[i64; 2]> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &[i64; 2]>> =
            if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for &p in iter {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0 {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// Quadrilatère d'aire maximale dont les sommets sont des sommets de l'enveloppe.
fn largest_quad(hull: &[[i64; 2]]) -> [[i64; 2]; 4] {
    // Enveloppe allégée (au plus ~40 sommets) : la recherche exhaustive reste rapide.
    let step = hull.len().div_ceil(40).max(1);
    let h: Vec<[i64; 2]> = hull.iter().step_by(step).copied().collect();
    let n = h.len();
    let area = |q: [[i64; 2]; 4]| {
        let mut s = 0i64;
        for k in 0..4 {
            let (a, b) = (q[k], q[(k + 1) % 4]);
            s += a[0] * b[1] - b[0] * a[1];
        }
        s.abs()
    };
    let mut best = ([h[0], h[0], h[0], h[0]], -1);
    for a in 0..n {
        for b in a + 1..n {
            for c in b + 1..n {
                for d in c + 1..n {
                    let q = [h[a], h[b], h[c], h[d]];
                    let s = area(q);
                    if s > best.1 {
                        best = (q, s);
                    }
                }
            }
        }
    }
    best.0
}

/// Range quatre coins : haut-gauche, haut-droit, bas-droit, bas-gauche.
pub fn order_corners(q: [[f32; 2]; 4]) -> [[f32; 2]; 4] {
    let cx = q.iter().map(|p| p[0]).sum::<f32>() / 4.0;
    let cy = q.iter().map(|p| p[1]).sum::<f32>() / 4.0;
    let mut sorted = q;
    // Par angle autour du centre, en partant du haut-gauche (sens horaire à l'écran).
    sorted.sort_by(|a, b| {
        let angle = |p: &[f32; 2]| {
            let t = (p[1] - cy).atan2(p[0] - cx) + std::f32::consts::PI; // départ : vers la gauche
            t.rem_euclid(std::f32::consts::TAU)
        };
        angle(a).total_cmp(&angle(b))
    });
    sorted
}

// ------------------------------------------------------------------ lecture des images

/// Lit une image, dans le bon sens (orientation EXIF des photos de téléphone). Les HEIC/HEIF
/// (photos d'iPhone) passent par l'outil d'images de macOS ou par ffmpeg.
pub fn load_image(path: &Path) -> Result<image::DynamicImage> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
    if ext == "heic" || ext == "heif" {
        return load_heif(path);
    }
    let read = || -> Result<image::DynamicImage> {
        use image::ImageDecoder as _;
        let mut decoder = image::ImageReader::open(path)?.with_guessed_format()?.into_decoder()?;
        let orientation = decoder.orientation()?;
        let mut img = image::DynamicImage::from_decoder(decoder)?;
        img.apply_orientation(orientation);
        Ok(img)
    };
    read().with_context(|| format!("lecture de {}", path.display()))
}

fn load_heif(path: &Path) -> Result<image::DynamicImage> {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("ozmapping-{}-{n}.png", std::process::id()));
    let converted = |cmd: &mut std::process::Command| {
        let _ = std::fs::remove_file(&tmp);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
            && tmp.exists()
    };
    // macOS sait lire le HEIC nativement ; ffmpeg (≥ 7.1) assemble aussi les photos en tuiles.
    let ok = (cfg!(target_os = "macos")
        && converted(std::process::Command::new("sips").args(["-s", "format", "png"]).arg(path).arg("--out").arg(&tmp)))
        || converted(
            crate::video::tool_command("ffmpeg")
                .args(["-v", "error", "-y", "-i"])
                .arg(path)
                .args(["-frames:v", "1", "-update", "1"])
                .arg(&tmp),
        );
    let result = if ok {
        image::open(&tmp).with_context(|| format!("lecture de {}", path.display()))
    } else {
        Err(anyhow::anyhow!(
            "impossible de lire ce HEIC (il faut ffmpeg 7.1 ou plus récent) : exportez-le en JPEG ou PNG"
        ))
    };
    let _ = std::fs::remove_file(&tmp);
    result
}

// ------------------------------------------------------------------ détection du sujet

/// Réseau U²-Net, version légère (U2NETP, licence Apache 2.0 : assets/models/LICENSE-U2Net.txt) :
/// détecte le sujet principal d'une photo. Exécuté sur le CPU par tract (Rust pur).
const MODEL: &[u8] = include_bytes!("../assets/models/u2netp.onnx");
const MODEL_SIZE: usize = 320;

type SaliencyModel = std::sync::Arc<tract_onnx::prelude::TypedRunnableModel>;

fn model() -> Result<&'static SaliencyModel> {
    use tract_onnx::prelude::*;
    static MODEL_CELL: std::sync::OnceLock<std::result::Result<SaliencyModel, String>> = std::sync::OnceLock::new();
    MODEL_CELL
        .get_or_init(|| {
            let load = || -> TractResult<SaliencyModel> {
                tract_onnx::onnx()
                    .model_for_read(&mut std::io::Cursor::new(MODEL))?
                    .with_input_fact(0, f32::fact([1, 3, MODEL_SIZE, MODEL_SIZE]).into())?
                    .into_optimized()?
                    .into_runnable()
            };
            load().map_err(|e| format!("{e:#}"))
        })
        .as_ref()
        .map_err(|e| anyhow::anyhow!("modèle de détourage : {e}"))
}

/// Zone de l'image en pixels : x0, y0, x1, y1 (bornes exclues à droite et en bas).
pub type Region = [usize; 4];

/// Probabilité « sujet » (0–1) de chaque pixel de l'image `w`×`h`, en ne regardant que la
/// zone `region` (0 en dehors).
fn saliency(rgb: &[[u8; 3]], w: usize, h: usize, region: Region) -> Result<Vec<f32>> {
    use tract_onnx::prelude::*;
    let n = MODEL_SIZE;
    let [rx0, ry0, rx1, ry1] = region;
    let (rw, rh) = ((rx1 - rx0).max(1) as f32, (ry1 - ry0).max(1) as f32);
    // Zone ramenée à 320×320 (bilinéaire), normalisée comme à l'entraînement.
    let sample = |fx: f32, fy: f32, k: usize| -> f32 {
        let (x, y) = ((fx - 0.5).clamp(0.0, (w - 1) as f32), (fy - 0.5).clamp(0.0, (h - 1) as f32));
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let v = |x: usize, y: usize| rgb[y * w + x][k] as f32;
        (v(x0, y0) * (1.0 - tx) + v(x1, y0) * tx) * (1.0 - ty) + (v(x0, y1) * (1.0 - tx) + v(x1, y1) * tx) * ty
    };
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let input = tract_ndarray::Array4::from_shape_fn((1, 3, n, n), |(_, c, y, x)| {
        let v = sample(rx0 as f32 + (x as f32 + 0.5) * rw / n as f32, ry0 as f32 + (y as f32 + 0.5) * rh / n as f32, c);
        (v / 255.0 - MEAN[c]) / STD[c]
    });
    let out = model()?.run(tvec!(Tensor::from(input).into_tvalue()))?;
    let pred = out[0].to_plain_array_view::<f32>()?;
    let pred: Vec<f32> = pred.iter().copied().collect();
    let (lo, hi) = pred.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    // Étalement sur 0–1, sauf quand rien ne ressort (image unie) : le bruit resterait du bruit.
    let (lo, span) = if hi >= 0.3 { (lo, (hi - lo).max(1e-6)) } else { (0.0, 1.0) };
    // Retour à la taille de l'image (bilinéaire).
    let at = |x: usize, y: usize| (pred[y * n + x] - lo) / span;
    Ok((0..w * h)
        .map(|i| {
            let (px, py) = (i % w, i / w);
            if px < rx0 || px >= rx1 || py < ry0 || py >= ry1 {
                return 0.0;
            }
            let x = ((px - rx0) as f32 + 0.5) * n as f32 / rw - 0.5;
            let y = ((py - ry0) as f32 + 0.5) * n as f32 / rh - 0.5;
            let (x, y) = (x.clamp(0.0, (n - 1) as f32), y.clamp(0.0, (n - 1) as f32));
            let (x0, y0) = (x.floor() as usize, y.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
            let (tx, ty) = (x - x0 as f32, y - y0 as f32);
            (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty
        })
        .collect())
}

/// Morceaux presque carrés (qui se chevauchent) d'une zone allongée ; rien si elle est déjà
/// presque carrée.
fn split_long(region: Region) -> Vec<Region> {
    let [x0, y0, x1, y1] = region;
    let (rw, rh) = (x1 - x0, y1 - y0);
    let (long, short) = (rw.max(rh), rw.min(rh).max(1));
    let count = (long as f32 / short as f32).round() as usize;
    if count < 2 {
        return Vec::new();
    }
    // Morceaux un peu plus longs que larges, avec recouvrement.
    let size = (long as f32 / count as f32 * 1.4).min(long as f32) as usize;
    (0..count)
        .map(|k| {
            let start = ((long - size) as f32 * k as f32 / (count - 1) as f32) as usize;
            if rw >= rh { [x0 + start, y0, x0 + start + size, y1] } else { [x0, y0 + start, x1, y0 + start + size] }
        })
        .collect()
}

// ------------------------------------------------------------------ couleurs

/// sRGB → CIELAB (D65) : distances proches de la perception.
fn to_lab(c: [u8; 3]) -> [f32; 3] {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Distance (au carré) à la couleur la plus proche du modèle.
fn nearest(model: &[[f32; 3]], c: [f32; 3]) -> f32 {
    model.iter().map(|&m| dist2(m, c)).fold(f32::MAX, f32::min)
}

/// Au plus ~6000 couleurs choisies régulièrement parmi les pixels retenus.
fn sample(lab: &[[f32; 3]], keep: impl Fn(usize) -> bool) -> Vec<[f32; 3]> {
    let all: Vec<usize> = (0..lab.len()).filter(|&i| keep(i)).collect();
    let step = (all.len() / 6000).max(1);
    all.iter().step_by(step).map(|&i| lab[i]).collect()
}

/// k-moyennes, départ « le plus éloigné d'abord » (déterministe).
fn kmeans(points: &[[f32; 3]], k: usize) -> Vec<[f32; 3]> {
    if points.is_empty() {
        return vec![[0.0; 3]];
    }
    let mut centers = vec![points[0]];
    let mut d: Vec<f32> = points.iter().map(|&p| dist2(p, centers[0])).collect();
    while centers.len() < k {
        let (i, &best) = d.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
        if best < 1.0 {
            break;
        }
        centers.push(points[i]);
        for (dj, &p) in d.iter_mut().zip(points) {
            *dj = dj.min(dist2(p, points[i]));
        }
    }
    for _ in 0..8 {
        let mut sum = vec![[0.0f32; 4]; centers.len()];
        for &p in points {
            let (j, _) = centers
                .iter()
                .enumerate()
                .map(|(j, &c)| (j, dist2(p, c)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            for k in 0..3 {
                sum[j][k] += p[k];
            }
            sum[j][3] += 1.0;
        }
        for (c, s) in centers.iter_mut().zip(&sum) {
            if s[3] > 0.0 {
                *c = [s[0] / s[3], s[1] / s[3], s[2] / s[3]];
            }
        }
    }
    centers
}

// ------------------------------------------------------------------ régions

/// Régions connexes (4-voisinage) d'une valeur donnée : (pixels, touche le bord).
fn regions(mask: &[bool], w: usize, h: usize, value: bool) -> Vec<(Vec<usize>, bool)> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    for start in 0..mask.len() {
        if seen[start] || mask[start] != value {
            continue;
        }
        seen[start] = true;
        let mut pixels = vec![start];
        let mut border = false;
        let mut k = 0;
        while k < pixels.len() {
            let i = pixels[k];
            k += 1;
            let (x, y) = (i % w, i / w);
            border |= x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            for (ok, j) in [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)] {
                if ok && !seen[j] && mask[j] == value {
                    seen[j] = true;
                    pixels.push(j);
                }
            }
        }
        out.push((pixels, border));
    }
    out
}

/// Garde les régions du sujet qui comptent : la plus grande, et celles d'au moins
/// 1/`min_share` de l'image.
fn keep_main_regions(mask: &[bool], w: usize, h: usize, min_share: usize) -> Vec<bool> {
    let regs = regions(mask, w, h, true);
    let biggest = regs.iter().map(|r| r.0.len()).max().unwrap_or(0);
    let min = (mask.len() / min_share).min(biggest);
    let mut out = vec![false; mask.len()];
    for (pixels, _) in regs.iter().filter(|r| r.0.len() >= min) {
        for &i in pixels {
            out[i] = true;
        }
    }
    out
}

/// Bouche les zones de fond entièrement entourées par le sujet (toutes, ou seulement les
/// plus petites que `max_area` pixels).
fn fill_holes(mask: &mut [bool], w: usize, h: usize, max_area: Option<usize>) {
    for (pixels, border) in regions(mask, w, h, false) {
        if !border && max_area.is_none_or(|m| pixels.len() <= m) {
            for i in pixels {
                mask[i] = true;
            }
        }
    }
}

fn dilate(region: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = region.to_vec();
    for (i, &r) in region.iter().enumerate() {
        if r {
            let (x, y) = (i % w, i / w);
            for (ok, j) in [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)] {
                if ok {
                    out[j] = true;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// « Photo » : fond en dégradé bleu-gris bruité, sujet orangé texturé (une arche).
    fn photo() -> image::DynamicImage {
        let mut seed = 3u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 24) as i32 - 128) / 10
        };
        let img = image::RgbaImage::from_fn(320, 200, |x, y| {
            let n = noise();
            let (fx, fy) = (x as f32 - 160.0, y as f32 - 120.0);
            let arch = fx.abs() < 90.0 && fy < 70.0 && (fy > 0.0 || fx * fx + fy * fy < 90.0 * 90.0);
            let door = fx.abs() < 25.0 && fy > 20.0; // ouverture qui donne sur le fond (touche le bas)
            let c = if arch && !door {
                let stripe = if (y / 8) % 2 == 0 { 20 } else { -10 };
                [200 + stripe + n, 120 + n, 60 + n]
            } else {
                [90 + (y as i32 / 6) + n, 110 + n, 150 + n]
            };
            image::Rgba([c[0].clamp(0, 255) as u8, c[1].clamp(0, 255) as u8, c[2].clamp(0, 255) as u8, 255])
        });
        image::DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn subject_is_separated_from_background() {
        let cut = Cutout::with_method(&photo(), Method::Color);
        let at = |x: usize, y: usize| cut.mask[y * cut.width + x];
        assert!(at(160, 80), "dans l'arche");
        assert!(at(100, 140), "pilier gauche");
        assert!(!at(10, 10) && !at(310, 190), "fond");
        assert!(!at(160, 180), "porte ouverte sur le fond");
        let cov = cut.coverage();
        assert!((0.25..0.45).contains(&cov), "{cov}");
        let (rings, _) = cut.rings(1.6).unwrap();
        assert!(!rings.is_empty());
    }

    #[test]
    fn wand_adds_and_removes_and_undo_restores() {
        let mut cut = Cutout::with_method(&photo(), Method::Color);
        let before = cut.mask.clone();
        cut.wand(10, 10, true); // le fond entier
        assert!(cut.coverage() > 0.9, "{}", cut.coverage());
        cut.undo();
        assert_eq!(cut.mask, before);
        cut.wand(160, 80, false);
        assert!(cut.coverage() < 0.2, "{}", cut.coverage());
    }

    #[test]
    fn neural_subject_detection_finds_the_arch() {
        let cut = Cutout::new(&photo());
        assert!(cut.model_error.is_none(), "{:?}", cut.model_error);
        let at = |x: usize, y: usize| cut.mask[y * cut.width + x];
        assert!(at(160, 80), "dans l'arche");
        assert!(!at(10, 10) && !at(310, 10), "ciel");
        let cov = cut.coverage();
        assert!((0.2..0.6).contains(&cov), "{cov}");
    }

    #[test]
    fn frame_restricts_detection_to_the_framed_object() {
        let mut cut = Cutout::new(&photo());
        // Cadre sur le pilier gauche de l'arche uniquement.
        cut.add_frame([60, 110, 140, 195], false);
        assert_eq!(cut.frames().len(), 1);
        let at = |c: &Cutout, x: usize, y: usize| c.mask[y * c.width + x];
        assert!(at(&cut, 100, 150), "pilier encadré");
        assert!(!at(&cut, 220, 150) && !at(&cut, 160, 50), "hors du cadre");
        cut.clear_frames();
        assert!(cut.frames().is_empty());
    }

    #[test]
    fn long_frames_are_split_into_square_parts() {
        assert!(split_long([0, 0, 100, 120]).is_empty());
        let parts = split_long([10, 0, 60, 300]);
        assert_eq!(parts.len(), 6);
        assert_eq!(parts[0][1], 0);
        assert_eq!(parts.last().unwrap()[3], 300);
        assert!(parts.iter().all(|p| p[0] == 10 && p[2] == 60));
    }

    #[test]
    fn projected_area_is_found_and_used_for_placement() {
        // Pièce sombre ; image projetée claire, en perspective (trapèze), sujet sombre dedans.
        let quad = [[60.0f32, 40.0], [260.0, 60.0], [250.0, 170.0], [70.0, 180.0]];
        let inside = |x: f32, y: f32| {
            (0..4).all(|k| {
                let (a, b) = (quad[k], quad[(k + 1) % 4]);
                (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]) >= 0.0
            })
        };
        let img = image::RgbaImage::from_fn(320, 220, |x, y| {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let subject = (140.0..180.0).contains(&fx) && (90.0..150.0).contains(&fy);
            let l = if inside(fx, fy) && !subject { 220 } else { 25 };
            image::Rgba([l, l, (l as f32 * 1.1).min(255.0) as u8, 255])
        });
        let mut cut = Cutout::with_method(&image::DynamicImage::ImageRgba8(img), Method::Color);
        let found = cut.detect_projection().expect("image projetée trouvée");
        for (f, q) in found.iter().zip(&quad) {
            assert!((f[0] - q[0]).abs() < 4.0 && (f[1] - q[1]).abs() < 4.0, "{found:?}");
        }
        // Masque = le sujet sombre, placé par rapport aux coins.
        cut.mask = (0..cut.width * cut.height)
            .map(|i| {
                let (x, y) = ((i % cut.width) as f32 + 0.5, (i / cut.width) as f32 + 0.5);
                (140.0..180.0).contains(&x) && (90.0..150.0).contains(&y)
            })
            .collect();
        cut.smooth = 0.0;
        cut.projection = Some(found);
        let (rings, _) = cut.rings(16.0 / 9.0).unwrap();
        let to_unit = crate::surface::quad_to_unit(&found).unwrap();
        let expect = to_unit([160.0, 120.0]);
        let (cx, cy) = (
            rings[0].iter().map(|p| p[0]).sum::<f32>() / rings[0].len() as f32,
            rings[0].iter().map(|p| p[1]).sum::<f32>() / rings[0].len() as f32,
        );
        assert!((cx as f64 - expect[0]).abs() < 0.03 && (cy as f64 - expect[1]).abs() < 0.03, "{cx},{cy} vs {expect:?}");
    }

    #[test]
    fn heic_photos_are_read() {
        // Petite photo HEIC fabriquée par ffmpeg (si l'encodeur est disponible).
        let dir = std::env::temp_dir().join(format!("oz-heic-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let heic = dir.join("test.heic");
        let made = crate::video::tool_command("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240", "-frames:v", "1", "-c:v", "libx265"])
            .arg(&heic)
            .status()
            .is_ok_and(|s| s.success());
        if made || cfg!(target_os = "macos") {
            if !made {
                // Sans encodeur HEVC dans ffmpeg : macOS en produit un.
                let png = dir.join("test.png");
                image::RgbaImage::from_pixel(320, 240, image::Rgba([200, 30, 30, 255])).save(&png).unwrap();
                std::process::Command::new("sips").args(["-s", "format", "heic"]).arg(&png).arg("--out").arg(&heic).status().unwrap();
            }
            let img = load_image(&heic).unwrap();
            assert_eq!((img.width(), img.height()), (320, 240));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transparency_is_used_directly() {
        let img = image::RgbaImage::from_fn(100, 100, |x, _| image::Rgba([10, 10, 10, if x < 50 { 255 } else { 0 }]));
        let cut = Cutout::new(&image::DynamicImage::ImageRgba8(img));
        assert!((cut.coverage() - 0.5).abs() < 0.02);
    }
}
