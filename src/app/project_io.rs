//! Projet : nouveau, ouvrir, enregistrer, ajout de médias.

use super::*;

impl OzApp {
    pub(super) fn set_project(&mut self, ctx: &egui::Context, project: Project, path: Option<PathBuf>) {
        self.layers = project.layers.iter().map(|_| LayerRuntime::default()).collect();
        // Le serveur de dessin reprend le dessin du nouveau projet.
        self.sketch.doc_synced = false;
        self.history.clear();
        self.project = project;
        self.saved = self.project.clone();
        self.project_path = path;
        self.selected = 0;
        self.fx_hidden_alert = None;
        self.chain_col = None;
        self.chain_timer = 0.0;
        self.cue = None;
        // Relance les vidéos des calques.
        for i in 0..self.project.layers.len() {
            if let Some(clip) = self.project.layers[i].clip.clone() {
                if clip.exists() {
                    self.launch_on(ctx, i, &clip);
                } else {
                    self.status = Some(format!("Média introuvable : {}", clip.display()));
                }
            }
        }
        // Relancer les vidéos ne compte pas comme une modification.
        self.saved = self.project.clone();
    }

    /// Propose d'enregistrer si besoin. Renvoie false si l'utilisateur annule.
    pub(super) fn confirm_discard(&mut self) -> bool {
        if !self.is_dirty() {
            return true;
        }
        let answer = rfd::MessageDialog::new()
            .set_title("Modifications non enregistrées")
            .set_description("Enregistrer les modifications du projet en cours ?")
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();
        match answer {
            rfd::MessageDialogResult::Yes => self.save(false),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    pub(super) fn new_project(&mut self, ctx: &egui::Context) {
        if self.confirm_discard() {
            self.set_project(ctx, Project::default(), None);
        }
    }

    pub(super) fn open_project(&mut self, ctx: &egui::Context) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Projet OZ Mapping", &[project::EXTENSION])
            .pick_file()
        {
            self.load_project_file(ctx, &path);
        }
    }

    pub(super) fn load_project_file(&mut self, ctx: &egui::Context, path: &Path) {
        match Project::load(path) {
            Ok(p) => {
                self.status = None;
                self.set_project(ctx, p, Some(path.to_path_buf()));
            }
            Err(e) => self.status = Some(format!("Ouverture impossible : {e:#}")),
        }
    }

    /// Enregistre (ou « enregistrer sous » si demandé / pas encore de fichier).
    pub(super) fn save(&mut self, save_as: bool) -> bool {
        let path = match (&self.project_path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("Projet OZ Mapping", &[project::EXTENSION])
                    .set_file_name(format!("{}.{}", self.project_name(), project::EXTENSION));
                if let Some(dir) = self.project_path.as_ref().and_then(|p| p.parent()) {
                    dialog = dialog.set_directory(dir);
                }
                let Some(mut p) = dialog.save_file() else {
                    return false;
                };
                if p.extension().is_none() {
                    p.set_extension(project::EXTENSION);
                }
                p
            }
        };
        self.store_active_cells();
        self.adopt_ipad_files(&path);
        match self.project.save(&path) {
            Ok(()) => {
                self.saved = self.project.clone();
                self.project_path = Some(path);
                self.status = None;
                true
            }
            Err(e) => {
                self.status = Some(format!("Enregistrement impossible : {e:#}"));
                false
            }
        }
    }

    pub(super) fn project_name(&self) -> String {
        self.project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "Sans titre".into(), |s| s.to_string_lossy().into_owned())
    }

    pub(super) fn add_media(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            // Un dossier : ajouté par référence (son contenu est suivi).
            if p.is_dir() {
                self.add_media_folder(p);
                continue;
            }
            // `absolute` plutôt que `canonicalize` : sous Windows ce dernier produit des chemins
            // « \\?\C:\… » que ffmpeg gère mal.
            let p = std::path::absolute(&p).unwrap_or(p);
            if (is_video_file(&p) || video::is_still_image(&p)) && !self.project.media.contains(&p) {
                self.project.media.push(p);
            }
        }
    }

    /// Fichiers venus de l'iPad (photos, photos du dessin, bassins de chimie), rangés jusque-là
    /// dans le dossier de l'app : copiés à côté du projet, dans « <projet> - fichiers iPad », et
    /// le projet pointe vers ces copies (il reste complet si on le déplace ou le copie).
    fn adopt_ipad_files(&mut self, project_file: &Path) {
        let Some(app_dir) = crate::sketch::data_dir() else { return };
        let Some(dir) = project_file.parent() else { return };
        let stem = project_file.file_stem().map_or_else(|| "Projet".into(), |s| s.to_string_lossy().into_owned());
        let target = dir.join(format!("{stem} - fichiers iPad"));
        let mut moved: std::collections::HashMap<PathBuf, PathBuf> = Default::default();
        let mut adopt = |p: &mut PathBuf| {
            if !p.starts_with(&app_dir) {
                return;
            }
            if let Some(new) = moved.get(p) {
                *p = new.clone();
                return;
            }
            let Some(name) = p.file_name() else { return };
            let new = target.join(name);
            let ok = new.exists() || (std::fs::create_dir_all(&target).is_ok() && std::fs::copy(&*p, &new).is_ok());
            if ok {
                moved.insert(p.clone(), new.clone());
                *p = new;
            }
        };
        let pr = &mut self.project;
        for m in &mut pr.media {
            adopt(m);
        }
        for f in pr.drawing.iter_mut().flat_map(|d| d.files_mut()) {
            adopt(f);
        }
        for l in &mut pr.layers {
            if let Some(c) = &mut l.clip {
                adopt(c);
            }
            if let Some(f) = l.chem.as_mut().and_then(|c| c.file.as_mut()) {
                adopt(f);
            }
            for c in l.cells.iter_mut().flatten().filter_map(|c| c.clip.as_mut()) {
                adopt(c);
            }
            for c in &mut l.mix.clips {
                adopt(c);
            }
        }
    }
}
