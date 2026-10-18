//! Fichier projet `.ozmap` (JSON) : médias, calques, presets et réglages.
//!
//! Les chemins des médias situés à côté du projet sont enregistrés en relatif,
//! pour pouvoir déplacer ou copier le dossier complet.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::automix::AutoMix;
use crate::effects::{AutoFx, BlendMode, Effects, Preset};
use crate::surface::{Mask, Warp};

pub const EXTENSION: &str = "ozmap";
const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerConfig {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub effects: Effects,
    /// Navigation automatique entre les effets.
    pub auto: AutoFx,
    /// Mix auto entre plusieurs vidéos.
    pub mix: AutoMix,
    pub warp: Warp,
    pub mask: Mask,
    /// Vidéo chargée sur ce calque (relancée à l'ouverture du projet).
    pub clip: Option<PathBuf>,
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            name: "Calque 1".into(),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            effects: Effects::default(),
            auto: AutoFx::default(),
            mix: AutoMix::default(),
            warp: Warp::default(),
            mask: Mask::default(),
            clip: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub media: Vec<PathBuf>,
    /// Du bas vers le haut.
    pub layers: Vec<LayerConfig>,
    pub presets: Vec<Preset>,
    pub fade_secs: f32,
    pub preset_secs: f32,
    pub looping: bool,
    pub master: f32,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: VERSION,
            media: Vec::new(),
            layers: vec![LayerConfig::default()],
            presets: Vec::new(),
            fade_secs: 1.0,
            preset_secs: 0.5,
            looping: true,
            master: 1.0,
        }
    }
}

impl Project {
    pub fn save(&self, path: &Path) -> Result<()> {
        let base = path.parent().unwrap_or(Path::new(""));
        let mut p = self.clone();
        p.version = VERSION;
        for m in &mut p.media {
            *m = relative_to(m, base);
        }
        for l in &mut p.layers {
            if let Some(c) = &mut l.clip {
                *c = relative_to(c, base);
            }
        }
        let json = serde_json::to_string_pretty(&p)?;
        std::fs::write(path, json).with_context(|| format!("écriture de {}", path.display()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))?;
        let mut p: Project = serde_json::from_str(&text).context("fichier projet invalide")?;
        let base = path.parent().unwrap_or(Path::new(""));
        for m in &mut p.media {
            *m = resolve(m, base);
        }
        for l in &mut p.layers {
            if let Some(c) = &mut l.clip {
                *c = resolve(c, base);
            }
        }
        p.sanitize();
        Ok(p)
    }

    /// Corrige les valeurs incohérentes (fichier édité à la main, ancienne version…).
    pub fn sanitize(&mut self) {
        if self.layers.is_empty() {
            self.layers.push(LayerConfig::default());
        }
        for l in &mut self.layers {
            l.warp.sanitize();
            l.mask.sanitize();
        }
    }
}

fn resolve(p: &Path, base: &Path) -> PathBuf {
    if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
}

/// Chemin relatif si le média est dans le dossier du projet (ou un sous-dossier).
/// Toujours avec des « / », compris par macOS comme par Windows : un projet reste
/// utilisable d'un système à l'autre.
fn relative_to(p: &Path, base: &Path) -> PathBuf {
    match p.strip_prefix(base) {
        Ok(rel) if !base.as_os_str().is_empty() && rel.components().all(|c| matches!(c, Component::Normal(_))) => {
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            PathBuf::from(parts.join("/"))
        }
        _ => p.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_next_to_project_are_relative() {
        let dir = std::env::temp_dir().join(format!("ozmap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("show.ozmap");
        let inside = dir.join("clips").join("a.mp4");
        let outside = std::env::temp_dir().join("ailleurs").join("b.mov");

        let mut p = Project::default();
        p.media = vec![inside.clone(), outside.clone()];
        p.layers[0].clip = Some(inside.clone());
        p.save(&file).unwrap();

        let raw = std::fs::read_to_string(&file).unwrap();
        assert!(raw.contains("\"clips/a.mp4\""), "{raw}");
        assert!(!raw.contains("\"b.mov\""), "le média hors du dossier doit rester absolu : {raw}");

        let loaded = Project::load(&file).unwrap();
        // Comparaison composant par composant (sous Windows, « / » et « \\ » sont équivalents).
        let same = |a: &Path, b: &Path| a.components().eq(b.components());
        assert!(same(&loaded.media[0], &inside), "{:?}", loaded.media[0]);
        assert!(same(&loaded.media[1], &outside));
        assert!(same(loaded.layers[0].clip.as_deref().unwrap(), &inside));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
