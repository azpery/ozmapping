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
    pub effects: Effects,
    pub opacity: f32,
    pub blend: BlendMode,
    pub grid: bool,
    pub phases: Phases,
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

/// Tout ce qu'il faut pour dessiner une image (copié dans le callback de peinture).
/// Les calques sont listés du bas vers le haut.
#[derive(Clone)]
pub struct DrawParams {
    pub layers: Vec<LayerDraw>,
    pub time: f32,
    pub master: f32,
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
float mask_value() {
    if (u_has_mask == 0) return 1.0;
    float m = texture(u_mask_tex, v_pos).r;
    return u_mask_invert == 1 ? 1.0 - m : m;
}

vec3 fetch(vec2 uv) {
    vec3 a = u_has_a == 1 ? texture(u_tex_a, uv).rgb : vec3(0.0);
    vec3 b = u_has_b == 1 ? texture(u_tex_b, uv).rgb : vec3(0.0);
    return mix(a, b, u_mix);
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
    float coverage = u_opacity * mask_value();
    if (coverage <= 0.0) {
        emit(vec3(0.0), 0.0);
        return;
    }
    vec2 quad_uv = v_uv;
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
    col *= tunnel_fade;
    if (u_strobe > 0.0 && fract(u_time * u_strobe) > 0.5) {
        col = vec3(0.0);
    }

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
            for (_, t) in self.mask_textures.drain() {
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
        if a.is_none() && b.is_none() && !layer.grid {
            return;
        }
        let fx = &layer.effects;
        let tex_size = a.or(b).map_or((1, 1), |(_, size)| size);

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
