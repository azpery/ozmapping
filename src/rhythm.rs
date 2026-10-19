//! Rythme : écoute d'une entrée audio (micro, carte son…) ou tempo tapé à la main,
//! et réaction des calques sur les temps.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};

use crate::effects::Effects;

// ------------------------------------------------------------------ analyse audio

/// Mesures du son, normalisées (0–1) par un gain automatique.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levels {
    /// Volume global.
    pub level: f32,
    /// Graves (≲ 150 Hz) : grosse caisse, basse.
    pub bass: f32,
    /// Aigus (≳ 2 kHz) : charleston, cymbales.
    pub high: f32,
    /// Nombre de temps détectés depuis l'ouverture (une hausse = un nouveau temps).
    pub beats: u64,
}

/// Bande de fréquences qui donne les temps et l'intensité.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Band {
    /// Graves : grosse caisse et basse. Le plus net et le plus réactif sur de la musique rythmée.
    #[default]
    Bass,
    /// Tout le spectre.
    Full,
    /// Aigus : charleston, cymbales, voix sifflantes.
    High,
}

impl Band {
    pub const ALL: [Band; 3] = [Band::Bass, Band::Full, Band::High];

    pub fn label(self) -> &'static str {
        match self {
            Band::Bass => "Basses",
            Band::Full => "Tout",
            Band::High => "Aigus",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Band::Bass => "Réagit à la grosse caisse et à la basse : le plus rapide et le plus franc",
            Band::Full => "Réagit à tout le son",
            Band::High => "Réagit aux charlestons, cymbales et sons aigus (temps plus serrés)",
        }
    }

    /// Indice dans les mesures de l'analyseur (tout, graves, aigus).
    fn index(self) -> usize {
        match self {
            Band::Full => 0,
            Band::Bass => 1,
            Band::High => 2,
        }
    }

    /// Écart minimal entre deux temps détectés.
    fn refractory(self) -> f32 {
        match self {
            Band::Bass => 0.2,
            Band::Full => 0.15,
            Band::High => 0.09,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Band::Full,
            2 => Band::High,
            _ => Band::Bass,
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Band::Bass => 0,
            Band::Full => 1,
            Band::High => 2,
        }
    }
}

/// Analyse échantillon par échantillon : filtres simples (passe-bas / passe-haut), énergie par
/// tranche de 5 ms et détection des attaques dans la bande choisie.
pub struct Analyzer {
    hop: usize,
    dt: f32,
    a_bass: f32,
    a_mid: f32,
    lp_bass: f32,
    lp_bass2: f32,
    lp_mid: f32,
    n: usize,
    sum: [f32; 3],
    /// Plus fortes valeurs récentes (gain automatique), décroissent lentement.
    peak: [f32; 3],
    band_avg: f32,
    since_beat: f32,
    pub band: Band,
    pub out: Levels,
}

impl Analyzer {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let one_pole = |hz: f32| 1.0 - (-std::f32::consts::TAU * hz / sr).exp();
        let hop = (sr / 200.0) as usize;
        Self {
            hop,
            dt: hop as f32 / sr,
            a_bass: one_pole(150.0),
            a_mid: one_pole(2000.0),
            lp_bass: 0.0,
            lp_bass2: 0.0,
            lp_mid: 0.0,
            n: 0,
            sum: [0.0; 3],
            peak: [0.0; 3],
            band_avg: 0.0,
            since_beat: 1.0,
            band: Band::Bass,
            out: Levels::default(),
        }
    }

    pub fn push(&mut self, x: f32) {
        // Deux passe-bas en cascade : graves mieux isolés des voix et caisses claires.
        self.lp_bass += self.a_bass * (x - self.lp_bass);
        self.lp_bass2 += self.a_bass * (self.lp_bass - self.lp_bass2);
        self.lp_mid += self.a_mid * (x - self.lp_mid);
        let high = x - self.lp_mid;
        self.sum[0] += x * x;
        self.sum[1] += self.lp_bass2 * self.lp_bass2;
        self.sum[2] += high * high;
        self.n += 1;
        if self.n >= self.hop {
            self.frame();
        }
    }

    fn frame(&mut self) {
        let dt = self.dt;
        let rms = self.sum.map(|s| (s / self.n as f32).sqrt());
        self.sum = [0.0; 3];
        self.n = 0;

        // Gain automatique : relatif au maximum des dernières secondes, avec un plancher pour
        // ne pas amplifier le bruit de fond d'une pièce silencieuse.
        const FLOOR: [f32; 3] = [0.02, 0.01, 0.004];
        let decay = (-dt / 6.0).exp();
        let mut norm = [0.0; 3];
        for k in 0..3 {
            self.peak[k] = rms[k].max(self.peak[k] * decay).max(FLOOR[k]);
            norm[k] = (rms[k] / self.peak[k]).clamp(0.0, 1.0);
        }
        // Montée immédiate, retombée en ~150 ms : lisible à l'image.
        let release = (-dt / 0.15).exp();
        let smooth = |old: f32, new: f32| new.max(old * release);
        self.out.level = smooth(self.out.level, norm[0]);
        self.out.bass = smooth(self.out.bass, norm[1]);
        self.out.high = smooth(self.out.high, norm[2]);

        // Temps : la bande choisie dépasse nettement sa moyenne récente.
        self.since_beat += dt;
        let k = self.band.index();
        let e = rms[k];
        if e > self.band_avg * 1.4 && e > FLOOR[k] * 0.8 && self.since_beat > self.band.refractory() {
            self.out.beats += 1;
            self.since_beat = 0.0;
        }
        self.band_avg += (e - self.band_avg) * (dt / 0.4).min(1.0);
    }
}

/// Entrée audio ouverte (le flux s'arrête quand elle est détruite).
pub struct AudioInput {
    _stream: cpal::Stream,
    levels: Arc<Mutex<Levels>>,
    error: Arc<Mutex<Option<String>>>,
    band: Arc<AtomicU8>,
    pub device: String,
}

/// Noms des entrées audio disponibles (micros, cartes son, entrées virtuelles…).
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else { return Vec::new() };
    let mut names: Vec<String> = devices.filter_map(|d| d.description().ok()).map(|d| d.name().to_owned()).collect();
    names.dedup();
    names
}

impl AudioInput {
    /// Ouvre l'entrée nommée (ou celle par défaut).
    pub fn open(name: Option<&str>, band: Band) -> Result<Self> {
        let host = cpal::default_host();
        let device = match name {
            Some(name) => host
                .input_devices()?
                .find(|d| d.description().is_ok_and(|desc| desc.name() == name))
                .ok_or_else(|| anyhow!("entrée audio « {name} » introuvable"))?,
            None => host.default_input_device().context("aucune entrée audio")?,
        };
        let device_name = device.description().map(|d| d.name().to_owned()).unwrap_or_else(|_| "Entrée audio".into());
        let config = device.default_input_config().context("format de l'entrée audio")?;
        let channels = config.channels().max(1) as usize;
        let mut analyzer = Analyzer::new(config.sample_rate() as f32);
        let levels = Arc::new(Mutex::new(Levels::default()));
        let error = Arc::new(Mutex::new(None));
        let band = Arc::new(AtomicU8::new(band.to_u8()));

        let shared = levels.clone();
        let band_in = band.clone();
        let mut feed = move |mono: &mut dyn Iterator<Item = f32>| {
            analyzer.band = Band::from_u8(band_in.load(Ordering::Relaxed));
            for x in mono {
                analyzer.push(x);
            }
            *shared.lock().unwrap() = analyzer.out;
        };
        let err = error.clone();
        let on_error = move |e: cpal::Error| *err.lock().unwrap() = Some(e.to_string());
        let mut stream_config: cpal::StreamConfig = config.clone().into();
        // Petit tampon (~5 ms) pour limiter la latence, si la carte son l'accepte.
        if let cpal::SupportedBufferSize::Range { min, max } = config.buffer_size() {
            stream_config.buffer_size = cpal::BufferSize::Fixed(256.clamp(*min, *max));
        }

        macro_rules! build {
            ($t:ty, $config:expr) => {
                device.build_input_stream::<$t, _, _>(
                    $config,
                    move |data: &[$t], _: &cpal::InputCallbackInfo| {
                        use cpal::Sample as _;
                        // Mixage mono : moyenne des canaux.
                        let mut mono = data.chunks(channels).map(|frame| {
                            frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
                        });
                        feed(&mut mono);
                    },
                    on_error,
                    None,
                )
            };
        }
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build!(f32, stream_config),
            cpal::SampleFormat::I16 => build!(i16, stream_config),
            cpal::SampleFormat::I32 => build!(i32, stream_config),
            cpal::SampleFormat::U16 => build!(u16, stream_config),
            cpal::SampleFormat::U8 => build!(u8, stream_config),
            cpal::SampleFormat::I8 => build!(i8, stream_config),
            cpal::SampleFormat::F64 => build!(f64, stream_config),
            f => return Err(anyhow!("format audio non pris en charge : {f}")),
        }
        .context("ouverture de l'entrée audio")?;
        stream.play().context("démarrage de l'entrée audio")?;
        Ok(Self { _stream: stream, levels, error, band, device: device_name })
    }

    pub fn set_band(&self, band: Band) {
        self.band.store(band.to_u8(), Ordering::Relaxed);
    }

    pub fn levels(&self) -> Levels {
        *self.levels.lock().unwrap()
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

// ------------------------------------------------------------------ tempo tapé

/// Tempo tapé à la main (bouton TAP) : la moyenne des derniers intervalles donne le BPM,
/// le dernier tap cale les temps.
#[derive(Clone, Debug, PartialEq)]
pub struct Tempo {
    pub bpm: f32,
    origin: f64,
    taps: Vec<f64>,
}

impl Default for Tempo {
    fn default() -> Self {
        Self { bpm: 120.0, origin: 0.0, taps: Vec::new() }
    }
}

impl Tempo {
    pub fn tap(&mut self, now: f64) {
        // Une pause de plus de 2 s recommence la mesure.
        if self.taps.last().is_some_and(|&t| now - t > 2.0) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > 8 {
            self.taps.remove(0);
        }
        if self.taps.len() >= 2 {
            let span = self.taps[self.taps.len() - 1] - self.taps[0];
            let bpm = 60.0 * (self.taps.len() - 1) as f64 / span.max(1e-3);
            self.bpm = (bpm as f32).clamp(30.0, 300.0);
        }
        self.origin = now;
    }

    /// Change le BPM sans décaler le temps en cours.
    pub fn set_bpm(&mut self, bpm: f32, now: f64) {
        let beat = self.beat_at(now);
        self.bpm = bpm.clamp(30.0, 300.0);
        self.origin = now - beat * 60.0 / self.bpm as f64;
    }

    /// Position en temps (fractionnaire) à l'instant `now`.
    pub fn beat_at(&self, now: f64) -> f64 {
        (now - self.origin) * self.bpm as f64 / 60.0
    }
}

// ------------------------------------------------------------------ horloge commune

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[default]
    Off,
    /// Entrée audio (`None` : celle par défaut).
    Audio(Option<String>),
    /// Tempo tapé.
    Tempo,
}

/// État du rythme pour l'image en cours.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pulse {
    /// Un temps tombe sur cette image.
    pub beat: bool,
    /// Enveloppe du dernier temps : 1 au temps, retombe vers 0.
    pub env: f32,
    /// Intensité continue du son (le tempo tapé en donne une synthétique).
    pub level: f32,
    pub bass: f32,
    pub high: f32,
}

pub struct Rhythm {
    pub source: Source,
    pub band: Band,
    pub tempo: Tempo,
    pub input: Option<AudioInput>,
    pub error: Option<String>,
    seen_beats: i64,
    last_beat: f64,
    /// Un tap demande un temps immédiat.
    forced_beat: bool,
    pub pulse: Pulse,
}

impl Default for Rhythm {
    fn default() -> Self {
        Self {
            source: Source::Off,
            band: Band::Bass,
            tempo: Tempo::default(),
            input: None,
            error: None,
            seen_beats: i64::MIN,
            last_beat: f64::MIN,
            forced_beat: false,
            pulse: Pulse::default(),
        }
    }
}

impl Rhythm {
    pub fn set_source(&mut self, source: Source) {
        self.input = None;
        self.error = None;
        if let Source::Audio(name) = &source {
            match AudioInput::open(name.as_deref(), self.band) {
                Ok(input) => self.input = Some(input),
                Err(e) => self.error = Some(format!("{e:#}")),
            }
        }
        self.source = source;
        self.seen_beats = i64::MIN;
    }

    pub fn set_band(&mut self, band: Band) {
        self.band = band;
        if let Some(input) = &self.input {
            input.set_band(band);
        }
    }

    /// Tap : cale le tempo et donne un temps tout de suite (le tempo tapé devient la source
    /// si aucune n'est active).
    pub fn tap(&mut self, now: f64) {
        if self.source == Source::Off {
            self.set_source(Source::Tempo);
        }
        self.tempo.tap(now);
        self.forced_beat = true;
    }

    pub fn active(&self) -> bool {
        match self.source {
            Source::Off => false,
            Source::Audio(_) => self.input.is_some(),
            Source::Tempo => true,
        }
    }

    /// Avance d'une image : détecte les temps et met à jour `pulse`.
    pub fn update(&mut self, now: f64) -> Pulse {
        let mut pulse = Pulse::default();
        let beats = match self.source {
            Source::Off => None,
            Source::Audio(_) => self.input.as_ref().map(|input| {
                if let Some(e) = input.error() {
                    self.error = Some(e);
                }
                let l = input.levels();
                (pulse.level, pulse.bass, pulse.high) = (l.level, l.bass, l.high);
                l.beats as i64
            }),
            Source::Tempo => Some(self.tempo.beat_at(now).floor() as i64),
        };
        if let Some(beats) = beats {
            if self.seen_beats != i64::MIN && beats > self.seen_beats {
                pulse.beat = true;
                self.last_beat = now;
            }
            self.seen_beats = beats;
        }
        if std::mem::take(&mut self.forced_beat) && self.active() {
            pulse.beat = true;
            self.last_beat = now;
        }
        if let Source::Audio(_) = self.source {
            // L'intensité suit la bande choisie.
            pulse.level = match self.band {
                Band::Full => pulse.level,
                Band::Bass => pulse.bass,
                Band::High => pulse.high,
            };
        }
        pulse.env = if self.active() { (-(now - self.last_beat) as f32 * 7.0).exp() } else { 0.0 };
        if self.source == Source::Tempo {
            // Pas de son : l'enveloppe du temps tient lieu d'intensité.
            pulse.level = pulse.env;
            pulse.bass = pulse.env;
            pulse.high = (-(now - self.last_beat) as f32 * 14.0).exp();
        }
        self.pulse = pulse;
        pulse
    }
}

// ------------------------------------------------------------------ réaction d'un calque

/// Comment un calque réagit au rythme.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct React {
    pub enabled: bool,
    /// Coup de zoom sur chaque temps (0–1).
    pub zoom: f32,
    /// Flash de lumière sur chaque temps (0–1).
    pub flash: f32,
    /// Saut de teinte à chaque temps (fraction du cercle, 0–0,5).
    pub hue: f32,
    /// Décalage RVB sur les temps et les aigus (0–1).
    pub shake: f32,
    /// Les déformations (ondes, liquide, tourbillon) et la saturation suivent le volume (0–1).
    pub drive: f32,
    /// Négatif bref sur chaque temps (0–1).
    pub invert: f32,
    /// Noir entre les temps : l'image ne s'allume que sur le rythme (0–1).
    pub gate: f32,
    /// Pixels et couleurs qui se cassent sur chaque temps (0–1).
    pub glitch: f32,
    /// Quart de tour (×) à chaque temps, en glissant (0–1 = 0–90°).
    pub spin: f32,
    /// Onde de choc sur chaque temps (0–1).
    pub shock: f32,
    /// Kaléidoscope qui s'ouvre sur chaque temps (0–1).
    pub kaleido: f32,
    /// Image retournée en miroir un temps sur deux.
    pub flip: bool,
    /// Mode AUTO : effet suivant tous les N temps (0 : selon l'intervalle en secondes).
    pub auto_beats: u32,
    /// Mix auto : coupe tous les N temps (0 : selon les durées du mix).
    pub mix_beats: u32,
}

impl Default for React {
    fn default() -> Self {
        Self {
            enabled: false,
            zoom: 0.4,
            flash: 0.3,
            hue: 0.0,
            shake: 0.2,
            drive: 0.5,
            invert: 0.0,
            gate: 0.0,
            glitch: 0.0,
            spin: 0.0,
            shock: 0.0,
            kaleido: 0.0,
            flip: false,
            auto_beats: 0,
            mix_beats: 0,
        }
    }
}

/// État d'un calque entre les temps (valeurs qui s'accumulent d'un temps à l'autre).
#[derive(Clone, Copy, Debug, Default)]
pub struct BeatState {
    pub count: u64,
    hue_target: f32,
    hue: f32,
    spin_target: f32,
    spin: f32,
}

impl BeatState {
    pub fn step(&mut self, react: &React, beat: bool, dt: f32) {
        if beat {
            self.count += 1;
            self.hue_target += react.hue;
            self.spin_target += react.spin * 90.0;
            // Repli pour garder de la précision (même décalage visible).
            if self.hue_target >= 1.0 && self.hue >= 1.0 {
                self.hue_target -= 1.0;
                self.hue -= 1.0;
            }
            if self.spin_target >= 360.0 && self.spin >= 360.0 {
                self.spin_target -= 360.0;
                self.spin -= 360.0;
            }
        }
        let k = (dt * 12.0).min(1.0);
        self.hue += (self.hue_target - self.hue) * k;
        self.spin += (self.spin_target - self.spin) * k;
    }
}

impl React {
    /// Effets modulés par le rythme.
    pub fn apply(&self, fx: &Effects, p: &Pulse, state: &BeatState) -> Effects {
        let mut fx = *fx;
        let env = p.env;
        fx.zoom *= 1.0 + 0.25 * self.zoom * env;
        fx.brightness += 0.4 * self.flash * env;
        fx.rgb_shift += 0.025 * self.shake * env.max(p.high * 0.6);
        let d = self.drive * p.level;
        fx.wave_amp = fx.wave_amp * (1.0 + 2.0 * d) + 0.012 * self.drive * p.level;
        fx.liquid *= 1.0 + 1.5 * d;
        fx.swirl *= 1.0 + d;
        fx.saturation *= 1.0 + 0.6 * d;
        // Lumières de la forme : elles s'allument sur les temps (si elles sont déjà utilisées).
        fx.shape_glow *= 1.0 + 1.5 * self.flash * env + d;
        fx.shape_ripple *= 1.0 + d;
        fx.hue += state.hue;
        fx.rotate += state.spin;

        fx.invert = (fx.invert + self.invert * env).min(1.0);
        // Enveloppe plus longue pour le « gate » : l'image tient un peu après le temps.
        let hold = env.powf(0.35);
        fx.brightness -= self.gate * (1.0 - hold);
        if self.glitch > 0.0 {
            let g = self.glitch * env;
            fx.pixelate = fx.pixelate.max(0.85 * g);
            fx.rgb_shift += 0.04 * g;
            fx.posterize = if g > 0.3 && fx.posterize < 2.0 { 4.0 } else { fx.posterize };
        }
        fx.wave_amp += 0.06 * self.shock * env;
        fx.wave_freq = if self.shock > 0.0 && fx.wave_amp > 0.0 && fx.wave_freq < 2.0 { 4.0 } else { fx.wave_freq };
        if self.kaleido > 0.0 && fx.kaleido < 2.0 && env > 1.0 - self.kaleido {
            fx.kaleido = 6.0;
        }
        if self.flip && state.count % 2 == 1 {
            fx.flip_x = !fx.flip_x;
        }
        fx
    }

    /// Réactions toutes prêtes (garde l'activation et les enchaînements sur les temps).
    pub fn presets() -> Vec<(&'static str, &'static str, React)> {
        let base = React {
            enabled: true,
            zoom: 0.0,
            flash: 0.0,
            hue: 0.0,
            shake: 0.0,
            drive: 0.0,
            ..React::default()
        };
        vec![
            ("Pulsation", "Zoom franc sur chaque temps", React { zoom: 0.7, flash: 0.15, drive: 0.3, ..base }),
            ("Club", "Zoom, flash, secousse et couleurs qui tournent", React { zoom: 0.4, flash: 0.5, shake: 0.4, hue: 0.08, drive: 0.5, ..base }),
            ("Stroboscope", "Image allumée seulement sur les temps", React { gate: 0.9, flash: 0.5, ..base }),
            ("Glitch", "L'image se casse en pixels sur les temps", React { glitch: 0.9, shake: 0.8, ..base }),
            ("Rotation", "Un quart de tour à chaque temps", React { spin: 1.0, zoom: 0.3, ..base }),
            ("Miroir", "Retournement un temps sur deux", React { flip: true, zoom: 0.3, flash: 0.2, ..base }),
            ("Négatif", "Flash négatif sur les temps", React { invert: 0.9, zoom: 0.2, ..base }),
            ("Onde de choc", "Ondulation qui part sur chaque temps", React { shock: 0.9, zoom: 0.3, ..base }),
            ("Kaléido", "Le kaléidoscope s'ouvre sur les temps", React { kaleido: 0.6, zoom: 0.3, ..base }),
            ("Arc-en-ciel", "Saut de teinte et couleurs qui suivent le volume", React { hue: 0.17, drive: 0.7, ..base }),
            ("Douceur", "Respiration légère avec le volume", React { zoom: 0.15, drive: 0.8, ..base }),
        ]
    }

    /// Même réaction (hors activation et enchaînements).
    pub fn same_feel(&self, other: &React) -> bool {
        let strip = |r: &React| React { enabled: true, auto_beats: 0, mix_beats: 0, ..*r };
        strip(self) == strip(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kicks_are_detected_as_beats() {
        // Grosse caisse synthétique (60 Hz, attaque sèche) à 120 BPM sur 4 s, sur un fond de bruit.
        let sr = 48_000.0;
        let mut a = Analyzer::new(sr);
        let mut seed = 1u32;
        for i in 0..(4 * sr as usize) {
            let t = i as f32 / sr;
            let tb = t % 0.5;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 9) as f32 / (1 << 23) as f32 - 0.5;
            let kick = (std::f32::consts::TAU * 60.0 * tb).sin() * (-tb * 18.0).exp();
            a.push(0.8 * kick + 0.02 * noise);
        }
        assert!((7..=9).contains(&a.out.beats), "{} temps", a.out.beats);
    }

    #[test]
    fn silence_has_no_beats_and_low_level() {
        let mut a = Analyzer::new(44_100.0);
        for i in 0..88_200 {
            a.push(0.0005 * (i as f32 * 0.3).sin());
        }
        assert_eq!(a.out.beats, 0);
        assert!(a.out.level < 0.1, "{}", a.out.level);
    }

    #[test]
    fn tapping_sets_the_tempo() {
        let mut t = Tempo::default();
        for k in 0..5 {
            t.tap(10.0 + k as f64 * 0.4); // 150 BPM
        }
        assert!((t.bpm - 150.0).abs() < 0.5, "{}", t.bpm);
        assert!(t.beat_at(11.6).abs() < 1e-6, "le dernier tap est un temps");
        assert!((t.beat_at(12.0) - 1.0).abs() < 1e-6);
        // Une longue pause recommence la mesure.
        t.tap(20.0);
        t.tap(20.5);
        assert!((t.bpm - 120.0).abs() < 0.5, "{}", t.bpm);
    }

    #[test]
    fn tap_gives_an_immediate_beat() {
        let mut r = Rhythm::default();
        let mut beats = 0;
        // Taps rapides à 0,3 s : chaque tap est un temps, sans attendre la fin des taps.
        for k in 0..6 {
            let now = 1.0 + k as f64 * 0.3;
            r.tap(now);
            beats += r.update(now).beat as usize;
            beats += r.update(now + 0.1).beat as usize;
        }
        assert_eq!(beats, 6);
        assert!((r.tempo.bpm - 200.0).abs() < 1.0, "{}", r.tempo.bpm);
    }

    #[test]
    fn hihats_count_on_high_band_only() {
        // Charleston (bruit très aigu, bref) toutes les 0,25 s, pas de graves.
        let sr = 48_000.0;
        let mut seed = 7u32;
        let mut run = |band: Band| {
            let mut a = Analyzer::new(sr);
            a.band = band;
            for i in 0..(2 * sr as usize) {
                let tb = (i as f32 / sr) % 0.25;
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 9) as f32 / (1 << 23) as f32 - 0.5;
                a.push(0.6 * noise * (-tb * 60.0).exp());
            }
            a.out.beats
        };
        let high = run(Band::High);
        assert!((6..=9).contains(&high), "aigus : {high}");
    }

    #[test]
    fn tempo_clock_ticks_on_beats() {
        let mut r = Rhythm::default();
        r.set_source(Source::Tempo);
        r.tempo.tap(0.0);
        r.tempo.tap(0.5); // 120 BPM
        let beats = (0..120).filter(|i| r.update(0.5 + *i as f64 / 60.0).beat).count();
        assert_eq!(beats, 3, "temps à 1 s, 1,5 s et 2 s");
    }
}
