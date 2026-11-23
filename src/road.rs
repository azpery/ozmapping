//! Visuel « Route cartoon » : une route en 3D qui défile, pilotée en direct.
//!
//! Ici, on tient le tracé de la route (virages et bosses à venir), la météo et les éclairs ;
//! le dessin lui-même (relief, ciel, décor, voiture) est fait par le shader du générateur,
//! qui reçoit la route sous forme de points échantillonnés devant la caméra.

use serde::{Deserialize, Serialize};

/// Points de route envoyés au shader, espacés de `STEP` devant la caméra.
pub const SAMPLES: usize = 96;
pub const STEP: f32 = 3.0;
/// Distance visible devant la caméra.
const RANGE: f32 = SAMPLES as f32 * STEP;
/// La distance parcourue repart de zéro à cette valeur (le décor du shader a la même période),
/// pour garder toute la précision des f32 pendant des heures.
pub const PERIOD: f32 = 8192.0;
/// Vitesse de croisière (unités de route par seconde, à la vitesse 1 du générateur).
const CRUISE: f32 = 40.0;
/// Courbure d'un virage serré, et hauteur maximale d'une bosse.
const MAX_CURVE: f32 = 0.011;
const MAX_BUMP: f32 = 16.0;
/// Hauteur des collines du paysage à fond (pour borner le lancer de rayons).
const HILL_HEIGHT: f32 = 38.0;
/// Temps que met un virage demandé à la main pour se former sous les yeux, en secondes.
const GROW_SECS: f32 = 0.8;
/// Temps de passage d'une météo à l'autre, en secondes.
const WEATHER_SECS: f32 = 1.5;

/// Voiture vue de dos. L'ordre (numéro envoyé au shader) ne doit pas changer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Car {
    None,
    #[default]
    Sport,
    Van,
    Pickup,
    Beetle,
}

impl Car {
    pub const ALL: [Car; 5] = [Car::Sport, Car::Van, Car::Pickup, Car::Beetle, Car::None];

    pub fn label(self) -> &'static str {
        match self {
            Car::None => "Sans voiture",
            Car::Sport => "Sportive",
            Car::Van => "Combi",
            Car::Pickup => "Pick-up",
            Car::Beetle => "Coccinelle",
        }
    }
}

/// Décor du bord de la route. L'ordre (numéro envoyé au shader) ne doit pas changer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Decor {
    None,
    Trees,
    Palms,
    Mushrooms,
    Cactus,
    Crystals,
    #[default]
    Mixed,
}

impl Decor {
    pub const ALL: [Decor; 7] =
        [Decor::Mixed, Decor::Trees, Decor::Palms, Decor::Mushrooms, Decor::Cactus, Decor::Crystals, Decor::None];

    pub fn label(self) -> &'static str {
        match self {
            Decor::None => "Rien",
            Decor::Trees => "Arbres",
            Decor::Palms => "Palmiers",
            Decor::Mushrooms => "Champignons",
            Decor::Cactus => "Cactus",
            Decor::Crystals => "Cristaux",
            Decor::Mixed => "Mélange",
        }
    }
}

/// Météos toutes prêtes : chacune règle d'un coup nuages, pluie, éclairs, lumière et brouillard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weather {
    Sunny,
    Cloudy,
    Rain,
    Storm,
    Fog,
    Night,
}

impl Weather {
    pub const ALL: [Weather; 6] =
        [Weather::Sunny, Weather::Cloudy, Weather::Rain, Weather::Storm, Weather::Fog, Weather::Night];

    pub fn label(self) -> &'static str {
        match self {
            Weather::Sunny => "Beau temps",
            Weather::Cloudy => "Nuageux",
            Weather::Rain => "Pluie",
            Weather::Storm => "Orage",
            Weather::Fog => "Brouillard",
            Weather::Night => "Nuit étoilée",
        }
    }

    /// Nuages, pluie, éclairs, lumière du jour, brouillard.
    fn values(self) -> [f32; 5] {
        match self {
            Weather::Sunny => [0.15, 0.0, 0.0, 1.0, 0.0],
            Weather::Cloudy => [0.65, 0.0, 0.0, 0.85, 0.1],
            Weather::Rain => [0.85, 0.7, 0.0, 0.7, 0.25],
            Weather::Storm => [1.0, 1.0, 0.7, 0.4, 0.3],
            Weather::Fog => [0.5, 0.0, 0.0, 0.8, 0.9],
            Weather::Night => [0.1, 0.0, 0.0, 0.0, 0.0],
        }
    }
}

/// Réglages de la route d'un calque (enregistrés avec son générateur).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoadSettings {
    pub car: Car,
    /// Couleur de la voiture : position dans la palette (0..1).
    pub car_hue: f32,
    pub decor: Decor,
    /// Virages et bosses tirés au hasard (0 = route droite et plate, sauf demandes à la main).
    pub curves: f32,
    pub bumps: f32,
    /// Hauteur des collines du paysage.
    pub hills: f32,
    pub clouds: f32,
    pub rain: f32,
    /// Fréquence des éclairs.
    pub storm: f32,
    /// 1 = plein jour, 0 = nuit.
    pub daylight: f32,
    pub fog: f32,
    /// Délire psychédélique : bandes qui ondulent, couleurs qui tournent.
    pub psyche: f32,
}

impl Default for RoadSettings {
    fn default() -> Self {
        let [clouds, rain, storm, daylight, fog] = Weather::Sunny.values();
        Self {
            car: Car::default(),
            car_hue: 0.0,
            decor: Decor::default(),
            curves: 0.6,
            bumps: 0.5,
            hills: 0.5,
            clouds,
            rain,
            storm,
            daylight,
            fog,
            psyche: 0.5,
        }
    }
}

impl RoadSettings {
    pub fn set_weather(&mut self, w: Weather) {
        [self.clouds, self.rain, self.storm, self.daylight, self.fog] = w.values();
    }

    /// La météo toute prête qui correspond aux réglages, s'il y en a une.
    pub fn weather(&self) -> Option<Weather> {
        let now = [self.clouds, self.rain, self.storm, self.daylight, self.fog];
        Weather::ALL.into_iter().find(|w| w.values() == now)
    }
}

/// Ce que le shader reçoit pour une image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadDraw {
    /// Route devant la caméra : décalage latéral et hauteur, tous les `STEP`.
    pub pts: [[f32; 2]; SAMPLES],
    /// Distance parcourue (0..PERIOD), cap accumulé (fait défiler le ciel), braquage (-1..1),
    /// inclinaison de la caméra dans les pentes.
    pub pos: f32,
    pub heading: f32,
    pub steer: f32,
    pub pitch: f32,
    /// Météo adoucie (voir `RoadSettings`).
    pub clouds: f32,
    pub rain: f32,
    pub daylight: f32,
    pub fog: f32,
    pub hills: f32,
    pub psyche: f32,
    /// Éclair : intensité (1 à l'impact, retombe vite), position (-1..1) et graine de sa forme.
    pub flash: f32,
    pub bolt_x: f32,
    pub bolt_seed: f32,
    pub car: Car,
    pub car_hue: f32,
    pub decor: Decor,
    /// Part des emplacements de décor occupés (0..1).
    pub density: f32,
    /// Hauteur au-dessus de laquelle il n'y a plus de relief.
    pub ceiling: f32,
}

impl Default for RoadDraw {
    fn default() -> Self {
        Self {
            pts: [[0.0; 2]; SAMPLES],
            pos: 0.0,
            heading: 0.0,
            steer: 0.0,
            pitch: 0.0,
            clouds: 0.0,
            rain: 0.0,
            daylight: 1.0,
            fog: 0.0,
            hills: 0.0,
            psyche: 0.0,
            flash: 0.0,
            bolt_x: 0.0,
            bolt_seed: 0.0,
            car: Car::None,
            car_hue: 0.0,
            decor: Decor::None,
            density: 0.0,
            ceiling: 0.0,
        }
    }
}

/// Virage ou bosse : une cloche lisse (nulle aux deux bouts) posée sur un tronçon de route.
/// Les cloches s'additionnent, on peut donc en ajouter une n'importe où sans cassure.
#[derive(Clone, Copy, Debug)]
struct Bell {
    start: f32,
    len: f32,
    amp: f32,
    /// 0..1 : une cloche demandée à la main grandit en douceur au lieu d'apparaître d'un coup.
    gain: f32,
}

impl Bell {
    fn phase(&self, s: f32) -> Option<f32> {
        let t = (s - self.start) / self.len;
        (0.0..=1.0).contains(&t).then_some(t * std::f32::consts::TAU)
    }

    fn at(&self, s: f32) -> f32 {
        self.phase(s).map_or(0.0, |p| self.amp * self.gain * (0.5 - 0.5 * p.cos()))
    }

    fn slope(&self, s: f32) -> f32 {
        self.phase(s).map_or(0.0, |p| self.amp * self.gain * std::f32::consts::PI / self.len * p.sin())
    }
}

/// Tracé de la route d'un calque : avance d'image en image.
#[derive(Default)]
pub struct Track {
    pos: f32,
    heading: f32,
    curves: Vec<Bell>,
    bumps: Vec<Bell>,
    /// Où commencera le prochain virage (la prochaine bosse) tiré au hasard.
    next_curve: f32,
    next_bump: f32,
    steer: f32,
    pitch: f32,
    /// Météo adoucie : nuages, pluie, lumière, brouillard, collines (None avant la première image).
    weather: Option<[f32; 5]>,
    flash: f32,
    bolt: [f32; 2],
    /// Secondes avant le prochain éclair d'orage.
    next_strike: f32,
}

impl Track {
    fn curve(&self, s: f32) -> f32 {
        self.curves.iter().map(|b| b.at(s)).sum()
    }

    fn height(&self, s: f32) -> f32 {
        self.bumps.iter().map(|b| b.at(s)).sum()
    }

    /// Virage à venir, juste devant la voiture : `dir` < 0 à gauche, > 0 à droite (1 = serré).
    pub fn turn(&mut self, dir: f32) {
        self.curves.push(Bell { start: self.pos + 25.0, len: 220.0, amp: dir * MAX_CURVE, gain: 0.0 });
    }

    /// Deux virages qui s'enchaînent en S.
    pub fn chicane(&mut self) {
        for (k, dir) in [(0.0, -1.0), (1.0, 1.0)] {
            self.curves.push(Bell { start: self.pos + 25.0 + 150.0 * k, len: 200.0, amp: dir * MAX_CURVE, gain: 0.0 });
        }
    }

    /// Bosse (`amount` > 0) ou creux (< 0) à venir ; 1 = la plus haute.
    pub fn bump(&mut self, amount: f32) {
        self.bumps.push(Bell { start: self.pos + 30.0, len: 170.0, amp: amount * 12.0, gain: 0.0 });
    }

    /// Série de bosses serrées.
    pub fn rollercoaster(&mut self) {
        for k in 0..4 {
            self.bumps.push(Bell { start: self.pos + 30.0 + 70.0 * k as f32, len: 90.0, amp: 6.0, gain: 0.0 });
        }
    }

    /// Éclair immédiat. `rand` tire dans 0..1.
    pub fn strike(&mut self, rand: &mut impl FnMut() -> f32) {
        self.flash = 1.0;
        self.bolt = [rand() * 2.0 - 1.0, rand() * 100.0];
    }

    /// Avance d'une image. `speed` est la vitesse du générateur, `density` la part de décor,
    /// `beat` un temps du rythme (déclenche des éclairs quand l'orage est réglé).
    pub fn step(
        &mut self,
        cfg: &RoadSettings,
        dt: f32,
        speed: f32,
        density: f32,
        beat: bool,
        rand: &mut impl FnMut() -> f32,
    ) -> RoadDraw {
        let advance = CRUISE * speed.abs() * dt;
        for b in self.curves.iter_mut().chain(&mut self.bumps) {
            b.gain = (b.gain + dt / GROW_SECS).min(1.0);
        }
        self.heading = (self.heading + self.curve(self.pos + advance * 0.5) * advance).rem_euclid(1024.0);
        self.pos += advance;
        if self.pos >= PERIOD {
            self.pos -= PERIOD;
            for b in self.curves.iter_mut().chain(&mut self.bumps) {
                b.start -= PERIOD;
            }
            self.next_curve -= PERIOD;
            self.next_bump -= PERIOD;
        }
        let pos = self.pos;
        self.curves.retain(|b| b.start + b.len > pos - 5.0);
        self.bumps.retain(|b| b.start + b.len > pos - 5.0);

        // Tracé au hasard : toujours un peu plus loin que ce qu'on voit.
        let mut between = |a: f32, b: f32| a + (b - a) * rand();
        if cfg.curves > 0.01 {
            self.next_curve = self.next_curve.max(pos + 60.0);
            while self.next_curve < pos + RANGE + 40.0 {
                let len = between(140.0, 340.0);
                let side = if between(0.0, 1.0) < 0.5 { -1.0 } else { 1.0 };
                let amp = side * between(0.3, 1.0) * MAX_CURVE * cfg.curves.min(1.0);
                self.curves.push(Bell { start: self.next_curve, len, amp, gain: 0.0 });
                self.next_curve += len * between(0.55, 1.2);
            }
        }
        if cfg.bumps > 0.01 {
            self.next_bump = self.next_bump.max(pos + 60.0);
            while self.next_bump < pos + RANGE + 40.0 {
                let len = between(90.0, 280.0);
                let side = if between(0.0, 1.0) < 0.4 { -1.0 } else { 1.0 };
                // Jamais plus raide qu'une vraie côte.
                let amp = side * between(0.25, 1.0) * MAX_BUMP.min(len * 0.07) * cfg.bumps.min(1.0);
                self.bumps.push(Bell { start: self.next_bump, len, amp, gain: 0.0 });
                self.next_bump += len * between(0.5, 1.1);
            }
        }

        // Météo : passage en douceur d'un réglage à l'autre.
        let target = [cfg.clouds, cfg.rain, cfg.daylight, cfg.fog, cfg.hills].map(|v| v.clamp(0.0, 1.0));
        let k = (dt / WEATHER_SECS).min(1.0);
        let weather = match self.weather {
            Some(now) => std::array::from_fn(|i| now[i] + (target[i] - now[i]) * k),
            None => target,
        };
        self.weather = Some(weather);

        // Éclairs : au hasard selon l'orage, et sur les temps du rythme.
        self.flash *= (-dt * 4.5).exp();
        if cfg.storm > 0.02 {
            self.next_strike -= dt;
            if self.next_strike <= 0.0 || (beat && rand() < 0.3 + 0.5 * cfg.storm) {
                self.strike(rand);
                self.next_strike = (9.0 - 7.8 * cfg.storm.min(1.0)) * (0.4 + 1.2 * rand());
            }
        } else {
            self.next_strike = 0.5;
        }

        // La voiture braque vers le virage qui arrive, la caméra suit la pente.
        let follow = (dt * 4.0).min(1.0);
        self.steer += ((self.curve(pos + 10.0) / MAX_CURVE).clamp(-1.0, 1.0) - self.steer) * follow;
        let slope: f32 = self.bumps.iter().map(|b| b.slope(pos + 4.0)).sum();
        self.pitch += (slope * 0.5 - self.pitch) * follow;

        // Route devant la caméra : la courbure est intégrée deux fois (cap, puis décalage).
        let mut pts = [[0.0; 2]; SAMPLES];
        let base = self.height(pos);
        let (mut x, mut dx) = (0.0f32, 0.0f32);
        const SUB: usize = 3;
        let h = STEP / SUB as f32;
        for (i, p) in pts.iter_mut().enumerate() {
            let s = pos + i as f32 * STEP;
            *p = [x, self.height(s) - base];
            for k in 0..SUB {
                dx += self.curve(s + (k as f32 + 0.5) * h) * h;
                x += dx * h;
            }
        }
        let top = pts.iter().map(|p| p[1]).fold(0.0f32, f32::max);

        RoadDraw {
            pts,
            pos,
            heading: self.heading,
            steer: self.steer,
            pitch: self.pitch,
            clouds: weather[0],
            rain: weather[1],
            daylight: weather[2],
            fog: weather[3],
            hills: weather[4],
            psyche: cfg.psyche.clamp(0.0, 1.0),
            flash: self.flash,
            bolt_x: self.bolt[0],
            bolt_seed: self.bolt[1],
            car: cfg.car,
            car_hue: cfg.car_hue,
            decor: cfg.decor,
            density: density.clamp(0.0, 1.0),
            ceiling: top + weather[4] * HILL_HEIGHT + 2.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> RoadSettings {
        RoadSettings { curves: 0.0, bumps: 0.0, ..Default::default() }
    }

    fn run(track: &mut Track, cfg: &RoadSettings, secs: f32) -> RoadDraw {
        let mut n = 0u32;
        let mut rand = move || {
            n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (n >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut out = RoadDraw::default();
        for _ in 0..(secs * 60.0) as usize {
            out = track.step(cfg, 1.0 / 60.0, 1.0, 0.5, false, &mut rand);
        }
        out
    }

    #[test]
    fn without_bends_the_road_is_straight_and_flat() {
        let d = run(&mut Track::default(), &flat(), 3.0);
        assert!(d.pts.iter().all(|p| p[0] == 0.0 && p[1] == 0.0));
    }

    #[test]
    fn a_requested_turn_bends_the_road_to_that_side() {
        let mut track = Track::default();
        track.turn(1.0);
        let d = run(&mut track, &flat(), 1.5);
        assert_eq!(d.pts[0], [0.0, 0.0]);
        assert!(d.pts[SAMPLES - 1][0] > 20.0, "{:?}", d.pts[SAMPLES - 1]);
        let mut track = Track::default();
        track.turn(-1.0);
        assert!(run(&mut track, &flat(), 1.5).pts[SAMPLES - 1][0] < -20.0);
    }

    #[test]
    fn a_bump_rises_then_comes_back_down() {
        let mut track = Track::default();
        track.bump(1.0);
        let d = run(&mut track, &flat(), 1.0);
        let top = d.pts.iter().map(|p| p[1]).fold(0.0f32, f32::max);
        assert!(top > 8.0 && d.pts[SAMPLES - 1][1].abs() < 0.5, "top {top}");
        assert!(d.ceiling >= top);
    }

    #[test]
    fn the_road_is_the_same_when_the_distance_wraps() {
        let cfg = RoadSettings::default();
        // Deux routes identiques ; la seconde est déplacée juste avant le retour à zéro.
        let (mut plain, mut wrapped) = (Track::default(), Track::default());
        run(&mut plain, &cfg, 5.0);
        run(&mut wrapped, &cfg, 5.0);
        let shift = PERIOD - 0.3 - wrapped.pos;
        wrapped.pos += shift;
        wrapped.next_curve += shift;
        wrapped.next_bump += shift;
        for b in wrapped.curves.iter_mut().chain(&mut wrapped.bumps) {
            b.start += shift;
        }
        let a = run(&mut plain, &cfg, 0.5);
        let b = run(&mut wrapped, &cfg, 0.5);
        assert!(b.pos < 100.0 && a.pos > 100.0);
        for (a, b) in a.pts.iter().zip(&b.pts) {
            assert!((a[0] - b[0]).abs() < 0.3 && (a[1] - b[1]).abs() < 0.3, "{a:?} {b:?}");
        }
    }

    #[test]
    fn weather_presets_are_recognised() {
        let mut cfg = RoadSettings::default();
        assert_eq!(cfg.weather(), Some(Weather::Sunny));
        cfg.set_weather(Weather::Storm);
        assert_eq!(cfg.weather(), Some(Weather::Storm));
        cfg.rain = 0.33;
        assert_eq!(cfg.weather(), None);
    }
}
