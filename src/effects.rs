use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mirror {
    #[default]
    Off,
    Horizontal,
    Vertical,
    Quad,
}

impl Mirror {
    pub const ALL: [Mirror; 4] = [Mirror::Off, Mirror::Horizontal, Mirror::Vertical, Mirror::Quad];

    pub fn label(self) -> &'static str {
        match self {
            Mirror::Off => "Aucun",
            Mirror::Horizontal => "Horizontal",
            Mirror::Vertical => "Vertical",
            Mirror::Quad => "4 quadrants",
        }
    }
}

/// Objet 3D sur lequel l'image est posée. L'ordre (numéro envoyé au shader) ne doit pas changer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Object3d {
    #[default]
    Off,
    Plane,
    Cube,
    Sphere,
    Cylinder,
    Torus,
    Terrain,
    Voxels,
}

impl Object3d {
    pub const ALL: [Object3d; 8] = [
        Object3d::Off,
        Object3d::Plane,
        Object3d::Cube,
        Object3d::Sphere,
        Object3d::Cylinder,
        Object3d::Torus,
        Object3d::Terrain,
        Object3d::Voxels,
    ];

    /// Objets construits à partir de l'image (hauteur = luminosité).
    pub fn uses_height(self) -> bool {
        matches!(self, Object3d::Terrain | Object3d::Voxels)
    }

    pub fn label(self) -> &'static str {
        match self {
            Object3d::Off => "Aucun",
            Object3d::Plane => "Plan",
            Object3d::Cube => "Cube",
            Object3d::Sphere => "Sphère",
            Object3d::Cylinder => "Cylindre",
            Object3d::Torus => "Tore",
            Object3d::Terrain => "Terrain",
            Object3d::Voxels => "Pixels 3D",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Object3d::Off => "L'image reste à plat",
            Object3d::Plane => "L'image comme une carte qui pivote dans l'espace, en perspective",
            Object3d::Cube => "L'image sur les six faces d'un cube qui tourne",
            Object3d::Sphere => "L'image enroulée sur une boule",
            Object3d::Cylinder => "L'image enroulée sur un cylindre qui tourne",
            Object3d::Torus => "L'image enroulée sur un anneau (donut)",
            Object3d::Terrain => "L'image devient un paysage en relief : les zones claires sont des sommets",
            Object3d::Voxels => "L'image devient une ville de colonnes : chaque pixel monte selon sa clarté",
        }
    }
}

/// Destruction du masque : la forme se casse et laisse voir les calques du dessous.
/// L'ordre (numéro envoyé au shader) ne doit pas changer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Shatter {
    #[default]
    Off,
    Bricks,
    Glass,
    Burn,
    Dust,
    Flip,
}

impl Shatter {
    pub const ALL: [Shatter; 6] = [Shatter::Off, Shatter::Bricks, Shatter::Glass, Shatter::Burn, Shatter::Dust, Shatter::Flip];

    pub fn label(self) -> &'static str {
        match self {
            Shatter::Off => "Aucune",
            Shatter::Bricks => "Mur qui s'écroule",
            Shatter::Glass => "Verre brisé",
            Shatter::Burn => "Combustion",
            Shatter::Dust => "Désintégration",
            Shatter::Flip => "Tuiles qui se retournent",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Shatter::Off => "La forme reste intacte",
            Shatter::Bricks => "Le mur se fissure puis les briques tombent une à une",
            Shatter::Glass => "La forme se fend en éclats qui volent et retombent",
            Shatter::Burn => "La forme brûle et part en braises depuis le point d'impact",
            Shatter::Dust => "La forme s'effrite en poussière emportée par le vent",
            Shatter::Flip => "La forme est découpée en tuiles qui pivotent et disparaissent",
        }
    }
}

/// Tous les paramètres d'effets envoyés au shader. Les valeurs par défaut = aucun effet.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    // Couleur
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub hue: f32,
    pub hue_speed: f32,
    pub invert: f32,
    pub posterize: f32,
    pub tint: [f32; 3],
    pub tint_amount: f32,
    pub edge: f32,
    pub vignette: f32,
    // Géométrie
    pub zoom: f32,
    pub rotate: f32,
    pub rotate_speed: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub mirror: Mirror,
    pub kaleido: f32,
    pub kaleido_spin: f32,
    // Distorsions
    pub pixelate: f32,
    pub rgb_shift: f32,
    pub wave_amp: f32,
    pub wave_freq: f32,
    pub wave_speed: f32,
    // Psychédélique
    /// Tourbillon : torsion autour du centre (radians au centre).
    pub swirl: f32,
    /// Oscillation du tourbillon (tours/s).
    pub swirl_speed: f32,
    /// Passage en coordonnées polaires : effet tunnel (0..1).
    pub polar: f32,
    /// Avancée dans le tunnel (tours/s).
    pub tunnel_speed: f32,
    /// Mosaïque en miroir : nombre de répétitions (1 = aucune).
    pub tiles: f32,
    /// Distorsion liquide : amplitude, échelle des ondulations, vitesse.
    pub liquid: f32,
    pub liquid_scale: f32,
    pub liquid_speed: f32,
    /// Palette acide : couleurs arc-en-ciel selon la luminosité (0..1), et sa rotation (tours/s).
    pub acid: f32,
    pub acid_speed: f32,
    /// Arc-en-ciel spatial : décalage de teinte selon la position dans l'image.
    pub rainbow: f32,
    /// Solarisation (0..1).
    pub solarize: f32,
    /// Pulsation : zoom qui respire (amplitude), et sa fréquence (Hz).
    pub pulse: f32,
    pub pulse_rate: f32,
    /// Écho : copies zoomées superposées (0..1).
    pub echo: f32,
    // Forme du masque (sans effet tant que le calque n'a pas de masque actif)
    /// Couleur des lumières dessinées sur la forme.
    pub shape_color: [f32; 3],
    /// Néon le long du contour (0..1), et sa largeur (en hauteurs de sortie).
    pub shape_glow: f32,
    pub shape_width: f32,
    /// Lignes concentriques qui épousent la forme (0..1), leur nombre par hauteur de sortie,
    /// et leur vitesse (lignes/s, positive = vers l'intérieur).
    pub shape_ripple: f32,
    pub shape_ripple_freq: f32,
    pub shape_ripple_speed: f32,
    /// Ondulation de l'image depuis les bords de la forme (mêmes lignes que ci-dessus).
    pub shape_wave: f32,
    /// Comètes qui parcourent le contour (0..1), leur nombre, leur vitesse (tours/s).
    pub shape_trace: f32,
    pub shape_trace_count: f32,
    pub shape_trace_speed: f32,
    /// Respiration : la forme grossit et rétrécit (amplitude en hauteurs de sortie), fréquence (Hz).
    pub shape_breathe: f32,
    pub shape_breathe_rate: f32,
    /// Couleurs arc-en-ciel qui tournent le long du contour, au lieu de la couleur unique (0..1).
    pub shape_rainbow: f32,
    /// Échos du contour qui s'éloignent à l'extérieur (0..1), avec la densité et la vitesse des lignes.
    pub shape_echo: f32,
    /// Lueur intérieure : la couleur monte depuis les bords (0..1).
    pub shape_inner: f32,
    /// Pointillés qui défilent sur le contour (0..1), et leur nombre (à la vitesse des comètes).
    pub shape_dashes: f32,
    pub shape_dash_count: f32,
    /// Destruction : style, avancement manuel (0..1), boucle automatique (cycles/s, 0 = manuel),
    /// taille des morceaux (par hauteur de sortie), lueur des fissures.
    pub shatter: Shatter,
    pub shatter_amount: f32,
    pub shatter_speed: f32,
    pub shatter_size: f32,
    pub shatter_glow: f32,
    /// Contours animés de l'image (photo ou vidéo) : lueur des contours (0..2), leur couleur
    /// et la part d'arc-en-ciel qui défile.
    pub lines_glow: f32,
    pub lines_color: [f32; 3],
    pub lines_rainbow: f32,
    /// Seuil (0..1) : plus il est haut, moins il reste de contours (seulement les plus nets).
    pub lines_threshold: f32,
    /// Largeur du halo, en pixels de l'image.
    pub lines_width: f32,
    /// Vagues de lumière qui parcourent les contours (0..1), et leur vitesse (vagues/s).
    pub lines_flow: f32,
    pub lines_flow_speed: f32,
    /// Les contours ondulent (amplitude, en largeurs d'image), à cette vitesse (cycles/s).
    pub lines_wobble: f32,
    pub lines_wobble_speed: f32,
    /// Assombrit l'image pour ne garder que les contours lumineux (0..1).
    pub lines_dim: f32,
    /// 3D : objet portant l'image, ses angles (degrés), sa rotation automatique (°/s)
    /// et la distance de la caméra (plus grande = perspective plus douce).
    pub obj3d: Object3d,
    pub rot3d_x: f32,
    pub rot3d_y: f32,
    pub spin3d_x: f32,
    pub spin3d_y: f32,
    pub dist3d: f32,
    /// Terrain et pixels 3D : hauteur du relief, nombre de colonnes (par hauteur d'image).
    pub height3d: f32,
    pub voxels: f32,
    /// Relief : l'image prend du volume selon sa luminosité, éclairée de biais (0..1).
    pub relief: f32,
    /// Anaglyphe rouge / cyan (décalage, en largeur d'image).
    pub anaglyph: f32,
    /// Drapeau : l'image ondule comme un tissu, avec ses ombres (0..1), et sa vitesse (rad/s).
    pub flag: f32,
    pub flag_speed: f32,
    /// Caméra thermique : la luminosité devient une palette de chaleur (0..1).
    pub thermal: f32,
    /// Flou (0..1) et halo lumineux autour des zones claires (bloom, 0..1).
    pub blur: f32,
    pub bloom: f32,
    /// Ondes de choc qui partent du centre (force 0..1) et leur fréquence (ondes/s).
    pub shock: f32,
    pub shock_speed: f32,
    /// Fish-eye : bombé (positif) ou pincé (négatif).
    pub fisheye: f32,
    /// Mosaïque hexagonale (0..1 = taille des alvéoles).
    pub hexagon: f32,
    /// Rétro : télé cathodique (lignes, courbure), glitch numérique, grain de pellicule,
    /// trame de points façon BD (0..1).
    pub crt: f32,
    pub glitch: f32,
    pub grain: f32,
    pub halftone: f32,
    /// Droste : l'image se répète en spirale infinie (0..1) et zoome (répétitions/s).
    pub droste: f32,
    pub droste_speed: f32,
    /// Flou de zoom radial (0..1).
    pub zoom_blur: f32,
    /// Rendu en caractères ASCII (0 = non ; plus grand = caractères plus gros).
    pub ascii: f32,
    /// Accordéon 3D : l'image pliée en panneaux (0..1), et leur nombre.
    pub fold: f32,
    pub fold_count: f32,
    // Rythme
    pub strobe: f32,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            hue: 0.0,
            hue_speed: 0.0,
            invert: 0.0,
            posterize: 0.0,
            tint: [1.0, 0.2, 0.6],
            tint_amount: 0.0,
            edge: 0.0,
            vignette: 0.0,
            zoom: 1.0,
            rotate: 0.0,
            rotate_speed: 0.0,
            flip_x: false,
            flip_y: false,
            mirror: Mirror::Off,
            kaleido: 0.0,
            kaleido_spin: 0.0,
            pixelate: 0.0,
            rgb_shift: 0.0,
            wave_amp: 0.0,
            wave_freq: 8.0,
            wave_speed: 2.0,
            swirl: 0.0,
            swirl_speed: 0.0,
            polar: 0.0,
            tunnel_speed: 0.0,
            tiles: 1.0,
            liquid: 0.0,
            liquid_scale: 6.0,
            liquid_speed: 1.0,
            acid: 0.0,
            acid_speed: 0.1,
            rainbow: 0.0,
            solarize: 0.0,
            pulse: 0.0,
            pulse_rate: 0.5,
            echo: 0.0,
            shape_color: [0.3, 0.85, 1.0],
            shape_glow: 0.0,
            shape_width: 0.008,
            shape_ripple: 0.0,
            shape_ripple_freq: 12.0,
            shape_ripple_speed: 0.5,
            shape_wave: 0.0,
            shape_trace: 0.0,
            shape_trace_count: 2.0,
            shape_trace_speed: 0.2,
            shape_breathe: 0.0,
            shape_breathe_rate: 0.3,
            shape_rainbow: 0.0,
            shape_echo: 0.0,
            shape_inner: 0.0,
            shape_dashes: 0.0,
            shape_dash_count: 30.0,
            shatter: Shatter::Off,
            shatter_amount: 0.0,
            shatter_speed: 0.0,
            shatter_size: 10.0,
            shatter_glow: 0.8,
            lines_glow: 0.0,
            lines_color: [1.0, 0.75, 0.3],
            lines_rainbow: 0.0,
            lines_threshold: 0.25,
            lines_width: 4.0,
            lines_flow: 0.0,
            lines_flow_speed: 0.4,
            lines_wobble: 0.0,
            lines_wobble_speed: 0.3,
            lines_dim: 0.0,
            obj3d: Object3d::Off,
            rot3d_x: 0.0,
            rot3d_y: 0.0,
            spin3d_x: 0.0,
            spin3d_y: 0.0,
            dist3d: 2.5,
            height3d: 0.3,
            voxels: 40.0,
            relief: 0.0,
            anaglyph: 0.0,
            flag: 0.0,
            flag_speed: 2.0,
            thermal: 0.0,
            blur: 0.0,
            bloom: 0.0,
            shock: 0.0,
            shock_speed: 0.5,
            fisheye: 0.0,
            hexagon: 0.0,
            crt: 0.0,
            glitch: 0.0,
            grain: 0.0,
            halftone: 0.0,
            droste: 0.0,
            droste_speed: 0.3,
            zoom_blur: 0.0,
            ascii: 0.0,
            fold: 0.0,
            fold_count: 6.0,
            strobe: 0.0,
        }
    }
}

impl Effects {
    /// Interpolation entre deux jeux d'effets (pour les transitions de presets).
    /// Les réglages discrets (miroir, retournements) basculent à mi-parcours.
    pub fn lerp(a: &Effects, b: &Effects, t: f32) -> Effects {
        let t = t.clamp(0.0, 1.0);
        let l = |x: f32, y: f32| x + (y - x) * t;
        fn pick<T>(x: T, y: T, t: f32) -> T {
            if t < 0.5 { x } else { y }
        }
        Effects {
            brightness: l(a.brightness, b.brightness),
            contrast: l(a.contrast, b.contrast),
            saturation: l(a.saturation, b.saturation),
            hue: l(a.hue, b.hue),
            hue_speed: l(a.hue_speed, b.hue_speed),
            invert: l(a.invert, b.invert),
            posterize: pick(a.posterize, b.posterize, t),
            tint: [l(a.tint[0], b.tint[0]), l(a.tint[1], b.tint[1]), l(a.tint[2], b.tint[2])],
            tint_amount: l(a.tint_amount, b.tint_amount),
            edge: l(a.edge, b.edge),
            vignette: l(a.vignette, b.vignette),
            zoom: l(a.zoom, b.zoom),
            rotate: l(a.rotate, b.rotate),
            rotate_speed: l(a.rotate_speed, b.rotate_speed),
            flip_x: pick(a.flip_x, b.flip_x, t),
            flip_y: pick(a.flip_y, b.flip_y, t),
            mirror: pick(a.mirror, b.mirror, t),
            kaleido: pick(a.kaleido, b.kaleido, t),
            kaleido_spin: l(a.kaleido_spin, b.kaleido_spin),
            pixelate: l(a.pixelate, b.pixelate),
            rgb_shift: l(a.rgb_shift, b.rgb_shift),
            wave_amp: l(a.wave_amp, b.wave_amp),
            wave_freq: l(a.wave_freq, b.wave_freq),
            wave_speed: l(a.wave_speed, b.wave_speed),
            swirl: l(a.swirl, b.swirl),
            swirl_speed: l(a.swirl_speed, b.swirl_speed),
            polar: l(a.polar, b.polar),
            tunnel_speed: l(a.tunnel_speed, b.tunnel_speed),
            tiles: pick(a.tiles, b.tiles, t),
            liquid: l(a.liquid, b.liquid),
            liquid_scale: l(a.liquid_scale, b.liquid_scale),
            liquid_speed: l(a.liquid_speed, b.liquid_speed),
            acid: l(a.acid, b.acid),
            acid_speed: l(a.acid_speed, b.acid_speed),
            rainbow: l(a.rainbow, b.rainbow),
            solarize: l(a.solarize, b.solarize),
            pulse: l(a.pulse, b.pulse),
            pulse_rate: l(a.pulse_rate, b.pulse_rate),
            echo: l(a.echo, b.echo),
            shape_color: [
                l(a.shape_color[0], b.shape_color[0]),
                l(a.shape_color[1], b.shape_color[1]),
                l(a.shape_color[2], b.shape_color[2]),
            ],
            shape_glow: l(a.shape_glow, b.shape_glow),
            shape_width: l(a.shape_width, b.shape_width),
            shape_ripple: l(a.shape_ripple, b.shape_ripple),
            shape_ripple_freq: l(a.shape_ripple_freq, b.shape_ripple_freq),
            shape_ripple_speed: l(a.shape_ripple_speed, b.shape_ripple_speed),
            shape_wave: l(a.shape_wave, b.shape_wave),
            shape_trace: l(a.shape_trace, b.shape_trace),
            shape_trace_count: pick(a.shape_trace_count, b.shape_trace_count, t),
            shape_trace_speed: l(a.shape_trace_speed, b.shape_trace_speed),
            shape_breathe: l(a.shape_breathe, b.shape_breathe),
            shape_breathe_rate: l(a.shape_breathe_rate, b.shape_breathe_rate),
            shape_rainbow: l(a.shape_rainbow, b.shape_rainbow),
            shape_echo: l(a.shape_echo, b.shape_echo),
            shape_inner: l(a.shape_inner, b.shape_inner),
            shape_dashes: l(a.shape_dashes, b.shape_dashes),
            shape_dash_count: pick(a.shape_dash_count, b.shape_dash_count, t),
            shatter: pick(a.shatter, b.shatter, t),
            shatter_amount: l(a.shatter_amount, b.shatter_amount),
            shatter_speed: l(a.shatter_speed, b.shatter_speed),
            shatter_size: l(a.shatter_size, b.shatter_size),
            shatter_glow: l(a.shatter_glow, b.shatter_glow),
            lines_glow: l(a.lines_glow, b.lines_glow),
            lines_color: [
                l(a.lines_color[0], b.lines_color[0]),
                l(a.lines_color[1], b.lines_color[1]),
                l(a.lines_color[2], b.lines_color[2]),
            ],
            lines_rainbow: l(a.lines_rainbow, b.lines_rainbow),
            lines_threshold: l(a.lines_threshold, b.lines_threshold),
            lines_width: l(a.lines_width, b.lines_width),
            lines_flow: l(a.lines_flow, b.lines_flow),
            lines_flow_speed: l(a.lines_flow_speed, b.lines_flow_speed),
            lines_wobble: l(a.lines_wobble, b.lines_wobble),
            lines_wobble_speed: l(a.lines_wobble_speed, b.lines_wobble_speed),
            lines_dim: l(a.lines_dim, b.lines_dim),
            obj3d: pick(a.obj3d, b.obj3d, t),
            rot3d_x: l(a.rot3d_x, b.rot3d_x),
            rot3d_y: l(a.rot3d_y, b.rot3d_y),
            spin3d_x: l(a.spin3d_x, b.spin3d_x),
            spin3d_y: l(a.spin3d_y, b.spin3d_y),
            dist3d: l(a.dist3d, b.dist3d),
            height3d: l(a.height3d, b.height3d),
            voxels: pick(a.voxels, b.voxels, t),
            relief: l(a.relief, b.relief),
            anaglyph: l(a.anaglyph, b.anaglyph),
            flag: l(a.flag, b.flag),
            flag_speed: l(a.flag_speed, b.flag_speed),
            thermal: l(a.thermal, b.thermal),
            blur: l(a.blur, b.blur),
            bloom: l(a.bloom, b.bloom),
            shock: l(a.shock, b.shock),
            shock_speed: l(a.shock_speed, b.shock_speed),
            fisheye: l(a.fisheye, b.fisheye),
            hexagon: l(a.hexagon, b.hexagon),
            crt: l(a.crt, b.crt),
            glitch: l(a.glitch, b.glitch),
            grain: l(a.grain, b.grain),
            halftone: l(a.halftone, b.halftone),
            droste: l(a.droste, b.droste),
            droste_speed: l(a.droste_speed, b.droste_speed),
            zoom_blur: l(a.zoom_blur, b.zoom_blur),
            ascii: pick(a.ascii, b.ascii, t),
            fold: l(a.fold, b.fold),
            fold_count: pick(a.fold_count, b.fold_count, t),
            strobe: pick(a.strobe, b.strobe, t),
        }
    }

    /// Avancement de la destruction (0 = intacte, 1 = détruite) pour une phase de boucle (0..1) :
    /// intacte, cassure, reste détruite un moment, puis se reconstruit.
    pub fn shatter_cycle(phase: f32) -> f32 {
        let smooth = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        let p = phase.rem_euclid(1.0);
        if p < 0.1 {
            0.0
        } else if p < 0.55 {
            smooth((p - 0.1) / 0.45)
        } else if p < 0.7 {
            1.0
        } else {
            1.0 - smooth((p - 0.7) / 0.3)
        }
    }

    /// Ces effets, avec les animations de forme du masque et les contours animés de `other`.
    /// Ces animations se règlent à part (onglet Visuels) : presets, mode Auto et « Aucun » ne
    /// les touchent pas.
    pub fn with_shape_of(self, other: &Effects) -> Effects {
        Effects {
            shape_color: other.shape_color,
            shape_glow: other.shape_glow,
            shape_width: other.shape_width,
            shape_ripple: other.shape_ripple,
            shape_ripple_freq: other.shape_ripple_freq,
            shape_ripple_speed: other.shape_ripple_speed,
            shape_wave: other.shape_wave,
            shape_trace: other.shape_trace,
            shape_trace_count: other.shape_trace_count,
            shape_trace_speed: other.shape_trace_speed,
            shape_breathe: other.shape_breathe,
            shape_breathe_rate: other.shape_breathe_rate,
            shape_rainbow: other.shape_rainbow,
            shape_echo: other.shape_echo,
            shape_inner: other.shape_inner,
            shape_dashes: other.shape_dashes,
            shape_dash_count: other.shape_dash_count,
            shatter: other.shatter,
            shatter_amount: other.shatter_amount,
            shatter_speed: other.shatter_speed,
            shatter_size: other.shatter_size,
            shatter_glow: other.shatter_glow,
            lines_glow: other.lines_glow,
            lines_color: other.lines_color,
            lines_rainbow: other.lines_rainbow,
            lines_threshold: other.lines_threshold,
            lines_width: other.lines_width,
            lines_flow: other.lines_flow,
            lines_flow_speed: other.lines_flow_speed,
            lines_wobble: other.lines_wobble,
            lines_wobble_speed: other.lines_wobble_speed,
            lines_dim: other.lines_dim,
            ..self
        }
    }

    /// Reprend seulement les animations de forme du masque de `other` (ni la destruction, ni les contours).
    pub fn with_mask_anim_of(self, other: &Effects) -> Effects {
        Effects {
            shape_color: other.shape_color,
            shape_glow: other.shape_glow,
            shape_width: other.shape_width,
            shape_ripple: other.shape_ripple,
            shape_ripple_freq: other.shape_ripple_freq,
            shape_ripple_speed: other.shape_ripple_speed,
            shape_wave: other.shape_wave,
            shape_trace: other.shape_trace,
            shape_trace_count: other.shape_trace_count,
            shape_trace_speed: other.shape_trace_speed,
            shape_breathe: other.shape_breathe,
            shape_breathe_rate: other.shape_breathe_rate,
            shape_rainbow: other.shape_rainbow,
            shape_echo: other.shape_echo,
            shape_inner: other.shape_inner,
            shape_dashes: other.shape_dashes,
            shape_dash_count: other.shape_dash_count,
            ..self
        }
    }

    /// Mêmes animations de forme du masque que `other`.
    pub fn same_mask_anim(&self, other: &Effects) -> bool {
        self.with_mask_anim_of(other) == *self
    }

    /// Styles tout prêts pour les contours animés : (nom, description).
    pub const LINE_STYLES: [(&'static str, &'static str); 5] = [
        ("Néon doré", "Contours dorés et lumineux qui frémissent"),
        ("Dessin vivant", "L'image s'efface derrière un trait blanc qui ondule, comme un dessin animé"),
        ("Arc-en-ciel", "Contours aux couleurs qui défilent, parcourus de vagues"),
        ("Vagues de lumière", "Des vagues partent du centre et allument les contours à leur passage"),
        ("Aucun", "Retirer l'animation des contours"),
    ];

    /// Applique le style `k` de `LINE_STYLES` (seulement les réglages des contours animés).
    pub fn set_lines_style(&mut self, k: usize) {
        let d = Effects::default();
        let (glow, color, rainbow, threshold, width, flow, wobble, wobble_speed, dim) = match k {
            0 => (1.2, [1.0, 0.75, 0.3], 0.0, 0.25, 4.0, 0.0, 0.002, 0.3, 0.3),
            1 => (1.1, [1.0, 1.0, 1.0], 0.0, 0.3, 2.0, 0.0, 0.004, 0.5, 0.85),
            2 => (1.3, d.lines_color, 1.0, 0.25, 4.0, 0.6, 0.002, 0.3, 0.5),
            3 => (1.5, [0.3, 0.85, 1.0], 0.0, 0.2, 5.0, 1.0, 0.0, d.lines_wobble_speed, 0.25),
            _ => (0.0, d.lines_color, 0.0, d.lines_threshold, d.lines_width, 0.0, 0.0, d.lines_wobble_speed, 0.0),
        };
        self.lines_glow = glow;
        self.lines_color = color;
        self.lines_rainbow = rainbow;
        self.lines_threshold = threshold;
        self.lines_width = width;
        self.lines_flow = flow;
        self.lines_flow_speed = d.lines_flow_speed;
        self.lines_wobble = wobble;
        self.lines_wobble_speed = wobble_speed;
        self.lines_dim = dim;
    }

    /// Vrai si une animation suit la forme du masque (il faut alors calculer son champ).
    pub fn uses_shape(&self) -> bool {
        self.shape_glow > 0.0
            || self.shape_ripple > 0.0
            || self.shape_wave > 0.0
            || self.shape_trace > 0.0
            || self.shape_breathe != 0.0
            || self.shape_echo > 0.0
            || self.shape_inner > 0.0
            || self.shape_dashes > 0.0
    }

    /// Combinaison aléatoire de 2 à 4 effets (tous thèmes), pour l'improvisation.
    pub fn random(seed: u64) -> Effects {
        Self::themed(AutoTheme::All, seed, 1.0, false)
    }

    /// Effet généré au hasard dans un thème. `intensity` (≈ 0,3 à 1,5) dose les réglages ;
    /// `strobe` autorise le stroboscope.
    pub fn themed(theme: AutoTheme, seed: u64, intensity: f32, strobe: bool) -> Effects {
        let mut r = Rng(seed | 1);
        let k = intensity.clamp(0.1, 2.0);
        let mut fx = Effects::default();
        // Chaque « ingrédient » modifie quelques réglages ; un effet en combine plusieurs.
        let pool: &[Ingredient] = match theme {
            AutoTheme::All => &Ingredient::ALL,
            AutoTheme::Psyche => &[
                Ingredient::Swirl,
                Ingredient::Tunnel,
                Ingredient::Liquid,
                Ingredient::Acid,
                Ingredient::Rainbow,
                Ingredient::Kaleido,
                Ingredient::Echo,
                Ingredient::HueCycle,
                Ingredient::Pulse,
                Ingredient::Wave,
            ],
            AutoTheme::Color => &[
                Ingredient::HueShift,
                Ingredient::HueCycle,
                Ingredient::Acid,
                Ingredient::Rainbow,
                Ingredient::Tint,
                Ingredient::Saturate,
                Ingredient::Posterize,
                Ingredient::Solarize,
                Ingredient::Invert,
                Ingredient::Contrast,
                Ingredient::Thermal,
                Ingredient::Halftone,
            ],
            AutoTheme::Speed => &[
                Ingredient::FastSpin,
                Ingredient::FastKaleido,
                Ingredient::FastPulse,
                Ingredient::FastWave,
                Ingredient::FastTunnel,
                Ingredient::FastHue,
                Ingredient::FastLiquid,
                Ingredient::Strobe,
                Ingredient::ZoomBlur,
            ],
            AutoTheme::Geometry => &[
                Ingredient::Kaleido,
                Ingredient::Mirror,
                Ingredient::Tiles,
                Ingredient::Tunnel,
                Ingredient::Zoom,
                Ingredient::Rotate,
                Ingredient::Flip,
                Ingredient::Swirl,
                Ingredient::Fisheye,
                Ingredient::Shock,
                Ingredient::Droste,
                Ingredient::Fold,
            ],
            AutoTheme::Glitch => &[
                Ingredient::RgbShift,
                Ingredient::Pixelate,
                Ingredient::Posterize,
                Ingredient::Edge,
                Ingredient::GlitchWave,
                Ingredient::Solarize,
                Ingredient::Invert,
                Ingredient::Strobe,
                Ingredient::Crt,
                Ingredient::BlockGlitch,
                Ingredient::Grain,
                Ingredient::Hexagon,
                Ingredient::Ascii,
            ],
            AutoTheme::Soft => &[
                Ingredient::SoftColor,
                Ingredient::Vignette,
                Ingredient::SlowHue,
                Ingredient::SoftLiquid,
                Ingredient::SlowRotate,
                Ingredient::SoftTint,
                Ingredient::SoftPulse,
                Ingredient::Bloom,
            ],
        };
        let count = 2 + r.below(3);
        let mut picked = 0;
        let mut tries = 0;
        while picked < count && tries < 40 {
            tries += 1;
            let ing = pool[r.below(pool.len())];
            if ing == Ingredient::Strobe && !strobe {
                continue;
            }
            ing.apply(&mut fx, &mut r, k);
            picked += 1;
        }
        fx
    }
}

/// Petit générateur pseudo-aléatoire (xorshift), suffisant ici.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        let x = &mut self.0;
        *x ^= *x << 13;
        *x ^= *x >> 7;
        *x ^= *x << 17;
        (*x >> 11) as f32 / (1u64 << 53) as f32
    }

    /// Entier dans 0..n.
    fn below(&mut self, n: usize) -> usize {
        ((self.next() * n as f32) as usize).min(n.saturating_sub(1))
    }

    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }

    /// Valeur de signe aléatoire.
    fn signed(&mut self, a: f32, b: f32) -> f32 {
        let v = self.range(a, b);
        if self.next() < 0.5 { -v } else { v }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ingredient {
    Swirl,
    Tunnel,
    Liquid,
    Acid,
    Rainbow,
    Kaleido,
    Echo,
    HueCycle,
    Pulse,
    Wave,
    HueShift,
    Tint,
    Saturate,
    Posterize,
    Solarize,
    Invert,
    Contrast,
    FastSpin,
    FastKaleido,
    FastPulse,
    FastWave,
    FastTunnel,
    FastHue,
    FastLiquid,
    Strobe,
    Mirror,
    Tiles,
    Zoom,
    Rotate,
    Flip,
    RgbShift,
    Pixelate,
    Edge,
    GlitchWave,
    SoftColor,
    Vignette,
    SlowHue,
    SoftLiquid,
    SlowRotate,
    SoftTint,
    SoftPulse,
    Thermal,
    Crt,
    BlockGlitch,
    Grain,
    Shock,
    Fisheye,
    Hexagon,
    Halftone,
    Bloom,
    Droste,
    ZoomBlur,
    Ascii,
    Fold,
}

impl Ingredient {
    /// Ingrédients du thème « Tout » (sans le stroboscope, géré à part).
    const ALL: [Ingredient; 32] = [
        Ingredient::Droste,
        Ingredient::ZoomBlur,
        Ingredient::Ascii,
        Ingredient::Fold,
        Ingredient::Thermal,
        Ingredient::Crt,
        Ingredient::BlockGlitch,
        Ingredient::Shock,
        Ingredient::Fisheye,
        Ingredient::Halftone,
        Ingredient::Bloom,
        Ingredient::Swirl,
        Ingredient::Tunnel,
        Ingredient::Liquid,
        Ingredient::Acid,
        Ingredient::Rainbow,
        Ingredient::Kaleido,
        Ingredient::Echo,
        Ingredient::HueCycle,
        Ingredient::Pulse,
        Ingredient::Wave,
        Ingredient::Tint,
        Ingredient::Saturate,
        Ingredient::Posterize,
        Ingredient::Solarize,
        Ingredient::Invert,
        Ingredient::Mirror,
        Ingredient::Tiles,
        Ingredient::Rotate,
        Ingredient::RgbShift,
        Ingredient::Pixelate,
        Ingredient::Edge,
    ];

    fn apply(self, fx: &mut Effects, r: &mut Rng, k: f32) {
        match self {
            Ingredient::Swirl => {
                fx.swirl = r.signed(0.8, 3.0) * k;
                fx.swirl_speed = r.range(0.0, 0.3);
            }
            Ingredient::Tunnel => {
                fx.polar = r.range(0.6, 1.0).min(k);
                fx.tunnel_speed = r.signed(0.1, 0.5) * k;
            }
            Ingredient::Liquid => {
                fx.liquid = r.range(0.01, 0.04) * k;
                fx.liquid_scale = r.range(3.0, 12.0);
                fx.liquid_speed = r.range(0.5, 2.0);
            }
            Ingredient::Acid => {
                fx.acid = r.range(0.5, 1.0).min(k);
                fx.acid_speed = r.signed(0.05, 0.3);
            }
            Ingredient::Rainbow => fx.rainbow = r.range(0.4, 1.5) * k,
            Ingredient::Kaleido => {
                fx.kaleido = (3 + r.below(10)) as f32;
                fx.kaleido_spin = r.signed(0.0, 40.0) * k;
            }
            Ingredient::Echo => fx.echo = r.range(0.4, 1.0).min(k),
            Ingredient::HueCycle => fx.hue_speed = r.signed(0.05, 0.3) * k,
            Ingredient::Pulse => {
                fx.pulse = r.range(0.04, 0.15) * k;
                fx.pulse_rate = r.range(0.2, 1.0);
            }
            Ingredient::Wave => {
                fx.wave_amp = r.range(0.005, 0.025) * k;
                fx.wave_freq = r.range(1.0, 10.0);
            }
            Ingredient::HueShift => fx.hue = r.next(),
            Ingredient::Tint => {
                fx.tint = [r.next(), r.next(), r.next()];
                fx.tint_amount = r.range(0.5, 0.9).min(k);
            }
            Ingredient::Saturate => fx.saturation = 1.0 + r.range(0.4, 1.4) * k,
            Ingredient::Posterize => fx.posterize = (3 + r.below(5)) as f32,
            Ingredient::Solarize => fx.solarize = r.range(0.6, 1.0).min(k),
            Ingredient::Invert => fx.invert = 1.0,
            Ingredient::Contrast => fx.contrast = 1.0 + r.range(0.2, 0.8) * k,
            Ingredient::FastSpin => fx.rotate_speed = r.signed(60.0, 240.0) * k,
            Ingredient::FastKaleido => {
                fx.kaleido = (4 + r.below(8)) as f32;
                fx.kaleido_spin = r.signed(60.0, 180.0) * k;
            }
            Ingredient::FastPulse => {
                fx.pulse = r.range(0.06, 0.2) * k;
                fx.pulse_rate = r.range(1.5, 4.0);
            }
            Ingredient::FastWave => {
                fx.wave_amp = r.range(0.01, 0.03) * k;
                fx.wave_freq = r.range(2.0, 12.0);
                fx.wave_speed = r.range(8.0, 20.0);
            }
            Ingredient::FastTunnel => {
                fx.polar = 1.0;
                fx.tunnel_speed = r.signed(0.8, 2.0) * k;
            }
            Ingredient::FastHue => fx.hue_speed = r.signed(0.5, 1.0) * k,
            Ingredient::FastLiquid => {
                fx.liquid = r.range(0.015, 0.04) * k;
                fx.liquid_scale = r.range(4.0, 10.0);
                fx.liquid_speed = r.range(4.0, 8.0);
            }
            Ingredient::Strobe => fx.strobe = r.range(4.0, 12.0),
            Ingredient::Mirror => fx.mirror = Mirror::ALL[1 + r.below(3)],
            Ingredient::Tiles => fx.tiles = (2 + r.below(4)) as f32,
            Ingredient::Zoom => fx.zoom = r.range(0.5, 1.8),
            Ingredient::Rotate => fx.rotate = r.signed(10.0, 180.0),
            Ingredient::Flip => {
                fx.flip_x = r.next() < 0.5;
                fx.flip_y = !fx.flip_x || r.next() < 0.3;
            }
            Ingredient::RgbShift => fx.rgb_shift = r.range(0.004, 0.02) * k,
            Ingredient::Pixelate => fx.pixelate = r.range(0.3, 0.8).min(k),
            Ingredient::Edge => fx.edge = r.range(0.6, 1.0).min(k),
            Ingredient::GlitchWave => {
                fx.wave_amp = r.range(0.002, 0.008) * k;
                fx.wave_freq = r.range(20.0, 30.0);
                fx.wave_speed = r.range(8.0, 20.0);
            }
            Ingredient::SoftColor => {
                fx.saturation = r.range(1.05, 1.4);
                fx.contrast = r.range(0.85, 1.1);
                fx.brightness = r.range(0.0, 0.08);
            }
            Ingredient::Vignette => fx.vignette = r.range(0.3, 0.7),
            Ingredient::SlowHue => fx.hue_speed = r.signed(0.01, 0.05),
            Ingredient::SoftLiquid => {
                fx.liquid = r.range(0.003, 0.01) * k;
                fx.liquid_scale = r.range(2.0, 5.0);
                fx.liquid_speed = r.range(0.2, 0.6);
            }
            Ingredient::SlowRotate => fx.rotate_speed = r.signed(2.0, 8.0),
            Ingredient::SoftTint => {
                fx.tint = [r.range(0.5, 1.0), r.range(0.5, 1.0), r.range(0.5, 1.0)];
                fx.tint_amount = r.range(0.2, 0.4);
            }
            Ingredient::SoftPulse => {
                fx.pulse = r.range(0.02, 0.05);
                fx.pulse_rate = r.range(0.1, 0.3);
            }
            Ingredient::Thermal => fx.thermal = r.range(0.7, 1.0).min(k),
            Ingredient::Crt => fx.crt = r.range(0.5, 1.0).min(k),
            Ingredient::BlockGlitch => fx.glitch = r.range(0.3, 0.8) * k,
            Ingredient::Grain => fx.grain = r.range(0.2, 0.6) * k,
            Ingredient::Shock => {
                fx.shock = r.range(0.4, 0.9).min(k);
                fx.shock_speed = r.range(0.3, 1.2);
            }
            Ingredient::Fisheye => fx.fisheye = r.signed(0.4, 1.0) * k,
            Ingredient::Hexagon => fx.hexagon = r.range(0.2, 0.6),
            Ingredient::Halftone => fx.halftone = r.range(0.3, 0.7),
            Ingredient::Droste => {
                fx.droste = 1.0;
                fx.droste_speed = r.signed(0.1, 0.5) * k;
            }
            Ingredient::ZoomBlur => fx.zoom_blur = r.range(0.3, 0.8).min(k),
            Ingredient::Ascii => fx.ascii = r.range(0.2, 0.6),
            Ingredient::Fold => {
                fx.fold = r.range(0.3, 0.8).min(k);
                fx.fold_count = (r.range(3.0, 10.0)).round();
            }
            Ingredient::Bloom => {
                fx.bloom = r.range(0.4, 0.9) * k;
                fx.blur = r.range(0.0, 0.2);
            }
        }
    }
}

/// Thème du mode Auto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AutoTheme {
    /// Presets et effets inventés, tous genres.
    #[default]
    All,
    Psyche,
    Color,
    Speed,
    Geometry,
    Glitch,
    Soft,
}

impl AutoTheme {
    pub const ALL: [AutoTheme; 7] = [
        AutoTheme::All,
        AutoTheme::Psyche,
        AutoTheme::Color,
        AutoTheme::Speed,
        AutoTheme::Geometry,
        AutoTheme::Glitch,
        AutoTheme::Soft,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AutoTheme::All => "Tout",
            AutoTheme::Psyche => "Psychédélique",
            AutoTheme::Color => "Couleur",
            AutoTheme::Speed => "Vitesse",
            AutoTheme::Geometry => "Géométrie",
            AutoTheme::Glitch => "Glitch",
            AutoTheme::Soft => "Doux",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            AutoTheme::All => "Presets et effets inventés de tous les genres",
            AutoTheme::Psyche => "Tourbillons, tunnels, liquide, palette acide, échos, kaléidoscopes",
            AutoTheme::Color => "Couleurs seulement : teintes, palettes, solarisation… sans déformer l'image",
            AutoTheme::Speed => "Mouvements rapides : rotations, pulsations, tunnels et ondes accélérés",
            AutoTheme::Geometry => "Kaléidoscopes, miroirs, mosaïques, tunnels, rotations",
            AutoTheme::Glitch => "Décalage RVB, pixels, postérisation, contours, parasites",
            AutoTheme::Soft => "Ambiance calme : couleurs douces, mouvements lents",
        }
    }
}

/// Mode Auto : enchaîne des effets au hasard à intervalle régulier.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoFx {
    pub enabled: bool,
    /// Durée pendant laquelle un effet reste affiché, en secondes.
    pub interval: f32,
    /// Durée du fondu vers l'effet suivant, en secondes.
    pub transition: f32,
    /// Univers des effets choisis.
    pub theme: AutoTheme,
    /// Dosage des effets générés (1 = normal).
    pub intensity: f32,
    /// Thème « Tout » : part des changements qui inventent un nouvel effet au lieu de reprendre
    /// un preset (0..1).
    pub novelty: f32,
    /// Autorise les presets avec stroboscope (désactivé par défaut : flashs agressifs).
    pub strobe: bool,
}

impl Default for AutoFx {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: 8.0,
            transition: 2.0,
            theme: AutoTheme::All,
            intensity: 1.0,
            novelty: 0.25,
            strobe: false,
        }
    }
}

/// Un préréglage d'effets mémorisé, rappelable en live.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub effects: Effects,
}

/// Mode de fusion d'un calque avec ceux du dessous.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Screen,
    Multiply,
    Lighten,
    Darken,
}

impl BlendMode {
    pub const ALL: [BlendMode; 6] = [
        BlendMode::Normal,
        BlendMode::Add,
        BlendMode::Screen,
        BlendMode::Multiply,
        BlendMode::Lighten,
        BlendMode::Darken,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Add => "Addition",
            BlendMode::Screen => "Écran",
            BlendMode::Multiply => "Produit",
            BlendMode::Lighten => "Éclaircir",
            BlendMode::Darken => "Assombrir",
        }
    }
}

/// Presets fournis avec le logiciel.
/// Presets intégrés des animations sur la forme du masque (seuls les réglages `shape_*` comptent).
pub fn mask_anim_presets() -> Vec<Preset> {
    let p = |name: &str, f: &dyn Fn(&mut Effects)| {
        let mut effects = Effects::default();
        f(&mut effects);
        Preset { name: name.into(), effects }
    };
    vec![
        p("Néon", &|e| {
            e.shape_glow = 1.2;
        }),
        p("Néon arc-en-ciel", &|e| {
            e.shape_glow = 1.3;
            e.shape_rainbow = 1.0;
            e.shape_trace_speed = 0.15;
        }),
        p("Comètes", &|e| {
            e.shape_glow = 0.35;
            e.shape_trace = 1.5;
            e.shape_trace_count = 3.0;
            e.shape_trace_speed = 0.25;
        }),
        p("Pointillés", &|e| {
            e.shape_glow = 0.3;
            e.shape_dashes = 1.2;
            e.shape_dash_count = 40.0;
            e.shape_trace_speed = 0.12;
        }),
        p("Ondes", &|e| {
            e.shape_ripple = 0.8;
            e.shape_ripple_freq = 14.0;
            e.shape_ripple_speed = 0.6;
        }),
        p("Échos", &|e| {
            e.shape_glow = 0.6;
            e.shape_echo = 1.0;
            e.shape_ripple_freq = 10.0;
            e.shape_ripple_speed = 0.5;
        }),
        p("Aura", &|e| {
            e.shape_color = [1.0, 0.35, 0.8];
            e.shape_inner = 1.0;
            e.shape_glow = 0.5;
        }),
        p("Respiration", &|e| {
            e.shape_glow = 0.8;
            e.shape_breathe = 0.05;
            e.shape_breathe_rate = 0.3;
        }),
        p("Onde liquide", &|e| {
            e.shape_wave = 0.02;
            e.shape_ripple = 0.25;
            e.shape_ripple_freq = 10.0;
            e.shape_ripple_speed = 0.8;
        }),
        p("Radar", &|e| {
            e.shape_color = [0.3, 1.0, 0.5];
            e.shape_glow = 0.5;
            e.shape_trace = 2.0;
            e.shape_trace_count = 1.0;
            e.shape_trace_speed = 0.5;
            e.shape_echo = 0.5;
        }),
        p("Fête", &|e| {
            e.shape_rainbow = 1.0;
            e.shape_glow = 1.0;
            e.shape_trace = 1.0;
            e.shape_trace_count = 4.0;
            e.shape_echo = 0.6;
            e.shape_breathe = 0.03;
            e.shape_breathe_rate = 0.5;
        }),
    ]
}

pub fn builtin_presets() -> Vec<Preset> {
    let d = Effects::default;
    let p = |name: &str, effects: Effects| Preset {
        name: name.to_owned(),
        effects,
    };
    vec![
        p("Néon", Effects { edge: 1.0, saturation: 2.2, contrast: 1.4, hue_speed: 0.05, ..d() }),
        p("Kaléido", Effects { kaleido: 6.0, kaleido_spin: 20.0, saturation: 1.3, ..d() }),
        p("Mandala", Effects { kaleido: 12.0, kaleido_spin: -10.0, zoom: 0.7, vignette: 0.7, saturation: 1.5, ..d() }),
        p("Miroir 4", Effects { mirror: Mirror::Quad, ..d() }),
        p("Tunnel", Effects { zoom: 0.45, rotate_speed: 40.0, vignette: 0.9, contrast: 1.2, ..d() }),
        p("Rétro VHS", Effects {
            rgb_shift: 0.006,
            wave_amp: 0.002,
            wave_freq: 25.0,
            wave_speed: 9.0,
            saturation: 0.7,
            contrast: 1.2,
            vignette: 0.5,
            ..d()
        }),
        p("Glitch", Effects { rgb_shift: 0.02, pixelate: 0.35, contrast: 1.5, saturation: 1.4, ..d() }),
        p("8-bit", Effects { pixelate: 0.85, posterize: 6.0, saturation: 1.3, ..d() }),
        p("Pop art", Effects { posterize: 4.0, saturation: 2.2, contrast: 1.3, ..d() }),
        p("Arc-en-ciel", Effects { hue_speed: 0.25, saturation: 1.8, ..d() }),
        p("Psyché", Effects {
            kaleido: 4.0,
            wave_amp: 0.02,
            wave_freq: 3.0,
            rgb_shift: 0.01,
            saturation: 2.0,
            hue_speed: 0.1,
            ..d()
        }),
        p("Rêve", Effects {
            wave_amp: 0.01,
            wave_freq: 2.0,
            wave_speed: 1.0,
            brightness: 0.08,
            contrast: 0.85,
            saturation: 1.3,
            vignette: 0.6,
            ..d()
        }),
        p("Noir & blanc", Effects { saturation: 0.0, contrast: 1.4, ..d() }),
        p("Duotone rose", Effects { tint: [1.0, 0.2, 0.6], tint_amount: 0.85, contrast: 1.3, ..d() }),
        p("Bleu glacé", Effects { tint: [0.3, 0.65, 1.0], tint_amount: 0.6, saturation: 0.6, brightness: 0.05, ..d() }),
        p("Négatif", Effects { invert: 1.0, ..d() }),
        p("Strobe", Effects { strobe: 8.0, contrast: 1.3, ..d() }),
        p("Vortex", Effects { swirl: 2.5, swirl_speed: 0.15, hue_speed: 0.08, saturation: 1.4, ..d() }),
        p("Tunnel acide", Effects { polar: 1.0, tunnel_speed: 0.35, acid: 0.8, acid_speed: 0.15, ..d() }),
        p("Liquide", Effects { liquid: 0.035, liquid_scale: 6.0, liquid_speed: 1.5, saturation: 1.4, ..d() }),
        p("Acide", Effects { acid: 1.0, acid_speed: 0.25, contrast: 1.2, ..d() }),
        p("Mosaïque", Effects { tiles: 3.0, rainbow: 0.6, saturation: 1.3, ..d() }),
        p("Écho", Effects { echo: 0.9, pulse: 0.06, pulse_rate: 0.4, ..d() }),
        p("Solarisé", Effects { solarize: 1.0, saturation: 1.6, ..d() }),
        p("Respiration", Effects { pulse: 0.1, pulse_rate: 0.25, vignette: 0.5, ..d() }),
        p("Trip", Effects {
            swirl: 1.2,
            swirl_speed: 0.1,
            liquid: 0.02,
            liquid_scale: 5.0,
            acid: 0.5,
            acid_speed: 0.1,
            rainbow: 0.5,
            kaleido: 6.0,
            kaleido_spin: 15.0,
            ..d()
        }),
        p("Cube 3D", Effects { obj3d: Object3d::Cube, rot3d_x: 20.0, spin3d_y: 30.0, spin3d_x: 8.0, ..d() }),
        p("Sphère", Effects { obj3d: Object3d::Sphere, rot3d_x: 15.0, spin3d_y: 25.0, ..d() }),
        p("Carte 3D", Effects { obj3d: Object3d::Plane, rot3d_x: 25.0, spin3d_y: 20.0, dist3d: 3.0, ..d() }),
        p("Cylindre", Effects { obj3d: Object3d::Cylinder, rot3d_x: 10.0, spin3d_y: 35.0, ..d() }),
        p("Relief", Effects { relief: 0.8, contrast: 1.1, ..d() }),
        p("Anaglyphe", Effects { anaglyph: 0.012, contrast: 1.1, ..d() }),
        p("Drapeau", Effects { flag: 0.8, flag_speed: 2.5, ..d() }),
        p("Thermique", Effects { thermal: 1.0, contrast: 1.2, blur: 0.15, ..d() }),
        p("Télé cathodique", Effects { crt: 0.9, rgb_shift: 0.003, grain: 0.25, vignette: 0.5, ..d() }),
        p("Glitch numérique", Effects { glitch: 0.7, rgb_shift: 0.008, grain: 0.2, ..d() }),
        p("Pellicule", Effects { grain: 0.6, saturation: 0.7, contrast: 1.15, vignette: 0.6, tint: [1.0, 0.85, 0.6], tint_amount: 0.25, ..d() }),
        p("Onde de choc", Effects { shock: 0.8, shock_speed: 0.6, ..d() }),
        p("Fish-eye", Effects { fisheye: 0.8, vignette: 0.4, ..d() }),
        p("Hexagones", Effects { hexagon: 0.4, saturation: 1.3, ..d() }),
        p("Trame BD", Effects { halftone: 0.5, posterize: 6.0, edge: 0.4, saturation: 1.4, ..d() }),
        p("Droste", Effects { droste: 1.0, droste_speed: 0.3, ..d() }),
        p("Zoom radial", Effects { zoom_blur: 0.6, saturation: 1.2, ..d() }),
        p("ASCII", Effects { ascii: 0.4, contrast: 1.3, ..d() }),
        p("Tore", Effects { obj3d: Object3d::Torus, rot3d_x: 55.0, spin3d_y: 30.0, ..d() }),
        p("Accordéon", Effects { fold: 0.6, fold_count: 6.0, ..d() }),
        p("Terrain 3D", Effects { obj3d: Object3d::Terrain, rot3d_x: -50.0, spin3d_y: 8.0, height3d: 0.35, dist3d: 2.8, ..d() }),
        p("Pixels 3D", Effects { obj3d: Object3d::Voxels, rot3d_x: -45.0, spin3d_y: 12.0, height3d: 0.3, voxels: 40.0, dist3d: 2.8, ..d() }),
        p("Halo", Effects { bloom: 0.8, blur: 0.2, brightness: 0.05, saturation: 1.2, ..d() }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_preset_names_are_unique() {
        let presets = builtin_presets();
        let mut names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), presets.len());
    }

    #[test]
    fn three_d_presets_switch_object_midway() {
        let presets = builtin_presets();
        let cube = presets.iter().find(|p| p.name == "Cube 3D").unwrap().effects;
        assert_eq!(cube.obj3d, Object3d::Cube);
        let d = Effects::default();
        assert_eq!(Effects::lerp(&d, &cube, 0.2).obj3d, Object3d::Off);
        assert_eq!(Effects::lerp(&d, &cube, 0.8).obj3d, Object3d::Cube);
        assert!(Effects::lerp(&d, &cube, 0.5).spin3d_y > 0.0);
    }

    #[test]
    fn mask_presets_only_touch_shape_animations() {
        let base = Effects { zoom: 2.0, shatter_amount: 0.4, ..Effects::default() };
        for p in mask_anim_presets() {
            assert!(p.effects.uses_shape(), "{} n'anime pas la forme", p.name);
            let fx = base.with_mask_anim_of(&p.effects);
            assert_eq!((fx.zoom, fx.shatter_amount), (2.0, 0.4), "{}", p.name);
            assert!(fx.same_mask_anim(&p.effects));
        }
    }

    #[test]
    fn random_effects_are_varied_and_never_neutral() {
        let all: Vec<Effects> = (1..200u64).map(|s| Effects::random(s.wrapping_mul(0x9E37_79B9_7F4A_7C15))).collect();
        assert!(all.iter().all(|fx| *fx != Effects::default()));
        let distinct = all.iter().enumerate().filter(|(i, fx)| !all[..*i].contains(fx)).count();
        assert!(distinct > 190, "seulement {distinct} effets différents");
    }

    #[test]
    fn color_theme_never_deforms_the_image() {
        let d = Effects::default();
        for seed in 1..300u64 {
            let fx = Effects::themed(AutoTheme::Color, seed * 7919, 1.5, true);
            assert_ne!(fx, d);
            let geometry_untouched = fx.zoom == d.zoom
                && fx.rotate == d.rotate
                && fx.rotate_speed == d.rotate_speed
                && fx.kaleido == d.kaleido
                && fx.swirl == d.swirl
                && fx.polar == d.polar
                && fx.tiles == d.tiles
                && fx.liquid == d.liquid
                && fx.wave_amp == d.wave_amp
                && fx.pixelate == d.pixelate
                && fx.mirror == d.mirror
                && !fx.flip_x
                && !fx.flip_y;
            assert!(geometry_untouched, "{fx:?}");
        }
    }

    #[test]
    fn shape_settings_survive_other_effects() {
        let mine = Effects { shape_glow: 1.2, shape_trace: 0.8, shape_color: [1.0, 0.0, 0.5], ..Effects::default() };
        for seed in 1..50u64 {
            let fx = Effects::themed(AutoTheme::All, seed * 13, 1.0, true).with_shape_of(&mine);
            assert_eq!(fx.with_shape_of(&Effects::default()), Effects::themed(AutoTheme::All, seed * 13, 1.0, true));
            assert_eq!(Effects::default().with_shape_of(&fx), Effects::default().with_shape_of(&mine));
        }
    }

    #[test]
    fn shatter_cycle_breaks_holds_and_rebuilds() {
        assert_eq!(Effects::shatter_cycle(0.0), 0.0);
        assert_eq!(Effects::shatter_cycle(0.6), 1.0);
        assert!(Effects::shatter_cycle(0.3) > 0.0 && Effects::shatter_cycle(0.3) < 1.0);
        assert!(Effects::shatter_cycle(0.999) < 0.01);
        // Sans à-coups : petites variations entre deux images.
        for k in 0..1000 {
            let (a, b) = (k as f32 / 1000.0, (k + 1) as f32 / 1000.0);
            assert!((Effects::shatter_cycle(a) - Effects::shatter_cycle(b)).abs() < 0.01);
        }
        // La destruction fait partie des effets de masque : le mode Auto n'y touche pas.
        let mine = Effects { shatter: Shatter::Glass, shatter_speed: 0.2, ..Effects::default() };
        assert_eq!(Effects::random(42).with_shape_of(&mine).shatter, Shatter::Glass);
    }

    #[test]
    fn strobe_only_when_allowed() {
        for theme in AutoTheme::ALL {
            for seed in 1..200u64 {
                assert_eq!(Effects::themed(theme, seed * 31, 1.0, false).strobe, 0.0, "{theme:?}");
            }
        }
        let with = (1..200u64).any(|s| Effects::themed(AutoTheme::Speed, s * 31, 1.0, true).strobe > 0.0);
        assert!(with, "le thème Vitesse doit pouvoir utiliser le stroboscope s'il est autorisé");
    }
}
