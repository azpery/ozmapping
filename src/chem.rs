//! Visuel « Chimie » : un bassin de liquides qu'on remplit depuis l'iPad (eau, lait, encre,
//! colorant, huile, essence, peintures, savon, alcool), qu'on touille au doigt, et qui coule ou
//! s'agite quand on penche ou secoue l'iPad — comme les « liquid light shows » des années 60.
//!
//! Ici : ce que la page envoie, mis en file, et ce que la simulation reçoit à chaque image.
//! La simulation elle-même (courants, mélanges, huile, pellicule irisée) tourne sur le GPU
//! (voir `chem_gpu.rs`).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Gouttes et coups de cuillère envoyés au GPU par image (les suivants attendent).
pub const MAX_SPLATS: usize = 16;
/// File d'attente maximale (une page qui envoie trop vite ne fait pas grossir la mémoire).
const MAX_QUEUE: usize = 512;

/// Ce qu'on verse dans le bassin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ingredient {
    /// Eau : dilue et chasse un peu ce qu'il y a.
    Water,
    /// Lait : un nuage blanc et gras.
    Milk,
    /// Encre : des volutes qui s'enroulent.
    #[default]
    Ink,
    /// Colorant alimentaire : se diffuse vite, en nappes.
    Coloring,
    /// Huile : des bulles qui ne se mélangent pas, regroupées, comme des loupes.
    Oil,
    /// Essence : une fine pellicule irisée.
    Gasoline,
    /// Savon : chasse tout vers l'extérieur.
    Soap,
    /// Alcool : creuse des trous, fait des cellules.
    Alcohol,
    /// Peintures : plus ou moins épaisses, couvrantes, brillantes.
    Acrylic,
    Watercolor,
    Gouache,
    Fluo,
    Metallic,
    Marbling,
    /// Cuillère : touille sans rien verser.
    Stir,
    /// Foudre : une figure de Lichtenberg pousse en arborescence depuis le doigt.
    Lightning,
}

impl Ingredient {
    /// Code lu par le shader.
    pub fn code(self) -> u8 {
        self as u8
    }
}

/// Fond du bassin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Base {
    /// Eau dans le noir : seules les couleurs éclairent.
    #[default]
    Dark,
    /// Eau éclairée par-dessous (rétroprojecteur) : les couleurs filtrent la lumière.
    Lit,
    /// Lait : blanc, opaque.
    Milk,
    /// Huile de lin, dorée et visqueuse : l'encre y éclate en doigts ramifiés.
    Oil,
    /// Bulle de savon : une membrane aux couleurs d'interférence, qu'on fait bouger en soufflant.
    Bubble,
}

/// Son qui fait vibrer le bassin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundFrom {
    /// Le rythme d'OZ Mapping (entrée audio ou tempo de l'ordinateur).
    #[default]
    App,
    /// Le micro de l'iPad : la tablette peut suivre un autre son que l'ordinateur.
    Ipad,
    Off,
}

/// Message de l'onglet Chimie de la page.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum ChemMsg {
    /// Verser un ingrédient en (x, y) (0..1, y vers le bas) : taille (part de la hauteur),
    /// couleur "#rrggbb", et le geste du doigt (dx, dy en hauteurs par seconde) pour touiller.
    Drop {
        ing: Ingredient,
        x: f32,
        y: f32,
        #[serde(default = "default_size")]
        size: f32,
        #[serde(default)]
        color: String,
        #[serde(default)]
        dx: f32,
        #[serde(default)]
        dy: f32,
    },
    /// Inclinaison (gravité dans le plan de l'écran, en g, y vers le bas) et secousses (0..1).
    Motion {
        gx: f32,
        gy: f32,
        #[serde(default)]
        shake: f32,
    },
    /// Changer le fond.
    Base { base: Base },
    /// Renouvellement du bassin (0 : rien ne s'efface, 1 : les vieilles couleurs partent vite).
    Renew { rate: f32 },
    /// Secouer d'un coup (bouton, sans capteurs).
    Shake {
        #[serde(default = "one")]
        amount: f32,
    },
    /// Souffle dans le micro de l'iPad (0..1) : la bulle de savon s'agite.
    Blow { amount: f32 },
    /// Foudre sur les temps de la musique.
    BoltBeat { on: bool },
    /// Le bassin vibre avec le son (0 : sourd, 1 : très sensible).
    Sound { amount: f32 },
    /// Quel son fait vibrer le bassin : celui de l'ordinateur, le micro de l'iPad, aucun.
    Listen { from: SoundFrom },
    /// Son entendu par le micro de l'iPad (0..1) : niveau, basses, aigus.
    Hear {
        #[serde(default)]
        level: f32,
        #[serde(default)]
        bass: f32,
        #[serde(default)]
        high: f32,
    },
    /// Tout vider.
    Clear,
}

/// Segments d'une figure de foudre envoyés au GPU (au-delà, les plus fins sont laissés).
pub const MAX_BOLT: usize = 256;
/// Durée de la pousse de la foudre (s), puis de sa rémanence.
const BOLT_SECS: f32 = 0.45;
const AFTERGLOW_SECS: f32 = 1.1;

/// Figure de foudre en train de pousser.
#[derive(Clone, Debug, Default)]
struct Bolt {
    /// Centre (0..1, y vers le bas) ; segments en hauteurs d'image autour du centre :
    /// (x0, y0, x1, y1), et (instant d'arrivée 0..1, épaisseur).
    center: [f32; 2],
    segs: Vec<([f32; 4], [f32; 2])>,
    color: [f32; 3],
    age: f32,
}

/// Sensibilité au son tant que la page n'en a pas donné.
const DEFAULT_SOUND: f32 = 0.5;

fn default_size() -> f32 {
    0.06
}
fn one() -> f32 {
    1.0
}

/// Une goutte (ou un coup de cuillère) pour cette image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Splat {
    /// Centre (0..1, y vers le bas) et rayon (en hauteur de l'image).
    pub pos: [f32; 2],
    pub radius: f32,
    pub kind: u8,
    pub color: [f32; 3],
    /// Mouvement imprimé (hauteurs d'image par seconde, y vers le bas).
    pub vel: [f32; 2],
    /// Graine de la forme (bord irrégulier).
    pub seed: f32,
}

/// Ce que la simulation reçoit pour une image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChemDraw {
    pub splats: [Splat; MAX_SPLATS],
    pub count: u8,
    /// Gravité dans le plan (iPad penché), y vers le bas.
    pub gravity: [f32; 2],
    /// Secousses (0..1), en baisse d'elles-mêmes.
    pub shake: f32,
    pub base: Base,
    /// Le bassin vient d'être vidé.
    pub clear: bool,
    /// Pas de simulation (s) et numéro d'image : deux rendus de la même image ne simulent qu'une fois.
    pub dt: f32,
    pub frame: u64,
    /// Battement des temps (0..1) : le bassin frémit sur la musique.
    pub beat: f32,
    /// Effacement par seconde de ce qui est dans le bassin (0 : jamais).
    pub fade: f32,
    /// Bassin auquel cette image s'adresse (un par calque, gardé tant que le calque existe).
    pub id: u64,
    /// Souffle dans le micro (0..1).
    pub blow: f32,
    /// Son de la musique (niveau, basses, aigus, 0..1, sensibilité comprise) : la surface vibre.
    pub sound: [f32; 3],
    /// Foudre : centre, couleur, segments (voir `Bolt`), et la tranche de pousse de cette image
    /// (les segments arrivés entre `bolt_from` et `bolt_to` s'allument) ; éclair (0..1).
    pub bolt_n: u16,
    pub bolt_center: [f32; 2],
    pub bolt_color: [f32; 3],
    pub bolt_pos: [[f32; 4]; MAX_BOLT],
    pub bolt_info: [[f32; 4]; MAX_BOLT],
    pub bolt_from: f32,
    pub bolt_to: f32,
    /// Affichage net de la foudre : avancée (0..1), intensité (rémanence, scintillement) et
    /// rayon de la figure (hauteurs d'image).
    pub bolt_grow: f32,
    pub bolt_glow: f32,
    pub bolt_reach: f32,
    pub flash: f32,
}

impl Default for ChemDraw {
    fn default() -> Self {
        Self {
            splats: Default::default(),
            count: 0,
            gravity: [0.0; 2],
            shake: 0.0,
            base: Base::default(),
            clear: false,
            dt: 0.0,
            frame: 0,
            beat: 0.0,
            fade: 0.0,
            id: 0,
            blow: 0.0,
            sound: [0.0; 3],
            bolt_n: 0,
            bolt_center: [0.0; 2],
            bolt_color: [0.0; 3],
            bolt_pos: [[0.0; 4]; MAX_BOLT],
            bolt_info: [[0.0; 4]; MAX_BOLT],
            bolt_from: 0.0,
            bolt_to: 0.0,
            bolt_grow: 0.0,
            bolt_glow: 0.0,
            bolt_reach: 0.0,
            flash: 0.0,
        }
    }
}

/// Bassin d'un calque, côté app : ce qui attend d'être versé, et l'état des capteurs.
#[derive(Default)]
pub struct ChemState {
    queue: VecDeque<Splat>,
    gravity: [f32; 2],
    shake: f32,
    base: Base,
    clear: bool,
    /// Renouvellement choisi sur la page (0..1).
    renew: f32,
    blow: f32,
    /// Sensibilité au son choisie sur la page (`None` : réglage par défaut).
    sound_gain: Option<f32>,
    /// Son lissé (niveau, basses, aigus).
    sound: [f32; 3],
    listen: SoundFrom,
    /// Dernier son du micro de l'iPad, et depuis combien de temps (s).
    heard: [f32; 3],
    heard_age: f32,
    bolt: Option<Bolt>,
    bolt_beat: bool,
    flash: f32,
    rng: u64,
    frame: u64,
    seed: u32,
    id: u64,
}

impl ChemState {
    pub fn apply(&mut self, msg: &ChemMsg) {
        match msg {
            ChemMsg::Drop { ing: Ingredient::Lightning, x, y, size, color, .. } => {
                let ok = |v: f32, d: f32| if v.is_finite() { v.clamp(-0.1, 1.1) } else { d };
                let reach = if size.is_finite() { (size * 3.5).clamp(0.08, 0.9) } else { 0.3 };
                self.strike([ok(*x, 0.5), ok(*y, 0.5)], reach, parse_color(color, Ingredient::Lightning));
            }
            ChemMsg::Drop { ing, x, y, size, color, dx, dy } => {
                if self.queue.len() >= MAX_QUEUE {
                    return;
                }
                let ok = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
                self.seed = self.seed.wrapping_add(1);
                self.queue.push_back(Splat {
                    pos: [ok(*x, -0.1, 1.1), ok(*y, -0.1, 1.1)],
                    radius: ok(*size, 0.005, 0.4),
                    kind: ing.code(),
                    color: parse_color(color, *ing),
                    vel: [ok(*dx, -20.0, 20.0), ok(*dy, -20.0, 20.0)],
                    seed: (self.seed % 997) as f32 * 0.731,
                });
            }
            ChemMsg::Motion { gx, gy, shake } => {
                let ok = |v: f32| if v.is_finite() { v.clamp(-1.5, 1.5) } else { 0.0 };
                self.gravity = [ok(*gx), ok(*gy)];
                self.shake = self.shake.max(if shake.is_finite() { shake.clamp(0.0, 1.0) } else { 0.0 });
            }
            ChemMsg::Base { base } => self.base = *base,
            ChemMsg::Renew { rate } => self.renew = if rate.is_finite() { rate.clamp(0.0, 1.0) } else { 0.0 },
            ChemMsg::Shake { amount } => self.shake = self.shake.max(amount.clamp(0.0, 1.0)),
            ChemMsg::Blow { amount } => self.blow = self.blow.max(if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 }),
            ChemMsg::BoltBeat { on } => self.bolt_beat = *on,
            ChemMsg::Listen { from } => self.listen = *from,
            ChemMsg::Hear { level, bass, high } => {
                let ok = |v: &f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
                self.heard = [ok(level), ok(bass), ok(high)];
                self.heard_age = 0.0;
            }
            ChemMsg::Sound { amount } => self.sound_gain = Some(if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 }),
            ChemMsg::Clear => {
                self.queue.clear();
                self.clear = true;
            }
        }
    }

    /// Son de cette image (niveau, basses, aigus, 0..1 ; zéro sans entrée audio) : monte vite,
    /// retombe en un dixième de seconde, comme une membrane.
    pub fn hear(&mut self, sound: [f32; 3], dt: f32) {
        self.heard_age += dt.max(0.0);
        // Micro de l'iPad : son d'il y a moins d'une demi-seconde, sinon silence (page fermée).
        let sound = match self.listen {
            SoundFrom::App => sound,
            SoundFrom::Ipad if self.heard_age < 0.5 => self.heard,
            _ => [0.0; 3],
        };
        let gain = self.sound_gain.unwrap_or(DEFAULT_SOUND) * 2.0;
        let fall = (-dt.max(0.0) * 10.0).exp();
        for (s, v) in self.sound.iter_mut().zip(sound) {
            let v = if v.is_finite() { (v * gain).clamp(0.0, 1.5) } else { 0.0 };
            *s = if v > *s { *s + (v - *s) * 0.6 } else { v + (*s - v) * fall };
        }
    }

    /// Image suivante : jusqu'à `MAX_SPLATS` gouttes de la file, les capteurs, le pas de temps.
    /// Nombre pseudo-aléatoire (0..1).
    fn rand(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x9E37_79B9_7F4A_7C15 ^ self.id.wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ 1;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Lance une figure de foudre en `center`, de rayon `reach` (en hauteurs d'image).
    fn strike(&mut self, center: [f32; 2], reach: f32, color: [f32; 3]) {
        let mut rand = || self.rand();
        let segs = lichtenberg(reach, &mut rand);
        self.bolt = Some(Bolt { center, segs, color, age: 0.0 });
        self.flash = 1.0;
        // La décharge fait un peu sursauter le liquide (sans brouiller la figure).
        self.shake = self.shake.max(0.12);
    }

    /// Image suivante : jusqu'à `MAX_SPLATS` gouttes de la file, les capteurs, la foudre, le
    /// pas de temps. `hit` : un temps de la musique vient de tomber.
    pub fn step(&mut self, dt: f32, beat: f32, hit: bool) -> ChemDraw {
        self.ensure_id();
        self.frame += 1;
        let mut d = ChemDraw {
            gravity: self.gravity,
            shake: self.shake,
            base: self.base,
            clear: std::mem::take(&mut self.clear),
            dt: dt.clamp(0.0, 0.05),
            frame: self.frame,
            beat: beat.clamp(0.0, 1.0),
            fade: renew_rate(self.renew),
            id: self.id,
            ..ChemDraw::default()
        };
        while (d.count as usize) < MAX_SPLATS {
            let Some(s) = self.queue.pop_front() else { break };
            d.splats[d.count as usize] = s;
            d.count += 1;
        }
        // Foudre sur les temps : un éclair à un endroit au hasard.
        if self.bolt_beat && hit {
            let (x, y) = (0.15 + 0.7 * self.rand(), 0.15 + 0.7 * self.rand());
            let reach = 0.2 + 0.3 * self.rand();
            let hue = self.rand();
            let color = if hue < 0.6 { [0.6, 0.78, 1.0] } else if hue < 0.8 { [0.85, 0.6, 1.0] } else { [1.0, 0.85, 0.55] };
            self.strike([x, y], reach, color);
        }
        // Foudre : elle pousse (la tranche de segments arrivés sur cette image laisse sa trace
        // dans le liquide), scintille, puis s'éteint doucement.
        let flicker = 0.75 + 0.35 * self.rand();
        if let Some(b) = &mut self.bolt {
            let from = (b.age / BOLT_SECS).min(1.0);
            b.age += dt.max(0.0);
            let to = (b.age / BOLT_SECS).min(1.0);
            let n = b.segs.len().min(MAX_BOLT);
            d.bolt_n = n as u16;
            d.bolt_center = b.center;
            d.bolt_color = b.color;
            let mut reach: f32 = 0.0;
            for (k, (p, i)) in b.segs.iter().take(n).enumerate() {
                d.bolt_pos[k] = *p;
                d.bolt_info[k] = [i[0], i[1], 0.0, 0.0];
                reach = reach.max(p[2].hypot(p[3]));
            }
            // Première image : tout ce qui arrive à l'instant 0 compte aussi.
            d.bolt_from = if from <= 0.0 { -1.0 } else { from };
            d.bolt_to = to;
            d.bolt_grow = to;
            d.bolt_reach = reach;
            let after = (b.age - BOLT_SECS).max(0.0);
            d.bolt_glow = if after <= 0.0 { flicker } else { (1.0 - after / AFTERGLOW_SECS).max(0.0).powi(2) };
            if after >= AFTERGLOW_SECS {
                self.bolt = None;
                d.bolt_n = 0;
            }
        }
        d.flash = self.flash;
        d.blow = self.blow;
        d.sound = self.sound;
        // Les secousses, le souffle et l'éclair retombent d'eux-mêmes.
        self.shake *= (-dt * 4.0).exp();
        self.blow *= (-dt * 3.0).exp();
        self.flash *= (-dt * 9.0).exp();
        d
    }
}

/// Figure de Lichtenberg : des branches qui partent du centre en zigzag et se divisent en
/// s'affinant. Segments en hauteurs d'image autour du centre, avec leur instant d'arrivée
/// (0..1, selon la distance parcourue) et leur épaisseur ; les plus épais d'abord.
fn lichtenberg(reach: f32, rand: &mut impl FnMut() -> f32) -> Vec<([f32; 4], [f32; 2])> {
    struct Tip {
        p: [f32; 2],
        ang: f32,
        w: f32,
        walked: f32,
    }
    let mut out: Vec<([f32; 4], [f32; 2])> = Vec::new();
    let roots = 3 + (rand() * 3.0) as usize;
    let mut tips: Vec<Tip> = (0..roots)
        .map(|k| Tip { p: [0.0, 0.0], ang: (k as f32 + rand() * 0.6) / roots as f32 * std::f32::consts::TAU, w: 0.0035, walked: 0.0 })
        .collect();
    let step = reach * 0.055;
    // Largeur d'abord (les branches poussent ensemble), jusqu'à remplir le budget.
    let mut guard = 0;
    while !tips.is_empty() && out.len() < MAX_BOLT * 2 && guard < 4000 {
        guard += 1;
        let mut t = tips.remove(0);
        let len = step * (0.6 + 0.8 * rand());
        t.ang += (rand() - 0.5) * 1.1;
        let q = [t.p[0] + t.ang.cos() * len, t.p[1] + t.ang.sin() * len];
        let walked = t.walked + len;
        out.push(([t.p[0], t.p[1], q[0], q[1]], [(walked / reach).min(1.0), t.w]));
        if walked >= reach * (0.55 + 0.45 * rand()) || t.w < 0.0006 {
            continue;
        }
        if rand() < 0.3 {
            let side = if rand() < 0.5 { -1.0 } else { 1.0 };
            tips.push(Tip { p: q, ang: t.ang + side * (0.45 + 0.6 * rand()), w: t.w * 0.62, walked });
        }
        tips.push(Tip { p: q, ang: t.ang, w: t.w * 0.9, walked });
    }
    // Les plus épais d'abord : ce sont eux qu'on garde si la figure est trop grande.
    out.sort_by(|a, b| b.1[1].total_cmp(&a.1[1]));
    out.truncate(MAX_BOLT);
    out
}

/// Réglage de la page (0..1) → effacement par seconde : progressif (au maximum, une couleur
/// perd la moitié de son intensité en un peu plus d'une seconde).
fn renew_rate(r: f32) -> f32 {
    r * r * 0.6
}

/// "#rrggbb" → couleur (0..1) ; sinon la couleur naturelle de l'ingrédient.
fn parse_color(s: &str, ing: Ingredient) -> [f32; 3] {
    let hex = s.trim_start_matches('#');
    let channel = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|v| v as f32 / 255.0);
    match (hex.len(), channel(0), channel(2), channel(4), ing) {
        (_, _, _, _, Ingredient::Milk) => [0.97, 0.96, 0.92],
        (_, _, _, _, Ingredient::Water) => [0.0, 0.0, 0.0],
        (6, Some(r), Some(g), Some(b), _) => [r, g, b],
        (_, _, _, _, Ingredient::Oil) => [1.0, 0.85, 0.35],
        (_, _, _, _, Ingredient::Lightning) => [0.6, 0.78, 1.0],
        _ => [0.2, 0.5, 1.0],
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn basin_png_round_trip() {
        let (w, h) = (6u32, 4u32);
        let n = (w * h * 4) as usize;
        let b = Basin { size: (w, h), fields: [(0..n as u16).collect(), vec![0x3C00; n], (0..n as u16).rev().collect()] };
        let path = std::env::temp_dir().join(format!("oz-bassin-{}.png", std::process::id()));
        b.save_png(&path).unwrap();
        assert_eq!(Basin::load_png(&path).unwrap(), b);
        let _ = std::fs::remove_file(&path);
        // Autre taille : au plus proche.
        assert_eq!(b.resized((3, 2)).fields[1].len(), 3 * 2 * 4);
    }

    use super::*;

    #[test]
    fn page_messages_parse() {
        let m: ChemMsg = serde_json::from_str(r##"{"op":"drop","ing":"oil","x":0.3,"y":0.6,"size":0.1,"color":"#ff0080"}"##).unwrap();
        assert!(matches!(m, ChemMsg::Drop { ing: Ingredient::Oil, .. }));
        let m: ChemMsg = serde_json::from_str(r#"{"op":"motion","gx":0.2,"gy":-0.5,"shake":0.8}"#).unwrap();
        assert_eq!(m, ChemMsg::Motion { gx: 0.2, gy: -0.5, shake: 0.8 });
        let m: ChemMsg = serde_json::from_str(r#"{"op":"base","base":"milk"}"#).unwrap();
        assert_eq!(m, ChemMsg::Base { base: Base::Milk });
        for ing in ["water", "milk", "ink", "coloring", "gasoline", "soap", "alcohol", "acrylic", "watercolor", "gouache", "fluo", "metallic", "marbling", "stir", "lightning"] {
            let m: ChemMsg = serde_json::from_str(&format!(r#"{{"op":"drop","ing":"{ing}","x":0.5,"y":0.5}}"#)).unwrap();
            assert!(matches!(m, ChemMsg::Drop { .. }), "{ing}");
        }
    }

    #[test]
    fn lightning_grows_then_ends() {
        let mut c = ChemState::default();
        let m: ChemMsg = serde_json::from_str(r#"{"op":"drop","ing":"lightning","x":0.5,"y":0.5,"size":0.1}"#).unwrap();
        c.apply(&m);
        let d = c.step(1.0 / 60.0, 0.0, false);
        assert!(d.bolt_n > 20, "une vraie arborescence : {}", d.bolt_n);
        assert!(d.flash > 0.9 && d.bolt_from < 0.0 && d.bolt_to > 0.0);
        // Segments dans le rayon, du plus épais au plus fin.
        let n = d.bolt_n as usize;
        assert!(d.bolt_info[..n].windows(2).all(|w| w[0][1] >= w[1][1]));
        assert!(d.bolt_pos[..n].iter().all(|p| p.iter().all(|v| v.abs() < 0.4)));
        let mut last = d;
        for _ in 0..30 {
            last = c.step(1.0 / 60.0, 0.0, false);
        }
        assert!(last.bolt_grow >= 1.0 && last.bolt_n > 0, "poussée entière, encore allumée");
        assert!(last.bolt_reach > 0.05 && last.bolt_reach < 0.4);
        for _ in 0..90 {
            last = c.step(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(last.bolt_n, 0, "puis elle s'éteint");
        assert!(last.flash < 0.05);
        // Sur les temps : un éclair à chaque temps quand c'est demandé.
        c.apply(&ChemMsg::BoltBeat { on: true });
        assert!(c.step(1.0 / 60.0, 0.0, true).bolt_n > 0);
        // Bases et souffle.
        let m: ChemMsg = serde_json::from_str(r#"{"op":"base","base":"bubble"}"#).unwrap();
        c.apply(&m);
        // Le son peut venir de l'iPad plutôt que de l'ordinateur.
        c.apply(&ChemMsg::Listen { from: SoundFrom::Ipad });
        c.apply(&ChemMsg::Hear { level: 0.8, bass: 0.9, high: 0.1 });
        c.hear([0.0; 3], 1.0 / 60.0);
        assert!(c.sound[1] > 0.5, "basses de l'iPad");
        c.apply(&ChemMsg::Listen { from: SoundFrom::Off });
        for _ in 0..30 {
            c.hear([1.0; 3], 1.0 / 60.0);
        }
        assert!(c.sound[1] < 0.05, "sourd");
        c.apply(&ChemMsg::Blow { amount: 0.8 });
        let d = c.step(1.0 / 60.0, 0.0, false);
        assert_eq!(d.base, Base::Bubble);
        assert!(d.blow > 0.7);
    }

    #[test]
    fn drops_wait_their_turn_and_shaking_fades() {
        let mut c = ChemState::default();
        for k in 0..40 {
            c.apply(&ChemMsg::Drop { ing: Ingredient::Ink, x: k as f32 / 40.0, y: 0.5, size: 0.05, color: "#00ff00".into(), dx: 0.0, dy: 0.0 });
        }
        let d = c.step(1.0 / 60.0, 0.0, false);
        assert_eq!(d.count as usize, MAX_SPLATS);
        assert_eq!(d.splats[0].color, [0.0, 1.0, 0.0]);
        assert_eq!(c.step(1.0 / 60.0, 0.0, false).count as usize, MAX_SPLATS);
        assert_eq!(c.step(1.0 / 60.0, 0.0, false).count, 8);
        // Valeurs folles : bornées.
        c.apply(&ChemMsg::Motion { gx: f32::NAN, gy: 9.0, shake: 1.0 });
        let d = c.step(0.016, 0.0, false);
        assert_eq!(d.gravity, [0.0, 1.5]);
        assert!(d.shake > 0.9);
        for _ in 0..60 {
            c.step(1.0 / 60.0, 0.0, false);
        }
        assert!(c.step(1.0 / 60.0, 0.0, false).shake < 0.05);
        // Renouvellement : rien à 0, de plus en plus vite ensuite, borné.
        assert_eq!(c.step(0.016, 0.0, false).fade, 0.0);
        let m: ChemMsg = serde_json::from_str(r#"{"op":"renew","rate":0.5}"#).unwrap();
        c.apply(&m);
        let half = c.step(0.016, 0.0, false).fade;
        c.apply(&ChemMsg::Renew { rate: 7.0 });
        let full = c.step(0.016, 0.0, false).fade;
        assert!(half > 0.0 && full > half && full <= 0.6);
        c.apply(&ChemMsg::Clear);
        assert!(c.step(0.016, 0.0, false).clear);
        assert!(!c.step(0.016, 0.0, false).clear);
    }
}

// ------------------------------------------------------------------ bassin enregistré

/// Contenu d'un bassin (les trois champs transportés : encres, huile, matières), en demi-flottants
/// RGBA, pour l'enregistrer avec le projet et le retrouver au lancement.
#[derive(Clone, Debug, PartialEq)]
pub struct Basin {
    pub size: (u32, u32),
    pub fields: [Vec<u16>; 3],
}

impl Basin {
    /// Une seule image PNG 16 bits (les trois champs l'un sous l'autre) : sans perte.
    pub fn save_png(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let (w, h) = self.size;
        let data: Vec<u16> = self.fields.iter().flatten().copied().collect();
        let img = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(w, h * 3, data)
            .ok_or_else(|| anyhow::anyhow!("bassin : taille incohérente"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        img.save_with_format(path, image::ImageFormat::Png)?;
        Ok(())
    }

    pub fn load_png(path: &std::path::Path) -> anyhow::Result<Self> {
        let img = image::open(path)?.into_rgba16();
        let (w, h3) = img.dimensions();
        anyhow::ensure!(h3 % 3 == 0 && w > 0, "bassin : image inattendue");
        let h = h3 / 3;
        let all = img.into_raw();
        let n = (w * h * 4) as usize;
        Ok(Self { size: (w, h), fields: [all[..n].to_vec(), all[n..2 * n].to_vec(), all[2 * n..].to_vec()] })
    }

    /// Même bassin à une autre taille (proportions de la sortie changées) : au plus proche.
    pub fn resized(&self, size: (u32, u32)) -> Self {
        if size == self.size {
            return self.clone();
        }
        let (w0, h0) = (self.size.0 as usize, self.size.1 as usize);
        let (w, h) = (size.0 as usize, size.1 as usize);
        let field = |f: &Vec<u16>| {
            let mut out = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                let sy = (y * h0 / h.max(1)).min(h0 - 1);
                for x in 0..w {
                    let sx = (x * w0 / w.max(1)).min(w0 - 1);
                    out.extend_from_slice(&f[(sy * w0 + sx) * 4..(sy * w0 + sx) * 4 + 4]);
                }
            }
            out
        };
        Self { size, fields: [field(&self.fields[0]), field(&self.fields[1]), field(&self.fields[2])] }
    }
}

/// Échanges avec la carte graphique, par bassin (`ChemDraw::id`) : photo demandée, photo
/// prise, bassin à recharger.
#[derive(Default)]
struct BasinIo {
    want: bool,
    got: Option<Basin>,
    restore: Option<std::sync::Arc<Basin>>,
}

static BASIN_IO: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u64, BasinIo>>> =
    std::sync::LazyLock::new(Default::default);

impl ChemState {
    /// Identifiant du bassin (attribué au premier besoin).
    pub fn ensure_id(&mut self) -> u64 {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        if self.id == 0 {
            self.id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        self.id
    }

    /// Demande une photo du bassin (prise à la prochaine image dessinée).
    pub fn request_snapshot(&mut self) {
        let id = self.ensure_id();
        BASIN_IO.lock().unwrap().entry(id).or_default().want = true;
    }

    /// Photo du bassin, si elle est prête.
    pub fn take_snapshot(&mut self) -> Option<Basin> {
        let id = self.ensure_id();
        BASIN_IO.lock().unwrap().get_mut(&id)?.got.take()
    }

    /// Recharge un bassin enregistré (à la prochaine image dessinée).
    pub fn restore(&mut self, basin: Basin) {
        let id = self.ensure_id();
        BASIN_IO.lock().unwrap().entry(id).or_default().restore = Some(std::sync::Arc::new(basin));
    }

    /// Réglages du bassin à garder avec le projet.
    pub fn settings(&self) -> ChemSettings {
        ChemSettings { base: self.base, renew: self.renew, sound: self.sound_gain, listen: self.listen, bolt_beat: self.bolt_beat }
    }

    pub fn apply_settings(&mut self, s: &ChemSettings) {
        self.base = s.base;
        self.renew = s.renew;
        self.sound_gain = s.sound;
        self.listen = s.listen;
        self.bolt_beat = s.bolt_beat;
    }
}

/// Réglages d'un bassin (fond, renouvellement, son), gardés avec le projet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChemSettings {
    pub base: Base,
    pub renew: f32,
    pub sound: Option<f32>,
    pub listen: SoundFrom,
    pub bolt_beat: bool,
}

/// Côté carte graphique : la photo est-elle demandée, et y a-t-il un bassin à recharger ?
pub fn gpu_requests(id: u64) -> (bool, Option<std::sync::Arc<Basin>>) {
    let mut io = BASIN_IO.lock().unwrap();
    match io.get_mut(&id) {
        Some(b) => (std::mem::take(&mut b.want), b.restore.take()),
        None => (false, None),
    }
}

/// Côté carte graphique : la photo demandée.
pub fn gpu_deliver(id: u64, basin: Basin) {
    BASIN_IO.lock().unwrap().entry(id).or_default().got = Some(basin);
}
