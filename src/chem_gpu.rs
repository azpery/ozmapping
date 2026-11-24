//! Simulation du visuel « Chimie » sur le GPU (voir `chem.rs`).
//!
//! Un fluide incompressible (vitesse, pression) transporte quatre champs :
//! - couleur : pigments et encres (rgb prémultiplié, quantité en a) ;
//! - huile : sa couleur (rgb prémultiplié) et sa présence (a), qui ne se mélange pas : elle se
//!   rassemble en bulles aux bords nets (équation d'Allen-Cahn) ;
//! - matières : pellicule d'essence (r), savon et alcool (g), épaisseur de la peinture (b, la
//!   rend visqueuse et couvrante), brillant métallique (a).
//! Les gouttes déposent ces matières et poussent le liquide ; le savon et l'alcool le chassent
//! (effet Marangoni) ; l'iPad penché fait glisser les parties plus lourdes ; les secousses
//! ajoutent des remous ; le confinement de tourbillons garde les volutes de l'encre.
//!
//! Convention : coordonnées 0..1 avec y vers le bas, comme la page et la simulation de peinture.

use eframe::glow::{self, HasContext as _};

use crate::chem::{Base, ChemDraw, MAX_SPLATS};

/// Définition de la simulation (hauteur) ; l'image affichée est deux fois plus fine.
const SIM_H: u32 = 480;
const DISPLAY_H: u32 = 1080;
/// Itérations de pression (incompressibilité).
const JACOBI: usize = 28;

const CHEM_SHADER: &str = r#"
precision highp float;
uniform sampler2D u_vel;
uniform sampler2D u_dye;
uniform sampler2D u_oil;
uniform sampler2D u_mat;
uniform sampler2D u_p;
uniform sampler2D u_div;
uniform sampler2D u_curl;
uniform vec2 u_texel;     // 1 / taille de la simulation
uniform vec2 u_out_texel; // 1 / taille de la cible écrite
uniform float u_aspect;
uniform float u_dt;
uniform float u_time;
uniform vec2 u_gravity;
uniform float u_shake;
uniform float u_beat;
uniform float u_fade;     // renouvellement : effacement par seconde
uniform float u_blow;     // souffle dans le micro (0..1)
uniform float u_flash;    // éclair de la foudre (0..1)
uniform vec3 u_sound;     // son : niveau, basses, aigus (0..1, sensibilité comprise)
uniform int u_bolt_n;     // foudre : segments, centre, couleur, tranche qui s'allume
uniform vec2 u_bolt_c;
uniform vec3 u_bolt_col;
uniform vec2 u_bolt_win;
uniform vec3 u_bolt_show; // affichage net : avancée, intensité, rayon
uniform vec4 u_bolt_p[256];
uniform vec4 u_bolt_i[256];
uniform int u_clear;
uniform int u_base;
uniform int u_count;
uniform vec4 u_sp_a[16]; // centre (x, y), rayon, genre
uniform vec4 u_sp_b[16]; // couleur (rgb), graine
uniform vec4 u_sp_c[16]; // mouvement (x, y)

#define K_WATER 0
#define K_MILK 1
#define K_INK 2
#define K_COLORING 3
#define K_OIL 4
#define K_GASOLINE 5
#define K_SOAP 6
#define K_ALCOHOL 7
#define K_ACRYLIC 8
#define K_WATERCOLOR 9
#define K_GOUACHE 10
#define K_FLUO 11
#define K_METALLIC 12
#define K_MARBLING 13
#define K_STIR 14
#define K_LIGHTNING 15
#define B_DARK 0
#define B_LIT 1
#define B_MILK 2
#define B_OIL 3
#define B_BUBBLE 4

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float noise(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}

// Coordonnées du fragment (0..1, y vers le bas) dans la cible écrite.
vec2 here() { return gl_FragCoord.xy * u_out_texel; }
vec2 asp() { return vec2(u_aspect, 1.0); }

// Le bassin posé sur un haut-parleur : la surface vibre avec le son. Basses : ondes
// stationnaires (figures de Faraday, carrées ou en étoile) qui battent ; niveau : ondes
// rondes depuis le centre ; aigus : un frémissement fin. Hauteur de la surface, en hauteurs
// d'image.
float standing(vec2 p, float k, float turn) {
    float s = 0.0;
    for (int i = 0; i < 4; i++) {
        float a = float(i) * 0.785398 + turn;
        s += cos(dot(p, vec2(cos(a), sin(a))) * k);
    }
    return s * 0.25;
}
float vibe(vec2 tc) {
    if (u_sound.x + u_sound.y + u_sound.z < 0.003) return 0.0;
    vec2 p = (tc - 0.5) * asp();
    // Plus il y a de basses, plus la figure se resserre (comme quand la fréquence monte).
    float k = 38.0 + 26.0 * u_sound.y;
    float low = standing(p, k, 0.3 + 0.05 * sin(u_time * 0.21)) * sin(u_time * 37.0) * u_sound.y;
    float r = length(p);
    float ring = sin(r * 140.0 - u_time * 46.0) * exp(-r * 1.2) * u_sound.x;
    float fine = (noise(p * 110.0 + vec2(u_time * 23.0, -u_time * 17.0)) - 0.5) * u_sound.z;
    return 0.0018 * low + 0.0005 * ring + 0.0005 * fine;
}

// Poids d'une goutte au point tc : disque au bord irrégulier (graine), doux sur un pixel.
float splat_w(int i, vec2 tc) {
    vec4 a = u_sp_a[i];
    vec2 d = (tc - a.xy) * asp();
    float len = length(d);
    if (len > a.z * 1.6 + 0.02) return 0.0;
    float ang = atan(d.y, d.x);
    float seed = u_sp_b[i].w;
    float wob = 1.0 + 0.22 * (noise(vec2(cos(ang), sin(ang)) * 1.7 + seed) - 0.5) + 0.08 * (noise(vec2(cos(ang), sin(ang)) * 5.0 + seed * 2.0) - 0.5);
    float r = a.z * wob;
    return 1.0 - smoothstep(r * 0.75, r, len);
}

// Douce : pour pousser le liquide (une gaussienne un peu plus large que la goutte).
float splat_soft(int i, vec2 tc) {
    vec4 a = u_sp_a[i];
    vec2 d = (tc - a.xy) * asp();
    return exp(-dot(d, d) / max(a.z * a.z * 1.4, 1e-6));
}

#ifdef PASS_CURL
out vec4 o;
void main() {
    vec2 tc = here(), t = u_texel;
    float vr = texture(u_vel, tc + vec2(t.x, 0.0)).y, vl = texture(u_vel, tc - vec2(t.x, 0.0)).y;
    float vt = texture(u_vel, tc + vec2(0.0, t.y)).x, vb = texture(u_vel, tc - vec2(0.0, t.y)).x;
    float h = t.y; // cellules carrées, en hauteurs d'image
    o = vec4(((vr - vl) - (vt - vb)) / (2.0 * h), 0.0, 0.0, 1.0);
}
#endif

#ifdef PASS_VEL
out vec4 o;
void main() {
    vec2 tc = here(), t = u_texel;
    if (u_clear == 1) { o = vec4(0.0); return; }
    vec2 v0 = texture(u_vel, tc).xy;
    // Transport (semi-lagrangien) : on remonte le courant.
    vec2 v = texture(u_vel, tc - v0 * u_dt / asp()).xy;
    vec4 dye = texture(u_dye, tc);
    vec4 oil = texture(u_oil, tc);
    vec4 mat = texture(u_mat, tc);
    vec2 f = vec2(0.0);
    // iPad penché : les parties plus lourdes (pigments, peinture épaisse) glissent, l'huile
    // plus légère remonte ; une force uniforme serait absorbée par la pression.
    float rho = 0.9 * min(dye.a, 2.0) + 1.6 * mat.b - 0.9 * oil.a + 0.25 * mat.r;
    f += u_gravity * rho * 1.1;
    // Secousses et musique : remous (rotationnel d'un bruit qui change vite).
    // Bulle de savon : lente convection permanente ; le souffle l'agite fort.
    float agit = u_shake * 2.2 + u_beat * 0.25 + (u_base == B_BUBBLE ? 0.35 : 0.0) + u_blow * 3.0
               + u_sound.x * u_sound.x * 0.6;
    if (agit > 0.001) {
        vec2 p = tc * asp() * 3.0 + vec2(u_time * 1.7, -u_time * 1.3);
        float e = 0.05;
        vec2 c = vec2(noise(p + vec2(0.0, e)) - noise(p - vec2(0.0, e)), noise(p - vec2(e, 0.0)) - noise(p + vec2(e, 0.0))) / (2.0 * e);
        f += c * agit;
    }
    // Son : la vibration fait tourner le liquide en petites cellules calées sur la figure des
    // ondes (un courant qui suit les lignes de niveau ; une simple poussée serait absorbée par
    // la pression).
    if (u_sound.y > 0.01) {
        vec2 p = (tc - 0.5) * asp();
        float k = 38.0 + 26.0 * u_sound.y, e = 0.6 / k, turn = 0.3 + 0.05 * sin(u_time * 0.21);
        float sx = standing(p + vec2(e, 0.0), k, turn) - standing(p - vec2(e, 0.0), k, turn);
        float sy = standing(p + vec2(0.0, e), k, turn) - standing(p - vec2(0.0, e), k, turn);
        f += vec2(sy, -sx) / (2.0 * e) * u_sound.y * u_sound.y * 0.0045;
    }
    // Souffle : un courant qui monte depuis le bas, et s'écarte.
    if (u_blow > 0.001) {
        vec2 d = (tc - vec2(0.5, 1.1)) * asp();
        f += normalize(d) * u_blow * 1.2 * exp(-dot(d, d) * 0.8);
    }
    // Savon et alcool : la surface se tend ailleurs, le liquide fuit (Marangoni).
    float sl = texture(u_mat, tc - vec2(t.x, 0.0)).g, sr = texture(u_mat, tc + vec2(t.x, 0.0)).g;
    float sb = texture(u_mat, tc - vec2(0.0, t.y)).g, st = texture(u_mat, tc + vec2(0.0, t.y)).g;
    f -= vec2(sr - sl, st - sb) / (2.0 * t.y) * 0.035;
    // Confinement des tourbillons : les volutes de l'encre ne s'éteignent pas.
    float cl = abs(texture(u_curl, tc - vec2(t.x, 0.0)).r), cr = abs(texture(u_curl, tc + vec2(t.x, 0.0)).r);
    float cb = abs(texture(u_curl, tc - vec2(0.0, t.y)).r), ct = abs(texture(u_curl, tc + vec2(0.0, t.y)).r);
    vec2 n = vec2(cr - cl, ct - cb);
    float c0 = texture(u_curl, tc).r;
    n /= max(length(n), 1e-5);
    f += vec2(n.y, -n.x) * c0 * (0.0009 + 0.0012 * min(dye.a, 1.0));
    v += f * u_dt;
    // Gouttes : chacune pousse le liquide autour d'elle (le savon très fort) ; la cuillère touille.
    for (int i = 0; i < 16; i++) {
        if (i >= u_count) break;
        float w = splat_soft(i, tc);
        if (w < 0.002) continue;
        int k = int(u_sp_a[i].w + 0.5);
        vec2 d = (tc - u_sp_a[i].xy) * asp();
        vec2 out_dir = d / max(length(d), 1e-4);
        float push = k == K_SOAP ? 1.6 : k == K_ALCOHOL ? 1.0 : k == K_WATER ? 0.6 : k == K_GASOLINE ? 0.45
            : k == K_INK ? 0.35 : k == K_STIR ? 0.0 : (k >= K_ACRYLIC ? 0.12 : 0.25);
        // Encre dans l'huile : elle éclate en doigts (plus fort dans certaines directions).
        if (u_base == B_OIL && (k == K_INK || k == K_COLORING || k == K_WATERCOLOR || k == K_ALCOHOL)) {
            vec2 dir = out_dir;
            float fingers = noise(dir * 3.5 + u_sp_b[i].w) * 0.7 + noise(dir * 9.0 + u_sp_b[i].w * 2.0) * 0.5;
            push = 2.4 * (0.15 + fingers * fingers * 1.6);
        }
        v += out_dir * w * push * u_sp_a[i].z * 6.0;
        v += u_sp_c[i].xy * w * (k == K_STIR ? 1.0 : 0.4);
    }
    // Frottement : la peinture épaisse freine le liquide.
    v *= exp(-u_dt * (0.12 + 5.0 * mat.b + (u_base == B_OIL ? 0.9 : 0.0)));
    // Bords du bassin.
    if (tc.x < t.x || tc.x > 1.0 - t.x || tc.y < t.y || tc.y > 1.0 - t.y) v = vec2(0.0);
    o = vec4(clamp(v, vec2(-6.0), vec2(6.0)), 0.0, 1.0);
}
#endif

#ifdef PASS_DIV
out vec4 o;
void main() {
    vec2 tc = here(), t = u_texel;
    float vl = texture(u_vel, tc - vec2(t.x, 0.0)).x, vr = texture(u_vel, tc + vec2(t.x, 0.0)).x;
    float vb = texture(u_vel, tc - vec2(0.0, t.y)).y, vt = texture(u_vel, tc + vec2(0.0, t.y)).y;
    // Parois : le liquide ne les traverse pas.
    if (tc.x - t.x < 0.0) vl = -texture(u_vel, tc).x;
    if (tc.x + t.x > 1.0) vr = -texture(u_vel, tc).x;
    if (tc.y - t.y < 0.0) vb = -texture(u_vel, tc).y;
    if (tc.y + t.y > 1.0) vt = -texture(u_vel, tc).y;
    o = vec4(((vr - vl) + (vt - vb)) / (2.0 * t.y), 0.0, 0.0, 1.0);
}
#endif

#ifdef PASS_JACOBI
out vec4 o;
void main() {
    vec2 tc = here(), t = u_texel;
    float pl = texture(u_p, tc - vec2(t.x, 0.0)).r, pr = texture(u_p, tc + vec2(t.x, 0.0)).r;
    float pb = texture(u_p, tc - vec2(0.0, t.y)).r, pt = texture(u_p, tc + vec2(0.0, t.y)).r;
    float d = texture(u_div, tc).r;
    o = vec4((pl + pr + pb + pt - d * t.y * t.y) * 0.25, 0.0, 0.0, 1.0);
}
#endif

#ifdef PASS_GRAD
out vec4 o;
void main() {
    vec2 tc = here(), t = u_texel;
    float pl = texture(u_p, tc - vec2(t.x, 0.0)).r, pr = texture(u_p, tc + vec2(t.x, 0.0)).r;
    float pb = texture(u_p, tc - vec2(0.0, t.y)).r, pt = texture(u_p, tc + vec2(0.0, t.y)).r;
    // La surface d'un liquide peut s'étirer : là où il y a du savon (ou de l'encre qui s'étale
    // sur l'huile), on ne retire qu'une partie de l'étalement, et la tache s'élargit vraiment.
    float stretch = clamp(texture(u_mat, tc).g * 1.5, 0.0, 0.7);
    vec2 v = texture(u_vel, tc).xy - (1.0 - stretch) * vec2(pr - pl, pt - pb) / (2.0 * t.y);
    o = vec4(v, 0.0, 1.0);
}
#endif

#ifdef PASS_FIELDS
layout(location = 0) out vec4 o_dye;
layout(location = 1) out vec4 o_oil;
layout(location = 2) out vec4 o_mat;
vec4 lap(sampler2D s, vec2 tc, vec4 c) {
    vec2 t = u_texel;
    return texture(s, tc + vec2(t.x, 0.0)) + texture(s, tc - vec2(t.x, 0.0)) + texture(s, tc + vec2(0.0, t.y)) + texture(s, tc - vec2(0.0, t.y)) - 4.0 * c;
}
// Transport précis (MacCormack) : on remonte le courant, on redescend, et on corrige de la
// moitié de l'écart ; borné par les 4 texels d'arrivée (pas d'oscillation). Les filaments
// d'encre restent fins au lieu de se brouiller.
vec4 advect(sampler2D s, vec2 tc, vec2 p1, vec2 p2) {
    vec4 f1 = texture(s, p1);
    vec4 r = f1 + 0.5 * (texture(s, tc) - texture(s, p2));
    vec2 size = vec2(textureSize(s, 0));
    ivec2 i0 = ivec2(floor(p1 * size - 0.5));
    ivec2 hi = ivec2(size) - 1;
    vec4 a = texelFetch(s, clamp(i0, ivec2(0), hi), 0);
    vec4 b = texelFetch(s, clamp(i0 + ivec2(1, 0), ivec2(0), hi), 0);
    vec4 c = texelFetch(s, clamp(i0 + ivec2(0, 1), ivec2(0), hi), 0);
    vec4 d = texelFetch(s, clamp(i0 + ivec2(1, 1), ivec2(0), hi), 0);
    return clamp(r, min(min(a, b), min(c, d)), max(max(a, b), max(c, d)));
}
float fbm(vec2 p) { return 0.5 * noise(p) + 0.25 * noise(p * 2.1 + 3.7) + 0.125 * noise(p * 4.3 + 7.1) + 0.125 * noise(p * 8.7 + 1.3); }
void main() {
    vec2 tc = here();
    if (u_clear == 1) { o_dye = vec4(0.0); o_oil = vec4(0.0); o_mat = vec4(0.0); return; }
    vec2 v = texture(u_vel, tc).xy;
    vec2 src = tc - v * u_dt / asp();
    vec2 back = src + texture(u_vel, src).xy * u_dt / asp();
    vec4 dye = advect(u_dye, tc, src, back);
    vec4 oil = advect(u_oil, tc, src, back);
    vec4 mat = advect(u_mat, tc, src, back);
    float k60 = clamp(u_dt * 60.0, 0.0, 2.0);
    // Diffusion : les encres et l'aquarelle s'étalent, la peinture épaisse beaucoup moins.
    float body = mat.b;
    // Dans l'huile, l'encre ne diffuse presque pas : bords nets.
    dye += lap(u_dye, src, dye) * clamp(0.02 * k60 * (1.0 - 0.9 * body) * (u_base == B_OIL ? 0.12 : 1.0), 0.0, 0.24);
    // Huile : se rassemble (Allen-Cahn) en bulles aux bords nets, et se regroupe lentement.
    float phi = oil.a;
    vec4 lo = lap(u_oil, src, oil);
    phi += (lo.a * 0.22 + phi * (1.0 - phi) * (phi - 0.5) * 1.1) * k60;
    // Renouvellement : l'huile fond (les bulles rétrécissent puis disparaissent).
    phi = clamp(phi - u_dt * u_fade * 0.35, 0.0, 1.0);
    vec3 oc = oil.rgb + lo.rgb * 0.05 * k60;
    oil = vec4(phi > 0.002 ? oc * (phi / max(oil.a, 1e-3)) : vec3(0.0), phi);
    // Pellicule d'essence : s'étale vite, s'amincit, fuit le savon. Savon : se dilue, s'use.
    vec4 lm = lap(u_mat, src, mat);
    mat.r += lm.r * 0.05 * k60;
    if (u_base == B_BUBBLE) {
        // Bulle de savon : le film s'écoule (mince en haut, épais en bas), le courant le brasse.
        float thick = 0.12 + 0.95 * tc.y;
        mat.r += (thick - mat.r) * min(u_dt * 0.3, 1.0);
        mat.r -= u_blow * u_dt * 0.25;
        mat.r = max(mat.r, 0.0);
    } else {
        mat.r *= exp(-u_dt * (0.1 + 2.5 * mat.g));
    }
    mat.g += lm.g * 0.15 * k60;
    mat.g *= exp(-u_dt * 0.35);
    mat.b += lm.b * 0.01 * k60;
    mat.b *= exp(-u_dt * 0.01);
    mat.a += lm.a * 0.01 * k60;
    // Très lent effacement : le bassin ne sature jamais. Renouvellement choisi sur la page :
    // tout s'estompe ; ce qui vient d'être versé, plus dense, reste vif plus longtemps.
    dye *= exp(-u_dt * (0.004 + u_fade));
    mat.ga *= exp(-u_dt * u_fade);
    if (u_base != B_BUBBLE) mat.r *= exp(-u_dt * u_fade);
    mat.b *= exp(-u_dt * u_fade * 0.5);

    for (int i = 0; i < 16; i++) {
        if (i >= u_count) break;
        float w = splat_w(i, tc);
        if (w <= 0.0) continue;
        int k = int(u_sp_a[i].w + 0.5);
        vec3 c = u_sp_b[i].rgb;
        if (k == K_STIR) continue;
        // Encre sur l'huile : elle porte de quoi s'étaler (la surface s'étire, voir PASS_GRAD).
        if (u_base == B_OIL && (k == K_INK || k == K_COLORING || k == K_WATERCOLOR)) mat.g += 0.9 * w;
        if (k == K_WATER) { dye *= 1.0 - 0.55 * w; mat.b *= 1.0 - 0.5 * w; mat.g += 0.15 * w; continue; }
        if (k == K_SOAP) { mat.g += 1.2 * w; continue; }
        if (k == K_ALCOHOL) { dye *= 1.0 - 0.7 * w; mat.r *= 1.0 - 0.8 * w; mat.g += 0.8 * w; continue; }
        if (k == K_GASOLINE) { mat.r += 0.6 * w; continue; }
        if (k == K_OIL) {
            float a = smoothstep(0.25, 0.6, w);
            oil.rgb = mix(oil.rgb, c * max(oil.a, a), a);
            oil.a = max(oil.a, a);
            oil.rgb = oil.a > 0.0 ? oil.rgb : vec3(0.0);
            continue;
        }
        // Pigments : quantité, épaisseur (visqueux, couvrant), brillant.
        float amount = k == K_INK ? 1.4 : k == K_COLORING ? 0.8 : k == K_MILK ? 1.6 : k == K_WATERCOLOR ? 0.7
            : k == K_GOUACHE ? 2.2 : k == K_ACRYLIC ? 2.0 : k == K_FLUO ? 0.9 : 1.5;
        float thick = k == K_ACRYLIC ? 0.55 : k == K_GOUACHE ? 0.75 : k == K_FLUO ? 0.35 : k == K_METALLIC ? 0.45
            : k == K_MARBLING ? 0.3 : k == K_MILK ? 0.1 : 0.0;
        float shine = k == K_METALLIC ? 1.0 : 0.0;
        // Fluo : couleur au-delà du blanc (elle éclaire).
        vec3 pc = k == K_FLUO ? c * 2.4 : c;
        // Encres et colorants : la goutte tombe en nuage irrégulier, pas en disque plein.
        float cloud = (k == K_INK || k == K_COLORING || k == K_WATERCOLOR || k == K_MILK)
            ? 0.35 + 1.3 * fbm((tc * asp() + u_sp_b[i].w) * 38.0) : 1.0;
        float add = amount * w * cloud;
        // Peinture couvrante : remplace la couleur dessous ; encres : s'y ajoutent.
        float cover = clamp(thick * 1.3 * w, 0.0, 1.0);
        dye = vec4(mix(dye.rgb, pc * dye.a, cover) + pc * add, dye.a + add);
        mat.b = mix(mat.b, thick, w);
        mat.a = mix(mat.a, shine, w);
    }
    // Foudre : les branches qui arrivent sur cette image s'allument (lumière au-delà du blanc).
    if (u_bolt_n > 0) {
        vec2 q = (tc - u_bolt_c) * asp();
        float lit = 0.0;
        for (int i = 0; i < 256; i++) {
            if (i >= u_bolt_n) break;
            vec4 inf = u_bolt_i[i];
            if (inf.x <= u_bolt_win.x || inf.x > u_bolt_win.y) continue;
            vec4 sp = u_bolt_p[i];
            vec2 a = sp.xy, ab = sp.zw - sp.xy;
            float h = clamp(dot(q - a, ab) / max(dot(ab, ab), 1e-8), 0.0, 1.0);
            float dd = length(q - a - ab * h);
            // Trait net et fin (au moins un pixel), un léger halo autour.
            float w = max(inf.y, u_texel.y * 0.9);
            lit = max(lit, exp(-dd * dd / (w * w)) + 0.12 * exp(-dd * dd / (w * w * 16.0)));
        }
        // Trace pâle laissée dans le liquide (la foudre elle-même est dessinée nette à l'affichage).
        if (lit > 0.002) {
            vec3 c = u_bolt_col * 0.9 + 0.1;
            dye = vec4(dye.rgb + c * lit * 0.35, dye.a + lit * 0.35);
        }
    }
    o_dye = max(dye, vec4(0.0));
    o_oil = max(oil, vec4(0.0));
    o_mat = clamp(mat, vec4(0.0), vec4(4.0));
}
#endif

#ifdef PASS_DISPLAY
out vec4 o;
float phi_at(vec2 tc) { return texture(u_oil, tc).a; }
// Vaguelettes de la surface (plus fortes quand on secoue) : hauteur en hauteurs d'image.
float ripple(vec2 tc) {
    vec2 p = tc * asp();
    float amp = 0.00035 + 0.0035 * u_shake + 0.0006 * u_beat;
    return amp * (noise(p * 22.0 + vec2(u_time * 0.6, u_time * 0.45)) + 0.6 * noise(p * 51.0 - vec2(u_time * 1.1, -u_time * 0.8)));
}
float surface(vec2 tc) { return phi_at(tc) * 0.010 + ripple(tc) + vibe(tc); }
void main() {
    vec2 tc = here(), t = u_texel;
    // Bulle de savon : le film est noir là où il est très mince, les encres y brillent.
    bool dark = u_base == B_DARK || u_base == B_BUBBLE;
    // Surface : relief des bulles d'huile et vaguelettes → normale, réfraction, reflets.
    vec2 dx = vec2(t.x, 0.0), dy = vec2(0.0, t.y);
    vec2 gh = vec2(surface(tc + dx) - surface(tc - dx), surface(tc + dy) - surface(tc - dy)) / (2.0 * t.y);
    vec3 n = normalize(vec3(-gh, 1.0));
    vec2 off = -gh * 0.035;
    // Franges colorées au bord des bulles : chaque couleur est un peu plus ou moins déviée.
    vec4 dye = texture(u_dye, tc + off);
    float ar = texture(u_dye, tc + off * 1.08).a, ab = texture(u_dye, tc + off * 0.92).a;
    vec4 mat = texture(u_mat, tc + off);
    float a = dye.a;
    vec3 c = clamp(dye.rgb / max(a, 1e-3), 0.0, 3.0);
    vec3 amt = vec3(ar, a, ab);
    float body = mat.b;
    vec3 col;
    if (dark) {
        // Dans le noir, éclairée de côté : l'encre diffuse la lumière, plus dense = plus vive,
        // avec un halo doux autour des nuages (la lumière se diffuse dans l'eau).
        vec3 dens = 1.0 - exp(-amt * (1.1 + 4.0 * body));
        col = c * dens;
        vec3 glow = vec3(0.0);
        for (int i = 0; i < 8; i++) {
            float ang = float(i) * 0.785398 + 0.4;
            vec4 s = texture(u_dye, tc + off + vec2(cos(ang), sin(ang)) * t * 7.0);
            glow += s.rgb / max(s.a, 1e-3) * (1.0 - exp(-s.a * 1.5));
        }
        col += glow / 8.0 * 0.22;
    } else {
        // Lumière qui traverse (eau éclairée) ou se réfléchit dans le lait : chaque pigment
        // absorbe le complément de sa couleur (Beer-Lambert) ; les colorants se mélangent en
        // soustractif. Les peintures épaisses couvrent : leur propre couleur, diffusée.
        vec3 light = u_base == B_LIT ? vec3(1.0, 0.97, 0.9) : u_base == B_OIL ? vec3(0.98, 0.78, 0.36) : vec3(0.97, 0.955, 0.92);
        vec2 q = (tc - 0.5) * asp();
        if (u_base == B_LIT || u_base == B_OIL) light *= 1.06 - 0.35 * dot(q, q); // point chaud du projecteur
        else light *= 0.97 + 0.03 * noise(tc * asp() * 140.0); // grain du lait
        // Huile de lin : un peu trouble, avec des veines plus sombres.
        if (u_base == B_OIL) light *= 0.9 + 0.1 * noise(tc * asp() * 6.0 + u_time * 0.05);
        vec3 absorb = (1.0 - clamp(c, 0.0, 1.0)) * 2.4 + 0.06;
        col = light * exp(-absorb * amt);
        float opaque = 1.0 - exp(-a * body * 4.5);
        col = mix(col, light * clamp(c, 0.0, 1.0) * 0.92, opaque);
        // Encre sur l'huile : un liseré plus sombre au bord de chaque tache.
        if (u_base == B_OIL) {
            float ga = length(vec2(texture(u_dye, tc + off + dx).a - texture(u_dye, tc + off - dx).a,
                                   texture(u_dye, tc + off + dy).a - texture(u_dye, tc + off - dy).a));
            col *= 1.0 - smoothstep(0.05, 0.5, ga) * 0.45;
        }
    }
    // Huile : teinte, bord (sombre en transparence, brillant dans le noir), lumière concentrée
    // au cœur de la bulle (effet loupe).
    vec4 oil = texture(u_oil, tc);
    float phi = oil.a;
    vec3 oc = clamp(oil.rgb / max(phi, 1e-3), 0.0, 1.0);
    vec2 gp = vec2(phi_at(tc + dx) - phi_at(tc - dx), phi_at(tc + dy) - phi_at(tc - dy));
    float edge = smoothstep(0.2, 1.0, length(gp) * 5.0);
    float lapp = phi_at(tc + dx) + phi_at(tc - dx) + phi_at(tc + dy) + phi_at(tc - dy) - 4.0 * phi;
    float focus = clamp(-lapp * 6.0, 0.0, 1.0) * phi;
    if (dark) {
        col = col * (1.0 - 0.3 * phi) + oc * phi * 0.3 + oc * focus * 0.35;
        col += mix(vec3(1.0), oc, 0.4) * edge * 0.3;
    } else {
        col *= exp(-(1.0 - oc) * phi * 1.4);
        col *= 1.0 + focus * 0.5;
        col *= 1.0 - edge * 0.55;
    }
    // Bulle de savon : couleurs d'interférence d'un film d'eau savonneuse (indice 1,33) selon
    // son épaisseur (jusqu'à ~1 µm) ; noir là où il est très mince, comme une vraie bulle.
    float h = texture(u_mat, tc).r;
    if (u_base == B_BUBBLE) {
        float d_nm = h * 1100.0 + dot(n.xy, vec2(400.0, 250.0));
        vec3 lambda = vec3(650.0, 532.0, 450.0);
        vec3 s = sin(6.2831853 * 1.33 * d_nm / lambda);
        vec3 film = s * s * 1.05 * smoothstep(0.0, 0.06, h);
        col = film + col * 0.9;
        h = 0.0;
    }
    // Pellicule d'essence : couleurs d'interférence, qui changent avec la pente de la surface.
    if (h > 0.004) {
        float th = h * 2.6 + dot(n.xy, vec2(3.0, 2.0)) + noise(tc * asp() * 9.0) * 0.5;
        vec3 irid = 0.5 + 0.5 * cos(6.2831853 * (th + vec3(0.0, 0.33, 0.67)));
        float k = smoothstep(0.01, 0.15, h) * (1.0 - smoothstep(1.4, 2.5, h) * 0.6);
        col = dark ? col + irid * k * 0.32 : mix(col, col * (0.55 + irid * 0.6), k * 0.55);
    }
    // Peinture métallisée : paillettes.
    if (mat.a > 0.02) {
        float sp = pow(noise(tc * asp() * 420.0 + u_time * 0.6), 18.0) * 6.0;
        float cov = 1.0 - exp(-a * 2.0);
        col += (0.3 * mat.a + sp * mat.a) * mix(vec3(1.0), clamp(c, 0.0, 1.0), 0.4) * cov;
    }
    // Reflet de la lumière sur la surface (bulles d'huile surtout).
    vec3 L = normalize(vec3(-0.45, -0.55, 1.0));
    float spec = pow(max(dot(reflect(-L, n), vec3(0.0, 0.0, 1.0)), 0.0), 70.0);
    col += spec * (0.15 + 0.7 * phi) * (dark ? 0.6 : 0.45);
    // Foudre, à pleine définition : cœur blanc très fin (bord lissé sur un pixel), lueur de sa
    // couleur autour, plus forte sur les grosses branches et à l'avant de la pousse.
    if (u_bolt_n > 0 && u_bolt_show.y > 0.002) {
        vec2 q = (tc - u_bolt_c) * asp();
        if (length(q) < u_bolt_show.z + 0.05) {
            float pix = u_out_texel.y;
            float core = 0.0, glow = 0.0;
            for (int i = 0; i < 256; i++) {
                if (i >= u_bolt_n) break;
                vec4 inf = u_bolt_i[i];
                if (inf.x > u_bolt_show.x) continue;
                vec4 sp = u_bolt_p[i];
                vec2 a = sp.xy, ab = sp.zw - sp.xy;
                float hh = clamp(dot(q - a, ab) / max(dot(ab, ab), 1e-9), 0.0, 1.0);
                float dd = length(q - a - ab * hh);
                float w = inf.y * 0.45;
                float front = 1.0 + 1.5 * smoothstep(u_bolt_show.x - 0.12, u_bolt_show.x, inf.x) * step(u_bolt_show.x, 0.999);
                core = max(core, (1.0 - smoothstep(w, w + pix * 1.2, dd)) * front);
                glow = max(glow, exp(-dd / (inf.y * 2.5 + pix * 2.0)) * (0.35 + inf.y / 0.0035) * front);
            }
            float k = u_bolt_show.y;
            col += (vec3(1.0) * core * 1.1 + u_bolt_col * glow * 0.55) * k;
        }
    }
    // Éclair de la foudre : tout le bassin s'illumine un instant.
    col += u_bolt_col * u_flash * 0.22;
    // Grain très fin (pas d'aplats numériques).
    col += (hash(gl_FragCoord.xy + fract(u_time) * 91.0) - 0.5) * 0.01;
    // Teintes très vives (fluo) : on écrase en douceur au-delà du blanc.
    col = mix(col, 1.0 - exp(-col * 1.25), step(1.0, max(col.r, max(col.g, col.b))) * 0.85);
    o = vec4(clamp(col, 0.0, 1.0), 1.0);
}
#endif
"#;

struct Programs {
    curl: glow::Program,
    vel: glow::Program,
    div: glow::Program,
    jacobi: glow::Program,
    grad: glow::Program,
    fields: glow::Program,
    display: glow::Program,
}

/// Une cible : texture et framebuffer.
struct Tex {
    tex: glow::Texture,
    fbo: glow::Framebuffer,
}

/// Jeu des trois champs transportés (un framebuffer, trois sorties).
struct Fields {
    dye: glow::Texture,
    oil: glow::Texture,
    mat: glow::Texture,
    fbo: glow::Framebuffer,
}

/// Bassin d'un calque.
pub struct ChemSim {
    size: (u32, u32),
    vel: [Tex; 2],
    p: [Tex; 2],
    div: Tex,
    curl: Tex,
    fields: [Fields; 2],
    display: Tex,
    cur: usize,
    frame: u64,
    time: f32,
    /// Dernière utilisation : un bassin oublié depuis quelques secondes est libéré.
    used: std::time::Instant,
}

/// Programmes (partagés par tous les bassins) et bassins des calques.
pub struct ChemRenderer {
    programs: Programs,
    /// Bassins, par calque (voir `ChemDraw::id`).
    sims: std::collections::HashMap<u64, ChemSim>,
}

unsafe fn texture(gl: &glow::Context, size: (u32, u32), internal: u32, format: u32, ty: u32) -> Option<glow::Texture> {
    unsafe {
        let t = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(t));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(glow::TEXTURE_2D, 0, internal as i32, size.0 as i32, size.1 as i32, 0, format, ty, glow::PixelUnpackData::Slice(None));
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(t)
    }
}

unsafe fn target(gl: &glow::Context, size: (u32, u32), internal: u32, format: u32, ty: u32) -> Option<Tex> {
    unsafe {
        let tex = texture(gl, size, internal, format, ty)?;
        let fbo = gl.create_framebuffer().ok()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
        Some(Tex { tex, fbo })
    }
}

impl ChemSim {
    unsafe fn new(gl: &glow::Context, size: (u32, u32), display_size: (u32, u32)) -> Option<Self> {
        unsafe {
            let f16 = |n: u32| match n {
                1 => (glow::R16F, glow::RED),
                _ => (glow::RGBA16F, glow::RGBA),
            };
            let t = |n: u32| {
                let (i, f) = f16(n);
                target(gl, size, i, f, glow::HALF_FLOAT)
            };
            let fields = || -> Option<Fields> {
                let (i, f) = f16(4);
                let dye = texture(gl, size, i, f, glow::HALF_FLOAT)?;
                let oil = texture(gl, size, i, f, glow::HALF_FLOAT)?;
                let mat = texture(gl, size, i, f, glow::HALF_FLOAT)?;
                let fbo = gl.create_framebuffer().ok()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                for (k, t) in [dye, oil, mat].into_iter().enumerate() {
                    gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0 + k as u32, glow::TEXTURE_2D, Some(t), 0);
                }
                gl.draw_buffers(&[glow::COLOR_ATTACHMENT0, glow::COLOR_ATTACHMENT1, glow::COLOR_ATTACHMENT2]);
                if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                    return None;
                }
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                Some(Fields { dye, oil, mat, fbo })
            };
            Some(Self {
                size,
                vel: [t(4)?, t(4)?],
                p: [t(1)?, t(1)?],
                div: t(1)?,
                curl: t(1)?,
                fields: [fields()?, fields()?],
                display: target(gl, display_size, glow::RGBA8, glow::RGBA, glow::UNSIGNED_BYTE)?,
                cur: 0,
                frame: 0,
                time: 0.0,
                used: std::time::Instant::now(),
            })
        }
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            for t in self.vel.iter().chain(&self.p).chain([&self.div, &self.curl, &self.display]) {
                gl.delete_texture(t.tex);
                gl.delete_framebuffer(t.fbo);
            }
            for f in &self.fields {
                gl.delete_texture(f.dye);
                gl.delete_texture(f.oil);
                gl.delete_texture(f.mat);
                gl.delete_framebuffer(f.fbo);
            }
        }
    }
}

impl ChemRenderer {
    /// `compile(en-tête)` compile `CHEM_SHADER` (voir `shader`) précédé de cet en-tête.
    pub fn new(compile: impl Fn(&str) -> Result<glow::Program, String>) -> Result<Self, String> {
        let pass = |name: &str| compile(&format!("#version 330\n#define PASS_{name}\n"));
        Ok(Self {
            programs: Programs {
                curl: pass("CURL")?,
                vel: pass("VEL")?,
                div: pass("DIV")?,
                jacobi: pass("JACOBI")?,
                grad: pass("GRAD")?,
                fields: pass("FIELDS")?,
                display: pass("DISPLAY")?,
            },
            sims: std::collections::HashMap::new(),
        })
    }

    pub fn shader() -> &'static str {
        CHEM_SHADER
    }

    /// Libère les bassins qui n'ont pas servi depuis quelques secondes (calque supprimé, visuel
    /// changé). Pas d'après la liste des calques d'une sortie : chaque sortie a la sienne.
    pub unsafe fn forget_unused(&mut self, gl: &glow::Context) {
        self.sims.retain(|_, s| {
            let ok = s.used.elapsed() < std::time::Duration::from_secs(5);
            if !ok {
                unsafe { s.delete(gl) };
            }
            ok
        });
    }

    pub unsafe fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            for (_, s) in self.sims.drain() {
                s.delete(gl);
            }
            let p = &self.programs;
            for prog in [p.curl, p.vel, p.div, p.jacobi, p.grad, p.fields, p.display] {
                gl.delete_program(prog);
            }
        }
    }

    /// Avance le bassin `d.id` d'une image (une fois par image, quel que soit le nombre de
    /// fenêtres qui l'affichent) et renvoie l'image à afficher. Le quad plein écran doit être
    /// en place (vao et vbo liés).
    pub unsafe fn step(&mut self, gl: &glow::Context, d: &ChemDraw, aspect: f32) -> Option<glow::Texture> {
        let k = d.id;
        unsafe {
            let aspect = aspect.clamp(0.2, 5.0);
            let size = (((SIM_H as f32) * aspect) as u32, SIM_H);
            let display_size = (((DISPLAY_H as f32) * aspect) as u32, DISPLAY_H);
            if self.sims.get(&k).is_some_and(|s| s.size != size) {
                let s = self.sims.remove(&k).unwrap();
                s.delete(gl);
            }
            if !self.sims.contains_key(&k) {
                let s = ChemSim::new(gl, size, display_size)?;
                self.sims.insert(k, s);
            }
            let p = &self.programs;
            let sim = self.sims.get_mut(&k)?;
            sim.used = std::time::Instant::now();
            // Bassin enregistré à recharger, photo du bassin demandée (voir `chem::Basin`).
            let (want, restore) = crate::chem::gpu_requests(k);
            if let Some(b) = restore {
                let b = b.resized(size);
                let f = &sim.fields[sim.cur];
                for (t, data) in [f.dye, f.oil, f.mat].into_iter().zip(&b.fields) {
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    let bytes: &[u8] = std::slice::from_raw_parts(data.as_ptr().cast(), data.len() * 2);
                    gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, size.0 as i32, size.1 as i32, glow::RGBA, glow::HALF_FLOAT, glow::PixelUnpackData::Slice(Some(bytes)));
                }
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            if want {
                let f = &sim.fields[sim.cur];
                let prev = gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(f.fbo));
                let n = (size.0 * size.1 * 4) as usize;
                let mut fields: [Vec<u16>; 3] = [vec![0; n], vec![0; n], vec![0; n]];
                for (k2, out) in fields.iter_mut().enumerate() {
                    gl.read_buffer(glow::COLOR_ATTACHMENT0 + k2 as u32);
                    let bytes: &mut [u8] = std::slice::from_raw_parts_mut(out.as_mut_ptr().cast(), n * 2);
                    gl.read_pixels(0, 0, size.0 as i32, size.1 as i32, glow::RGBA, glow::HALF_FLOAT, glow::PixelPackData::Slice(Some(bytes)));
                }
                gl.read_buffer(glow::COLOR_ATTACHMENT0);
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, prev);
                crate::chem::gpu_deliver(k, crate::chem::Basin { size, fields });
            }
            if sim.frame == d.frame {
                return Some(sim.display.tex);
            }
            sim.frame = d.frame;
            let dt = d.dt.clamp(0.0, 0.05);
            sim.time += dt;
            let texel = (1.0 / size.0 as f32, 1.0 / size.1 as f32);

            // Uniformes communs d'un programme.
            let common = |prog: glow::Program, out: (u32, u32)| {
                gl.use_program(Some(prog));
                let loc = |n: &str| gl.get_uniform_location(prog, n);
                gl.uniform_2_f32(loc("u_texel").as_ref(), texel.0, texel.1);
                gl.uniform_2_f32(loc("u_out_texel").as_ref(), 1.0 / out.0 as f32, 1.0 / out.1 as f32);
                gl.uniform_1_f32(loc("u_aspect").as_ref(), aspect);
                gl.uniform_1_f32(loc("u_dt").as_ref(), dt);
                gl.uniform_1_f32(loc("u_time").as_ref(), sim.time);
                gl.uniform_2_f32(loc("u_gravity").as_ref(), d.gravity[0], d.gravity[1]);
                gl.uniform_1_f32(loc("u_shake").as_ref(), d.shake);
                gl.uniform_1_f32(loc("u_beat").as_ref(), d.beat);
                gl.uniform_1_f32(loc("u_fade").as_ref(), d.fade);
                gl.uniform_1_f32(loc("u_blow").as_ref(), d.blow);
                gl.uniform_1_f32(loc("u_flash").as_ref(), d.flash);
                gl.uniform_3_f32(loc("u_sound").as_ref(), d.sound[0], d.sound[1], d.sound[2]);
                gl.uniform_3_f32(loc("u_bolt_col").as_ref(), d.bolt_color[0], d.bolt_color[1], d.bolt_color[2]);
                gl.uniform_1_i32(loc("u_clear").as_ref(), d.clear as i32);
                gl.uniform_1_i32(
                    loc("u_base").as_ref(),
                    match d.base {
                        Base::Dark => 0,
                        Base::Lit => 1,
                        Base::Milk => 2,
                        Base::Oil => 3,
                        Base::Bubble => 4,
                    },
                );
                let n = (d.count as usize).min(MAX_SPLATS);
                gl.uniform_1_i32(loc("u_count").as_ref(), n as i32);
                if n > 0 {
                    let s = &d.splats[..n];
                    let a: Vec<f32> = s.iter().flat_map(|s| [s.pos[0], s.pos[1], s.radius, s.kind as f32]).collect();
                    let b: Vec<f32> = s.iter().flat_map(|s| [s.color[0], s.color[1], s.color[2], s.seed]).collect();
                    let c: Vec<f32> = s.iter().flat_map(|s| [s.vel[0], s.vel[1], 0.0, 0.0]).collect();
                    gl.uniform_4_f32_slice(loc("u_sp_a[0]").as_ref(), &a);
                    gl.uniform_4_f32_slice(loc("u_sp_b[0]").as_ref(), &b);
                    gl.uniform_4_f32_slice(loc("u_sp_c[0]").as_ref(), &c);
                }
            };
            let bind = |prog: glow::Program, unit: u32, name: &str, t: glow::Texture| {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.uniform_1_i32(gl.get_uniform_location(prog, name).as_ref(), unit as i32);
            };
            let draw = |fbo: glow::Framebuffer, out: (u32, u32)| {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.viewport(0, 0, out.0 as i32, out.1 as i32);
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
            };
            let (r, w) = (sim.cur, 1 - sim.cur);

            // 1. Tourbillons.
            common(p.curl, size);
            bind(p.curl, 0, "u_vel", sim.vel[r].tex);
            draw(sim.curl.fbo, size);
            // 2. Forces et transport de la vitesse.
            common(p.vel, size);
            bind(p.vel, 0, "u_vel", sim.vel[r].tex);
            bind(p.vel, 1, "u_dye", sim.fields[r].dye);
            bind(p.vel, 2, "u_oil", sim.fields[r].oil);
            bind(p.vel, 3, "u_mat", sim.fields[r].mat);
            bind(p.vel, 4, "u_curl", sim.curl.tex);
            draw(sim.vel[w].fbo, size);
            // 3. Incompressibilité : divergence, pression, puis on retire son gradient.
            common(p.div, size);
            bind(p.div, 0, "u_vel", sim.vel[w].tex);
            draw(sim.div.fbo, size);
            common(p.jacobi, size);
            bind(p.jacobi, 1, "u_div", sim.div.tex);
            for it in 0..JACOBI {
                let (a, b) = (it % 2, 1 - it % 2);
                bind(p.jacobi, 0, "u_p", sim.p[a].tex);
                draw(sim.p[b].fbo, size);
            }
            let p_last = JACOBI % 2;
            common(p.grad, size);
            bind(p.grad, 0, "u_vel", sim.vel[w].tex);
            bind(p.grad, 1, "u_p", sim.p[p_last].tex);
            bind(p.grad, 2, "u_mat", sim.fields[r].mat);
            draw(sim.vel[r].fbo, size);
            // La vitesse finale est de nouveau dans vel[r] ; on garde vel[r] comme courante.
            // 4. Transport des champs (couleur, huile, matières), et la foudre qui pousse.
            common(p.fields, size);
            {
                let loc = |n: &str| gl.get_uniform_location(p.fields, n);
                let n = (d.bolt_n as usize).min(crate::chem::MAX_BOLT);
                gl.uniform_1_i32(loc("u_bolt_n").as_ref(), n as i32);
                if n > 0 {
                    gl.uniform_2_f32(loc("u_bolt_c").as_ref(), d.bolt_center[0], d.bolt_center[1]);
                    gl.uniform_2_f32(loc("u_bolt_win").as_ref(), d.bolt_from, d.bolt_to);
                    let pos: Vec<f32> = d.bolt_pos[..n].iter().flatten().copied().collect();
                    let inf: Vec<f32> = d.bolt_info[..n].iter().flatten().copied().collect();
                    gl.uniform_4_f32_slice(loc("u_bolt_p[0]").as_ref(), &pos);
                    gl.uniform_4_f32_slice(loc("u_bolt_i[0]").as_ref(), &inf);
                }
            }
            bind(p.fields, 0, "u_vel", sim.vel[r].tex);
            bind(p.fields, 1, "u_dye", sim.fields[r].dye);
            bind(p.fields, 2, "u_oil", sim.fields[r].oil);
            bind(p.fields, 3, "u_mat", sim.fields[r].mat);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(sim.fields[w].fbo));
            gl.draw_buffers(&[glow::COLOR_ATTACHMENT0, glow::COLOR_ATTACHMENT1, glow::COLOR_ATTACHMENT2]);
            draw(sim.fields[w].fbo, size);
            // 5. Image à afficher, deux fois plus fine que la simulation (avec la foudre, nette).
            common(p.display, display_size);
            {
                let loc = |n: &str| gl.get_uniform_location(p.display, n);
                let n = (d.bolt_n as usize).min(crate::chem::MAX_BOLT);
                gl.uniform_1_i32(loc("u_bolt_n").as_ref(), n as i32);
                if n > 0 {
                    gl.uniform_2_f32(loc("u_bolt_c").as_ref(), d.bolt_center[0], d.bolt_center[1]);
                    gl.uniform_3_f32(loc("u_bolt_show").as_ref(), d.bolt_grow, d.bolt_glow, d.bolt_reach);
                    let pos: Vec<f32> = d.bolt_pos[..n].iter().flatten().copied().collect();
                    let inf: Vec<f32> = d.bolt_info[..n].iter().flatten().copied().collect();
                    gl.uniform_4_f32_slice(loc("u_bolt_p[0]").as_ref(), &pos);
                    gl.uniform_4_f32_slice(loc("u_bolt_i[0]").as_ref(), &inf);
                }
            }
            bind(p.display, 0, "u_dye", sim.fields[w].dye);
            bind(p.display, 1, "u_oil", sim.fields[w].oil);
            bind(p.display, 2, "u_mat", sim.fields[w].mat);
            draw(sim.display.fbo, display_size);
            for unit in 0..5 {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            gl.active_texture(glow::TEXTURE0);
            // Champs : le jeu écrit devient le courant ; la vitesse est restée dans vel[r].
            sim.cur = w;
            // Garder la vitesse avec le jeu courant : on la recopie dans vel[w] au prochain tour
            // en échangeant les rôles (voir plus haut : on lit vel[cur]).
            sim.vel.swap(0, 1);
            Some(sim.display.tex)
        }
    }
}
