use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mirror {
    #[default]
    Off,
    Horizontal,
    Vertical,
    Quad,
}

impl Mirror {
    pub const ALL: [Mirror; 4] = [Mirror::Off, Mirror::Horizontal, Mirror::Vertical, Mirror::Quad];

    pub fn label(self) -> &'static str {
        match self {
            Mirror::Off => "Aucun",
            Mirror::Horizontal => "Horizontal",
            Mirror::Vertical => "Vertical",
            Mirror::Quad => "4 quadrants",
        }
    }
}

/// Tous les paramètres d'effets envoyés au shader. Les valeurs par défaut = aucun effet.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    // Couleur
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub hue: f32,
    pub hue_speed: f32,
    pub invert: f32,
    pub posterize: f32,
    pub tint: [f32; 3],
    pub tint_amount: f32,
    pub edge: f32,
    pub vignette: f32,
    // Géométrie
    pub zoom: f32,
    pub rotate: f32,
    pub rotate_speed: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub mirror: Mirror,
    pub kaleido: f32,
    pub kaleido_spin: f32,
    // Distorsions
    pub pixelate: f32,
    pub rgb_shift: f32,
    pub wave_amp: f32,
    pub wave_freq: f32,
    pub wave_speed: f32,
    // Rythme
    pub strobe: f32,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            hue: 0.0,
            hue_speed: 0.0,
            invert: 0.0,
            posterize: 0.0,
            tint: [1.0, 0.2, 0.6],
            tint_amount: 0.0,
            edge: 0.0,
            vignette: 0.0,
            zoom: 1.0,
            rotate: 0.0,
            rotate_speed: 0.0,
            flip_x: false,
            flip_y: false,
            mirror: Mirror::Off,
            kaleido: 0.0,
            kaleido_spin: 0.0,
            pixelate: 0.0,
            rgb_shift: 0.0,
            wave_amp: 0.0,
            wave_freq: 8.0,
            wave_speed: 2.0,
            strobe: 0.0,
        }
    }
}

impl Effects {
    /// Interpolation entre deux jeux d'effets (pour les transitions de presets).
    /// Les réglages discrets (miroir, retournements) basculent à mi-parcours.
    pub fn lerp(a: &Effects, b: &Effects, t: f32) -> Effects {
        let t = t.clamp(0.0, 1.0);
        let l = |x: f32, y: f32| x + (y - x) * t;
        fn pick<T>(x: T, y: T, t: f32) -> T {
            if t < 0.5 { x } else { y }
        }
        Effects {
            brightness: l(a.brightness, b.brightness),
            contrast: l(a.contrast, b.contrast),
            saturation: l(a.saturation, b.saturation),
            hue: l(a.hue, b.hue),
            hue_speed: l(a.hue_speed, b.hue_speed),
            invert: l(a.invert, b.invert),
            posterize: pick(a.posterize, b.posterize, t),
            tint: [l(a.tint[0], b.tint[0]), l(a.tint[1], b.tint[1]), l(a.tint[2], b.tint[2])],
            tint_amount: l(a.tint_amount, b.tint_amount),
            edge: l(a.edge, b.edge),
            vignette: l(a.vignette, b.vignette),
            zoom: l(a.zoom, b.zoom),
            rotate: l(a.rotate, b.rotate),
            rotate_speed: l(a.rotate_speed, b.rotate_speed),
            flip_x: pick(a.flip_x, b.flip_x, t),
            flip_y: pick(a.flip_y, b.flip_y, t),
            mirror: pick(a.mirror, b.mirror, t),
            kaleido: pick(a.kaleido, b.kaleido, t),
            kaleido_spin: l(a.kaleido_spin, b.kaleido_spin),
            pixelate: l(a.pixelate, b.pixelate),
            rgb_shift: l(a.rgb_shift, b.rgb_shift),
            wave_amp: l(a.wave_amp, b.wave_amp),
            wave_freq: l(a.wave_freq, b.wave_freq),
            wave_speed: l(a.wave_speed, b.wave_speed),
            strobe: pick(a.strobe, b.strobe, t),
        }
    }

    /// Combinaison aléatoire de 2 à 4 effets, pour l'improvisation.
    pub fn random(seed: u64) -> Effects {
        // Petit générateur pseudo-aléatoire (xorshift), suffisant ici.
        let mut x = seed | 1;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f32 / (1u64 << 53) as f32
        };
        let mut fx = Effects::default();
        let count = 2 + (rnd() * 3.0) as usize;
        for _ in 0..count {
            match (rnd() * 15.0) as u32 {
                0 => fx.hue_speed = 0.05 + rnd() * 0.3,
                1 => {
                    fx.kaleido = (3 + (rnd() * 8.0) as u32) as f32;
                    fx.kaleido_spin = (rnd() - 0.5) * 60.0;
                }
                2 => fx.rgb_shift = 0.003 + rnd() * 0.015,
                3 => fx.posterize = (3 + (rnd() * 5.0) as u32) as f32,
                4 => fx.edge = 0.6 + rnd() * 0.4,
                5 => fx.mirror = Mirror::ALL[1 + (rnd() * 3.0) as usize % 3],
                6 => {
                    fx.wave_amp = 0.005 + rnd() * 0.02;
                    fx.wave_freq = 1.0 + rnd() * 10.0;
                }
                7 => fx.pixelate = 0.3 + rnd() * 0.5,
                8 => {
                    fx.tint = [rnd(), rnd(), rnd()];
                    fx.tint_amount = 0.5 + rnd() * 0.4;
                }
                9 => fx.rotate_speed = (rnd() - 0.5) * 90.0,
                10 => fx.invert = 1.0,
                11 => {
                    fx.vignette = 0.4 + rnd() * 0.5;
                    fx.contrast = 1.1 + rnd() * 0.5;
                }
                12 => fx.zoom = 0.5 + rnd() * 1.1,
                13 => fx.hue = rnd(),
                _ => fx.saturation = 1.5 + rnd(),
            }
        }
        fx
    }
}

/// Mode Auto : enchaîne des effets au hasard à intervalle régulier.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoFx {
    pub enabled: bool,
    /// Durée pendant laquelle un effet reste affiché, en secondes.
    pub interval: f32,
    /// Durée du fondu vers l'effet suivant, en secondes.
    pub transition: f32,
    /// Part des changements qui inventent un nouvel effet au lieu de reprendre un preset (0..1).
    pub novelty: f32,
    /// Autorise les presets avec stroboscope (désactivé par défaut : flashs agressifs).
    pub strobe: bool,
}

impl Default for AutoFx {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: 8.0,
            transition: 2.0,
            novelty: 0.25,
            strobe: false,
        }
    }
}

/// Un préréglage d'effets mémorisé, rappelable en live.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub effects: Effects,
}

/// Mode de fusion d'un calque avec ceux du dessous.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Screen,
    Multiply,
    Lighten,
    Darken,
}

impl BlendMode {
    pub const ALL: [BlendMode; 6] = [
        BlendMode::Normal,
        BlendMode::Add,
        BlendMode::Screen,
        BlendMode::Multiply,
        BlendMode::Lighten,
        BlendMode::Darken,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Add => "Addition",
            BlendMode::Screen => "Écran",
            BlendMode::Multiply => "Produit",
            BlendMode::Lighten => "Éclaircir",
            BlendMode::Darken => "Assombrir",
        }
    }
}

/// Presets fournis avec le logiciel.
pub fn builtin_presets() -> Vec<Preset> {
    let d = Effects::default;
    let p = |name: &str, effects: Effects| Preset {
        name: name.to_owned(),
        effects,
    };
    vec![
        p("Néon", Effects { edge: 1.0, saturation: 2.2, contrast: 1.4, hue_speed: 0.05, ..d() }),
        p("Kaléido", Effects { kaleido: 6.0, kaleido_spin: 20.0, saturation: 1.3, ..d() }),
        p("Mandala", Effects { kaleido: 12.0, kaleido_spin: -10.0, zoom: 0.7, vignette: 0.7, saturation: 1.5, ..d() }),
        p("Miroir 4", Effects { mirror: Mirror::Quad, ..d() }),
        p("Tunnel", Effects { zoom: 0.45, rotate_speed: 40.0, vignette: 0.9, contrast: 1.2, ..d() }),
        p("Rétro VHS", Effects {
            rgb_shift: 0.006,
            wave_amp: 0.002,
            wave_freq: 25.0,
            wave_speed: 9.0,
            saturation: 0.7,
            contrast: 1.2,
            vignette: 0.5,
            ..d()
        }),
        p("Glitch", Effects { rgb_shift: 0.02, pixelate: 0.35, contrast: 1.5, saturation: 1.4, ..d() }),
        p("8-bit", Effects { pixelate: 0.85, posterize: 6.0, saturation: 1.3, ..d() }),
        p("Pop art", Effects { posterize: 4.0, saturation: 2.2, contrast: 1.3, ..d() }),
        p("Arc-en-ciel", Effects { hue_speed: 0.25, saturation: 1.8, ..d() }),
        p("Psyché", Effects {
            kaleido: 4.0,
            wave_amp: 0.02,
            wave_freq: 3.0,
            rgb_shift: 0.01,
            saturation: 2.0,
            hue_speed: 0.1,
            ..d()
        }),
        p("Rêve", Effects {
            wave_amp: 0.01,
            wave_freq: 2.0,
            wave_speed: 1.0,
            brightness: 0.08,
            contrast: 0.85,
            saturation: 1.3,
            vignette: 0.6,
            ..d()
        }),
        p("Noir & blanc", Effects { saturation: 0.0, contrast: 1.4, ..d() }),
        p("Duotone rose", Effects { tint: [1.0, 0.2, 0.6], tint_amount: 0.85, contrast: 1.3, ..d() }),
        p("Bleu glacé", Effects { tint: [0.3, 0.65, 1.0], tint_amount: 0.6, saturation: 0.6, brightness: 0.05, ..d() }),
        p("Négatif", Effects { invert: 1.0, ..d() }),
        p("Strobe", Effects { strobe: 8.0, contrast: 1.3, ..d() }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_effects_are_varied_and_never_neutral() {
        let all: Vec<Effects> = (1..200u64).map(|s| Effects::random(s.wrapping_mul(0x9E37_79B9_7F4A_7C15))).collect();
        assert!(all.iter().all(|fx| *fx != Effects::default()));
        let distinct = all.iter().enumerate().filter(|(i, fx)| !all[..*i].contains(fx)).count();
        assert!(distinct > 190, "seulement {distinct} effets différents");
    }
}
