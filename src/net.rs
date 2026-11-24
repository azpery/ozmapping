//! Plusieurs OZ Mapping sur le même réseau local : une modification faite sur l'un se répercute
//! sur les autres.
//!
//! - Découverte : chaque machine annonce sa présence par diffusion UDP (port 47821), avec le nom
//!   de sa « salle » : seules les machines d'une même salle se lient.
//! - Liaison : une connexion TCP par paire de machines ; des messages JSON précédés de leur
//!   longueur. Le projet circule par morceaux (un calque, la scène 3D…), chacun avec un numéro
//!   de version (horloge de Lamport, puis identifiant de la machine) : toutes les machines gardent
//!   la même version de chaque morceau, la plus récente.
//! - Médias : un fichier absent d'une machine est copié depuis celle qui l'a, par une connexion
//!   TCP à part (une vidéo de plusieurs Go ne retarde pas les modifications).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DISCOVERY_PORT: u16 = 47821;
pub const TCP_PORT: u16 = 47822;
const MAGIC: &str = "OZMAP1";
/// Taille maximale d'un message (un projet avec beaucoup de LEDs reste bien en dessous).
const MAX_FRAME: usize = 256 << 20;

/// Réglages gardés d'une session à l'autre.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetSettings {
    pub enabled: bool,
    /// Nom de cette machine, montré aux autres.
    pub name: String,
    /// Seules les machines de la même salle se synchronisent.
    pub room: String,
    /// Adresses ajoutées à la main (réseau qui bloque la diffusion), « ip » ou « ip:port ».
    pub manual: Vec<String>,
}

impl Default for NetSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            name: sysinfo::System::host_name().unwrap_or_else(|| "OZ Mapping".into()),
            room: "OZ".into(),
            manual: Vec::new(),
        }
    }
}

/// Version d'un morceau du projet : (horloge, machine). La plus grande gagne.
pub type Ver = (u64, u64);

/// Lecture d'un calque : ce qui joue, où, comment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveLayer {
    /// Fichier en lecture (chemin de référence, voir `canonical`).
    pub path: Option<String>,
    pub pos: f64,
    pub speed: f32,
    pub reverse: bool,
    pub paused: bool,
    pub ver: Ver,
}

/// État du direct : lecture des calques et boutons de la barre du haut.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Live {
    pub layers: Vec<LiveLayer>,
    /// Blackout, écran blanc, sans effets, mire.
    pub flags: [bool; 4],
    pub flags_ver: Ver,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Msg {
    Hello { id: u64, name: String, room: String, port: u16, since: u64, peers: Vec<String> },
    /// Demande du projet complet (machine qui arrive).
    Want,
    Full { state: Value, vers: HashMap<String, Ver>, clock: u64 },
    Patch { changes: Vec<(String, Value, Ver)> },
    Live(Live),
    FileGet { path: String },
    FileInfo { size: Option<u64> },
}

pub enum Event {
    Up { id: u64, name: String, since: u64 },
    Down { name: String },
    Msg { from: u64, msg: Msg },
    /// Copie d'un média terminée : chemin de référence, fichier local.
    File { canon: String, result: Result<PathBuf, String> },
}

#[derive(Clone, Debug)]
pub struct PeerView {
    pub id: u64,
    pub name: String,
    pub addr: SocketAddr,
    pub since: u64,
}

struct Peer {
    view: PeerView,
    tx: mpsc::Sender<Vec<u8>>,
    stream: TcpStream,
    /// Numéro de la connexion : une connexion remplacée ne retire pas sa remplaçante.
    serial: u64,
    /// Machine qui a ouvert la connexion (en double, on garde celle ouverte par le plus petit id).
    opener: u64,
}

struct Shared {
    id: u64,
    name: String,
    room: String,
    since: u64,
    port: u16,
    peers: Mutex<HashMap<u64, Peer>>,
    /// Médias que cette machine accepte d'envoyer : chemin de référence → fichier local.
    files: Mutex<HashMap<String, PathBuf>>,
    /// Copies en cours : chemin de référence → (reçu, taille).
    progress: Mutex<HashMap<String, (u64, u64)>>,
    /// Dernières tentatives de connexion (pour ne pas insister).
    tried: Mutex<HashMap<SocketAddr, Instant>>,
    events: mpsc::Sender<Event>,
    repaint: Box<dyn Fn() + Send + Sync>,
    stop: AtomicBool,
    serial: AtomicU64,
}

impl Shared {
    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
        (self.repaint)();
    }

    fn hello(&self) -> Msg {
        let peers = self.peers.lock().unwrap().values().map(|p| p.view.addr.to_string()).collect();
        Msg::Hello { id: self.id, name: self.name.clone(), room: self.room.clone(), port: self.port, since: self.since, peers }
    }
}

/// Liaison réseau active. La couper (drop) ferme toutes les connexions.
pub struct Net {
    shared: Arc<Shared>,
    pub events: mpsc::Receiver<Event>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn new_id() -> u64 {
    // Identifiant aléatoire : heure en nanosecondes, processus, compteur (deux liaisons du même
    // processus ne se confondent pas).
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut x = nanos ^ (u64::from(std::process::id()) << 32) ^ COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x.max(1)
}

pub fn encode(msg: &Msg) -> Vec<u8> {
    let json = serde_json::to_vec(msg).unwrap_or_default();
    let mut out = Vec::with_capacity(json.len() + 4);
    out.extend_from_slice(&(json.len() as u32).to_be_bytes());
    out.extend_from_slice(&json);
    out
}

pub fn read_msg(r: &mut impl Read) -> Result<Msg> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        bail!("message trop gros ({len} octets)");
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}

impl Net {
    pub fn start(settings: &NetSettings, repaint: impl Fn() + Send + Sync + 'static) -> Result<Net> {
        // Port fixe si libre (pratique pour les adresses saisies à la main), sinon n'importe lequel
        // (deux OZ Mapping sur la même machine).
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, TCP_PORT))
            .or_else(|_| TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)))
            .context("ouverture du port TCP")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            id: new_id(),
            name: settings.name.trim().to_string(),
            room: settings.room.trim().to_string(),
            since: now_ms(),
            port,
            peers: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
            progress: Mutex::new(HashMap::new()),
            tried: Mutex::new(HashMap::new()),
            events: tx,
            repaint: Box::new(repaint),
            stop: AtomicBool::new(false),
            serial: AtomicU64::new(1),
        });
        let s = shared.clone();
        std::thread::Builder::new().name("oz-net-accept".into()).spawn(move || accept_loop(s, listener))?;
        let udp = discovery_socket().context("ouverture du port de découverte")?;
        let s = shared.clone();
        std::thread::Builder::new().name("oz-net-discover".into()).spawn(move || discovery_loop(s, udp))?;
        let net = Net { shared, events: rx };
        for addr in &settings.manual {
            net.connect(addr);
        }
        Ok(net)
    }

    pub fn id(&self) -> u64 {
        self.shared.id
    }

    pub fn since(&self) -> u64 {
        self.shared.since
    }

    pub fn port(&self) -> u16 {
        self.shared.port
    }

    pub fn peers(&self) -> Vec<PeerView> {
        let mut v: Vec<PeerView> = self.shared.peers.lock().unwrap().values().map(|p| p.view.clone()).collect();
        v.sort_by_key(|p| p.since);
        v
    }

    pub fn send_all(&self, msg: &Msg) {
        let frame = encode(msg);
        for p in self.shared.peers.lock().unwrap().values() {
            let _ = p.tx.send(frame.clone());
        }
    }

    pub fn send_to(&self, id: u64, msg: &Msg) {
        if let Some(p) = self.shared.peers.lock().unwrap().get(&id) {
            let _ = p.tx.send(encode(msg));
        }
    }

    /// Se lie à une adresse saisie à la main (« 192.168.1.20 » ou « 192.168.1.20:47822 »).
    pub fn connect(&self, addr: &str) {
        let addr = addr.trim().to_string();
        if addr.is_empty() {
            return;
        }
        let s = self.shared.clone();
        std::thread::spawn(move || {
            let with_port = if addr.contains(':') { addr.clone() } else { format!("{addr}:{TCP_PORT}") };
            if let Some(a) = with_port.to_socket_addrs().ok().and_then(|mut it| it.next()) {
                dial(&s, a);
            }
        });
    }

    /// Médias que les autres peuvent demander (chemin de référence → fichier local).
    pub fn set_files(&self, files: HashMap<String, PathBuf>) {
        *self.shared.files.lock().unwrap() = files;
    }

    pub fn progress(&self) -> Vec<(String, u64, u64)> {
        self.shared.progress.lock().unwrap().iter().map(|(k, (a, b))| (k.clone(), *a, *b)).collect()
    }

    /// Copie un média depuis une machine, en arrière-plan (événement `File` à la fin).
    pub fn fetch(&self, from: u64, canon: String, dest: PathBuf) {
        let Some(addr) = self.shared.peers.lock().unwrap().get(&from).map(|p| p.view.addr) else { return };
        let s = self.shared.clone();
        s.progress.lock().unwrap().insert(canon.clone(), (0, 0));
        std::thread::spawn(move || {
            let result = download(&s, addr, &canon, &dest).map(|()| dest.clone()).map_err(|e| format!("{e:#}"));
            s.progress.lock().unwrap().remove(&canon);
            s.emit(Event::File { canon, result });
        });
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        for p in self.shared.peers.lock().unwrap().drain().map(|(_, p)| p) {
            let _ = p.stream.shutdown(Shutdown::Both);
        }
    }
}

fn discovery_socket() -> Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    s.set_reuse_port(true)?;
    s.set_broadcast(true)?;
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).into())?;
    let udp: UdpSocket = s.into();
    udp.set_read_timeout(Some(Duration::from_millis(300)))?;
    Ok(udp)
}

fn discovery_loop(s: Arc<Shared>, udp: UdpSocket) {
    let announce = format!("{MAGIC}|{}|{}|{}", s.id, s.port, s.room);
    let mut last = Instant::now() - Duration::from_secs(5);
    let mut buf = [0u8; 512];
    while !s.stop.load(Ordering::Relaxed) {
        if last.elapsed() >= Duration::from_secs(1) {
            last = Instant::now();
            let _ = udp.send_to(announce.as_bytes(), (Ipv4Addr::BROADCAST, DISCOVERY_PORT));
            // Même machine (deux OZ Mapping) : la diffusion ne revient pas toujours en local.
            let _ = udp.send_to(announce.as_bytes(), (Ipv4Addr::LOCALHOST, DISCOVERY_PORT));
        }
        let Ok((n, from)) = udp.recv_from(&mut buf) else { continue };
        let Ok(text) = std::str::from_utf8(&buf[..n]) else { continue };
        let parts: Vec<&str> = text.splitn(4, '|').collect();
        let [magic, id, port, room] = parts[..] else { continue };
        let (Ok(id), Ok(port)) = (id.parse::<u64>(), port.parse::<u16>()) else { continue };
        // La machine au plus petit identifiant ouvre la connexion : une seule par paire.
        if magic != MAGIC || room != s.room || id == s.id || id < s.id || s.peers.lock().unwrap().contains_key(&id) {
            continue;
        }
        let addr = SocketAddr::new(from.ip(), port);
        let s2 = s.clone();
        std::thread::spawn(move || dial(&s2, addr));
    }
}

fn accept_loop(s: Arc<Shared>, listener: TcpListener) {
    while !s.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let s2 = s.clone();
                std::thread::spawn(move || {
                    let _ = incoming(&s2, stream);
                });
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Connexion reçue : une machine qui se lie, ou une demande de média.
fn incoming(s: &Arc<Shared>, mut stream: TcpStream) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    match read_msg(&mut stream)? {
        Msg::Hello { id, name, room, port, since, peers } => {
            stream.write_all(&encode(&s.hello()))?;
            attach(s, stream, id, name, room, port, since, peers, id)
        }
        Msg::FileGet { path } => serve_file(s, stream, &path),
        _ => Ok(()),
    }
}

/// Ouvre une connexion vers une machine.
fn dial(s: &Arc<Shared>, addr: SocketAddr) {
    {
        let mut tried = s.tried.lock().unwrap();
        if tried.get(&addr).is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            return;
        }
        tried.insert(addr, Instant::now());
    }
    if s.peers.lock().unwrap().values().any(|p| p.view.addr == addr) {
        return;
    }
    let run = || -> Result<()> {
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.write_all(&encode(&s.hello()))?;
        match read_msg(&mut stream)? {
            Msg::Hello { id, name, room, port, since, peers } => attach(s, stream, id, name, room, port, since, peers, s.id),
            _ => bail!("réponse inattendue"),
        }
    };
    let _ = run();
}

#[allow(clippy::too_many_arguments)]
fn attach(
    s: &Arc<Shared>,
    stream: TcpStream,
    id: u64,
    name: String,
    room: String,
    port: u16,
    since: u64,
    gossip: Vec<String>,
    opener: u64,
) -> Result<()> {
    if id == s.id || room != s.room {
        return Ok(());
    }
    stream.set_read_timeout(None)?;
    stream.set_nodelay(true)?;
    let ip = stream.peer_addr()?.ip();
    let view = PeerView { id, name: name.clone(), addr: SocketAddr::new(ip, port), since };
    let serial = s.serial.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    {
        let mut peers = s.peers.lock().unwrap();
        if let Some(old) = peers.get(&id) {
            // Deux connexions pour la même paire : on garde celle ouverte par le plus petit id
            // (les deux machines font le même choix).
            let keep_new = opener == s.id.min(id) && old.opener != opener;
            if !keep_new {
                return Ok(());
            }
            let _ = old.stream.shutdown(Shutdown::Both);
        }
        peers.insert(id, Peer { view, tx, stream: stream.try_clone()?, serial, opener });
    }
    s.emit(Event::Up { id, name: name.clone(), since });

    // Écriture : un fil par connexion, pour ne jamais bloquer l'interface.
    let mut w = stream.try_clone()?;
    std::thread::spawn(move || {
        for frame in rx {
            if w.write_all(&frame).is_err() {
                break;
            }
        }
        let _ = w.shutdown(Shutdown::Both);
    });

    // Les machines que l'autre connaît : on s'y lie aussi (réseau maillé).
    for a in gossip {
        if let Ok(a) = a.parse::<SocketAddr>() {
            let s2 = s.clone();
            std::thread::spawn(move || dial(&s2, a));
        }
    }

    // Lecture, jusqu'à la coupure.
    let mut r = stream;
    while let Ok(msg) = read_msg(&mut r) {
        s.emit(Event::Msg { from: id, msg });
    }
    let mut peers = s.peers.lock().unwrap();
    if peers.get(&id).is_some_and(|p| p.serial == serial) {
        peers.remove(&id);
        drop(peers);
        s.emit(Event::Down { name });
    }
    Ok(())
}

fn serve_file(s: &Arc<Shared>, mut stream: TcpStream, path: &str) -> Result<()> {
    // Seulement les médias du projet : jamais un fichier quelconque de la machine.
    let local = s.files.lock().unwrap().get(path).cloned();
    let file = local.as_deref().and_then(|p| std::fs::File::open(p).ok());
    let size = file.as_ref().and_then(|f| f.metadata().ok()).map(|m| m.len());
    stream.write_all(&encode(&Msg::FileInfo { size: file.as_ref().and(size) }))?;
    if let Some(mut f) = file {
        std::io::copy(&mut f, &mut stream)?;
    }
    Ok(())
}

fn download(s: &Arc<Shared>, addr: SocketAddr, canon: &str, dest: &Path) -> Result<()> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.write_all(&encode(&Msg::FileGet { path: canon.to_string() }))?;
    let Msg::FileInfo { size: Some(size) } = read_msg(&mut stream)? else {
        bail!("l'autre machine n'a pas ce fichier");
    };
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = dest.with_extension("part");
    let mut out = std::fs::File::create(&part)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut got = 0u64;
    while got < size {
        if s.stop.load(Ordering::Relaxed) {
            bail!("liaison coupée");
        }
        let n = stream.read(&mut buf)?;
        if n == 0 {
            bail!("copie interrompue");
        }
        out.write_all(&buf[..n])?;
        got += n as u64;
        s.progress.lock().unwrap().insert(canon.to_string(), (got, size));
        (s.repaint)();
    }
    out.flush()?;
    drop(out);
    std::fs::rename(&part, dest)?;
    Ok(())
}

/// Dossier où arrivent les médias copiés depuis les autres machines.
pub fn cache_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.unwrap_or_default().join("Library/Caches/OZ-Mapping/reseau")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("OZ-Mapping\\reseau")
    } else {
        home.unwrap_or_default().join(".cache/oz-mapping/reseau")
    }
}

/// Fichier local d'un média copié : un nom stable (même média → même fichier, réutilisé).
pub fn cache_path(canon: &str) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in canon.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
    }
    let name = canon.rsplit(['/', '\\']).next().unwrap_or("media");
    cache_dir().join(format!("{h:016x}-{name}"))
}

/// Un chemin de média (absolu, sur n'importe quel système) ?
pub fn looks_like_media(s: &str) -> bool {
    let b = s.as_bytes();
    let absolute = s.starts_with('/') || (b.len() > 3 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'));
    let ext = s.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    const EXT: &[&str] = &[
        "mp4", "mkv", "mov", "avi", "webm", "m4v", "mpg", "mpeg", "wmv", "flv", "gif", "jpg", "jpeg", "png", "heic", "heif",
        "webp", "bmp", "tif", "tiff", "svg",
    ];
    absolute && ext.is_some_and(|e| EXT.contains(&e.as_str())) && !s.contains('\n')
}

/// Applique `f` à chaque texte du JSON (le remplace s'il rend quelque chose).
pub fn map_strings(v: &mut Value, f: &mut impl FnMut(&str) -> Option<String>) {
    match v {
        Value::String(s) => {
            if let Some(n) = f(s) {
                *s = n;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| map_strings(x, f)),
        Value::Object(o) => o.values_mut().for_each(|x| map_strings(x, f)),
        _ => {}
    }
}

/// Morceaux du projet qui ont changé : un champ, ou un élément d'un champ (un calque, une
/// partie de la scène 3D) quand le reste n'a pas bougé.
pub fn diff(old: &Value, new: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let Some(new) = new.as_object() else { return out };
    for (k, n) in new {
        let o = old.get(k);
        if o == Some(n) {
            continue;
        }
        match (o, n) {
            (Some(Value::Object(a)), Value::Object(b)) if a.len() == b.len() && b.keys().all(|key| a.contains_key(key)) => {
                for (sk, bv) in b {
                    if a.get(sk) != Some(bv) {
                        out.push((format!("{k}/{sk}"), bv.clone()));
                    }
                }
            }
            (Some(Value::Array(a)), Value::Array(b)) if a.len() == b.len() => {
                for (i, (av, bv)) in a.iter().zip(b).enumerate() {
                    if av != bv {
                        out.push((format!("{k}/{i}"), bv.clone()));
                    }
                }
            }
            _ => out.push((k.clone(), n.clone())),
        }
    }
    out
}

/// Remplace le morceau `key` (« champ » ou « champ/sous-champ » ou « champ/3 »).
pub fn set_at(root: &mut Value, key: &str, v: Value) -> bool {
    let mut cur = root;
    let parts: Vec<&str> = key.split('/').collect();
    for (i, p) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let next = match cur {
            Value::Object(o) => {
                if last {
                    o.insert((*p).to_string(), v);
                    return true;
                }
                o.get_mut(*p)
            }
            Value::Array(a) => {
                let Ok(idx) = p.parse::<usize>() else { return false };
                if last {
                    return a.get_mut(idx).map(|slot| *slot = v).is_some();
                }
                a.get_mut(idx)
            }
            _ => None,
        };
        match next {
            Some(n) => cur = n,
            None => return false,
        }
    }
    false
}

/// Champ parent d'un morceau (« layers/3 » → « layers »).
pub fn parent(key: &str) -> Option<&str> {
    key.rsplit_once('/').map(|(p, _)| p)
}

/// Version actuelle d'un morceau : la sienne, ou celle de son champ s'il a été remplacé en entier
/// plus récemment.
pub fn version_of(vers: &HashMap<String, Ver>, key: &str) -> Ver {
    let own = vers.get(key).copied().unwrap_or_default();
    let up = parent(key).and_then(|p| vers.get(p)).copied().unwrap_or_default();
    own.max(up)
}

/// Adresses IPv4 de cette machine (pour les montrer à qui veut se lier à la main).
pub fn local_ips() -> Vec<IpAddr> {
    // Astuce sans paquet envoyé : l'adresse locale d'une « connexion » UDP vers l'extérieur.
    let mut out = Vec::new();
    if let Ok(s) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        && s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).is_ok()
        && let Ok(a) = s.local_addr()
    {
        out.push(a.ip());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diff_by_pieces() {
        let a = json!({"layers": [{"o": 1}, {"o": 2}], "leds": {"room": [1, 2], "fixtures": []}, "master": 1.0});
        let mut b = a.clone();
        b["layers"][1]["o"] = json!(3);
        b["leds"]["fixtures"] = json!([{"id": 1}]);
        let d = diff(&a, &b);
        assert_eq!(d.len(), 2);
        assert!(d.iter().any(|(k, v)| k == "layers/1" && v["o"] == 3));
        assert!(d.iter().any(|(k, _)| k == "leds/fixtures"));
        // Un calque en plus : tout le champ.
        b["layers"].as_array_mut().unwrap().push(json!({"o": 4}));
        assert!(diff(&a, &b).iter().any(|(k, _)| k == "layers"));
        // Appliquer les morceaux redonne le nouvel état.
        let mut c = a.clone();
        for (k, v) in diff(&a, &b) {
            assert!(set_at(&mut c, &k, v));
        }
        assert_eq!(c, b);
    }

    #[test]
    fn versions_follow_parent() {
        let mut vers = HashMap::new();
        vers.insert("layers/1".to_string(), (3, 1));
        vers.insert("layers".to_string(), (5, 2));
        assert_eq!(version_of(&vers, "layers/1"), (5, 2));
        assert_eq!(version_of(&vers, "master"), (0, 0));
    }

    #[test]
    fn media_paths() {
        assert!(looks_like_media("/Users/x/Vidéos/clip.MP4"));
        assert!(looks_like_media("C:\\clips\\a.mov"));
        assert!(!looks_like_media("clip.mp4"));
        assert!(!looks_like_media("Calque 1"));
        assert_ne!(cache_path("/a/clip.mp4"), cache_path("/b/clip.mp4"));
        assert!(cache_path("/a/clip.mp4").to_string_lossy().ends_with("-clip.mp4"));
    }

    #[test]
    fn frames_round_trip() {
        let msg = Msg::Patch { changes: vec![("master".into(), json!(0.5), (2, 9))] };
        let bytes = encode(&msg);
        let Msg::Patch { changes } = read_msg(&mut &bytes[..]).unwrap() else { panic!() };
        assert_eq!(changes[0].2, (2, 9));
    }

    /// Deux liaisons sur la même machine se trouvent et échangent un message.
    #[test]
    fn two_peers_link_up() {
        let settings = NetSettings { enabled: true, name: "a".into(), room: format!("test-{}", now_ms()), manual: vec![] };
        let a = Net::start(&settings, || {}).unwrap();
        let b = Net::start(&NetSettings { name: "b".into(), ..settings.clone() }, || {}).unwrap();
        // Sans compter sur la diffusion (machines de test parfois sans réseau) : adresse directe.
        b.connect(&format!("127.0.0.1:{}", a.port()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while a.peers().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(a.peers().len(), 1);
        // Renvoyé jusqu'à l'arrivée : si les deux se sont aussi trouvés par la diffusion (réseau
        // de la machine de test), l'une des deux connexions est fermée et ce qui y passait est
        // perdu — dans l'app, l'état est renvoyé en continu.
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut got = false;
        let mut sent = Instant::now() - Duration::from_secs(1);
        while !got && Instant::now() < deadline {
            if sent.elapsed() >= Duration::from_millis(400) {
                b.send_all(&Msg::Want);
                sent = Instant::now();
            }
            if let Ok(Event::Msg { msg: Msg::Want, .. }) = a.events.recv_timeout(Duration::from_millis(100)) {
                got = true;
            }
        }
        assert!(got);
    }
    /// Un média est copié d'une machine à l'autre, et seulement s'il fait partie du projet.
    #[test]
    fn media_copy() {
        let dir = std::env::temp_dir().join(format!("oz-net-test-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("clip.mp4");
        std::fs::write(&src, vec![7u8; 3 << 20]).unwrap();
        let settings = NetSettings { enabled: true, name: "a".into(), room: format!("copie-{}", now_ms()), manual: vec![] };
        let a = Net::start(&settings, || {}).unwrap();
        let b = Net::start(&NetSettings { name: "b".into(), ..settings.clone() }, || {}).unwrap();
        a.set_files(HashMap::from([("/autre/clip.mp4".to_string(), src.clone())]));
        b.connect(&format!("127.0.0.1:{}", a.port()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while b.peers().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let wait_file = |canon: &str, dest: PathBuf| {
            b.fetch(a.id(), canon.to_string(), dest);
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if let Ok(Event::File { result, .. }) = b.events.recv_timeout(Duration::from_millis(100)) {
                    return result;
                }
            }
            Err("délai dépassé".into())
        };
        let dest = dir.join("copie.mp4");
        assert_eq!(wait_file("/autre/clip.mp4", dest.clone()).unwrap(), dest);
        assert_eq!(std::fs::read(&dest).unwrap(), std::fs::read(&src).unwrap());
        // Un fichier hors du projet n'est jamais envoyé.
        assert!(wait_file(&src.to_string_lossy(), dir.join("non.mp4")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
