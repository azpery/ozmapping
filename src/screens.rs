//! Écrans branchés, pour choisir où envoyer la sortie.

use eframe::egui;

/// Un écran, en coordonnées du bureau (points sur macOS, pixels physiques sous Windows).
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    /// Identifiant stable d'un lancement à l'autre (nom système).
    pub key: String,
    pub label: String,
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub primary: bool,
}

/// Liste des écrans, l'écran principal en premier.
pub fn list() -> Vec<Screen> {
    let mut infos = display_info::DisplayInfo::all().unwrap_or_default();
    infos.sort_by_key(|d| (!d.is_primary, d.x, d.y));
    infos
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let name = [&d.friendly_name, &d.name]
                .into_iter()
                .find(|n| !n.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Écran {}", i + 1));
            let (w, h) = (d.width as f32 * screen_scale(d), d.height as f32 * screen_scale(d));
            Screen {
                key: if d.name.is_empty() { d.id.to_string() } else { d.name.clone() },
                label: format!(
                    "{}. {name}  ·  {w:.0}×{h:.0}{}",
                    i + 1,
                    if d.is_primary { "  (principal)" } else { "" }
                ),
                pos: [d.x as f32, d.y as f32],
                size: [d.width as f32, d.height as f32],
                primary: d.is_primary,
            }
        })
        .collect()
}

/// Facteur pour afficher la définition en pixels réels (macOS donne des points).
fn screen_scale(d: &display_info::DisplayInfo) -> f32 {
    if cfg!(target_os = "macos") { d.scale_factor.max(1.0) } else { 1.0 }
}

/// Position (en points egui de la fenêtre déplacée) d'un coin proche du haut-gauche de l'écran.
pub fn window_position(screen: &Screen, pixels_per_point: f32) -> egui::Pos2 {
    let margin = 0.05;
    let x = screen.pos[0] + screen.size[0] * margin;
    let y = screen.pos[1] + screen.size[1] * margin;
    // egui convertit la position en pixels physiques avec l'échelle de la fenêtre :
    // sous macOS, winit la ramène aussitôt en points ; ailleurs les écrans sont en pixels.
    if cfg!(target_os = "macos") {
        egui::pos2(x, y)
    } else {
        egui::pos2(x, y) / pixels_per_point.max(0.1)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_screens_without_panicking() {
        for s in super::list() {
            println!("{s:?}");
        }
    }
}
