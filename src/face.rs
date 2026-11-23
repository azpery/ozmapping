//! Visuel « Visage qui parle » : un visage en 3D, de face, fait pour être projeté sur une tête
//! de mannequin, une sphère ou une boule de cristal (illusion de la tête parlante).
//!
//! Ici, on tient les réglages du personnage (âge, genre, corpulence…) et ce qui le rend vivant :
//! la parole (syllabes tirées au hasard, ou bouche qui suit le son), les clignements, le regard
//! et les mouvements de tête. Le dessin lui-même est fait par `face_mesh` (maillage et textures).

use serde::{Deserialize, Serialize};

/// Rendu du visage. L'ordre (numéro envoyé au shader) ne doit pas changer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Look {
    Natural,
    Ghost,
    Statue,
    Hologram,
    #[default]
    Projection,
}

impl Look {
    pub const ALL: [Look; 5] = [Look::Projection, Look::Natural, Look::Ghost, Look::Statue, Look::Hologram];

    pub fn label(self) -> &'static str {
        match self {
            Look::Natural => "Naturel",
            Look::Ghost => "Spectre",
            Look::Statue => "Statue",
            Look::Hologram => "Hologramme",
            Look::Projection => "Projection",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Look::Natural => "Peau et yeux en couleurs réelles",
            Look::Ghost => "Apparition lumineuse aux couleurs de la palette, avec un halo (boule de cristal)",
            Look::Statue => "Tête de pierre qui s'anime, yeux sans regard",
            Look::Hologram => "Projection à lignes de balayage, aux couleurs de la palette",
            Look::Projection => {
                "Visage très pâle, blanc bleuté et contrasté, comme filmé pour être projeté sur une tête blanche \
                 (illusion de la voyante dans sa boule de cristal)"
            }
        }
    }
}

/// Réglages du visage d'un calque (enregistrés avec son générateur).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FaceSettings {
    /// Âge apparent, en années (5..95).
    pub age: f32,
    /// 0 = traits féminins, 1 = traits masculins.
    pub gender: f32,
    /// Corpulence : joues pleines, double menton.
    pub weight: f32,
    /// Teint, de clair (0) à foncé (1).
    pub skin: f32,
    /// Couleur des yeux (0 brun, puis vert, bleu, gris) et des sourcils (0 blond, puis roux,
    /// châtain, noir). Les sourcils blanchissent tout seuls avec l'âge.
    pub eyes: f32,
    pub hair_color: f32,
    /// Maquillage : yeux charbonneux, lèvres sombres, sourcils marqués.
    pub makeup: f32,
    pub look: Look,
    /// Visage seul, découpé en ovale : c'est ce qu'il faut pour le projeter sur une tête.
    /// Sinon, la tête entière (crâne nu, oreilles, haut du cou).
    pub only: bool,
    /// Halo lumineux autour de la tête.
    pub aura: f32,
    /// Expression : sourire (-1 triste, 1 souriant), sourcils (-1 froncés, 1 levés).
    pub smile: f32,
    pub brows: f32,
    /// Bavardage (0 = muet, 1 = ne s'arrête jamais) et ampleur de l'articulation.
    pub talk: f32,
    pub articulation: f32,
    /// La bouche suit le son (source de l'onglet Rythme) au lieu de parler toute seule.
    pub follow_sound: bool,
    /// Ampleur des mouvements de tête et du regard.
    pub motion: f32,
    pub gaze: f32,
    /// Jeu du visage pendant qu'il parle : sourires, sourcils qui se lèvent, yeux qui s'écarquillent.
    pub expressive: f32,
    /// Photo plaquée sur le visage (voir `LayerConfig::face_photo`) : présence (0..1), puis calage
    /// du milieu des yeux dans la photo (décalage depuis son centre, en fractions de sa taille),
    /// taille du visage dans la photo, rotation en degrés, étirement vertical.
    pub photo_mix: f32,
    pub photo_x: f32,
    pub photo_y: f32,
    pub photo_zoom: f32,
    pub photo_rot: f32,
    pub photo_stretch: f32,
    /// Les yeux sont ceux de la photo (fixes, mais fidèles) plutôt que ceux du modèle (qui bougent).
    pub photo_eyes: bool,
}

/// Photo d'un visage à plaquer sur la tête : pixels RGBA, et teinte moyenne de la peau
/// (pour accorder le reste de la tête). `key` identifie l'image.
#[derive(Clone, Debug)]
pub struct FacePhoto {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub data: std::sync::Arc<Vec<u8>>,
    pub tone: [f32; 3],
}

impl FacePhoto {
    /// Charge la photo (réduite à 1400 px de côté au plus).
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        use std::hash::{Hash, Hasher};
        let image = crate::cutout::load_image(path)?;
        let image = if image.width().max(image.height()) > 1400 { image.thumbnail(1400, 1400) } else { image }.to_rgba8();
        let (width, height) = image.dimensions();
        // Teinte de la peau : moyenne du centre de l'image, là où se trouve le visage.
        let (mut sum, mut n) = ([0.0f64; 3], 0.0f64);
        for y in (height * 2 / 5..height * 3 / 5).step_by(3) {
            for x in (width * 2 / 5..width * 3 / 5).step_by(3) {
                let p = image.get_pixel(x, y);
                for k in 0..3 {
                    sum[k] += p[k] as f64 / 255.0;
                }
                n += 1.0;
            }
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hasher);
        Ok(Self {
            key: hasher.finish() | 1,
            width,
            height,
            data: std::sync::Arc::new(image.into_raw()),
            tone: sum.map(|s| (s / n.max(1.0)) as f32),
        })
    }
}

impl Default for FaceSettings {
    fn default() -> Self {
        Self {
            age: 30.0,
            gender: 0.0,
            weight: 0.3,
            skin: 0.0,
            eyes: 0.6,
            hair_color: 0.8,
            makeup: 0.8,
            look: Look::Projection,
            only: true,
            aura: 0.0,
            smile: 0.1,
            brows: 0.0,
            talk: 0.7,
            articulation: 0.8,
            follow_sound: false,
            motion: 0.5,
            gaze: 0.5,
            expressive: 0.7,
            photo_mix: 1.0,
            photo_x: 0.0,
            photo_y: 0.1,
            photo_zoom: 0.4,
            photo_rot: 0.0,
            photo_stretch: 1.0,
            photo_eyes: true,
        }
    }
}

/// Personnages tout prêts (ne changent ni la parole, ni les mouvements, ni le cadrage).
pub fn characters() -> Vec<(&'static str, FaceSettings)> {
    let d = FaceSettings::default();
    // Les personnages « au naturel » : couleurs réelles, sans maquillage.
    let n = FaceSettings { look: Look::Natural, makeup: 0.0, eyes: 0.0, hair_color: 0.7, ..d };
    vec![
        ("Voyante", FaceSettings { smile: 0.2, brows: 0.2, ..d }),
        ("Vieille voyante", FaceSettings { age: 70.0, makeup: 0.6, brows: 0.3, ..d }),
        ("Jeune femme", FaceSettings { age: 24.0, weight: 0.25, eyes: 0.5, hair_color: 0.6, makeup: 0.3, ..n }),
        ("Homme", FaceSettings { age: 40.0, gender: 1.0, hair_color: 0.8, ..n }),
        ("Vieil homme", FaceSettings { age: 85.0, gender: 1.0, eyes: 0.7, weight: 0.2, brows: -0.2, ..n }),
        ("Grand-mère", FaceSettings { age: 80.0, weight: 0.55, smile: 0.5, ..n }),
        ("Enfant", FaceSettings { age: 7.0, gender: 0.5, eyes: 0.7, hair_color: 0.2, smile: 0.4, ..n }),
        ("Fantôme", FaceSettings { age: 60.0, gender: 0.8, weight: 0.0, look: Look::Ghost, aura: 1.0, smile: -0.4, brows: -0.5, ..n }),
        ("Statue", FaceSettings { age: 45.0, gender: 0.9, look: Look::Statue, smile: 0.0, ..n }),
        ("Hologramme", FaceSettings { gender: 0.2, look: Look::Hologram, aura: 0.5, ..n }),
    ]
}

impl FaceSettings {
    /// Reprend le personnage `c` en gardant la parole et les mouvements réglés ici.
    pub fn with_character(&self, c: &FaceSettings) -> Self {
        Self {
            talk: self.talk,
            articulation: self.articulation,
            follow_sound: self.follow_sound,
            motion: self.motion,
            gaze: self.gaze,
            expressive: self.expressive,
            only: self.only,
            photo_mix: self.photo_mix,
            photo_x: self.photo_x,
            photo_y: self.photo_y,
            photo_zoom: self.photo_zoom,
            photo_rot: self.photo_rot,
            photo_stretch: self.photo_stretch,
            photo_eyes: self.photo_eyes,
            ..*c
        }
    }

    pub fn same_character(&self, c: &FaceSettings) -> bool {
        *self == self.with_character(c)
    }

    /// Le personnage seul (âge, genre, corpulence), sans expression : de quoi situer ses yeux et sa bouche.
    pub fn identity(&self) -> FaceDraw {
        FaceDraw {
            old: ((self.age - 30.0) / 60.0).clamp(0.0, 1.0),
            kid: ((20.0 - self.age) / 14.0).clamp(0.0, 1.0),
            masc: self.gender.clamp(0.0, 1.0),
            weight: self.weight.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// Cale la photo d'après trois points montrés dessus (coordonnées 0..1, y vers le bas) : les deux
    /// yeux, dans n'importe quel ordre, et le milieu de la bouche. `marks` : repères du personnage
    /// (demi-écart des yeux, hauteur des yeux, hauteur de la bouche), `aspect` : largeur / hauteur
    /// de la photo.
    pub fn fit_photo(&mut self, marks: [f32; 3], aspect: f32, eye_a: [f32; 2], eye_b: [f32; 2], mouth: [f32; 2]) {
        let (l, r) = if eye_a[0] <= eye_b[0] { (eye_a, eye_b) } else { (eye_b, eye_a) };
        let center = [(l[0] + r[0]) * 0.5, (l[1] + r[1]) * 0.5];
        // En hauteurs de photo, pour comparer les distances dans les deux sens.
        let (du, dv) = ((r[0] - l[0]) * aspect, r[1] - l[1]);
        let span = du.hypot(dv);
        if span < 1e-4 || marks[0] < 1e-4 {
            return;
        }
        let zoom = span / (2.0 * marks[0]);
        self.photo_x = center[0] - 0.5;
        self.photo_y = 0.5 - center[1];
        self.photo_zoom = zoom;
        self.photo_rot = (-dv).atan2(du).to_degrees();
        // Distance des yeux à la bouche, perpendiculairement à la ligne des yeux.
        let (mu, mv) = ((mouth[0] - center[0]) * aspect, mouth[1] - center[1]);
        let drop = (mv * du - mu * dv) / span;
        self.photo_stretch = (drop / ((marks[1] - marks[2]).max(1e-3) * zoom)).clamp(0.5, 1.8);
    }
}

/// Ce que le shader reçoit pour une image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FaceDraw {
    /// Vieillesse (0 à 30 ans, 1 à 90), enfance (1 à 6 ans, 0 à 20), masculinité, corpulence.
    pub old: f32,
    pub kid: f32,
    pub masc: f32,
    pub weight: f32,
    /// Bouche : ouverture (0..1), forme (-1 arrondie « o », 1 étirée « i »), sourire, sourcils.
    pub open: f32,
    pub wide: f32,
    pub smile: f32,
    pub brows: f32,
    /// Paupières (1 = fermées) et direction du regard (-1..1).
    pub blink: f32,
    pub gaze: [f32; 2],
    pub look: Look,
    /// Tête : lacet, tangage, roulis, en radians.
    pub turn: [f32; 3],
    pub skin: f32,
    pub eyes: f32,
    pub hair_color: f32,
    pub aura: f32,
    /// Yeux écarquillés (0..1) et maquillage (0..1).
    pub wide_eyes: f32,
    pub makeup: f32,
    /// Photo plaquée : son identifiant (0 = aucune, rempli par l'application), sa présence, son
    /// calage (x, y, taille, rotation en radians, étirement).
    pub photo: u64,
    pub photo_mix: f32,
    pub photo_fit: [f32; 5],
    pub photo_eyes: bool,
    pub only: bool,
}

/// Formes de bouche des voyelles et des consonnes : (ouverture, étirement).
const VOWELS: [(f32, f32); 6] = [(1.0, 0.2), (0.6, 0.6), (0.35, 1.0), (0.7, -0.8), (0.4, -1.0), (0.8, 0.0)];
const CONSONANTS: [(f32, f32); 6] = [(0.0, 0.0), (0.0, 0.1), (0.1, 0.3), (0.15, 0.6), (0.25, 0.1), (0.12, -0.8)];

/// Durée d'un clignement, en secondes.
const BLINK_SECS: f32 = 0.17;

/// Vie du visage d'un calque : parole, clignements, regard, mouvements de tête.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speaker {
    /// Phonème en cours : temps restant, forme visée, et s'il s'agit d'une voyelle.
    phoneme_left: f32,
    target: (f32, f32),
    vowel: bool,
    /// Phonèmes restants dans la phrase, puis silence restant avant la suivante.
    phrase_left: u32,
    pause_left: f32,
    /// Bouche adoucie.
    open: f32,
    wide: f32,
    /// Son : niveau adouci et plus fort niveau récent (pour s'adapter au volume).
    level: f32,
    peak: f32,
    /// Clignement : temps avant le prochain, avancement de celui en cours (0..1).
    blink_in: f32,
    blink: Option<f32>,
    /// Regard : direction adoucie, direction visée, temps avant le prochain coup d'œil.
    gaze: [f32; 2],
    gaze_target: [f32; 2],
    gaze_in: f32,
    /// Sourcils qui se lèvent en début de phrase, puis retombent.
    brow_kick: f32,
    /// Mimique en cours (sourire, sourcils, yeux écarquillés), celle visée, temps avant la suivante.
    emote: [f32; 3],
    emote_target: [f32; 3],
    emote_in: f32,
    /// Phases des balancements de la tête, en radians.
    sway: [f32; 4],
}

impl Default for Speaker {
    fn default() -> Self {
        Self {
            phoneme_left: 0.0,
            target: (0.0, 0.0),
            vowel: false,
            phrase_left: 0,
            pause_left: 0.5,
            open: 0.0,
            wide: 0.0,
            level: 0.0,
            peak: 0.1,
            blink_in: 2.0,
            blink: None,
            gaze: [0.0; 2],
            gaze_target: [0.0; 2],
            gaze_in: 1.0,
            brow_kick: 0.0,
            emote: [0.0; 3],
            emote_target: [0.0; 3],
            emote_in: 1.0,
            sway: [0.0, 1.3, 2.0, 0.7],
        }
    }
}

impl Speaker {
    /// Avance d'une image. `sound` : intensité du son (0..1) si le rythme est actif.
    pub fn step(&mut self, s: &FaceSettings, dt: f32, speed: f32, sound: Option<f32>, rand: &mut impl FnMut() -> f32) -> FaceDraw {
        let dt = dt.clamp(0.0, 0.1) * speed.abs().min(4.0);
        let approach = |rate: f32| 1.0 - (-dt * rate).exp();

        // Phonèmes : la forme de la bouche change sans arrêt, qu'elle parle seule ou suive le son.
        self.phoneme_left -= dt;
        if self.phrase_left == 0 {
            // Entre deux phrases : bouche au repos, puis une nouvelle phrase commence.
            self.target = (0.0, 0.0);
            self.pause_left -= dt;
            if self.pause_left <= 0.0 && (s.talk > 0.01 || s.follow_sound) {
                self.phrase_left = 6 + (rand() * 26.0) as u32;
                self.phoneme_left = 0.0;
                self.vowel = false;
                if rand() < 0.5 {
                    self.brow_kick = 0.3 + 0.5 * rand();
                }
            }
        }
        if self.phrase_left > 0 && self.phoneme_left <= 0.0 {
            self.phrase_left -= 1;
            self.vowel = !self.vowel;
            let set = if self.vowel { &VOWELS } else { &CONSONANTS };
            self.target = set[((rand() * set.len() as f32) as usize).min(set.len() - 1)];
            self.phoneme_left = if self.vowel { 0.09 + 0.10 * rand() } else { 0.05 + 0.06 * rand() };
            if self.phrase_left == 0 {
                // Fin de phrase : silence d'autant plus court que le visage est bavard.
                self.pause_left = self.phoneme_left + (0.25 + 2.8 * (1.0 - s.talk)) * (0.5 + rand());
            }
        }
        let speaking = self.phrase_left > 0;

        // Ouverture visée : la parole inventée, ou le son (niveau rapporté au plus fort entendu).
        let mut open_target = if speaking && s.talk > 0.01 { self.target.0 } else { 0.0 };
        if let Some(level) = sound.filter(|_| s.follow_sound) {
            let level = level.clamp(0.0, 1.0);
            self.level += (level - self.level) * approach(if level > self.level { 40.0 } else { 12.0 });
            self.peak = (self.peak * (1.0 - 0.1 * dt)).max(self.level).max(0.05);
            let loud = ((self.level / self.peak - 0.15) / 0.85).clamp(0.0, 1.0);
            // La syllabe en cours fait articuler, même sur un son tenu.
            open_target = loud * (0.45 + 0.55 * self.target.0.max(0.2));
        }
        open_target *= s.articulation;
        self.open += (open_target - self.open) * approach(26.0);
        self.wide += (self.target.1 * s.articulation.min(1.0) - self.wide) * approach(18.0);

        // Clignements.
        self.blink_in -= dt;
        if self.blink_in <= 0.0 && self.blink.is_none() {
            self.blink = Some(0.0);
            // Parfois deux clignements de suite.
            self.blink_in = if rand() < 0.15 { 0.3 } else { 1.8 + 4.5 * rand() };
        }
        let mut lids = 0.0;
        if let Some(b) = self.blink {
            let b = b + dt / BLINK_SECS;
            // Fermeture rapide, ouverture plus lente.
            lids = if b < 0.35 { b / 0.35 } else { 1.0 - (b - 0.35) / 0.65 };
            self.blink = (b < 1.0).then_some(b);
        }

        // Regard : coups d'œil brefs, retour fréquent vers le public.
        self.gaze_in -= dt;
        if self.gaze_in <= 0.0 {
            self.gaze_in = 0.6 + 2.6 * rand();
            self.gaze_target = if rand() < 0.45 { [0.0; 2] } else { [(rand() * 2.0 - 1.0) * s.gaze, (rand() * 2.0 - 1.0) * 0.6 * s.gaze] };
        }
        for k in 0..2 {
            self.gaze[k] += (self.gaze_target[k] - self.gaze[k]) * approach(14.0);
        }

        // Tête : balancements lents, et la tête suit un peu le regard et la parole.
        for (phase, rate) in self.sway.iter_mut().zip([0.43, 0.97, 0.61, 0.37]) {
            *phase = (*phase + dt * rate) % std::f32::consts::TAU;
        }
        let m = s.motion;
        let turn = [
            m * (0.16 * self.sway[0].sin() + 0.07 * self.sway[1].sin()) + 0.12 * self.gaze[0] * m,
            m * 0.07 * self.sway[2].sin() - 0.05 * self.gaze[1] * m + 0.04 * self.open * m,
            m * 0.06 * self.sway[3].sin(),
        ];
        self.brow_kick *= 1.0 - approach(1.6);
        // Jeu du visage : une nouvelle mimique de temps en temps (sourire, sourcils, yeux), surtout en parlant.
        self.emote_in -= dt;
        if self.emote_in <= 0.0 {
            self.emote_in = 0.7 + 2.3 * rand();
            let e = s.expressive;
            self.emote_target = if !speaking && rand() < 0.6 {
                [0.0; 3]
            } else {
                [(rand() * 1.1 - 0.25) * e, (rand() * 1.3 - 0.35) * e, (rand() * 1.2 - 0.4) * e]
            };
        }
        for k in 0..3 {
            self.emote[k] += (self.emote_target[k] - self.emote[k]) * approach(3.5);
        }

        FaceDraw {
            photo_eyes: s.photo_eyes,
            open: self.open.clamp(0.0, 1.0),
            wide: self.wide.clamp(-1.0, 1.0),
            smile: (s.smile + self.emote[0]).clamp(-1.0, 1.0),
            brows: (s.brows + self.brow_kick + self.emote[1]).clamp(-1.0, 1.0),
            wide_eyes: self.emote[2].clamp(0.0, 1.0),
            makeup: s.makeup.clamp(0.0, 1.0),
            photo: 0,
            photo_mix: s.photo_mix.clamp(0.0, 1.0),
            photo_fit: [s.photo_x, s.photo_y, s.photo_zoom.max(0.02), s.photo_rot.to_radians(), s.photo_stretch.max(0.1)],
            blink: lids.clamp(0.0, 1.0),
            gaze: self.gaze,
            look: s.look,
            turn,
            skin: s.skin.clamp(0.0, 1.0),
            eyes: s.eyes.clamp(0.0, 1.0),
            hair_color: s.hair_color.clamp(0.0, 1.0),
            aura: s.aura.clamp(0.0, 1.0),
            only: s.only,
            ..s.identity()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng() -> impl FnMut() -> f32 {
        let mut n = 0x9E37_79B9_7F4A_7C15u64;
        move || {
            n ^= n << 13;
            n ^= n >> 7;
            n ^= n << 17;
            (n >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    /// Fait vivre le visage `secs` secondes et renvoie (plus grande ouverture, part du temps bouche ouverte, clignements).
    fn run(s: &FaceSettings, secs: f32, sound: impl Fn(f32) -> Option<f32>) -> (f32, f32, u32) {
        let mut sp = Speaker::default();
        let mut rand = rng();
        let (mut max_open, mut opened, mut blinks, mut closed) = (0.0f32, 0u32, 0u32, false);
        let frames = (secs * 60.0) as u32;
        for f in 0..frames {
            let d = sp.step(s, 1.0 / 60.0, 1.0, sound(f as f32 / 60.0), &mut rand);
            assert!((0.0..=1.0).contains(&d.open) && (0.0..=1.0).contains(&d.blink), "{d:?}");
            max_open = max_open.max(d.open);
            opened += (d.open > 0.15) as u32;
            if d.blink > 0.8 && !closed {
                blinks += 1;
            }
            closed = d.blink > 0.8;
        }
        (max_open, opened as f32 / frames as f32, blinks)
    }

    #[test]
    fn talks_with_pauses_and_blinks() {
        let (max_open, share, blinks) = run(&FaceSettings::default(), 60.0, |_| None);
        assert!(max_open > 0.5, "{max_open}");
        assert!((0.1..0.8).contains(&share), "{share}");
        assert!((5..40).contains(&blinks), "{blinks}");
    }

    #[test]
    fn silent_face_keeps_its_mouth_shut() {
        let (max_open, ..) = run(&FaceSettings { talk: 0.0, ..Default::default() }, 30.0, |_| None);
        assert!(max_open < 0.01, "{max_open}");
    }

    #[test]
    fn mouth_follows_the_sound() {
        let s = FaceSettings { follow_sound: true, ..Default::default() };
        // Son coupé : bouche fermée, même si le visage est bavard.
        let (quiet, ..) = run(&s, 20.0, |_| Some(0.0));
        assert!(quiet < 0.05, "{quiet}");
        // Son faible mais présent : la bouche s'ouvre quand même (adaptation au volume).
        let (loud, share, _) = run(&s, 20.0, |t| Some(if (t % 2.0) < 1.0 { 0.12 } else { 0.0 }));
        assert!(loud > 0.3, "{loud}");
        assert!((0.15..0.75).contains(&share), "{share}");
    }

    #[test]
    fn three_clicks_place_the_photo() {
        // Repères du personnage : yeux à ±0,15 et à la hauteur 0,05, bouche à -0,3.
        let marks = [0.15, 0.05, -0.3];
        let aspect = 1.5;
        let mut s = FaceSettings::default();
        // Les yeux sont donnés dans le désordre : celui de droite d'abord.
        s.fit_photo(marks, aspect, [0.6, 0.4], [0.4, 0.4], [0.5, 0.75]);
        // Où tombe un point du visage dans la photo (même calcul que le shader).
        let place = |s: &FaceSettings, p: [f32; 2]| {
            let (sin, cos) = s.photo_rot.to_radians().sin_cos();
            let d = [p[0], p[1] - marks[1]];
            let d = [(d[0] * cos - d[1] * sin) * s.photo_zoom, (d[0] * sin + d[1] * cos) * s.photo_zoom];
            [0.5 + s.photo_x + d[0] / aspect, 0.5 - s.photo_y - d[1] * s.photo_stretch]
        };
        for (model, photo) in [([-0.15, 0.05], [0.4, 0.4]), ([0.15, 0.05], [0.6, 0.4]), ([0.0, -0.3], [0.5, 0.75])] {
            let got = place(&s, model);
            assert!((got[0] - photo[0]).abs() < 1e-4 && (got[1] - photo[1]).abs() < 1e-4, "{model:?} : {got:?}");
        }
        // Photo penchée : la rotation suit la ligne des yeux.
        s.fit_photo(marks, aspect, [0.4, 0.45], [0.6, 0.35], [0.55, 0.75]);
        assert!(s.photo_rot > 10.0, "{}", s.photo_rot);
        let eye = place(&s, [0.15, 0.05]);
        assert!((eye[0] - 0.6).abs() < 1e-3, "{eye:?}");
    }

    #[test]
    fn characters_keep_speech_settings() {
        let mine = FaceSettings { talk: 0.2, motion: 0.9, ..Default::default() };
        for (name, c) in characters() {
            let got = mine.with_character(&c);
            assert_eq!((got.talk, got.motion), (0.2, 0.9), "{name}");
            assert!(got.same_character(&c), "{name}");
        }
    }
}
