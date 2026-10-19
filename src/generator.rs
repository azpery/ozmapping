//! Générateurs : visuels psychédéliques calculés par le shader, utilisables sans vidéo.
//!
//! Un générateur remplace (ou se mélange à) l'image de la vidéo du calque : tous les effets,
//! le masque et la déformation s'appliquent ensuite comme sur une vidéo.

use serde::{Deserialize, Serialize};

/// Motif généré. L'ordre (et donc le numéro envoyé au shader) ne doit pas changer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Pattern {
    #[default]
    Off,
    Plasma,
    Spiral,
    Tunnel,
    Julia,
    Mandala,
    Moire,
    Lava,
    Clouds,
    Crystals,
    OpArt,
    Hyperspace,
    Rays,
    Hive,
    Caustics,
    Aurora,
    Quasicrystal,
    Galaxy,
    Maze,
    Synthwave,
    Flames,
    Electric,
    Ripples,
}

impl Pattern {
    pub const ALL: [Pattern; 23] = [
        Pattern::Off,
        Pattern::Plasma,
        Pattern::Spiral,
        Pattern::Tunnel,
        Pattern::Julia,
        Pattern::Mandala,
        Pattern::Moire,
        Pattern::Lava,
        Pattern::Clouds,
        Pattern::Crystals,
        Pattern::OpArt,
        Pattern::Hyperspace,
        Pattern::Rays,
        Pattern::Hive,
        Pattern::Caustics,
        Pattern::Aurora,
        Pattern::Quasicrystal,
        Pattern::Galaxy,
        Pattern::Maze,
        Pattern::Synthwave,
        Pattern::Flames,
        Pattern::Electric,
        Pattern::Ripples,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Pattern::Off => "Aucun",
            Pattern::Plasma => "Plasma",
            Pattern::Spiral => "Spirale hypnotique",
            Pattern::Tunnel => "Tunnel infini",
            Pattern::Julia => "Fractale",
            Pattern::Mandala => "Mandala",
            Pattern::Moire => "Moiré",
            Pattern::Lava => "Lampe à lave",
            Pattern::Clouds => "Nuages fractals",
            Pattern::Crystals => "Cristaux",
            Pattern::OpArt => "Op art",
            Pattern::Hyperspace => "Hyperespace",
            Pattern::Rays => "Soleil rayonnant",
            Pattern::Hive => "Ruche",
            Pattern::Caustics => "Caustiques",
            Pattern::Aurora => "Aurore boréale",
            Pattern::Quasicrystal => "Quasicristal",
            Pattern::Galaxy => "Galaxie",
            Pattern::Maze => "Labyrinthe",
            Pattern::Synthwave => "Grille rétro",
            Pattern::Flames => "Flammes",
            Pattern::Electric => "Électricité",
            Pattern::Ripples => "Gouttes",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Pattern::Off => "Pas de générateur : le calque affiche sa vidéo",
            Pattern::Plasma => "Ondes de couleurs qui se mélangent, façon démo des années 90",
            Pattern::Spiral => "Bras de spirale qui s'enroulent vers le centre",
            Pattern::Tunnel => "Anneaux et rayures qui défilent vers le fond",
            Pattern::Julia => "Ensemble de Julia qui se transforme en continu",
            Pattern::Mandala => "Pétales et anneaux symétriques qui pulsent",
            Pattern::Moire => "Interférences de cercles concentriques en mouvement",
            Pattern::Lava => "Bulles molles qui fusionnent et se séparent",
            Pattern::Clouds => "Volutes de fumée colorée qui se déforment",
            Pattern::Crystals => "Cellules cristallines qui dérivent",
            Pattern::OpArt => "Rayures noires et colorées qui ondulent",
            Pattern::Hyperspace => "Traînées d'étoiles qui jaillissent du centre",
            Pattern::Rays => "Rayons ondulants qui tournent autour d'un cœur lumineux",
            Pattern::Hive => "Alvéoles hexagonales qui s'allument en vagues",
            Pattern::Caustics => "Reflets de lumière dansants, comme au fond d'une piscine",
            Pattern::Aurora => "Voiles de lumière qui ondulent dans un ciel étoilé",
            Pattern::Quasicrystal => "Rosace d'ondes superposées, jamais tout à fait répétée",
            Pattern::Galaxy => "Bras spiraux poussiéreux autour d'un noyau brillant",
            Pattern::Maze => "Arcs entrelacés dont les tuiles basculent au fil du temps",
            Pattern::Synthwave => "Sol quadrillé qui défile vers un soleil couchant, façon années 80",
            Pattern::Flames => "Flammes qui montent et vacillent (idéal avec la palette Feu)",
            Pattern::Electric => "Arcs électriques qui crépitent et serpentent",
            Pattern::Ripples => "Ronds dans l'eau qui apparaissent et s'élargissent",
        }
    }
}

/// Palette de couleurs du générateur (palettes « cosinus » : a + b·cos 2π(c·t + d)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Palette {
    #[default]
    Rainbow,
    Neon,
    Fire,
    Ocean,
    Acid,
    Pastel,
    Sunset,
    Mono,
}

impl Palette {
    pub const ALL: [Palette; 8] = [
        Palette::Rainbow,
        Palette::Neon,
        Palette::Fire,
        Palette::Ocean,
        Palette::Acid,
        Palette::Pastel,
        Palette::Sunset,
        Palette::Mono,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Palette::Rainbow => "Arc-en-ciel",
            Palette::Neon => "Néon",
            Palette::Fire => "Feu",
            Palette::Ocean => "Océan",
            Palette::Acid => "Acide",
            Palette::Pastel => "Pastel",
            Palette::Sunset => "Coucher de soleil",
            Palette::Mono => "Noir & blanc",
        }
    }

    /// Coefficients a, b, c, d de la palette.
    pub fn coefs(self) -> [[f32; 3]; 4] {
        match self {
            Palette::Rainbow => [[0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [1.0, 1.0, 1.0], [0.0, 0.33, 0.67]],
            Palette::Neon => [[0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [1.0, 1.0, 0.5], [0.8, 0.9, 0.3]],
            Palette::Fire => [[0.6, 0.3, 0.1], [0.5, 0.35, 0.15], [1.0, 1.0, 1.0], [0.0, 0.1, 0.2]],
            Palette::Ocean => [[0.1, 0.4, 0.6], [0.15, 0.35, 0.4], [1.0, 1.0, 1.0], [0.5, 0.6, 0.7]],
            Palette::Acid => [[0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [2.0, 1.0, 0.0], [0.5, 0.2, 0.25]],
            Palette::Pastel => [[0.8, 0.75, 0.8], [0.2, 0.25, 0.2], [1.0, 1.0, 1.0], [0.0, 0.33, 0.67]],
            Palette::Sunset => [[0.6, 0.35, 0.4], [0.4, 0.3, 0.3], [1.0, 0.8, 0.6], [0.0, 0.15, 0.4]],
            Palette::Mono => [[0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]],
        }
    }
}

/// Réglages du générateur d'un calque.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Generator {
    pub pattern: Pattern,
    pub palette: Palette,
    /// Part du générateur par-dessus la vidéo (1 = la remplace). Sans vidéo, il est toujours entier.
    pub mix: f32,
    /// Vitesse d'animation (1 = normale, négative = à l'envers).
    pub speed: f32,
    /// Taille du motif (plus grand = motif plus serré).
    pub scale: f32,
    /// Finesse du motif (0..1) : nombre de bras, de pétales, d'itérations…
    pub detail: f32,
    /// Décalage de la palette (0..1), et sa rotation (tours/s).
    pub hue: f32,
    pub color_speed: f32,
    /// Enchaînement automatique de motifs et de palettes au hasard.
    pub auto: bool,
    /// Durée d'un motif en mode auto, et du fondu vers le suivant, en secondes.
    pub interval: f32,
    pub transition: f32,
}

impl Default for Generator {
    fn default() -> Self {
        Self {
            pattern: Pattern::Off,
            palette: Palette::Rainbow,
            mix: 1.0,
            speed: 1.0,
            scale: 1.0,
            detail: 0.5,
            hue: 0.0,
            color_speed: 0.05,
            auto: false,
            interval: 15.0,
            transition: 3.0,
        }
    }
}

impl Generator {
    pub fn is_active(&self) -> bool {
        self.pattern != Pattern::Off
    }
}
