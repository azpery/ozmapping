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
    // Psychédélique
    /// Tourbillon : torsion autour du centre (radians au centre).
    pub swirl: f32,
    /// Oscillation du tourbillon (tours/s).
    pub swirl_speed: f32,
    /// Passage en coordonnées polaires : effet tunnel (0..1).
    pub polar: f32,
    /// Avancée dans le tunnel (tours/s).
    pub tunnel_speed: f32,
    /// Mosaïque en miroir : nombre de répétitions (1 = aucune).
    pub tiles: f32,
    /// Distorsion liquide : amplitude, échelle des ondulations, vitesse.
    pub liquid: f32,
    pub liquid_scale: f32,
    pub liquid_speed: f32,
    /// Palette acide : couleurs arc-en-ciel selon la luminosité (0..1), et sa rotation (tours/s).
    pub acid: f32,
    pub acid_speed: f32,
    /// Arc-en-ciel spatial : décalage de teinte selon la position dans l'image.
    pub rainbow: f32,
    /// Solarisation (0..1).
    pub solarize: f32,
    /// Pulsation : zoom qui respire (amplitude), et sa fréquence (Hz).
    pub pulse: f32,
    pub pulse_rate: f32,
    /// Écho : copies zoomées superposées (0..1).
    pub echo: f32,
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
            swirl: 0.0,
            swirl_speed: 0.0,
            polar: 0.0,
            tunnel_speed: 0.0,
            tiles: 1.0,
            liquid: 0.0,
            liquid_scale: 6.0,
            liquid_speed: 1.0,
            acid: 0.0,
            acid_speed: 0.1,
            rainbow: 0.0,
            solarize: 0.0,
            pulse: 0.0,
            pulse_rate: 0.5,
            echo: 0.0,
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
            swirl: l(a.swirl, b.swirl),
            swirl_speed: l(a.swirl_speed, b.swirl_speed),
            polar: l(a.polar, b.polar),
            tunnel_speed: l(a.tunnel_speed, b.tunnel_speed),
            tiles: pick(a.tiles, b.tiles, t),
            liquid: l(a.liquid, b.liquid),
            liquid_scale: l(a.liquid_scale, b.liquid_scale),
            liquid_speed: l(a.liquid_speed, b.liquid_speed),
            acid: l(a.acid, b.acid),
            acid_speed: l(a.acid_speed, b.acid_speed),
            rainbow: l(a.rainbow, b.rainbow),
            solarize: l(a.solarize, b.solarize),
            pulse: l(a.pulse, b.pulse),
            pulse_rate: l(a.pulse_rate, b.pulse_rate),
            echo: l(a.echo, b.echo),
            strobe: pick(a.strobe, b.strobe, t),
        }
    }

    /// Combinaison aléatoire de 2 à 4 effets (tous thèmes), pour l'improvisation.
    pub fn random(seed: u64) -> Effects {
        Self::themed(AutoTheme::All, seed, 1.0, false)
    }

    /// Effet généré au hasard dans un thème. `intensity` (≈ 0,3 à 1,5) dose les réglages ;
    /// `strobe` autorise le stroboscope.
    pub fn themed(theme: AutoTheme, seed: u64, intensity: f32, strobe: bool) -> Effects {
        let mut r = Rng(seed | 1);
        let k = intensity.clamp(0.1, 2.0);
        let mut fx = Effects::default();
        // Chaque « ingrédient » modifie quelques réglages ; un effet en combine plusieurs.
        let pool: &[Ingredient] = match theme {
            AutoTheme::All => &Ingredient::ALL,
            AutoTheme::Psyche => &[
                Ingredient::Swirl,
                Ingredient::Tunnel,
                Ingredient::Liquid,
                Ingredient::Acid,
                Ingredient::Rainbow,
                Ingredient::Kaleido,
                Ingredient::Echo,
                Ingredient::HueCycle,
                Ingredient::Pulse,
                Ingredient::Wave,
            ],
            AutoTheme::Color => &[
                Ingredient::HueShift,
                Ingredient::HueCycle,
                Ingredient::Acid,
                Ingredient::Rainbow,
                Ingredient::Tint,
                Ingredient::Saturate,
                Ingredient::Posterize,
                Ingredient::Solarize,
                Ingredient::Invert,
                Ingredient::Contrast,
            ],
            AutoTheme::Speed => &[
                Ingredient::FastSpin,
                Ingredient::FastKaleido,
                Ingredient::FastPulse,
                Ingredient::FastWave,
                Ingredient::FastTunnel,
                Ingredient::FastHue,
                Ingredient::FastLiquid,
                Ingredient::Strobe,
            ],
            AutoTheme::Geometry => &[
                Ingredient::Kaleido,
                Ingredient::Mirror,
                Ingredient::Tiles,
                Ingredient::Tunnel,
                Ingredient::Zoom,
                Ingredient::Rotate,
                Ingredient::Flip,
                Ingredient::Swirl,
            ],
            AutoTheme::Glitch => &[
                Ingredient::RgbShift,
                Ingredient::Pixelate,
                Ingredient::Posterize,
                Ingredient::Edge,
                Ingredient::GlitchWave,
                Ingredient::Solarize,
                Ingredient::Invert,
                Ingredient::Strobe,
            ],
            AutoTheme::Soft => &[
                Ingredient::SoftColor,
                Ingredient::Vignette,
                Ingredient::SlowHue,
                Ingredient::SoftLiquid,
                Ingredient::SlowRotate,
                Ingredient::SoftTint,
                Ingredient::SoftPulse,
            ],
        };
        let count = 2 + r.below(3);
        let mut picked = 0;
        let mut tries = 0;
        while picked < count && tries < 40 {
            tries += 1;
            let ing = pool[r.below(pool.len())];
            if ing == Ingredient::Strobe && !strobe {
                continue;
            }
            ing.apply(&mut fx, &mut r, k);
            picked += 1;
        }
        fx
    }
}

/// Petit générateur pseudo-aléatoire (xorshift), suffisant ici.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        let x = &mut self.0;
        *x ^= *x << 13;
        *x ^= *x >> 7;
        *x ^= *x << 17;
        (*x >> 11) as f32 / (1u64 << 53) as f32
    }

    /// Entier dans 0..n.
    fn below(&mut self, n: usize) -> usize {
        ((self.next() * n as f32) as usize).min(n.saturating_sub(1))
    }

    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }

    /// Valeur de signe aléatoire.
    fn signed(&mut self, a: f32, b: f32) -> f32 {
        let v = self.range(a, b);
        if self.next() < 0.5 { -v } else { v }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ingredient {
    Swirl,
    Tunnel,
    Liquid,
    Acid,
    Rainbow,
    Kaleido,
    Echo,
    HueCycle,
    Pulse,
    Wave,
    HueShift,
    Tint,
    Saturate,
    Posterize,
    Solarize,
    Invert,
    Contrast,
    FastSpin,
    FastKaleido,
    FastPulse,
    FastWave,
    FastTunnel,
    FastHue,
    FastLiquid,
    Strobe,
    Mirror,
    Tiles,
    Zoom,
    Rotate,
    Flip,
    RgbShift,
    Pixelate,
    Edge,
    GlitchWave,
    SoftColor,
    Vignette,
    SlowHue,
    SoftLiquid,
    SlowRotate,
    SoftTint,
    SoftPulse,
}

impl Ingredient {
    /// Ingrédients du thème « Tout » (sans le stroboscope, géré à part).
    const ALL: [Ingredient; 21] = [
        Ingredient::Swirl,
        Ingredient::Tunnel,
        Ingredient::Liquid,
        Ingredient::Acid,
        Ingredient::Rainbow,
        Ingredient::Kaleido,
        Ingredient::Echo,
        Ingredient::HueCycle,
        Ingredient::Pulse,
        Ingredient::Wave,
        Ingredient::Tint,
        Ingredient::Saturate,
        Ingredient::Posterize,
        Ingredient::Solarize,
        Ingredient::Invert,
        Ingredient::Mirror,
        Ingredient::Tiles,
        Ingredient::Rotate,
        Ingredient::RgbShift,
        Ingredient::Pixelate,
        Ingredient::Edge,
    ];

    fn apply(self, fx: &mut Effects, r: &mut Rng, k: f32) {
        match self {
            Ingredient::Swirl => {
                fx.swirl = r.signed(0.8, 3.0) * k;
                fx.swirl_speed = r.range(0.0, 0.3);
            }
            Ingredient::Tunnel => {
                fx.polar = r.range(0.6, 1.0).min(k);
                fx.tunnel_speed = r.signed(0.1, 0.5) * k;
            }
            Ingredient::Liquid => {
                fx.liquid = r.range(0.01, 0.04) * k;
                fx.liquid_scale = r.range(3.0, 12.0);
                fx.liquid_speed = r.range(0.5, 2.0);
            }
            Ingredient::Acid => {
                fx.acid = r.range(0.5, 1.0).min(k);
                fx.acid_speed = r.signed(0.05, 0.3);
            }
            Ingredient::Rainbow => fx.rainbow = r.range(0.4, 1.5) * k,
            Ingredient::Kaleido => {
                fx.kaleido = (3 + r.below(10)) as f32;
                fx.kaleido_spin = r.signed(0.0, 40.0) * k;
            }
            Ingredient::Echo => fx.echo = r.range(0.4, 1.0).min(k),
            Ingredient::HueCycle => fx.hue_speed = r.signed(0.05, 0.3) * k,
            Ingredient::Pulse => {
                fx.pulse = r.range(0.04, 0.15) * k;
                fx.pulse_rate = r.range(0.2, 1.0);
            }
            Ingredient::Wave => {
                fx.wave_amp = r.range(0.005, 0.025) * k;
                fx.wave_freq = r.range(1.0, 10.0);
            }
            Ingredient::HueShift => fx.hue = r.next(),
            Ingredient::Tint => {
                fx.tint = [r.next(), r.next(), r.next()];
                fx.tint_amount = r.range(0.5, 0.9).min(k);
            }
            Ingredient::Saturate => fx.saturation = 1.0 + r.range(0.4, 1.4) * k,
            Ingredient::Posterize => fx.posterize = (3 + r.below(5)) as f32,
            Ingredient::Solarize => fx.solarize = r.range(0.6, 1.0).min(k),
            Ingredient::Invert => fx.invert = 1.0,
            Ingredient::Contrast => fx.contrast = 1.0 + r.range(0.2, 0.8) * k,
            Ingredient::FastSpin => fx.rotate_speed = r.signed(60.0, 240.0) * k,
            Ingredient::FastKaleido => {
                fx.kaleido = (4 + r.below(8)) as f32;
                fx.kaleido_spin = r.signed(60.0, 180.0) * k;
            }
            Ingredient::FastPulse => {
                fx.pulse = r.range(0.06, 0.2) * k;
                fx.pulse_rate = r.range(1.5, 4.0);
            }
            Ingredient::FastWave => {
                fx.wave_amp = r.range(0.01, 0.03) * k;
                fx.wave_freq = r.range(2.0, 12.0);
                fx.wave_speed = r.range(8.0, 20.0);
            }
            Ingredient::FastTunnel => {
                fx.polar = 1.0;
                fx.tunnel_speed = r.signed(0.8, 2.0) * k;
            }
            Ingredient::FastHue => fx.hue_speed = r.signed(0.5, 1.0) * k,
            Ingredient::FastLiquid => {
                fx.liquid = r.range(0.015, 0.04) * k;
                fx.liquid_scale = r.range(4.0, 10.0);
                fx.liquid_speed = r.range(4.0, 8.0);
            }
            Ingredient::Strobe => fx.strobe = r.range(4.0, 12.0),
            Ingredient::Mirror => fx.mirror = Mirror::ALL[1 + r.below(3)],
            Ingredient::Tiles => fx.tiles = (2 + r.below(4)) as f32,
            Ingredient::Zoom => fx.zoom = r.range(0.5, 1.8),
            Ingredient::Rotate => fx.rotate = r.signed(10.0, 180.0),
            Ingredient::Flip => {
                fx.flip_x = r.next() < 0.5;
                fx.flip_y = !fx.flip_x || r.next() < 0.3;
            }
            Ingredient::RgbShift => fx.rgb_shift = r.range(0.004, 0.02) * k,
            Ingredient::Pixelate => fx.pixelate = r.range(0.3, 0.8).min(k),
            Ingredient::Edge => fx.edge = r.range(0.6, 1.0).min(k),
            Ingredient::GlitchWave => {
                fx.wave_amp = r.range(0.002, 0.008) * k;
                fx.wave_freq = r.range(20.0, 30.0);
                fx.wave_speed = r.range(8.0, 20.0);
            }
            Ingredient::SoftColor => {
                fx.saturation = r.range(1.05, 1.4);
                fx.contrast = r.range(0.85, 1.1);
                fx.brightness = r.range(0.0, 0.08);
            }
            Ingredient::Vignette => fx.vignette = r.range(0.3, 0.7),
            Ingredient::SlowHue => fx.hue_speed = r.signed(0.01, 0.05),
            Ingredient::SoftLiquid => {
                fx.liquid = r.range(0.003, 0.01) * k;
                fx.liquid_scale = r.range(2.0, 5.0);
                fx.liquid_speed = r.range(0.2, 0.6);
            }
            Ingredient::SlowRotate => fx.rotate_speed = r.signed(2.0, 8.0),
            Ingredient::SoftTint => {
                fx.tint = [r.range(0.5, 1.0), r.range(0.5, 1.0), r.range(0.5, 1.0)];
                fx.tint_amount = r.range(0.2, 0.4);
            }
            Ingredient::SoftPulse => {
                fx.pulse = r.range(0.02, 0.05);
                fx.pulse_rate = r.range(0.1, 0.3);
            }
        }
    }
}

/// Thème du mode Auto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AutoTheme {
    /// Presets et effets inventés, tous genres.
    #[default]
    All,
    Psyche,
    Color,
    Speed,
    Geometry,
    Glitch,
    Soft,
}

impl AutoTheme {
    pub const ALL: [AutoTheme; 7] = [
        AutoTheme::All,
        AutoTheme::Psyche,
        AutoTheme::Color,
        AutoTheme::Speed,
        AutoTheme::Geometry,
        AutoTheme::Glitch,
        AutoTheme::Soft,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AutoTheme::All => "Tout",
            AutoTheme::Psyche => "Psychédélique",
            AutoTheme::Color => "Couleur",
            AutoTheme::Speed => "Vitesse",
            AutoTheme::Geometry => "Géométrie",
            AutoTheme::Glitch => "Glitch",
            AutoTheme::Soft => "Doux",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            AutoTheme::All => "Presets et effets inventés de tous les genres",
            AutoTheme::Psyche => "Tourbillons, tunnels, liquide, palette acide, échos, kaléidoscopes",
            AutoTheme::Color => "Couleurs seulement : teintes, palettes, solarisation… sans déformer l'image",
            AutoTheme::Speed => "Mouvements rapides : rotations, pulsations, tunnels et ondes accélérés",
            AutoTheme::Geometry => "Kaléidoscopes, miroirs, mosaïques, tunnels, rotations",
            AutoTheme::Glitch => "Décalage RVB, pixels, postérisation, contours, parasites",
            AutoTheme::Soft => "Ambiance calme : couleurs douces, mouvements lents",
        }
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
    /// Univers des effets choisis.
    pub theme: AutoTheme,
    /// Dosage des effets générés (1 = normal).
    pub intensity: f32,
    /// Thème « Tout » : part des changements qui inventent un nouvel effet au lieu de reprendre
    /// un preset (0..1).
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
            theme: AutoTheme::All,
            intensity: 1.0,
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
        p("Vortex", Effects { swirl: 2.5, swirl_speed: 0.15, hue_speed: 0.08, saturation: 1.4, ..d() }),
        p("Tunnel acide", Effects { polar: 1.0, tunnel_speed: 0.35, acid: 0.8, acid_speed: 0.15, ..d() }),
        p("Liquide", Effects { liquid: 0.035, liquid_scale: 6.0, liquid_speed: 1.5, saturation: 1.4, ..d() }),
        p("Acide", Effects { acid: 1.0, acid_speed: 0.25, contrast: 1.2, ..d() }),
        p("Mosaïque", Effects { tiles: 3.0, rainbow: 0.6, saturation: 1.3, ..d() }),
        p("Écho", Effects { echo: 0.9, pulse: 0.06, pulse_rate: 0.4, ..d() }),
        p("Solarisé", Effects { solarize: 1.0, saturation: 1.6, ..d() }),
        p("Respiration", Effects { pulse: 0.1, pulse_rate: 0.25, vignette: 0.5, ..d() }),
        p("Trip", Effects {
            swirl: 1.2,
            swirl_speed: 0.1,
            liquid: 0.02,
            liquid_scale: 5.0,
            acid: 0.5,
            acid_speed: 0.1,
            rainbow: 0.5,
            kaleido: 6.0,
            kaleido_spin: 15.0,
            ..d()
        }),
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

    #[test]
    fn color_theme_never_deforms_the_image() {
        let d = Effects::default();
        for seed in 1..300u64 {
            let fx = Effects::themed(AutoTheme::Color, seed * 7919, 1.5, true);
            assert_ne!(fx, d);
            let geometry_untouched = fx.zoom == d.zoom
                && fx.rotate == d.rotate
                && fx.rotate_speed == d.rotate_speed
                && fx.kaleido == d.kaleido
                && fx.swirl == d.swirl
                && fx.polar == d.polar
                && fx.tiles == d.tiles
                && fx.liquid == d.liquid
                && fx.wave_amp == d.wave_amp
                && fx.pixelate == d.pixelate
                && fx.mirror == d.mirror
                && !fx.flip_x
                && !fx.flip_y;
            assert!(geometry_untouched, "{fx:?}");
        }
    }

    #[test]
    fn strobe_only_when_allowed() {
        for theme in AutoTheme::ALL {
            for seed in 1..200u64 {
                assert_eq!(Effects::themed(theme, seed * 31, 1.0, false).strobe, 0.0, "{theme:?}");
            }
        }
        let with = (1..200u64).any(|s| Effects::themed(AutoTheme::Speed, s * 31, 1.0, true).strobe > 0.0);
        assert!(with, "le thème Vitesse doit pouvoir utiliser le stroboscope s'il est autorisé");
    }
}
