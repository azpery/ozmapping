//! Petits widgets réutilisables.

use super::*;

/// Onglet latéral : icône au-dessus du nom, barre d'accent quand il est choisi.
/// `h` : hauteur de l'onglet (réduite quand la fenêtre est basse, pour que tous restent visibles).
pub(super) fn side_tab(ui: &mut egui::Ui, icon: &str, name: &str, selected: bool, h: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 6.0, theme::ACCENT_DIM.gamma_multiply(0.55));
        // Barre d'accent côté bord de la fenêtre (les onglets sont à droite).
        let bar = Rect::from_min_size(egui::pos2(rect.max.x - 3.0, rect.min.y + 8.0), Vec2::new(3.0, rect.height() - 16.0));
        p.rect_filled(bar, 1.5, ACCENT);
    } else if resp.hovered() {
        p.rect_filled(rect, 6.0, theme::WIDGET);
    }
    let color = if selected { ACCENT } else if resp.hovered() { TEXT } else { TEXT_WEAK };
    let k = (h / 50.0).clamp(0.7, 1.0);
    p.text(rect.center() - Vec2::new(0.0, 8.0 * k), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(20.0 * k), color);
    p.text(
        rect.center() + Vec2::new(0.0, 13.0 * k),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(11.0),
        if selected { TEXT } else { TEXT_WEAK },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Petit titre d'une famille dans une liste rangée (presets, visuels).
pub(super) fn family_title(ui: &mut egui::Ui, title: &str) {
    ui.add_space(3.0);
    ui.label(egui::RichText::new(title.to_uppercase()).size(10.5).strong().color(TEXT_WEAK));
}

/// Jauge CPU : l'app avec ses décodeurs ffmpeg, sur fond de charge totale de la machine.
pub(super) fn cpu_gauge(ui: &mut egui::Ui, stats: CpuStats) {
    // Couleur selon la charge de l'app ; la partie grise de la barre montre le reste de la machine.
    let color = theme::load_color(stats.app);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(96.0, 8.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, theme::WIDGET);
    let w = rect.width();
    let sys = egui::Rect::from_min_size(rect.min, Vec2::new(w * stats.system / 100.0, rect.height()));
    p.rect_filled(sys, 4.0, TEXT_WEAK.gamma_multiply(0.35));
    let app = egui::Rect::from_min_size(rect.min, Vec2::new(w * stats.app / 100.0, rect.height()));
    p.rect_filled(app, 4.0, color);
    resp.on_hover_text(format!(
        "OZ Mapping + {} décodeur(s) ffmpeg : {:.0} % de la machine\nMachine entière : {:.0} %\n{} cœurs",
        stats.decoders, stats.app, stats.system, stats.cores
    ));
    ui.label(egui::RichText::new(format!("{:.0} %", stats.app)).small().color(color));
    ui.label(egui::RichText::new(ic::CPU).size(16.0).color(color));
}

/// Place prévue pour la case de valeur d'un slider (« 30 % », « 12 temps »…).
const SLIDER_VALUE_W: f32 = 64.0;
/// Barre de slider la plus courte qu'on s'autorise.
const SLIDER_MIN_W: f32 = 40.0;

/// Raccourcit la barre des sliders de `ui` pour que barre + valeur + `label` + `extra`
/// tiennent dans la largeur disponible (le libellé d'un slider ne passe jamais à la ligne).
fn fit_slider_width(ui: &mut egui::Ui, label: &str, extra: f32) {
    let label_w = egui::WidgetText::from(label)
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body)
        .size()
        .x;
    let gap = ui.spacing().item_spacing.x;
    let room = ui.available_width() - label_w - SLIDER_VALUE_W - 2.0 * gap - extra;
    let s = ui.spacing_mut();
    s.slider_width = room.clamp(SLIDER_MIN_W, s.slider_width.max(SLIDER_MIN_W));
}

/// Slider avec libellé dont la barre s'adapte à la largeur du panneau.
pub(super) fn fit_slider(ui: &mut egui::Ui, label: &str, slider: egui::Slider<'_>) -> egui::Response {
    ui.scope(|ui| {
        fit_slider_width(ui, label, 0.0);
        ui.add(slider.text(label))
    })
    .inner
}

/// Slider de pourcentage (valeur stockée entre 0 et 1).
pub(super) fn percent_slider(ui: &mut egui::Ui, value: &mut f32, label: &str) -> egui::Response {
    fit_slider(
        ui,
        label,
        egui::Slider::new(value, 0.0..=1.0)
            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
            .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
    )
}

/// Slider d'effet avec bouton de remise à zéro. Renvoie true si l'utilisateur l'a modifié.
pub(super) fn fx_slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, default: f32) -> bool {
    ui.horizontal(|ui| {
        // Place gardée pour le bouton de réinitialisation.
        fit_slider_width(ui, label, 28.0);
        // Clamping « Edits » : le slider ne réécrit pas la valeur tant qu'on n'y touche pas
        // (sinon il l'arrondit à chaque image et interrompt les transitions de preset).
        let mut changed = ui
            .add(egui::Slider::new(value, range).text(label).clamping(egui::SliderClamping::Edits))
            .changed();
        if *value != default
            && ui
                .add(theme::icon_button(ic::ARROW_COUNTER_CLOCKWISE).small().frame(false))
                .on_hover_text("Réinitialiser")
                .clicked()
        {
            *value = default;
            changed = true;
        }
        changed
    })
    .inner
}

/// Gros bouton de remise à zéro, toute la largeur, à placer en tête d'onglet.
/// Rouge quand il y a quelque chose à retirer (`active`), discret sinon.
pub(super) fn reset_button(ui: &mut egui::Ui, text: &str, hover: &str, active: bool) -> bool {
    let color = if active { theme::DANGER } else { TEXT_WEAK };
    let label = egui::RichText::new(with_icon(ic::PROHIBIT, text)).size(15.0).strong().color(color);
    let mut button = egui::Button::new(label).min_size(Vec2::new(ui.available_width(), 34.0));
    if active {
        button = button.fill(theme::DANGER.gamma_multiply(0.18)).stroke(Stroke::new(1.0, theme::DANGER.gamma_multiply(0.6)));
    }
    let clicked = ui.add(button).on_hover_text(hover).clicked();
    ui.add_space(6.0);
    clicked
}

pub(super) fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
