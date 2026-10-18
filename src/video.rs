//! Décodage vidéo via un sous-processus `ffmpeg` qui sort des frames RGBA brutes.
//!
//! Chaque `VideoPlayer` possède un thread qui lit les frames sur le pipe de ffmpeg,
//! les cadence selon le fps de la vidéo (et la vitesse / pause demandées) et publie
//! la dernière frame dans un `FrameSlot` partagé avec le rendu.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

/// Taille maximale de décodage pour la sortie (les vidéos plus grandes sont réduites).
pub const OUTPUT_MAX: (u32, u32) = (1920, 1080);
/// Taille maximale de décodage pour la pré-visualisation.
pub const PREVIEW_MAX: (u32, u32) = (640, 360);

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

/// Trouve un exécutable ffmpeg/ffprobe, y compris quand l'app est lancée hors d'un shell
/// (sur macOS le PATH ne contient alors pas Homebrew).
pub fn tool_path(name: &str) -> PathBuf {
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        let p = Path::new(dir).join(name);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(name)
}

pub fn probe(path: &Path) -> Result<VideoInfo> {
    let out = Command::new(tool_path("ffprobe"))
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

/// État partagé entre l'UI et le thread de lecture.
struct Shared {
    stop: AtomicBool,
    paused: AtomicBool,
    looping: AtomicBool,
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
    pub fn open(
        path: &Path,
        looping: bool,
        max_size: (u32, u32),
        repaint: impl Fn() + Send + 'static,
    ) -> Result<Self> {
        let info = probe(path)?;
        let slot: FrameSlot = Arc::new(Mutex::new(None));
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            looping: AtomicBool::new(looping),
            ended: AtomicBool::new(false),
            speed: AtomicU32::new(1.0f32.to_bits()),
            position: AtomicU64::new(0f64.to_bits()),
            seek: Mutex::new(None),
            child: Mutex::new(None),
            error: Mutex::new(None),
        });

        let thread_path = path.to_path_buf();
        let thread_info = info.clone();
        let thread_slot = slot.clone();
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("video-decoder".into())
            .spawn(move || {
                if let Err(e) = decode_loop(&thread_path, &thread_info, max_size, &thread_slot, &thread_shared, &repaint) {
                    *thread_shared.error.lock().unwrap() = Some(format!("{e:#}"));
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

fn spawn_ffmpeg(path: &Path, start: f64, fps: f64, w: u32, h: u32) -> Result<Child> {
    let mut cmd = Command::new(tool_path("ffmpeg"));
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-hwaccel", "auto"]);
    if start > 0.0 {
        cmd.args(["-ss", &format!("{start:.3}")]);
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

fn decode_loop(
    path: &Path,
    info: &VideoInfo,
    max_size: (u32, u32),
    slot: &FrameSlot,
    shared: &Shared,
    repaint: &dyn Fn(),
) -> Result<()> {
    let (w, h) = output_size(info.width, info.height, max_size);
    let frame_bytes = (w * h * 4) as usize;
    let frame_dur = 1.0 / info.fps;
    let mut buf = vec![0u8; frame_bytes];
    let mut start = 0.0f64;

    'outer: while !shared.stop.load(Ordering::Relaxed) {
        let mut child = spawn_ffmpeg(path, start, info.fps, w, h)?;
        let mut stdout = child.stdout.take().context("pas de stdout ffmpeg")?;
        *shared.child.lock().unwrap() = Some(child);
        shared.ended.store(false, Ordering::Relaxed);

        let mut frames_read: u64 = 0;
        let mut deadline = Instant::now();

        loop {
            if shared.stop.load(Ordering::Relaxed) {
                break 'outer;
            }
            if let Some(pos) = shared.seek.lock().unwrap().take() {
                start = pos;
                kill_child(shared);
                continue 'outer;
            }
            if shared.paused.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                deadline = Instant::now();
                continue;
            }

            if stdout.read_exact(&mut buf).is_err() {
                break; // fin du fichier (ou ffmpeg tué pour un seek)
            }
            frames_read += 1;
            let pos = start + frames_read as f64 * frame_dur;
            shared.position.store(pos.to_bits(), Ordering::Relaxed);

            // Publie la frame en recyclant l'ancien buffer.
            {
                let mut guard = slot.lock().unwrap();
                match guard.as_mut() {
                    Some(f) if f.data.len() == frame_bytes => {
                        std::mem::swap(&mut f.data, &mut buf);
                        f.id = NEXT_FRAME_ID.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        *guard = Some(Frame {
                            data: std::mem::replace(&mut buf, vec![0u8; frame_bytes]),
                            width: w,
                            height: h,
                            id: NEXT_FRAME_ID.fetch_add(1, Ordering::Relaxed),
                        });
                    }
                }
            }
            repaint();

            // Cadence temps réel (avec vitesse variable).
            let speed = f32::from_bits(shared.speed.load(Ordering::Relaxed)).max(0.05) as f64;
            deadline += Duration::from_secs_f64(frame_dur / speed);
            let now = Instant::now();
            if deadline > now {
                std::thread::sleep(deadline - now);
            } else if now - deadline > Duration::from_millis(250) {
                deadline = now; // trop en retard : on ne cherche pas à rattraper
            }
        }

        kill_child(shared);
        if let Some(pos) = shared.seek.lock().unwrap().take() {
            start = pos;
            continue;
        }
        if frames_read == 0 && start == 0.0 {
            bail!("ffmpeg n'a produit aucune image");
        }
        if shared.looping.load(Ordering::Relaxed) {
            start = 0.0;
        } else {
            // Fin de clip : on garde la dernière image et on attend un seek / l'arrêt.
            shared.ended.store(true, Ordering::Relaxed);
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    break 'outer;
                }
                if let Some(pos) = shared.seek.lock().unwrap().take() {
                    start = pos;
                    continue 'outer;
                }
                if shared.looping.load(Ordering::Relaxed) {
                    start = 0.0;
                    continue 'outer;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    kill_child(shared);
    Ok(())
}

fn kill_child(shared: &Shared) {
    if let Some(mut child) = shared.child.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Extrait une miniature RGBA (w×h, avec bandes noires) vers 10 % de la vidéo.
pub fn thumbnail(path: &Path, w: u32, h: u32) -> Result<Vec<u8>> {
    let info = probe(path)?;
    let at = info.duration * 0.1;
    let out = Command::new(tool_path("ffmpeg"))
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
