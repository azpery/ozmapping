//! Formes de mapping : grille de déformation (warp) et masques polygonaux,
//! avec leurs poignées d'édition egui.
//!
//! Toutes les coordonnées sont normalisées sur la sortie : 0..1, y vers le bas.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use serde::{Deserialize, Serialize};

pub type Point = [f32; 2];

pub const MAX_GRID: usize = 8;
pub const MAX_MASK_POINTS: usize = 64;

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
}

impl Default for Warp {
    fn default() -> Self {
        Self::regular(1, 1, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
    }
}

impl Warp {
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
        Self { cols, rows, points }
    }

    fn is_valid(&self) -> bool {
        (1..=MAX_GRID).contains(&self.cols)
            && (1..=MAX_GRID).contains(&self.rows)
            && self.points.len() == (self.cols + 1) * (self.rows + 1)
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
        [
            self.points[self.idx(0, 0)],
            self.points[self.idx(0, self.cols)],
            self.points[self.idx(self.rows, self.cols)],
            self.points[self.idx(self.rows, 0)],
        ]
    }

    /// Évalue la surface : (u, v) dans 0..1 → position sur la sortie.
    /// Renvoie aussi un évaluateur réutilisable pour éviter de recalculer l'homographie.
    pub fn evaluator(&self) -> impl Fn(f32, f32) -> Point + use<> {
        let h = square_to_quad(&self.corners());
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
        if cols == self.cols && rows == self.rows {
            return;
        }
        let eval = self.evaluator();
        let mut points = Vec::with_capacity((cols + 1) * (rows + 1));
        for r in 0..=rows {
            for c in 0..=cols {
                points.push(eval(c as f32 / cols as f32, r as f32 / rows as f32));
            }
        }
        *self = Self { cols, rows, points };
    }

    /// Remet la grille en perspective régulière entre ses 4 coins actuels.
    pub fn flatten(&mut self) {
        *self = Self::regular(self.cols, self.rows, &self.corners());
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

    pub fn translate(&mut self, d: Point) {
        for p in &mut self.points {
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

/// Masque polygonal en coordonnées de sortie : seul l'intérieur du calque est visible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mask {
    pub enabled: bool,
    pub invert: bool,
    /// Adoucissement des bords, en pixels de sortie.
    pub feather: f32,
    pub points: Vec<Point>,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            enabled: false,
            invert: false,
            feather: 0.0,
            points: MaskShape::Rectangle.points(16.0 / 9.0),
        }
    }
}

impl Mask {
    pub fn is_active(&self) -> bool {
        self.enabled && self.points.len() >= 3
    }

    pub fn sanitize(&mut self) {
        self.points.truncate(MAX_MASK_POINTS);
        if self.points.len() < 3 {
            self.points = MaskShape::Rectangle.points(16.0 / 9.0);
        }
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

/// Poignées du masque : glisser = déplacer, double-clic sur un bord = ajouter un point,
/// clic droit sur un point = le supprimer.
pub fn edit_mask(ui: &mut egui::Ui, rect: Rect, mask: &mut Mask, salt: &str) {
    let base_id = ui.scope_id().with(("mask", salt));
    let pts: Vec<Pos2> = mask.points.iter().map(|p| to_screen(rect, *p)).collect();
    ui.painter().add(Shape::closed_line(pts.clone(), Stroke::new(1.5, MASK_COLOR)));

    // Double-clic près d'un bord : insertion d'un point.
    let edge_hit = ui.interact(rect, base_id.with("edges"), Sense::click());
    if edge_hit.double_clicked()
        && mask.points.len() < MAX_MASK_POINTS
        && let Some(pointer) = edge_hit.interact_pointer_pos()
    {
        let mut best = (f32::MAX, 0, Pos2::ZERO);
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            let ab = b - a;
            let t = ((pointer - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
            let proj = a + ab * t;
            let dist = proj.distance(pointer);
            if dist < best.0 {
                best = (dist, i, proj);
            }
        }
        if best.0 < 20.0 {
            let p = best.2;
            let n = [(p.x - rect.min.x) / rect.width(), (p.y - rect.min.y) / rect.height()];
            mask.points.insert(best.1 + 1, n);
            return;
        }
    }

    let n = mask.points.len() as f32;
    let centroid = mask.points.iter().fold([0.0, 0.0], |acc, p| [acc[0] + p[0] / n, acc[1] + p[1] / n]);
    let d = center_handle(ui, rect, base_id.with("center"), to_screen(rect, centroid), MASK_COLOR);
    if d != [0.0, 0.0] {
        for p in &mut mask.points {
            p[0] += d[0];
            p[1] += d[1];
        }
    }

    let mut remove = None;
    for i in 0..mask.points.len() {
        let pos = to_screen(rect, mask.points[i]);
        let (resp, d) = handle(ui, rect, base_id.with(i), pos, 16.0);
        let resp = resp.on_hover_text("Glisser : déplacer · Clic droit : supprimer");
        if resp.secondary_clicked() && mask.points.len() > 3 {
            remove = Some(i);
        }
        let p = &mut mask.points[i];
        p[0] = (p[0] + d[0]).clamp(-0.5, 1.5);
        p[1] = (p[1] + d[1]).clamp(-0.5, 1.5);
        draw_point(ui, pos, MASK_COLOR, resp.hovered() || resp.dragged(), false, None);
    }
    if let Some(i) = remove {
        mask.points.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
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
