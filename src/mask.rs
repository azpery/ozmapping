//! Masques : import de formes (SVG ou image vectorisée) et rastérisation en texture.
//!
//! Le masque est dessiné sur le CPU (tiny-skia) dans une image en niveaux de gris, une seule
//! fois à chaque modification ; le shader n'a plus qu'à la lire. Le coût de rendu ne dépend
//! donc pas du nombre de points, ce qui permet des formes SVG détaillées.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use usvg::tiny_skia_path::{self as tsp, PathSegment};

use crate::surface::{MAX_MASK_POINTS, Mask, Point};

/// Pas de découpage des courbes (segments par courbe), avant simplification.
const CURVE_STEPS: usize = 16;

// ------------------------------------------------------------------ import SVG

/// Contours extraits d'un SVG, placés au centre de la sortie (80 % de sa hauteur ou largeur),
/// en coordonnées de sortie normalisées. Renvoie aussi la règle « non nul » du SVG.
pub fn import_svg(path: &Path, aspect: f32) -> Result<(Vec<Vec<Point>>, bool)> {
    let data = std::fs::read(path).with_context(|| format!("lecture de {}", path.display()))?;
    svg_to_rings(&data, aspect)
}

pub fn svg_to_rings(data: &[u8], aspect: f32) -> Result<(Vec<Vec<Point>>, bool)> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).context("SVG invalide")?;
    let mut rings: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut nonzero = None;
    collect_group(tree.root(), &mut rings, &mut nonzero);
    rings.retain(|r| r.len() >= 3);
    if rings.is_empty() {
        bail!("aucune forme exploitable dans ce SVG (les textes et images ne sont pas pris en charge)");
    }

    // Placement : centré, contenu dans 80 % de la sortie, proportions conservées.
    let (mut min, mut max) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in rings.iter().flatten() {
        for k in 0..2 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    Ok((limit_points(place(&rings, min, max, 0.8, aspect)), nonzero.unwrap_or(true)))
}

/// Place le cadre `min`–`max` au centre de la sortie, dans `fill` (0–1) de sa largeur ou de sa
/// hauteur, proportions conservées. Renvoie des coordonnées de sortie normalisées.
fn place(rings: &[Vec<[f64; 2]>], min: [f64; 2], max: [f64; 2], fill: f64, aspect: f32) -> Vec<Vec<Point>> {
    let (w, h) = ((max[0] - min[0]).max(1e-9), (max[1] - min[1]).max(1e-9));
    let aspect = aspect.max(0.01) as f64;
    // Espace « sortie » de largeur `aspect` et de hauteur 1.
    let scale = (fill * aspect / w).min(fill / h);
    let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    rings
        .iter()
        .map(|r| {
            r.iter()
                .map(|p| {
                    let x = aspect / 2.0 + (p[0] - center[0]) * scale;
                    let y = 0.5 + (p[1] - center[1]) * scale;
                    [(x / aspect) as f32, y as f32]
                })
                .collect()
        })
        .collect()
}

fn collect_group(group: &usvg::Group, rings: &mut Vec<Vec<[f64; 2]>>, nonzero: &mut Option<bool>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect_group(g, rings, nonzero),
            usvg::Node::Path(p) if p.is_visible() => {
                let transform = p.abs_transform();
                if let Some(fill) = p.fill() {
                    nonzero.get_or_insert(fill.rule() == usvg::FillRule::NonZero);
                    flatten(p.data(), transform, rings);
                } else if let Some(stroke) = p.stroke() {
                    // Tracé seul (pictogramme au trait) : on prend la surface du trait.
                    if let Some(outline) = p.data().stroke(&stroke.to_tiny_skia(), 1.0) {
                        nonzero.get_or_insert(true);
                        flatten(&outline, transform, rings);
                    }
                }
            }
            _ => {} // textes, images : ignorés
        }
    }
}

/// Découpe un chemin en contours fermés (courbes approchées par des segments).
fn flatten(path: &tsp::Path, transform: tsp::Transform, rings: &mut Vec<Vec<[f64; 2]>>) {
    let map = |p: tsp::Point| {
        let mut q = p;
        transform.map_point(&mut q);
        [q.x as f64, q.y as f64]
    };
    let mut current: Vec<[f64; 2]> = Vec::new();
    let mut last = tsp::Point::zero();
    let finish = |ring: &mut Vec<[f64; 2]>, rings: &mut Vec<Vec<[f64; 2]>>| {
        if ring.len() >= 3 {
            rings.push(std::mem::take(ring));
        } else {
            ring.clear();
        }
    };
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                finish(&mut current, rings);
                current.push(map(p));
                last = p;
            }
            PathSegment::LineTo(p) => {
                current.push(map(p));
                last = p;
            }
            PathSegment::QuadTo(c, p) => {
                for i in 1..=CURVE_STEPS {
                    let t = i as f32 / CURVE_STEPS as f32;
                    let u = 1.0 - t;
                    let q = tsp::Point::from_xy(
                        u * u * last.x + 2.0 * u * t * c.x + t * t * p.x,
                        u * u * last.y + 2.0 * u * t * c.y + t * t * p.y,
                    );
                    current.push(map(q));
                }
                last = p;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                for i in 1..=CURVE_STEPS {
                    let t = i as f32 / CURVE_STEPS as f32;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    let q = tsp::Point::from_xy(
                        a * last.x + b * c1.x + c * c2.x + d * p.x,
                        a * last.y + b * c1.y + c * c2.y + d * p.y,
                    );
                    current.push(map(q));
                }
                last = p;
            }
            PathSegment::Close => finish(&mut current, rings),
        }
    }
    finish(&mut current, rings);
}

// ------------------------------------------------------------------ import d'image

/// Extensions d'images acceptées pour créer un masque.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"];

/// Taille maximale (plus grand côté, en pixels) de l'image analysée.
const TRACE_MAX: u32 = 720;

/// Contours de la forme d'une image. L'image entière est ajustée à la sortie (centrée,
/// proportions conservées) : une image aux proportions de la sortie s'y superpose exactement.
pub fn import_image(path: &Path, aspect: f32) -> Result<(Vec<Vec<Point>>, bool)> {
    let img = image::open(path).with_context(|| format!("lecture de {}", path.display()))?;
    image_to_rings(&img, aspect)
}

/// La forme est déduite de la transparence si l'image en a, sinon de ce qui se détache du
/// fond (couleur des bords) : un logo noir sur blanc ou blanc sur noir donne le logo.
pub fn image_to_rings(img: &image::DynamicImage, aspect: f32) -> Result<(Vec<Vec<Point>>, bool)> {
    let rgba = img.to_rgba8();
    let (w0, h0) = rgba.dimensions();
    if w0 < 2 || h0 < 2 {
        bail!("image trop petite");
    }
    let scale = (TRACE_MAX as f32 / w0.max(h0) as f32).min(1.0);
    let rgba = if scale < 1.0 {
        let (w, h) = (((w0 as f32 * scale).round() as u32).max(2), ((h0 as f32 * scale).round() as u32).max(2));
        image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Triangle)
    } else {
        rgba
    };
    let (w, h) = rgba.dimensions();
    let field = coverage(&rgba);

    // Les petites taches (bruit, poussières) sont ignorées.
    let min_area = (w as f64 * h as f64 * 0.0002).max(6.0);
    let rings: Vec<Vec<[f64; 2]>> =
        trace(&field, w as usize, h as usize).into_iter().filter(|r| ring_area(r).abs() >= min_area).collect();
    if rings.is_empty() {
        bail!("aucune forme détectée dans cette image (il faut un sujet qui se détache du fond)");
    }
    // Règle pair-impair : les contours intérieurs (trous) se découpent d'eux-mêmes.
    Ok((limit_points(place(&rings, [0.0, 0.0], [w as f64, h as f64], 1.0, aspect)), false))
}

/// Appartenance de chaque pixel à la forme (0–1, seuil à 0,5).
fn coverage(img: &image::RgbaImage) -> Vec<f32> {
    let total = img.pixels().len();
    let transparent = img.pixels().filter(|p| p[3] < 128).count();
    if transparent > total / 200 && transparent < total {
        return img.pixels().map(|p| p[3] as f32 / 255.0).collect();
    }

    // Luminance (sur fond blanc pour les pixels semi-transparents), seuil d'Otsu.
    let lum: Vec<f32> = img
        .pixels()
        .map(|p| {
            let a = p[3] as f32 / 255.0;
            let l = 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32;
            l * a + 255.0 * (1.0 - a)
        })
        .collect();
    let threshold = otsu(&lum);
    let (w, h) = (img.width() as usize, img.height() as usize);
    let border: Vec<f32> = (0..w)
        .flat_map(|x| [lum[x], lum[(h - 1) * w + x]])
        .chain((0..h).flat_map(|y| [lum[y * w], lum[y * w + w - 1]]))
        .collect();
    let light_background = border.iter().filter(|&&l| l > threshold).count() * 2 >= border.len();
    // Valeur continue autour du seuil : des contours lisses plutôt qu'en escalier.
    let sign = if light_background { 1.0 } else { -1.0 };
    lum.iter().map(|l| (0.5 + sign * (threshold - l) / 64.0).clamp(0.0, 1.0)).collect()
}

/// Seuil séparant au mieux les deux populations de luminance (méthode d'Otsu).
fn otsu(lum: &[f32]) -> f32 {
    let mut hist = [0u64; 256];
    for l in lum {
        hist[l.clamp(0.0, 255.0) as usize] += 1;
    }
    let total = lum.len() as f64;
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
            threshold = i as f32 + 0.5;
        }
    }
    threshold
}

/// Contours fermés de la zone `field >= 0,5` (carrés marchants), en pixels de l'image.
fn trace(field: &[f32], w: usize, h: usize) -> Vec<Vec<[f64; 2]>> {
    const ISO: f32 = 0.5;
    // Bordure vide d'un pixel : tous les contours sont fermés.
    let (gw, gh) = (w + 2, h + 2);
    let v = |x: usize, y: usize| {
        if x == 0 || y == 0 || x > w || y > h { 0.0 } else { field[(y - 1) * w + (x - 1)] }
    };
    // Arêtes : horizontale (x,y)→(x+1,y) = 2·i, verticale (x,y)→(x,y+1) = 2·i + 1.
    let hz = |x: usize, y: usize| 2 * (y * gw + x);
    let vt = |x: usize, y: usize| 2 * (y * gw + x) + 1;
    let point = |edge: usize| -> [f64; 2] {
        let (i, vertical) = (edge / 2, edge % 2 == 1);
        let (x, y) = (i % gw, i / gw);
        let (a, b) = if vertical { (v(x, y), v(x, y + 1)) } else { (v(x, y), v(x + 1, y)) };
        let t = if (b - a).abs() < 1e-6 { 0.5 } else { ((ISO - a) / (b - a)).clamp(0.0, 1.0) } as f64;
        // Échantillon (x, y) = centre du pixel (x−1, y−1).
        let (px, py) = (x as f64 - 0.5, y as f64 - 0.5);
        if vertical { [px, py + t] } else { [px + t, py] }
    };

    let mut segments: Vec<[usize; 2]> = Vec::new();
    for y in 0..gh - 1 {
        for x in 0..gw - 1 {
            let (tl, tr, br, bl) = (v(x, y), v(x + 1, y), v(x + 1, y + 1), v(x, y + 1));
            let case = (tl >= ISO) as u8 * 8 | (tr >= ISO) as u8 * 4 | (br >= ISO) as u8 * 2 | (bl >= ISO) as u8;
            let (top, right, bottom, left) = (hz(x, y), vt(x + 1, y), hz(x, y + 1), vt(x, y));
            let center_in = (tl + tr + br + bl) / 4.0 >= ISO;
            let mut seg = |a, b| segments.push([a, b]);
            match case {
                1 | 14 => seg(left, bottom),
                2 | 13 => seg(bottom, right),
                3 | 12 => seg(left, right),
                4 | 11 => seg(top, right),
                6 | 9 => seg(top, bottom),
                7 | 8 => seg(left, top),
                5 | 10 => {
                    // Point selle : on tranche d'après la valeur au centre.
                    if (case == 5) == center_in {
                        seg(left, top);
                        seg(bottom, right);
                    } else {
                        seg(top, right);
                        seg(left, bottom);
                    }
                }
                _ => {}
            }
        }
    }

    // Chaînage : chaque arête traversée touche exactement deux segments.
    let mut by_edge: std::collections::HashMap<usize, [usize; 2]> = std::collections::HashMap::new();
    for (i, s) in segments.iter().enumerate() {
        for &e in s {
            by_edge.entry(e).and_modify(|p| p[1] = i).or_insert([i, usize::MAX]);
        }
    }
    let mut used = vec![false; segments.len()];
    let mut rings = Vec::new();
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut ring = vec![point(segments[start][0])];
        let (mut seg, mut edge) = (start, segments[start][1]);
        loop {
            ring.push(point(edge));
            let pair = by_edge[&edge];
            let next = if pair[0] == seg { pair[1] } else { pair[0] };
            if next == usize::MAX || used[next] {
                break;
            }
            used[next] = true;
            edge = if segments[next][0] == edge { segments[next][1] } else { segments[next][0] };
            seg = next;
        }
        if ring.len() >= 3 {
            rings.push(ring);
        }
    }
    rings
}

/// Aire signée d'un contour (formule du lacet).
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    (0..n).map(|i| {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        a[0] * b[1] - b[0] * a[1]
    }).sum::<f64>() / 2.0
}

// ------------------------------------------------------------------ simplification

/// Simplifie les contours (Ramer-Douglas-Peucker) jusqu'à tenir dans `MAX_MASK_POINTS`.
fn limit_points(rings: Vec<Vec<Point>>) -> Vec<Vec<Point>> {
    let mut epsilon = 0.0004; // ≈ 0,8 px sur une sortie 1920×1080
    loop {
        let simplified: Vec<Vec<Point>> = rings
            .iter()
            .map(|r| simplify_ring(r, epsilon))
            .filter(|r| r.len() >= 3)
            .collect();
        let total: usize = simplified.iter().map(Vec::len).sum();
        if total <= MAX_MASK_POINTS || epsilon > 0.05 {
            return simplified;
        }
        epsilon *= 1.6;
    }
}

fn simplify_ring(ring: &[Point], epsilon: f32) -> Vec<Point> {
    // Points en double (fin de contour = début) retirés avant simplification.
    let mut pts: Vec<Point> = Vec::with_capacity(ring.len());
    for p in ring {
        if pts.last().is_none_or(|q: &Point| (q[0] - p[0]).abs() > 1e-7 || (q[1] - p[1]).abs() > 1e-7) {
            pts.push(*p);
        }
    }
    while pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 4 {
        return pts;
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    rdp(&pts, 0, pts.len() - 1, epsilon, &mut keep);
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

fn rdp(pts: &[Point], first: usize, last: usize, epsilon: f32, keep: &mut [bool]) {
    if last <= first + 1 {
        return;
    }
    let (a, b) = (pts[first], pts[last]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    let mut best = (0.0f32, first);
    for (i, p) in pts.iter().enumerate().take(last).skip(first + 1) {
        let d = if len < 1e-9 {
            ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt()
        } else {
            ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len
        };
        if d > best.0 {
            best = (d, i);
        }
    }
    if best.0 > epsilon {
        keep[best.1] = true;
        rdp(pts, first, best.1, epsilon, keep);
        rdp(pts, best.1, last, epsilon, keep);
    }
}

// ------------------------------------------------------------------ rastérisation

/// Dessine le masque dans une image en niveaux de gris (`width`×`height`, une ligne après
/// l'autre en partant du haut), avec bords adoucis de `feather_px` pixels de cette image.
pub fn rasterize(mask: &Mask, width: u32, height: u32, feather_px: f32) -> Vec<u8> {
    let (w, h) = (width.max(1), height.max(1));
    let mut out = vec![0u8; (w * h) as usize];
    let mut pb = tiny_skia::PathBuilder::new();
    for ring in mask.all_rings().filter(|r| r.len() >= 3) {
        pb.move_to(ring[0][0] * w as f32, ring[0][1] * h as f32);
        for p in &ring[1..] {
            pb.line_to(p[0] * w as f32, p[1] * h as f32);
        }
        pb.close();
    }
    let (Some(path), Some(mut pixmap)) = (pb.finish(), tiny_skia::Pixmap::new(w, h)) else {
        return out;
    };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::WHITE);
    paint.anti_alias = true;
    let rule = if mask.nonzero { tiny_skia::FillRule::Winding } else { tiny_skia::FillRule::EvenOdd };
    pixmap.fill_path(&path, &paint, rule, tiny_skia::Transform::identity(), None);
    for (o, px) in out.iter_mut().zip(pixmap.data().chunks_exact(4)) {
        *o = px[3];
    }
    // Trois flous « boîte » successifs ≈ flou gaussien.
    let radius = (feather_px / 3.0).round() as usize;
    if radius > 0 {
        for _ in 0..3 {
            box_blur(&mut out, w as usize, h as usize, radius);
        }
    }
    out
}

/// Flou boîte séparable (horizontal puis vertical), en place.
fn box_blur(img: &mut [u8], w: usize, h: usize, r: usize) {
    let mut line = Vec::new();
    let blur_line = |get: &dyn Fn(usize) -> u8, n: usize, line: &mut Vec<u8>| {
        line.clear();
        let window = (2 * r + 1) as u32;
        let at = |i: isize| get(i.clamp(0, n as isize - 1) as usize) as u32;
        let mut sum: u32 = (-(r as isize)..=r as isize).map(at).sum();
        for i in 0..n {
            line.push((sum / window) as u8);
            sum += at(i as isize + r as isize + 1);
            sum -= at(i as isize - r as isize);
        }
    };
    for y in 0..h {
        let row = img[y * w..(y + 1) * w].to_vec();
        blur_line(&|i| row[i], w, &mut line);
        img[y * w..(y + 1) * w].copy_from_slice(&line);
    }
    for x in 0..w {
        let col: Vec<u8> = (0..h).map(|y| img[y * w + x]).collect();
        blur_line(&|i| col[i], h, &mut line);
        for (y, v) in line.iter().enumerate() {
            img[y * w + x] = *v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE_WITH_HOLE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
        <path fill-rule="evenodd" d="M10 10 H90 V90 H10 Z M40 40 V60 H60 V40 Z"/>
    </svg>"#;

    #[test]
    fn svg_square_with_hole_gives_two_centered_rings() {
        let (rings, nonzero) = svg_to_rings(SQUARE_WITH_HOLE.as_bytes(), 16.0 / 9.0).unwrap();
        assert_eq!(rings.len(), 2);
        assert!(!nonzero);
        assert_eq!(rings[0].len(), 4, "{:?}", rings[0]);
        // Centré sur la sortie, 80 % de la hauteur.
        let ys: Vec<f32> = rings[0].iter().map(|p| p[1]).collect();
        let (min, max) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
        assert!((min - 0.1).abs() < 1e-3 && (max - 0.9).abs() < 1e-3, "{min} {max}");
    }

    #[test]
    fn curves_and_strokes_are_converted() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <circle cx="50" cy="50" r="40"/>
            <path d="M10 90 L90 10" stroke="black" stroke-width="6" fill="none"/>
        </svg>"#;
        let (rings, _) = svg_to_rings(svg.as_bytes(), 1.0).unwrap();
        assert!(rings.len() >= 2);
        assert!(rings[0].len() > 8, "le cercle doit garder sa rondeur : {}", rings[0].len());
    }

    #[test]
    fn rasterized_hole_is_transparent() {
        let (rings, nonzero) = svg_to_rings(SQUARE_WITH_HOLE.as_bytes(), 1.0).unwrap();
        let mut mask = Mask::default();
        mask.set_rings(rings, nonzero);
        let img = rasterize(&mask, 100, 100, 0.0);
        let at = |x: usize, y: usize| img[y * 100 + x];
        assert_eq!(at(20, 20), 255, "dans la forme");
        assert_eq!(at(50, 50), 0, "dans le trou");
        assert_eq!(at(2, 2), 0, "hors de la forme");
    }

    #[test]
    fn huge_shapes_are_simplified_under_the_limit() {
        let ring: Vec<Point> = (0..20_000)
            .map(|i| {
                let a = i as f32 / 20_000.0 * std::f32::consts::TAU;
                [0.5 + 0.4 * a.cos() + 0.01 * (a * 300.0).sin(), 0.5 + 0.4 * a.sin()]
            })
            .collect();
        let out = limit_points(vec![ring]);
        assert!(out.iter().map(Vec::len).sum::<usize>() <= MAX_MASK_POINTS);
    }
    /// Image de test : fond clair, anneau sombre (disque percé), et un pixel isolé (bruit).
    fn ring_image(dark_on_light: bool) -> image::DynamicImage {
        let (fg, bg) = if dark_on_light { (20u8, 235u8) } else { (235, 20) };
        let img = image::RgbaImage::from_fn(200, 100, |x, y| {
            let d = ((x as f32 - 100.0).powi(2) + (y as f32 - 50.0).powi(2)).sqrt();
            let on = (15.0..40.0).contains(&d) || (x, y) == (5, 5);
            let l = if on { fg } else { bg };
            image::Rgba([l, l, l, 255])
        });
        image::DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn image_shape_is_traced_with_its_hole() {
        for dark_on_light in [true, false] {
            let (rings, nonzero) = image_to_rings(&ring_image(dark_on_light), 2.0).unwrap();
            assert!(!nonzero);
            assert_eq!(rings.len(), 2, "contour extérieur + trou, sans le pixel isolé");
            let mut mask = Mask::default();
            mask.set_rings(rings, nonzero);
            // Sortie 2:1 comme l'image : superposition exacte.
            let img = rasterize(&mask, 200, 100, 0.0);
            let at = |x: usize, y: usize| img[y * 200 + x];
            assert_eq!(at(100, 50), 0, "trou");
            assert_eq!(at(100, 22), 255, "anneau");
            assert_eq!(at(130, 50), 255, "anneau");
            assert_eq!(at(10, 10), 0, "fond");
            assert_eq!(at(180, 90), 0, "fond");
        }
    }

    #[test]
    fn transparency_defines_the_shape() {
        let img = image::RgbaImage::from_fn(100, 100, |x, y| {
            let inside = (30..70).contains(&x) && (20..80).contains(&y);
            image::Rgba([255, 255, 255, if inside { 255 } else { 0 }])
        });
        let (rings, _) = image_to_rings(&image::DynamicImage::ImageRgba8(img), 1.0).unwrap();
        assert_eq!(rings.len(), 1);
        let xs: Vec<f32> = rings[0].iter().map(|p| p[0]).collect();
        let min = xs.iter().cloned().fold(f32::MAX, f32::min);
        let max = xs.iter().cloned().fold(f32::MIN, f32::max);
        assert!((min - 0.3).abs() < 0.01 && (max - 0.7).abs() < 0.01, "{min} {max}");
        assert!(rings[0].len() <= 10, "un rectangle reste simple : {}", rings[0].len());
    }

    #[test]
    fn plain_image_is_rejected() {
        let img = image::RgbaImage::from_pixel(50, 50, image::Rgba([128, 128, 128, 255]));
        assert!(image_to_rings(&image::DynamicImage::ImageRgba8(img), 1.0).is_err());
    }
}
