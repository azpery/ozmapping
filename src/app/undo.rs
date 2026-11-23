//! Annuler / rétablir (Cmd+Z, Cmd+Maj+Z ou Ctrl+Y).
//!
//! Avant chaque geste de l'utilisateur (clic, touche, texte, fichier déposé), une copie du projet
//! est mise de côté ; une fois le geste fini (bouton de la souris relâché), elle entre dans
//! l'historique si le projet a changé. Un glissement (curseur, objet de la scène 3D) compte donc
//! pour une seule étape, et les changements automatiques (mode AUTO, transitions, contrôleur
//! MIDI) n'en créent pas.

use std::time::Instant;

use super::*;

/// Étapes gardées au plus.
const MAX_STEPS: usize = 100;

#[derive(Default)]
pub(super) struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
    /// Projet tel qu'il était au début du geste en cours.
    pending: Option<Project>,
    /// Le geste en cours est de la saisie de texte, et heure de la dernière étape de texte :
    /// une frappe continue ne fait qu'une étape.
    typing: bool,
    last_text: Option<Instant>,
}

impl History {
    pub(super) fn clear(&mut self) {
        *self = History::default();
    }

    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(super) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

impl OzApp {
    /// Début d'image : un geste commence, on garde le projet tel qu'il est.
    pub(super) fn undo_begin(&mut self, ctx: &egui::Context) {
        let (gesture, text) = ctx.input(|i| {
            let text = i.events.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Paste(_)));
            let key = i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }));
            (i.pointer.any_pressed() || key || text || !i.raw.dropped_files.is_empty(), text)
        });
        if gesture && self.history.pending.is_none() {
            self.history.pending = Some(self.project.clone());
            self.history.typing = text;
        }
    }

    /// Fin d'image : le geste est fini (souris relâchée) ; s'il a changé le projet, c'est une étape.
    pub(super) fn undo_end(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let Some(before) = self.history.pending.take() else { return };
        if before == self.project {
            return;
        }
        let h = &mut self.history;
        // Frappe continue (moins d'une seconde entre deux lettres) : on garde l'état d'avant la frappe.
        let merge = h.typing && h.last_text.is_some_and(|t| t.elapsed().as_secs_f32() < 1.0);
        h.last_text = h.typing.then(Instant::now);
        if !merge {
            h.undo.push(before);
            if h.undo.len() > MAX_STEPS {
                h.undo.remove(0);
            }
        }
        h.redo.clear();
    }

    pub(super) fn undo(&mut self, ctx: &egui::Context) {
        let Some(prev) = self.history.undo.pop() else { return };
        let current = std::mem::replace(&mut self.project, prev);
        self.history.redo.push(current);
        self.after_history(ctx);
    }

    pub(super) fn redo(&mut self, ctx: &egui::Context) {
        let Some(next) = self.history.redo.pop() else { return };
        let current = std::mem::replace(&mut self.project, next);
        self.history.undo.push(current);
        self.after_history(ctx);
    }

    /// Remet l'état de lecture en accord avec le projet rétabli.
    fn after_history(&mut self, ctx: &egui::Context) {
        self.history.pending = None;
        self.history.last_text = None;
        // Une transition d'effets en cours réécrirait les effets rétablis.
        for rt in &mut self.layers {
            rt.fx_anim = None;
        }
        // Calques ajoutés, retirés ou déplacés : chaque lecture en cours retrouve son calque par
        // son fichier ; un calque sans lecture correspondante relance la sienne.
        let clips: Vec<Option<PathBuf>> = self.project.layers.iter().map(|l| l.clip.clone()).collect();
        let playing: Vec<Option<PathBuf>> =
            self.layers.iter().map(|rt| rt.active().map(|p| p.path.clone())).collect();
        if clips.len() != self.layers.len() || clips.iter().zip(&playing).any(|(c, p)| c.is_some() && c != p) {
            let mut old: Vec<Option<LayerRuntime>> = std::mem::take(&mut self.layers).into_iter().map(Some).collect();
            let mut relaunch = Vec::new();
            for (i, clip) in clips.iter().enumerate() {
                // Même calque, même fichier ; sinon une autre lecture du même fichier ; sinon rien.
                let same = old.get(i).is_some_and(Option::is_some) && playing.get(i) == Some(clip);
                let k = if same { Some(i) } else { (0..old.len()).find(|&k| old[k].is_some() && playing[k] == *clip && clip.is_some()) };
                match k.and_then(|k| old[k].take()) {
                    Some(rt) => self.layers.push(rt),
                    None => {
                        self.layers.push(LayerRuntime::default());
                        if let Some(c) = clip.clone().filter(|c| c.exists()) {
                            relaunch.push((i, c));
                        }
                    }
                }
            }
            for mut rt in old.into_iter().flatten() {
                rt.stop();
            }
            for (i, c) in relaunch {
                self.launch_on(ctx, i, &c);
            }
        }
        self.selected = self.selected.min(self.project.layers.len() - 1);
        self.fx_hidden_alert = None;
    }
}
