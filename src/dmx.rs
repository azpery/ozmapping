//! Appareils DMX posés dans la scène : laser, derby, machine à bulles, machine à fumée,
//! projecteur RGB, ou appareil quelconque. Sortie en Art-Net, sACN (E1.31) ou par une interface
//! USB Enttec DMX USB Pro.
//!
//! Chaque appareil a un profil (ses canaux et leur rôle). Les cartes de canaux changent d'un
//! modèle à l'autre (voir la notice de l'appareil) : les profils proposés sont des points de
//! départ, modifiables. Les lasers Laserworld se pilotent en DMX (leur réseau ShowNET est un
//! protocole fermé, réservé à leur logiciel).

use std::collections::HashMap;
use std::io::Write as _;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Laser,
    Derby,
    Bubble,
    Smoke,
    Par,
    Generic,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Laser, Kind::Derby, Kind::Bubble, Kind::Smoke, Kind::Par, Kind::Generic];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Laser => "Laser",
            Kind::Derby => "Derby",
            Kind::Bubble => "Machine à bulles",
            Kind::Smoke => "Machine à fumée",
            Kind::Par => "Projecteur RGB",
            Kind::Generic => "Appareil DMX",
        }
    }

    /// Couleur et dimensions (m) du boîtier dans la scène.
    pub fn body(self) -> ([u8; 3], [f32; 3]) {
        match self {
            Kind::Laser => ([30, 30, 34], [0.26, 0.12, 0.3]),
            Kind::Derby => ([36, 36, 40], [0.32, 0.22, 0.26]),
            Kind::Bubble => ([40, 80, 160], [0.34, 0.26, 0.3]),
            Kind::Smoke => ([70, 72, 78], [0.5, 0.26, 0.3]),
            Kind::Par => ([26, 26, 30], [0.22, 0.22, 0.2]),
            Kind::Generic => ([60, 60, 66], [0.25, 0.2, 0.25]),
        }
    }
}

/// Rôle d'un canal : ce que le simulateur en fait, et la valeur « éteint ».
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[default]
    Other,
    /// Mode (laser, derby) : 0 = éteint.
    Mode,
    Dimmer,
    Red,
    Green,
    Blue,
    White,
    Strobe,
    Pattern,
    Size,
    Rotation,
    PosX,
    PosY,
    Color,
    Speed,
    /// Débit : fumée, bulles.
    Output,
}

impl Role {
    pub const ALL: [Role; 16] = [
        Role::Mode,
        Role::Dimmer,
        Role::Red,
        Role::Green,
        Role::Blue,
        Role::White,
        Role::Strobe,
        Role::Pattern,
        Role::Size,
        Role::Rotation,
        Role::PosX,
        Role::PosY,
        Role::Color,
        Role::Speed,
        Role::Output,
        Role::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Role::Other => "Autre",
            Role::Mode => "Mode",
            Role::Dimmer => "Intensité",
            Role::Red => "Rouge",
            Role::Green => "Vert",
            Role::Blue => "Bleu",
            Role::White => "Blanc",
            Role::Strobe => "Stroboscope",
            Role::Pattern => "Motif",
            Role::Size => "Taille",
            Role::Rotation => "Rotation",
            Role::PosX => "Position X",
            Role::PosY => "Position Y",
            Role::Color => "Couleur",
            Role::Speed => "Vitesse",
            Role::Output => "Débit",
        }
    }

    /// Canaux remis à zéro quand l'appareil est éteint.
    fn off_at_zero(self) -> bool {
        matches!(self, Role::Mode | Role::Dimmer | Role::Output | Role::Red | Role::Green | Role::Blue | Role::White)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Channel {
    pub name: String,
    pub role: Role,
    /// Valeur réglée (0-255).
    pub value: u8,
}

impl Default for Channel {
    fn default() -> Self {
        Self { name: "Canal".into(), role: Role::Other, value: 0 }
    }
}

fn ch(name: &str, role: Role, value: u8) -> Channel {
    Channel { name: name.into(), role, value }
}

/// Profil de départ d'un type d'appareil.
pub fn profile(kind: Kind) -> Vec<Channel> {
    match kind {
        // Laser DMX type Laserworld (vérifier la notice : l'ordre change selon les modèles).
        Kind::Laser => vec![
            ch("Mode (0 éteint, DMX)", Role::Mode, 200),
            ch("Motif", Role::Pattern, 0),
            ch("Taille", Role::Size, 128),
            ch("Rotation", Role::Rotation, 0),
            ch("Position X", Role::PosX, 128),
            ch("Position Y", Role::PosY, 128),
            ch("Couleur", Role::Color, 0),
            ch("Stroboscope", Role::Strobe, 0),
            ch("Vitesse", Role::Speed, 128),
        ],
        Kind::Derby => vec![
            ch("Mode (0 éteint)", Role::Mode, 255),
            ch("Couleur", Role::Color, 0),
            ch("Stroboscope", Role::Strobe, 0),
            ch("Rotation", Role::Rotation, 128),
        ],
        Kind::Bubble => vec![ch("Bulles", Role::Output, 255)],
        Kind::Smoke => vec![ch("Fumée", Role::Output, 0)],
        Kind::Par => vec![
            ch("Intensité", Role::Dimmer, 255),
            ch("Rouge", Role::Red, 255),
            ch("Vert", Role::Green, 0),
            ch("Bleu", Role::Blue, 200),
            ch("Stroboscope", Role::Strobe, 0),
        ],
        Kind::Generic => vec![ch("Canal 1", Role::Dimmer, 255)],
    }
}

/// Appareil DMX posé dans la scène.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fixture {
    pub id: u32,
    pub name: String,
    pub kind: Kind,
    /// Univers (0 = le premier) et adresse du premier canal (1-512).
    pub universe: u16,
    pub address: u16,
    pub channels: Vec<Channel>,
    /// Milieu du dessous du boîtier ; orientation comme les objets de la scène (le faisceau part
    /// vers l'avant du boîtier, vers le fond de la salle si rien n'est tourné).
    pub pos: [f32; 3],
    pub rot: [f32; 3],
    /// Allumé (éteint : mode, intensité et débit à 0).
    pub on: bool,
    /// Réagit aux temps : le laser change de motif, le projecteur flashe.
    pub beat: bool,
    /// Groupe d'objets de la scène (id, 0 : aucun).
    pub set: u32,
    /// Noms des motifs (laser : 16 cases de la galerie, valeurs 0, 16, 32…).
    pub labels: Vec<String>,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            id: 1,
            name: "Laser 1".into(),
            kind: Kind::Laser,
            universe: 0,
            address: 1,
            channels: profile(Kind::Laser),
            pos: [0.0, 2.0, 1.0],
            rot: [0.0; 3],
            on: true,
            beat: false,
            set: 0,
            labels: Vec::new(),
        }
    }
}

/// Ce qui change les valeurs envoyées en direct : rafale de fumée (ou bulles) en cours, nombre
/// de temps écoulés, enveloppe du dernier temps (0-1).
#[derive(Clone, Copy, Default)]
pub struct Live {
    pub burst: bool,
    pub beats: u64,
    pub env: f32,
}

impl Fixture {
    /// Valeurs envoyées (et simulées), canal par canal.
    pub fn values(&self, live: Live) -> Vec<u8> {
        // Appui maintenu (rafale) : allumé le temps de l'appui.
        let on = self.on || live.burst;
        self.channels
            .iter()
            .map(|c| {
                if !on && c.role.off_at_zero() {
                    return 0;
                }
                match c.role {
                    Role::Output if live.burst => 255,
                    // Sur les temps : le motif avance d'un cran (16 valeurs) à chaque temps.
                    Role::Pattern if self.beat => c.value.wrapping_add((live.beats % 16) as u8 * 16),
                    Role::Dimmer if self.beat => (f32::from(c.value) * (0.25 + 0.75 * live.env)) as u8,
                    _ => c.value,
                }
            })
            .collect()
    }

    /// Valeur effective d'un rôle (simulation).
    pub fn live_role(&self, r: Role, live: Live) -> Option<u8> {
        let v = self.values(live);
        self.channels.iter().position(|c| c.role == r).map(|i| v[i])
    }

    /// Dernière adresse occupée.
    pub fn last_address(&self) -> u16 {
        self.address + self.channels.len().max(1) as u16 - 1
    }
}

/// Protocole de sortie.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    #[default]
    ArtNet,
    Sacn,
    /// Interface USB Enttec DMX USB Pro (univers 0 seulement).
    EnttecPro,
}

impl Protocol {
    pub const ALL: [Protocol; 3] = [Protocol::ArtNet, Protocol::Sacn, Protocol::EnttecPro];

    pub fn name(self) -> &'static str {
        match self {
            Protocol::ArtNet => "Art-Net (réseau)",
            Protocol::Sacn => "sACN / E1.31 (réseau)",
            Protocol::EnttecPro => "Enttec DMX USB Pro",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Output {
    pub protocol: Protocol,
    /// Adresse du boîtier réseau (vide : diffusion à tout le réseau / multicast sACN).
    pub host: String,
    /// Port série de l'interface USB.
    pub serial: String,
    pub fps: u32,
}

impl Default for Output {
    fn default() -> Self {
        Self { protocol: Protocol::ArtNet, host: String::new(), serial: String::new(), fps: 40 }
    }
}

/// Univers DMX à envoyer, construits depuis les appareils.
pub fn universes(fixtures: &[Fixture], live: &dyn Fn(&Fixture) -> Live) -> Vec<(u16, [u8; 512])> {
    let mut map: HashMap<u16, [u8; 512]> = HashMap::new();
    for f in fixtures {
        let data = map.entry(f.universe).or_insert([0; 512]);
        for (k, v) in f.values(live(f)).into_iter().enumerate() {
            let a = f.address as usize + k;
            if (1..=512).contains(&a) {
                data[a - 1] = v;
            }
        }
    }
    let mut out: Vec<(u16, [u8; 512])> = map.into_iter().collect();
    out.sort_by_key(|u| u.0);
    out
}

/// Paquet ArtDMX.
pub fn artnet_packet(universe: u16, seq: u8, data: &[u8; 512]) -> Vec<u8> {
    let mut p = Vec::with_capacity(18 + 512);
    p.extend_from_slice(b"Art-Net\0");
    p.extend_from_slice(&0x5000u16.to_le_bytes()); // OpDmx
    p.extend_from_slice(&[0, 14]); // version du protocole
    p.push(seq);
    p.push(0); // physique
    p.push((universe & 0xFF) as u8); // SubUni
    p.push(((universe >> 8) & 0x7F) as u8); // Net
    p.extend_from_slice(&512u16.to_be_bytes());
    p.extend_from_slice(data);
    p
}

/// Paquet sACN (E1.31) pour l'univers `universe` (le premier univers sACN est 1).
pub fn sacn_packet(universe: u16, seq: u8, data: &[u8; 512], cid: &[u8; 16]) -> Vec<u8> {
    let mut p = Vec::with_capacity(638);
    // Couche racine.
    p.extend_from_slice(&0x0010u16.to_be_bytes());
    p.extend_from_slice(&0u16.to_be_bytes());
    p.extend_from_slice(b"ASC-E1.17\0\0\0");
    p.extend_from_slice(&(0x7000u16 | (638 - 16)).to_be_bytes());
    p.extend_from_slice(&4u32.to_be_bytes());
    p.extend_from_slice(cid);
    // Couche de trame.
    p.extend_from_slice(&(0x7000u16 | (638 - 38)).to_be_bytes());
    p.extend_from_slice(&2u32.to_be_bytes());
    let mut name = [0u8; 64];
    name[..10].copy_from_slice(b"OZ Mapping");
    p.extend_from_slice(&name);
    p.push(100); // priorité
    p.extend_from_slice(&0u16.to_be_bytes()); // synchronisation
    p.push(seq);
    p.push(0); // options
    p.extend_from_slice(&universe.to_be_bytes());
    // Couche DMP.
    p.extend_from_slice(&(0x7000u16 | (638 - 115)).to_be_bytes());
    p.push(0x02);
    p.push(0xA1);
    p.extend_from_slice(&0u16.to_be_bytes());
    p.extend_from_slice(&1u16.to_be_bytes());
    p.extend_from_slice(&513u16.to_be_bytes());
    p.push(0); // code de départ
    p.extend_from_slice(data);
    p
}

/// Message « envoyer DMX » de l'Enttec DMX USB Pro.
pub fn enttec_packet(data: &[u8; 512]) -> Vec<u8> {
    let len = 513u16;
    let mut p = Vec::with_capacity(518);
    p.extend_from_slice(&[0x7E, 6, (len & 0xFF) as u8, (len >> 8) as u8, 0]);
    p.extend_from_slice(data);
    p.push(0xE7);
    p
}

/// Ports série disponibles (pour l'interface USB).
pub fn serial_ports() -> Vec<String> {
    serialport::available_ports().map(|v| v.into_iter().map(|p| p.port_name).collect()).unwrap_or_default()
}

struct Frame {
    out: Output,
    universes: Vec<(u16, [u8; 512])>,
}

/// Envoi en arrière-plan ; `status` : Ok(dernier envoi) ou l'erreur.
pub struct Sender {
    tx: SyncSender<Frame>,
    pub status: Arc<Mutex<Option<Result<Instant, String>>>>,
}

impl Default for Sender {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel::<Frame>(4);
        let status = Arc::new(Mutex::new(None));
        let st = status.clone();
        std::thread::spawn(move || send_loop(rx, st));
        Self { tx, status }
    }
}

impl Sender {
    pub fn send(&self, out: &Output, universes: Vec<(u16, [u8; 512])>) {
        let _ = self.tx.try_send(Frame { out: out.clone(), universes });
    }
}

fn send_loop(rx: Receiver<Frame>, status: Arc<Mutex<Option<Result<Instant, String>>>>) {
    let socket = UdpSocket::bind("0.0.0.0:0").ok();
    if let Some(s) = &socket {
        let _ = s.set_broadcast(true);
    }
    let mut seq: u8 = 0;
    let mut serial: Option<(String, Box<dyn serialport::SerialPort>)> = None;
    let mut addrs: HashMap<String, Option<SocketAddr>> = HashMap::new();
    let cid: [u8; 16] = *b"OZ-Mapping-sACN!";
    for frame in rx {
        seq = seq.wrapping_add(1).max(1);
        let res: Result<(), String> = (|| {
            match frame.out.protocol {
                Protocol::ArtNet | Protocol::Sacn => {
                    let sock = socket.as_ref().ok_or("socket UDP indisponible")?;
                    let artnet = frame.out.protocol == Protocol::ArtNet;
                    for (u, data) in &frame.universes {
                        let host = frame.out.host.trim();
                        let target = if host.is_empty() {
                            if artnet {
                                SocketAddr::from(([255, 255, 255, 255], 6454))
                            } else {
                                // Multicast sACN : 239.255.<univers>.
                                let uni = u + 1;
                                SocketAddr::from(([239, 255, (uni >> 8) as u8, (uni & 0xFF) as u8], 5568))
                            }
                        } else {
                            let port = if artnet { 6454 } else { 5568 };
                            let key = format!("{host}:{port}");
                            let a = *addrs.entry(key.clone()).or_insert_with(|| key.to_socket_addrs().ok().and_then(|mut i| i.next()));
                            a.ok_or_else(|| format!("adresse {host} introuvable"))?
                        };
                        let pkt = if artnet { artnet_packet(*u, seq, data) } else { sacn_packet(u + 1, seq, data, &cid) };
                        sock.send_to(&pkt, target).map_err(|e| format!("envoi : {e}"))?;
                    }
                }
                Protocol::EnttecPro => {
                    let name = frame.out.serial.trim().to_owned();
                    if name.is_empty() {
                        return Err("choisissez le port de l'interface USB".into());
                    }
                    if serial.as_ref().is_none_or(|s| s.0 != name) {
                        let port = serialport::new(&name, 57_600)
                            .timeout(Duration::from_millis(100))
                            .open()
                            .map_err(|e| format!("{name} : {e}"))?;
                        serial = Some((name.clone(), port));
                    }
                    let data = frame.universes.iter().find(|u| u.0 == 0).map_or([0u8; 512], |u| u.1);
                    let port = &mut serial.as_mut().unwrap().1;
                    if let Err(e) = port.write_all(&enttec_packet(&data)) {
                        serial = None;
                        return Err(format!("{name} : {e}"));
                    }
                }
            }
            Ok(())
        })();
        *status.lock().unwrap() = Some(res.map(|_| Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_have_the_right_shape() {
        let mut data = [0u8; 512];
        data[0] = 255;
        let a = artnet_packet(1, 7, &data);
        assert_eq!(&a[..8], b"Art-Net\0");
        assert_eq!(a.len(), 530);
        assert_eq!((a[14], a[15]), (1, 0));
        assert_eq!(a[18], 255);
        let s = sacn_packet(1, 7, &data, b"0123456789abcdef");
        assert_eq!(s.len(), 638);
        assert_eq!(s[125], 0); // code de départ
        assert_eq!(s[126], 255);
        let e = enttec_packet(&data);
        assert_eq!((e[0], e[1], e[517]), (0x7E, 6, 0xE7));
    }

    #[test]
    fn fixtures_fill_their_channels() {
        let laser = Fixture { address: 10, ..Fixture::default() };
        let smoke = Fixture { id: 2, kind: Kind::Smoke, channels: profile(Kind::Smoke), address: 30, ..Fixture::default() };
        let u = universes(&[laser.clone(), smoke.clone()], &|f| Live { burst: f.kind == Kind::Smoke, ..Live::default() });
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].1[9], 200); // mode du laser
        assert_eq!(u[0].1[29], 255); // rafale de fumée
        let off = Fixture { on: false, ..laser };
        assert_eq!(off.values(Live::default())[0], 0);
    }
}
