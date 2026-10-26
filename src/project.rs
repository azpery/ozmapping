//! Fichier projet `.ozmap` (JSON) : médias, calques, presets et réglages.
//!
//! Les chemins des médias situés à côté du projet sont enregistrés en relatif,
//! pour pouvoir déplacer ou copier le dossier complet.

use crate::rhythm::React;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::automix::AutoMix;
use crate::effects::{AutoFx, BlendMode, Effects, Preset};
use crate::generator::Generator;
use crate::surface::{Fit, Mask, Warp};

pub const EXTENSION: &str = "ozmap";
const VERSION: u32 = 1;
pub const MAX_COLUMNS: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerConfig {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    /// Cadrage de l'image dans la surface (étirer, ajuster, remplir).
    pub fit: Fit,
    pub effects: Effects,
    /// Navigation automatique entre les effets.
    pub auto: AutoFx,
    /// Mix auto entre plusieurs vidéos.
    pub mix: AutoMix,
    /// Réaction au rythme (son ou tempo tapé).
    pub react: React,
    /// Visuel généré (utilisable sans vidéo).
    pub generator: Generator,
    pub warp: Warp,
    pub mask: Mask,
    /// Vidéo chargée sur ce calque (relancée à l'ouverture du projet).
    pub clip: Option<PathBuf>,
    /// Ligne du tableau : cases enregistrées pour ce calque (index = colonne).
    pub cells: Vec<Option<Cell>>,
}

/// Case du tableau : un média avec ses effets et ses réglages, rejoué d'un clic sur son calque.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cell {
    pub name: String,
    /// Média joué (aucun : seulement le visuel généré, ou rien).
    pub clip: Option<PathBuf>,
    pub effects: Effects,
    /// Nom du preset d'origine, pour le surligner une fois la case jouée.
    pub preset: Option<String>,
    pub generator: Generator,
    pub auto: AutoFx,
    pub fit: Fit,
    pub opacity: f32,
    pub blend: BlendMode,
    // Réglages ajoutés ensuite : absents (None) dans une case plus ancienne, le calque garde alors les siens.
    pub warp: Option<Warp>,
    pub mask: Option<Mask>,
    pub react: Option<React>,
    pub mix: Option<AutoMix>,
    /// Lecture : vitesse et sens.
    pub speed: Option<f32>,
    pub reverse: Option<bool>,
    pub paused: Option<bool>,
}

impl Default for Cell {
    fn default() -> Self {
        let layer = LayerConfig::default();
        Self {
            name: String::new(),
            clip: None,
            effects: layer.effects,
            preset: None,
            generator: layer.generator,
            auto: layer.auto,
            fit: layer.fit,
            opacity: layer.opacity,
            blend: layer.blend,
            warp: None,
            mask: None,
            react: None,
            mix: None,
            speed: None,
            reverse: None,
            paused: None,
        }
    }
}

/// Enchaînement automatique des colonnes du tableau.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chain {
    pub enabled: bool,
    /// Durée d'une colonne, en secondes.
    pub secs: f32,
}

impl Default for Chain {
    fn default() -> Self {
        Self { enabled: false, secs: 8.0 }
    }
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            name: "Calque 1".into(),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            fit: Fit::Stretch,
            effects: Effects::default(),
            auto: AutoFx::default(),
            mix: AutoMix::default(),
            react: React::default(),
            generator: Generator::default(),
            warp: Warp::default(),
            mask: Mask::default(),
            clip: None,
            cells: Vec::new(),
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
    /// Presets mémorisés des animations sur la forme du masque.
    pub mask_presets: Vec<Preset>,
    pub fade_secs: f32,
    pub preset_secs: f32,
    pub looping: bool,
    pub master: f32,
    /// Nombre de colonnes du tableau.
    pub columns: usize,
    pub chain: Chain,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: VERSION,
            media: Vec::new(),
            layers: vec![LayerConfig::default()],
            presets: Vec::new(),
            mask_presets: Vec::new(),
            fade_secs: 1.0,
            preset_secs: 0.5,
            looping: true,
            master: 1.0,
            columns: 8,
            chain: Chain::default(),
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
            for c in l.cells.iter_mut().flatten().filter_map(|c| c.clip.as_mut()) {
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
            for c in l.cells.iter_mut().flatten().filter_map(|c| c.clip.as_mut()) {
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
        self.columns = self.columns.clamp(1, MAX_COLUMNS);
        for l in &mut self.layers {
            l.warp.sanitize();
            l.mask.sanitize();
            // Une case au-delà de la dernière colonne agrandit le tableau.
            if let Some(last) = l.cells.iter().rposition(Option::is_some) {
                self.columns = self.columns.max((last + 1).min(MAX_COLUMNS));
            }
            l.cells.truncate(MAX_COLUMNS);
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
        p.layers[0].cells = vec![None, Some(Cell { name: "A".into(), clip: Some(inside.clone()), ..Cell::default() })];
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
        let cell = loaded.layers[0].cells[1].as_ref().expect("case enregistrée");
        assert!(same(cell.clip.as_deref().unwrap(), &inside), "{:?}", cell.clip);
        assert!(loaded.layers[0].cells[0].is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
