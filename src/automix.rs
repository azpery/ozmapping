//! Mix auto : enchaîne au hasard des séquences tirées de plusieurs vidéos (saut dans le temps,
//! changement de vidéo, lecture à l'envers, vitesse variable, coupe franche ou fondu).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Réglages du mix auto d'un calque (enregistrés dans le projet).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoMix {
    pub enabled: bool,
    /// Vidéos utilisées par le mix.
    pub clips: Vec<PathBuf>,
    /// Durée d'une séquence, en secondes (tirée entre les deux bornes).
    pub seg_min: f32,
    pub seg_max: f32,
    /// Probabilité qu'une séquence soit jouée à l'envers.
    pub reverse_chance: f32,
    /// Probabilité qu'une séquence change de vitesse (sinon vitesse normale), et bornes.
    pub speed_chance: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    /// Probabilité de rester sur la même vidéo (en sautant à un autre moment).
    pub same_clip_chance: f32,
    /// Probabilité d'une coupe franche ; sinon fondu de `fade` secondes.
    pub cut_chance: f32,
    pub fade: f32,
}

impl Default for AutoMix {
    fn default() -> Self {
        Self {
            enabled: false,
            clips: Vec::new(),
            seg_min: 1.0,
            seg_max: 4.0,
            reverse_chance: 0.3,
            speed_chance: 0.5,
            speed_min: 0.5,
            speed_max: 2.0,
            same_clip_chance: 0.3,
            cut_chance: 0.6,
            fade: 0.5,
        }
    }
}

/// Une séquence à jouer.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub clip: PathBuf,
    /// Position de départ, en fraction de la durée de la vidéo (0..1).
    pub start: f64,
    pub reverse: bool,
    pub speed: f32,
    /// Durée de la séquence, en secondes.
    pub length: f32,
    /// Durée du fondu d'entrée (0 = coupe franche).
    pub fade: f32,
    /// Même vidéo que la séquence précédente : on saute dans le temps sans relancer de lecteur.
    pub same_clip: bool,
}

/// Tire la séquence suivante. `rnd` renvoie des nombres dans 0..1.
pub fn next_segment(mix: &AutoMix, current: Option<&Path>, rnd: &mut impl FnMut() -> f32) -> Option<Segment> {
    if mix.clips.is_empty() {
        return None;
    }
    let current_in_mix = current.filter(|c| mix.clips.iter().any(|p| p == c));
    let same_clip = current_in_mix.is_some() && (mix.clips.len() == 1 || rnd() < mix.same_clip_chance);
    let clip = match current_in_mix {
        Some(c) if same_clip => c.to_path_buf(),
        _ => {
            // Une autre vidéo que l'actuelle quand c'est possible.
            let others: Vec<&PathBuf> = mix.clips.iter().filter(|p| Some(p.as_path()) != current).collect();
            let pool = if others.is_empty() { mix.clips.iter().collect() } else { others };
            pool[((rnd() * pool.len() as f32) as usize).min(pool.len() - 1)].clone()
        }
    };
    let (lo, hi) = (mix.seg_min.min(mix.seg_max), mix.seg_min.max(mix.seg_max));
    let speed = if rnd() < mix.speed_chance {
        // Tirage logarithmique : autant de ralentis que d'accélérés.
        let (a, b) = (mix.speed_min.max(0.05).ln(), mix.speed_max.max(0.05).ln());
        (a + (b - a) * rnd()).exp()
    } else {
        1.0
    };
    Some(Segment {
        clip,
        start: rnd() as f64,
        reverse: rnd() < mix.reverse_chance,
        speed,
        length: (lo + (hi - lo) * rnd()).max(0.2),
        fade: if same_clip || rnd() < mix.cut_chance { 0.0 } else { mix.fade },
        same_clip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: u64) -> impl FnMut() -> f32 {
        let mut x = seed | 1;
        move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f32 / (1u64 << 53) as f32
        }
    }

    fn mix(clips: &[&str]) -> AutoMix {
        AutoMix {
            enabled: true,
            clips: clips.iter().map(PathBuf::from).collect(),
            ..AutoMix::default()
        }
    }

    #[test]
    fn segments_respect_the_settings() {
        let m = AutoMix { seg_min: 2.0, seg_max: 5.0, speed_min: 0.5, speed_max: 2.0, ..mix(&["a", "b", "c"]) };
        let mut r = rng(42);
        let mut reversed = 0;
        let mut current: Option<PathBuf> = None;
        for _ in 0..500 {
            let s = next_segment(&m, current.as_deref(), &mut r).unwrap();
            assert!(m.clips.contains(&s.clip));
            assert!((2.0..=5.0).contains(&s.length), "{}", s.length);
            assert!((0.49..=2.01).contains(&s.speed), "{}", s.speed);
            assert!((0.0..1.0).contains(&s.start));
            if !s.same_clip {
                assert_ne!(Some(&s.clip), current.as_ref(), "un changement de vidéo doit changer de vidéo");
            } else {
                assert_eq!(s.fade, 0.0);
            }
            reversed += s.reverse as usize;
            current = Some(s.clip);
        }
        // ≈ 30 % à l'envers.
        assert!((100..200).contains(&reversed), "{reversed}");
    }

    #[test]
    fn empty_mix_gives_nothing_and_single_clip_stays() {
        let mut r = rng(7);
        assert!(next_segment(&mix(&[]), None, &mut r).is_none());
        let one = mix(&["seul"]);
        let first = next_segment(&one, None, &mut r).unwrap();
        assert!(!first.same_clip);
        let second = next_segment(&one, Some(&first.clip), &mut r).unwrap();
        assert!(second.same_clip && second.clip == first.clip);
    }
}
