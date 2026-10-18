//! Décodage vidéo via un sous-processus `ffmpeg` qui sort des frames RGBA brutes.
//!
//! Chaque `VideoPlayer` possède un thread qui lit les frames sur le pipe de ffmpeg,
//! les cadence selon le fps de la vidéo (et la vitesse / pause demandées) et publie
//! la dernière frame dans un `FrameSlot` partagé avec le rendu.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

/// Taille maximale de décodage pour la sortie (les vidéos plus grandes sont réduites).
pub const OUTPUT_MAX: (u32, u32) = (1920, 1080);
/// Taille maximale de décodage pour la pré-visualisation.
pub const PREVIEW_MAX: (u32, u32) = (640, 360);
/// Lecture à l'envers : taille maximale (les morceaux sont gardés en mémoire) et durée d'un morceau.
const REVERSE_MAX: (u32, u32) = (1280, 720);
const REVERSE_CHUNK_SECS: f64 = 0.4;

static NEXT_FRAME_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PLAYER_ID: AtomicU64 = AtomicU64::new(1);

pub struct Frame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Identifiant unique global : permet au rendu de savoir s'il doit ré-uploader.
    pub id: u64,
}

pub type FrameSlot = Arc<Mutex<Option<Frame>>>;

#[derive(Clone, Debug)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
}

/// Trouve un exécutable ffmpeg/ffprobe : d'abord à côté de l'app (version livrée avec elle),
/// puis dans les dossiers d'installation courants, sinon dans le PATH.
/// (Lancée depuis le Finder, l'app n'a pas Homebrew dans son PATH.)
pub fn tool_path(name: &str) -> PathBuf {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(exe_dir.join("ffmpeg").join("bin"));
        dirs.push(exe_dir.join("ffmpeg"));
        dirs.push(exe_dir);
    }
    if !cfg!(windows) {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    }
    dirs.into_iter()
        .map(|d| d.join(&file))
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(file))
}

/// Commande pour ffmpeg/ffprobe. Sous Windows, sans ouvrir de fenêtre de console.
pub fn tool_command(name: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(tool_path(name));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Informations sur une vidéo (mises en cache : le mix auto change souvent de clip,
/// et relancer ffprobe à chaque fois ferait saccader l'interface).
pub fn probe(path: &Path) -> Result<VideoInfo> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, VideoInfo>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(info) = cache.lock().unwrap().get(path) {
        return Ok(info.clone());
    }
    let info = probe_uncached(path)?;
    cache.lock().unwrap().insert(path.to_path_buf(), info.clone());
    Ok(info)
}

fn probe_uncached(path: &Path) -> Result<VideoInfo> {
    let out = tool_command("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,avg_frame_rate,r_frame_rate:stream_side_data=rotation:stream_tags=rotate:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .context("impossible de lancer ffprobe (ffmpeg est-il installé ?)")?;
    if !out.status.success() {
        bail!("ffprobe : {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let stream = json["streams"]
        .get(0)
        .context("aucune piste vidéo dans ce fichier")?;

    let mut width = stream["width"].as_u64().context("largeur inconnue")? as u32;
    let mut height = stream["height"].as_u64().context("hauteur inconnue")? as u32;

    // ffmpeg applique automatiquement la rotation : on inverse w/h si besoin.
    let rotation = stream["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
        .or_else(|| stream["tags"]["rotate"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0.0);
    if (rotation.abs() as i64) % 180 == 90 {
        std::mem::swap(&mut width, &mut height);
    }

    let parse_rate = |v: &serde_json::Value| -> Option<f64> {
        let s = v.as_str()?;
        let (n, d) = s.split_once('/')?;
        let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
        (n > 0.0 && d > 0.0).then_some(n / d)
    };
    let fps = parse_rate(&stream["avg_frame_rate"])
        .or_else(|| parse_rate(&stream["r_frame_rate"]))
        .filter(|f| *f > 1.0 && *f < 241.0)
        .unwrap_or(30.0);
    let duration = json["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    Ok(VideoInfo {
        width,
        height,
        fps,
        duration,
    })
}

/// Taille de décodage : on garde le ratio, on limite à `max`, dimensions paires.
fn output_size(w: u32, h: u32, max: (u32, u32)) -> (u32, u32) {
    let scale = (max.0 as f64 / w as f64).min(max.1 as f64 / h as f64).min(1.0);
    let even = |v: f64| ((v.round() as u32) / 2 * 2).max(2);
    (even(w as f64 * scale), even(h as f64 * scale))
}

/// Réglages de départ d'un lecteur.
#[derive(Clone, Copy, Debug)]
pub struct PlayOptions {
    pub looping: bool,
    pub max_size: (u32, u32),
    /// Position de départ, en secondes.
    pub start: f64,
    pub reverse: bool,
    pub speed: f32,
}

impl PlayOptions {
    pub fn new(looping: bool, max_size: (u32, u32)) -> Self {
        Self {
            looping,
            max_size,
            start: 0.0,
            reverse: false,
            speed: 1.0,
        }
    }
}

/// État partagé entre l'UI et le thread de lecture.
struct Shared {
    stop: AtomicBool,
    paused: AtomicBool,
    looping: AtomicBool,
    reverse: AtomicBool,
    ended: AtomicBool,
    speed: AtomicU32,    // f32 bits
    position: AtomicU64, // f64 bits, secondes
    seek: Mutex<Option<f64>>,
    child: Mutex<Option<Child>>,
    error: Mutex<Option<String>>,
}

pub struct VideoPlayer {
    /// Identifiant unique : sert de clé à la texture GPU du lecteur.
    pub uid: u64,
    pub path: PathBuf,
    pub info: VideoInfo,
    pub slot: FrameSlot,
    shared: Arc<Shared>,
}

impl VideoPlayer {
    pub fn open(path: &Path, opts: PlayOptions, repaint: impl Fn() + Send + 'static) -> Result<Self> {
        let info = probe(path)?;
        let start = if info.duration > 0.0 { opts.start.clamp(0.0, info.duration) } else { opts.start.max(0.0) };
        let slot: FrameSlot = Arc::new(Mutex::new(None));
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            looping: AtomicBool::new(opts.looping),
            reverse: AtomicBool::new(opts.reverse),
            ended: AtomicBool::new(false),
            speed: AtomicU32::new(opts.speed.to_bits()),
            position: AtomicU64::new(start.to_bits()),
            seek: Mutex::new(None),
            child: Mutex::new(None),
            error: Mutex::new(None),
        });

        let decoder = Decoder {
            path: path.to_path_buf(),
            info: info.clone(),
            max_size: opts.max_size,
            slot: slot.clone(),
            shared: shared.clone(),
            repaint: Box::new(repaint),
        };
        std::thread::Builder::new()
            .name("video-decoder".into())
            .spawn(move || {
                if let Err(e) = decoder.run(start) {
                    *decoder.shared.error.lock().unwrap() = Some(format!("{e:#}"));
                }
            })?;

        Ok(Self {
            uid: NEXT_PLAYER_ID.fetch_add(1, Ordering::Relaxed),
            path: path.to_path_buf(),
            info,
            slot,
            shared,
        })
    }

    pub fn has_frame(&self) -> bool {
        self.slot.lock().unwrap().is_some()
    }

    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    pub fn speed(&self) -> f32 {
        f32::from_bits(self.shared.speed.load(Ordering::Relaxed))
    }

    pub fn set_speed(&self, speed: f32) {
        self.shared.speed.store(speed.to_bits(), Ordering::Relaxed);
    }

    pub fn set_looping(&self, looping: bool) {
        self.shared.looping.store(looping, Ordering::Relaxed);
    }

    pub fn is_reverse(&self) -> bool {
        self.shared.reverse.load(Ordering::Relaxed)
    }

    /// Lecture à l'envers (true) ou à l'endroit, depuis la position actuelle.
    pub fn set_reverse(&self, reverse: bool) {
        self.shared.reverse.store(reverse, Ordering::Relaxed);
    }

    pub fn position(&self) -> f64 {
        f64::from_bits(self.shared.position.load(Ordering::Relaxed))
    }

    pub fn ended(&self) -> bool {
        self.shared.ended.load(Ordering::Relaxed)
    }

    pub fn seek(&self, secs: f64) {
        *self.shared.seek.lock().unwrap() = Some(secs.max(0.0));
        // Débloque une éventuelle lecture en cours sur le pipe.
        if let Some(child) = self.shared.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.shared.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_ffmpeg(path: &Path, start: f64, duration: Option<f64>, fps: f64, w: u32, h: u32) -> Result<Child> {
    let mut cmd = tool_command("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-hwaccel", "auto"]);
    if start > 0.0 {
        cmd.args(["-ss", &format!("{start:.3}")]);
    }
    if let Some(d) = duration {
        cmd.args(["-t", &format!("{d:.3}")]);
    }
    cmd.arg("-i")
        .arg(path)
        .args([
            "-an",
            "-sn",
            "-vf",
            &format!("fps={fps:.4},scale={w}:{h}:flags=bilinear,format=rgba"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd.spawn().context("impossible de lancer ffmpeg (est-il installé ?)")
}

/// Décode toutes les images d'un intervalle (pour la lecture à l'envers).
fn decode_chunk(path: &Path, start: f64, len: f64, fps: f64, w: u32, h: u32) -> Result<Vec<Vec<u8>>> {
    // `-t` est placé avant `-i` (limite la lecture), ce qui reste exact avec `-ss` en entrée.
    let mut child = spawn_ffmpeg(path, start, Some(len), fps, w, h)?;
    let mut stdout = child.stdout.take().context("pas de stdout ffmpeg")?;
    let mut frames = Vec::new();
    loop {
        let mut buf = vec![0u8; (w * h * 4) as usize];
        if stdout.read_exact(&mut buf).is_err() {
            break;
        }
        frames.push(buf);
    }
    let _ = child.wait();
    Ok(frames)
}

/// Suite de la lecture après une interruption.
enum Next {
    /// Reprendre à cette position (le sens est relu dans l'état partagé).
    At(f64),
    Stop,
}

/// Cadence temps réel, vitesse variable comprise.
struct Pacer {
    deadline: Instant,
}

impl Pacer {
    fn new() -> Self {
        Self { deadline: Instant::now() }
    }

    fn reset(&mut self) {
        self.deadline = Instant::now();
    }

    fn wait(&mut self, frame_dur: f64, speed: f32) {
        self.deadline += Duration::from_secs_f64(frame_dur / speed.max(0.05) as f64);
        let now = Instant::now();
        if self.deadline > now {
            std::thread::sleep(self.deadline - now);
        } else if now - self.deadline > Duration::from_millis(250) {
            self.deadline = now; // trop en retard : on ne cherche pas à rattraper
        }
    }
}

struct Decoder {
    path: PathBuf,
    info: VideoInfo,
    max_size: (u32, u32),
    slot: FrameSlot,
    shared: Arc<Shared>,
    repaint: Box<dyn Fn() + Send>,
}

impl Decoder {
    fn run(&self, start: f64) -> Result<()> {
        let mut pos = start;
        loop {
            let next = if self.shared.reverse.load(Ordering::Relaxed) {
                self.play_reverse(pos)?
            } else {
                self.play_forward(pos)?
            };
            match next {
                Next::At(p) => pos = p,
                Next::Stop => break,
            }
        }
        self.kill_child();
        Ok(())
    }

    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
    }

    fn speed(&self) -> f32 {
        f32::from_bits(self.shared.speed.load(Ordering::Relaxed))
    }

    fn set_position(&self, pos: f64) {
        self.shared.position.store(pos.to_bits(), Ordering::Relaxed);
    }

    fn take_seek(&self) -> Option<f64> {
        self.shared.seek.lock().unwrap().take()
    }

    fn kill_child(&self) {
        if let Some(mut child) = self.shared.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Publie une image ; renvoie l'ancien buffer pour le recycler s'il a la bonne taille.
    fn publish(&self, data: Vec<u8>, w: u32, h: u32) -> Option<Vec<u8>> {
        let old = {
            let mut guard = self.slot.lock().unwrap();
            let id = NEXT_FRAME_ID.fetch_add(1, Ordering::Relaxed);
            guard.replace(Frame { data, width: w, height: h, id })
        };
        (self.repaint)();
        old.map(|f| f.data).filter(|d| d.len() == (w * h * 4) as usize)
    }

    fn play_forward(&self, start: f64) -> Result<Next> {
        let (w, h) = output_size(self.info.width, self.info.height, self.max_size);
        let frame_bytes = (w * h * 4) as usize;
        let frame_dur = 1.0 / self.info.fps;
        let mut child = spawn_ffmpeg(&self.path, start, None, self.info.fps, w, h)?;
        let mut stdout = child.stdout.take().context("pas de stdout ffmpeg")?;
        *self.shared.child.lock().unwrap() = Some(child);
        self.shared.ended.store(false, Ordering::Relaxed);

        let mut buf = vec![0u8; frame_bytes];
        let mut frames_read: u64 = 0;
        let mut pos = start;
        let mut pacer = Pacer::new();
        loop {
            if self.stopped() {
                return Ok(Next::Stop);
            }
            if let Some(p) = self.take_seek() {
                self.kill_child();
                return Ok(Next::At(p));
            }
            if self.shared.reverse.load(Ordering::Relaxed) {
                self.kill_child();
                return Ok(Next::At(pos));
            }
            if self.shared.paused.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                pacer.reset();
                continue;
            }
            if stdout.read_exact(&mut buf).is_err() {
                break; // fin du fichier (ou ffmpeg tué pour un seek)
            }
            frames_read += 1;
            pos = start + frames_read as f64 * frame_dur;
            self.set_position(pos);
            let data = std::mem::take(&mut buf);
            buf = self.publish(data, w, h).unwrap_or_else(|| vec![0u8; frame_bytes]);
            pacer.wait(frame_dur, self.speed());
        }

        self.kill_child();
        if let Some(p) = self.take_seek() {
            return Ok(Next::At(p));
        }
        if frames_read == 0 && start == 0.0 {
            bail!("ffmpeg n'a produit aucune image");
        }
        if self.shared.looping.load(Ordering::Relaxed) {
            return Ok(Next::At(0.0));
        }
        Ok(self.wait_ended(pos))
    }

    /// Lecture à l'envers : on décode de petits morceaux de la fin vers le début et on joue
    /// chaque morceau à rebours, pendant que le morceau précédent est décodé en parallèle.
    fn play_reverse(&self, start: f64) -> Result<Next> {
        let (w, h) = output_size(self.info.width, self.info.height, (
            self.max_size.0.min(REVERSE_MAX.0),
            self.max_size.1.min(REVERSE_MAX.1),
        ));
        let fps = self.info.fps;
        let frame_dur = 1.0 / fps;
        let duration = self.info.duration;
        self.shared.ended.store(false, Ordering::Relaxed);

        let end = if start <= frame_dur && duration > 0.0 && self.shared.looping.load(Ordering::Relaxed) {
            duration
        } else {
            start
        };
        if end <= frame_dur {
            return Ok(self.wait_ended(0.0));
        }
        let decode = |end: f64| {
            let s = (end - REVERSE_CHUNK_SECS).max(0.0);
            (s, decode_chunk(&self.path, s, end - s, fps, w, h))
        };
        let (mut chunk_start, mut frames) = {
            let (s, f) = decode(end);
            (s, f?)
        };
        let mut pacer = Pacer::new();

        loop {
            // Morceau suivant (plus tôt dans la vidéo), décodé pendant la lecture de celui-ci.
            let looping = self.shared.looping.load(Ordering::Relaxed);
            let next_end = if chunk_start > frame_dur {
                Some(chunk_start)
            } else if looping && duration > 0.0 {
                Some(duration)
            } else {
                None
            };
            let path = self.path.clone();
            let prefetch = next_end.map(|e| {
                std::thread::spawn(move || {
                    let s = (e - REVERSE_CHUNK_SECS).max(0.0);
                    (s, decode_chunk(&path, s, e - s, fps, w, h))
                })
            });

            let n = frames.len();
            for (k, data) in frames.drain(..).rev().enumerate() {
                loop {
                    if self.stopped() {
                        return Ok(Next::Stop);
                    }
                    if let Some(p) = self.take_seek() {
                        return Ok(Next::At(p));
                    }
                    let pos = chunk_start + (n - 1 - k) as f64 * frame_dur;
                    if !self.shared.reverse.load(Ordering::Relaxed) {
                        return Ok(Next::At(pos));
                    }
                    if self.shared.paused.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(10));
                        pacer.reset();
                        continue;
                    }
                    break;
                }
                self.set_position(chunk_start + (n - 1 - k) as f64 * frame_dur);
                self.publish(data, w, h);
                pacer.wait(frame_dur, self.speed());
            }

            match prefetch {
                Some(handle) => {
                    let (s, f) = handle.join().map_err(|_| anyhow::anyhow!("décodage inverse interrompu"))?;
                    let f = f?;
                    chunk_start = s;
                    frames = f;
                    if frames.is_empty() {
                        return Ok(self.wait_ended(chunk_start));
                    }
                }
                None => return Ok(self.wait_ended(0.0)),
            }
        }
    }

    /// Fin de clip sans boucle : on garde la dernière image et on attend une consigne.
    fn wait_ended(&self, pos: f64) -> Next {
        self.shared.ended.store(true, Ordering::Relaxed);
        let reverse = self.shared.reverse.load(Ordering::Relaxed);
        loop {
            if self.stopped() {
                return Next::Stop;
            }
            if let Some(p) = self.take_seek() {
                return Next::At(p);
            }
            if self.shared.reverse.load(Ordering::Relaxed) != reverse {
                return Next::At(pos);
            }
            if self.shared.looping.load(Ordering::Relaxed) {
                return Next::At(if reverse { self.info.duration } else { 0.0 });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Extrait une miniature RGBA (w×h, avec bandes noires) vers 10 % de la vidéo.
pub fn thumbnail(path: &Path, w: u32, h: u32) -> Result<Vec<u8>> {
    let info = probe(path)?;
    let at = info.duration * 0.1;
    let out = tool_command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &format!("{at:.3}")])
        .arg("-i")
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-an",
            "-vf",
            &format!(
                "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,format=rgba"
            ),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .context("impossible de lancer ffmpeg")?;
    let expected = (w * h * 4) as usize;
    if out.stdout.len() < expected {
        bail!("miniature indisponible");
    }
    Ok(out.stdout[..expected].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vidéo de test de 3 s générée par ffmpeg (test ignoré si ffmpeg est absent).
    fn test_video() -> Option<PathBuf> {
        let path = std::env::temp_dir().join(format!("ozmapping-test-{}.mp4", std::process::id()));
        if !path.exists() {
            let ok = tool_command("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30", "-t", "3"])
                .arg(&path)
                .status()
                .ok()?
                .success();
            if !ok {
                return None;
            }
        }
        Some(path)
    }

    fn play(path: &Path, opts: PlayOptions, secs: f64) -> (VideoPlayer, f64) {
        let player = VideoPlayer::open(path, opts, || {}).unwrap();
        std::thread::sleep(Duration::from_secs_f64(secs));
        let pos = player.position();
        (player, pos)
    }

    #[test]
    fn plays_forward_and_reverse() {
        let Some(path) = test_video() else { return };
        let mut opts = PlayOptions::new(true, OUTPUT_MAX);

        opts.start = 0.5;
        let (fwd, pos) = play(&path, opts, 1.0);
        assert!(fwd.has_frame());
        assert!(pos > 0.9, "à l'endroit, la position doit avancer : {pos}");

        opts.start = 2.5;
        opts.reverse = true;
        let (rev, pos) = play(&path, opts, 1.2);
        assert!(rev.has_frame());
        assert!(pos < 2.2, "à l'envers, la position doit reculer : {pos}");
        assert!(rev.error().is_none(), "{:?}", rev.error());

        // Retour à l'endroit en cours de lecture.
        let before = rev.position();
        rev.set_reverse(false);
        std::thread::sleep(Duration::from_millis(800));
        assert!(rev.position() > before, "{} -> {}", before, rev.position());
    }

    #[test]
    fn reverse_loops_back_to_the_end() {
        let Some(path) = test_video() else { return };
        let mut opts = PlayOptions::new(true, OUTPUT_MAX);
        opts.start = 0.3;
        opts.reverse = true;
        opts.speed = 2.0;
        let (p, pos) = play(&path, opts, 0.9);
        // 0,3 s à l'envers en ×2 → repart de la fin (3 s) et redescend.
        assert!(pos > 1.5, "après le début, la lecture inverse doit repartir de la fin : {pos}");
        assert!(p.error().is_none());
    }
}
