//! Dossiers de médias ajoutés par référence : leur contenu (sous-dossiers compris) est relu
//! régulièrement, en arrière-plan ; ce qu'on y dépose apparaît dans les médias, ce qu'on en
//! retire disparaît. Rien n'est copié.

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// Relecture des dossiers (secondes).
const SCAN_EVERY: f64 = 2.0;
/// Limites d'un dossier : profondeur des sous-dossiers et nombre de fichiers retenus.
const MAX_DEPTH: usize = 8;
const MAX_FILES: usize = 5000;

/// Contenu d'un dossier : ses médias (chemins complets), triés par sous-dossier puis par nom.
pub(super) type FolderFiles = Vec<(PathBuf, Vec<PathBuf>)>;

#[derive(Default)]
pub(super) struct MediaFolders {
    /// Contenu connu de chaque dossier (dans l'ordre de `Project::media_folders`).
    pub(super) files: FolderFiles,
    /// Médias de l'app : ceux ajoutés un par un, puis ceux des dossiers.
    pub(super) all: Vec<PathBuf>,
    /// Médias ajoutés un par un, tels qu'au dernier calcul de `all`.
    own: Vec<PathBuf>,
    /// Sous-dossiers repliés dans la liste.
    pub(super) collapsed: std::collections::HashSet<PathBuf>,
    result: Arc<Mutex<Option<FolderFiles>>>,
    busy: Arc<AtomicBool>,
    at: f64,
}

/// Médias d'un dossier et de ses sous-dossiers (fichiers cachés ignorés).
fn scan(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
        let (mut dirs, mut files) = (Vec::new(), Vec::new());
        for e in entries {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => dirs.push(p),
                Ok(_) if is_video_file(&p) || video::is_still_image(&p) => files.push(p),
                _ => {}
            }
        }
        // Les fichiers d'un dossier avant ceux de ses sous-dossiers.
        for f in files {
            if out.len() >= MAX_FILES {
                return;
            }
            out.push(f);
        }
        if depth < MAX_DEPTH {
            for d in dirs {
                walk(&d, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out
}

impl OzApp {
    /// Ajoute un dossier par référence (ses médias, sous-dossiers compris, suivent ce qu'il contient).
    pub(super) fn add_media_folder(&mut self, dir: PathBuf) {
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        if !self.project.media_folders.contains(&dir) {
            self.project.media_folders.push(dir.clone());
            self.status = Some(format!("Dossier « {} » ajouté aux médias (suivi : ce qu'on y dépose apparaît)", file_name(&dir)));
        }
        // Relu tout de suite.
        self.media_folders.at = f64::MIN;
    }

    /// Relit les dossiers de temps en temps, et tient à jour la liste de tous les médias.
    pub(super) fn media_folders_tick(&mut self, ctx: &egui::Context) {
        let now = self.clock();
        let mf = &mut self.media_folders;
        if let Some(files) = mf.result.lock().unwrap().take() {
            // Résultat d'une liste de dossiers qui a changé entre-temps : ignoré (relu ensuite).
            if files.iter().map(|(d, _)| d).eq(self.project.media_folders.iter()) && files != mf.files {
                mf.files = files;
                mf.own.clear();
                mf.all.clear();
            }
        }
        let folders = self.project.media_folders.clone();
        if !folders.is_empty() && now - mf.at >= SCAN_EVERY && !mf.busy.swap(true, Ordering::AcqRel) {
            mf.at = now;
            let (result, busy, ctx, folders) = (mf.result.clone(), mf.busy.clone(), ctx.clone(), folders.clone());
            let _ = std::thread::Builder::new().name("dossiers-medias".into()).spawn(move || {
                let files: FolderFiles = folders.into_iter().map(|d| {
                    let f = scan(&d);
                    (d, f)
                }).collect();
                *result.lock().unwrap() = Some(files);
                busy.store(false, Ordering::Release);
                ctx.request_repaint();
            });
        }
        if folders.is_empty() && !mf.files.is_empty() {
            mf.files.clear();
            mf.all.clear();
        }
        if mf.own != self.project.media || (mf.all.is_empty() && !mf.files.is_empty()) {
            mf.own = self.project.media.clone();
            let mut all = self.project.media.clone();
            let known: std::collections::HashSet<&PathBuf> = self.project.media.iter().collect();
            let extra: Vec<PathBuf> =
                mf.files.iter().flat_map(|(_, f)| f.iter()).filter(|p| !known.contains(p)).cloned().collect();
            all.extend(extra);
            mf.all = all;
        }
    }

    /// Dossiers suivis, dans la liste des médias : arborescence repliable, médias à glisser
    /// sur une case, à envoyer sur le calque (double-clic) ou à regarder (clic).
    pub(super) fn media_folders_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let cue_path = self.cue.as_ref().map(|c| c.path.clone());
        let mut to_cue: Option<PathBuf> = None;
        let mut to_launch: Option<PathBuf> = None;
        let mut to_remove: Option<usize> = None;
        let mut toggle: Option<PathBuf> = None;
        let playing: std::collections::HashSet<PathBuf> =
            self.layers.iter().filter_map(|rt| rt.active().map(|p| p.path.clone())).collect();
        let files = self.media_folders.files.clone();
        for (k, root) in self.project.media_folders.clone().iter().enumerate() {
            let list: &[PathBuf] = files.iter().find(|(d, _)| d == root).map_or(&[], |(_, f)| f.as_slice());
            let collapsed = self.media_folders.collapsed.contains(root);
            ui.horizontal(|ui| {
                let caret = if collapsed { ic::CARET_RIGHT } else { ic::CARET_DOWN };
                if ui.add(egui::Button::new(caret).frame(false)).clicked() {
                    toggle = Some(root.clone());
                }
                ui.label(egui::RichText::new(ic::FOLDER).color(ACCENT));
                let missing = !root.is_dir();
                let name = egui::RichText::new(file_name(root)).strong();
                ui.label(if missing { name.color(WARN) } else { name.color(TEXT) }).on_hover_text(if missing {
                    format!("{}\nDossier introuvable (débranché, déplacé ?)", root.display())
                } else {
                    format!("{}\nSuivi : ce qu'on y dépose apparaît ici", root.display())
                });
                ui.label(egui::RichText::new(format!("{} médias", list.len())).small().color(TEXT_WEAK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(theme::icon_button(ic::TRASH).small().frame(false))
                        .on_hover_text("Retirer le dossier de la liste (les fichiers restent sur le disque)")
                        .clicked()
                    {
                        to_remove = Some(k);
                    }
                    if ui.add(theme::icon_button(ic::FOLDER_OPEN).small().frame(false)).on_hover_text("Montrer dans le Finder").clicked() {
                        reveal_in_file_manager(root);
                    }
                });
            });
            if collapsed {
                continue;
            }
            // Sous-dossiers : un titre repliable à chaque changement de dossier, décalé selon la profondeur.
            let mut current: Option<PathBuf> = None;
            let mut hidden_under: Option<PathBuf> = None;
            for path in list {
                let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
                if hidden_under.as_ref().is_some_and(|h| dir.starts_with(h)) {
                    continue;
                }
                hidden_under = None;
                if current.as_ref() != Some(&dir) {
                    current = Some(dir.clone());
                    if dir != *root {
                        // Titres des sous-dossiers traversés depuis le dernier affiché.
                        let rel = dir.strip_prefix(root).unwrap_or(&dir).to_path_buf();
                        let depth = rel.components().count();
                        let sub_collapsed = self.media_folders.collapsed.contains(&dir);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0 * depth as f32);
                            let caret = if sub_collapsed { ic::CARET_RIGHT } else { ic::CARET_DOWN };
                            if ui.add(egui::Button::new(caret).small().frame(false)).clicked() {
                                toggle = Some(dir.clone());
                            }
                            ui.label(egui::RichText::new(ic::FOLDER).small().color(TEXT_WEAK));
                            ui.label(egui::RichText::new(rel.to_string_lossy()).small().color(TEXT));
                        });
                        if sub_collapsed {
                            hidden_under = Some(dir.clone());
                            continue;
                        }
                    }
                }
                let depth = path.parent().and_then(|d| d.strip_prefix(root).ok()).map_or(0, |r| r.components().count());
                let on = playing.contains(path);
                let is_cue = cue_path.as_deref() == Some(path.as_path());
                let resp = ui
                    .horizontal(|ui| {
                        ui.add_space(12.0 * depth as f32 + 18.0);
                        let size = Vec2::new(48.0, 27.0);
                        let img = match self.thumbs.get(&ctx, path) {
                            Some(t) => ui.add(egui::Image::new(&t).fit_to_exact_size(size).corner_radius(3.0).sense(Sense::click_and_drag())),
                            None => {
                                let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                                ui.painter().rect_filled(r, 3.0, theme::WIDGET);
                                ui.painter().text(
                                    r.center(),
                                    egui::Align2::CENTER_CENTER,
                                    if video::is_still_image(path) { ic::IMAGE } else { ic::FILM_SLATE },
                                    egui::FontId::proportional(13.0),
                                    TEXT_WEAK,
                                );
                                resp
                            }
                        };
                        if is_cue {
                            ui.painter().rect_stroke(img.rect, 3.0, Stroke::new(1.5, WARN), egui::StrokeKind::Outside);
                        }
                        let name = egui::RichText::new(file_name(path)).small();
                        let label = ui
                            .add(egui::Label::new(if on { name.color(ACCENT) } else { name.color(TEXT) }).truncate().sense(Sense::click_and_drag()))
                            .on_hover_text(format!("{}\nClic : aperçu · double-clic : envoyer sur le calque · glisser vers une case", path.display()));
                        label.union(img)
                    })
                    .inner;
                if resp.drag_started() {
                    egui::DragAndDrop::set_payload(&ctx, MediaDrag::File(path.clone()));
                }
                if resp.double_clicked() {
                    to_launch = Some(path.clone());
                } else if resp.clicked() {
                    to_cue = Some(path.clone());
                }
            }
            if list.is_empty() {
                ui.label(egui::RichText::new("   Aucune vidéo ni photo dans ce dossier (pour l'instant)").small().color(TEXT_WEAK));
            }
        }
        if let Some(d) = toggle {
            let c = &mut self.media_folders.collapsed;
            if !c.remove(&d) {
                c.insert(d);
            }
        }
        if let Some(p) = to_cue {
            self.cue_media(&ctx, &p);
        }
        if let Some(p) = to_launch {
            let sel = self.sel();
            self.launch_any(&ctx, sel, &MediaDrag::File(p));
        }
        if let Some(k) = to_remove {
            self.project.media_folders.remove(k);
            self.media_folders.at = f64::MIN;
            self.media_folders.files.retain(|(d, _)| self.project.media_folders.contains(d));
            self.media_folders.own.clear();
            self.media_folders.all.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_subfolders() {
        let root = std::env::temp_dir().join(format!("oz-dossier-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["", "B/sous", "A", ".cache"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in ["z.mp4", "notes.txt", "B/clip.mov", "B/sous/photo.jpg", "A/a.png", ".cache/x.mp4", ".cachee.mp4"] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        let found: Vec<String> =
            scan(&root).iter().map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect();
        // Fichiers du dossier d'abord, puis sous-dossiers par ordre alphabétique ; rien de caché, que des médias.
        assert_eq!(found, ["z.mp4", "A/a.png", "B/clip.mov", "B/sous/photo.jpg"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
