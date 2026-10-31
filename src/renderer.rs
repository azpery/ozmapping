//! Rendu OpenGL : upload des frames vidéo, mapping 4 coins (homographie) et effets en shader.
//!
//! eframe (backend glow) partage un seul contexte GL entre toutes les fenêtres,
//! donc les textures sont communes à l'aperçu et à la sortie projecteur.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use eframe::egui;
use eframe::egui_glow;
use eframe::glow::{self, HasContext as _};

use crate::effects::{BlendMode, Effects};
use crate::generator::{Palette, Pattern};
use crate::surface::Fit;
use crate::video::FrameSlot;

/// Une source vidéo : clé de texture (uid du lecteur) + dernière frame partagée.
#[derive(Clone)]
pub struct SourceRef {
    pub key: u64,
    pub frame: FrameSlot,
}

/// Un calque à composer : deux sources (fondu enchaîné), mapping, effets, fusion.
#[derive(Clone)]
pub struct LayerDraw {
    pub a: Option<SourceRef>,
    pub b: Option<SourceRef>,
    pub mix: f32,
    /// Maillage de la surface déformée : [x, y, u, v] par sommet (voir `Warp::mesh`).
    pub mesh: Arc<Vec<f32>>,
    pub mask: Option<MaskDraw>,
    /// Champ de forme du masque, seulement si une animation de forme est utilisée.
    pub shape: Option<ShapeDraw>,
    pub effects: Effects,
    pub opacity: f32,
    pub blend: BlendMode,
    /// Cadrage de l'image, et proportions de la surface sur la sortie.
    pub fit: Fit,
    /// Rotation du média dans la surface, en degrés.
    pub media_rotate: f32,
    pub surface_aspect: f32,
    pub grid: bool,
    pub phases: Phases,
    pub generator: GenDraw,
    /// Destruction : avancement (0..1) et point d'impact (coordonnées de sortie).
    pub shatter: f32,
    pub shatter_center: [f32; 2],
}

/// Générateur à dessiner, avec le fondu entre l'ancien et le nouveau motif.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GenDraw {
    /// Motif et palette de départ (fade = 0) et d'arrivée (fade = 1).
    pub from: Pattern,
    pub from_palette: Palette,
    pub to: Pattern,
    pub to_palette: Palette,
    pub fade: f32,
    /// Part du générateur par-dessus la vidéo (1 = la remplace).
    pub mix: f32,
    /// Temps d'animation, intégré côté application (vitesse variable sans à-coups).
    pub time: f32,
    /// Décalage de la palette, en tours.
    pub hue: f32,
    pub scale: f32,
    pub detail: f32,
    /// Largeur / hauteur du motif (celles de la sortie, pour des cercles ronds).
    pub aspect: f32,
}

impl GenDraw {
    pub fn is_active(&self) -> bool {
        self.from != Pattern::Off || self.to != Pattern::Off
    }
}

/// Phases des effets animés, intégrées image par image côté application.
/// (Calculer `vitesse × temps total` ferait sauter l'image dès que la vitesse change,
/// par exemple pendant une transition de preset.)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phases {
    /// Décalage de teinte, en tours (0..1).
    pub hue: f32,
    /// Rotation automatique, en degrés.
    pub rotate: f32,
    /// Rotation du kaléidoscope, en degrés.
    pub kaleido: f32,
    /// Phase de l'ondulation, en radians.
    pub wave: f32,
    /// Oscillation du tourbillon, avancée du tunnel, liquide, palette acide, pulsation.
    pub swirl: f32,
    pub tunnel: f32,
    pub liquid: f32,
    pub acid: f32,
    pub pulse: f32,
    /// Animations de forme : lignes concentriques (en lignes), comètes (en tours de contour),
    /// respiration (en radians).
    pub shape_ripple: f32,
    pub shape_trace: f32,
    pub shape_breathe: f32,
    /// Boucle de destruction, en cycles (0..1).
    pub shatter: f32,
    /// Contours animés : vagues de lumière (en vagues), ondulation (en radians).
    pub lines_flow: f32,
    pub lines_wobble: f32,
    /// 3D : rotation automatique de l'objet (degrés), ondulation du drapeau (radians).
    pub rot3d_x: f32,
    pub rot3d_y: f32,
    pub flag: f32,
    /// Ondes de choc, en cycles (0..1) ; zoom du Droste, en répétitions (0..1).
    pub shock: f32,
    pub droste: f32,
    /// Rotation automatique de la forme du masque, en degrés.
    pub mask_spin: f32,
}

impl Phases {
    pub fn advance(&mut self, fx: &Effects, dt: f32) {
        // Repli sur une période pour garder une bonne précision en f32.
        const WAVE_PERIOD: f32 = 10.0 * std::f32::consts::PI; // sin(x + φ) et cos(x + 0,8 φ)
        // Une animation arrêtée (vitesse nulle) revient en douceur à sa position neutre,
        // sinon l'image resterait tournée / décalée en couleur après un retour « sans effet ».
        let step = |phase: f32, speed: f32, period: f32| -> f32 {
            if speed != 0.0 {
                return (phase + speed * dt).rem_euclid(period);
            }
            // Écart signé au neutre, par le chemin le plus court.
            let mut d = phase.rem_euclid(period);
            if d > period / 2.0 {
                d -= period;
            }
            let k = (dt * 6.0).min(1.0);
            let next = d * (1.0 - k);
            if next.abs() < period * 1e-4 { 0.0 } else { next.rem_euclid(period) }
        };
        self.hue = step(self.hue, fx.hue_speed, 1.0);
        self.rotate = step(self.rotate, fx.rotate_speed, 360.0);
        self.kaleido = step(self.kaleido, fx.kaleido_spin, 360.0);
        self.wave = step(self.wave, fx.wave_speed, WAVE_PERIOD);
        let tau = std::f32::consts::TAU;
        self.swirl = step(self.swirl, fx.swirl_speed * tau, tau);
        self.tunnel = step(self.tunnel, fx.tunnel_speed, 2.0); // profondeur répétée en miroir
        self.liquid = step(self.liquid, fx.liquid_speed, 200.0 * tau);
        self.acid = step(self.acid, fx.acid_speed, 1.0);
        self.pulse = step(self.pulse, fx.pulse_rate * tau, tau);
        self.shape_ripple = step(self.shape_ripple, fx.shape_ripple_speed, 1.0);
        self.shape_trace = step(self.shape_trace, fx.shape_trace_speed, 1.0);
        self.shape_breathe = step(self.shape_breathe, fx.shape_breathe_rate * tau, tau);
        self.shatter = step(self.shatter, fx.shatter_speed, 1.0);
        self.mask_spin = step(self.mask_spin, fx.mask_spin, 360.0);
        self.lines_flow = step(self.lines_flow, fx.lines_flow_speed, 1.0);
        self.lines_wobble = step(self.lines_wobble, fx.lines_wobble_speed * tau, tau);
        self.rot3d_x = step(self.rot3d_x, fx.spin3d_x, 360.0);
        self.rot3d_y = step(self.rot3d_y, fx.spin3d_y, 360.0);
        // Le drapeau ondule tant qu'il est actif.
        let flag_speed = if fx.flag > 0.0 { fx.flag_speed } else { 0.0 };
        self.flag = step(self.flag, flag_speed, 100.0 * tau);
        // Les ondes de choc repartent du centre tant qu'elles sont actives.
        let shock_speed = if fx.shock > 0.0 { fx.shock_speed } else { 0.0 };
        self.shock = step(self.shock, shock_speed, 1.0);
        let droste_speed = if fx.droste > 0.0 { fx.droste_speed } else { 0.0 };
        self.droste = step(self.droste, droste_speed, 1.0);
    }
}

/// Masque déjà rastérisé (niveaux de gris, en coordonnées de sortie).
/// `key` change à chaque nouvelle image : c'est ce qui déclenche l'envoi au GPU.
#[derive(Clone)]
pub struct MaskDraw {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub data: Arc<Vec<u8>>,
    pub invert: bool,
}

/// Champ de forme du masque (voir `mask::shape_field`), en coordonnées de sortie.
/// `key` change à chaque nouveau calcul, comme pour `MaskDraw`.
#[derive(Clone)]
pub struct ShapeDraw {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub data: Arc<Vec<f32>>,
    /// Demi-largeur du bord adouci, en hauteurs de sortie (pour la respiration).
    pub feather: f32,
}

/// Tout ce qu'il faut pour dessiner une image (copié dans le callback de peinture).
/// Les calques sont listés du bas vers le haut.
#[derive(Clone)]
pub struct DrawParams {
    pub layers: Vec<LayerDraw>,
    pub time: f32,
    pub master: f32,
    /// Écran blanc (pour photographier l'image projetée) à la place de la composition.
    pub white: bool,
}

struct VideoTexture {
    texture: glow::Texture,
    frame_id: u64,
    size: (u32, u32),
}

/// Texture où un calque reçoit son visuel généré (rendu à part, une fois par image).
struct GenTarget {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    size: (u32, u32),
    /// Réglages du dernier rendu : identiques (aperçu puis sortie, même image) = pas de nouveau calcul.
    last: Option<GenDraw>,
}

pub struct Renderer {
    program: glow::Program,
    /// Programme des visuels générés (même source, compilée avec GEN_PASS).
    gen_program: glow::Program,
    gen_targets: HashMap<usize, GenTarget>,
    /// Cible hors écran de l'enregistrement vidéo.
    capture_target: Option<GenTarget>,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    black: glow::Texture,
    textures: HashMap<u64, VideoTexture>,
    mask_textures: HashMap<u64, glow::Texture>,
    shape_textures: HashMap<u64, glow::Texture>,
}

const VERTEX_SHADER: &str = r#"
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
out vec2 v_pos;
out vec2 v_uv;
void main() {
    // a_pos : coordonnées de sortie 0..1, y vers le bas (comme egui).
    v_pos = a_pos;
    v_uv = a_uv;
    gl_Position = vec4(a_pos.x * 2.0 - 1.0, 1.0 - a_pos.y * 2.0, 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER: &str = r#"
precision highp float;
in vec2 v_pos;
in vec2 v_uv;
out vec4 out_color;

uniform sampler2D u_tex_a;
uniform sampler2D u_tex_b;
uniform int u_has_a;
uniform int u_has_b;
uniform float u_mix;
uniform vec2 u_res;
uniform sampler2D u_mask_tex;
uniform int u_has_mask;
uniform int u_mask_invert;
// Rotation de la forme du masque (radians) autour de son centre (coordonnées de sortie).
uniform float u_mask_angle;
uniform vec2 u_mask_center;
uniform float u_time;
uniform vec2 u_tex_size;
uniform int u_fit;
// Rotation du média dans la surface (radians).
uniform float u_media_rot;
uniform float u_surface_aspect;
uniform vec2 u_size_a;
uniform vec2 u_size_b;
uniform int u_grid;
uniform float u_opacity;
uniform int u_blend;
uniform int u_solid;
uniform float u_solid_alpha;

uniform float u_brightness;
uniform float u_contrast;
uniform float u_saturation;
uniform float u_hue;
uniform float u_invert;
uniform float u_posterize;
uniform vec3 u_tint;
uniform float u_tint_amount;
uniform float u_edge;
uniform float u_vignette;
uniform float u_zoom;
uniform float u_rotate;
uniform int u_flip_x;
uniform int u_flip_y;
uniform int u_mirror;
uniform float u_kaleido;
uniform float u_kaleido_angle;
uniform float u_pixelate;
uniform float u_rgb_shift;
uniform float u_wave_amp;
uniform float u_wave_freq;
uniform float u_wave_phase;
uniform float u_strobe;
uniform float u_swirl;
uniform float u_swirl_phase;
uniform float u_polar;
uniform float u_tunnel_phase;
uniform float u_tiles;
uniform float u_liquid;
uniform float u_liquid_scale;
uniform float u_liquid_phase;
uniform float u_acid;
uniform float u_acid_phase;
uniform float u_rainbow;
uniform float u_solarize;
uniform float u_pulse;
uniform float u_pulse_phase;
uniform float u_echo;

// Animations qui suivent la forme du masque (voir `mask::shape_field`).
uniform sampler2D u_shape_tex;
uniform int u_has_shape;
uniform vec2 u_shape_texel;
uniform float u_feather;
uniform vec3 u_shape_color;
uniform float u_shape_glow;
uniform float u_shape_width;
uniform float u_shape_ripple;
uniform float u_shape_freq;
uniform float u_shape_ripple_phase;
uniform float u_shape_wave;
uniform float u_shape_trace;
uniform float u_shape_trace_count;
uniform float u_shape_trace_phase;
uniform float u_shape_trace_dir;
uniform float u_shape_grow;
uniform float u_shape_rainbow;
uniform float u_shape_echo;
uniform float u_shape_inner;
uniform float u_shape_dashes;
uniform float u_shape_dash_count;

// Destruction : style, avancement (0 = intact, 1 = détruit), taille des morceaux (par hauteur
// de sortie), point d'impact (coordonnées de sortie), lueur des fissures.
uniform int u_break_style;
uniform float u_break;
uniform float u_break_size;
uniform vec2 u_break_center;
uniform float u_break_glow;

// Contours animés de l'image.
uniform float u_lines_glow;
uniform vec3 u_lines_color;
uniform float u_lines_rainbow;
uniform float u_lines_threshold;
uniform float u_lines_width;
uniform float u_lines_flow;
uniform float u_lines_flow_phase;
uniform float u_lines_wobble;
uniform float u_lines_wobble_phase;
uniform float u_lines_dim;
uniform int u_obj3d;
uniform vec2 u_rot3d;
uniform float u_dist3d;
uniform float u_relief;
uniform vec2 u_relief_dir;
uniform float u_anaglyph;
uniform float u_flag;
uniform float u_flag_phase;
uniform float u_thermal;
uniform float u_blur;
uniform float u_bloom;
uniform float u_shock;
uniform float u_shock_phase;
uniform float u_fisheye;
uniform float u_hexagon;
uniform float u_crt;
uniform float u_glitch;
uniform float u_grain;
uniform float u_halftone;
uniform float u_droste;
uniform float u_droste_phase;
uniform float u_zoom_blur;
uniform float u_ascii;
uniform float u_fold;
uniform float u_fold_count;
uniform float u_height3d;
uniform float u_voxels;

// Générateurs (motif de départ A, d'arrivée B, fondu entre les deux).
uniform int u_gen_a;
uniform int u_gen_b;
uniform sampler2D u_gen_tex;
uniform int u_gen_on;
uniform float u_gen_fade;
uniform float u_gen_mix;
uniform float u_gen_time;
uniform float u_gen_hue;
uniform float u_gen_scale;
uniform float u_gen_detail;
uniform float u_gen_aspect;
uniform vec3 u_pal_a[4];
uniform vec3 u_pal_b[4];

const float PI = 3.14159265359;

mat3 rot_x(float a) { float c = cos(a), s = sin(a); return mat3(1.0, 0.0, 0.0, 0.0, c, s, 0.0, -s, c); }
mat3 rot_y(float a) { float c = cos(a), s = sin(a); return mat3(c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c); }

// 3D : rayon lancé depuis la caméra vers le point `q` de la surface (0..1).
// Renvoie les coordonnées image du point touché sur l'objet, hit = 0 si le rayon passe à côté,
// et l'éclairage du point. Sans rotation, le plan remplit exactement la surface.
void object3d(vec2 q, out vec2 ouv, out float hit, out float light3) {
    float asp = max(u_surface_aspect, 0.05);
    vec2 p = (q - 0.5) * vec2(asp, 1.0);
    p.y = -p.y;
    float D = max(u_dist3d, 0.5);
    mat3 R = rot_y(u_rot3d.y) * rot_x(u_rot3d.x);
    mat3 Rt = transpose(R);
    vec3 ro = Rt * vec3(0.0, 0.0, -D);
    vec3 rd = Rt * normalize(vec3(p, D));
    // Lumière en haut à gauche, du côté de la caméra.
    vec3 L = Rt * normalize(vec3(-0.4, 0.5, -1.0));
    hit = 0.0;
    ouv = vec2(0.0);
    light3 = 1.0;
    float t = -1.0;
    vec3 n = vec3(0.0, 0.0, -1.0);
    if (u_obj3d == 1) {
        // Plan : l'image à ses proportions, dans le plan z = 0.
        if (abs(rd.z) > 1e-5) {
            float tt = -ro.z / rd.z;
            vec3 h = ro + rd * tt;
            if (tt > 0.0 && abs(h.x) <= asp * 0.5 && abs(h.y) <= 0.5) {
                t = tt;
                ouv = vec2(h.x / asp + 0.5, 0.5 - h.y);
            }
        }
    } else if (u_obj3d == 2) {
        // Cube : l'image sur chaque face.
        float b = 0.42;
        // Jamais de division par zéro (sign(0) vaut 0).
        vec3 safe = mix(vec3(-1.0), vec3(1.0), step(0.0, rd)) * max(abs(rd), vec3(1e-6));
        vec3 t1 = (vec3(-b) - ro) / safe;
        vec3 t2 = (vec3(b) - ro) / safe;
        vec3 tmin = min(t1, t2);
        vec3 tmax = max(t1, t2);
        float tn = max(max(tmin.x, tmin.y), tmin.z);
        float tf = min(min(tmax.x, tmax.y), tmax.z);
        if (tn < tf && tn > 0.0) {
            t = tn;
            vec3 h = ro + rd * t;
            vec3 a = abs(h);
            if (a.x >= a.y && a.x >= a.z) {
                n = vec3(sign(h.x), 0.0, 0.0);
                ouv = vec2(h.z * sign(h.x), -h.y) / (2.0 * b) + 0.5;
            } else if (a.y >= a.z) {
                n = vec3(0.0, sign(h.y), 0.0);
                ouv = vec2(h.x, h.z * sign(h.y)) / (2.0 * b) + 0.5;
            } else {
                n = vec3(0.0, 0.0, sign(h.z));
                ouv = vec2(-h.x * sign(h.z), -h.y) / (2.0 * b) + 0.5;
            }
        }
    } else if (u_obj3d == 3) {
        // Sphère : l'image enroulée (longitude, latitude).
        float r = 0.45;
        float bq = dot(ro, rd);
        float disc = bq * bq - (dot(ro, ro) - r * r);
        if (disc > 0.0) {
            float tt = -bq - sqrt(disc);
            if (tt > 0.0) {
                t = tt;
                vec3 h = ro + rd * t;
                n = h / r;
                ouv = vec2(atan(h.z, h.x) / (2.0 * PI) + 0.5, acos(clamp(-h.y / r, -1.0, 1.0)) / PI);
            }
        }
    } else if (u_obj3d == 4) {
        // Cylindre vertical ouvert : face extérieure, sinon l'intérieur par le haut ou le bas.
        float r = 0.38;
        float hh = 0.5;
        float a = dot(rd.xz, rd.xz);
        float bq = dot(ro.xz, rd.xz);
        float disc = bq * bq - a * (dot(ro.xz, ro.xz) - r * r);
        if (disc > 0.0 && a > 1e-8) {
            float s = sqrt(disc);
            for (int k = 0; k < 2; k++) {
                float tt = (-bq + (k == 0 ? -s : s)) / a;
                vec3 h = ro + rd * tt;
                if (tt > 0.0 && abs(h.y) <= hh) {
                    t = tt;
                    n = vec3(h.x, 0.0, h.z) / r;
                    ouv = vec2(atan(h.z, h.x) / (2.0 * PI) + 0.5, 0.5 - h.y / (2.0 * hh));
                    break;
                }
            }
        }
    }
    if (u_obj3d == 5) {
        // Tore (anneau couché dans le plan xz) : marche sur sa distance.
        vec2 tor = vec2(0.32, 0.13);
        float tt = 0.0;
        for (int k = 0; k < 64; k++) {
            vec3 h = ro + rd * tt;
            float lxz = max(length(h.xz), 1e-5);
            vec2 qq = vec2(lxz - tor.x, h.y);
            float dd = length(qq) - tor.y;
            if (dd < 0.0008) {
                t = tt;
                n = normalize(vec3(h.x * (1.0 - tor.x / lxz), h.y, h.z * (1.0 - tor.x / lxz)));
                ouv = vec2(atan(h.z, h.x) / (2.0 * PI) + 0.5, atan(qq.y, qq.x) / (2.0 * PI) + 0.5);
                break;
            }
            tt += dd;
            if (tt > D + 2.0) break;
        }
    }
    if (t > 0.0) {
        hit = 1.0;
        if (dot(n, rd) > 0.0) n = -n;
        light3 = 0.3 + 0.75 * max(dot(n, L), 0.0);
    }
}

// Sortie selon le mode de fusion (les facteurs GL correspondants sont réglés côté Rust).
// Une couverture nulle donne toujours une valeur neutre pour le mode.
void emit(vec3 col, float a) {
    if (u_blend == 0) {
        out_color = vec4(col, a);                       // normal
    } else if (u_blend == 1 || u_blend == 2 || u_blend == 4) {
        out_color = vec4(col * a, 1.0);                 // addition, écran, éclaircir
    } else {
        out_color = vec4(mix(vec3(1.0), col, a), 1.0);  // produit, assombrir
    }
}

// Position lue dans le masque (et son champ de forme) : tourner la forme revient à lire
// le masque tourné dans l'autre sens, autour de son centre, en tenant compte des proportions.
vec2 mask_pos(vec2 pos) {
    if (u_mask_angle == 0.0) return pos;
    float aspect = u_res.x / max(u_res.y, 1.0);
    vec2 q = (pos - u_mask_center) * vec2(aspect, 1.0);
    float c = cos(u_mask_angle), s = sin(u_mask_angle);
    q = mat2(c, -s, s, c) * q;
    return u_mask_center + q / vec2(aspect, 1.0);
}

// Masque : image pré-calculée (bords adoucis compris), lue en coordonnées de sortie.
float mask_value(vec2 pos) {
    if (u_has_mask == 0) return 1.0;
    // Hors de l'image du masque (forme tournée qui dépasse) : en dehors de la forme.
    bool outside = any(lessThan(pos, vec2(0.0))) || any(greaterThan(pos, vec2(1.0)));
    float m = outside ? 0.0 : texture(u_mask_tex, pos).r;
    return u_mask_invert == 1 ? 1.0 - m : m;
}

// ---------------------------------------------------------------- générateurs

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
vec2 hash2(vec2 p) {
    return fract(sin(vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)))) * 43758.5453);
}
float noise(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}
float fbm(vec2 p) {
    int octaves = 2 + int(u_gen_detail * 4.0);
    float v = 0.0, a = 0.5;
    for (int i = 0; i < 6; i++) {
        if (i >= octaves) break;
        v += a * noise(p);
        p = p * 2.03 + vec2(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

// Motif `kind` au point p (centré, hauteur ≈ 1) : x = position dans la palette, y = intensité.
#ifdef GEN_PASS
// Visuels générés : compilés seulement dans le programme du générateur (une passe à part,
// rendue dans une texture). Le shader des calques ne fait que lire cette texture.
vec2 march3d(int kind, vec2 p, float t); // visuels 3D, plus bas

vec2 pattern(int kind, vec2 p, float t) {
    if (kind >= 23) return march3d(kind, p, t);
    float d = u_gen_detail;
    float r = length(p);
    float a = atan(p.y, p.x);
    if (kind == 1) { // plasma
        float v = sin(p.x * 3.0 + t) + sin(p.y * 3.3 - t * 1.2) + sin((p.x + p.y) * 2.7 + t * 0.7)
                + sin(length(p * 4.0 + 2.0 * vec2(sin(t * 0.5), cos(t * 0.4))) - t);
        v += d * 2.0 * sin(p.x * 9.0 * cos(t * 0.21) + p.y * 8.0 * sin(t * 0.17) + t * 2.0);
        return vec2(v * 0.125 + 0.5, 1.0);
    }
    if (kind == 2) { // spirale
        float arms = 1.0 + floor(d * 6.0);
        float v = 0.5 + 0.5 * sin(arms * a + 8.0 * log(r + 1e-3) - t * 4.0);
        return vec2(v * 0.4 + r * 0.6 - t * 0.1, 0.25 + 0.75 * smoothstep(0.2, 0.8, v));
    }
    if (kind == 3) { // tunnel
        float z = 0.3 / max(r, 1e-3) + t;
        float rings = 0.5 + 0.5 * sin(z * 4.0 * PI);
        float stripes = 0.5 + 0.5 * sin(a * (4.0 + floor(d * 8.0)) + z * 3.0);
        float v = mix(rings, stripes, 0.5);
        return vec2(z * 0.15 + a / (2.0 * PI), smoothstep(0.1, 0.9, v) * smoothstep(0.0, 0.25, r));
    }
    if (kind == 4) { // fractale de Julia
        vec2 c = 0.7885 * vec2(cos(t * 0.25), sin(t * 0.25));
        vec2 z = p * 1.3;
        int max_it = 24 + int(d * 72.0);
        float n = 0.0;
        for (int i = 0; i < 96; i++) {
            if (i >= max_it || dot(z, z) > 16.0) break;
            z = vec2(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
            n += 1.0;
        }
        if (n >= float(max_it)) return vec2(0.0, 0.0);
        float sm = n - log2(log2(max(dot(z, z), 1.0001))) + 4.0;
        return vec2(sm / 24.0 + t * 0.05, 1.0);
    }
    if (kind == 5) { // mandala
        float k = 6.0 + floor(d * 10.0);
        float petals = abs(cos(a * k * 0.5 + sin(r * 4.0 - t) * 0.6));
        float rings = 0.5 + 0.5 * sin(r * 14.0 - t * 2.0 + petals * 3.0);
        float v = petals * 0.6 + 0.4 * rings;
        return vec2(r * 0.8 - t * 0.1 + petals * 0.3, smoothstep(0.15, 0.85, v));
    }
    if (kind == 6) { // moiré
        vec2 c1 = 0.4 * vec2(sin(t * 0.7), cos(t * 0.5));
        vec2 c2 = 0.4 * vec2(cos(t * 0.6), sin(t * 0.8));
        float f = 30.0 + d * 60.0;
        float v = sin(length(p - c1) * f) * sin(length(p - c2) * f);
        return vec2(r * 0.5 + t * 0.05 + 0.2 * v, smoothstep(-0.25, 0.25, v));
    }
    if (kind == 7) { // lampe à lave
        int balls = 4 + int(d * 6.0);
        float s = 0.0;
        for (int i = 0; i < 10; i++) {
            if (i >= balls) break;
            float k = float(i);
            vec2 c = 0.6 * vec2(sin(t * (0.3 + 0.07 * k) + k * 1.7), cos(t * (0.4 + 0.05 * k) + k * 2.3));
            c.x *= u_gen_aspect;
            vec2 q = p - c;
            s += 0.04 / max(dot(q, q), 1e-4);
        }
        return vec2(s * 0.12 + t * 0.03, 0.15 + 0.85 * smoothstep(0.8, 1.1, s));
    }
    if (kind == 8) { // nuages fractals (déformation de domaine)
        vec2 q = vec2(fbm(p * 2.0 + t * 0.1), fbm(p * 2.0 + vec2(5.2, 1.3) - t * 0.12));
        vec2 w = vec2(fbm(p * 2.0 + 4.0 * q + vec2(1.7, 9.2) + t * 0.15), fbm(p * 2.0 + 4.0 * q + vec2(8.3, 2.8)));
        float v = fbm(p * 2.0 + 4.0 * w);
        return vec2(v * 1.5 + length(q) * 0.5, clamp(0.25 + v * 1.2, 0.0, 1.0));
    }
    if (kind == 9) { // cristaux (Voronoï)
        vec2 g = p * (3.0 + d * 6.0);
        vec2 i = floor(g), f = fract(g);
        float d1 = 8.0, d2 = 8.0;
        vec2 cell = vec2(0.0);
        for (int y = -1; y <= 1; y++) {
            for (int x = -1; x <= 1; x++) {
                vec2 n = vec2(float(x), float(y));
                vec2 o = 0.5 + 0.5 * sin(t + 6.2831 * hash2(i + n));
                vec2 rr = n + o - f;
                float dd = dot(rr, rr);
                if (dd < d1) { d2 = d1; d1 = dd; cell = i + n; } else if (dd < d2) { d2 = dd; }
            }
        }
        float edge = sqrt(d2) - sqrt(d1);
        return vec2(hash(cell) + t * 0.05, smoothstep(0.0, 0.08, edge) * (0.55 + 0.45 * (1.0 - sqrt(d1))));
    }
    if (kind == 10) { // op art
        float y = p.y + 0.3 * sin(p.x * 3.0 + t) + 0.2 * sin(r * 5.0 - t * 1.3);
        float v = sin(y * (20.0 + d * 40.0));
        return vec2(p.x * 0.3 + t * 0.05 + 0.5, smoothstep(-0.15, 0.15, v));
    }
    if (kind == 11) { // hyperespace
        float n = 60.0 + d * 140.0;
        float cellf = (a / (2.0 * PI) + 0.5) * n;
        float id = floor(cellf);
        float h = hash(vec2(id, 1.0));
        float z = fract(h * 10.0 + t * (0.3 + h * 0.5));
        float head = z * 1.2;
        float len = 0.05 + 0.4 * z * z;
        float along = smoothstep(head - len, head, r) * (1.0 - smoothstep(head, head + 0.01, r));
        float across = 1.0 - smoothstep(0.1, 0.35, abs(fract(cellf) - 0.5));
        float glow = 0.12 * exp(-r * 6.0);
        return vec2(h + t * 0.02, clamp(along * across * (0.4 + z) + glow, 0.0, 1.0));
    }
    if (kind == 12) { // soleil rayonnant
        float n = 8.0 + floor(d * 24.0);
        float rays = 0.5 + 0.5 * sin(a * n + t * 2.0 + sin(r * 6.0 - t) * 1.5);
        float core = exp(-r * 3.0);
        return vec2(r * 0.7 - t * 0.15 + rays * 0.2, clamp(smoothstep(0.3, 0.7, rays) * (0.5 + 0.5 * core) + core * 0.6, 0.0, 1.0));
    }
    if (kind == 13) { // ruche : hexagones qui pulsent
        vec2 q = p * (4.0 + d * 10.0);
        const vec2 hs = vec2(1.0, 1.7320508);
        vec4 hc = floor(vec4(q, q - vec2(0.5, 1.0)) / hs.xyxy) + 0.5;
        vec4 h = vec4(q - hc.xy * hs, q - (hc.zw + 0.5) * hs);
        vec4 hex = dot(h.xy, h.xy) < dot(h.zw, h.zw) ? vec4(h.xy, hc.xy) : vec4(h.zw, hc.zw + 0.5);
        vec2 hp = abs(hex.xy);
        float edge = 0.5 - max(dot(hp, hs * 0.5), hp.x);
        float id = hash(hex.zw);
        float pulse = 0.5 + 0.5 * sin(t * 2.0 + id * 6.2831 - length(hex.zw) * 0.6);
        return vec2(id * 0.5 + t * 0.05 + length(hex.zw) * 0.05, smoothstep(0.0, 0.05, edge) * (0.25 + 0.75 * pulse));
    }
    if (kind == 14) { // caustiques : reflets de fond de piscine
        vec2 q = p * (3.0 + d * 4.0) + 20.0;
        vec2 i = q;
        float c = 1.0;
        for (int n = 0; n < 5; n++) {
            float tt = t * 0.5 * (1.0 - 3.5 / float(n + 1));
            i = q + vec2(cos(tt - i.x) + sin(tt + i.y), sin(tt - i.y) + cos(tt + i.x));
            c += 1.0 / length(vec2(q.x / (sin(i.x + tt) / 0.005), q.y / (cos(i.y + tt) / 0.005)));
        }
        c = 1.17 - pow(c / 5.0, 1.4);
        float v = clamp(pow(abs(c), 8.0), 0.0, 1.0);
        return vec2(0.55 + v * 0.2 + t * 0.02, 0.15 + 0.85 * v);
    }
    if (kind == 15) { // aurore boréale
        float v = 0.0;
        for (int i = 0; i < 3; i++) {
            float k = float(i);
            float y = p.y + 0.3 * sin(p.x * (1.5 + k * 0.7) + t * (0.3 + 0.2 * k) + k)
                    + 0.15 * fbm(vec2(p.x * 2.0 + t * 0.2, k * 3.1));
            v += exp(-abs(y - (k - 1.0) * 0.25) * (8.0 - d * 4.0)) * (0.6 + 0.4 * sin(p.x * 10.0 + t + k));
        }
        float stars = step(0.997, hash(floor(p * 180.0))) * 0.6;
        return vec2(0.3 + p.x * 0.2 + v * 0.3 + t * 0.03, clamp(v + stars, 0.0, 1.0));
    }
    if (kind == 16) { // quasicristal : ondes planes superposées
        vec2 q = p * (8.0 + d * 10.0);
        int n = 5 + int(d * 6.0);
        float v = 0.0;
        for (int i = 0; i < 11; i++) {
            if (i >= n) break;
            float ang = float(i) * PI / float(n);
            v += cos(dot(q, vec2(cos(ang), sin(ang))) + t * 2.0);
        }
        v /= float(n);
        return vec2(v * 0.5 + 0.5 + t * 0.05, smoothstep(-0.6, 0.8, v));
    }
    if (kind == 17) { // galaxie
        float arms = 2.0 + floor(d * 4.0);
        float armv = pow(0.5 + 0.5 * cos(a * arms - log(r + 1e-3) * 4.0 + t * 0.8), 3.0);
        float dust = fbm(p * 6.0 + vec2(cos(t * 0.1), sin(t * 0.1)));
        float core = exp(-r * 6.0);
        float stars = step(0.995, hash(floor(p * 200.0))) * 0.8;
        float v = armv * exp(-r * 1.5) * (0.5 + dust) + core * 1.5 + stars;
        return vec2(r * 0.6 + 0.6 + t * 0.02 - core * 0.3, clamp(v, 0.0, 1.0));
    }
    if (kind == 18) { // labyrinthe : tuiles de Truchet qui basculent
        vec2 q = p * (6.0 + d * 14.0);
        vec2 id = floor(q);
        vec2 f = fract(q) - 0.5;
        float h = hash(id + floor(t * 0.3 + hash(id + 7.0) * 3.0) * 13.0);
        if (h > 0.5) f.x = -f.x;
        float dd = min(length(f - 0.5), length(f + 0.5));
        float line = 1.0 - smoothstep(0.06, 0.12, abs(dd - 0.5));
        return vec2(dd * 0.5 + id.x * 0.03 + id.y * 0.05 + t * 0.1, line);
    }
    if (kind == 19) { // grille rétro : sol quadrillé et soleil couchant
        float z = 0.4 / max(abs(p.y - 0.05), 0.01);
        vec2 gl = vec2(p.x * z, z + t * 2.0);
        vec2 gg = 0.5 - abs(fract(gl) - 0.5);
        vec2 gw = fwidth(gl) * 1.5;
        float grid = max(1.0 - smoothstep(0.0, gw.x + 0.02, gg.x), 1.0 - smoothstep(0.0, gw.y + 0.02, gg.y));
        if (p.y > 0.05) {
            float fade = smoothstep(0.05, 0.2, p.y);
            return vec2(0.8 + p.x * 0.05, clamp(grid * (0.3 + 0.7 * fade), 0.0, 1.0) * 0.95 + 0.05);
        }
        float sd = length(p - vec2(0.0, -0.17));
        float sun = 1.0 - smoothstep(0.28, 0.29, sd);
        // Bandes horizontales qui défilent dans le bas du soleil.
        float band = step(0.35 + 0.3 * smoothstep(-0.2, 0.05, p.y), fract(p.y * 25.0 - t * 0.5));
        sun *= p.y < -0.12 ? 1.0 : band;
        return vec2(0.1 - p.y * 0.8, max(sun, 0.2 * exp(-abs(p.y - 0.05) * 4.0)));
    }
    if (kind == 20) { // flammes
        float n = fbm(vec2(p.x * 3.0, p.y * 2.0 + t * 1.5));
        float height = 0.5 - p.y;
        float flame = clamp((n * 1.6 - height * 1.1 + 0.25) * 2.0, 0.0, 1.0);
        return vec2(flame * 0.35, flame);
    }
    if (kind == 21) { // électricité : arcs crépitants
        float n = fbm(p * 2.5 + vec2(0.0, t * 0.8));
        float m2 = fbm(p * 2.5 + vec2(t * 0.6, 3.0) + n);
        float v = 0.02 / max(abs(n - 0.5), 1e-3) + 0.015 / max(abs(m2 - 0.5), 1e-3);
        return vec2(0.6 + n * 0.3 + t * 0.03, clamp(v * (0.25 + d * 0.3), 0.0, 1.0));
    }
    if (kind == 22) { // gouttes : ronds dans l'eau
        int n = 3 + int(d * 6.0);
        float v = 0.0;
        for (int i = 0; i < 9; i++) {
            if (i >= n) break;
            float k = float(i);
            float tt = t * 0.4 + hash(vec2(k, 0.0));
            vec2 c = (hash2(vec2(k, floor(tt))) - 0.5) * vec2(u_gen_aspect, 1.0) * u_gen_scale;
            float age = fract(tt);
            float dr = length(p - c) - age * 1.2;
            v += sin(dr * 40.0) * exp(-abs(dr) * 12.0) * (1.0 - age);
        }
        return vec2(0.55 + v * 0.2 + t * 0.02, clamp(0.3 + v * 0.7, 0.0, 1.0));
    }
    return vec2(0.0);
}
#endif

// ---------------------------------------------------------------- destruction
//
// Chaque fragment cherche le morceau qui le recouvre une fois déplacé (morceaux voisins, et
// ceux d'au-dessus qui tombent), puis renvoie la position d'origine de ce point du morceau :
// l'image, le masque et le contour y sont lus, le morceau emporte donc son bout d'image.
// Repère : hauteurs de sortie (x de 0 à `aspect`).

mat2 rot2(float a) { float c = cos(a), s = sin(a); return mat2(c, s, -s, c); }

// Avancement d'un morceau centré en c : x = chute (0..1), y = fissures qui l'annoncent (0..1).
// Les morceaux proches du point d'impact partent les premiers.
vec2 piece_t(vec2 c, vec2 impact) {
    float delay = clamp(length(c - impact) * 0.6, 0.0, 0.6) + 0.15 * hash(c * 17.31);
    float x = u_break * 1.75 - delay;
    return vec2(clamp(x, 0.0, 1.0), clamp(x / 0.25 + 1.0, 0.0, 1.0));
}

vec2 vor_seed(vec2 cell) { return cell + 0.15 + 0.7 * hash2(cell); }

// Cellule de Voronoï contenant x, et écart à la frontière la plus proche.
vec3 vor_cell(vec2 x) {
    vec2 i = floor(x);
    float d1 = 1e9, d2 = 1e9;
    vec2 best = i;
    for (int y = -1; y <= 1; y++) {
        for (int k = -1; k <= 1; k++) {
            vec2 c = i + vec2(float(k), float(y));
            vec2 r = x - vor_seed(c);
            float d = dot(r, r);
            if (d < d1) { d2 = d1; d1 = d; best = c; } else if (d < d2) { d2 = d; }
        }
    }
    return vec3(best, sqrt(d2) - sqrt(d1));
}

// q : point (repère hauteurs). Renvoie le point d'origine dans src ; alpha = visibilité,
// crack = lueur des fissures, shade = assombrissement (brûlure, tuile de biais).
void shatter(vec2 q, vec2 impact, out vec2 src, out float alpha, out float crack, out float shade) {
    src = q;
    alpha = 1.0;
    crack = 0.0;
    shade = 1.0;
    float h = 1.0 / max(u_break_size, 1.0);
    if (u_break_style == 1) { // mur de briques qui s'écroule
        float bw = 2.0 * h;
        float row = floor(q.y / h);
        for (int k = 0; k < 7; k++) {
            float r = row - float(k);
            float shift = mod(r, 2.0) * 0.5 * bw;
            float colc = floor((q.x - shift) / bw);
            for (int dc = -1; dc <= 1; dc++) {
                float col = colc + float(dc);
                vec2 c = vec2((col + 0.5) * bw + shift, (r + 0.5) * h);
                vec2 lt = piece_t(c, impact);
                float hh = hash(vec2(col, r) + 0.37);
                vec2 off = vec2((hh - 0.5) * h * lt.x, 5.0 * h * lt.x * lt.x);
                vec2 lp = rot2(-(hh - 0.5) * 1.4 * lt.x) * (q - c - off);
                if (abs(lp.x) < bw * 0.5 && abs(lp.y) < h * 0.5) {
                    src = c + lp;
                    alpha = 1.0 - smoothstep(0.55, 1.0, lt.x);
                    float e = min(bw * 0.5 - abs(lp.x), h * 0.5 - abs(lp.y));
                    crack = lt.y * (1.0 - lt.x) * exp(-e / (h * 0.07));
                    shade = 1.0 - 0.35 * lt.x;
                    return;
                }
            }
        }
        alpha = 0.0;
    } else if (u_break_style == 2) { // verre brisé
        float g = max(u_break_size, 1.0) * 0.6;
        vec2 x = q * g;
        vec2 xc = floor(x);
        vec2 imp = impact * g;
        for (int y = -1; y <= 1; y++) {
            for (int k = -1; k <= 1; k++) {
                vec2 c = xc + vec2(float(k), float(y));
                vec2 sd = vor_seed(c);
                vec2 lt = piece_t(sd / g, impact);
                float hh = hash(c + 0.71);
                vec2 dir = normalize(sd - imp + vec2(1e-4, 0.0));
                vec2 off = (dir * 0.6 + vec2(0.0, 0.6 * lt.x)) * lt.x;
                float scale = 1.0 - 0.7 * lt.x;
                vec2 x0 = sd + rot2(-(hh - 0.5) * 3.0 * lt.x) * (x - sd - off) / scale;
                vec3 v = vor_cell(x0);
                if (v.xy == c) {
                    src = x0 / g;
                    alpha = 1.0 - smoothstep(0.5, 1.0, lt.x);
                    crack = lt.y * (1.0 - lt.x) * exp(-v.z / 0.05);
                    // Reflet sur la tranche du verre.
                    shade = 1.0 + 0.4 * lt.x * (1.0 - smoothstep(0.0, 0.08, v.z));
                    return;
                }
            }
        }
        alpha = 0.0;
    } else if (u_break_style == 3) { // combustion : la forme brûle depuis le point d'impact
        float n = fbm(q * max(u_break_size, 1.0) * 0.4);
        float b = length(q - impact) * 0.6 + n * 0.5 - (u_break * 1.35 - 0.2);
        alpha = smoothstep(0.0, 0.006, b);
        crack = alpha * (1.0 - smoothstep(0.0, 0.05, b)) * 1.5;
        shade = mix(0.15, 1.0, smoothstep(0.0, 0.12, b));
    } else if (u_break_style == 4) { // désintégration en poussière
        float cs = h / 3.0;
        vec2 xc = floor(q / cs);
        for (int k = 0; k < 8; k++) {
            for (int dx = -1; dx <= 0; dx++) {
                vec2 c = xc + vec2(float(dx), float(k));
                vec2 center = (c + 0.5) * cs;
                vec2 lt = piece_t(center, impact);
                float hh = hash(c + 0.13);
                vec2 off = vec2((0.3 + 0.6 * hh) * cs, -(2.0 + 5.0 * hh) * cs) * lt.x;
                float size = 1.0 - 0.85 * lt.x;
                vec2 lp = q - center - off;
                if (max(abs(lp.x), abs(lp.y)) < 0.5 * cs * size) {
                    src = center + lp / size;
                    alpha = 1.0 - smoothstep(0.6, 1.0, lt.x);
                    crack = smoothstep(0.0, 0.3, lt.x) * 0.6 + lt.y * (1.0 - lt.x) * 0.3;
                    return;
                }
            }
        }
        alpha = 0.0;
    } else if (u_break_style == 5) { // tuiles qui se retournent
        float ts = h * 1.5;
        vec2 cell = floor(q / ts);
        vec2 center = (cell + 0.5) * ts;
        vec2 lt = piece_t(center, impact);
        float sx = cos(lt.x * PI);
        vec2 lp = q - center;
        if (sx <= 0.0 || abs(lp.x) > 0.5 * ts * sx) {
            alpha = 0.0;
            return;
        }
        src = center + vec2(lp.x / sx, lp.y);
        vec2 e2 = 0.5 * ts - abs(vec2(lp.x / sx, lp.y));
        crack = lt.y * (1.0 - lt.x) * exp(-min(e2.x, e2.y) / (ts * 0.05));
        shade = 0.45 + 0.55 * sx;
    }
}

#ifdef GEN_PASS
// ------------------------------------------------------------ visuels 3D (lancer de rayons)

float sd_box(vec3 p, vec3 b) {
    vec3 q = abs(p) - b;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}

// Distance à la scène 3D du motif `kind` ; `trap` sert à colorer (palette).
float sdf3d(int kind, vec3 p, float t, inout float trap) {
    float d = u_gen_detail;
    if (kind == 23) { // Mandelbulb
        float power = 6.0 + 2.0 * sin(t * 0.15) + d * 2.0;
        vec3 z = p;
        float dr = 1.0;
        float r = 0.0;
        float tr = 1e9;
        for (int i = 0; i < 7; i++) {
            r = length(z);
            if (r > 2.0) break;
            float theta = acos(clamp(z.z / max(r, 1e-6), -1.0, 1.0)) * power;
            float phi = atan(z.y, z.x) * power;
            dr = pow(r, power - 1.0) * power * dr + 1.0;
            z = pow(r, power) * vec3(sin(theta) * cos(phi), sin(phi) * sin(theta), cos(theta)) + p;
            tr = min(tr, length(z));
        }
        trap = tr;
        return 0.5 * log(max(r, 1e-6)) * r / dr;
    }
    if (kind == 24) { // éponge de Menger
        float dist = sd_box(p, vec3(1.0));
        float s = 1.0;
        int levels = 3 + int(d * 2.0);
        for (int m = 0; m < 5; m++) {
            if (m >= levels) break;
            vec3 a = mod(p * s, 2.0) - 1.0;
            s *= 3.0;
            vec3 r = abs(1.0 - 3.0 * abs(a));
            float c = (min(max(r.x, r.y), min(max(r.y, r.z), max(r.z, r.x))) - 1.0) / s;
            if (c > dist) {
                dist = c;
                trap = float(m) * 0.25;
            }
        }
        return dist;
    }
    if (kind == 25) { // cubes infinis
        vec3 id = floor(p / 2.0);
        vec3 q = mod(p, 2.0) - 1.0;
        float h = hash(id.xy + id.z * 7.31);
        q.xy = rot2(t * (0.5 + h) + h * 6.0) * q.xy;
        q.yz = rot2(t * 0.7 + h * 3.0) * q.yz;
        trap = h;
        return sd_box(q, vec3(0.28 + d * 0.2)) - 0.04;
    }
    if (kind == 26) { // gyroïde
        float sc = 2.0 + d * 2.0;
        vec3 q = p * sc;
        float g = abs(dot(sin(q), cos(q.zxy))) / sc;
        trap = 0.5 + 0.5 * sin(p.z * 0.3 + q.x * 0.2);
        return g - 0.06;
    }
    if (kind == 27) { // kaléidoscope 3D (fractale à replis)
        float s = 1.0;
        float tr = 1e9;
        int it = 5 + int(d * 4.0);
        for (int i = 0; i < 9; i++) {
            if (i >= it) break;
            p = abs(p);
            if (p.x < p.y) p.xy = p.yx;
            if (p.x < p.z) p.xz = p.zx;
            if (p.y < p.z) p.yz = p.zy;
            p.xy = rot2(0.3 + 0.2 * sin(t * 0.2)) * p.xy;
            p.yz = rot2(0.2 + 0.15 * cos(t * 0.17)) * p.yz;
            p = p * 2.0 - vec3(1.0);
            s *= 2.0;
            tr = min(tr, length(p));
        }
        trap = tr * 0.2;
        return (length(p) - 1.2) / s;
    }
    if (kind == 29) { // colonnes torsadées
        vec2 cell = floor(p.xy / 2.0);
        vec3 q = vec3(mod(p.xy, 2.0) - 1.0, p.z);
        float h = hash(cell);
        q.xy = rot2(q.z * (0.4 + d * 0.8) + t * 0.5 + h * 6.0) * q.xy;
        trap = h + q.z * 0.05;
        return sd_box(vec3(q.xy, 0.0), vec3(0.22, 0.22, 1.0)) - 0.03;
    }
    if (kind == 30) { // anneaux 3D imbriqués
        float dist = 1e9;
        int count = 3 + int(d * 3.0);
        for (int k = 0; k < 6; k++) {
            if (k >= count) break;
            float fk = float(k);
            vec3 q = p;
            q.xy = rot2(t * (0.3 + fk * 0.17)) * q.xy;
            q.yz = rot2(t * (0.2 + fk * 0.11) + fk) * q.yz;
            float R = 0.5 + fk * 0.28;
            float dd = length(vec2(length(q.xz) - R, q.y)) - 0.06;
            if (dd < dist) {
                dist = dd;
                trap = fk / 6.0;
            }
        }
        return dist;
    }
    // 28 : sphères infinies
    vec3 id = floor(p / 1.6);
    vec3 q = mod(p, 1.6) - 0.8;
    float h = hash(id.xy + id.z * 3.17);
    trap = h;
    return length(q) - (0.25 + 0.15 * sin(t * 2.0 + h * 6.28) + d * 0.15);
}

// Lance un rayon par pixel : (indice de palette, luminosité), comme les motifs 2D.
vec2 march3d(int kind, vec2 p, float t) {
    float d = u_gen_detail;
    bool fly = kind == 25 || kind == 26 || kind == 28 || kind == 29;
    vec3 ro;
    vec3 ta;
    if (fly) {
        // Vol vers l'avant, en serpentant.
        ro = vec3(0.4 * sin(t * 0.3), 0.3 * cos(t * 0.23), t * 1.2);
        // (Cubes centrés sur les coordonnées impaires, sphères sur ±0,8 : la caméra passe entre eux.)
        ta = ro + vec3(0.3 * sin(t * 0.2), 0.2 * cos(t * 0.27), 1.0);
    } else {
        // Orbite autour de l'objet.
        float dist = kind == 23 ? 3.1 : (kind == 24 ? 4.4 : (kind == 30 ? 5.0 : 4.5));
        float a = t * 0.2;
        ro = vec3(sin(a), 0.45 * sin(t * 0.11), cos(a)) * dist;
        ta = vec3(0.0);
    }
    vec3 fw = normalize(ta - ro);
    vec3 rt = normalize(cross(vec3(0.0, 1.0, 0.0), fw));
    vec3 up = cross(fw, rt);
    vec3 rd = normalize(p.x * rt - p.y * up + 1.6 * fw);
    float far = fly ? 28.0 : 9.0;
    int steps = 48 + int(d * 40.0);
    float dist = 0.0;
    float trap = 0.0;
    bool hit = false;
    int i;
    for (i = 0; i < 88; i++) {
        if (i >= steps) break;
        float h = sdf3d(kind, ro + rd * dist, t, trap);
        if (h < 0.0008 + 0.0015 * dist) {
            hit = true;
            break;
        }
        dist += h * 0.8;
        if (dist > far) break;
    }
    if (!hit) return vec2(t * 0.02, 0.0);
    vec3 q = ro + rd * dist;
    // Normale par différences (tétraèdre).
    float e = 0.0015 + 0.001 * dist;
    float dummy = 0.0;
    vec3 n = vec3(0.0);
    for (int j = 0; j < 4; j++) {
        // Tétraèdre : (1,-1,-1), (-1,-1,1), (-1,1,-1), (1,1,1) ; une seule copie de sdf3d.
        vec3 o = vec3(j == 0 || j == 3 ? 1.0 : -1.0, j >= 2 ? 1.0 : -1.0, j == 1 || j == 3 ? 1.0 : -1.0);
        n += o * sdf3d(kind, q + o * e, t, dummy);
    }
    n = normalize(n);
    vec3 light = normalize(up * 0.8 - rt * 0.5 - fw * 0.6);
    float diff = max(dot(n, light), 0.0);
    float rim = pow(1.0 - max(dot(n, -rd), 0.0), 3.0);
    float ao = 1.0 - float(i) / float(steps);
    float fog = exp(-dist * (fly ? 0.11 : 0.12));
    float bright = ((0.2 + 0.8 * diff) * (0.35 + 0.65 * ao) + rim * 0.4) * fog;
    return vec2(trap * 0.6 + dist * 0.04 + t * 0.02, clamp(bright * 1.3, 0.0, 1.0));
}

vec3 palette(vec3 pal[4], float x) {
    return clamp(pal[0] + pal[1] * cos(2.0 * PI * (pal[2] * x + pal[3])), 0.0, 1.0);
}

vec3 generate(int kind, vec3 pal[4], vec2 uv) {
    vec2 p = (uv - 0.5) * vec2(u_gen_aspect, 1.0) * u_gen_scale;
    vec2 v = pattern(kind, p, u_gen_time);
    return palette(pal, v.x + u_gen_hue) * v.y;
}

// Passe du générateur : fondu entre l'ancien et le nouveau motif. Alpha = part de motif
// (un motif « aucun » laisse passer la vidéo du calque).
void main() {
    vec2 uv = vec2(v_uv.x, 1.0 - v_uv.y); // la texture est lue avec y vers le bas
    vec3 col = vec3(0.0);
    float cover = 0.0;
    vec3 pal[4];
    // Un seul appel à `generate` (dans la boucle) : le code compilé reste petit.
    for (int k = 0; k < 2; k++) {
        int kind = k == 0 ? u_gen_a : u_gen_b;
        float w = k == 0 ? 1.0 - u_gen_fade : u_gen_fade;
        if (kind == 0 || w <= 0.0) continue;
        for (int j = 0; j < 4; j++) pal[j] = k == 0 ? u_pal_a[j] : u_pal_b[j];
        col += generate(kind, pal, uv) * w;
        cover += w;
    }
    out_color = vec4(col, cover);
}
#endif

// Cadrage : coordonnées dans l'image (de taille `size`) d'un point de la surface.
vec2 fit_uv(vec2 uv, vec2 size) {
    if (u_media_rot != 0.0) {
        // Média tourné : on cadre son rectangle englobant, puis on revient dans l'image.
        float as_ = max(u_surface_aspect, 1e-3);
        float am = size.x / max(size.y, 1.0);
        float c = cos(u_media_rot), s = sin(u_media_rot);
        // Étirer : l'image prend les proportions qui remplissent la surface une fois tournée.
        if (u_fit == 0) am = abs(c) >= abs(s) ? as_ : 1.0 / as_;
        float w = abs(c) * am + abs(s);
        float h = abs(s) * am + abs(c);
        float k = u_fit == 2 ? max(as_ / w, 1.0 / h) : min(as_ / w, 1.0 / h);
        vec2 p = (uv - 0.5) * vec2(as_, 1.0) / k;
        p = mat2(c, -s, s, c) * p;
        return p / vec2(am, 1.0) + 0.5;
    }
    if (u_fit == 0) return uv;
    float r = (size.x / max(size.y, 1.0)) / max(u_surface_aspect, 1e-3);
    vec2 s = vec2(1.0);
    if (u_fit == 1) {
        // Ajuster : l'image entière, bandes autour.
        if (r > 1.0) s.y = r; else s.x = 1.0 / r;
    } else {
        // Remplir : toute la surface, image rognée.
        if (r > 1.0) s.x = 1.0 / r; else s.y = r;
    }
    return (uv - 0.5) * s + 0.5;
}

// 1 dans l'image, 0 dans les bandes du mode « Ajuster ».
float in_frame(vec2 uv) {
    // Un média tourné laisse des coins vides, quel que soit le cadrage.
    if (u_fit != 1 && u_media_rot == 0.0) return 1.0;
    return step(0.0, uv.x) * step(uv.x, 1.0) * step(0.0, uv.y) * step(uv.y, 1.0);
}

vec3 fetch(vec2 uv) {
    vec2 ua = fit_uv(uv, u_size_a);
    vec2 ub = fit_uv(uv, u_size_b);
    vec3 a = u_has_a == 1 ? texture(u_tex_a, ua).rgb * in_frame(ua) : vec3(0.0);
    vec3 b = u_has_b == 1 ? texture(u_tex_b, ub).rgb * in_frame(ub) : vec3(0.0);
    vec3 video = mix(a, b, u_mix);
    if (u_gen_on == 0) return video;
    // Visuel généré, calculé à part une fois par image (alpha = part de motif).
    vec4 g = texture(u_gen_tex, uv);
    return mix(video, g.rgb + video * (1.0 - g.a), u_gen_mix);
}

float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }

// Palette de caméra thermique : noir, bleu, violet, rouge, jaune, blanc.
// ------------------------------------------------------------ 3D construite depuis l'image
// Terrain (relief lisse) et pixels 3D (colonnes) : la clarté de l'image donne la hauteur.
// L'image est posée dans le plan z = 0, le relief monte vers la caméra (z négatif).

float height_at(vec2 xy, float asp) {
    vec2 q = vec2(xy.x / asp + 0.5, 0.5 - xy.y);
    return luma(fetch(q)) * u_height3d;
}

void heightfield3d(vec2 sq, out vec2 ouv, out float hit, out float light3) {
    float asp = max(u_surface_aspect, 0.05);
    vec2 p = (sq - 0.5) * vec2(asp, 1.0);
    p.y = -p.y;
    float D = max(u_dist3d, 0.5);
    mat3 R = rot_y(u_rot3d.y) * rot_x(u_rot3d.x);
    mat3 Rt = transpose(R);
    vec3 ro = Rt * vec3(0.0, 0.0, -D);
    vec3 rd = Rt * normalize(vec3(p, D));
    vec3 L = Rt * normalize(vec3(-0.4, 0.5, -1.0));
    hit = 0.0;
    ouv = vec2(0.0);
    light3 = 1.0;
    float H = max(u_height3d, 1e-3);
    vec2 half_size = vec2(asp * 0.5, 0.5);
    if (rd.z <= 1e-4) return; // le rayon s'éloigne du plan de l'image
    float t_end = -ro.z / rd.z;          // plan de base z = 0
    float t_start = max((-H - ro.z) / rd.z, 0.0); // haut du relief
    vec3 n = vec3(0.0, 0.0, -1.0);
    float t = -1.0;
    vec2 cell_uv = vec2(0.0);
    if (u_obj3d == 6) {
        // Terrain : pas réguliers, puis interpolation entre les deux derniers points.
        float prev_t = t_start;
        float prev_gap = 1.0;
        for (int k = 0; k <= 96; k++) {
            float tt = mix(t_start, t_end, float(k) / 96.0);
            vec3 h = ro + rd * tt;
            bool inside = all(lessThanEqual(abs(h.xy), half_size));
            float gap = h.z + (inside ? height_at(h.xy, asp) : -1e3); // >= 0 : sous la surface
            if (inside && gap >= 0.0) {
                float f = prev_gap < 0.0 ? prev_gap / (prev_gap - gap) : 0.0;
                t = mix(prev_t, tt, f);
                break;
            }
            prev_t = tt;
            prev_gap = gap;
        }
        if (t > 0.0) {
            vec3 h = ro + rd * t;
            float e = 0.004;
            float hx = height_at(h.xy + vec2(e, 0.0), asp) - height_at(h.xy - vec2(e, 0.0), asp);
            float hy = height_at(h.xy + vec2(0.0, e), asp) - height_at(h.xy - vec2(0.0, e), asp);
            n = normalize(vec3(hx, hy, 2.0 * e));
            cell_uv = vec2(h.x / asp + 0.5, 0.5 - h.y);
        }
    } else {
        // Pixels 3D : traversée case par case (DDA) d'une grille de colonnes.
        float cs = 1.0 / max(floor(u_voxels), 4.0);
        vec3 h0 = ro + rd * t_start;
        vec2 cell = floor(h0.xy / cs);
        vec2 stp = vec2(rd.x >= 0.0 ? 1.0 : -1.0, rd.y >= 0.0 ? 1.0 : -1.0);
        vec2 ird = 1.0 / max(abs(rd.xy), vec2(1e-6));
        vec2 next = ((cell + max(stp, 0.0)) * cs - h0.xy) * stp * ird + t_start;
        vec2 delta = cs * ird;
        float t_in = t_start;
        float side = -1.0; // axe de la dernière face traversée (0 : x, 1 : y)
        for (int k = 0; k < 200; k++) {
            float t_out = min(min(next.x, next.y), t_end);
            vec2 center = (cell + 0.5) * cs;
            if (all(lessThanEqual(abs(center), half_size))) {
                float hc = height_at(center, asp);
                float z_out = ro.z + rd.z * t_out;
                if (z_out >= -hc) {
                    float z_in = ro.z + rd.z * t_in;
                    if (z_in >= -hc && side >= 0.0) {
                        t = t_in; // flanc de la colonne
                        n = side < 0.5 ? vec3(-stp.x, 0.0, 0.0) : vec3(0.0, -stp.y, 0.0);
                    } else {
                        t = max((-hc - ro.z) / rd.z, t_in); // dessus
                        n = vec3(0.0, 0.0, -1.0);
                    }
                    cell_uv = vec2(center.x / asp + 0.5, 0.5 - center.y);
                    // Joints sombres entre les colonnes, sur le dessus.
                    vec3 hp = ro + rd * t;
                    vec2 local = abs(fract(hp.xy / cs) - 0.5);
                    if (n.z < -0.5) light3 *= 0.75 + 0.25 * smoothstep(0.5, 0.42, max(local.x, local.y));
                    break;
                }
            }
            if (t_out >= t_end) break;
            if (next.x < next.y) {
                t_in = next.x;
                next.x += delta.x;
                cell.x += stp.x;
                side = 0.0;
            } else {
                t_in = next.y;
                next.y += delta.y;
                cell.y += stp.y;
                side = 1.0;
            }
        }
    }
    if (t > 0.0) {
        hit = 1.0;
        ouv = cell_uv;
        if (dot(n, rd) > 0.0) n = -n;
        light3 *= 0.3 + 0.8 * max(dot(n, L), 0.0);
    }
}

vec3 thermal(float t) {
    t = clamp(t, 0.0, 1.0);
    vec3 c = mix(vec3(0.0, 0.0, 0.15), vec3(0.35, 0.0, 0.65), smoothstep(0.0, 0.25, t));
    c = mix(c, vec3(0.95, 0.1, 0.2), smoothstep(0.25, 0.5, t));
    c = mix(c, vec3(1.0, 0.75, 0.0), smoothstep(0.5, 0.75, t));
    return mix(c, vec3(1.0), smoothstep(0.75, 1.0, t));
}

// Caractère ASCII (5×5 points codés dans un entier) au point p (-1..1 dans la case).
float ascii_char(int n, vec2 p) {
    p = floor(p * vec2(-2.5, 2.5) + 2.5);
    if (p.x < 0.0 || p.x > 4.0 || p.y < 0.0 || p.y > 4.0) return 0.0;
    int a = int(p.x + 0.5) + 5 * int(p.y + 0.5);
    return float((n >> a) & 1);
}

vec3 rgb2hsv(vec3 c) {
    vec4 K = vec4(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
    vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
    float d = q.x - min(q.w, q.y);
    float e = 1.0e-10;
    return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

vec3 hsv2rgb(vec3 c) {
    vec4 K = vec4(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}

// Force des contours (filtre de Sobel sur la luminosité), avec un écart de `t` entre les points.
float sobel(vec2 uv, vec2 t) {
    float tl = luma(fetch(uv + t * vec2(-1, -1))), tc = luma(fetch(uv + t * vec2(0, -1))), tr = luma(fetch(uv + t * vec2(1, -1)));
    float ml = luma(fetch(uv + t * vec2(-1, 0))),                                          mr = luma(fetch(uv + t * vec2(1, 0)));
    float bl = luma(fetch(uv + t * vec2(-1, 1))),  bc = luma(fetch(uv + t * vec2(0, 1))),  br = luma(fetch(uv + t * vec2(1, 1)));
    float gx = -tl - 2.0 * ml - bl + tr + 2.0 * mr + br;
    float gy = -tl - 2.0 * tc - tr + bl + 2.0 * bc + br;
    return length(vec2(gx, gy));
}

// Ondulation des contours : somme de sinus dont la phase fait un tour complet (boucle sans saut).
vec2 wobble(vec2 p, float ph) {
    return vec2(
        sin(p.y * 7.0 + ph) + 0.5 * sin(p.x * 13.0 - 2.0 * ph + 1.7) + 0.3 * sin((p.x + p.y) * 23.0 + 3.0 * ph),
        cos(p.x * 8.0 - ph) + 0.5 * cos(p.y * 11.0 + 2.0 * ph + 0.4) + 0.3 * cos((p.x - p.y) * 19.0 - 3.0 * ph)
    ) / 1.8;
}

#ifndef GEN_PASS
void main() {
    if (u_solid == 1) {
        out_color = vec4(0.0, 0.0, 0.0, u_solid_alpha);
        return;
    }
    // 1. Mapping : le maillage déformé fournit directement les coordonnées texture.
    // 0. Destruction : position d'origine du morceau qui recouvre ce point.
    vec2 pos = v_pos;
    vec2 tex_uv = v_uv;
    float piece_a = 1.0, crack = 0.0, shade = 1.0;
    {
        // Passage position de sortie → coordonnées texture, localement (le maillage est lisse).
        // Calculé hors des branches : les dérivées doivent être évaluées partout.
        mat2 P = mat2(dFdx(v_pos), dFdy(v_pos));
        mat2 U = mat2(dFdx(v_uv), dFdy(v_uv));
        if (u_break_style > 0 && u_break > 0.0) {
            float out_aspect = u_res.x / max(u_res.y, 1.0);
            vec2 q = vec2(v_pos.x * out_aspect, v_pos.y);
            vec2 src;
            shatter(q, vec2(u_break_center.x * out_aspect, u_break_center.y), src, piece_a, crack, shade);
            pos = vec2(src.x / out_aspect, src.y);
            if (abs(determinant(P)) > 1e-14) tex_uv = v_uv + U * inverse(P) * (pos - v_pos);
        }
    }
    vec2 mpos = mask_pos(pos);
    float m = mask_value(mpos);

    // Forme du masque : distance au contour, comptée positivement vers la zone visible.
    vec4 field = vec4(0.0);
    float dv = 0.0;
    vec3 light = vec3(0.0);
    vec3 scol = u_shape_color;
    if (u_has_shape == 1) {
        field = texture(u_shape_tex, mpos);
        dv = (u_mask_invert == 1 ? -field.r : field.r) + u_shape_grow;
        // Respiration : la forme est redessinée d'après sa distance, décalée.
        if (u_shape_grow != 0.0) m = smoothstep(-u_feather, u_feather, dv);
        // Position sur le contour (0..1 sur un tour).
        float s = atan(field.b, field.g) / (2.0 * PI);
        // Arc-en-ciel le long du contour, qui tourne avec les comètes.
        vec3 rainbow = 0.5 + 0.5 * cos(2.0 * PI * (s - u_shape_trace_phase + vec3(0.0, 0.33, 0.67)));
        scol = mix(u_shape_color, rainbow, u_shape_rainbow);
        // Néon : trait fin et halo plus large, de part et d'autre du contour.
        float e = abs(dv) / u_shape_width;
        float core = exp(-e * e);
        float glow = core + 0.45 * exp(-e * 0.6);
        light += scol * u_shape_glow * glow;
        if (u_shape_trace > 0.0) {
            // Comètes : tête brillante et traînée le long du contour.
            float x = fract(u_shape_trace_dir * u_shape_trace_count * (u_shape_trace_phase - s));
            float comet = pow(1.0 - x, 7.0);
            light += mix(scol, vec3(1.0), 0.6) * u_shape_trace * comet * glow * 1.8;
        }
        if (u_shape_dashes > 0.0) {
            // Pointillés qui défilent sur le contour.
            float dash = fract(u_shape_dash_count * (s - u_shape_trace_phase));
            light += scol * u_shape_dashes * core * 1.5 * smoothstep(0.4, 0.5, dash) * (1.0 - smoothstep(0.9, 1.0, dash));
        }
        if (u_shape_echo > 0.0) {
            // Échos : copies du contour qui s'éloignent de la forme, de plus en plus pâles.
            float r = -dv * u_shape_freq - u_shape_ripple_phase;
            float line = 1.0 - smoothstep(0.04, 0.04 + fwidth(r) + 0.06, abs(fract(r) - 0.5));
            light += scol * u_shape_echo * line * exp(dv * 4.0) * step(dv, 0.0);
        }
    }
    // Fissures : lumière sur les morceaux encore visibles (orange braise pour la combustion).
    if (crack > 0.0 && u_break_glow > 0.0) {
        vec3 crack_col = u_break_style == 3 ? vec3(1.0, 0.45, 0.1) : scol;
        light += mix(crack_col, vec3(1.0), 0.3) * crack * u_break_glow * m * piece_a;
    }
    m *= piece_a;
    light *= piece_a;
    // Les lumières débordent du masque : elles restent visibles juste autour.
    float light_a = clamp(max(light.r, max(light.g, light.b)), 0.0, 1.0);
    float shape_a = max(m, light_a);
    float coverage = u_opacity * shape_a;
    if (coverage <= 0.0) {
        emit(vec3(0.0), 0.0);
        return;
    }
    vec2 quad_uv = tex_uv;
    vec2 uv = quad_uv;
    if ((u_fit == 1 || u_media_rot != 0.0) && (u_has_a == 1 || u_has_b == 1)) {
        // Ajuster : les bandes autour de l'image laissent voir les calques de derrière.
        float ia = u_has_a == 1 ? in_frame(fit_uv(quad_uv, u_size_a)) : 0.0;
        float ib = u_has_b == 1 ? in_frame(fit_uv(quad_uv, u_size_b)) : 0.0;
        float inside = u_has_b == 1 ? mix(ia, ib, u_mix) : ia;
        float gen = u_gen_on == 1 ? u_gen_mix : 0.0;
        coverage *= mix(inside, 1.0, gen);
        if (coverage <= 0.0) {
            emit(vec3(0.0), 0.0);
            return;
        }
    }

    // 1 bis. Objet 3D : l'image est posée sur un plan, un cube, une sphère ou un cylindre.
    float shade3d = 1.0;
    if (u_obj3d > 0) {
        float hit3d;
        if (u_obj3d >= 6) {
            heightfield3d(quad_uv, uv, hit3d, shade3d);
        } else {
            object3d(quad_uv, uv, hit3d, shade3d);
        }
        coverage *= hit3d;
        if (coverage <= 0.0) {
            emit(vec3(0.0), 0.0);
            return;
        }
    }

    // 2. Géométrie
    if (u_tiles > 1.0) {
        // Mosaïque en miroir : chaque tuile est le reflet de sa voisine (pas de couture).
        uv = 1.0 - abs(1.0 - mod(uv * u_tiles, 2.0));
    }
    if (u_flip_x == 1) uv.x = 1.0 - uv.x;
    if (u_flip_y == 1) uv.y = 1.0 - uv.y;
    if (u_mirror == 1 || u_mirror == 3) uv.x = 0.5 - abs(uv.x - 0.5);
    if (u_mirror == 2 || u_mirror == 3) uv.y = 0.5 - abs(uv.y - 0.5);

    float aspect = u_tex_size.x / max(u_tex_size.y, 1.0);
    vec2 c = uv - 0.5;
    c.x *= aspect;
    float ca = cos(u_rotate), sa = sin(u_rotate);
    c = mat2(ca, -sa, sa, ca) * c;
    c /= max(u_zoom * (1.0 + u_pulse * sin(u_pulse_phase)), 0.01);

    if (u_swirl != 0.0) {
        // Tourbillon : rotation d'autant plus forte qu'on est près du centre.
        float r = length(c);
        float s = u_swirl * (0.6 + 0.4 * sin(u_swirl_phase)) * exp(-r * 2.5);
        float cs = cos(s), sn = sin(s);
        c = mat2(cs, -sn, sn, cs) * c;
    }

    if (u_kaleido >= 2.0) {
        float r = length(c);
        float a = atan(c.y, c.x) + u_kaleido_angle;
        float seg = 2.0 * PI / floor(u_kaleido);
        a = mod(a, seg);
        a = abs(a - seg * 0.5);
        c = r * vec2(cos(a), sin(a));
    }
    // Tunnel : coordonnées polaires (angle, 1/distance) qui défilent vers le centre.
    // Angle et profondeur sont répétés en miroir (pas de couture), et le fond du tunnel
    // laisse voir l'image d'origine au lieu de converger vers un point noir.
    float tunnel_fade = 1.0;
    vec2 polar_uv = uv;
    float tunnel = 0.0;
    if (u_polar > 0.0) {
        float r = length(c);
        float turn = atan(c.y, c.x) / (2.0 * PI) + 0.5;
        float depth = 0.18 / max(r, 0.03) + u_tunnel_phase;
        polar_uv = vec2(1.0 - abs(1.0 - 2.0 * turn), 1.0 - abs(1.0 - mod(depth, 2.0)));
        tunnel = u_polar * smoothstep(0.04, 0.16, r);
        // Légère pénombre vers le fond, pour la profondeur.
        tunnel_fade = mix(1.0, 0.55 + 0.45 * smoothstep(0.04, 0.45, r), tunnel);
    }
    c.x /= aspect;
    uv = c + 0.5;
    if (tunnel > 0.0) {
        uv = mix(uv, polar_uv, tunnel);
    }

    // 3. Distorsions
    if (u_wave_amp > 0.0) {
        uv.x += sin(uv.y * u_wave_freq * 2.0 * PI + u_wave_phase) * u_wave_amp;
        uv.y += cos(uv.x * u_wave_freq * 2.0 * PI + u_wave_phase * 0.8) * u_wave_amp * 0.5;
    }
    if (u_liquid > 0.0) {
        // Liquide : somme d'ondulations croisées qui dérivent dans le temps.
        vec2 p = uv * u_liquid_scale;
        float ph = u_liquid_phase;
        vec2 o = vec2(sin(p.y + ph) + sin(p.x * 1.7 - ph * 1.3), cos(p.x * 1.3 + ph * 0.7) + cos(p.y * 2.1 - ph));
        o += 0.5 * vec2(sin(p.x * 3.1 + p.y * 1.7 + ph * 1.9), cos(p.y * 2.7 - p.x * 1.3 + ph * 1.5));
        uv += o * u_liquid * 0.4;
    }
    if (u_has_shape == 1 && u_shape_wave > 0.0) {
        // Onde qui suit la forme : l'image est poussée perpendiculairement au contour.
        vec2 t = u_shape_texel;
        vec2 g = vec2(
            texture(u_shape_tex, mpos + vec2(t.x, 0.0)).r - texture(u_shape_tex, mpos - vec2(t.x, 0.0)).r,
            texture(u_shape_tex, mpos + vec2(0.0, t.y)).r - texture(u_shape_tex, mpos - vec2(0.0, t.y)).r
        );
        vec2 n = g / max(length(g), 1e-6);
        // Direction lue dans la forme tournée : on la remet dans le sens de la sortie.
        float mc = cos(u_mask_angle), ms = sin(u_mask_angle);
        n = mat2(mc, ms, -ms, mc) * n;
        float w = sin(2.0 * PI * (dv * u_shape_freq - u_shape_ripple_phase));
        uv += n * vec2(t.x / t.y, 1.0) * w * u_shape_wave;
    }
    if (u_fisheye != 0.0) {
        // Fish-eye : bombé (positif) ou pincé (négatif) depuis le centre.
        vec2 c = (uv - 0.5) * vec2(aspect, 1.0);
        float r2 = dot(c, c);
        c *= 1.0 / (1.0 + u_fisheye * r2 * 1.2);
        uv = c / vec2(aspect, 1.0) + 0.5;
    }
    if (u_crt > 0.0) {
        // Écran cathodique bombé.
        vec2 c = uv - 0.5;
        uv = c * (1.0 + dot(c, c) * 0.35 * u_crt) + 0.5;
    }
    float shock_glow = 0.0;
    if (u_shock > 0.0) {
        // Onde de choc : un anneau part du centre et pousse l'image.
        vec2 c = (uv - 0.5) * vec2(aspect, 1.0);
        float r = length(c);
        float d = r - u_shock_phase * 1.3;
        float ring = exp(-d * d * 250.0) * (1.0 - u_shock_phase);
        uv += c / max(r, 1e-4) / vec2(aspect, 1.0) * ring * u_shock * 0.05;
        shock_glow = ring * u_shock * 0.35;
    }
    if (u_droste > 0.0) {
        // Droste : chaque anneau reprend l'image en plus petit, en spirale, et ça zoome.
        vec2 c = (uv - 0.5) * vec2(aspect, 1.0);
        float r = max(length(c), 1e-5);
        float a = atan(c.y, c.x);
        float L = log(4.0);
        float r0 = log(0.5);
        float lr = log(r) + a / (2.0 * PI) * L - u_droste_phase * L;
        float rr = exp(mod(lr - r0, L) + r0 - L * 0.999);
        vec2 dst = vec2(cos(a), sin(a)) * rr / vec2(aspect, 1.0) + 0.5;
        uv = mix(uv, dst, clamp(u_droste, 0.0, 1.0));
    }
    if (u_glitch > 0.0) {
        // Glitch numérique : bandes décalées et blocs qui sautent, plusieurs fois par seconde.
        float tt = floor(u_time * 12.0);
        float band = floor(quad_uv.y * 28.0);
        if (hash(vec2(band, tt)) < u_glitch * 0.45) {
            uv.x += (hash(vec2(band + 7.0, tt)) - 0.5) * 0.25 * u_glitch;
        }
        vec2 blk = floor(quad_uv * vec2(9.0, 6.0));
        if (hash(blk + tt * 1.7) < u_glitch * 0.12) {
            uv += (hash2(blk + tt) - 0.5) * 0.15;
        }
    }
    float fold_shade = 1.0;
    if (u_fold > 0.0) {
        // Accordéon : l'image pliée en panneaux, vus en perspective.
        float nf = max(floor(u_fold_count), 2.0);
        float ang = u_fold * 1.1;
        float x = (quad_uv.x - 0.5) / cos(ang) + 0.5;
        coverage *= step(0.0, x) * step(x, 1.0);
        float sx = x * nf;
        bool even = mod(floor(sx), 2.0) < 1.0;
        float f = fract(sx);
        float depth = (even ? f : 1.0 - f) * sin(ang) / nf;
        uv.x += x - quad_uv.x;
        uv.y = 0.5 + (uv.y - 0.5) / (1.0 + depth * 2.0);
        fold_shade = even ? 1.0 - 0.35 * u_fold : 1.0 + 0.08 * u_fold;
    }
    float flag_shade = 1.0;
    if (u_flag > 0.0) {
        // Drapeau : vagues qui partent du bord gauche (le mât) et grandissent vers la droite.
        float reach = 0.25 + 0.75 * clamp(uv.x, 0.0, 1.0);
        float a = uv.x * 7.0 + uv.y * 2.0 - u_flag_phase;
        uv.y += sin(a) * reach * u_flag * 0.04;
        uv.x += cos(a) * reach * u_flag * 0.01;
        flag_shade = 1.0 + cos(a) * reach * u_flag * 0.45;
    }
    float relief_shade = 1.0;
    if (u_relief > 0.0) {
        // Relief : la luminosité devient une hauteur ; l'image se décale avec le point de vue
        // et s'éclaire de biais.
        vec2 tx = 2.0 / max(u_tex_size, vec2(1.0));
        float hc = luma(fetch(uv));
        float hx = luma(fetch(uv + vec2(tx.x, 0.0))) - luma(fetch(uv - vec2(tx.x, 0.0)));
        float hy = luma(fetch(uv + vec2(0.0, tx.y))) - luma(fetch(uv - vec2(0.0, tx.y)));
        uv += u_relief_dir * (hc - 0.5) * u_relief * 0.02;
        vec3 nr = normalize(vec3(-hx, -hy, 0.25 / u_relief));
        vec3 lr = normalize(vec3(-0.5, -0.5, 0.7));
        relief_shade = mix(1.0, max(dot(nr, lr), 0.0) / lr.z, clamp(u_relief, 0.0, 1.0));
    }
    if (u_pixelate > 0.0) {
        float cells = mix(320.0, 6.0, u_pixelate);
        vec2 grid = vec2(cells, cells / aspect);
        uv = (floor(uv * grid) + 0.5) / grid;
    }
    if (u_hexagon > 0.0) {
        // Mosaïque hexagonale : chaque alvéole prend la couleur de son centre.
        float sz = mix(0.004, 0.06, u_hexagon);
        vec2 hp = uv * vec2(aspect, 1.0) / sz;
        vec2 rr = vec2(1.0, 1.7320508);
        vec2 ha = mod(hp, rr) - rr * 0.5;
        vec2 hb = mod(hp - rr * 0.5, rr) - rr * 0.5;
        vec2 g = dot(ha, ha) < dot(hb, hb) ? ha : hb;
        uv = (hp - g) * sz / vec2(aspect, 1.0);
    }

    // 4. Échantillonnage (avec décalage RVB)
    vec3 col;
    if (u_rgb_shift > 0.0) {
        vec2 off = vec2(u_rgb_shift, u_rgb_shift * 0.3);
        col = vec3(fetch(uv + off).r, fetch(uv).g, fetch(uv - off).b);
    } else {
        col = fetch(uv);
    }
    if (u_anaglyph > 0.0) {
        // Anaglyphe : rouge et cyan décalés, d'autant plus que le point est clair (fausse profondeur).
        vec2 off = vec2(u_anaglyph * (0.5 + luma(col)), 0.0);
        col = vec3(fetch(uv - off).r, fetch(uv + off).gb);
    }
    if (u_blur > 0.0 || u_bloom > 0.0 || u_zoom_blur > 0.0) {
        // Flou (disque), halo des zones claires, et flou de zoom (le long des rayons) :
        // une seule boucle de lectures.
        vec3 acc = vec3(0.0);
        vec3 bright = vec3(0.0);
        vec3 zacc = vec3(0.0);
        float rad = mix(0.002, 0.03, max(u_blur, u_bloom * 0.8));
        for (int k = 0; k < 12; k++) {
            float fk = float(k);
            float a = fk * 2.39996;
            vec2 o = vec2(cos(a), sin(a)) * sqrt((fk + 0.5) / 12.0) * rad * vec2(1.0 / aspect, 1.0);
            vec2 zuv = 0.5 + (uv - 0.5) * (1.0 - u_zoom_blur * 0.18 * fk / 11.0);
            // Une seule lecture par tour : disque ou rayon selon l'effet le plus fort.
            bool zoom = u_zoom_blur > max(u_blur, u_bloom);
            vec3 sc = fetch(zoom ? zuv : uv + o);
            if (zoom) zacc += sc; else { acc += sc; bright += max(sc - 0.55, 0.0); }
        }
        if (u_zoom_blur > max(u_blur, u_bloom)) {
            col = mix(col, zacc / 12.0, clamp(u_zoom_blur * 1.5, 0.0, 1.0));
        } else {
            col = mix(col, acc / 12.0, clamp(u_blur, 0.0, 1.0));
            col += bright / 12.0 * u_bloom * 3.0;
        }
    }
    if (u_ascii > 0.0) {
        // ASCII : chaque case devient un caractère, plus « plein » quand la case est claire.
        float cols = mix(160.0, 40.0, clamp(u_ascii, 0.0, 1.0));
        vec2 grid = vec2(cols, cols / max(u_surface_aspect, 0.05) * 1.25);
        vec2 cell = floor(quad_uv * grid);
        vec2 center = (cell + 0.5) / grid;
        vec3 cc = fetch(uv + center - quad_uv);
        float gray = luma(cc);
        int n = 4096;
        if (gray > 0.2) n = 65600;
        if (gray > 0.3) n = 332772;
        if (gray > 0.4) n = 15255086;
        if (gray > 0.5) n = 23385164;
        if (gray > 0.6) n = 15252014;
        if (gray > 0.7) n = 13199452;
        if (gray > 0.8) n = 11512810;
        vec2 local = fract(quad_uv * grid) * 2.0 - 1.0;
        col = cc * ascii_char(n, local) * 1.3;
    }

    if (u_echo > 0.0) {
        // Écho : copies de plus en plus petites, superposées en mode « écran ».
        for (int k = 1; k <= 3; k++) {
            vec2 e = 0.5 + (uv - 0.5) * (1.0 - 0.08 * float(k));
            vec3 ec = fetch(e) * (u_echo * (1.0 - 0.22 * float(k)));
            col = 1.0 - (1.0 - col) * (1.0 - ec);
        }
    }

    if (u_edge > 0.0) {
        float e = clamp(sobel(uv, 1.0 / max(u_tex_size, vec2(1.0))) * 2.0, 0.0, 1.0);
        col = mix(col, vec3(e) * mix(vec3(1.0), col * 2.0, 0.5), u_edge);
    }

    // 5. Couleur
    col += u_brightness;
    col = (col - 0.5) * u_contrast + 0.5;
    float l = luma(col);
    col = mix(vec3(l), col, u_saturation);
    float hue_shift = u_hue + u_rainbow * (quad_uv.x * 0.6 + quad_uv.y * 0.4);
    if (hue_shift != 0.0) {
        vec3 hsv = rgb2hsv(clamp(col, 0.0, 1.0));
        hsv.x = fract(hsv.x + hue_shift);
        col = hsv2rgb(hsv);
    }
    if (u_acid > 0.0) {
        // Palette acide : la luminosité est remplacée par un arc-en-ciel qui tourne.
        float t = luma(clamp(col, 0.0, 1.0)) * 1.5 + u_acid_phase;
        vec3 pal = 0.5 + 0.5 * cos(2.0 * PI * (t + vec3(0.0, 0.33, 0.67)));
        col = mix(col, pal, u_acid);
    }
    if (u_solarize > 0.0) {
        vec3 sol = mix(col, 1.0 - col, step(0.5, col)) * 2.0;
        col = mix(col, sol, u_solarize);
    }
    if (u_posterize >= 2.0) {
        col = floor(col * u_posterize) / (u_posterize - 1.0);
    }
    col = mix(col, 1.0 - col, u_invert);
    col = mix(col, luma(col) * u_tint * 1.5, u_tint_amount);
    if (u_thermal > 0.0) {
        col = mix(col, thermal(luma(clamp(col, 0.0, 1.0)) * 1.1), clamp(u_thermal, 0.0, 1.0));
    }
    if (u_halftone > 0.0) {
        // Trame de points façon BD : gros points d'encre dans les zones sombres.
        float cells = mix(140.0, 28.0, clamp(u_halftone, 0.0, 1.0));
        vec2 hp = rot2(0.785) * (quad_uv * vec2(u_surface_aspect, 1.0) * cells);
        float rad = sqrt(1.0 - clamp(luma(col), 0.0, 1.0)) * 0.62;
        float ink = 1.0 - smoothstep(rad - 0.08, rad + 0.08, length(fract(hp) - 0.5));
        vec3 paper = vec3(0.97, 0.95, 0.88);
        col = mix(col, mix(paper, col * 0.45, ink), smoothstep(0.0, 0.15, u_halftone));
    }
    if (u_crt > 0.0) {
        // Lignes de balayage et grille de phosphores RVB.
        float sl = 0.7 + 0.3 * sin(gl_FragCoord.y * PI * 0.5);
        int m = int(mod(gl_FragCoord.x, 3.0));
        vec3 mask = vec3(m == 0 ? 1.2 : 0.85, m == 1 ? 1.2 : 0.85, m == 2 ? 1.2 : 0.85);
        col *= mix(vec3(1.0), sl * mask * 1.15, clamp(u_crt, 0.0, 1.0));
    }
    if (u_grain > 0.0) {
        col += (hash(gl_FragCoord.xy + fract(u_time * 7.3) * 100.0) - 0.5) * u_grain * 0.35;
    }
    col += shock_glow;

    if (u_vignette > 0.0) {
        float d = length(quad_uv - 0.5) * 1.4142;
        col *= 1.0 - smoothstep(1.0 - u_vignette, 1.0 + 0.0001, d) * clamp(u_vignette * 1.5, 0.0, 1.0);
    }
    col *= tunnel_fade * shade * shade3d * relief_shade * flag_shade * fold_shade;
    if (u_lines_glow > 0.0 || u_lines_dim > 0.0) {
        // Contours animés : les contours de l'image s'illuminent, ondulent et sont parcourus
        // par des vagues de lumière. L'image elle-même reste en place (et peut être assombrie).
        vec2 t = 1.0 / max(u_tex_size, vec2(1.0));
        vec2 luv = uv + wobble(uv * vec2(aspect, 1.0), u_lines_wobble_phase) * u_lines_wobble * vec2(1.0, aspect);
        float th = u_lines_threshold;
        float core = smoothstep(th, th + 0.25, sobel(luv, t) * 1.5);
        float halo = smoothstep(th * 0.6, th * 0.6 + 0.4, sobel(luv, t * max(u_lines_width, 1.0)) * 1.5);
        float lit = core + 0.6 * halo * halo;
        // Vagues qui partent du centre de l'image et allument les contours à leur passage.
        float d = length((quad_uv - 0.5) * vec2(u_tex_size.x / max(u_tex_size.y, 1.0), 1.0));
        float wave = pow(0.5 + 0.5 * sin(2.0 * PI * (d * 3.0 - u_lines_flow_phase)), 4.0);
        lit *= mix(1.0, 0.15 + 2.2 * wave, clamp(u_lines_flow, 0.0, 1.0));
        vec3 rainbow = 0.5 + 0.5 * cos(2.0 * PI * (quad_uv.x * 0.7 + quad_uv.y * 0.5 - u_lines_flow_phase + vec3(0.0, 0.33, 0.67)));
        vec3 lcol = mix(u_lines_color, rainbow, u_lines_rainbow);
        col = col * (1.0 - u_lines_dim) + mix(lcol, vec3(1.0), 0.35 * core) * lit * u_lines_glow;
    }
    if (u_has_shape == 1 && u_shape_ripple > 0.0) {
        // Lignes concentriques qui épousent la forme et avancent vers l'intérieur.
        float r = dv * u_shape_freq - u_shape_ripple_phase;
        float fw = fwidth(r);
        float line = 1.0 - smoothstep(0.06, 0.06 + fw + 0.08, abs(fract(r) - 0.5));
        col += scol * line * u_shape_ripple;
    }
    if (u_has_shape == 1 && u_shape_inner > 0.0) {
        // Lueur intérieure : la couleur monte depuis les bords vers le centre.
        col += scol * u_shape_inner * exp(-max(dv, 0.0) / (u_shape_width * 6.0));
    }
    if (u_strobe > 0.0 && fract(u_time * u_strobe) > 0.5) {
        col = vec3(0.0);
        light = vec3(0.0);
    }
    // Lumières de contour, ajoutées à l'image (seules, à l'extérieur du masque).
    col = (col * m + light) / max(shape_a, 1e-4);

    col = clamp(col, 0.0, 1.0);

    // 6. Mire de calibration du mapping
    if (u_grid == 1) {
        vec2 g = abs(fract(quad_uv * 8.0 + 0.5) - 0.5) / fwidth(quad_uv * 8.0);
        float line = 1.0 - clamp(min(g.x, g.y), 0.0, 1.0);
        vec2 border = min(quad_uv, 1.0 - quad_uv) / fwidth(quad_uv);
        line = max(line, 1.0 - clamp(min(border.x, border.y) / 3.0, 0.0, 1.0));
        float diag = abs(quad_uv.x - quad_uv.y) / fwidth(quad_uv.x - quad_uv.y);
        float diag2 = abs(quad_uv.x + quad_uv.y - 1.0) / fwidth(quad_uv.x + quad_uv.y);
        line = max(line, 1.0 - clamp(min(diag, diag2), 0.0, 1.0));
        col = mix(col * 0.6, vec3(0.2, 1.0, 0.6), line);
    }

    emit(col, coverage);
}
#endif
"#;

impl Renderer {
    unsafe fn compile(gl: &glow::Context, header: &str) -> Result<glow::Program, String> {
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, src) in [
                (glow::VERTEX_SHADER, VERTEX_SHADER),
                (glow::FRAGMENT_SHADER, FRAGMENT_SHADER),
            ] {
                let shader = gl.create_shader(kind)?;
                gl.shader_source(shader, &format!("{header}{src}"));
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return Err(format!("shader : {}", gl.get_shader_info_log(shader)));
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(format!("link : {}", gl.get_program_info_log(program)));
            }
            for s in shaders {
                gl.detach_shader(program, s);
                gl.delete_shader(s);
            }
            Ok(program)
        }
    }

    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let program = Self::compile(gl, "#version 330\n")?;
            let gen_program = Self::compile(gl, "#version 330\n#define GEN_PASS\n")?;

            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = 4 * std::mem::size_of::<f32>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 2 * std::mem::size_of::<f32>() as i32);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            let black = Self::create_texture(gl)?;
            gl.bind_texture(glow::TEXTURE_2D, None);

            Ok(Self {
                program,
                gen_program,
                gen_targets: HashMap::new(),
                capture_target: None,
                vao,
                vbo,
                black,
                textures: HashMap::new(),
                mask_textures: HashMap::new(),
                shape_textures: HashMap::new(),
            })
        }
    }

    unsafe fn create_texture(gl: &glow::Context) -> Result<glow::Texture, String> {
        unsafe {
            let t = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::MIRRORED_REPEAT as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::MIRRORED_REPEAT as i32);
            // 1 pixel noir pour que la texture soit valide avant la première frame.
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                1,
                1,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&[0, 0, 0, 255])),
            );
            Ok(t)
        }
    }

    pub fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_program(self.gen_program);
            if let Some(t) = self.capture_target.take() {
                gl.delete_framebuffer(t.fbo);
                gl.delete_texture(t.texture);
            }
            for (_, t) in self.gen_targets.drain() {
                gl.delete_framebuffer(t.fbo);
                gl.delete_texture(t.texture);
            }
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
            gl.delete_texture(self.black);
            for (_, t) in self.textures.drain() {
                gl.delete_texture(t.texture);
            }
            for (_, t) in self.mask_textures.drain().chain(self.shape_textures.drain()) {
                gl.delete_texture(t);
            }
        }
    }

    /// Texture du masque (créée et envoyée au GPU à la première utilisation de cette image).
    fn mask_texture(&mut self, gl: &glow::Context, m: &MaskDraw) -> Option<glow::Texture> {
        if let Some(t) = self.mask_textures.get(&m.key) {
            return Some(*t);
        }
        unsafe {
            let t = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                m.width as i32,
                m.height as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&m.data)),
            );
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            self.mask_textures.insert(m.key, t);
            Some(t)
        }
    }

    /// Texture du champ de forme (flottante : la distance doit rester précise).
    fn shape_texture(&mut self, gl: &glow::Context, s: &ShapeDraw) -> Option<glow::Texture> {
        if let Some(t) = self.shape_textures.get(&s.key) {
            return Some(*t);
        }
        unsafe {
            let t = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA16F as i32,
                s.width as i32,
                s.height as i32,
                0,
                glow::RGBA,
                glow::FLOAT,
                glow::PixelUnpackData::Slice(Some(as_bytes(&s.data))),
            );
            self.shape_textures.insert(s.key, t);
            Some(t)
        }
    }

    /// Envoie la dernière frame d'une source au GPU si elle a changé.
    /// Renvoie la texture et sa taille, ou None tant qu'aucune image n'est disponible.
    fn upload(&mut self, gl: &glow::Context, src: &SourceRef) -> Option<(glow::Texture, (u32, u32))> {
        let guard = src.frame.lock().unwrap();
        let frame = guard.as_ref()?;
        let entry = match self.textures.entry(src.key) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => e.insert(VideoTexture {
                texture: unsafe { Self::create_texture(gl).ok()? },
                frame_id: 0,
                size: (1, 1),
            }),
        };
        if entry.frame_id != frame.id {
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, Some(entry.texture));
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
                let size = (frame.width, frame.height);
                let pixels = glow::PixelUnpackData::Slice(Some(&frame.data));
                if entry.size == size {
                    gl.tex_sub_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        0,
                        0,
                        size.0 as i32,
                        size.1 as i32,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        pixels,
                    );
                } else {
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        size.0 as i32,
                        size.1 as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        pixels,
                    );
                    entry.size = size;
                }
            }
            entry.frame_id = frame.id;
        }
        Some((entry.texture, entry.size))
    }

    /// Libère les textures des lecteurs qui ne sont plus utilisés.
    fn collect_garbage(&mut self, gl: &glow::Context, p: &DrawParams) {
        let alive: HashSet<u64> = p
            .layers
            .iter()
            .flat_map(|l| [&l.a, &l.b])
            .flatten()
            .map(|s| s.key)
            .collect();
        self.textures.retain(|key, t| {
            let keep = alive.contains(key);
            if !keep {
                unsafe { gl.delete_texture(t.texture) };
            }
            keep
        });
        let masks: HashSet<u64> = p.layers.iter().filter_map(|l| l.mask.as_ref()).map(|m| m.key).collect();
        self.mask_textures.retain(|key, t| {
            let keep = masks.contains(key);
            if !keep {
                unsafe { gl.delete_texture(*t) };
            }
            keep
        });
        self.gen_targets.retain(|k, t| {
            let keep = p.layers.get(*k).is_some_and(|l| l.generator.is_active());
            if !keep {
                unsafe {
                    gl.delete_framebuffer(t.fbo);
                    gl.delete_texture(t.texture);
                }
            }
            keep
        });
        let shapes: HashSet<u64> = p.layers.iter().filter_map(|l| l.shape.as_ref()).map(|s| s.key).collect();
        self.shape_textures.retain(|key, t| {
            let keep = shapes.contains(key);
            if !keep {
                unsafe { gl.delete_texture(*t) };
            }
            keep
        });
    }

    /// Visuels générés : chaque calque qui en a un le reçoit dans sa texture, une seule fois
    /// par image (l'aperçu et la sortie partagent le résultat).
    unsafe fn render_generators(&mut self, gl: &glow::Context, p: &DrawParams) {
        unsafe {
            if !p.layers.iter().any(|l| l.generator.is_active()) {
                return;
            }
            // État d'egui à remettre en place ensuite.
            let prev_fbo = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            let mut prev_vp = [0i32; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut prev_vp);
            let scissor = gl.is_enabled(glow::SCISSOR_TEST);
            let blend = gl.is_enabled(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.use_program(Some(self.gen_program));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let full: [f32; 24] = [
                0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, //
                0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0,
            ];
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&full), glow::STREAM_DRAW);
            for (k, layer) in p.layers.iter().enumerate() {
                let g = &layer.generator;
                if !g.is_active() {
                    continue;
                }
                // Les visuels 3D (lancer de rayons) sont calculés en plus basse définition.
                let is_3d = |pt: Pattern| pt as i32 >= Pattern::Mandelbulb as i32;
                let h: u32 = if is_3d(g.from) || is_3d(g.to) { 540 } else { 900 };
                let size = (((h as f32) * g.aspect.clamp(0.2, 5.0)) as u32, h);
                let Some(target) = self.gen_target(gl, k, size) else {
                    continue;
                };
                if target.last == Some(*g) {
                    continue;
                }
                target.last = Some(*g);
                let fbo = target.fbo;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.viewport(0, 0, size.0 as i32, size.1 as i32);
                Self::set_gen_uniforms(gl, self.gen_program, g);
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, prev_fbo);
            gl.viewport(prev_vp[0], prev_vp[1], prev_vp[2], prev_vp[3]);
            if scissor {
                gl.enable(glow::SCISSOR_TEST);
            }
            if blend {
                gl.enable(glow::BLEND);
            }
        }
    }

    /// Texture + framebuffer du visuel généré du calque `k`, (re)créés à la bonne taille.
    unsafe fn gen_target(&mut self, gl: &glow::Context, k: usize, size: (u32, u32)) -> Option<&mut GenTarget> {
        unsafe {
            if self.gen_targets.get(&k).is_some_and(|t| t.size != size) {
                let t = self.gen_targets.remove(&k).unwrap();
                gl.delete_framebuffer(t.fbo);
                gl.delete_texture(t.texture);
            }
            if !self.gen_targets.contains_key(&k) {
                let texture = Self::create_texture(gl).ok()?;
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
                let fbo = gl.create_framebuffer().ok()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);
                self.gen_targets.insert(k, GenTarget { fbo, texture, size, last: None });
            }
            self.gen_targets.get_mut(&k)
        }
    }

    unsafe fn set_gen_uniforms(gl: &glow::Context, program: glow::Program, g: &GenDraw) {
        unsafe {
            let loc = |name: &str| gl.get_uniform_location(program, name);
            let f = |name: &str, v: f32| gl.uniform_1_f32(loc(name).as_ref(), v);
            let i = |name: &str, v: i32| gl.uniform_1_i32(loc(name).as_ref(), v);
            i("u_gen_a", g.from as i32);
            i("u_gen_b", g.to as i32);
            f("u_gen_fade", g.fade.clamp(0.0, 1.0));
            f("u_gen_time", g.time);
            f("u_gen_hue", g.hue);
            f("u_gen_scale", g.scale.max(0.05));
            f("u_gen_detail", g.detail.clamp(0.0, 1.0));
            f("u_gen_aspect", g.aspect.max(0.1));
            for (name, pal) in [("u_pal_a", g.from_palette), ("u_pal_b", g.to_palette)] {
                let flat: Vec<f32> = pal.coefs().iter().flatten().copied().collect();
                gl.uniform_3_f32_slice(loc(&format!("{name}[0]")).as_ref(), &flat);
            }
        }
    }

    pub fn paint(&mut self, gl: &glow::Context, info: &egui::PaintCallbackInfo, p: &DrawParams) {
        self.collect_garbage(gl, p);
        unsafe { self.render_generators(gl, p) };

        unsafe {
            // Fond noir limité au rectangle du callback.
            let vp = info.viewport_in_pixels();
            let clip = info.clip_rect_in_pixels();
            let left = vp.left_px.max(clip.left_px);
            let bottom = vp.from_bottom_px.max(clip.from_bottom_px);
            let right = (vp.left_px + vp.width_px).min(clip.left_px + clip.width_px);
            let top = (vp.from_bottom_px + vp.height_px).min(clip.from_bottom_px + clip.height_px);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(left, bottom, (right - left).max(0), (top - bottom).max(0));
            self.draw_scene(gl, (vp.width_px as f32, vp.height_px as f32), p);
        }
    }

    /// Rend la sortie hors écran à la taille `size` et renvoie l'image (RGBA, lignes du bas
    /// vers le haut). Sert à l'enregistrement vidéo.
    pub fn capture(&mut self, gl: &glow::Context, size: (u32, u32), p: &DrawParams) -> Option<Vec<u8>> {
        self.collect_garbage(gl, p);
        unsafe {
            self.render_generators(gl, p);
            let prev_fbo = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            let prev_read = gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
            let mut prev_vp = [0i32; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut prev_vp);
            let scissor = gl.is_enabled(glow::SCISSOR_TEST);
            let mut prev_box = [0i32; 4];
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut prev_box);

            if self.capture_target.as_ref().is_some_and(|t| t.size != size) {
                let t = self.capture_target.take().unwrap();
                gl.delete_framebuffer(t.fbo);
                gl.delete_texture(t.texture);
            }
            if self.capture_target.is_none() {
                let texture = Self::create_texture(gl).ok()?;
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
                let fbo = gl.create_framebuffer().ok()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);
                self.capture_target = Some(GenTarget { fbo, texture, size, last: None });
            }
            let fbo = self.capture_target.as_ref()?.fbo;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.viewport(0, 0, size.0 as i32, size.1 as i32);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(0, 0, size.0 as i32, size.1 as i32);
            self.draw_scene(gl, (size.0 as f32, size.1 as f32), p);

            let mut pixels = vec![0u8; size.0 as usize * size.1 as usize * 4];
            gl.read_pixels(
                0,
                0,
                size.0 as i32,
                size.1 as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );

            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, prev_fbo);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, prev_read);
            gl.viewport(prev_vp[0], prev_vp[1], prev_vp[2], prev_vp[3]);
            gl.scissor(prev_box[0], prev_box[1], prev_box[2], prev_box[3]);
            if !scissor {
                gl.disable(glow::SCISSOR_TEST);
            }
            Some(pixels)
        }
    }

    /// Compose les calques dans la cible actuelle (taille `res` en pixels ; le découpage
    /// est déjà réglé par l'appelant).
    unsafe fn draw_scene(&mut self, gl: &glow::Context, res: (f32, f32), p: &DrawParams) {
        unsafe {
            if p.white {
                gl.clear_color(1.0, 1.0, 1.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                return;
            }
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.enable(glow::BLEND);
            let program = self.program;
            let loc = |name: &str| gl.get_uniform_location(program, name);
            gl.uniform_1_i32(loc("u_solid").as_ref(), 0);
            gl.uniform_2_f32(loc("u_res").as_ref(), res.0, res.1);

            for (k, layer) in p.layers.iter().enumerate() {
                self.paint_layer(gl, k, layer, p.time);
            }

            // Master / blackout : voile noir par-dessus la composition.
            if p.master < 1.0 {
                gl.use_program(Some(self.program));
                gl.uniform_1_i32(loc("u_solid").as_ref(), 1);
                gl.uniform_1_f32(loc("u_solid_alpha").as_ref(), 1.0 - p.master.clamp(0.0, 1.0));
                gl.blend_equation(glow::FUNC_ADD);
                gl.blend_func_separate(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::ZERO, glow::ONE);
                let full: [f32; 24] = [
                    0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, //
                    0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0,
                ];
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&full), glow::STREAM_DRAW);
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
            }

            gl.active_texture(glow::TEXTURE4);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE3);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }

    unsafe fn paint_layer(&mut self, gl: &glow::Context, k: usize, layer: &LayerDraw, time: f32) {
        let a = layer.a.as_ref().and_then(|s| self.upload(gl, s));
        let b = layer.b.as_ref().and_then(|s| self.upload(gl, s));
        let mask = layer.mask.as_ref().and_then(|m| Some((m.invert, self.mask_texture(gl, m)?)));
        let shape = layer.shape.as_ref().filter(|_| mask.is_some()).and_then(|s| Some((s, self.shape_texture(gl, s)?)));
        let gen_on = layer.generator.is_active();
        if a.is_none() && b.is_none() && !layer.grid && !gen_on {
            return;
        }
        let fx = &layer.effects;
        // Sans vidéo, le générateur prend les proportions de la sortie.
        let gen_size = ((1000.0 * layer.generator.aspect.max(0.1)) as u32, 1000);
        let tex_size = a.or(b).map_or(if gen_on { gen_size } else { (1, 1) }, |(_, size)| size);

        unsafe {
            let loc = |name: &str| gl.get_uniform_location(self.program, name);
            let f = |name: &str, v: f32| gl.uniform_1_f32(loc(name).as_ref(), v);
            let i = |name: &str, v: i32| gl.uniform_1_i32(loc(name).as_ref(), v);

            for (unit, tex, name) in [(0u32, a, "u_tex_a"), (1, b, "u_tex_b")] {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex.map_or(self.black, |(t, _)| t)));
                i(name, unit as i32);
            }
            gl.active_texture(glow::TEXTURE0);

            i("u_has_a", a.is_some() as i32);
            i("u_has_b", b.is_some() as i32);
            // Tant que B n'a pas d'image on reste sur A ; sans A : fondu depuis le noir.
            f("u_mix", if b.is_some() { layer.mix } else { 0.0 });
            match mask {
                Some((invert, tex)) => {
                    gl.active_texture(glow::TEXTURE2);
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    gl.active_texture(glow::TEXTURE0);
                    i("u_mask_tex", 2);
                    i("u_has_mask", 1);
                    i("u_mask_invert", invert as i32);
                }
                None => i("u_has_mask", 0),
            }
            match shape {
                Some((s, tex)) => {
                    gl.active_texture(glow::TEXTURE3);
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    gl.active_texture(glow::TEXTURE0);
                    i("u_shape_tex", 3);
                    i("u_has_shape", 1);
                    gl.uniform_2_f32(
                        loc("u_shape_texel").as_ref(),
                        1.0 / s.width.max(1) as f32,
                        1.0 / s.height.max(1) as f32,
                    );
                    f("u_feather", s.feather.max(0.5 / s.height.max(1) as f32));
                }
                None => i("u_has_shape", 0),
            }
            f("u_time", time);
            gl.uniform_2_f32(loc("u_tex_size").as_ref(), tex_size.0 as f32, tex_size.1 as f32);
            let size_of = |t: Option<(glow::Texture, (u32, u32))>| t.map_or(tex_size, |(_, s)| s);
            let (sa, sb) = (size_of(a), size_of(b));
            gl.uniform_2_f32(loc("u_size_a").as_ref(), sa.0 as f32, sa.1 as f32);
            gl.uniform_2_f32(loc("u_size_b").as_ref(), sb.0 as f32, sb.1 as f32);
            i("u_fit", layer.fit as i32);
            f("u_media_rot", layer.media_rotate.rem_euclid(360.0).to_radians());
            f("u_surface_aspect", layer.surface_aspect);
            i("u_grid", layer.grid as i32);
            f("u_opacity", layer.opacity.clamp(0.0, 1.0));
            i("u_blend", layer.blend as i32);

            f("u_brightness", fx.brightness);
            f("u_contrast", fx.contrast);
            f("u_saturation", fx.saturation);
            f("u_hue", (fx.hue + layer.phases.hue).fract());
            f("u_invert", fx.invert);
            f("u_posterize", fx.posterize);
            gl.uniform_3_f32(loc("u_tint").as_ref(), fx.tint[0], fx.tint[1], fx.tint[2]);
            f("u_tint_amount", fx.tint_amount);
            f("u_edge", fx.edge);
            f("u_vignette", fx.vignette);
            f("u_zoom", fx.zoom);
            f("u_rotate", (fx.rotate + layer.phases.rotate).to_radians());
            i("u_flip_x", fx.flip_x as i32);
            i("u_flip_y", fx.flip_y as i32);
            i("u_mirror", fx.mirror as i32);
            f("u_kaleido", fx.kaleido);
            f("u_kaleido_angle", layer.phases.kaleido.to_radians());
            f("u_pixelate", fx.pixelate);
            f("u_rgb_shift", fx.rgb_shift);
            f("u_wave_amp", fx.wave_amp);
            f("u_wave_freq", fx.wave_freq);
            f("u_wave_phase", layer.phases.wave);
            f("u_strobe", fx.strobe);
            let ph = &layer.phases;
            f("u_swirl", fx.swirl);
            f("u_swirl_phase", ph.swirl);
            f("u_polar", fx.polar.clamp(0.0, 1.0));
            f("u_tunnel_phase", ph.tunnel);
            f("u_tiles", fx.tiles.max(1.0));
            f("u_liquid", fx.liquid);
            f("u_liquid_scale", fx.liquid_scale);
            f("u_liquid_phase", ph.liquid);
            f("u_acid", fx.acid.clamp(0.0, 1.0));
            f("u_acid_phase", ph.acid);
            f("u_rainbow", fx.rainbow);
            f("u_solarize", fx.solarize.clamp(0.0, 1.0));
            f("u_pulse", fx.pulse);
            f("u_pulse_phase", ph.pulse);
            f("u_echo", fx.echo.clamp(0.0, 1.0));

            let g = &layer.generator;
            // Visuel généré : texture rendue à part (voir `render_generators`), unité 4.
            let gen_tex = self.gen_targets.get(&k).filter(|_| gen_on).map(|t| t.texture);
            i("u_gen_on", gen_tex.is_some() as i32);
            gl.active_texture(glow::TEXTURE4);
            gl.bind_texture(glow::TEXTURE_2D, Some(gen_tex.unwrap_or(self.black)));
            gl.active_texture(glow::TEXTURE0);
            i("u_gen_tex", 4);
            // Sans vidéo, rien à mélanger : le générateur est montré entier.
            f("u_gen_mix", if a.is_none() && b.is_none() { 1.0 } else { g.mix.clamp(0.0, 1.0) });
            gl.uniform_3_f32(loc("u_shape_color").as_ref(), fx.shape_color[0], fx.shape_color[1], fx.shape_color[2]);
            f("u_shape_glow", fx.shape_glow.max(0.0));
            f("u_shape_width", fx.shape_width.max(0.0005));
            f("u_shape_ripple", fx.shape_ripple.max(0.0));
            f("u_shape_freq", fx.shape_ripple_freq.max(0.1));
            f("u_shape_ripple_phase", ph.shape_ripple);
            f("u_shape_wave", fx.shape_wave.max(0.0));
            f("u_shape_trace", fx.shape_trace.max(0.0));
            f("u_shape_trace_count", fx.shape_trace_count.round().max(1.0));
            f("u_shape_trace_phase", ph.shape_trace);
            f("u_shape_trace_dir", if fx.shape_trace_speed < 0.0 { -1.0 } else { 1.0 });
            f("u_shape_grow", fx.shape_breathe * ph.shape_breathe.sin());
            // Pivot : le centre de la forme (le même point que pour la destruction).
            f("u_mask_angle", (fx.mask_rotate + ph.mask_spin).to_radians());
            gl.uniform_2_f32(loc("u_mask_center").as_ref(), layer.shatter_center[0], layer.shatter_center[1]);
            f("u_shape_rainbow", fx.shape_rainbow.clamp(0.0, 1.0));
            f("u_shape_echo", fx.shape_echo.max(0.0));
            f("u_shape_inner", fx.shape_inner.max(0.0));
            f("u_shape_dashes", fx.shape_dashes.max(0.0));
            f("u_shape_dash_count", fx.shape_dash_count.round().max(1.0));
            i("u_break_style", fx.shatter as i32);
            f("u_break", layer.shatter.clamp(0.0, 1.0));
            f("u_break_size", fx.shatter_size.max(1.0));
            gl.uniform_2_f32(loc("u_break_center").as_ref(), layer.shatter_center[0], layer.shatter_center[1]);
            f("u_break_glow", fx.shatter_glow.max(0.0));
            f("u_lines_glow", fx.lines_glow.max(0.0));
            gl.uniform_3_f32(loc("u_lines_color").as_ref(), fx.lines_color[0], fx.lines_color[1], fx.lines_color[2]);
            f("u_lines_rainbow", fx.lines_rainbow.clamp(0.0, 1.0));
            f("u_lines_threshold", fx.lines_threshold.clamp(0.0, 1.0));
            f("u_lines_width", fx.lines_width.max(1.0));
            f("u_lines_flow", fx.lines_flow.max(0.0));
            f("u_lines_flow_phase", ph.lines_flow);
            f("u_lines_wobble", fx.lines_wobble.max(0.0));
            f("u_lines_wobble_phase", ph.lines_wobble);
            f("u_lines_dim", fx.lines_dim.clamp(0.0, 1.0));
            i("u_obj3d", fx.obj3d as i32);
            let (rx, ry) = (
                (fx.rot3d_x + layer.phases.rot3d_x).to_radians(),
                (fx.rot3d_y + layer.phases.rot3d_y).to_radians(),
            );
            gl.uniform_2_f32(loc("u_rot3d").as_ref(), rx, ry);
            f("u_dist3d", fx.dist3d);
            f("u_height3d", fx.height3d.max(0.0));
            f("u_voxels", fx.voxels.max(4.0));
            f("u_relief", fx.relief.max(0.0));
            // Le relief bouge avec le point de vue : angles 3D, sinon un léger biais fixe.
            gl.uniform_2_f32(loc("u_relief_dir").as_ref(), 0.4 + ry.sin(), 0.3 + rx.sin());
            f("u_anaglyph", fx.anaglyph.max(0.0));
            f("u_flag", fx.flag.max(0.0));
            f("u_flag_phase", layer.phases.flag);
            f("u_thermal", fx.thermal.max(0.0));
            f("u_blur", fx.blur.max(0.0));
            f("u_bloom", fx.bloom.max(0.0));
            f("u_shock", fx.shock.max(0.0));
            f("u_shock_phase", layer.phases.shock);
            f("u_fisheye", fx.fisheye);
            f("u_hexagon", fx.hexagon.max(0.0));
            f("u_crt", fx.crt.max(0.0));
            f("u_glitch", fx.glitch.max(0.0));
            f("u_grain", fx.grain.max(0.0));
            f("u_halftone", fx.halftone.max(0.0));
            f("u_droste", fx.droste.max(0.0));
            f("u_droste_phase", layer.phases.droste);
            f("u_zoom_blur", fx.zoom_blur.max(0.0));
            f("u_ascii", fx.ascii.max(0.0));
            f("u_fold", fx.fold.clamp(0.0, 1.0));
            f("u_fold_count", fx.fold_count);

            let (src, dst, eq) = match layer.blend {
                BlendMode::Normal => (glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::FUNC_ADD),
                BlendMode::Add => (glow::ONE, glow::ONE, glow::FUNC_ADD),
                BlendMode::Screen => (glow::ONE, glow::ONE_MINUS_SRC_COLOR, glow::FUNC_ADD),
                BlendMode::Multiply => (glow::DST_COLOR, glow::ZERO, glow::FUNC_ADD),
                BlendMode::Lighten => (glow::ONE, glow::ONE, glow::MAX),
                BlendMode::Darken => (glow::ONE, glow::ONE, glow::MIN),
            };
            gl.blend_equation(eq);
            gl.blend_func_separate(src, dst, glow::ZERO, glow::ONE);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&layer.mesh), glow::STREAM_DRAW);
            gl.draw_arrays(glow::TRIANGLES, 0, (layer.mesh.len() / 4) as i32);
        }
    }
}

/// Ajoute au painter egui un rectangle dessiné par le renderer.
pub fn paint_callback(rect: egui::Rect, renderer: Arc<Mutex<Renderer>>, params: DrawParams) -> egui::PaintCallback {
    egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
            renderer.lock().unwrap().paint(painter.gl(), &info, &params);
        })),
    }
}

/// Callback qui, si une image est due, rend la sortie hors écran et l'ajoute à l'enregistrement.
pub fn capture_callback(
    rect: egui::Rect,
    renderer: Arc<Mutex<Renderer>>,
    params: DrawParams,
    recorder: Arc<Mutex<crate::recorder::Recorder>>,
) -> egui::PaintCallback {
    egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |_info, painter| {
            let mut rec = recorder.lock().unwrap();
            if rec.frames >= rec.frames_due() {
                return;
            }
            let size = rec.size;
            if let Some(frame) = renderer.lock().unwrap().capture(painter.gl(), size, &params) {
                rec.push(frame);
            }
        })),
    }
}

fn as_bytes(v: &[f32]) -> &[u8] {
    // SAFETY : un f32 n'a pas de bits de remplissage et u8 n'a pas d'exigence d'alignement.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_animations_return_to_neutral() {
        let spinning = Effects { rotate_speed: 40.0, hue_speed: 0.3, ..Effects::default() };
        let mut p = Phases::default();
        for _ in 0..100 {
            p.advance(&spinning, 1.0 / 60.0);
        }
        assert!(p.rotate > 1.0 && p.hue > 0.01);
        for _ in 0..120 {
            p.advance(&Effects::default(), 1.0 / 60.0);
        }
        // (L'onde continue d'avancer : sa vitesse par défaut n'est pas nulle, mais son amplitude l'est.)
        assert_eq!((p.hue, p.rotate, p.kaleido), (0.0, 0.0, 0.0));
    }
}
