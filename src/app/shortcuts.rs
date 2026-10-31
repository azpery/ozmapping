//! Raccourcis clavier.

use super::*;

impl OzApp {
    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |shift: bool, key: Key| {
            let mods = if shift { Modifiers::COMMAND | Modifiers::SHIFT } else { Modifiers::COMMAND };
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)))
        };
        if cmd(true, Key::S) {
            self.save(true);
        }
        if cmd(false, Key::S) {
            self.save(false);
        }
        if cmd(false, Key::O) {
            self.open_project(ctx);
        }
        if cmd(false, Key::N) {
            self.new_project(ctx);
        }

        if ctx.egui_wants_keyboard_input() || ctx.input(|i| i.modifiers.command) {
            return;
        }
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if pressed(Key::Space) {
            self.toggle_pause();
        }
        if pressed(Key::B) {
            self.blackout = !self.blackout;
        }
        if pressed(Key::W) {
            self.white_screen = !self.white_screen;
        }
        if pressed(Key::G) {
            self.grid = !self.grid;
        }
        if pressed(Key::R) {
            self.toggle_reverse();
        }
        // Sans la répétition automatique : garder T enfoncé ne compte qu'un tap.
        let tapped = ctx.input(|i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::T, pressed: true, repeat: false, .. }))
        });
        if tapped {
            self.tap();
        }
        if pressed(Key::E) {
            self.bypass_fx = !self.bypass_fx;
        }
        if pressed(Key::Num0) {
            self.clear_effects();
        }
        if pressed(Key::A) {
            let i = self.sel();
            let auto = &mut self.project.layers[i].auto;
            auto.enabled = !auto.enabled;
            if auto.enabled {
                self.auto_next(i);
            }
        }
        if pressed(Key::M) {
            self.edit_mapping = !self.edit_mapping;
        }
        if pressed(Key::Tab) {
            self.selected = (self.sel() + 1) % self.project.layers.len();
        }
        if pressed(Key::F) {
            self.set_fullscreen(ctx, !self.output_fullscreen);
        }
        if pressed(Key::Escape) && self.output_fullscreen {
            self.set_fullscreen(ctx, false);
        }
        if pressed(Key::ArrowRight) {
            self.launch_relative(ctx, 1);
        }
        if pressed(Key::ArrowLeft) {
            self.launch_relative(ctx, -1);
        }
        let digits = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (i, k) in digits.into_iter().enumerate() {
            if pressed(k) {
                self.launch_media(ctx, i);
            }
        }
    }
}
