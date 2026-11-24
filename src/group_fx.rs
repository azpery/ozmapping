//! Effets de groupe de calques : les calques d'un groupe s'allument tour à tour (chenillard,
//! vague, remplissage…), chacun de sa couleur, au tempo ou à la vitesse choisie. Calculé à
//! chaque image : l'opacité et la teinte de chaque calque sont modulées, rien n'est enregistré.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GroupFx {
    #[default]
    Off,
    /// Les calques s'allument l'un après l'autre (avec une traînée qui s'éteint).
    Chase,
    /// Pareil, en allant et revenant.
    Bounce,
    /// Une vague douce qui traverse le groupe.
    Wave,
    /// Un calque au hasard à chaque pas.
    Random,
    /// Les calques s'allument un à un jusqu'à tous, puis tout s'éteint.
    Fill,
    /// Un sur deux, puis les autres.
    Alternate,
    /// Tous allumés, les couleurs passent d'un calque à l'autre.
    ColorCycle,
    /// Le nombre de calques allumés suit le volume du son.
    Meter,
}

impl GroupFx {
    pub const ALL: [GroupFx; 9] = [
        GroupFx::Off,
        GroupFx::Chase,
        GroupFx::Bounce,
        GroupFx::Wave,
        GroupFx::Random,
        GroupFx::Fill,
        GroupFx::Alternate,
        GroupFx::ColorCycle,
        GroupFx::Meter,
    ];

    pub fn name(self) -> &'static str {
        match self {
            GroupFx::Off => "Aucun",
            GroupFx::Chase => "Chenillard",
            GroupFx::Bounce => "Aller-retour",
            GroupFx::Wave => "Vague",
            GroupFx::Random => "Aléatoire",
            GroupFx::Fill => "Remplissage",
            GroupFx::Alternate => "Alterné",
            GroupFx::ColorCycle => "Couleurs tournantes",
            GroupFx::Meter => "VU-mètre (son)",
        }
    }
}

/// Couleurs données aux calques du groupe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupPalette {
    #[default]
    Rainbow,
    Fire,
    Ice,
    Neon,
    /// Pas de couleur : seule la lumière change.
    White,
}

impl GroupPalette {
    pub const ALL: [GroupPalette; 5] = [GroupPalette::Rainbow, GroupPalette::Fire, GroupPalette::Ice, GroupPalette::Neon, GroupPalette::White];

    pub fn name(self) -> &'static str {
        match self {
            GroupPalette::Rainbow => "Arc-en-ciel",
            GroupPalette::Fire => "Feu",
            GroupPalette::Ice => "Glace",
            GroupPalette::Neon => "Néon",
            GroupPalette::White => "Blanc",
        }
    }

    /// Couleur à la position `t` (0..1, en boucle).
    pub fn color(self, t: f32) -> [f32; 3] {
        let t = t.rem_euclid(1.0);
        let lerp = |stops: &[[f32; 3]]| {
            let x = t * stops.len() as f32;
            let i = x.floor() as usize % stops.len();
            let j = (i + 1) % stops.len();
            let f = x.fract();
            [0, 1, 2].map(|k| stops[i][k] + (stops[j][k] - stops[i][k]) * f)
        };
        match self {
            GroupPalette::Rainbow => hsv(t, 0.85, 1.0),
            GroupPalette::Fire => lerp(&[[1.0, 0.15, 0.05], [1.0, 0.55, 0.0], [1.0, 0.9, 0.3], [0.9, 0.3, 0.1]]),
            GroupPalette::Ice => lerp(&[[0.3, 0.6, 1.0], [0.6, 0.95, 1.0], [0.85, 0.9, 1.0], [0.4, 0.45, 1.0]]),
            GroupPalette::Neon => lerp(&[[1.0, 0.1, 0.8], [0.1, 1.0, 0.9], [1.0, 0.95, 0.1], [0.5, 0.2, 1.0]]),
            GroupPalette::White => [1.0, 1.0, 1.0],
        }
    }
}

fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let f = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        v - v * s * (k.min(4.0 - k).clamp(0.0, 1.0))
    };
    [f(5.0), f(3.0), f(1.0)]
}

/// Réglages d'un effet de groupe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupEffect {
    pub kind: GroupFx,
    /// Pas par temps (au tempo), sinon pas par seconde.
    pub speed: f32,
    pub sync: bool,
    pub palette: GroupPalette,
    /// Traînée des calques qui s'éteignent (0 : net ; 1 : longue).
    pub trail: f32,
    /// Niveau des calques éteints (0 : noirs ; 1 : comme d'habitude).
    pub dim: f32,
    /// Force de la couleur posée sur chaque calque (0 : aucune).
    pub tint: f32,
    /// Sens : du bas de la liste vers le haut (sinon du haut vers le bas).
    pub upward: bool,
}

impl Default for GroupEffect {
    fn default() -> Self {
        Self { kind: GroupFx::Off, speed: 1.0, sync: true, palette: GroupPalette::Rainbow, trail: 0.3, dim: 0.0, tint: 0.6, upward: false }
    }
}

/// Ce que l'effet fait à un calque : niveau de lumière (0..1) et couleur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub level: f32,
    pub color: [f32; 3],
}

fn hash(a: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^ (x >> 29)
}

impl GroupEffect {
    /// Lumière du calque de rang `k` (0 : le premier dans le sens de l'effet) parmi `n`, au pas
    /// `phase` (nombre de pas écoulés, fractionnaire) ; `sound` : volume (0..1) pour le VU-mètre.
    pub fn light(&self, k: usize, n: usize, phase: f64, sound: f32) -> Light {
        let n = n.max(1);
        let step = phase.floor();
        let frac = (phase - step) as f32;
        let s = step as i64;
        let nn = n as i64;
        let trail = |age: f32| -> f32 {
            if age < 1.0 {
                1.0
            } else if self.trail > 0.0 {
                (1.0 - (age - 1.0) / (self.trail * n as f32).max(0.3)).max(0.0)
            } else {
                0.0
            }
        };
        let level = match self.kind {
            GroupFx::Off => 1.0,
            GroupFx::Chase => {
                let lit = s.rem_euclid(nn);
                let age = (lit - k as i64).rem_euclid(nn) as f32 + frac;
                trail(age)
            }
            GroupFx::Bounce => {
                if n == 1 {
                    1.0
                } else {
                    let period = 2 * nn - 2;
                    let p = s.rem_euclid(period);
                    let lit = if p < nn { p } else { period - p };
                    if lit == k as i64 { 1.0 } else { 0.0 }
                }
            }
            GroupFx::Wave => {
                let x = (phase / n as f64 - k as f64 / n as f64) * std::f64::consts::TAU;
                (0.5 + 0.5 * x.cos()) as f32
            }
            GroupFx::Random => {
                let lit = (hash(s as u64) % n as u64) as usize;
                if lit == k { trail(frac) } else { 0.0 }
            }
            GroupFx::Fill => {
                let count = s.rem_euclid(nn + 1);
                if (k as i64) < count { 1.0 } else { 0.0 }
            }
            GroupFx::Alternate => {
                if (k as i64 + s).rem_euclid(2) == 0 { 1.0 } else { 0.0 }
            }
            GroupFx::ColorCycle => 1.0,
            GroupFx::Meter => {
                let on = sound.clamp(0.0, 1.0) * n as f32;
                (on - k as f32).clamp(0.0, 1.0)
            }
        };
        // Couleur : chaque calque la sienne (qui tourne avec les pas pour « Couleurs tournantes »).
        let shift = if self.kind == GroupFx::ColorCycle { phase as f32 } else { 0.0 };
        let color = self.palette.color((k as f32 + shift) / n as f32);
        Light { level, color }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(kind: GroupFx) -> GroupEffect {
        GroupEffect { kind, trail: 0.0, ..GroupEffect::default() }
    }

    #[test]
    fn chase_lights_one_after_another() {
        let f = fx(GroupFx::Chase);
        for step in 0..8 {
            let lit: Vec<usize> = (0..4).filter(|&k| f.light(k, 4, step as f64 + 0.2, 0.0).level > 0.5).collect();
            assert_eq!(lit, vec![step % 4]);
        }
    }

    #[test]
    fn bounce_fill_meter() {
        let b = fx(GroupFx::Bounce);
        let order: Vec<usize> = (0..6).map(|s| (0..4).find(|&k| b.light(k, 4, s as f64, 0.0).level > 0.5).unwrap()).collect();
        assert_eq!(order, vec![0, 1, 2, 3, 2, 1]);
        let f = fx(GroupFx::Fill);
        assert_eq!((0..3).filter(|&k| f.light(k, 3, 2.0, 0.0).level > 0.5).count(), 2);
        assert_eq!((0..3).filter(|&k| f.light(k, 3, 3.0, 0.0).level > 0.5).count(), 3);
        assert_eq!((0..3).filter(|&k| f.light(k, 3, 4.0, 0.0).level > 0.5).count(), 0);
        let m = fx(GroupFx::Meter);
        assert_eq!((0..4).filter(|&k| m.light(k, 4, 0.0, 0.5).level > 0.5).count(), 2);
    }

    #[test]
    fn layers_get_different_colors() {
        let f = GroupEffect { kind: GroupFx::Chase, ..GroupEffect::default() };
        let a = f.light(0, 3, 0.0, 0.0).color;
        let b = f.light(1, 3, 0.0, 0.0).color;
        assert!(a.iter().zip(b).any(|(x, y)| (x - y).abs() > 0.2));
    }
}
