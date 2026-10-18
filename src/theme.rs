//! Apparence de l'interface : palette, police d'icônes, espacements et petits composants.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke, TextStyle};

pub use crate::surface::ACCENT;

pub const BG: Color32 = Color32::from_rgb(13, 15, 19);
pub const PANEL: Color32 = Color32::from_rgb(21, 24, 30);
pub const CARD: Color32 = Color32::from_rgb(27, 31, 38);
pub const WIDGET: Color32 = Color32::from_rgb(36, 41, 50);
pub const WIDGET_HOVER: Color32 = Color32::from_rgb(46, 52, 63);
pub const WIDGET_ACTIVE: Color32 = Color32::from_rgb(56, 64, 78);
pub const BORDER: Color32 = Color32::from_rgb(40, 45, 55);
pub const TEXT: Color32 = Color32::from_rgb(222, 226, 233);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(132, 140, 154);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(26, 92, 64);
pub const WARN: Color32 = Color32::from_rgb(255, 170, 60);
pub const DANGER: Color32 = Color32::from_rgb(255, 95, 95);

pub fn apply(ctx: &egui::Context) {
    // Police d'icônes Phosphor en repli : les icônes s'utilisent directement dans le texte.
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "phosphor".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/Phosphor.ttf")).tweak(egui::FontTweak {
            y_offset_factor: 0.08,
            ..Default::default()
        })),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("phosphor".into());
    }
    ctx.set_fonts(fonts);

    ctx.global_style_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(17.0)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(8.0, 7.0);
        s.button_padding = egui::vec2(9.0, 4.0);
        s.interact_size.y = 24.0;
        s.slider_width = 130.0;
        s.combo_width = 110.0;
        s.window_margin = egui::Margin::same(12);
        s.indent = 14.0;

        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = PANEL;
        v.window_fill = CARD;
        v.extreme_bg_color = BG;
        v.faint_bg_color = CARD;
        v.override_text_color = None;
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT_DIM;
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.slider_trailing_fill = true;

        let radius = CornerRadius::same(6);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = radius;
        for (state, fill) in [
            (&mut w.inactive, WIDGET),
            (&mut w.hovered, WIDGET_HOVER),
            (&mut w.active, WIDGET_ACTIVE),
            (&mut w.open, WIDGET_HOVER),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.corner_radius = radius;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.0, TEXT);
            state.expansion = 0.0;
        }
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(70, 78, 92));
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    });
}

/// Icône suivie d'un texte.
pub fn with_icon(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}

/// Titre de section : icône colorée + texte.
pub fn section(ui: &mut egui::Ui, icon: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(17.0).color(ACCENT));
        ui.label(RichText::new(title).size(15.5).strong().color(TEXT));
    });
}

/// Bouton principal (fond vert).
pub fn primary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(BG).strong()).fill(ACCENT)
}

/// Bouton icône seul, discret.
pub fn icon_button(icon: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(icon).size(15.0))
}

/// Bloc à fond légèrement plus clair, pour regrouper des réglages.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10))
        .stroke(Stroke::new(1.0, BORDER))
}

/// Couleur selon un niveau de charge en % (vert, orange, rouge).
pub fn load_color(percent: f32) -> Color32 {
    if percent < 50.0 {
        ACCENT
    } else if percent < 80.0 {
        WARN
    } else {
        DANGER
    }
}
