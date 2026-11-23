//! Lasers ILDA pilotés point par point par un boîtier Ether Dream (réseau) : formes
//! (cercle, ligne, étoile, vague, spirale, tunnel, dessin à main levée, logo SVG), animations
//! (rotation, couleurs, pulsation sur les temps), sécurité (désarmés au démarrage, zone
//! interdite, luminosité maximale, arrêt immédiat).
//!
//! Protocole Ether Dream : TCP 7765 ; chaque commande reçoit une réponse de 22 octets (réponse,
//! commande, état du boîtier). Les boîtiers s'annoncent en UDP sur le port 7654.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape {
    #[default]
    Circle,
    Line,
    Square,
    Triangle,
    Star,
    Wave,
    Spiral,
    Tunnel,
    /// Traits dessinés à la main, ou logo importé (SVG) : `Cue::strokes`.
    Drawn,
    /// Texte (`Cue::text`), qui peut défiler.
    Text,
    /// Contours de l'image d'une sortie, en direct.
    Video,
    /// Oscilloscope : la forme d'onde du son.
    Scope,
    /// Spectre du son en barres.
    Spectrum,
    /// Cercle qui grandit et se déforme avec le son.
    AudioCircle,
    /// Éventail de faisceaux qui balaie (à voir dans la fumée).
    Fan,
    /// Nappe de lumière (« liquid sky ») qui ondule au-dessus du public.
    Sheet,
    /// Passe d'un réglage mémorisé à un autre, en douceur.
    Morph,
}

impl Shape {
    pub const ALL: [Shape; 17] = [
        Shape::Circle,
        Shape::Line,
        Shape::Square,
        Shape::Triangle,
        Shape::Star,
        Shape::Wave,
        Shape::Spiral,
        Shape::Tunnel,
        Shape::Drawn,
        Shape::Text,
        Shape::Video,
        Shape::Scope,
        Shape::Spectrum,
        Shape::AudioCircle,
        Shape::Fan,
        Shape::Sheet,
        Shape::Morph,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Shape::Circle => "Cercle",
            Shape::Line => "Ligne",
            Shape::Square => "Carré",
            Shape::Triangle => "Triangle",
            Shape::Star => "Étoile",
            Shape::Wave => "Vague",
            Shape::Spiral => "Spirale",
            Shape::Tunnel => "Tunnel",
            Shape::Drawn => "Dessin / logo",
            Shape::Text => "Texte",
            Shape::Video => "Contours vidéo",
            Shape::Scope => "Oscilloscope",
            Shape::Spectrum => "Spectre",
            Shape::AudioCircle => "Cercle sonore",
            Shape::Fan => "Éventail",
            Shape::Sheet => "Nappe",
            Shape::Morph => "Morphing",
        }
    }
}

/// Un réglage de laser (forme et animation), mémorisable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cue {
    pub name: String,
    pub shape: Shape,
    /// Traits de la forme « Dessin / logo » (coordonnées -1..1, y vers le haut).
    pub strokes: Vec<Vec<[f32; 2]>>,
    /// Taille (0-1 : part du champ de balayage) et position (-1..1).
    pub size: f32,
    pub x: f32,
    pub y: f32,
    /// Tours par seconde (négatif : dans l'autre sens).
    pub spin: f32,
    pub color: [u8; 3],
    /// Arc-en-ciel le long du trait, qui défile à `color_speed`.
    pub rainbow: bool,
    pub color_speed: f32,
    /// Pulsation de la taille sur les temps (0-1).
    pub pulse: f32,
    /// Ondulation animée (vague, cercle qui ondule) (0-1).
    pub wobble: f32,
    /// Texte et vitesse de défilement (0 : fixe).
    pub text: String,
    pub scroll: f32,
    /// Contours vidéo : sortie suivie et seuil de clarté (0-1).
    pub video_output: usize,
    pub threshold: f32,
    /// Éventail : nombre de faisceaux ; éventail et nappe : amplitude et vitesse du balayage.
    pub beams: u32,
    pub sweep: f32,
    pub sweep_speed: f32,
    /// Morphing : réglages mémorisés (rangs) de départ et d'arrivée, allers-retours par seconde.
    pub morph_a: usize,
    pub morph_b: usize,
    pub morph_speed: f32,
    /// Réagit au son : la taille suit le volume (0-1).
    pub audio: f32,
}

/// Son qui anime les lasers : la même entrée que l'onglet Rythme, ou une entrée à part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// Entrée à part (`Some(None)` : celle par défaut) ; `None` : celle de l'onglet Rythme.
    pub device: Option<Option<String>>,
    /// Sensibilité (1 : normale).
    pub gain: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { device: None, gain: 1.0 }
    }
}

impl Default for Cue {
    fn default() -> Self {
        Self {
            name: "Cercle".into(),
            shape: Shape::Circle,
            strokes: Vec::new(),
            size: 0.5,
            x: 0.0,
            y: 0.2,
            spin: 0.0,
            color: [0, 255, 80],
            rainbow: false,
            color_speed: 0.3,
            pulse: 0.0,
            wobble: 0.0,
            text: "OZ".into(),
            scroll: 0.0,
            video_output: 0,
            threshold: 0.5,
            beams: 8,
            sweep: 0.5,
            sweep_speed: 0.3,
            morph_a: 0,
            morph_b: 1,
            morph_speed: 0.2,
            audio: 0.0,
        }
    }
}

/// Ce que certaines formes lisent en direct : contours de la vidéo, son, réglages mémorisés.
#[derive(Default)]
pub struct Input<'a> {
    /// Contours de l'image suivie (coordonnées -1..1, y vers le haut).
    pub video: Option<&'a [Vec<[f32; 2]>]>,
    /// Forme d'onde (-1..1) et spectre (bandes 0-1, des graves aux aigus).
    pub wave: &'a [f32],
    pub bands: &'a [f32],
    /// Niveau du son (0-1).
    pub level: f32,
    pub library: &'a [Cue],
}

/// Spectre : `n` bandes (graves → aigus, échelle logarithmique) d'une forme d'onde, 0-1.
pub fn spectrum(wave: &[f32], sample_rate: f32, n: usize) -> Vec<f32> {
    if wave.len() < 64 || sample_rate <= 0.0 {
        return vec![0.0; n];
    }
    let len = wave.len() as f32;
    let bands: Vec<f32> = (0..n)
        .map(|k| {
            // Goertzel à la fréquence centrale de la bande (60 Hz à 10 kHz).
            let f = 60.0 * (10_000.0f32 / 60.0).powf((k as f32 + 0.5) / n as f32);
            let w = std::f32::consts::TAU * f / sample_rate;
            let coeff = 2.0 * w.cos();
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for (i, &x) in wave.iter().enumerate() {
                // Fenêtre de Hann : moins de fuites d'une bande à l'autre.
                let h = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / len).cos();
                let s0 = x * h + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt() / len
        })
        .collect();
    let max = bands.iter().copied().fold(0.004, f32::max);
    bands.into_iter().map(|b| (b / max).sqrt().min(1.0)).collect()
}

/// Police à un seul trait (comme les lasers) : grille 4 × 6, y vers le haut.
fn glyph(c: char) -> &'static str {
    match c {
        'A' => "0,0 2,6 4,0;1,3 3,3",
        'B' => "0,0 0,6 3,6 4,5 4,4 3,3 0,3;3,3 4,2 4,1 3,0 0,0",
        'C' => "4,5 3,6 1,6 0,5 0,1 1,0 3,0 4,1",
        'D' => "0,0 0,6 2,6 4,4 4,2 2,0 0,0",
        'E' => "4,6 0,6 0,0 4,0;0,3 3,3",
        'F' => "4,6 0,6 0,0;0,3 3,3",
        'G' => "4,5 3,6 1,6 0,5 0,1 1,0 3,0 4,1 4,3 2,3",
        'H' => "0,0 0,6;4,0 4,6;0,3 4,3",
        'I' => "1,6 3,6;2,6 2,0;1,0 3,0",
        'J' => "4,6 4,1 3,0 1,0 0,1",
        'K' => "0,0 0,6;4,6 0,2;1,3 4,0",
        'L' => "0,6 0,0 4,0",
        'M' => "0,0 0,6 2,3 4,6 4,0",
        'N' => "0,0 0,6 4,0 4,6",
        'O' => "1,0 0,1 0,5 1,6 3,6 4,5 4,1 3,0 1,0",
        'P' => "0,0 0,6 3,6 4,5 4,4 3,3 0,3",
        'Q' => "1,0 0,1 0,5 1,6 3,6 4,5 4,1 3,0 1,0;2,2 4,0",
        'R' => "0,0 0,6 3,6 4,5 4,4 3,3 0,3;2,3 4,0",
        'S' => "4,5 3,6 1,6 0,5 0,4 1,3 3,3 4,2 4,1 3,0 1,0 0,1",
        'T' => "0,6 4,6;2,6 2,0",
        'U' => "0,6 0,1 1,0 3,0 4,1 4,6",
        'V' => "0,6 2,0 4,6",
        'W' => "0,6 1,0 2,3 3,0 4,6",
        'X' => "0,0 4,6;0,6 4,0",
        'Y' => "0,6 2,3 4,6;2,3 2,0",
        'Z' => "0,6 4,6 0,0 4,0",
        '0' => "1,0 0,1 0,5 1,6 3,6 4,5 4,1 3,0 1,0;0,1 4,5",
        '1' => "1,5 2,6 2,0;1,0 3,0",
        '2' => "0,5 1,6 3,6 4,5 4,4 0,0 4,0",
        '3' => "0,5 1,6 3,6 4,5 4,4 3,3 1,3;3,3 4,2 4,1 3,0 1,0 0,1",
        '4' => "3,0 3,6 0,2 4,2",
        '5' => "4,6 0,6 0,3 3,3 4,2 4,1 3,0 0,0",
        '6' => "4,5 3,6 1,6 0,5 0,1 1,0 3,0 4,1 4,2 3,3 0,3",
        '7' => "0,6 4,6 1,0",
        '8' => "1,3 0,4 0,5 1,6 3,6 4,5 4,4 3,3 1,3 0,2 0,1 1,0 3,0 4,1 4,2 3,3",
        '9' => "4,3 1,3 0,4 0,5 1,6 3,6 4,5 4,1 3,0 1,0",
        '!' => "2,6 2,2;2,0.4 2,0",
        '?' => "0,5 1,6 3,6 4,5 4,4 2,3 2,2;2,0.4 2,0",
        '.' => "2,0.4 2,0",
        ',' => "2,0.6 1.5,-0.6",
        '-' => "1,3 3,3",
        ':' => "2,4 2,3.6;2,1 2,0.6",
        '\'' => "2,6 2,5",
        '+' => "2,1 2,5;0,3 4,3",
        '/' => "0,0 4,6",
        '*' => "2,1 2,5;0.5,2 3.5,4;0.5,4 3.5,2",
        _ => "",
    }
}

/// Lettre de base (accents retirés), en majuscule.
fn base_char(c: char) -> char {
    let c = c.to_uppercase().next().unwrap_or(c);
    match c {
        'À' | 'Â' | 'Ä' | 'Á' => 'A',
        'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'Î' | 'Ï' | 'Í' => 'I',
        'Ô' | 'Ö' | 'Ó' => 'O',
        'Ù' | 'Û' | 'Ü' | 'Ú' => 'U',
        'Ç' => 'C',
        'Ÿ' => 'Y',
        c => c,
    }
}

/// Texte en traits : centré, hauteur 0,5 ; défile à `scroll` (largeurs par seconde) dans la
/// fenêtre -1..1, coupé à ses bords.
fn text_strokes(text: &str, scroll: f32, t: f32) -> Vec<Vec<[f32; 2]>> {
    const ADV: f32 = 5.5;
    let k = 0.5 / 6.0;
    let chars: Vec<char> = text.chars().map(base_char).collect();
    let width = (chars.len() as f32 * ADV - 1.5).max(0.0) * k;
    // Défilement : le texte entre par la droite et sort par la gauche, en boucle.
    let offset = if scroll.abs() > 1e-3 {
        let span = width + 2.0;
        1.0 + width / 2.0 - (t * scroll * 2.0).rem_euclid(span)
    } else {
        0.0
    };
    let mut out = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        for stroke in glyph(*c).split(';').filter(|s| !s.is_empty()) {
            let pts: Vec<[f32; 2]> = stroke
                .split(' ')
                .filter_map(|p| {
                    let (x, y) = p.split_once(',')?;
                    Some([x.parse::<f32>().ok()?, y.parse::<f32>().ok()?])
                })
                .map(|[x, y]| [(x + i as f32 * ADV) * k - width / 2.0 + offset, (y - 3.0) * k])
                .collect();
            // Coupé aux bords de la fenêtre.
            let mut cur: Vec<[f32; 2]> = Vec::new();
            for p in pts {
                if p[0].abs() <= 1.0 {
                    cur.push(p);
                } else if cur.len() >= 2 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
            if cur.len() >= 2 {
                out.push(cur);
            }
        }
    }
    out
}

/// Un trait unique, `n` points répartis le long de tous les traits mis bout à bout.
fn resample(lines: &[Vec<[f32; 2]>], n: usize) -> Vec<[f32; 2]> {
    let pts: Vec<[f32; 2]> = lines.iter().flatten().copied().collect();
    if pts.len() < 2 {
        return vec![pts.first().copied().unwrap_or([0.0, 0.0]); n];
    }
    let seg = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let total: f32 = pts.windows(2).map(|w| seg(w[0], w[1])).sum::<f32>().max(1e-5);
    let mut out = Vec::with_capacity(n);
    let (mut i, mut walked) = (0usize, 0.0f32);
    for k in 0..n {
        let target = total * k as f32 / (n - 1).max(1) as f32;
        while i + 1 < pts.len() - 1 && walked + seg(pts[i], pts[i + 1]) < target {
            walked += seg(pts[i], pts[i + 1]);
            i += 1;
        }
        let l = seg(pts[i], pts[i + 1]).max(1e-6);
        let f = ((target - walked) / l).clamp(0.0, 1.0);
        out.push([pts[i][0] + (pts[i + 1][0] - pts[i][0]) * f, pts[i][1] + (pts[i + 1][1] - pts[i][1]) * f]);
    }
    out
}

/// Laser ILDA de la scène.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Laser {
    pub id: u32,
    pub name: String,
    /// Adresse du boîtier Ether Dream (vide : simulation seulement).
    pub host: String,
    /// Points par seconde (le scanner du laser : voir sa notice, souvent 20 000 à 30 000).
    pub pps: u32,
    /// Position et orientation dans la scène (le faisceau part vers l'avant du boîtier).
    pub pos: [f32; 3],
    pub rot: [f32; 3],
    pub cue: Cue,
    /// Zone interdite : rien n'est tracé sous cette hauteur du champ (-1 : bas, 1 : haut).
    /// Protège le public : les faisceaux restent au-dessus des têtes.
    pub floor: f32,
    /// Luminosité maximale (0-1).
    pub max: f32,
    /// Ouverture du faisceau (degrés), pour la simulation.
    pub fov: f32,
    /// Groupe d'objets de la scène (id, 0 : aucun).
    pub set: u32,
}

impl Default for Laser {
    fn default() -> Self {
        Self {
            id: 1,
            name: "Laser ILDA 1".into(),
            host: String::new(),
            pps: 20_000,
            pos: [0.0, 2.3, 8.0],
            rot: [0.0; 3],
            cue: Cue::default(),
            floor: -0.3,
            max: 0.8,
            fov: 40.0,
            set: 0,
        }
    }
}

/// Point laser : position (-1..1, y vers le haut) et couleur (0-1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Point {
    pub fn lit(&self) -> bool {
        self.r + self.g + self.b > 0.02
    }
}

fn hue(h: f32) -> [f32; 3] {
    crate::leds::hue(h).map(|v| v / 255.0)
}

/// Traits de la forme (avant échelle, rotation et position), rayon 1.
fn strokes(cue: &Cue, t: f32, input: &Input) -> Vec<Vec<[f32; 2]>> {
    use std::f32::consts::TAU;
    let wob = cue.wobble.clamp(0.0, 1.0);
    let poly = |n: usize, r: f32, a0: f32| -> Vec<[f32; 2]> {
        (0..=n).map(|k| {
            let a = a0 + k as f32 / n as f32 * TAU;
            [a.cos() * r, a.sin() * r]
        }).collect()
    };
    match cue.shape {
        Shape::Circle => vec![(0..=96)
            .map(|k| {
                let a = k as f32 / 96.0 * TAU;
                let r = 1.0 + wob * 0.15 * (a * 6.0 + t * 4.0).sin();
                [a.cos() * r, a.sin() * r]
            })
            .collect()],
        Shape::Line => vec![vec![[-1.0, 0.0], [1.0, 0.0]]],
        Shape::Square => vec![poly(4, 1.0, std::f32::consts::FRAC_PI_4)],
        Shape::Triangle => vec![poly(3, 1.0, std::f32::consts::FRAC_PI_2)],
        Shape::Star => vec![(0..=10)
            .map(|k| {
                let a = std::f32::consts::FRAC_PI_2 + k as f32 / 10.0 * TAU;
                let r = if k % 2 == 0 { 1.0 } else { 0.42 };
                [a.cos() * r, a.sin() * r]
            })
            .collect()],
        Shape::Wave => vec![(0..=80)
            .map(|k| {
                let x = k as f32 / 80.0 * 2.0 - 1.0;
                [x, (0.25 + 0.4 * wob) * (x * 6.0 + t * 3.0).sin()]
            })
            .collect()],
        Shape::Spiral => vec![(0..=160)
            .map(|k| {
                let f = k as f32 / 160.0;
                let a = f * TAU * 4.0;
                [a.cos() * f, a.sin() * f]
            })
            .collect()],
        Shape::Tunnel => (0..3)
            .map(|i| {
                // Cercles qui grandissent sans fin : l'effet tunnel.
                let r = ((t * 0.5 + i as f32 / 3.0).fract()).max(0.05);
                poly(64, r, 0.0)
            })
            .collect(),
        Shape::Drawn => cue.strokes.clone(),
        Shape::Text => text_strokes(&cue.text, cue.scroll, t),
        Shape::Video => input.video.map(<[_]>::to_vec).unwrap_or_default(),
        Shape::Scope => {
            // La forme d'onde, de gauche à droite (sans son : une onde calme).
            let n = 120;
            vec![(0..=n)
                .map(|k| {
                    let x = k as f32 / n as f32;
                    let y = if input.wave.len() > n {
                        let i = (x * (input.wave.len() - 1) as f32) as usize;
                        (input.wave[i] * 3.0).clamp(-1.0, 1.0)
                    } else {
                        0.2 * (x * 12.0 + t * 4.0).sin()
                    };
                    [x * 2.0 - 1.0, y * 0.7]
                })
                .collect()]
        }
        Shape::Spectrum => {
            // Une barre par bande, en un seul trait qui monte et descend.
            let n = input.bands.len().max(1);
            let mut line = Vec::new();
            for k in 0..n {
                let h = input.bands.get(k).copied().unwrap_or(0.1 + 0.1 * (t * 3.0 + k as f32).sin().abs()) * 1.6 - 0.8;
                let x = -0.9 + 1.8 * k as f32 / (n - 1).max(1) as f32;
                line.push([x, -0.8]);
                line.push([x, h]);
                line.push([x, -0.8]);
            }
            vec![line]
        }
        Shape::AudioCircle => {
            // Rayon selon le niveau, bord déformé par la forme d'onde.
            let n = 96;
            let base = 0.45 + 0.5 * input.level.clamp(0.0, 1.0);
            vec![(0..=n)
                .map(|k| {
                    let a = k as f32 / n as f32 * TAU;
                    let w = if input.wave.len() > n { input.wave[k * (input.wave.len() - 1) / n] * 2.0 } else { 0.0 };
                    let r = (base + w * (0.3 + wob)).clamp(0.05, 1.0);
                    [a.cos() * r, a.sin() * r]
                })
                .collect()]
        }
        Shape::Fan => {
            // Points fixes (chacun un faisceau dans la fumée), qui balaient de gauche à droite.
            let n = cue.beams.clamp(1, 32) as usize;
            let sweep = cue.sweep.clamp(0.0, 1.0) * 0.5 * (t * cue.sweep_speed * TAU).sin();
            (0..n)
                .map(|k| {
                    let x = if n > 1 { -0.6 + 1.2 * k as f32 / (n - 1) as f32 } else { 0.0 } + sweep;
                    vec![[x, 0.0], [x, 0.0]]
                })
                .collect()
        }
        Shape::Sheet => {
            // Une ligne tracée très vite : une nappe de lumière, qui ondule de haut en bas.
            let tilt = cue.sweep.clamp(0.0, 1.0) * 0.6 * (t * cue.sweep_speed * TAU).sin();
            vec![(0..=40).map(|k| {
                let x = k as f32 / 40.0 * 2.0 - 1.0;
                [x, tilt * x + 0.08 * wob * (x * 8.0 + t * 5.0).sin()]
            }).collect()]
        }
        Shape::Morph => {
            // Les deux réglages mémorisés, ramenés au même nombre de points, puis mélangés.
            let get = |i: usize| input.library.get(i).filter(|c| c.shape != Shape::Morph);
            match (get(cue.morph_a), get(cue.morph_b)) {
                (Some(a), Some(b)) => {
                    let n = 240;
                    let pa = resample(&strokes(a, t, input), n);
                    let pb = resample(&strokes(b, t, input), n);
                    let f = 0.5 - 0.5 * (t * cue.morph_speed * TAU).cos();
                    // Chaque forme garde sa taille et sa position mémorisées.
                    let (sa, sb) = (a.size / cue.size.max(0.05), b.size / cue.size.max(0.05));
                    vec![pa.iter().zip(&pb).map(|(p, q)| {
                        [p[0] * sa * (1.0 - f) + q[0] * sb * f, p[1] * sa * (1.0 - f) + q[1] * sb * f]
                    }).collect()]
                }
                _ => vec![poly(48, 1.0, 0.0)],
            }
        }
    }
}

/// Une image du laser : la forme animée, découpée en `budget` points, avec les déplacements
/// éteints entre les traits et quelques points d'arrêt aux extrémités (les coins restent nets).
pub fn frame(laser: &Laser, t: f32, beat_env: f32, budget: usize, input: &Input) -> Vec<Point> {
    let cue = &laser.cue;
    let scale = cue.size.clamp(0.0, 1.0)
        * (1.0 + cue.pulse.clamp(0.0, 1.0) * 0.6 * beat_env.clamp(0.0, 1.0))
        * (1.0 - cue.audio.clamp(0.0, 1.0) * 0.6 * (1.0 - input.level.clamp(0.0, 1.0)));
    let ang = t * cue.spin * std::f32::consts::TAU;
    let (sa, ca) = ang.sin_cos();
    let place = |p: [f32; 2]| -> [f32; 2] {
        let (x, y) = (p[0] * scale, p[1] * scale);
        [(x * ca - y * sa + cue.x).clamp(-1.0, 1.0), (x * sa + y * ca + cue.y).clamp(-1.0, 1.0)]
    };
    let lines: Vec<Vec<[f32; 2]>> = strokes(cue, t, input)
        .into_iter()
        .filter(|s| s.len() >= 2)
        .map(|s| s.into_iter().map(place).collect())
        .collect();
    if lines.is_empty() {
        return vec![Point::default(); budget.max(1)];
    }
    let seg = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let total: f32 = lines.iter().map(|l| l.windows(2).map(|w| seg(w[0], w[1])).sum::<f32>()).sum::<f32>().max(1e-4);
    const BLANK: usize = 6;
    const DWELL: usize = 3;
    let overhead = lines.len() * (BLANK + 2 * DWELL);
    let lit_budget = budget.saturating_sub(overhead).max(lines.len() * 2);
    let base = [cue.color[0], cue.color[1], cue.color[2]].map(|v| f32::from(v) / 255.0);
    let max = laser.max.clamp(0.0, 1.0);
    let mut out: Vec<Point> = Vec::with_capacity(budget);
    let mut walked = 0.0f32;
    let color_at = |d: f32| -> [f32; 3] {
        if cue.rainbow { hue(d / total + t * cue.color_speed) } else { base }
    };
    let push = |out: &mut Vec<Point>, p: [f32; 2], c: [f32; 3]| {
        // Zone interdite : éteint sous la limite.
        let c = if p[1] < laser.floor { [0.0; 3] } else { c.map(|v| v * max) };
        out.push(Point { x: p[0], y: p[1], r: c[0], g: c[1], b: c[2] });
    };
    let mut last = out.last().map_or(lines[0][0], |p: &Point| [p.x, p.y]);
    for line in &lines {
        // Déplacement éteint jusqu'au début du trait.
        for k in 1..=BLANK {
            let f = k as f32 / BLANK as f32;
            push(&mut out, [last[0] + (line[0][0] - last[0]) * f, last[1] + (line[0][1] - last[1]) * f], [0.0; 3]);
        }
        // Éventail : chaque faisceau est un point fixe, tenu longtemps (il brille dans la fumée).
        let dwell = if cue.shape == Shape::Fan { (lit_budget / lines.len()).max(DWELL) } else { DWELL };
        for _ in 0..dwell {
            push(&mut out, line[0], color_at(walked));
        }
        for w in line.windows(2) {
            let l = seg(w[0], w[1]);
            let n = ((l / total) * lit_budget as f32).round().max(1.0) as usize;
            for k in 1..=n {
                let f = k as f32 / n as f32;
                push(&mut out, [w[0][0] + (w[1][0] - w[0][0]) * f, w[0][1] + (w[1][1] - w[0][1]) * f], color_at(walked + l * f));
            }
            walked += l;
        }
        let end = *line.last().unwrap();
        for _ in 0..DWELL {
            push(&mut out, end, color_at(walked));
        }
        last = end;
    }
    out
}

// ---------------------------------------------------------------- Ether Dream

/// Commande « envoyer des points » : 'd', nombre, puis 18 octets par point.
pub fn data_command(points: &[Point]) -> Vec<u8> {
    let mut c = Vec::with_capacity(3 + points.len() * 18);
    c.push(b'd');
    c.extend_from_slice(&(points.len() as u16).to_le_bytes());
    for p in points {
        let x = (p.x.clamp(-1.0, 1.0) * 32767.0) as i16;
        let y = (p.y.clamp(-1.0, 1.0) * 32767.0) as i16;
        let col = |v: f32| (v.clamp(0.0, 1.0) * 65535.0) as u16;
        let (r, g, b) = (col(p.r), col(p.g), col(p.b));
        c.extend_from_slice(&0u16.to_le_bytes()); // contrôle
        c.extend_from_slice(&x.to_le_bytes());
        c.extend_from_slice(&y.to_le_bytes());
        c.extend_from_slice(&r.to_le_bytes());
        c.extend_from_slice(&g.to_le_bytes());
        c.extend_from_slice(&b.to_le_bytes());
        c.extend_from_slice(&r.max(g).max(b).to_le_bytes()); // intensité
        c.extend_from_slice(&[0, 0, 0, 0]); // u1, u2
    }
    c
}

/// État renvoyé par le boîtier.
#[derive(Clone, Copy, Debug, Default)]
pub struct DacStatus {
    pub light_engine: u8,
    pub playback: u8,
    pub fullness: u16,
}

fn parse_status(b: &[u8]) -> DacStatus {
    DacStatus { light_engine: b[1], playback: b[2], fullness: u16::from_le_bytes([b[10], b[11]]) }
}

/// Points envoyés en continu à un boîtier (la dernière image, rejouée en boucle).
#[derive(Default)]
pub struct Shared {
    pub frame: Vec<Point>,
    pub host: String,
    pub pps: u32,
    /// Lasers armés : sinon, rien ne sort (le boîtier est arrêté).
    pub armed: bool,
    pub status: String,
    pub ok: bool,
}

/// Fil d'envoi d'un laser : se connecte, remplit le tampon du boîtier, s'arrête si désarmé.
pub fn spawn_stream(shared: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || loop {
        let (host, armed) = {
            let s = shared.lock().unwrap();
            (s.host.trim().to_owned(), s.armed)
        };
        if host.is_empty() || !armed {
            {
                let mut s = shared.lock().unwrap();
                s.ok = false;
                s.status = if host.is_empty() { "simulation".into() } else { "désarmé".into() };
            }
            std::thread::sleep(Duration::from_millis(150));
            continue;
        }
        if let Err(e) = stream_once(&host, &shared) {
            let mut s = shared.lock().unwrap();
            s.ok = false;
            s.status = e;
            drop(s);
            std::thread::sleep(Duration::from_millis(800));
        }
    });
}

fn stream_once(host: &str, shared: &Arc<Mutex<Shared>>) -> Result<(), String> {
    let addr: SocketAddr = format!("{host}:7765")
        .to_socket_addrs()
        .map_err(|e| format!("{host} : {e}"))?
        .next()
        .ok_or_else(|| format!("{host} introuvable"))?;
    let mut sock = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|e| format!("{host} ne répond pas ({e})"))?;
    sock.set_read_timeout(Some(Duration::from_secs(1))).ok();
    sock.set_nodelay(true).ok();
    let read = |sock: &mut TcpStream| -> Result<(u8, DacStatus), String> {
        let mut b = [0u8; 22];
        sock.read_exact(&mut b).map_err(|e| format!("réponse : {e}"))?;
        Ok((b[0], parse_status(&b[2..])))
    };
    let cmd = |sock: &mut TcpStream, c: &[u8]| -> Result<(u8, DacStatus), String> {
        sock.write_all(c).map_err(|e| format!("envoi : {e}"))?;
        read(sock)
    };
    // Le boîtier annonce son état dès la connexion.
    let (_, mut st) = read(&mut sock)?;
    if st.light_engine == 3 {
        cmd(&mut sock, b"c")?; // fin d'arrêt d'urgence
    }
    let mut started = false;
    let mut idx = 0usize;
    {
        let mut s = shared.lock().unwrap();
        s.status = "connecté".into();
        s.ok = true;
    }
    loop {
        let (frame, pps, armed, same_host) = {
            let s = shared.lock().unwrap();
            (s.frame.clone(), s.pps.clamp(1000, 100_000), s.armed, s.host.trim() == host)
        };
        if !armed || !same_host {
            // Désarmé : arrêt du boîtier (plus aucun point).
            let _ = cmd(&mut sock, b"s");
            return Ok(());
        }
        if st.playback == 0 {
            let (r, s2) = cmd(&mut sock, b"p")?;
            if r != b'a' {
                return Err(format!("préparation refusée ({})", r as char));
            }
            st = s2;
            started = false;
        }
        let room = 1700usize.saturating_sub(st.fullness as usize);
        if room < 100 || frame.is_empty() {
            std::thread::sleep(Duration::from_millis(4));
            // Interroger l'état pour suivre le remplissage.
            let (_, s2) = cmd(&mut sock, b"?")?;
            st = s2;
            continue;
        }
        let n = room.min(400);
        let pts: Vec<Point> = (0..n).map(|k| frame[(idx + k) % frame.len()]).collect();
        idx = (idx + n) % frame.len();
        let (r, s2) = cmd(&mut sock, &data_command(&pts))?;
        st = s2;
        if r == b'!' {
            return Err("arrêt d'urgence du boîtier".into());
        }
        if !started && st.fullness > 500 {
            let mut b = vec![b'b'];
            b.extend_from_slice(&0u16.to_le_bytes());
            b.extend_from_slice(&pps.to_le_bytes());
            let (r, s2) = cmd(&mut sock, &b)?;
            st = s2;
            started = r == b'a';
        }
    }
}

/// Boîtiers Ether Dream qui s'annoncent sur le réseau (écoute de `wait`).
pub fn discover(wait: Duration) -> Vec<String> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:7654") else { return Vec::new() };
    sock.set_read_timeout(Some(Duration::from_millis(200))).ok();
    let start = Instant::now();
    let mut found = Vec::new();
    let mut buf = [0u8; 64];
    while start.elapsed() < wait {
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            let ip = from.ip().to_string();
            if n >= 36 && !found.contains(&ip) {
                found.push(ip);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fit_the_budget_and_respect_the_floor() {
        let mut l = Laser { floor: 0.0, ..Laser::default() };
        l.cue.y = 0.0;
        let f = frame(&l, 0.0, 0.0, 600, &Input::default());
        assert!(f.len() >= 500 && f.len() <= 700, "{}", f.len());
        // Rien d'allumé sous la zone interdite.
        assert!(f.iter().all(|p| p.y >= 0.0 || !p.lit()));
        assert!(f.iter().any(Point::lit));
    }

    #[test]
    fn new_shapes_draw_something() {
        let mut l = Laser { floor: -1.0, ..Laser::default() };
        let wave: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
        let bands = spectrum(&wave, 44_100.0, 16);
        assert_eq!(bands.len(), 16);
        let library = vec![Cue::default(), Cue { shape: Shape::Star, ..Cue::default() }];
        let input = Input { wave: &wave, bands: &bands, level: 0.5, library: &library, ..Input::default() };
        for sh in [Shape::Text, Shape::Scope, Shape::Spectrum, Shape::AudioCircle, Shape::Fan, Shape::Sheet, Shape::Morph] {
            l.cue.shape = sh;
            l.cue.text = "Bravo à tous !".into();
            let f = frame(&l, 1.0, 0.0, 600, &input);
            assert!(f.iter().any(Point::lit), "{sh:?}");
        }
        // Texte qui défile : il reste dans la fenêtre.
        l.cue.shape = Shape::Text;
        l.cue.scroll = 0.5;
        l.cue.size = 1.0;
        l.cue.x = 0.0;
        assert!(frame(&l, 3.3, 0.0, 600, &input).iter().all(|p| p.x.abs() <= 1.0));
    }

    #[test]
    fn data_command_is_18_bytes_per_point() {
        let c = data_command(&[Point { x: 1.0, y: -1.0, r: 1.0, g: 0.0, b: 0.0 }; 3]);
        assert_eq!(c.len(), 3 + 3 * 18);
        assert_eq!(c[0], b'd');
        assert_eq!(i16::from_le_bytes([c[5], c[6]]), 32767);
        assert_eq!(i16::from_le_bytes([c[7], c[8]]), -32767);
        assert_eq!(u16::from_le_bytes([c[9], c[10]]), 65535);
    }
}
