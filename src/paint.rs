//! Visuel « Taches de peinture » : les gouttes qui tombent, choisies image par image d'après le son.
//!
//! La peinture elle-même (étalement, mélange, séchage) est simulée sur le GPU par le renderer,
//! qui garde son état d'une image à l'autre. Ici, on décide seulement où tombent les gouttes,
//! leur taille et leur couleur.

use crate::generator::Palette;
use crate::rhythm::Pulse;

/// Gouttes envoyées au GPU par image (au-delà, les suivantes attendent l'image d'après).
pub const MAX_DROPS: usize = 16;
/// Impacts récents dont l'onde pousse encore la peinture.
pub const MAX_IMPACTS: usize = 8;
/// Durée de l'onde d'un impact, en secondes.
const IMPACT_SECS: f32 = 0.7;

/// Rapport K/S de Kubelka-Munk d'une couleur (réflectance d'une couche opaque, par canal).
/// Mélanger des peintures revient à faire la moyenne de leurs K/S, pondérée par la quantité ;
/// le rendu repasse en couleur avec `R = 1 + K/S − √((K/S)² + 2·K/S)`. Les teintes pures
/// sont un peu adoucies : un pigment ne réfléchit jamais 0 ni 100 % de la lumière.
pub fn km_ks(c: f32) -> f32 {
    let r = c.clamp(0.0, 1.0) * 0.94 + 0.03;
    (1.0 - r) * (1.0 - r) / (2.0 * r)
}

/// Une goutte qui tombe sur cette image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaintDrop {
    /// Centre (0..1, y vers le bas) et rayon (en hauteur de l'image).
    pub pos: [f32; 2],
    pub radius: f32,
    /// Graine de la forme (bord irrégulier, éclaboussures).
    pub seed: f32,
    /// Couleur du pigment selon Kubelka-Munk (rapport absorption / diffusion K/S, par canal) :
    /// les pigments se mélangent comme de la vraie peinture (voir [`km_ks`]).
    pub ks: [f32; 3],
    /// Quantité de peinture déposée.
    pub amount: f32,
    /// Éclaboussures autour de la tache (0 = aucune, 1 = beaucoup).
    pub splatter: f32,
}

/// Ce que la simulation reçoit pour une image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaintDraw {
    pub drops: [PaintDrop; MAX_DROPS],
    pub count: u8,
    /// Impacts récents : centre (x, y), rayon de la goutte, âge (s). Leur onde chasse la
    /// peinture alentour.
    pub impacts: [[f32; 4]; MAX_IMPACTS],
    pub impact_count: u8,
    /// Remous de la peinture fraîche (suit le volume).
    pub flow: f32,
    /// Temps de simulation écoulé depuis l'image précédente (vitesse comprise), en secondes.
    pub dt: f32,
    /// Numéro d'image : deux rendus de la même image (aperçu, sortie) ne simulent qu'une fois.
    pub frame: u64,
}

/// Réglages qui viennent du générateur du calque.
pub struct Look {
    pub palette: Palette,
    pub hue: f32,
    /// Taille du motif : plus grand = gouttes plus petites.
    pub scale: f32,
    /// Détail : quantité d'éclaboussures.
    pub detail: f32,
}

/// Cadence des gouttes d'un calque (reste fractionnaire d'une image à l'autre).
#[derive(Default)]
pub struct Spawner {
    acc_bass: f32,
    acc_high: f32,
    acc_idle: f32,
    frame: u64,
    impacts: Vec<[f32; 4]>,
}

impl Spawner {
    /// Gouttes de cette image. `pulse` n'est utilisé que si le rythme est actif ; sans son,
    /// des gouttes tombent quand même, calmement.
    pub fn step(&mut self, pulse: Option<&Pulse>, dt: f32, speed: f32, look: &Look, rand: &mut impl FnMut() -> f32) -> PaintDraw {
        self.frame += 1;
        let mut out = PaintDraw { frame: self.frame, dt: dt * speed.abs(), ..Default::default() };
        let size = 1.0 / look.scale.max(0.2);
        let splash = 0.3 + 1.4 * look.detail.clamp(0.0, 1.0);
        let push = |out: &mut PaintDraw, rand: &mut dyn FnMut() -> f32, radius: f32, splatter: f32, amount: f32| {
            if out.count as usize >= MAX_DROPS {
                return;
            }
            let [r, g, b] = look.palette.color(rand() + look.hue);
            out.drops[out.count as usize] = PaintDrop {
                // Un peu de marge : une tache peut déborder du bord.
                pos: [rand() * 1.1 - 0.05, rand() * 1.1 - 0.05],
                radius: radius * size,
                seed: rand() * 100.0,
                ks: [r, g, b].map(km_ks),
                amount,
                splatter: (splatter * splash).clamp(0.0, 1.0),
            };
            out.count += 1;
        };
        let range = |rand: &mut dyn FnMut() -> f32, a: f32, b: f32| a + (b - a) * rand();

        match pulse {
            Some(p) => {
                // Temps : une grosse tache qui éclabousse, d'autant plus forte que les graves.
                // Temps forts : deux ou trois taches d'un coup.
                if p.beat {
                    let strength = p.bass.max(p.env);
                    let n = 1 + (strength > 0.7) as u32 + (strength > 0.9 && rand() < 0.5) as u32;
                    for _ in 0..n {
                        let k = 0.6 + 0.6 * strength;
                        let r = range(rand, 0.07, 0.14) * k;
                        let s = range(rand, 0.5, 1.0);
                        let a = range(rand, 1.0, 1.6);
                        push(&mut out, rand, r, s, a);
                    }
                }
                // Graves soutenus : gouttes moyennes.
                self.acc_bass += 3.0 * ((p.bass - 0.35).max(0.0) / 0.65).powf(1.5) * dt;
                // Aigus : fines gouttelettes, nombreuses.
                self.acc_high += 14.0 * ((p.high - 0.25).max(0.0) / 0.75).powf(1.5) * dt;
                while self.acc_bass >= 1.0 {
                    self.acc_bass -= 1.0;
                    let r = range(rand, 0.03, 0.065);
                    let s = range(rand, 0.15, 0.5);
                    let a = range(rand, 0.8, 1.3);
                    push(&mut out, rand, r, s, a);
                }
                while self.acc_high >= 1.0 {
                    self.acc_high -= 1.0;
                    let r = range(rand, 0.005, 0.013);
                    let s = range(rand, 0.0, 0.2);
                    let a = range(rand, 0.6, 1.0);
                    push(&mut out, rand, r, s, a);
                }
                out.flow = 0.4 + 2.2 * p.level;
            }
            None => {
                self.acc_idle += 1.6 * dt;
                while self.acc_idle >= 1.0 {
                    self.acc_idle -= 1.0;
                    let big = rand() < 0.25;
                    let r = if big { range(rand, 0.07, 0.12) } else { range(rand, 0.02, 0.06) };
                    let s = if big { range(rand, 0.4, 0.9) } else { range(rand, 0.0, 0.4) };
                    let a = range(rand, 0.9, 1.4);
                    push(&mut out, rand, r, s, a);
                }
                out.flow = 0.6;
            }
        }

        // Impacts : les gouttes assez grosses lancent une onde ; on garde les plus récentes.
        let sim_dt = out.dt;
        self.impacts.retain_mut(|i| {
            i[3] += sim_dt;
            i[3] < IMPACT_SECS
        });
        for d in &out.drops[..out.count as usize] {
            if d.radius > 0.02 {
                self.impacts.push([d.pos[0], d.pos[1], d.radius, 0.0]);
            }
        }
        let skip = self.impacts.len().saturating_sub(MAX_IMPACTS);
        self.impacts.drain(..skip);
        for (slot, i) in out.impacts.iter_mut().zip(&self.impacts) {
            *slot = *i;
        }
        out.impact_count = self.impacts.len() as u8;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look { palette: Palette::Rainbow, hue: 0.0, scale: 1.0, detail: 0.5 }
    }

    fn rng() -> impl FnMut() -> f32 {
        let mut x = 0x1234_5678u32;
        move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as f32 / (1u32 << 24) as f32
        }
    }

    #[test]
    fn beats_drop_a_big_splash() {
        let mut s = Spawner::default();
        let mut r = rng();
        let pulse = Pulse { beat: true, env: 1.0, level: 0.8, bass: 0.9, high: 0.0 };
        let d = s.step(Some(&pulse), 1.0 / 60.0, 1.0, &look(), &mut r);
        assert!((2..=3).contains(&d.count), "temps fort : {} taches", d.count);
        assert!(d.drops[0].radius > 0.05);
        assert!(d.drops[0].ks.iter().all(|a| a.is_finite() && *a >= 0.0));
    }

    #[test]
    fn silence_drops_nothing_and_highs_spray() {
        let mut s = Spawner::default();
        let mut r = rng();
        let quiet = Pulse::default();
        let n: u32 = (0..120).map(|_| s.step(Some(&quiet), 1.0 / 60.0, 1.0, &look(), &mut r).count as u32).sum();
        assert_eq!(n, 0);
        let loud = Pulse { high: 1.0, ..Default::default() };
        let n: u32 = (0..60).map(|_| s.step(Some(&loud), 1.0 / 60.0, 1.0, &look(), &mut r).count as u32).sum();
        assert!(n >= 6, "{n} gouttelettes en 1 s");
    }

    #[test]
    fn without_sound_drops_still_fall() {
        let mut s = Spawner::default();
        let mut r = rng();
        let n: u32 = (0..600).map(|_| s.step(None, 1.0 / 60.0, 1.0, &look(), &mut r).count as u32).sum();
        assert!((10..=22).contains(&n), "{n} gouttes en 10 s");
    }

    #[test]
    fn impacts_push_then_fade() {
        let mut s = Spawner::default();
        let mut r = rng();
        let pulse = Pulse { beat: true, env: 0.5, level: 0.5, bass: 0.5, high: 0.0 };
        let d = s.step(Some(&pulse), 1.0 / 60.0, 1.0, &look(), &mut r);
        assert_eq!(d.impact_count, 1);
        let quiet = Pulse::default();
        let d = (0..60).map(|_| s.step(Some(&quiet), 1.0 / 60.0, 1.0, &look(), &mut r)).last().unwrap();
        assert_eq!(d.impact_count, 0);
    }

    #[test]
    fn kubelka_munk_round_trip() {
        // Une couleur seule redonne (presque) elle-même ; deux couleurs donnent un mélange
        // plus sombre que leur moyenne, comme en peinture.
        let back = |ks: f32| 1.0 + ks - (ks * ks + 2.0 * ks).sqrt();
        for c in [0.0, 0.2, 0.5, 0.8, 1.0] {
            assert!((back(km_ks(c)) - (c * 0.94 + 0.03)).abs() < 1e-4);
        }
        let mix = back(0.5 * (km_ks(0.9) + km_ks(0.1)));
        assert!(mix > 0.1 && mix < 0.5, "{mix}");
    }

    #[test]
    fn frames_are_numbered() {
        let mut s = Spawner::default();
        let mut r = rng();
        let a = s.step(None, 0.016, 1.0, &look(), &mut r);
        let b = s.step(None, 0.016, 1.0, &look(), &mut r);
        assert_ne!(a, b);
    }
}
