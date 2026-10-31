//! Petits widgets réutilisables.

use super::*;

/// Onglet latéral : icône au-dessus du nom, barre d'accent quand il est choisi.
pub(super) fn side_tab(ui: &mut egui::Ui, icon: &str, name: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), Sense::click());
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
    p.text(rect.center() - Vec2::new(0.0, 8.0), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(20.0), color);
    p.text(
        rect.center() + Vec2::new(0.0, 13.0),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(11.0),
        if selected { TEXT } else { TEXT_WEAK },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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

/// Slider de pourcentage (valeur stockée entre 0 et 1).
pub(super) fn percent_slider(ui: &mut egui::Ui, value: &mut f32, label: &str) -> egui::Response {
    ui.add(
        egui::Slider::new(value, 0.0..=1.0)
            .text(label)
            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
            .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
    )
}

/// Slider d'effet avec bouton de remise à zéro. Renvoie true si l'utilisateur l'a modifié.
pub(super) fn fx_slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, default: f32) -> bool {
    ui.horizontal(|ui| {
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
