//! Mesure de la charge CPU, en arrière-plan (une mesure par seconde).
//!
//! Le gros du travail est fait par les processus ffmpeg lancés par l'app (un par vidéo) :
//! la charge de l'app inclut donc ses processus enfants.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuStats {
    /// App + ffmpeg, en % de la capacité totale de la machine (0..100).
    pub app: f32,
    /// Charge globale de la machine (0..100).
    pub system: f32,
    /// Nombre de processus ffmpeg en cours.
    pub decoders: usize,
    pub cores: usize,
}

pub fn spawn_monitor() -> Arc<Mutex<CpuStats>> {
    let stats = Arc::new(Mutex::new(CpuStats::default()));
    let shared = stats.clone();
    std::thread::Builder::new()
        .name("cpu-monitor".into())
        .spawn(move || {
            let me = Pid::from_u32(std::process::id());
            let mut sys = System::new();
            loop {
                sys.refresh_cpu_usage();
                sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cpu());
                let cores = sys.cpus().len().max(1);
                let mut total = 0.0;
                let mut decoders = 0;
                for (pid, p) in sys.processes() {
                    let child = p.parent() == Some(me);
                    if *pid == me || child {
                        total += p.cpu_usage();
                        decoders += child as usize;
                    }
                }
                *shared.lock().unwrap() = CpuStats {
                    // sysinfo compte 100 % par cœur : on ramène à la capacité totale.
                    app: (total / cores as f32).clamp(0.0, 100.0),
                    system: sys.global_cpu_usage().clamp(0.0, 100.0),
                    decoders,
                    cores,
                };
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("thread cpu");
    stats
}
