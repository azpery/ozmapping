//! Contrôleur MIDI : Akai MPD218 (16 pads et 6 potentiomètres, en trois banques A, B, C).
//!
//! Chaque pad ou potentiomètre du dessin du contrôleur correspond à un message MIDI (note ou
//! contrôleur) : les valeurs d'usine du MPD218 (preset 1) sont connues, et « Apprendre » les
//! remplace par le message réellement reçu (preset modifié, autre contrôleur). Une commande du
//! logiciel est assignée à chaque pad (déclenchement) ou potentiomètre (valeur continue).

use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::effects::Effects;
use crate::leds::ScenarioKind;

/// Banques du MPD218 (boutons Pad Bank et Ctrl Bank).
pub const BANKS: usize = 3;
pub const PADS: usize = 16;
pub const KNOBS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    Note,
    Cc,
}

/// Message MIDI qui identifie une commande physique : note ou contrôleur, canal (0-15), numéro.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Key {
    pub kind: Kind,
    pub channel: u8,
    pub number: u8,
}

impl Key {
    pub fn label(self) -> String {
        match self.kind {
            Kind::Note => format!("note {} · canal {}", self.number, self.channel + 1),
            Kind::Cc => format!("CC {} · canal {}", self.number, self.channel + 1),
        }
    }
}

/// Message reçu : appui (vélocité), relâchement, ou valeur d'un potentiomètre (0-127).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub key: Key,
    pub value: u8,
    /// Note jouée (false : note relâchée). Toujours vrai pour un contrôleur.
    pub on: bool,
}

/// Décode un message MIDI (notes et contrôleurs ; le reste est ignoré).
pub fn parse(bytes: &[u8]) -> Option<Event> {
    let (&status, rest) = bytes.split_first()?;
    let channel = status & 0x0F;
    let (a, b) = (*rest.first()?, rest.get(1).copied().unwrap_or(0));
    match status & 0xF0 {
        0x90 => Some(Event { key: Key { kind: Kind::Note, channel, number: a }, value: b, on: b > 0 }),
        0x80 => Some(Event { key: Key { kind: Kind::Note, channel, number: a }, value: 0, on: false }),
        0xB0 => Some(Event { key: Key { kind: Kind::Cc, channel, number: a }, value: b, on: true }),
        _ => None,
    }
}

/// Commande physique du MPD218 : pad (0 = en bas à gauche, de gauche à droite puis de bas en
/// haut, comme les numéros imprimés) ou potentiomètre (0 = en haut à gauche), dans une banque.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Control {
    Pad { bank: u8, index: u8 },
    Knob { bank: u8, index: u8 },
}

impl Control {
    pub fn all() -> impl Iterator<Item = Control> {
        let pads = (0..BANKS as u8).flat_map(|bank| (0..PADS as u8).map(move |index| Control::Pad { bank, index }));
        let knobs = (0..BANKS as u8).flat_map(|bank| (0..KNOBS as u8).map(move |index| Control::Knob { bank, index }));
        pads.chain(knobs)
    }

    pub fn is_pad(self) -> bool {
        matches!(self, Control::Pad { .. })
    }

    pub fn name(self) -> String {
        let letter = |b: u8| (b'A' + b) as char;
        match self {
            Control::Pad { bank, index } => format!("Pad {} (banque {})", bank as usize * PADS + index as usize + 1, letter(bank)),
            Control::Knob { bank, index } => {
                format!("Potentiomètre {} (banque {})", bank as usize * KNOBS + index as usize + 1, letter(bank))
            }
        }
    }

    /// Message d'usine du MPD218 (preset 1) : pads en notes 36 à 83 sur le canal 10,
    /// potentiomètres en CC 3, 9, 12 à 27 sur le canal 1.
    pub fn default_key(self) -> Key {
        const KNOB_CC: [u8; 18] = [3, 9, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27];
        match self {
            Control::Pad { bank, index } => Key { kind: Kind::Note, channel: 9, number: 36 + bank * 16 + index },
            Control::Knob { bank, index } => {
                Key { kind: Kind::Cc, channel: 0, number: KNOB_CC[bank as usize * KNOBS + index as usize] }
            }
        }
    }
}

/// Réglage d'effet qu'un potentiomètre peut piloter : nom, minimum, maximum.
pub const FX_PARAMS: [(&str, f32, f32); 28] = [
    ("Luminosité", -1.0, 1.0),
    ("Contraste", 0.0, 3.0),
    ("Saturation", 0.0, 3.0),
    ("Teinte", 0.0, 1.0),
    ("Cycle teinte", -1.0, 1.0),
    ("Négatif", 0.0, 1.0),
    ("Flou", 0.0, 1.0),
    ("Halo lumineux", 0.0, 1.5),
    ("Zoom", 0.2, 5.0),
    ("Rotation", -180.0, 180.0),
    ("Rotation auto", -360.0, 360.0),
    ("Kaléidoscope", 0.0, 16.0),
    ("Pixelisation", 0.0, 1.0),
    ("Décalage RVB", 0.0, 0.05),
    ("Ondulation", 0.0, 0.1),
    ("Fish-eye", -1.5, 1.5),
    ("Tourbillon", -4.0, 4.0),
    ("Tunnel", 0.0, 1.0),
    ("Mosaïque miroir", 1.0, 8.0),
    ("Liquide", 0.0, 0.08),
    ("Palette acide", 0.0, 1.0),
    ("Arc-en-ciel", 0.0, 3.0),
    ("Solarisation", 0.0, 1.0),
    ("Écho", 0.0, 1.0),
    ("Glitch", 0.0, 1.0),
    ("Télé cathodique", 0.0, 1.0),
    ("Stroboscope", 0.0, 20.0),
    ("Pulsation", 0.0, 0.3),
];

/// Champ des effets piloté par le réglage `k` de `FX_PARAMS`.
pub fn fx_param(fx: &mut Effects, k: usize) -> Option<&mut f32> {
    Some(match k {
        0 => &mut fx.brightness,
        1 => &mut fx.contrast,
        2 => &mut fx.saturation,
        3 => &mut fx.hue,
        4 => &mut fx.hue_speed,
        5 => &mut fx.invert,
        6 => &mut fx.blur,
        7 => &mut fx.bloom,
        8 => &mut fx.zoom,
        9 => &mut fx.rotate,
        10 => &mut fx.rotate_speed,
        11 => &mut fx.kaleido,
        12 => &mut fx.pixelate,
        13 => &mut fx.rgb_shift,
        14 => &mut fx.wave_amp,
        15 => &mut fx.fisheye,
        16 => &mut fx.swirl,
        17 => &mut fx.polar,
        18 => &mut fx.tiles,
        19 => &mut fx.liquid,
        20 => &mut fx.acid,
        21 => &mut fx.rainbow,
        22 => &mut fx.solarize,
        23 => &mut fx.echo,
        24 => &mut fx.glitch,
        25 => &mut fx.crt,
        26 => &mut fx.strobe,
        27 => &mut fx.pulse,
        _ => return None,
    })
}

/// Commande du logiciel déclenchée par un pad, ou réglée par un potentiomètre.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Action {
    #[default]
    None,
    // --- Pads : déclenchements.
    /// Case du tableau (calque, colonne).
    PlayCell { layer: usize, column: usize },
    /// Toute une colonne du tableau.
    PlayColumn { column: usize },
    /// Preset d'effets (par son nom) sur le calque sélectionné.
    Preset { name: String },
    RandomEffect,
    ClearEffects,
    /// Média de la liste (index) sur le calque sélectionné.
    LaunchMedia { index: usize },
    NextMedia,
    PrevMedia,
    NextLayer,
    /// Sélectionne le calque.
    SelectLayer { layer: usize },
    Blackout,
    /// Noir tant que le pad est enfoncé.
    BlackoutHold,
    White,
    Bypass,
    Grid,
    Tap,
    Pause,
    Reverse,
    AutoFx,
    /// « Casser ! » : destruction du masque du calque sélectionné.
    Shatter,
    /// Envoi aux WLED (bascule).
    LedLive,
    /// Scénario LED.
    LedScenario { kind: ScenarioKind },
    /// Appareil DMX (id) : allumer / éteindre.
    DmxToggle { fixture: u32 },
    /// Appareil DMX (id) à fond tant qu'on appuie (fumée, bulles, flash).
    DmxHold { fixture: u32 },
    /// Lasers ILDA : armer / désarmer, tout arrêter, réglage mémorisé (rang) sur tous les lasers.
    LaserArm,
    LaserStop,
    LaserCue { index: usize },
    // --- Potentiomètres : valeurs continues.
    Master,
    /// Opacité d'un calque (aucun : le calque sélectionné).
    Opacity { layer: Option<usize> },
    /// Vitesse de lecture du calque sélectionné (×0,25 à ×4, ×1 au milieu).
    Speed,
    /// Durée des transitions entre presets.
    PresetFade,
    LedBrightness,
    /// Réglage d'effet (index dans `FX_PARAMS`) du calque sélectionné.
    Effect { param: usize },
}

impl Action {
    /// Commandes proposées pour un pad.
    pub fn triggers() -> Vec<Action> {
        vec![
            Action::PlayCell { layer: 0, column: 0 },
            Action::PlayColumn { column: 0 },
            Action::Preset { name: String::new() },
            Action::RandomEffect,
            Action::ClearEffects,
            Action::LaunchMedia { index: 0 },
            Action::NextMedia,
            Action::PrevMedia,
            Action::SelectLayer { layer: 0 },
            Action::NextLayer,
            Action::Blackout,
            Action::BlackoutHold,
            Action::White,
            Action::Bypass,
            Action::Grid,
            Action::Tap,
            Action::Pause,
            Action::Reverse,
            Action::AutoFx,
            Action::Shatter,
            Action::LedLive,
            Action::LedScenario { kind: ScenarioKind::HeartBeat },
            Action::DmxToggle { fixture: 0 },
            Action::DmxHold { fixture: 0 },
            Action::LaserArm,
            Action::LaserStop,
            Action::LaserCue { index: 0 },
        ]
    }

    /// Commandes proposées pour un potentiomètre.
    pub fn continuous() -> Vec<Action> {
        vec![
            Action::Master,
            Action::Opacity { layer: None },
            Action::Speed,
            Action::PresetFade,
            Action::LedBrightness,
            Action::Effect { param: 0 },
        ]
    }

    /// Même commande, paramètres mis à part.
    pub fn same_kind(&self, other: &Action) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Nom du type de commande.
    pub fn title(&self) -> &'static str {
        match self {
            Action::None => "Rien",
            Action::PlayCell { .. } => "Case du tableau",
            Action::PlayColumn { .. } => "Colonne du tableau",
            Action::Preset { .. } => "Preset d'effets",
            Action::RandomEffect => "Effet au hasard",
            Action::ClearEffects => "Sans effet (calque)",
            Action::LaunchMedia { .. } => "Média de la liste",
            Action::NextMedia => "Média suivant",
            Action::PrevMedia => "Média précédent",
            Action::SelectLayer { .. } => "Choisir un calque",
            Action::NextLayer => "Calque suivant",
            Action::Blackout => "Blackout",
            Action::BlackoutHold => "Noir tant qu'on appuie",
            Action::White => "Écran blanc",
            Action::Bypass => "Tout sans effets",
            Action::Grid => "Mire",
            Action::Tap => "Tap tempo",
            Action::Pause => "Pause / lecture",
            Action::Reverse => "Lecture à l'envers",
            Action::AutoFx => "Effets AUTO",
            Action::Shatter => "Casser ! (destruction)",
            Action::LedLive => "Envoi aux WLED",
            Action::LedScenario { .. } => "Scénario LED",
            Action::DmxToggle { .. } => "Appareil DMX : allumer / éteindre",
            Action::DmxHold { .. } => "Appareil DMX : tant qu'on appuie",
            Action::LaserArm => "Lasers : armer / désarmer",
            Action::LaserStop => "Lasers : STOP",
            Action::LaserCue { .. } => "Laser : réglage mémorisé",
            Action::Master => "Luminosité générale",
            Action::Opacity { .. } => "Opacité d'un calque",
            Action::Speed => "Vitesse de lecture",
            Action::PresetFade => "Durée des transitions",
            Action::LedBrightness => "Luminosité des LEDs",
            Action::Effect { .. } => "Réglage d'effet",
        }
    }

    /// Texte court affiché sur le dessin du contrôleur.
    pub fn short(&self) -> String {
        match self {
            Action::None => String::new(),
            Action::PlayCell { layer, column } => format!("Case C{} L{}", column + 1, layer + 1),
            Action::PlayColumn { column } => format!("Colonne {}", column + 1),
            Action::Preset { name } => name.clone(),
            Action::LaunchMedia { index } => format!("Média {}", index + 1),
            Action::SelectLayer { layer } => format!("Calque {}", layer + 1),
            Action::LedScenario { kind } => kind.name().split(" (").next().unwrap_or("").to_owned(),
            Action::Opacity { layer: Some(l) } => format!("Opacité C{}", l + 1),
            Action::Opacity { layer: None } => "Opacité".into(),
            Action::DmxToggle { .. } => "DMX on/off".into(),
            Action::DmxHold { .. } => "DMX à fond".into(),
            Action::LaserCue { index } => format!("Laser {}", index + 1),
            Action::Effect { param } => FX_PARAMS.get(*param).map_or("Effet", |p| p.0).to_owned(),
            other => other.title().to_owned(),
        }
    }
}

/// Commande assignée à un pad ou un potentiomètre.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub control: Control,
    pub action: Action,
}

/// Réglages MIDI (gardés d'une session à l'autre, quel que soit le projet).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MidiSetup {
    /// Port choisi (sinon : le premier qui ressemble à un MPD218, ou le premier tout court).
    pub port: Option<String>,
    /// Messages appris, à la place des valeurs d'usine.
    pub learned: Vec<(Control, Key)>,
    pub bindings: Vec<Binding>,
}

impl MidiSetup {
    pub fn key_of(&self, c: Control) -> Key {
        self.learned.iter().find(|l| l.0 == c).map_or(c.default_key(), |l| l.1)
    }

    pub fn control_of(&self, k: Key) -> Option<Control> {
        Control::all().find(|&c| self.key_of(c) == k)
    }

    /// Associe `k` à `c` (et le retire d'une autre commande qui l'avait).
    pub fn learn(&mut self, c: Control, k: Key) {
        self.learned.retain(|l| l.0 != c);
        // Une autre commande qui recevait ce message (en usine ou appris) n'en reçoit plus.
        let others: Vec<Control> = Control::all().filter(|&o| o != c && self.key_of(o) == k).collect();
        self.learned.push((c, k));
        for o in others {
            self.learned.retain(|l| l.0 != o);
            // Message impossible : la commande ne répond plus jusqu'à un nouvel apprentissage.
            self.learned.push((o, Key { kind: Kind::Note, channel: 15, number: 127 }));
        }
    }

    pub fn action(&self, c: Control) -> &Action {
        static NONE: Action = Action::None;
        self.bindings.iter().find(|b| b.control == c).map_or(&NONE, |b| &b.action)
    }

    pub fn set_action(&mut self, c: Control, a: Action) {
        self.bindings.retain(|b| b.control != c);
        if a != Action::None {
            self.bindings.push(Binding { control: c, action: a });
        }
    }
}

/// Entrée MIDI ouverte : les messages arrivent dans `queue` (thread de midir).
pub struct Input {
    _conn: midir::MidiInputConnection<()>,
    pub port: String,
    queue: Arc<Mutex<Vec<Event>>>,
}

/// Ports MIDI d'entrée disponibles.
pub fn ports() -> Vec<String> {
    let Ok(input) = midir::MidiInput::new("OZ Mapping") else { return Vec::new() };
    input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect()
}

impl Input {
    pub fn open(name: &str) -> Result<Self> {
        let mut input = midir::MidiInput::new("OZ Mapping").map_err(|e| anyhow!("MIDI indisponible : {e}"))?;
        input.ignore(midir::Ignore::All);
        let port = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).is_ok_and(|n| n == name))
            .ok_or_else(|| anyhow!("{name} n'est plus branché"))?;
        let queue = Arc::new(Mutex::new(Vec::new()));
        let q = queue.clone();
        let conn = input
            .connect(
                &port,
                "oz-mapping-in",
                move |_, bytes, _| {
                    if let Some(e) = parse(bytes) {
                        q.lock().unwrap().push(e);
                    }
                },
                (),
            )
            .map_err(|e| anyhow!("connexion à {name} : {e}"))?;
        Ok(Self { _conn: conn, port: name.to_owned(), queue })
    }

    pub fn drain(&self) -> Vec<Event> {
        std::mem::take(&mut *self.queue.lock().unwrap())
    }
}

/// Port à ouvrir : celui choisi s'il est là, sinon un MPD218, sinon le premier.
pub fn pick_port(ports: &[String], wanted: Option<&str>) -> Option<String> {
    wanted
        .and_then(|w| ports.iter().find(|p| p.as_str() == w))
        .or_else(|| ports.iter().find(|p| p.to_uppercase().contains("MPD218")))
        .or_else(|| ports.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pads_and_knobs() {
        let pad = parse(&[0x99, 40, 100]).unwrap();
        assert_eq!(pad.key, Key { kind: Kind::Note, channel: 9, number: 40 });
        assert!(pad.on);
        assert!(!parse(&[0x99, 40, 0]).unwrap().on);
        assert!(!parse(&[0x89, 40, 64]).unwrap().on);
        let knob = parse(&[0xB0, 3, 64]).unwrap();
        assert_eq!(knob.key.kind, Kind::Cc);
        assert!(parse(&[0xA9, 40, 10]).is_none());
    }

    #[test]
    fn factory_layout_is_unique_and_learnable() {
        let mut m = MidiSetup::default();
        let keys: std::collections::HashSet<Key> = Control::all().map(|c| m.key_of(c)).collect();
        assert_eq!(keys.len(), BANKS * (PADS + KNOBS));
        let pad5 = Control::Pad { bank: 0, index: 4 };
        assert_eq!(m.control_of(Key { kind: Kind::Note, channel: 9, number: 40 }), Some(pad5));
        // Un pad appris sur le message d'un autre : l'autre ne répond plus.
        let k = m.key_of(Control::Pad { bank: 1, index: 0 });
        m.learn(pad5, k);
        assert_eq!(m.control_of(k), Some(pad5));
        for i in 0..FX_PARAMS.len() {
            assert!(fx_param(&mut Effects::default(), i).is_some());
        }
    }
}
