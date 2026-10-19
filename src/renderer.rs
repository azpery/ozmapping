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

pub struct Renderer {
    program: glow::Program,
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
uniform float u_time;
uniform vec2 u_tex_size;
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

// Générateurs (motif de départ A, d'arrivée B, fondu entre les deux).
uniform int u_gen_a;
uniform int u_gen_b;
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

// Masque : image pré-calculée (bords adoucis compris), lue en coordonnées de sortie.
float mask_value(vec2 pos) {
    if (u_has_mask == 0) return 1.0;
    float m = texture(u_mask_tex, pos).r;
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
vec2 pattern(int kind, vec2 p, float t) {
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

vec3 palette(vec3 pal[4], float x) {
    return clamp(pal[0] + pal[1] * cos(2.0 * PI * (pal[2] * x + pal[3])), 0.0, 1.0);
}

vec3 generate(int kind, vec3 pal[4], vec2 uv) {
    vec2 p = (uv - 0.5) * vec2(u_gen_aspect, 1.0) * u_gen_scale;
    vec2 v = pattern(kind, p, u_gen_time);
    return palette(pal, v.x + u_gen_hue) * v.y;
}

vec3 fetch(vec2 uv) {
    vec3 a = u_has_a == 1 ? texture(u_tex_a, uv).rgb : vec3(0.0);
    vec3 b = u_has_b == 1 ? texture(u_tex_b, uv).rgb : vec3(0.0);
    vec3 video = mix(a, b, u_mix);
    if (u_gen_a == 0 && u_gen_b == 0) return video;
    // Fondu entre deux motifs ; « aucun » motif laisse passer la vidéo.
    vec3 gb = u_gen_b > 0 ? generate(u_gen_b, u_pal_b, uv) : video;
    vec3 g = gb;
    if (u_gen_fade < 1.0) {
        vec3 ga = u_gen_a > 0 ? generate(u_gen_a, u_pal_a, uv) : video;
        g = mix(ga, gb, u_gen_fade);
    }
    return mix(video, g, u_gen_mix);
}

float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }

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
    float m = mask_value(pos);

    // Forme du masque : distance au contour, comptée positivement vers la zone visible.
    vec4 field = vec4(0.0);
    float dv = 0.0;
    vec3 light = vec3(0.0);
    vec3 scol = u_shape_color;
    if (u_has_shape == 1) {
        field = texture(u_shape_tex, pos);
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
            texture(u_shape_tex, pos + vec2(t.x, 0.0)).r - texture(u_shape_tex, pos - vec2(t.x, 0.0)).r,
            texture(u_shape_tex, pos + vec2(0.0, t.y)).r - texture(u_shape_tex, pos - vec2(0.0, t.y)).r
        );
        vec2 n = g / max(length(g), 1e-6);
        float w = sin(2.0 * PI * (dv * u_shape_freq - u_shape_ripple_phase));
        uv += n * vec2(t.x / t.y, 1.0) * w * u_shape_wave;
    }
    if (u_pixelate > 0.0) {
        float cells = mix(320.0, 6.0, u_pixelate);
        vec2 grid = vec2(cells, cells / aspect);
        uv = (floor(uv * grid) + 0.5) / grid;
    }

    // 4. Échantillonnage (avec décalage RVB)
    vec3 col;
    if (u_rgb_shift > 0.0) {
        vec2 off = vec2(u_rgb_shift, u_rgb_shift * 0.3);
        col = vec3(fetch(uv + off).r, fetch(uv).g, fetch(uv - off).b);
    } else {
        col = fetch(uv);
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
        vec2 t = 1.0 / max(u_tex_size, vec2(1.0));
        float tl = luma(fetch(uv + t * vec2(-1, -1))), tc = luma(fetch(uv + t * vec2(0, -1))), tr = luma(fetch(uv + t * vec2(1, -1)));
        float ml = luma(fetch(uv + t * vec2(-1, 0))),                                          mr = luma(fetch(uv + t * vec2(1, 0)));
        float bl = luma(fetch(uv + t * vec2(-1, 1))),  bc = luma(fetch(uv + t * vec2(0, 1))),  br = luma(fetch(uv + t * vec2(1, 1)));
        float gx = -tl - 2.0 * ml - bl + tr + 2.0 * mr + br;
        float gy = -tl - 2.0 * tc - tr + bl + 2.0 * bc + br;
        float e = clamp(length(vec2(gx, gy)) * 2.0, 0.0, 1.0);
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

    if (u_vignette > 0.0) {
        float d = length(quad_uv - 0.5) * 1.4142;
        col *= 1.0 - smoothstep(1.0 - u_vignette, 1.0 + 0.0001, d) * clamp(u_vignette * 1.5, 0.0, 1.0);
    }
    col *= tunnel_fade * shade;
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
"#;

impl Renderer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let program = gl.create_program()?;
            let header = "#version 330\n";
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
        let shapes: HashSet<u64> = p.layers.iter().filter_map(|l| l.shape.as_ref()).map(|s| s.key).collect();
        self.shape_textures.retain(|key, t| {
            let keep = shapes.contains(key);
            if !keep {
                unsafe { gl.delete_texture(*t) };
            }
            keep
        });
    }

    pub fn paint(&mut self, gl: &glow::Context, info: &egui::PaintCallbackInfo, p: &DrawParams) {
        self.collect_garbage(gl, p);

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
            gl.uniform_2_f32(loc("u_res").as_ref(), vp.width_px as f32, vp.height_px as f32);

            for layer in &p.layers {
                self.paint_layer(gl, layer, p.time);
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

    unsafe fn paint_layer(&mut self, gl: &glow::Context, layer: &LayerDraw, time: f32) {
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
            i("u_gen_a", g.from as i32);
            i("u_gen_b", g.to as i32);
            f("u_gen_fade", g.fade.clamp(0.0, 1.0));
            // Sans vidéo, rien à mélanger : le générateur est montré entier.
            f("u_gen_mix", if a.is_none() && b.is_none() { 1.0 } else { g.mix.clamp(0.0, 1.0) });
            f("u_gen_time", g.time);
            f("u_gen_hue", g.hue);
            f("u_gen_scale", g.scale.max(0.05));
            f("u_gen_detail", g.detail.clamp(0.0, 1.0));
            f("u_gen_aspect", g.aspect.max(0.1));
            for (name, pal) in [("u_pal_a", g.from_palette), ("u_pal_b", g.to_palette)] {
                let flat: Vec<f32> = pal.coefs().iter().flatten().copied().collect();
                gl.uniform_3_f32_slice(loc(&format!("{name}[0]")).as_ref(), &flat);
            }
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
