//! Fichiers : reconnaissance des types, affichage dans le gestionnaire de fichiers.

use super::*;

pub(super) const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mkv", "mov", "avi", "webm", "mpg", "mpeg", "wmv", "flv", "ts", "mts", "m2ts", "ogv", "gif", "hap",
];

pub(super) fn is_video_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

pub(super) fn is_svg_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

pub(super) fn is_project_file(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()) == Some(project::EXTENSION)
}

/// Montre le fichier dans le Finder (ou l'explorateur de fichiers).
pub(super) fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}

pub(super) fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}
