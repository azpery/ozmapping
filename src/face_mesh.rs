//! Tête 3D du visuel « Visage qui parle » : un vrai maillage de tête humaine (MakeHuman, CC0),
//! déformé par des cibles (âge, genre, corpulence, expressions) et dessiné avec ses textures de peau.
//!
//! Les données viennent de `assets/face` (préparées par `scripts/bake-face.py`). À chaque image,
//! on recalcule ici la position des sommets et leurs normales, puis on dessine la tête dans une
//! texture que le shader des générateurs affiche comme n'importe quel autre visuel.

use std::collections::HashMap;

use eframe::glow::{self, HasContext};

use crate::face::{FaceDraw, Look};
use crate::generator::Palette;

const HEAD: &[u8] = include_bytes!("../assets/face/head.bin");
const SKINS: [&[u8]; 6] = [
    include_bytes!("../assets/face/skin_young_female.jpg"),
    include_bytes!("../assets/face/skin_young_male.jpg"),
    include_bytes!("../assets/face/skin_middleage_female.jpg"),
    include_bytes!("../assets/face/skin_middleage_male.jpg"),
    include_bytes!("../assets/face/skin_old_female.jpg"),
    include_bytes!("../assets/face/skin_old_male.jpg"),
];
/// Dans l'ordre du réglage « Yeux » : brun, vert, bleu, gris.
const EYES: [&[u8]; 4] = [
    include_bytes!("../assets/face/eye_brown.png"),
    include_bytes!("../assets/face/eye_green.png"),
    include_bytes!("../assets/face/eye_blue.png"),
    include_bytes!("../assets/face/eye_grey.png"),
];
const TEETH: &[u8] = include_bytes!("../assets/face/teeth.jpg");
const BROWS: &[u8] = include_bytes!("../assets/face/eyebrow001.png");
const LASHES: &[u8] = include_bytes!("../assets/face/eyelashes01.png");

/// Matières, dans l'ordre de dessin : peau, peau unie, yeux, dents, langue, intérieur de la bouche,
/// puis sourcils et cils (transparents, dessinés en dernier).
const MATERIALS: usize = 8;
const OPAQUE: usize = 6;
/// Hauteur de la tête (du menton au sommet du crâne) dans le repère de dessin, et hauteur du menton.
const HEAD_HEIGHT: f32 = 1.16;
const CHIN_Y: f32 = -0.55;
/// Ombre des creux : nombre de passes d'étalement, poids des grandes cavités et des plis fins.
const HOLLOW_SPREAD: usize = 10;
const HOLLOW_WIDE: f32 = 12.0;
const HOLLOW_FINE: f32 = 2.2;
/// Flottants par sommet : position, normale, coordonnées de texture, matière, ombre des creux,
/// position au repos (sans expression, pour y plaquer une photo).
const STRIDE: usize = 12;

struct Vertex {
    pos: u32,
    uv: [f32; 2],
    mat: u8,
}

/// Le maillage tel qu'il est enregistré.
struct Head {
    positions: Vec<[f32; 3]>,
    /// 1 = crâne ou visage : sert à cadrer la tête.
    frame: Vec<u8>,
    verts: Vec<Vertex>,
    /// Triangles de toutes les matières à la suite, et la plage de chacune.
    indices: Vec<u32>,
    ranges: [(usize, usize); MATERIALS],
    /// Positions de l'œil gauche puis du droit.
    eyes: [Vec<u32>; 2],
    morphs: HashMap<String, Vec<(u32, [f32; 3])>>,
    chin: usize,
    flat_uv: [f32; 2],
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Result<&[u8], String> {
        if self.0.len() < n {
            return Err("tête 3D : fichier tronqué".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn vec3(&mut self) -> Result<[f32; 3], String> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
}

impl Head {
    fn parse(data: &[u8]) -> Result<Self, String> {
        let mut r = Reader(data);
        if r.bytes(8)? != b"OZFACE02" {
            return Err("tête 3D : format inconnu".into());
        }
        let (np, nv, nm, chin) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        let flat_uv = [r.f32()?, r.f32()?];
        let positions = (0..np).map(|_| r.vec3()).collect::<Result<Vec<_>, _>>()?;
        let frame = r.bytes(np)?.to_vec();
        let mut verts = Vec::with_capacity(nv);
        for _ in 0..nv {
            verts.push(Vertex { pos: r.u32()?, uv: [r.f32()?, r.f32()?], mat: r.bytes(1)?[0] });
        }
        let mut indices = Vec::new();
        let mut ranges = [(0, 0); MATERIALS];
        for range in &mut ranges {
            let n = r.u32()? as usize;
            *range = (indices.len(), n);
            for _ in 0..n {
                indices.push(r.u32()?);
            }
        }
        let mut eyes = [Vec::new(), Vec::new()];
        for eye in &mut eyes {
            let n = r.u32()? as usize;
            *eye = (0..n).map(|_| r.u32()).collect::<Result<_, _>>()?;
        }
        let mut morphs = HashMap::new();
        for _ in 0..nm {
            let len = r.bytes(1)?[0] as usize;
            let name = String::from_utf8_lossy(r.bytes(len)?).into_owned();
            let n = r.u32()? as usize;
            let mut sparse = Vec::with_capacity(n);
            for _ in 0..n {
                sparse.push((r.u32()?, r.vec3()?));
            }
            morphs.insert(name, sparse);
        }
        let valid = verts.iter().all(|v| (v.pos as usize) < np)
            && indices.iter().all(|i| (*i as usize) < nv)
            && eyes.iter().flatten().all(|i| (*i as usize) < np)
            && morphs.values().flatten().all(|(i, _)| (*i as usize) < np)
            && chin < np;
        if !valid {
            return Err("tête 3D : indices invalides".into());
        }
        Ok(Self { positions, frame, verts, indices, ranges, eyes, morphs, chin, flat_uv })
    }

    fn add(&self, out: &mut [[f32; 3]], name: &str, weight: f32, scale: f32) {
        if weight.abs() < 1e-4 {
            return;
        }
        let Some(morph) = self.morphs.get(name) else {
            return;
        };
        let w = weight * scale;
        for (i, d) in morph {
            let p = &mut out[*i as usize];
            for k in 0..3 {
                p[k] += d[k] * w;
            }
        }
    }
}

/// Poids des cibles qui font le personnage : âge × genre × corpulence.
fn identity_weights(d: &FaceDraw) -> Vec<(String, f32)> {
    let sexes = [("female", 1.0 - d.masc), ("male", d.masc)];
    let ages = [("child", d.kid), ("young", (1.0 - d.kid - d.old).max(0.0)), ("old", d.old)];
    // Corpulence : mince en dessous de 0,3, forte au-dessus.
    let w = d.weight;
    let sizes = if w < 0.3 {
        [("min", (0.3 - w) / 0.3), ("average", w / 0.3), ("max", 0.0)]
    } else {
        [("min", 0.0), ("average", 1.0 - (w - 0.3) / 0.7), ("max", (w - 0.3) / 0.7)]
    };
    let mut out = Vec::new();
    for (sex, ws) in sexes {
        for (age, wa) in ages {
            out.push((format!("caucasian-{sex}-{age}"), ws * wa));
            for (size, wz) in sizes {
                out.push((format!("u-{sex}-{age}-{size}"), ws * wa * wz));
            }
        }
    }
    out
}

/// Poids des cibles d'expression pour cette image.
fn expression_weights(d: &FaceDraw) -> Vec<(&'static str, f32)> {
    let (smile, sad) = (d.smile.max(0.0), (-d.smile).max(0.0));
    let (up, down) = (d.brows.max(0.0), (-d.brows).max(0.0));
    let (wide, round) = (d.wide.max(0.0), (-d.wide).max(0.0));
    // Les lèvres s'arrondissent ou s'étirent surtout quand la bouche articule.
    let shape = 0.35 + 0.65 * d.open;
    vec![
        ("mouth-open", d.open * 0.8),
        // La lèvre du haut se relève un peu en parlant : on voit les dents du haut.
        ("mouth-upward-retraction", d.open * 0.35),
        ("mouth-retraction", wide * shape * 0.6),
        ("mouth-protusion", round * shape * 0.8),
        ("mouth-pursing", round * shape * 0.35),
        ("mouth-corner-puller", smile * 0.85),
        ("mouth-depression", sad * 0.9),
        ("eye-left-slit", smile * 0.3),
        ("eye-right-slit", smile * 0.3),
        // Les paupières suivent le regard : elles s'ouvrent quand il monte, tombent quand il descend.
        ("eye-left-closure", (d.blink + 0.25 * (-d.gaze[1]).max(0.0)).min(1.0)),
        ("eye-right-closure", (d.blink + 0.25 * (-d.gaze[1]).max(0.0)).min(1.0)),
        ("eye-left-opened-up", (0.35 * d.gaze[1].max(0.0) + 0.3 * up + 0.7 * d.wide_eyes).min(1.0)),
        ("eye-right-opened-up", (0.35 * d.gaze[1].max(0.0) + 0.3 * up + 0.7 * d.wide_eyes).min(1.0)),
        ("eyebrows-left-up", up),
        ("eyebrows-right-up", up),
        ("eyebrows-left-down", down),
        ("eyebrows-right-down", down),
    ]
}

/// Repères du personnage `d` dans le repère de dessin : demi-écart des yeux, hauteur des yeux,
/// hauteur de la bouche (pour caler une photo dessus).
pub fn landmarks(d: &FaceDraw) -> Option<[f32; 3]> {
    static POSER: std::sync::OnceLock<Option<std::sync::Mutex<Poser>>> = std::sync::OnceLock::new();
    let poser = POSER.get_or_init(|| Head::parse(HEAD).ok().map(|h| std::sync::Mutex::new(Poser::new(h))));
    let mut poser = poser.as_ref()?.lock().ok()?;
    poser.set_identity(d);
    Some([poser.eye_centers[0][0].abs(), poser.eye_centers[0][1], poser.mouth[1]])
}

/// Calcule les sommets de la tête pour une image.
struct Poser {
    head: Head,
    /// Personnage en cours (âge, genre, corpulence) : positions cadrées, échelle du cadrage,
    /// centre de chaque œil.
    key: Option<[f32; 4]>,
    identity: Vec<[f32; 3]>,
    scale: f32,
    eye_centers: [[f32; 3]; 2],
    /// Positions des dents, et leur centre (bouche fermée) : il situe la bouche pour le maquillage.
    teeth: Vec<u32>,
    mouth: [f32; 3],
    posed: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    /// Ombre des creux par position (1 = à découvert), et de quoi la calculer.
    shade: Vec<f32>,
    hollow: Vec<f32>,
    spread: Vec<f32>,
    links: Vec<f32>,
    buffer: Vec<f32>,
}

impl Poser {
    fn new(head: Head) -> Self {
        let n = head.positions.len();
        let mut teeth: Vec<u32> = head.verts.iter().filter(|v| v.mat == 3).map(|v| v.pos).collect();
        teeth.sort_unstable();
        teeth.dedup();
        let mut buffer = vec![0.0; head.verts.len() * STRIDE];
        for (v, out) in head.verts.iter().zip(buffer.chunks_exact_mut(STRIDE)) {
            out[6] = v.uv[0];
            out[7] = v.uv[1];
            out[8] = v.mat as f32;
        }
        Self {
            head,
            key: None,
            identity: Vec::new(),
            scale: 1.0,
            eye_centers: [[0.0; 3]; 2],
            teeth,
            mouth: [0.0; 3],
            posed: vec![[0.0; 3]; n],
            normals: vec![[0.0; 3]; n],
            shade: vec![1.0; n],
            hollow: vec![0.0; n],
            spread: vec![0.0; n],
            links: vec![0.0; n],
            buffer,
        }
    }

    /// Le personnage : cibles d'âge, de genre et de corpulence, puis cadrage de la tête
    /// (menton et sommet du crâne toujours à la même place, quelle que soit sa taille réelle).
    fn set_identity(&mut self, d: &FaceDraw) {
        let key = [d.old, d.kid, d.masc, d.weight];
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        let head = &self.head;
        let mut p = head.positions.clone();
        for (name, w) in identity_weights(d) {
            head.add(&mut p, &name, w, 1.0);
        }
        let framed = || p.iter().zip(&head.frame).filter(|(_, f)| **f == 1).map(|(q, _)| q);
        let top = framed().map(|q| q[1]).fold(f32::MIN, f32::max);
        let (back, front) = framed().fold((f32::MAX, f32::MIN), |(a, b), q| (a.min(q[2]), b.max(q[2])));
        let chin = p[head.chin][1];
        let scale = HEAD_HEIGHT / (top - chin).max(1e-3);
        let mid = 0.5 * (back + front);
        for q in &mut p {
            *q = [q[0] * scale, (q[1] - chin) * scale + CHIN_Y, (q[2] - mid) * scale];
        }
        for (center, eye) in self.eye_centers.iter_mut().zip(&head.eyes) {
            let mut sum = [0.0f32; 3];
            for i in eye {
                for k in 0..3 {
                    sum[k] += p[*i as usize][k];
                }
            }
            *center = sum.map(|s| s / eye.len().max(1) as f32);
        }
        let mut sum = [0.0f32; 3];
        for i in &self.teeth {
            for k in 0..3 {
                sum[k] += p[*i as usize][k];
            }
        }
        self.mouth = sum.map(|s| s / self.teeth.len().max(1) as f32);
        self.scale = scale;
        self.identity = p;
    }

    /// Sommets de l'image : expression, regard, normales.
    fn pose(&mut self, d: &FaceDraw) -> &[f32] {
        self.set_identity(d);
        self.posed.copy_from_slice(&self.identity);
        for (name, w) in expression_weights(d) {
            self.head.add(&mut self.posed, name, w, self.scale);
        }
        // Les yeux tournent autour de leur centre.
        let (yaw, pitch) = (d.gaze[0] * 0.4, -d.gaze[1] * 0.3);
        let ((sy, cy), (sp, cp)) = (yaw.sin_cos(), pitch.sin_cos());
        for (center, eye) in self.eye_centers.iter().zip(&self.head.eyes) {
            for i in eye {
                let p = &mut self.posed[*i as usize];
                let (x, y, z) = (p[0] - center[0], p[1] - center[1], p[2] - center[2]);
                let (y, z) = (y * cp - z * sp, y * sp + z * cp);
                let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
                *p = [center[0] + x, center[1] + y, center[2] + z];
            }
        }
        // Normales : somme des normales des triangles autour de chaque position.
        self.normals.fill([0.0; 3]);
        for t in self.head.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| self.head.verts[i as usize].pos as usize);
            let (pa, pb, pc) = (self.posed[a], self.posed[b], self.posed[c]);
            let (u, v) = ([pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]], [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]]);
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            for i in [a, b, c] {
                for k in 0..3 {
                    self.normals[i][k] += n[k];
                }
            }
        }
        for n in &mut self.normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
            *n = n.map(|x| x / len);
        }
        self.shade_hollows();
        for (v, out) in self.head.verts.iter().zip(self.buffer.chunks_exact_mut(STRIDE)) {
            out[..3].copy_from_slice(&self.posed[v.pos as usize]);
            out[3..6].copy_from_slice(&self.normals[v.pos as usize]);
            out[9] = self.shade[v.pos as usize];
            out[10..12].copy_from_slice(&self.identity[v.pos as usize][..2]);
        }
        &self.buffer
    }

    /// Ombre des creux (orbites, ailes du nez, commissures, rides) : un point est dans un creux
    /// quand ses voisins s'élèvent au-dessus de son plan tangent. Le creux fin est gardé tel quel,
    /// et étalé de proche en proche pour ombrer aussi les grandes cavités. Recalculée à chaque
    /// image : les plis d'expression s'ombrent quand ils se forment.
    fn shade_hollows(&mut self) {
        let head = &self.head;
        let edges = |f: &mut dyn FnMut(usize, usize)| {
            for t in head.indices.chunks_exact(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| head.verts[i as usize].pos as usize);
                f(a, b);
                f(b, c);
                f(c, a);
            }
        };
        self.hollow.fill(0.0);
        self.links.fill(0.0);
        let (posed, normals, hollow, links) = (&self.posed, &self.normals, &mut self.hollow, &mut self.links);
        edges(&mut |a, b| {
            let d = [posed[b][0] - posed[a][0], posed[b][1] - posed[a][1], posed[b][2] - posed[a][2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-9);
            let rise = |n: [f32; 3]| (n[0] * d[0] + n[1] * d[1] + n[2] * d[2]) / len;
            hollow[a] += rise(normals[a]);
            hollow[b] -= rise(normals[b]);
            links[a] += 1.0;
            links[b] += 1.0;
        });
        for (h, n) in hollow.iter_mut().zip(links.iter()) {
            *h /= n.max(1.0);
        }
        // Étalement : chaque passe moyenne avec les voisins (une arête de plus à chaque fois).
        self.shade.copy_from_slice(&self.hollow);
        for _ in 0..HOLLOW_SPREAD {
            self.spread.fill(0.0);
            let (shade, spread) = (&self.shade, &mut self.spread);
            edges(&mut |a, b| {
                spread[a] += shade[b];
                spread[b] += shade[a];
            });
            for ((s, sum), n) in self.shade.iter_mut().zip(&self.spread).zip(&self.links) {
                *s = 0.5 * *s + 0.5 * sum / n.max(1.0);
            }
        }
        for (s, fine) in self.shade.iter_mut().zip(&self.hollow) {
            *s = (1.0 - HOLLOW_WIDE * s.max(0.0) - HOLLOW_FINE * fine.max(0.0)).clamp(0.15, 1.0);
        }
    }
}

const VERTEX_SHADER: &str = r#"#version 330
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in float a_mat;
layout(location = 4) in float a_shade;
layout(location = 5) in vec2 a_rest;
uniform mat3 u_rot;
uniform vec3 u_view; // échelle en x, en y, hauteur du centre
out vec3 v_pos;
out vec3 v_normal;
out vec3 v_head_normal;
out vec2 v_uv;
out float v_shade;
out vec2 v_rest;
flat out int v_mat;
void main() {
    // La tête tourne autour de la base du cou ; caméra lointaine (peu de perspective).
    vec3 pivot = vec3(0.0, -0.5, -0.05);
    vec3 w = u_rot * (a_pos - pivot) + pivot;
    v_pos = a_pos;
    v_normal = u_rot * a_normal;
    v_head_normal = a_normal;
    v_uv = a_uv;
    v_shade = a_shade;
    v_rest = a_rest;
    v_mat = int(a_mat + 0.5);
    float persp = 3.2 / (3.2 - w.z);
    // Dents et gencives reculées d'un cheveu en profondeur : elles ne percent jamais les joues.
    float depth = -w.z * 0.5 + (v_mat >= 3 && v_mat <= 5 ? 0.02 : 0.0);
    gl_Position = vec4(w.x * u_view.x * persp, (w.y - u_view.z) * u_view.y * persp, depth, 1.0);
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 330
in vec3 v_pos;
in vec3 v_normal;
in vec3 v_head_normal;
in vec2 v_uv;
in float v_shade;
in vec2 v_rest;
flat in int v_mat;
out vec4 out_color;
uniform sampler2D u_skin0;
uniform sampler2D u_skin1;
uniform sampler2D u_skin2;
uniform sampler2D u_skin3;
uniform sampler2D u_skin4;
uniform sampler2D u_skin5;
uniform float u_skin_w[6];
uniform sampler2D u_eye_a;
uniform sampler2D u_eye_b;
uniform float u_eye_mix;
uniform sampler2D u_teeth;
uniform sampler2D u_hairs; // sourcils ou cils, selon la passe
uniform vec2 u_flat_uv;
uniform vec3 u_brow_color;
uniform vec4 u_look; // rendu, visage seul, teint, vieillesse
uniform vec4 u_makeup; // intensité, écart et hauteur des yeux, hauteur de la bouche
uniform sampler2D u_photo;
uniform vec4 u_photo_fit; // milieu des yeux dans la photo (x, y), taille, rotation
uniform vec4 u_photo_use; // présence, largeur / hauteur, étirement vertical, yeux de la photo
uniform vec3 u_photo_tone; // teinte moyenne de la peau sur la photo
uniform vec4 u_anim; // temps, décalage de palette, ouverture de la bouche, paupières (1 = fermées)
uniform vec3 u_pal[4];

const float PI = 3.14159265359;

vec3 palette(float x) {
    return clamp(u_pal[0] + u_pal[1] * cos(2.0 * PI * (u_pal[2] * x + u_pal[3])), 0.0, 1.0);
}
float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float noise(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}

// Peau : mélange des textures selon l'âge et le genre.
vec3 skin_at(vec2 uv) {
    vec3 c = vec3(0.0);
    if (u_skin_w[0] > 0.0) c += u_skin_w[0] * texture(u_skin0, uv).rgb;
    if (u_skin_w[1] > 0.0) c += u_skin_w[1] * texture(u_skin1, uv).rgb;
    if (u_skin_w[2] > 0.0) c += u_skin_w[2] * texture(u_skin2, uv).rgb;
    if (u_skin_w[3] > 0.0) c += u_skin_w[3] * texture(u_skin3, uv).rgb;
    if (u_skin_w[4] > 0.0) c += u_skin_w[4] * texture(u_skin4, uv).rgb;
    if (u_skin_w[5] > 0.0) c += u_skin_w[5] * texture(u_skin5, uv).rgb;
    return c;
}

// Où tombe ce point du visage dans la photo plaquée (calée sur le milieu des yeux, au repos).
vec2 photo_uv() {
    vec2 d = v_rest - vec2(0.0, u_makeup.z);
    float c = cos(u_photo_fit.w), s = sin(u_photo_fit.w);
    d = vec2(d.x * c - d.y * s, d.x * s + d.y * c) * u_photo_fit.z;
    return vec2(0.5 + u_photo_fit.x + d.x / u_photo_use.y, 0.5 - u_photo_fit.y - d.y * u_photo_use.z);
}

void main() {
    int look = int(u_look.x + 0.5);
    bool only = u_look.y > 0.5;
    vec3 n = normalize(v_normal);
    vec3 tone = mix(vec3(1.0), vec3(0.5, 0.36, 0.28), u_look.z);
    vec3 alb;
    float alpha = 1.0, spec = 0.1, gloss = 22.0;
    bool lit = true, glint = false;
    float lips = 0.0, photo = 0.0;
    if (v_mat <= 1) {
        // Un pli profond (rides) peut présenter la peau de dos : elle reste de la peau.
        if (gl_FrontFacing || v_pos.z > 0.2) {
            alb = skin_at(v_mat == 0 ? v_uv : u_flat_uv) * tone;
            if (u_photo_use.x > 0.0) {
                // Photo plaquée de face, calée sur le milieu des yeux ; elle suit la peau quand le
                // visage bouge (position au repos). Le reste de la tête prend sa teinte de peau.
                vec3 base = max(skin_at(u_flat_uv), vec3(0.05));
                alb *= clamp(u_photo_tone / base, 0.3, 1.6);
                vec2 puv = photo_uv();
                vec2 edge = min(puv, 1.0 - puv);
                float w = smoothstep(0.0, 0.03, min(edge.x, edge.y));
                // Zone plaquée resserrée sur le visage : au-delà, le fond de la photo baverait sur les joues.
                w *= smoothstep(1.0, 0.68, length((v_rest - vec2(0.0, -0.08)) / vec2(0.33, 0.56)));
                w *= smoothstep(0.3, 0.65, normalize(v_head_normal).z) * u_photo_use.x;
                // Paupière qui se ferme : elle montre de la peau, pas l'œil de la photo étiré dessus.
                vec2 lid = (vec2(abs(v_rest.x) - u_makeup.y, v_rest.y - u_makeup.z)) / vec2(0.1, 0.055);
                w *= 1.0 - u_anim.w * smoothstep(1.0, 0.6, length(lid));
                alb = mix(alb, texture(u_photo, clamp(puv, 0.0, 1.0)).rgb, w);
                photo = w;
            }
            if (u_makeup.x > 0.0 && v_pos.z > 0.1) {
                // Lèvres : là où la texture rougit, autour de la bouche (qui s'ouvre vers le bas).
                float jaw = u_anim.z * 0.07;
                vec2 m = (v_pos.xy - vec2(0.0, u_makeup.w - jaw * 0.5)) / vec2(0.17, 0.08 + jaw * 0.6);
                float lip = smoothstep(1.0, 0.7, length(m)) * smoothstep(0.8, 0.68, alb.g / max(alb.r, 0.01));
                alb = mix(alb, vec3(0.45, 0.07, 0.14), lip * u_makeup.x * 0.85);
                lips = lip * u_makeup.x;
                // Fard : une ombre sombre autour de chaque œil, plus haute au-dessus qu'en dessous.
                vec2 e = (vec2(abs(v_pos.x) - u_makeup.y, v_pos.y - u_makeup.z - 0.014)) / vec2(0.115, 0.075);
                alb = mix(alb, alb * vec3(0.22, 0.2, 0.32), smoothstep(1.0, 0.3, length(e)) * u_makeup.x * 0.9);
            }
            if (v_mat == 0 && gl_FrontFacing) {
                // Relief fin : les rides et les pores peints dans la texture accrochent la lumière.
                float lum = dot(alb, vec3(0.3, 0.59, 0.11));
                vec2 slope = vec2(dFdx(lum), dFdy(lum)) / max(fwidth(v_pos.y), 1e-5);
                n = normalize(n + vec3(slope, 0.0) * (0.004 + 0.006 * u_look.w));
                spec = 0.2 + 0.5 * lips;
                gloss = 48.0;
            }
        } else {
            // Envers de la peau, loin derrière le visage : on ne le voit qu'au fond de la bouche.
            alb = vec3(0.13, 0.03, 0.04);
            lit = false;
        }
    } else if (v_mat == 2) {
        vec4 eye = mix(texture(u_eye_a, v_uv), texture(u_eye_b, v_uv), u_eye_mix);
        // Avec une photo, les yeux peuvent être ceux de la photo, plaqués sur le globe.
        bool own_eyes = u_photo_use.x > 0.0 && u_photo_use.w > 0.5 && look != 2;
        // La cornée (coque transparente devant l'iris) n'est pas dessinée, sauf le reflet de la
        // lumière qui s'y pose : c'est lui qui donne un regard vivant.
        if (eye.a < 0.5) {
            vec3 hv = normalize(normalize(vec3(-0.5, 0.6, 0.65)) + vec3(0.0, 0.0, 1.0));
            if (look == 2 || own_eyes || pow(max(dot(n, hv), 0.0), 110.0) < 0.45) discard;
            glint = true;
        }
        // Iris un peu moins saturé que la texture (le brun tirait sur le rouge).
        alb = mix(vec3(dot(eye.rgb, vec3(0.3, 0.59, 0.11))), eye.rgb, 0.7);
        alb = mix(alb * vec3(1.0, 0.97, 0.94), alb * vec3(1.0, 0.9, 0.78), u_look.w * 0.6);
        if (glint) {
            alb = vec3(1.0);
            lit = false;
        }
        // Le globe s'assombrit vers les coins et sous la paupière du haut : sans cela le regard est plat.
        vec3 hn = normalize(v_head_normal);
        alb = min(alb * 1.3, 1.0) * mix(0.62, 1.0, smoothstep(0.7, 0.95, hn.z)) * (1.0 - 0.32 * smoothstep(0.05, 0.3, hn.y));
        spec = 1.2;
        gloss = 160.0;
        if (own_eyes) {
            alb = mix(alb, texture(u_photo, clamp(photo_uv(), 0.0, 1.0)).rgb, u_photo_use.x);
            photo = u_photo_use.x;
            spec = 0.25;
        }
    } else if (v_mat == 3) {
        alb = texture(u_teeth, v_uv).rgb * mix(vec3(1.0), vec3(0.92, 0.84, 0.66), u_look.w);
        // Un peu plus chaudes et claires que la texture, grisâtre.
        alb = min(alb * vec3(1.18, 1.12, 1.02), 1.0);
        spec = 0.3;
        gloss = 50.0;
    } else if (v_mat == 4) {
        alb = vec3(0.62, 0.27, 0.29);
        spec = 0.25;
    } else if (v_mat == 5) {
        // Intérieur de la bouche : vu de dehors (à travers une joue creusée), il n'existe pas.
        if (!gl_FrontFacing) discard;
        alb = skin_at(v_uv) * vec3(0.3, 0.2, 0.22);
        spec = 0.2;
    } else {
        vec4 t = texture(u_hairs, v_uv);
        alpha = t.a;
        alb = u_brow_color * (0.7 + 0.6 * t.rgb);
        if (v_mat == 6) {
            // Sourcils clairsemés avec l'âge, redessinés plus sombres par le maquillage.
            alpha *= mix(1.0 - 0.45 * u_look.w, 1.0, u_makeup.x);
            alb *= 1.0 - 0.6 * u_makeup.x;
        } else {
            alb = mix(alb, vec3(0.03), 0.5 + 0.5 * u_makeup.x);
        }
        spec = 0.0;
    }
    if (look == 2) {
        // Statue : toutes les matières deviennent la même pierre veinée, les yeux sans regard.
        float vein = noise(v_pos.xy * 6.0 + v_pos.z * 4.0) + 0.5 * noise(v_pos.xy * 17.0 - v_pos.z * 9.0);
        vec3 stone = vec3(0.66, 0.64, 0.6) * (0.78 + 0.2 * vein) * (1.0 - 0.3 * smoothstep(0.06, 0.0, abs(vein - 0.75)));
        float lum = dot(alb, vec3(0.3, 0.59, 0.11));
        alb = v_mat == 2 ? stone : !lit ? stone * 0.2 : stone * (0.7 + 0.4 * lum);
        if (v_mat >= 6) alpha *= 0.5;
        spec = 0.06;
        gloss = 12.0;
    }

    // Lumière principale chaude en haut à gauche, appoint froid à droite, ciel au-dessus.
    // Calcul en lumière linéaire (les textures sont en sRGB), puis retour à l'écran.
    vec3 view = vec3(0.0, 0.0, 1.0);
    vec3 l1 = normalize(vec3(-0.5, 0.6, 0.65));
    vec3 l2 = normalize(vec3(0.75, 0.05, 0.4));
    vec3 col = alb;
    float rim = 0.0;
    if (lit) {
        if (!gl_FrontFacing) n = -n;
        bool skin = v_mat <= 1 && look != 2;
        float shade = v_mat <= 1 || v_mat == 5 ? v_shade : 1.0;
        vec3 lin = pow(alb, vec3(2.2));
        float n1 = dot(n, l1), n2 = dot(n, l2);
        // Sous la peau, la lumière se diffuse : le rouge passe plus loin dans l'ombre que le bleu.
        vec3 wrap = skin ? vec3(0.45, 0.22, 0.14) : vec3(0.08);
        vec3 d1 = clamp((n1 + wrap) / (1.0 + wrap), 0.0, 1.0);
        vec3 d2 = clamp((n2 + wrap) / (1.0 + wrap), 0.0, 1.0);
        vec3 key = vec3(1.0, 0.95, 0.88) * 1.3;
        vec3 fill = vec3(0.62, 0.72, 0.9) * 0.2;
        vec3 sky = mix(vec3(0.1, 0.08, 0.07), vec3(0.24, 0.24, 0.27), n.y * 0.5 + 0.5);
        // Les creux reçoivent moins de ciel, et un peu moins de lumière directe ; ils rougissent.
        vec3 hollow = skin ? mix(vec3(1.0, 0.55, 0.45), vec3(1.0), shade) : vec3(1.0);
        vec3 light = sky * shade * hollow + (key * d1 + fill * d2) * (0.4 + 0.6 * shade);
        // Une photo porte déjà sa lumière : on ne lui ajoute qu'un peu de modelé.
        col = lin * mix(light, vec3(1.0), photo * 0.65);
        // Reflets : deux lobes (net et large), plus forts à contre-jour (Fresnel).
        float facing = max(dot(n, view), 0.0);
        float fresnel = 0.04 + 0.96 * pow(1.0 - facing, 5.0);
        float ndh = max(dot(n, normalize(l1 + view)), 0.0);
        float lobes = 0.7 * pow(ndh, gloss) + 0.3 * pow(ndh, gloss * 0.25);
        col += spec * lobes * (0.5 + 4.0 * fresnel) * max(n1, 0.0) * shade * key;
        col = pow(clamp(col, 0.0, 1.0), vec3(1.0 / 2.2));
        rim = pow(1.0 - facing, 3.0);
    }

    // Visage seul : découpé en ovale aux bords fondus. Sinon, le cou se fond dans le noir.
    float fade;
    if (only) {
        float jaw = u_anim.z * 0.07;
        float oval = length((v_pos.xy - vec2(0.0, -0.08 - jaw * 0.5)) / vec2(0.385, 0.56 + jaw * 0.5));
        fade = smoothstep(1.0, 0.82, oval) * smoothstep(-0.05, 0.2, v_pos.z);
    } else {
        fade = max(smoothstep(-0.66, -0.53, v_pos.y), smoothstep(0.12, 0.3, v_pos.z));
    }
    alpha *= fade;

    if (look == 1) {
        // Spectre : la lumière du visage passe dans la palette, les bords luisent, l'image tremble.
        float lum = dot(col, vec3(0.3, 0.59, 0.11));
        lum = smoothstep(0.05, 0.95, lum);
        vec3 gc = palette(0.55 + lum * 0.12 + u_anim.y);
        col = mix(gc * lum * 1.1, vec3(1.0), 0.35 * lum * lum) + palette(0.55 + u_anim.y) * rim * 0.6;
        col *= 0.88 + 0.12 * sin(u_anim.x * 9.0 + v_pos.y * 24.0);
        alpha *= 0.82 + 0.18 * noise(gl_FragCoord.xy * 0.012 + vec2(0.0, -u_anim.x * 0.8));
    } else if (look == 3) {
        // Hologramme : lignes de balayage, bords lumineux, scintillement.
        float lum = dot(col, vec3(0.3, 0.59, 0.11));
        float scan = 0.62 + 0.38 * sin(gl_FragCoord.y * 0.55 - u_anim.x * 7.0);
        float flick = 0.92 + 0.08 * sin(u_anim.x * 31.0) * sin(u_anim.x * 4.3);
        vec3 hc = palette(0.5 + u_anim.y + lum * 0.15);
        col = (hc * (0.2 + lum) + palette(0.55 + u_anim.y) * rim * 1.3) * scan * flick;
        alpha *= (0.5 + 0.5 * max(rim, lum)) * (0.75 + 0.25 * scan);
    }
    if (look == 4) {
        // Projection : visage presque blanc, bleuté, très contrasté ; il ne garde qu'un soupçon
        // de couleur (lèvres, fard). C'est le rendu d'un visage filmé pour être projeté sur une tête blanche.
        float lum = dot(col, vec3(0.3, 0.59, 0.11));
        float g = smoothstep(0.04, 0.86, lum);
        vec3 tint = col * (g / max(lum, 0.04));
        col = mix(g * vec3(0.9, 0.95, 1.0), tint, 0.22);
    }
    out_color = vec4(clamp(col, 0.0, 1.0) * alpha, alpha);
}
"#;

struct Target {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    depth: glow::Renderbuffer,
    size: (u32, u32),
}

/// Ressources OpenGL de la tête : programme, maillage, textures, et une image par calque.
pub struct FaceRenderer {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    skins: Vec<glow::Texture>,
    eyes: Vec<glow::Texture>,
    teeth: glow::Texture,
    brows: glow::Texture,
    lashes: glow::Texture,
    targets: HashMap<usize, Target>,
    /// Photos plaquées, par identifiant.
    photos: HashMap<u64, glow::Texture>,
    poser: Poser,
}

unsafe fn load_texture(gl: &glow::Context, data: &[u8]) -> Result<glow::Texture, String> {
    let image = image::load_from_memory(data).map_err(|e| format!("tête 3D : texture illisible ({e})"))?.to_rgba8();
    unsafe {
        let t = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(t));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            image.width() as i32,
            image.height() as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(image.as_raw())),
        );
        gl.generate_mipmap(glow::TEXTURE_2D);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        Ok(t)
    }
}

fn as_bytes<T: Copy>(v: &[T]) -> &[u8] {
    // Sûr : f32 et u32 n'ont ni remplissage ni valeurs interdites.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v)) }
}

impl FaceRenderer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        let poser = Poser::new(Head::parse(HEAD)?);
        unsafe {
            let program = gl.create_program()?;
            for (kind, src) in [(glow::VERTEX_SHADER, VERTEX_SHADER), (glow::FRAGMENT_SHADER, FRAGMENT_SHADER)] {
                let shader = gl.create_shader(kind)?;
                gl.shader_source(shader, src);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return Err(format!("shader du visage : {}", gl.get_shader_info_log(shader)));
                }
                gl.attach_shader(program, shader);
                gl.delete_shader(shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(format!("shader du visage : {}", gl.get_program_info_log(program)));
            }
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            let ebo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let float = std::mem::size_of::<f32>() as i32;
            for (loc, (size, offset)) in [(3, 0), (3, 3), (2, 6), (1, 8), (1, 9), (2, 10)].into_iter().enumerate() {
                gl.enable_vertex_attrib_array(loc as u32);
                gl.vertex_attrib_pointer_f32(loc as u32, size, glow::FLOAT, false, STRIDE as i32 * float, offset * float);
            }
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, as_bytes(&poser.head.indices), glow::STATIC_DRAW);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            let load = |data: &[&[u8]]| data.iter().map(|d| load_texture(gl, d)).collect::<Result<Vec<_>, _>>();
            let out = Self {
                program,
                vao,
                vbo,
                ebo,
                skins: load(&SKINS)?,
                eyes: load(&EYES)?,
                teeth: load_texture(gl, TEETH)?,
                brows: load_texture(gl, BROWS)?,
                lashes: load_texture(gl, LASHES)?,
                targets: HashMap::new(),
                photos: HashMap::new(),
                poser,
            };
            gl.bind_texture(glow::TEXTURE_2D, None);
            Ok(out)
        }
    }

    pub fn destroy(&mut self, gl: &glow::Context) {
        self.retain(gl, |_| false);
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
            gl.delete_buffer(self.ebo);
            for t in self.skins.drain(..).chain(self.eyes.drain(..)).chain([self.teeth, self.brows, self.lashes]) {
                gl.delete_texture(t);
            }
        }
    }

    /// Ne garde que les photos dont l'identifiant vérifie `keep`.
    pub fn retain_photos(&mut self, gl: &glow::Context, keep: impl Fn(u64) -> bool) {
        self.photos.retain(|key, t| {
            let keep = keep(*key);
            if !keep {
                unsafe { gl.delete_texture(*t) };
            }
            keep
        });
    }

    /// Ne garde que les images des calques pour lesquels `keep` est vrai.
    pub fn retain(&mut self, gl: &glow::Context, keep: impl Fn(usize) -> bool) {
        self.targets.retain(|k, t| {
            let keep = keep(*k);
            if !keep {
                unsafe {
                    gl.delete_framebuffer(t.fbo);
                    gl.delete_texture(t.texture);
                    gl.delete_renderbuffer(t.depth);
                }
            }
            keep
        });
    }

    unsafe fn target(&mut self, gl: &glow::Context, k: usize, size: (u32, u32)) -> Option<&Target> {
        unsafe {
            if self.targets.get(&k).is_some_and(|t| t.size != size) {
                self.retain(gl, |other| other != k);
            }
            if !self.targets.contains_key(&k) {
                let texture = gl.create_texture().ok()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                for (name, value) in [
                    (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                    (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                    (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                    (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
                }
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    size.0 as i32,
                    size.1 as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                gl.bind_texture(glow::TEXTURE_2D, None);
                let depth = gl.create_renderbuffer().ok()?;
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
                gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, size.0 as i32, size.1 as i32);
                gl.bind_renderbuffer(glow::RENDERBUFFER, None);
                let fbo = gl.create_framebuffer().ok()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);
                gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
                self.targets.insert(k, Target { fbo, texture, depth, size });
            }
            self.targets.get(&k)
        }
    }

    /// Dessine la tête du calque `k` et renvoie sa texture (couleur prémultipliée, fond transparent,
    /// origine en bas à gauche). `size` : taille de l'image du visuel ; la tête est dessinée deux
    /// fois plus grande pour des bords lisses. Laisse l'état OpenGL à remettre en place par l'appelant
    /// (programme, tableau de sommets, cadre de dessin).
    pub unsafe fn render(
        &mut self,
        gl: &glow::Context,
        k: usize,
        size: (u32, u32),
        d: &FaceDraw,
        look: (Palette, f32, f32, f32),
        photo: Option<&crate::face::FacePhoto>,
    ) -> Option<glow::Texture> {
        unsafe {
            // Photo plaquée : envoyée à la carte graphique la première fois qu'on la voit.
            let photo = photo.filter(|p| p.key == d.photo && p.data.len() == (p.width * p.height * 4) as usize);
            if let Some(p) = photo
                && !self.photos.contains_key(&p.key)
                && let Ok(t) = gl.create_texture()
            {
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    p.width as i32,
                    p.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&p.data)),
                );
                gl.generate_mipmap(glow::TEXTURE_2D);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                gl.bind_texture(glow::TEXTURE_2D, None);
                self.photos.insert(p.key, t);
            }
            let photo = photo.and_then(|p| self.photos.get(&p.key).map(|t| (p, *t)));
            let fit = (4096.0 / size.0.max(size.1) as f32).min(2.0);
            let size = ((size.0 as f32 * fit) as u32, (size.1 as f32 * fit) as u32);
            let (fbo, texture) = self.target(gl, k, size).map(|t| (t.fbo, t.texture))?;
            let (palette, hue, time, scale) = look;

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.viewport(0, 0, size.0 as i32, size.1 as i32);
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(self.poser.pose(d)), glow::STREAM_DRAW);

            let loc = |name: &str| gl.get_uniform_location(self.program, name);
            // Rotation de la tête : lacet, tangage, roulis.
            let ((sy, cy), (sx, cx), (sz, cz)) = (d.turn[0].sin_cos(), d.turn[1].sin_cos(), d.turn[2].sin_cos());
            let ry = [[cy, 0.0, -sy], [0.0, 1.0, 0.0], [sy, 0.0, cy]];
            let rx = [[1.0, 0.0, 0.0], [0.0, cx, sx], [0.0, -sx, cx]];
            let rz = [[cz, sz, 0.0], [-sz, cz, 0.0], [0.0, 0.0, 1.0]];
            // Matrices en colonnes : (a × b)[c][r] = Σ a[k][r] · b[c][k].
            let mul = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| -> [[f32; 3]; 3] {
                std::array::from_fn(|c| std::array::from_fn(|r| (0..3).map(|k| a[k][r] * b[c][k]).sum()))
            };
            let rot = mul(mul(ry, rx), rz);
            gl.uniform_matrix_3_f32_slice(loc("u_rot").as_ref(), false, rot.as_flattened());
            // Cadrage : le visage seul est plus serré, centré entre le front et le menton.
            let (span, center) = if d.only { (1.12, -0.07) } else { (1.5, 0.02) };
            let half = 0.5 * scale.max(0.05) * span;
            let aspect = size.0 as f32 / size.1.max(1) as f32;
            gl.uniform_3_f32(loc("u_view").as_ref(), 1.0 / (half * aspect), 1.0 / half, center);

            // Peau : jeune, d'âge mûr (vers 48 ans) ou âgée (dès 75 ans), féminine ou masculine.
            let middle = (d.old / 0.3).min(1.0) - ((d.old - 0.3) / 0.45).clamp(0.0, 1.0);
            let ages = [1.0 - (d.old / 0.3).min(1.0), middle, ((d.old - 0.3) / 0.45).clamp(0.0, 1.0)];
            // Un enfant n'a pas encore la peau d'un homme.
            let masc = d.masc * (1.0 - 0.7 * d.kid);
            let mut weights = [0.0f32; 6];
            for (a, wa) in ages.into_iter().enumerate() {
                weights[2 * a] = wa * (1.0 - masc);
                weights[2 * a + 1] = wa * masc;
            }
            gl.uniform_1_f32_slice(loc("u_skin_w[0]").as_ref(), &weights);
            let unit = std::cell::Cell::new(0u32);
            let bind = |name: &str, t: glow::Texture| {
                gl.active_texture(glow::TEXTURE0 + unit.get());
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.uniform_1_i32(loc(name).as_ref(), unit.get() as i32);
                unit.set(unit.get() + 1);
            };
            for (i, t) in self.skins.iter().enumerate() {
                bind(&format!("u_skin{i}"), *t);
            }
            // Yeux : fondu entre les deux couleurs voisines du réglage.
            let e = d.eyes.clamp(0.0, 1.0) * (self.eyes.len() - 1) as f32;
            let first = (e as usize).min(self.eyes.len() - 2);
            bind("u_eye_a", self.eyes[first]);
            bind("u_eye_b", self.eyes[first + 1]);
            gl.uniform_1_f32(loc("u_eye_mix").as_ref(), e - first as f32);
            bind("u_teeth", self.teeth);
            let hairs_unit = unit.get();
            bind("u_hairs", self.brows);
            bind("u_photo", photo.map_or(self.teeth, |(_, t)| t));
            let fit = d.photo_fit;
            let (shown, aspect, tone) = photo.map_or((0.0, 1.0, [1.0; 3]), |(p, _)| (d.photo_mix, p.width as f32 / p.height.max(1) as f32, p.tone));
            gl.uniform_4_f32(loc("u_photo_fit").as_ref(), fit[0], fit[1], fit[2], fit[3]);
            gl.uniform_4_f32(loc("u_photo_use").as_ref(), shown, aspect, fit[4], d.photo_eyes as i32 as f32);
            gl.uniform_3_f32(loc("u_photo_tone").as_ref(), tone[0], tone[1], tone[2]);

            gl.uniform_2_f32(loc("u_flat_uv").as_ref(), self.poser.head.flat_uv[0], self.poser.head.flat_uv[1]);
            // Maquillage : intensité, et où sont les yeux et la bouche de ce personnage.
            let (eye, mouth) = (self.poser.eye_centers[0], self.poser.mouth);
            gl.uniform_4_f32(loc("u_makeup").as_ref(), d.makeup, eye[0].abs(), eye[1], mouth[1]);
            // Sourcils : blond, roux, châtain, noir ; ils blanchissent avec l'âge.
            let stops = [[0.62, 0.48, 0.26], [0.45, 0.2, 0.08], [0.2, 0.12, 0.07], [0.05, 0.04, 0.04]];
            let h = d.hair_color.clamp(0.0, 1.0) * 3.0;
            let i = (h as usize).min(2);
            let grey = ((d.old - 0.25) / 0.6).clamp(0.0, 1.0);
            let brow: [f32; 3] = std::array::from_fn(|c| {
                let color = stops[i][c] + (stops[i + 1][c] - stops[i][c]) * (h - i as f32);
                color + (0.6 - color) * grey
            });
            gl.uniform_3_f32(loc("u_brow_color").as_ref(), brow[0], brow[1], brow[2]);
            gl.uniform_4_f32(loc("u_look").as_ref(), d.look as i32 as f32, d.only as i32 as f32, d.skin, d.old);
            gl.uniform_4_f32(loc("u_anim").as_ref(), time, hue, d.open, d.blink);
            let flat: Vec<f32> = palette.coefs().iter().flatten().copied().collect();
            gl.uniform_3_f32_slice(loc("u_pal[0]").as_ref(), &flat);

            let ranges = self.poser.head.ranges;
            let draw = |m: usize| {
                let (start, count) = ranges[m];
                gl.draw_elements(glow::TRIANGLES, count as i32, glow::UNSIGNED_INT, (start * std::mem::size_of::<u32>()) as i32);
            };
            (0..OPAQUE).for_each(draw);
            // Sourcils et cils : transparents, par-dessus la peau, sans écrire la profondeur.
            // Le spectre et l'hologramme s'en passent (ils feraient des taches sombres).
            // Une photo bien présente apporte déjà les siens.
            if !matches!(d.look, Look::Ghost | Look::Hologram) && shown < 0.5 {
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                gl.depth_mask(false);
                draw(OPAQUE);
                gl.active_texture(glow::TEXTURE0 + hairs_unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(self.lashes));
                draw(OPAQUE + 1);
                gl.depth_mask(true);
                gl.disable(glow::BLEND);
            }

            gl.disable(glow::DEPTH_TEST);
            for u in 0..unit.get() {
                gl.active_texture(glow::TEXTURE0 + u);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(None);
            Some(texture)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poser() -> Poser {
        Poser::new(Head::parse(HEAD).expect("tête 3D lisible"))
    }

    fn bounds(p: &[[f32; 3]], frame: &[u8]) -> ([f32; 3], [f32; 3]) {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (q, _) in p.iter().zip(frame).filter(|(_, f)| **f == 1) {
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        (lo, hi)
    }

    #[test]
    fn every_target_used_by_the_app_exists() {
        let p = poser();
        let all = FaceDraw { old: 0.5, kid: 0.5, masc: 0.5, weight: 0.5, ..Default::default() };
        for (name, _) in identity_weights(&all) {
            assert!(p.head.morphs.contains_key(&name), "{name}");
        }
        for (name, _) in expression_weights(&all) {
            assert!(p.head.morphs.contains_key(name), "{name}");
        }
    }

    #[test]
    fn head_keeps_its_framing_at_every_age() {
        let mut p = poser();
        for (old, kid) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)] {
            for masc in [0.0, 1.0] {
                p.set_identity(&FaceDraw { old, kid, masc, weight: 0.3, ..Default::default() });
                let (lo, hi) = bounds(&p.identity, &p.head.frame);
                assert!((hi[1] - (CHIN_Y + HEAD_HEIGHT)).abs() < 1e-3, "{old} {kid} {masc} : {hi:?}");
                assert!((p.identity[p.head.chin][1] - CHIN_Y).abs() < 1e-3);
                // Tête à peu près symétrique et de largeur plausible.
                assert!((lo[0] + hi[0]).abs() < 0.02 && (0.6..1.1).contains(&(hi[0] - lo[0])), "{lo:?} {hi:?}");
            }
        }
    }

    #[test]
    fn mouth_opens_and_normals_stay_unit() {
        let mut p = poser();
        let shut = p.pose(&FaceDraw::default()).to_vec();
        let open = p.pose(&FaceDraw { open: 1.0, ..Default::default() }).to_vec();
        // Le menton descend quand la bouche s'ouvre.
        let chin = p.head.verts.iter().position(|v| v.pos as usize == p.head.chin).unwrap() * STRIDE;
        assert!(open[chin + 1] < shut[chin + 1] - 0.03, "{} {}", open[chin + 1], shut[chin + 1]);
        for v in open.chunks_exact(STRIDE) {
            let len = (v[3] * v[3] + v[4] * v[4] + v[5] * v[5]).sqrt();
            assert!((len - 1.0).abs() < 1e-3 || len < 1e-6, "{len}");
        }
    }
}
