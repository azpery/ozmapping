//! Enregistrement de la sortie dans un fichier vidéo (MP4, H.264) avec ffmpeg.
//!
//! Le rendu envoie des images RGBA brutes (lues sur le GPU, donc à l'envers) ; un thread les
//! écrit sur l'entrée de ffmpeg. L'horloge est celle du temps réel : si l'application rend
//! moins d'images que la cadence demandée, la dernière image est répétée, pour que la vidéo
//! dure exactement le temps enregistré.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::{Context as _, Result};

use crate::video;

/// Définitions proposées (hauteur ; la largeur suit les proportions de la sortie).
pub const HEIGHTS: [u32; 3] = [720, 1080, 2160];
pub const FPS: [u32; 3] = [25, 30, 60];

pub struct Recorder {
    pub path: PathBuf,
    pub size: (u32, u32),
    pub fps: u32,
    /// Arrêt automatique après cette durée (0 : jusqu'à l'arrêt à la main).
    pub max_secs: f32,
    started: Instant,
    /// Images envoyées à ffmpeg (répétitions comprises) et images perdues (ffmpeg trop lent).
    pub frames: u64,
    pub dropped: u64,
    tx: Option<SyncSender<Arc<Vec<u8>>>>,
    thread: Option<JoinHandle<Result<()>>>,
    error: Arc<Mutex<Option<String>>>,
}

impl Recorder {
    pub fn start(path: &Path, size: (u32, u32), fps: u32, max_secs: f32) -> Result<Self> {
        // H.264 en 4:2:0 : largeur et hauteur paires.
        let size = (size.0.max(2) & !1, size.1.max(2) & !1);
        let mut child = video::tool_command("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", size.0, size.1)])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            // Les images lues sur le GPU arrivent la tête en bas.
            .args(["-vf", "vflip"])
            .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p"])
            .args(["-movflags", "+faststart"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("lancement de ffmpeg")?;
        let stdin = child.stdin.take().context("entrée de ffmpeg")?;
        // Quelques images d'avance au plus : au-delà, on en perd plutôt que de figer l'affichage.
        let (tx, rx) = mpsc::sync_channel::<Arc<Vec<u8>>>(6);
        let error = Arc::new(Mutex::new(None));
        let thread_error = error.clone();
        let thread = std::thread::spawn(move || write_frames(child, stdin, rx, thread_error));
        Ok(Self {
            path: path.to_path_buf(),
            size,
            fps,
            max_secs,
            started: Instant::now(),
            frames: 0,
            dropped: 0,
            tx: Some(tx),
            thread: Some(thread),
            error,
        })
    }

    pub fn elapsed(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    /// Nombre d'images que la vidéo devrait contenir à cet instant.
    pub fn frames_due(&self) -> u64 {
        let secs = if self.max_secs > 0.0 { self.elapsed().min(self.max_secs) } else { self.elapsed() };
        (secs as f64 * self.fps as f64).floor() as u64 + 1
    }

    /// Durée maximale atteinte : il faut arrêter.
    pub fn done(&self) -> bool {
        self.max_secs > 0.0 && self.elapsed() >= self.max_secs
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    /// Ajoute une image (RGBA, `size`), répétée autant que nécessaire pour rattraper l'horloge.
    pub fn push(&mut self, frame: Vec<u8>) {
        let Some(tx) = &self.tx else {
            return;
        };
        let frame = Arc::new(frame);
        let due = self.frames_due();
        while self.frames < due {
            match tx.try_send(frame.clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => self.dropped += 1,
                Err(TrySendError::Disconnected(_)) => return,
            }
            self.frames += 1;
        }
    }

    /// Termine le fichier (ffmpeg finit d'encoder). Peut prendre un instant.
    pub fn finish(mut self) -> Result<PathBuf> {
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| anyhow::anyhow!("le thread d'écriture s'est arrêté"))??;
        }
        Ok(self.path.clone())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn write_frames(
    child: Child,
    mut stdin: std::process::ChildStdin,
    rx: Receiver<Arc<Vec<u8>>>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<()> {
    for frame in rx {
        if let Err(e) = stdin.write_all(&frame) {
            *error.lock().unwrap() = Some(format!("écriture vers ffmpeg : {e}"));
            break;
        }
    }
    drop(stdin); // fin du flux : ffmpeg termine le fichier
    let out = child.wait_with_output().context("fin de ffmpeg")?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        let msg = if msg.is_empty() { format!("ffmpeg : {}", out.status) } else { format!("ffmpeg : {msg}") };
        *error.lock().unwrap() = Some(msg.clone());
        anyhow::bail!(msg);
    }
    Ok(())
}

/// Nom de fichier proposé : « OZ Mapping 2026-10-27 22-30-05.mp4 ».
pub fn default_file_name() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Date UTC simple (sans dépendance) : suffisante pour un nom de fichier unique.
    let days = secs / 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    let s = secs % 86_400;
    format!("OZ Mapping {y:04}-{m:02}-{d:02} {:02}-{:02}-{:02}.mp4", s / 3600, (s / 60) % 60, s % 60)
}

/// Jours depuis 1970 → (année, mois, jour), calendrier grégorien.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_right() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_753), (2026, 10, 27));
    }

    #[test]
    fn records_a_short_mp4() {
        let dir = std::env::temp_dir().join(format!("ozrec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.mp4");
        let mut rec = match Recorder::start(&path, (64, 36), 25, 0.0) {
            Ok(r) => r,
            Err(_) => return, // pas de ffmpeg sur cette machine
        };
        // Une image tout de suite, puis 0,2 s plus tard : l'horloge en demande 5 de plus
        // (25 images/s), la seconde est donc répétée.
        rec.push(vec![40; 64 * 36 * 4]);
        std::thread::sleep(std::time::Duration::from_millis(200));
        rec.push(vec![200; 64 * 36 * 4]);
        assert!(rec.frames >= 5, "{} images", rec.frames);
        rec.finish().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
