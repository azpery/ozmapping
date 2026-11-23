//! Formes de mapping : grille de déformation (warp) et masques polygonaux,
//! avec leurs poignées d'édition egui.
//!
//! Toutes les coordonnées sont normalisées sur la sortie : 0..1, y vers le bas.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use serde::{Deserialize, Serialize};

pub type Point = [f32; 2];

/// Agrandissement uniforme autour de `center` (identique en largeur et en hauteur, en pixels aussi).
fn scale_points<'a>(points: impl Iterator<Item = &'a mut Point>, center: Point, factor: f32) {
    for p in points {
        p[0] = center[0] + (p[0] - center[0]) * factor;
        p[1] = center[1] + (p[1] - center[1]) * factor;
    }
}

pub const MAX_GRID: usize = 8;
/// Nombre total de points d'un masque (tous contours confondus).
pub const MAX_MASK_POINTS: usize = 2000;

// ------------------------------------------------------------------ homographie

type Mat3 = [[f64; 3]; 3];

/// Homographie du carré unité vers le quad (Heckbert) : (u, v, 1) -> (x, y, w).
/// Coins : haut-gauche, haut-droit, bas-droit, bas-gauche.
fn square_to_quad(c: &[Point; 4]) -> Mat3 {
    let [x0, y0] = c[0].map(f64::from);
    let [x1, y1] = c[1].map(f64::from);
    let [x2, y2] = c[2].map(f64::from);
    let [x3, y3] = c[3].map(f64::from);

    let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
    let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
    let det = dx1 * dy2 - dx2 * dy1;
    let (g, h) = if det.abs() > 1e-12 {
        ((dx3 * dy2 - dx2 * dy3) / det, (dx1 * dy3 - dx3 * dy1) / det)
    } else {
        (0.0, 0.0)
    };
    [
        [x1 - x0 + g * x1, x3 - x0 + h * x3, x0],
        [y1 - y0 + g * y1, y3 - y0 + h * y3, y0],
        [g, h, 1.0],
    ]
}

fn invert3(m: Mat3) -> Option<Mat3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let co = [
        [e * i - f * h, -(d * i - f * g), d * h - e * g],
        [-(b * i - c * h), a * i - c * g, -(a * h - b * g)],
        [b * f - c * e, -(a * f - c * d), a * e - b * d],
    ];
    let det = a * co[0][0] + b * co[0][1] + c * co[0][2];
    if det.abs() < 1e-12 {
        return None;
    }
    let mut r = [[0.0; 3]; 3];
    for (row, r_row) in r.iter_mut().enumerate() {
        for (col, v) in r_row.iter_mut().enumerate() {
            *v = co[col][row] / det; // adjugée = transposée des cofacteurs
        }
    }
    Some(r)
}

fn apply(m: &Mat3, p: [f64; 2]) -> [f64; 2] {
    let x = m[0][0] * p[0] + m[0][1] * p[1] + m[0][2];
    let y = m[1][0] * p[0] + m[1][1] * p[1] + m[1][2];
    let w = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
    if w.abs() < 1e-12 { [x, y] } else { [x / w, y / w] }
}

/// Transformation qui ramène le quad `c` (haut-gauche, haut-droit, bas-droit, bas-gauche) sur le
/// carré unité : la perspective d'une photo prise de biais est corrigée.
pub fn quad_to_unit(c: &[Point; 4]) -> Option<impl Fn([f64; 2]) -> [f64; 2] + use<>> {
    let inv = invert3(square_to_quad(c))?;
    Some(move |p| apply(&inv, p))
}

/// Spline de Catmull-Rom entre p1 et p2.
fn catmull_rom(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
    let (t2, t3) = (t * t, t * t * t);
    let f = |a: f64, b: f64, c: f64, d: f64| {
        0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3)
    };
    [f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])]
}

// ------------------------------------------------------------------ grille de déformation

/// Surface déformable : grille de (cols+1)×(rows+1) points, rangés ligne par ligne.
///
/// L'interpolation se fait dans l'espace « redressé » par la perspective des 4 coins :
/// une grille régulière donne donc exactement une projection en perspective, et les
/// points intermédiaires permettent de courber la surface (spline de Catmull-Rom).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Warp {
    pub cols: usize,
    pub rows: usize,
    pub points: Vec<Point>,
    /// Forme de la surface (grille, Bézier, triangle, ellipse) et ses points de contrôle
    /// (autres que la grille) : voir `WarpShape`.
    pub shape: WarpShape,
    pub ctrl: Vec<Point>,
}

impl Default for Warp {
    fn default() -> Self {
        Self::regular(1, 1, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
    }
}

/// Forme d'une surface de mapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarpShape {
    /// Grille de points (courbes lisses entre eux).
    #[default]
    Grid,
    /// 4 coins et 2 poignées par bord (bords en courbes de Bézier) : `ctrl` = coins haut-gauche,
    /// haut-droit, bas-droit, bas-gauche, puis poignées haut (2), droite (2), bas (2), gauche (2).
    Bezier,
    /// 3 coins : sommet, bas-droit, bas-gauche.
    Triangle,
    /// Centre, point du rayon horizontal, point du rayon vertical (l'image entière dans l'ellipse).
    Ellipse,
}

impl WarpShape {
    pub const ALL: [WarpShape; 4] = [WarpShape::Grid, WarpShape::Bezier, WarpShape::Triangle, WarpShape::Ellipse];

    pub fn label(self) -> &'static str {
        match self {
            WarpShape::Grid => "Grille",
            WarpShape::Bezier => "Bézier",
            WarpShape::Triangle => "Triangle",
            WarpShape::Ellipse => "Ellipse",
        }
    }

    fn ctrl_len(self) -> usize {
        match self {
            WarpShape::Grid => 0,
            WarpShape::Bezier => 12,
            WarpShape::Triangle => 3,
            WarpShape::Ellipse => 3,
        }
    }
}

fn lerp2(a: Point, b: Point, t: f32) -> Point {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn cubic(p0: Point, p1: Point, p2: Point, p3: Point, t: f32) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]]
}

/// Cadrage de l'image dans la surface, quand leurs proportions diffèrent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fit {
    /// L'image prend exactement la forme de la surface (déformée si besoin).
    #[default]
    Stretch,
    /// L'image entière, sans déformation : bandes transparentes sur les côtés.
    Contain,
    /// Toute la surface, sans déformation : l'image est rognée.
    Cover,
}

impl Fit {
    pub const ALL: [Fit; 3] = [Fit::Stretch, Fit::Contain, Fit::Cover];

    pub fn label(self) -> &'static str {
        match self {
            Fit::Stretch => "Étirer",
            Fit::Contain => "Ajuster",
            Fit::Cover => "Remplir",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Fit::Stretch => "L'image prend toute la surface, quitte à être déformée",
            Fit::Contain => "L'image entière, à ses proportions : bandes transparentes autour (fit)",
            Fit::Cover => "Toute la surface, aux proportions de l'image : les bords sont rognés (fill)",
        }
    }
}

impl Warp {
    /// Proportions (largeur / hauteur) de la surface sur une sortie de `out` pixels.
    pub fn aspect(&self, out: (f32, f32)) -> f32 {
        if matches!(self.shape, WarpShape::Triangle | WarpShape::Ellipse) {
            // Rectangle englobant.
            let o = self.outline();
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in &o {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
            return (((hi[0] - lo[0]) * out.0) / ((hi[1] - lo[1]) * out.1).max(1e-3)).clamp(0.05, 20.0);
        }
        let [tl, tr, br, bl] = self.corners();
        let dist = |a: Point, b: Point| ((a[0] - b[0]) * out.0).hypot((a[1] - b[1]) * out.1);
        let w = (dist(tl, tr) + dist(bl, br)) / 2.0;
        let h = (dist(tl, bl) + dist(tr, br)) / 2.0;
        (w / h.max(1e-3)).clamp(0.05, 20.0)
    }

    /// Grille régulière (en perspective) entre 4 coins.
    pub fn regular(cols: usize, rows: usize, corners: &[Point; 4]) -> Self {
        let h = square_to_quad(corners);
        let mut points = Vec::with_capacity((cols + 1) * (rows + 1));
        for r in 0..=rows {
            for c in 0..=cols {
                let p = apply(&h, [c as f64 / cols as f64, r as f64 / rows as f64]);
                points.push([p[0] as f32, p[1] as f32]);
            }
        }
        Self { cols, rows, points, shape: WarpShape::Grid, ctrl: Vec::new() }
    }

    fn is_valid(&self) -> bool {
        (1..=MAX_GRID).contains(&self.cols)
            && (1..=MAX_GRID).contains(&self.rows)
            && self.points.len() == (self.cols + 1) * (self.rows + 1)
            && self.ctrl.len() == self.shape.ctrl_len()
    }

    /// Change la forme de la surface en gardant sa place et sa taille.
    pub fn set_shape(&mut self, shape: WarpShape) {
        if shape == self.shape {
            return;
        }
        // Triangle ou ellipse : on part de leur rectangle englobant (leurs « coins » n'en sont pas).
        let [tl, tr, br, bl] = if matches!(self.shape, WarpShape::Triangle | WarpShape::Ellipse) {
            let o = self.outline();
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in &o {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
            [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]]
        } else {
            self.corners()
        };
        if shape == WarpShape::Grid {
            let (c, r) = (self.cols, self.rows);
            *self = Self::regular(c, r, &[tl, tr, br, bl]);
            return;
        }
        // Les points de la grille suivent : 4 coins (pour revenir à la grille plus tard).
        let grid = Self::regular(self.cols, self.rows, &[tl, tr, br, bl]);
        self.points = grid.points;
        self.ctrl = match shape {
            WarpShape::Bezier => {
                let third = |a: Point, b: Point, t: f32| lerp2(a, b, t);
                vec![
                    tl, tr, br, bl,
                    third(tl, tr, 1.0 / 3.0), third(tl, tr, 2.0 / 3.0),
                    third(tr, br, 1.0 / 3.0), third(tr, br, 2.0 / 3.0),
                    third(br, bl, 1.0 / 3.0), third(br, bl, 2.0 / 3.0),
                    third(bl, tl, 1.0 / 3.0), third(bl, tl, 2.0 / 3.0),
                ]
            }
            WarpShape::Triangle => vec![lerp2(tl, tr, 0.5), br, bl],
            WarpShape::Ellipse => {
                let c = [(tl[0] + tr[0] + br[0] + bl[0]) / 4.0, (tl[1] + tr[1] + br[1] + bl[1]) / 4.0];
                vec![c, lerp2(tr, br, 0.5), lerp2(bl, br, 0.5)]
            }
            WarpShape::Grid => Vec::new(),
        };
        self.shape = shape;
    }

    /// Répare une grille chargée d'un fichier incohérent.
    pub fn sanitize(&mut self) {
        if !self.is_valid() {
            *self = Self::default();
        }
    }

    pub fn idx(&self, r: usize, c: usize) -> usize {
        r * (self.cols + 1) + c
    }

    /// Coins extérieurs : haut-gauche, haut-droit, bas-droit, bas-gauche.
    pub fn corners(&self) -> [Point; 4] {
        match self.shape {
            WarpShape::Bezier if self.ctrl.len() == 12 => return [self.ctrl[0], self.ctrl[1], self.ctrl[2], self.ctrl[3]],
            WarpShape::Triangle | WarpShape::Ellipse if self.ctrl.len() == 3 => {
                let e = self.evaluator();
                return [e(0.0, 0.0), e(1.0, 0.0), e(1.0, 1.0), e(0.0, 1.0)];
            }
            _ => {}
        }
        [
            self.points[self.idx(0, 0)],
            self.points[self.idx(0, self.cols)],
            self.points[self.idx(self.rows, self.cols)],
            self.points[self.idx(self.rows, 0)],
        ]
    }

    /// Contour extérieur de la surface déformée, sur la sortie (pour tester si un point est dessus).
    pub fn outline(&self) -> Vec<Point> {
        const N: usize = 24;
        let eval = self.evaluator();
        let t = |k: usize| k as f32 / N as f32;
        let mut out = Vec::with_capacity(4 * N);
        out.extend((0..N).map(|k| eval(t(k), 0.0)));
        out.extend((0..N).map(|k| eval(1.0, t(k))));
        out.extend((0..N).map(|k| eval(1.0 - t(k), 1.0)));
        out.extend((0..N).map(|k| eval(0.0, 1.0 - t(k))));
        out
    }

    /// Évalue la surface : (u, v) dans 0..1 → position sur la sortie.
    /// Renvoie aussi un évaluateur réutilisable pour éviter de recalculer l'homographie.
    pub fn evaluator(&self) -> Box<dyn Fn(f32, f32) -> Point> {
        let c = self.ctrl.clone();
        match self.shape {
            WarpShape::Bezier if c.len() == 12 => {
                // Coons : les 4 bords en courbes de Bézier, l'intérieur se remplit entre eux.
                return Box::new(move |u, v| {
                    let top = cubic(c[0], c[4], c[5], c[1], u);
                    let bottom = cubic(c[3], c[9], c[8], c[2], u);
                    let left = cubic(c[0], c[11], c[10], c[3], v);
                    let right = cubic(c[1], c[6], c[7], c[2], v);
                    let bil = |k: usize| {
                        (1.0 - u) * (1.0 - v) * c[0][k] + u * (1.0 - v) * c[1][k] + (1.0 - u) * v * c[3][k] + u * v * c[2][k]
                    };
                    [0, 1].map(|k| (1.0 - v) * top[k] + v * bottom[k] + (1.0 - u) * left[k] + u * right[k] - bil(k))
                });
            }
            WarpShape::Triangle if c.len() == 3 => {
                // Le bord du haut de l'image se resserre jusqu'au sommet.
                return Box::new(move |u, v| lerp2(c[0], lerp2(c[2], c[1], u), v));
            }
            WarpShape::Ellipse if c.len() == 3 => {
                // Carré → disque (sans pli), puis l'ellipse donnée par ses deux rayons.
                return Box::new(move |u, v| {
                    let (x, y) = (2.0 * u - 1.0, 2.0 * v - 1.0);
                    let (dx, dy) = (x * (1.0 - y * y / 2.0).sqrt(), y * (1.0 - x * x / 2.0).sqrt());
                    let a = [c[1][0] - c[0][0], c[1][1] - c[0][1]];
                    let b = [c[2][0] - c[0][0], c[2][1] - c[0][1]];
                    [c[0][0] + a[0] * dx + b[0] * dy, c[0][1] + a[1] * dx + b[1] * dy]
                });
            }
            _ => {}
        }
        Box::new(self.grid_evaluator())
    }

    fn grid_evaluator(&self) -> impl Fn(f32, f32) -> Point + use<> {
        let corners = [
            self.points[self.idx(0, 0)],
            self.points[self.idx(0, self.cols)],
            self.points[self.idx(self.rows, self.cols)],
            self.points[self.idx(self.rows, 0)],
        ];
        let h = square_to_quad(&corners);
        let h_inv = invert3(h).unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        // Points de contrôle dans l'espace redressé.
        let q: Vec<[f64; 2]> = self.points.iter().map(|p| apply(&h_inv, p.map(f64::from))).collect();
        let (cols, rows) = (self.cols as isize, self.rows as isize);
        let at = move |r: isize, c: isize| -> [f64; 2] {
            // En dehors de la grille : extrapolation linéaire.
            let cr = r.clamp(0, rows);
            let cc = c.clamp(0, cols);
            let base = q[(cr * (cols + 1) + cc) as usize];
            if cr == r && cc == c {
                return base;
            }
            let nr = (cr + (cr - r).signum() * if rows > 0 { 1 } else { 0 }).clamp(0, rows);
            let nc = (cc + (cc - c).signum() * if cols > 0 { 1 } else { 0 }).clamp(0, cols);
            let inner = q[(nr * (cols + 1) + nc) as usize];
            [2.0 * base[0] - inner[0], 2.0 * base[1] - inner[1]]
        };
        move |u, v| {
            let su = (u as f64 * cols as f64).clamp(0.0, cols as f64);
            let sv = (v as f64 * rows as f64).clamp(0.0, rows as f64);
            let c = (su.floor() as isize).min(cols - 1);
            let r = (sv.floor() as isize).min(rows - 1);
            let (tu, tv) = (su - c as f64, sv - r as f64);
            let row = |rr: isize| {
                catmull_rom(at(rr, c - 1), at(rr, c), at(rr, c + 1), at(rr, c + 2), tu)
            };
            let p = catmull_rom(row(r - 1), row(r), row(r + 1), row(r + 2), tv);
            let out = apply(&h, p);
            [out[0] as f32, out[1] as f32]
        }
    }

    /// Change la résolution de la grille en conservant la forme actuelle.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.clamp(1, MAX_GRID), rows.clamp(1, MAX_GRID));
        if cols == self.cols && rows == self.rows || self.shape != WarpShape::Grid {
            return;
        }
        let eval = self.evaluator();
        let mut points = Vec::with_capacity((cols + 1) * (rows + 1));
        for r in 0..=rows {
            for c in 0..=cols {
                points.push(eval(c as f32 / cols as f32, r as f32 / rows as f32));
            }
        }
        *self = Self { cols, rows, points, shape: WarpShape::Grid, ctrl: Vec::new() };
    }

    /// Remet la grille en perspective régulière entre ses 4 coins actuels (Bézier : bords droits).
    pub fn flatten(&mut self) {
        match self.shape {
            WarpShape::Grid => *self = Self::regular(self.cols, self.rows, &self.corners()),
            WarpShape::Bezier => {
                self.shape = WarpShape::Grid;
                self.set_shape(WarpShape::Bezier);
            }
            _ => {}
        }
    }

    /// Maillage en triangles : [x, y, u, v] par sommet.
    pub fn mesh(&self) -> Vec<f32> {
        let nu = (16 * self.cols).clamp(24, 128);
        let nv = (16 * self.rows).clamp(24, 128);
        let eval = self.evaluator();
        let mut grid = Vec::with_capacity((nu + 1) * (nv + 1));
        for j in 0..=nv {
            for i in 0..=nu {
                let (u, v) = (i as f32 / nu as f32, j as f32 / nv as f32);
                grid.push((eval(u, v), [u, v]));
            }
        }
        let mut out = Vec::with_capacity(nu * nv * 6 * 4);
        let mut push = |k: usize| {
            let (p, uv) = grid[k];
            out.extend_from_slice(&[p[0], p[1], uv[0], uv[1]]);
        };
        for j in 0..nv {
            for i in 0..nu {
                let a = j * (nu + 1) + i;
                let (b, c, d) = (a + 1, a + nu + 1, a + nu + 2);
                for k in [a, b, d, a, d, c] {
                    push(k);
                }
            }
        }
        out
    }

    /// Centre de la surface (moyenne des 4 coins).
    pub fn center(&self) -> Point {
        let c = self.corners();
        [(c[0][0] + c[1][0] + c[2][0] + c[3][0]) / 4.0, (c[0][1] + c[1][1] + c[2][1] + c[3][1]) / 4.0]
    }

    /// Agrandit (`factor` > 1) ou réduit la surface autour du point `center`.
    pub fn scale(&mut self, center: Point, factor: f32) {
        scale_points(self.points.iter_mut().chain(self.ctrl.iter_mut()), center, factor);
    }

    pub fn translate(&mut self, d: Point) {
        for p in self.points.iter_mut().chain(self.ctrl.iter_mut()) {
            p[0] += d[0];
            p[1] += d[1];
        }
    }
}

// ------------------------------------------------------------------ masques

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskShape {
    Rectangle,
    Circle,
    Triangle,
    Hexagon,
    Star,
}

impl MaskShape {
    pub const ALL: [MaskShape; 5] = [
        MaskShape::Rectangle,
        MaskShape::Circle,
        MaskShape::Triangle,
        MaskShape::Hexagon,
        MaskShape::Star,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MaskShape::Rectangle => "Rectangle",
            MaskShape::Circle => "Cercle",
            MaskShape::Triangle => "Triangle",
            MaskShape::Hexagon => "Hexagone",
            MaskShape::Star => "Étoile",
        }
    }

    /// Polygone centré, `aspect` = largeur/hauteur de la sortie (pour des cercles ronds).
    pub fn points(self, aspect: f32) -> Vec<Point> {
        let radial = |n: usize, radius: &dyn Fn(usize) -> f32, offset: f32| -> Vec<Point> {
            (0..n)
                .map(|i| {
                    let a = offset + i as f32 / n as f32 * std::f32::consts::TAU;
                    let r = radius(i) * 0.4;
                    [0.5 + a.cos() * r / aspect, 0.5 + a.sin() * r]
                })
                .collect()
        };
        let up = -std::f32::consts::FRAC_PI_2;
        match self {
            MaskShape::Rectangle => vec![[0.2, 0.2], [0.8, 0.2], [0.8, 0.8], [0.2, 0.8]],
            MaskShape::Circle => radial(48, &|_| 1.0, 0.0),
            MaskShape::Triangle => radial(3, &|_| 1.0, up),
            MaskShape::Hexagon => radial(6, &|_| 1.0, 0.0),
            MaskShape::Star => radial(10, &|i| if i % 2 == 0 { 1.0 } else { 0.42 }, up),
        }
    }
}

/// Masque en coordonnées de sortie : seul l'intérieur du calque est visible.
/// Un ou plusieurs contours (formes séparées, trous), par exemple importés d'un SVG.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mask {
    pub enabled: bool,
    pub invert: bool,
    /// Adoucissement des bords, en pixels de sortie.
    pub feather: f32,
    /// Premier contour.
    pub points: Vec<Point>,
    /// Contours supplémentaires.
    pub rings: Vec<Vec<Point>>,
    /// Règle de remplissage « non nul » (celle du SVG d'origine) ; sinon pair-impair :
    /// un contour à l'intérieur d'un autre fait alors un trou.
    pub nonzero: bool,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            enabled: false,
            invert: false,
            feather: 0.0,
            points: MaskShape::Rectangle.points(16.0 / 9.0),
            rings: Vec::new(),
            nonzero: false,
        }
    }
}

impl Mask {
    pub fn is_active(&self) -> bool {
        self.enabled && self.points.len() >= 3
    }

    pub fn sanitize(&mut self) {
        self.rings.retain(|r| r.len() >= 3);
        if self.points.len() < 3 {
            self.points = MaskShape::Rectangle.points(16.0 / 9.0);
        }
    }

    /// Remplace la forme (un seul contour).
    pub fn set_shape(&mut self, points: Vec<Point>) {
        self.points = points;
        self.rings.clear();
        self.nonzero = false;
    }

    /// Remplace la forme par plusieurs contours (le premier doit exister).
    pub fn set_rings(&mut self, mut rings: Vec<Vec<Point>>, nonzero: bool) {
        if rings.is_empty() {
            return;
        }
        self.points = rings.remove(0);
        self.rings = rings;
        self.nonzero = nonzero;
    }

    pub fn ring_count(&self) -> usize {
        1 + self.rings.len()
    }

    pub fn ring(&self, k: usize) -> &[Point] {
        if k == 0 { &self.points } else { &self.rings[k - 1] }
    }

    pub fn ring_mut(&mut self, k: usize) -> &mut Vec<Point> {
        if k == 0 { &mut self.points } else { &mut self.rings[k - 1] }
    }

    pub fn all_rings(&self) -> impl Iterator<Item = &[Point]> {
        (0..self.ring_count()).map(|k| self.ring(k))
    }

    /// Le point (coordonnées de sortie) laisse-t-il passer l'image ? Vrai partout sans masque.
    pub fn contains(&self, p: Point) -> bool {
        if !self.is_active() {
            return true;
        }
        let winding: i32 = self.all_rings().map(|r| winding(r, p)).sum();
        let inside = if self.nonzero { winding != 0 } else { winding % 2 != 0 };
        inside != self.invert
    }

    pub fn total_points(&self) -> usize {
        self.all_rings().map(<[Point]>::len).sum()
    }

    /// Moyenne des points de tous les contours.
    /// Tourne tous les contours de `degrees` autour du centre de la forme.
    /// `aspect` (largeur / hauteur de la sortie) garde les proportions : un carré reste un carré.
    pub fn rotate(&mut self, degrees: f32, aspect: f32) {
        let [cx, cy] = self.centroid();
        let (s, c) = degrees.to_radians().sin_cos();
        let turn = |p: &mut Point| {
            let (x, y) = ((p[0] - cx) * aspect, p[1] - cy);
            *p = [cx + (x * c - y * s) / aspect, cy + x * s + y * c];
        };
        self.points.iter_mut().for_each(turn);
        self.rings.iter_mut().flatten().for_each(turn);
    }

    /// Agrandit (`factor` > 1) ou réduit tous les contours autour du point `center`.
    pub fn scale(&mut self, center: Point, factor: f32) {
        scale_points(self.points.iter_mut().chain(self.rings.iter_mut().flatten()), center, factor);
    }

    /// Étire tous les contours autour de `pivot` : `sx` en largeur, `sy` en hauteur.
    pub fn scale_xy(&mut self, pivot: Point, sx: f32, sy: f32) {
        for p in self.points.iter_mut().chain(self.rings.iter_mut().flatten()) {
            *p = [pivot[0] + (p[0] - pivot[0]) * sx, pivot[1] + (p[1] - pivot[1]) * sy];
        }
    }

    /// Tourne tous les contours autour de `pivot` (`aspect` : largeur / hauteur de la sortie).
    pub fn rotate_around(&mut self, pivot: Point, degrees: f32, aspect: f32) {
        let (s, c) = degrees.to_radians().sin_cos();
        let [cx, cy] = pivot;
        for p in self.points.iter_mut().chain(self.rings.iter_mut().flatten()) {
            let (x, y) = ((p[0] - cx) * aspect, p[1] - cy);
            *p = [cx + (x * c - y * s) / aspect, cy + x * s + y * c];
        }
    }

    /// Coins du rectangle qui englobe tous les contours (sortie, 0-1).
    pub fn bounds(&self) -> Option<(Point, Point)> {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in self.all_rings().flatten() {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        (lo[0] <= hi[0]).then_some((lo, hi))
    }

    pub fn centroid(&self) -> Point {
        let n = self.total_points().max(1) as f32;
        self.all_rings().flatten().fold([0.0, 0.0], |acc, p| [acc[0] + p[0] / n, acc[1] + p[1] / n])
    }
}

// ------------------------------------------------------------------ édition

pub const ACCENT: Color32 = Color32::from_rgb(80, 255, 160);
const MASK_COLOR: Color32 = Color32::from_rgb(255, 170, 60);

fn to_screen(rect: Rect, p: Point) -> Pos2 {
    Pos2::new(rect.min.x + p[0] * rect.width(), rect.min.y + p[1] * rect.height())
}

fn drag_factor(ui: &egui::Ui) -> f32 {
    if ui.input(|i| i.modifiers.shift) { 0.2 } else { 1.0 }
}

/// Une poignée déplaçable. Renvoie la réponse egui et le déplacement en coordonnées normalisées.
fn handle(ui: &mut egui::Ui, rect: Rect, id: egui::Id, pos: Pos2, size: f32) -> (egui::Response, Point) {
    let resp = ui
        .interact(Rect::from_center_size(pos, Vec2::splat(size)), id, Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Grab);
    let mut d = [0.0, 0.0];
    if resp.dragged() {
        let delta = resp.drag_delta() * drag_factor(ui);
        d = [delta.x / rect.width().max(1.0), delta.y / rect.height().max(1.0)];
    }
    (resp, d)
}

fn draw_point(ui: &egui::Ui, pos: Pos2, color: Color32, active: bool, big: bool, label: Option<String>) {
    let p = ui.painter();
    let r = match (big, active) {
        (true, true) => 9.0,
        (true, false) => 7.0,
        (false, true) => 6.5,
        (false, false) => 4.5,
    };
    p.circle(
        pos,
        r,
        if active { color } else { Color32::from_black_alpha(170) },
        Stroke::new(1.5, color),
    );
    if let Some(text) = label {
        p.text(
            pos,
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(10.0),
            if active { Color32::BLACK } else { color },
        );
    }
}

fn center_handle(ui: &mut egui::Ui, rect: Rect, id: egui::Id, center: Pos2, color: Color32) -> Point {
    let (resp, d) = handle(ui, rect, id, center, 26.0);
    let resp = resp.on_hover_cursor(egui::CursorIcon::Move);
    let stroke = Stroke::new(if resp.hovered() { 2.5 } else { 1.5 }, color);
    let p = ui.painter();
    p.line_segment([center - Vec2::X * 9.0, center + Vec2::X * 9.0], stroke);
    p.line_segment([center - Vec2::Y * 9.0, center + Vec2::Y * 9.0], stroke);
    d
}

/// Contour fin d'une surface (pour visualiser les calques non sélectionnés).
pub fn draw_warp_outline(ui: &egui::Ui, rect: Rect, warp: &Warp, color: Color32) {
    let eval = warp.evaluator();
    let n = 24;
    let mut pts = Vec::with_capacity(4 * n);
    for i in 0..n {
        pts.push(eval(i as f32 / n as f32, 0.0));
    }
    for i in 0..n {
        pts.push(eval(1.0, i as f32 / n as f32));
    }
    for i in 0..n {
        pts.push(eval(1.0 - i as f32 / n as f32, 1.0));
    }
    for i in 0..n {
        pts.push(eval(0.0, 1.0 - i as f32 / n as f32));
    }
    let pts = pts.into_iter().map(|p| to_screen(rect, p)).collect();
    ui.painter().add(Shape::closed_line(pts, Stroke::new(1.0, color)));
}

/// Poignées de la grille de déformation.
pub fn edit_warp(ui: &mut egui::Ui, rect: Rect, warp: &mut Warp, salt: &str) {
    let base_id = ui.scope_id().with(("warp", salt));
    if warp.shape != WarpShape::Grid {
        edit_shape(ui, rect, warp, base_id);
        return;
    }

    // Lignes de la grille, échantillonnées pour montrer les courbes.
    {
        let eval = warp.evaluator();
        let steps = 32;
        let stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.8));
        for r in 0..=warp.rows {
            let v = r as f32 / warp.rows as f32;
            let pts = (0..=steps).map(|i| to_screen(rect, eval(i as f32 / steps as f32, v))).collect();
            ui.painter().add(Shape::line(pts, stroke));
        }
        for c in 0..=warp.cols {
            let u = c as f32 / warp.cols as f32;
            let pts = (0..=steps).map(|i| to_screen(rect, eval(u, i as f32 / steps as f32))).collect();
            ui.painter().add(Shape::line(pts, stroke));
        }
    }

    let center = {
        let c = warp.evaluator()(0.5, 0.5);
        to_screen(rect, c)
    };
    let d = center_handle(ui, rect, base_id.with("center"), center, ACCENT);
    if d != [0.0, 0.0] {
        warp.translate(d);
    }

    let corner_idx = {
        let (c, r) = (warp.cols, warp.rows);
        [warp.idx(0, 0), warp.idx(0, c), warp.idx(r, c), warp.idx(r, 0)]
    };
    for i in 0..warp.points.len() {
        let pos = to_screen(rect, warp.points[i]);
        let corner = corner_idx.iter().position(|&k| k == i);
        let (resp, d) = handle(ui, rect, base_id.with(i), pos, if corner.is_some() { 24.0 } else { 18.0 });
        let p = &mut warp.points[i];
        p[0] = (p[0] + d[0]).clamp(-1.0, 2.0);
        p[1] = (p[1] + d[1]).clamp(-1.0, 2.0);
        let active = resp.hovered() || resp.dragged();
        draw_point(ui, pos, ACCENT, active, corner.is_some(), corner.map(|k| (k + 1).to_string()));
    }
}

/// Surfaces Bézier, triangle, ellipse : contour, poignées (coins, tangentes, rayons), croix
/// centrale pour tout déplacer.
fn edit_shape(ui: &mut egui::Ui, rect: Rect, warp: &mut Warp, base_id: egui::Id) {
    let outline: Vec<Pos2> = warp.outline().into_iter().map(|p| to_screen(rect, p)).collect();
    ui.painter().add(Shape::closed_line(outline, Stroke::new(1.5, ACCENT)));
    // Quelques lignes intérieures, pour voir la déformation de l'image.
    {
        let eval = warp.evaluator();
        let stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.35));
        for k in 1..4 {
            let t = k as f32 / 4.0;
            let h: Vec<Pos2> = (0..=24).map(|i| to_screen(rect, eval(i as f32 / 24.0, t))).collect();
            let v: Vec<Pos2> = (0..=24).map(|i| to_screen(rect, eval(t, i as f32 / 24.0))).collect();
            ui.painter().add(Shape::line(h, stroke));
            ui.painter().add(Shape::line(v, stroke));
        }
    }
    let center = to_screen(rect, warp.evaluator()(0.5, 0.5));
    let d = center_handle(ui, rect, base_id.with("center"), center, ACCENT);
    if d != [0.0, 0.0] {
        warp.translate(d);
        return;
    }
    // Bézier : traits des tangentes (coin → poignée).
    if warp.shape == WarpShape::Bezier && warp.ctrl.len() == 12 {
        let c = &warp.ctrl;
        let pairs = [(0, 4), (1, 5), (1, 6), (2, 7), (2, 8), (3, 9), (3, 10), (0, 11)];
        for (a, b) in pairs {
            ui.painter().line_segment([to_screen(rect, c[a]), to_screen(rect, c[b])], Stroke::new(1.0, Color32::from_rgb(120, 200, 255)));
        }
    }
    let n = warp.ctrl.len();
    for i in 0..n {
        let pos = to_screen(rect, warp.ctrl[i]);
        // Coins (Bézier : les 4 premiers ; triangle : tous ; ellipse : le centre) en gros.
        let big = match warp.shape {
            WarpShape::Bezier => i < 4,
            WarpShape::Triangle => true,
            WarpShape::Ellipse => i == 0,
            WarpShape::Grid => false,
        };
        let (resp, d) = handle(ui, rect, base_id.with(("ctrl", i)), pos, if big { 24.0 } else { 16.0 });
        let hint = match (warp.shape, big) {
            (WarpShape::Bezier, true) => "Coin : glisser (ses poignées suivent)",
            (WarpShape::Bezier, false) => "Poignée : courbe le bord",
            (WarpShape::Ellipse, true) => "Centre : déplacer l'ellipse",
            (WarpShape::Ellipse, false) => "Rayon : agrandir, aplatir, tourner",
            _ => "Coin du triangle",
        };
        let resp = resp.on_hover_text(hint);
        if d != [0.0, 0.0] {
            let mv = |p: &mut Point| {
                p[0] = (p[0] + d[0]).clamp(-1.0, 2.0);
                p[1] = (p[1] + d[1]).clamp(-1.0, 2.0);
            };
            mv(&mut warp.ctrl[i]);
            // Un coin de Bézier emmène ses deux poignées ; le centre de l'ellipse, ses rayons.
            let followers: &[usize] = match (warp.shape, i) {
                (WarpShape::Bezier, 0) => &[4, 11],
                (WarpShape::Bezier, 1) => &[5, 6],
                (WarpShape::Bezier, 2) => &[7, 8],
                (WarpShape::Bezier, 3) => &[9, 10],
                (WarpShape::Ellipse, 0) => &[1, 2],
                _ => &[],
            };
            for &f in followers {
                mv(&mut warp.ctrl[f]);
            }
        }
        let active = resp.hovered() || resp.dragged();
        let color = if big { ACCENT } else { Color32::from_rgb(120, 200, 255) };
        draw_point(ui, pos, color, active, big, None);
    }
}

/// Édition de l'ensemble média + masque : un cadre autour des deux.
/// Glisser un coin = redimensionner (autour du coin opposé ; Alt : autour du centre),
/// glisser la croix = déplacer. Le masque reste à la même place sur le média.
pub fn edit_both(ui: &mut egui::Ui, rect: Rect, warp: &mut Warp, mask: &mut Mask, salt: &str) {
    let base_id = ui.scope_id().with(("both", salt));
    let frame_color = Color32::from_rgb(120, 200, 255);
    draw_warp_outline(ui, rect, warp, ACCENT);
    if mask.enabled {
        for ring in mask.all_rings() {
            let pts = ring.iter().map(|p| to_screen(rect, *p)).collect();
            ui.painter().add(Shape::closed_line(pts, Stroke::new(1.5, MASK_COLOR)));
        }
    }

    // Cadre englobant (coordonnées de sortie, 0..1) : au plus près de la découpe quand un masque
    // la détoure (photo détourée : le cadre suit l'objet, pas toute la photo) ; sinon tout le média.
    let outline = warp.outline();
    let mask_pts: Vec<Point> = if mask.enabled { mask.all_rings().flatten().copied().collect() } else { Vec::new() };
    let tight = mask.enabled && !mask.invert && !mask_pts.is_empty();
    let framed: Vec<Point> = if tight { mask_pts.clone() } else { outline.iter().chain(mask_pts.iter()).copied().collect() };
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in framed.iter() {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    if lo[0] > hi[0] {
        return;
    }
    let bb = Rect::from_min_max(to_screen(rect, lo), to_screen(rect, hi));
    ui.painter().rect_stroke(bb, 0.0, Stroke::new(1.5, frame_color), egui::StrokeKind::Middle);

    let to_norm = |p: Pos2| -> Point { [(p.x - rect.min.x) / rect.width().max(1.0), (p.y - rect.min.y) / rect.height().max(1.0)] };
    let mut scale = |pivot: Point, factor: f32| {
        warp.scale(pivot, factor);
        mask.scale(pivot, factor);
    };

    let d = center_handle(ui, rect, base_id.with("center"), bb.center(), frame_color);
    if d != [0.0, 0.0] {
        warp.translate(d);
        for k in 0..mask.ring_count() {
            for p in mask.ring_mut(k) {
                p[0] += d[0];
                p[1] += d[1];
            }
        }
        return;
    }

    let corners = [bb.left_top(), bb.right_top(), bb.right_bottom(), bb.left_bottom()];
    for (k, &pos) in corners.iter().enumerate() {
        let (resp, d) = handle(ui, rect, base_id.with(("corner", k)), pos, 22.0);
        let cursor = if k % 2 == 0 { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw };
        let resp = resp.on_hover_cursor(cursor).on_hover_text("Glisser : redimensionner le média et le masque · Alt : depuis le centre");
        let active = resp.hovered() || resp.dragged();
        let side = if active { 12.0 } else { 9.0 };
        ui.painter().rect(
            Rect::from_center_size(pos, Vec2::splat(side)),
            1.0,
            if active { frame_color } else { Color32::from_black_alpha(170) },
            Stroke::new(1.5, frame_color),
            egui::StrokeKind::Middle,
        );
        if d == [0.0, 0.0] {
            continue;
        }
        // Agrandissement uniforme : la projection du déplacement sur la diagonale (en pixels).
        let pivot_px = if ui.input(|i| i.modifiers.alt) { bb.center() } else { corners[(k + 2) % 4] };
        let from = pos - pivot_px;
        let to = from + Vec2::new(d[0] * rect.width(), d[1] * rect.height());
        let factor = (to.dot(from) / from.length_sq().max(1.0)).max(0.05);
        // Pas plus petit qu'une vingtaine de pixels.
        let factor = if from.length() * factor < 20.0 { 1.0 } else { factor };
        if factor != 1.0 {
            scale(to_norm(pivot_px), factor);
        }
    }
}

/// Poignées du masque : glisser = déplacer, double-clic sur un bord = ajouter un point,
/// clic droit sur un point = le supprimer. Tous les contours sont éditables.
pub fn edit_mask(ui: &mut egui::Ui, rect: Rect, mask: &mut Mask, salt: &str) {
    let base_id = ui.scope_id().with(("mask", salt));
    let screen: Vec<Vec<Pos2>> = mask.all_rings().map(|r| r.iter().map(|p| to_screen(rect, *p)).collect()).collect();
    for ring in &screen {
        ui.painter().add(Shape::closed_line(ring.clone(), Stroke::new(1.5, MASK_COLOR)));
    }
    let total = mask.total_points();
    // Beaucoup de points (SVG importé) : poignées plus petites pour rester lisible.
    let dense = total > 80;

    // Double-clic près d'un bord : insertion d'un point.
    let edge_hit = ui.interact(rect, base_id.with("edges"), Sense::click());
    if edge_hit.double_clicked()
        && total < MAX_MASK_POINTS
        && let Some(pointer) = edge_hit.interact_pointer_pos()
    {
        let mut best = (f32::MAX, 0, 0, Pos2::ZERO);
        for (k, pts) in screen.iter().enumerate() {
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                let ab = b - a;
                let t = ((pointer - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
                let proj = a + ab * t;
                let dist = proj.distance(pointer);
                if dist < best.0 {
                    best = (dist, k, i, proj);
                }
            }
        }
        if best.0 < 20.0 {
            let p = best.3;
            let n = [(p.x - rect.min.x) / rect.width(), (p.y - rect.min.y) / rect.height()];
            mask.ring_mut(best.1).insert(best.2 + 1, n);
            return;
        }
    }

    let centroid = mask.centroid();
    let d = center_handle(ui, rect, base_id.with("center"), to_screen(rect, centroid), MASK_COLOR);
    if d != [0.0, 0.0] {
        for k in 0..mask.ring_count() {
            for p in mask.ring_mut(k) {
                p[0] += d[0];
                p[1] += d[1];
            }
        }
    }

    // Points d'abord (chacun se déplace seul), puis le cadre de transformation par-dessus.
    let mut remove = None;
    for k in 0..mask.ring_count() {
        let ring_len = mask.ring(k).len();
        for i in 0..ring_len {
            let pos = to_screen(rect, mask.ring(k)[i]);
            let (resp, d) = handle(ui, rect, base_id.with((k, i)), pos, if dense { 10.0 } else { 16.0 });
            let resp = resp.on_hover_text("Glisser : déplacer · Clic droit : supprimer");
            if resp.secondary_clicked() && ring_len > 3 {
                remove = Some((k, i));
            }
            let p = &mut mask.ring_mut(k)[i];
            p[0] = (p[0] + d[0]).clamp(-0.5, 1.5);
            p[1] = (p[1] + d[1]).clamp(-0.5, 1.5);
            let active = resp.hovered() || resp.dragged();
            if dense && !active {
                ui.painter().circle_filled(pos, 2.5, MASK_COLOR);
            } else {
                draw_point(ui, pos, MASK_COLOR, active, false, None);
            }
        }
    }
    if let Some((k, i)) = remove {
        mask.ring_mut(k).remove(i);
    }
    transform_frame(ui, rect, mask, base_id.with("frame"));
}

/// Cadre de transformation de tout le masque : coins = agrandir ou réduire (proportions gardées ;
/// Maj : librement), milieux des côtés = plus large / plus étroit, plus haut / plus bas, poignée
/// ronde au-dessus = tourner (Maj : par 15°). Alt : depuis le centre.
fn transform_frame(ui: &mut egui::Ui, rect: Rect, mask: &mut Mask, id: egui::Id) {
    let Some((lo, hi)) = mask.bounds() else { return };
    let (a, b) = (to_screen(rect, lo), to_screen(rect, hi));
    // Cadre un peu plus grand que la forme : ses poignées ne recouvrent pas les points.
    const M: f32 = 14.0;
    let frame = Rect::from_min_max(a - Vec2::splat(M), b + Vec2::splat(M));
    let color = Color32::from_rgb(255, 210, 120);
    let painter = ui.painter();
    painter.add(Shape::dashed_line(
        &[frame.left_top(), frame.right_top(), frame.right_bottom(), frame.left_bottom(), frame.left_top()],
        Stroke::new(1.0, color.gamma_multiply(0.8)),
        5.0,
        4.0,
    ));
    let (alt, shift) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift));
    let raw = Rect::from_min_max(a, b);
    let to_norm = |p: Pos2| -> Point { [(p.x - rect.min.x) / rect.width().max(1.0), (p.y - rect.min.y) / rect.height().max(1.0)] };
    // Poignées : (position dans le cadre, axes qu'elle étire, coin correspondant de la forme).
    let handles: [(Pos2, (bool, bool), Pos2); 8] = [
        (frame.left_top(), (true, true), raw.left_top()),
        (frame.right_top(), (true, true), raw.right_top()),
        (frame.right_bottom(), (true, true), raw.right_bottom()),
        (frame.left_bottom(), (true, true), raw.left_bottom()),
        (frame.center_top(), (false, true), raw.center_top()),
        (frame.right_center(), (true, false), raw.right_center()),
        (frame.center_bottom(), (false, true), raw.center_bottom()),
        (frame.left_center(), (true, false), raw.left_center()),
    ];
    for (k, &(pos, (ax, ay), corner)) in handles.iter().enumerate() {
        let (resp, d) = handle(ui, rect, id.with(k), pos, 16.0);
        let cursor = match (ax, ay) {
            (true, false) => egui::CursorIcon::ResizeHorizontal,
            (false, true) => egui::CursorIcon::ResizeVertical,
            _ if k % 2 == 0 => egui::CursorIcon::ResizeNwSe,
            _ => egui::CursorIcon::ResizeNeSw,
        };
        let hint = match (ax, ay) {
            (true, false) => "Glisser : plus large / plus étroit · Alt : depuis le centre",
            (false, true) => "Glisser : plus haut / moins haut · Alt : depuis le centre",
            _ => "Glisser : agrandir / réduire tout le masque · Maj : librement · Alt : depuis le centre",
        };
        let resp = resp.on_hover_cursor(cursor).on_hover_text(hint);
        let active = resp.hovered() || resp.dragged();
        ui.painter().rect(
            Rect::from_center_size(pos, Vec2::splat(if active { 10.0 } else { 8.0 })),
            1.0,
            if active { color } else { Color32::from_black_alpha(170) },
            Stroke::new(1.5, color),
            egui::StrokeKind::Middle,
        );
        if d == [0.0, 0.0] {
            continue;
        }
        // Point fixe : le côté ou le coin opposé (Alt : le centre).
        let pivot = if alt { raw.center() } else { raw.center() + (raw.center() - corner) };
        let from = corner - pivot;
        let delta = Vec2::new(d[0] * rect.width(), d[1] * rect.height());
        let ratio = |num: f32, den: f32| if den.abs() < 4.0 { 1.0 } else { ((den + num) / den).max(0.02) };
        let (mut sx, mut sy) = (if ax { ratio(delta.x, from.x) } else { 1.0 }, if ay { ratio(delta.y, from.y) } else { 1.0 });
        if ax && ay && !shift {
            // Coin : proportions gardées (projection sur la diagonale).
            let f = ((from + delta).dot(from) / from.length_sq().max(1.0)).max(0.02);
            sx = f;
            sy = f;
        }
        // Pas plus petit qu'une dizaine de pixels.
        if raw.width() * sx < 10.0 {
            sx = 1.0;
        }
        if raw.height() * sy < 10.0 {
            sy = 1.0;
        }
        mask.scale_xy(to_norm(pivot), sx, sy);
        return;
    }
    // Rotation : poignée ronde au-dessus du cadre.
    let knob = frame.center_top() - Vec2::new(0.0, 26.0);
    ui.painter().line_segment([frame.center_top(), knob], Stroke::new(1.0, color.gamma_multiply(0.8)));
    let (resp, _) = handle(ui, rect, id.with("rotate"), knob, 18.0);
    let resp = resp.on_hover_cursor(egui::CursorIcon::Alias).on_hover_text("Glisser : tourner tout le masque · Maj : par pas de 15°");
    let active = resp.hovered() || resp.dragged();
    ui.painter().circle(knob, if active { 7.0 } else { 5.5 }, if active { color } else { Color32::from_black_alpha(170) }, Stroke::new(1.5, color));
    if resp.dragged() {
        if let Some(p) = resp.interact_pointer_pos() {
            let c = raw.center();
            let prev = p - resp.drag_delta();
            let a0 = (prev - c).angle();
            let a1 = (p - c).angle();
            let mut deg = (a1 - a0).to_degrees();
            if shift {
                // Par pas de 15° : on accumule le glissement jusqu'au pas suivant.
                let acc_id = id.with("acc");
                let acc: f32 = ui.data(|d| d.get_temp(acc_id)).unwrap_or(0.0) + deg;
                let steps = (acc / 15.0).trunc();
                deg = steps * 15.0;
                ui.data_mut(|d| d.insert_temp(acc_id, acc - deg));
            }
            if deg != 0.0 {
                let aspect = rect.width() / rect.height().max(1.0);
                mask.rotate_around(to_norm(c), deg, aspect);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn shapes_keep_their_place() {
        let mut w = Warp::regular(1, 1, &[[0.2, 0.2], [0.8, 0.2], [0.8, 0.8], [0.2, 0.8]]);
        w.set_shape(WarpShape::Bezier);
        // Bézier aux poignées droites : la même surface qu'avant.
        let e = w.evaluator();
        assert!(close(e(0.0, 0.0), [0.2, 0.2]) && close(e(1.0, 1.0), [0.8, 0.8]) && close(e(0.5, 0.5), [0.5, 0.5]));
        w.set_shape(WarpShape::Triangle);
        let e = w.evaluator();
        assert!(close(e(0.3, 0.0), [0.5, 0.2]) && close(e(1.0, 1.0), [0.8, 0.8]));
        w.set_shape(WarpShape::Ellipse);
        let e = w.evaluator();
        assert!(close(e(0.5, 0.5), [0.5, 0.5]) && close(e(1.0, 0.5), [0.8, 0.5]));
        w.set_shape(WarpShape::Grid);
        assert!(w.points.len() == 4 && w.ctrl.is_empty());
        assert!(!w.mesh().is_empty());
    }

    #[test]
    fn mask_stretches_and_turns_around_a_pivot() {
        let mut m = Mask::default();
        m.set_shape(vec![[0.4, 0.4], [0.6, 0.4], [0.6, 0.6], [0.4, 0.6]]);
        m.scale_xy([0.4, 0.5], 2.0, 1.0);
        let (lo, hi) = m.bounds().unwrap();
        assert!(close(lo, [0.4, 0.4]) && close(hi, [0.8, 0.6]));
        m.rotate_around([0.6, 0.5], 180.0, 1.0);
        let (lo, hi) = m.bounds().unwrap();
        assert!(close(lo, [0.4, 0.4]) && close(hi, [0.8, 0.6]));
    }

    #[test]
    fn single_cell_is_exact_perspective() {
        let corners = [[0.1, 0.2], [0.9, 0.05], [0.8, 0.95], [0.15, 0.7]];
        let warp = Warp::regular(1, 1, &corners);
        let h = square_to_quad(&corners);
        let eval = warp.evaluator();
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.3, 0.7), (0.5, 0.5)] {
            let expected = apply(&h, [u as f64, v as f64]);
            assert!(close(eval(u, v), [expected[0] as f32, expected[1] as f32]));
        }
    }

    #[test]
    fn regular_grid_matches_single_cell_and_resize_keeps_shape() {
        let corners = [[0.1, 0.2], [0.9, 0.05], [0.8, 0.95], [0.15, 0.7]];
        let one = Warp::regular(1, 1, &corners);
        let mut many = Warp::regular(4, 3, &corners);
        for (u, v) in [(0.25, 0.4), (0.9, 0.1), (0.5, 0.5)] {
            assert!(close(one.evaluator()(u, v), many.evaluator()(u, v)));
        }
        many.points[7][0] += 0.05; // courbure
        let before = many.evaluator()(0.33, 0.66);
        let mut resized = many.clone();
        resized.resize(4, 3);
        assert!(close(before, resized.evaluator()(0.33, 0.66)));
        // Les points de contrôle sont bien interpolés.
        let p = many.points[7];
        let (r, c) = (7 / 5, 7 % 5);
        assert!(close(p, many.evaluator()(c as f32 / 4.0, r as f32 / 3.0)));
    }
}

/// Nombre d'enroulement du contour fermé `ring` autour de `p` (0 = dehors).
pub fn winding(ring: &[Point], p: Point) -> i32 {
    let mut w = 0;
    for (k, a) in ring.iter().enumerate() {
        let b = ring[(k + 1) % ring.len()];
        let side = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
        if a[1] <= p[1] {
            if b[1] > p[1] && side > 0.0 {
                w += 1;
            }
        } else if b[1] <= p[1] && side < 0.0 {
            w -= 1;
        }
    }
    w
}

#[cfg(test)]
mod cover_tests {
    use super::*;

    #[test]
    fn mask_contains_follows_shape_and_invert() {
        let mut mask = Mask { enabled: true, ..Mask::default() }; // rectangle 0.2..0.8
        assert!(mask.contains([0.5, 0.5]));
        assert!(!mask.contains([0.1, 0.1]));
        mask.invert = true;
        assert!(!mask.contains([0.5, 0.5]));
        assert!(mask.contains([0.1, 0.1]));
        mask.enabled = false;
        assert!(mask.contains([0.1, 0.1]), "sans masque, tout passe");
    }

    #[test]
    fn default_warp_outline_covers_the_output() {
        let o = Warp::default().outline();
        assert_ne!(winding(&o, [0.01, 0.01]), 0);
        assert_ne!(winding(&o, [0.99, 0.5]), 0);
        assert_eq!(winding(&o, [1.2, 0.5]), 0);
    }
}
