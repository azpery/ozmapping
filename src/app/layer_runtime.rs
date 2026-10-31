//! État d'exécution d'un calque (lecteurs, fondus, caches de masque et de maillage).

use super::*;

/// Transition douce entre deux jeux d'effets (rappel de preset).
pub(super) struct FxAnim {
    pub(super) from: Effects,
    pub(super) to: Effects,
    pub(super) t: f32,
    pub(super) secs: f32,
}

/// État d'exécution d'un calque (sa configuration est dans `Project::layers`).
#[derive(Default)]
pub(super) struct LayerRuntime {
    pub(super) current: Option<VideoPlayer>,
    pub(super) next: Option<VideoPlayer>,
    pub(super) fade: f32,
    /// Durée du fondu en cours, si elle diffère du réglage général (mix auto).
    pub(super) fade_secs: Option<f32>,
    pub(super) fx_anim: Option<FxAnim>,
    /// Mix auto : temps écoulé dans la séquence, durée prévue, séquence en cours.
    pub(super) mix_timer: f32,
    pub(super) mix_len: f32,
    pub(super) mix_seg: Option<automix::Segment>,
    /// Temps écoulé depuis le dernier changement du mode Auto.
    pub(super) auto_timer: f32,
    /// Rythme : temps comptés pour AUTO et le mix, état des réactions.
    pub(super) auto_beats: u32,
    pub(super) mix_beats: u32,
    pub(super) beat: rhythm::BeatState,
    /// Nom du dernier preset appliqué (pour le surligner, même pendant la transition).
    pub(super) preset: Option<String>,
    pub(super) phases: Phases,
    pub(super) mesh: Option<(Warp, Arc<Vec<f32>>)>,
    /// Dernière image du masque, avec ce qui a servi à la calculer.
    pub(super) mask_raster: Option<(Mask, (u32, u32), u32, MaskDraw)>,
    /// Dernier champ de forme du masque (animations de forme), avec la forme qui l'a produit.
    pub(super) shape_field: Option<(Mask, (u32, u32), ShapeDraw)>,
    /// Générateur : temps d'animation et rotation de la palette (intégrés image par image).
    pub(super) gen_time: f32,
    pub(super) gen_hue: f32,
    /// Motif affiché, motif d'où part le fondu en cours, avancement du fondu (1 = fini).
    pub(super) gen_shown: (Pattern, Palette),
    pub(super) gen_from: (Pattern, Palette),
    pub(super) gen_fade: f32,
    pub(super) gen_fade_secs: f32,
    /// Durée imposée au prochain fondu (mode auto) ; sinon fondu court d'un changement manuel.
    pub(super) gen_next_secs: Option<f32>,
    /// Temps écoulé depuis le dernier changement automatique de motif.
    pub(super) gen_timer: f32,
    /// Destruction déclenchée à la main (« Casser ! ») : avancement du cycle (0..1).
    pub(super) shatter_shot: Option<f32>,
    /// Dernière case du tableau jouée sur ce calque.
    pub(super) cell: Option<usize>,
    /// Position de la vidéo de chaque case quittée (par colonne), pour y reprendre au retour.
    pub(super) cell_pos: std::collections::HashMap<usize, f64>,
}

/// Durée d'une destruction déclenchée à la main (cassure, pause, reconstruction).
pub(super) const SHATTER_SHOT_SECS: f32 = 5.0;

impl LayerRuntime {
    /// Le clip principal (celui qui arrive s'il y a un fondu).
    pub(super) fn active(&self) -> Option<&VideoPlayer> {
        self.next.as_ref().or(self.current.as_ref())
    }

    pub(super) fn players(&self) -> impl Iterator<Item = &VideoPlayer> {
        [&self.current, &self.next].into_iter().flatten()
    }

    pub(super) fn stop(&mut self) {
        self.current = None;
        self.next = None;
        self.fade = 0.0;
    }

    /// Image du masque, recalculée seulement si la forme, la taille ou l'adoucissement changent.
    pub(super) fn mask(&mut self, mask: &Mask, size: (u32, u32), feather_px: f32) -> MaskDraw {
        let feather_key = (feather_px * 4.0).round() as u32;
        if let Some((m, s, f, draw)) = &self.mask_raster
            && m == mask
            && *s == size
            && *f == feather_key
        {
            return draw.clone();
        }
        static NEXT_MASK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let draw = MaskDraw {
            key: NEXT_MASK.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            width: size.0,
            height: size.1,
            data: Arc::new(crate::mask::rasterize(mask, size.0, size.1, feather_px)),
            invert: mask.invert,
        };
        self.mask_raster = Some((mask.clone(), size, feather_key, draw.clone()));
        draw
    }

    /// Champ de forme du masque, recalculé seulement si les contours ou la taille changent.
    pub(super) fn shape(&mut self, mask: &Mask, size: (u32, u32), feather: f32) -> ShapeDraw {
        // Seuls les contours comptent (l'inversion et l'adoucissement sont appliqués au rendu).
        let outline = Mask { enabled: true, invert: false, feather: 0.0, ..mask.clone() };
        let draw = match &self.shape_field {
            Some((m, s, draw)) if *m == outline && *s == size => draw.clone(),
            _ => {
                static NEXT_SHAPE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                let draw = ShapeDraw {
                    key: NEXT_SHAPE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    width: size.0,
                    height: size.1,
                    data: Arc::new(crate::mask::shape_field(&outline, size.0, size.1)),
                    feather: 0.0,
                };
                self.shape_field = Some((outline, size, draw.clone()));
                draw
            }
        };
        ShapeDraw { feather, ..draw }
    }

    pub(super) fn mesh(&mut self, warp: &Warp) -> Arc<Vec<f32>> {
        match &self.mesh {
            Some((w, m)) if w == warp => m.clone(),
            _ => {
                let m = Arc::new(warp.mesh());
                self.mesh = Some((warp.clone(), m.clone()));
                m
            }
        }
    }
}

pub(super) fn source(p: &VideoPlayer) -> SourceRef {
    SourceRef {
        key: p.uid,
        frame: p.slot.clone(),
    }
}
