//! Theme editor: edits a copy of a theme's colours with the whole app
//! previewing them live, then saves it as a user theme.

use super::{Gradient, Look, segments, text};
use crate::themes::{GradientDef, Rgb, ThemePalette};
use eframe::egui::{self, Color32, Ui, color_picker::Alpha, vec2};

/// What the user did in the editor this frame.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    None,
    /// A colour changed: refresh the preview.
    Changed,
    Save,
    Cancel,
    Delete,
}

pub(super) struct ThemeEditor {
    /// Name to save under.
    pub(super) name: String,
    /// The theme this edit started from.
    pub(super) from: String,
    pub(super) palette: ThemePalette,
    /// Why the last save or delete failed.
    pub(super) error: Option<String>,
}

impl ThemeEditor {
    /// Edit `from`: a user theme is edited in place, a bundled one as a copy.
    pub(super) fn new(from: &str) -> Self {
        let name = if crate::theme::is_bundled(from) {
            format!("{from}_custom")
        } else {
            from.to_owned()
        };
        Self {
            name,
            from: from.to_owned(),
            palette: crate::theme::palette(from),
            error: None,
        }
    }

    pub(super) fn show(&mut self, ui: &mut Ui, look: &Look) -> Action {
        let mut action = Action::None;
        let what = if crate::theme::is_bundled(&self.from) {
            format!("Editing a copy of {}", self.from)
        } else {
            format!("Editing {}", self.from)
        };
        ui.label(
            text(
                format!("{what}. The whole window previews your changes until you save or cancel."),
                look.label,
            )
            .size(11.5),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(text("Name", look.fg));
            ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .desired_width(220.0)
                    .font(egui::TextStyle::Monospace),
            );
            let valid =
                crate::theme::valid_name(&self.name) && !crate::theme::is_bundled(&self.name);
            if ui
                .add_enabled(valid, egui::Button::new("Save"))
                .on_disabled_hover_text(
                    "Use letters, digits, _ or -, and not a bundled theme's name",
                )
                .clicked()
            {
                action = Action::Save;
            }
            if ui.button("Cancel").clicked() {
                action = Action::Cancel;
            }
            let existing = !crate::theme::is_bundled(&self.from) && self.name == self.from;
            if existing && ui.button("Delete theme").clicked() {
                action = Action::Delete;
            }
        });
        if let Some(error) = &self.error {
            ui.label(text(error, look.warn));
        }
        ui.add_space(12.0);

        ui.label(text("Base colours", look.fg).strong());
        ui.add_space(4.0);
        let c = &mut self.palette.colors;
        let base: [(&str, &mut Rgb); 11] = [
            ("Background", &mut c.main_bg),
            ("Text", &mut c.main_fg),
            ("Labels", &mut c.data_label_fg),
            ("Dim text", &mut c.graph_text),
            ("Meter off", &mut c.meter_bg),
            ("Selection", &mut c.selected_bg),
            ("Highlight", &mut c.hi_fg),
            ("Title bar", &mut c.statusbar_bg),
            ("Title text", &mut c.statusbar_fg),
            ("Title accent", &mut c.statusbar_hi),
            ("Title labels", &mut c.statusbar_sep),
        ];
        egui::Grid::new("theme-base")
            .num_columns(6)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                for (i, (name, rgb)) in base.into_iter().enumerate() {
                    ui.label(text(name, look.fg));
                    if colour(ui, rgb) {
                        action = action_or_changed(action);
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(12.0);

        ui.label(text("Gradients", look.fg).strong());
        ui.label(
            text(
                "Start, middle and end colour of each role; the strip shows the full range.",
                look.label,
            )
            .size(11.5),
        );
        ui.add_space(4.0);
        let g = &mut self.palette.gradients;
        let roles: [(&str, &str, &mut GradientDef); 10] = [
            ("Load", "CPU, GPU, cores, disk busy, ping", &mut g.cpu_upper),
            ("Sys", "System half of the CPU graph", &mut g.cpu_lower),
            ("Temp", "Temperatures; warnings use its middle", &mut g.temp),
            ("Power", "CPU and GPU watts", &mut g.gpu_power),
            ("Clock", "CPU and GPU clocks", &mut g.gpu_clock),
            ("Fill", "Used, commit, swap, VRAM, disks", &mut g.used),
            ("Avail", "Available memory", &mut g.available),
            ("Cache", "Cached memory", &mut g.cached),
            ("In", "Download, disk read, I/O read", &mut g.download),
            ("Out", "Upload, disk write, I/O write", &mut g.upload),
        ];
        let off = super::rgb(self.palette.colors.meter_bg);
        let off = super::mix(super::rgb(self.palette.colors.main_bg), off, 0.6);
        egui::Grid::new("theme-gradients")
            .num_columns(6)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for (name, used_for, def) in roles {
                    ui.label(text(name, look.fg));
                    for stop in [&mut def.start, &mut def.mid, &mut def.end] {
                        if colour(ui, stop) {
                            action = action_or_changed(action);
                        }
                    }
                    let (strip, _) = ui.allocate_exact_size(vec2(180.0, 9.0), egui::Sense::hover());
                    segments(ui.painter(), strip, 100, &Gradient::new(def), off);
                    ui.label(text(used_for, look.label).size(11.5));
                    ui.end_row();
                }
            });
        action
    }
}

/// A colour swatch that opens a picker; true when the colour changed.
fn colour(ui: &mut Ui, rgb: &mut Rgb) -> bool {
    let mut c = Color32::from_rgb(rgb.0, rgb.1, rgb.2);
    let changed = egui::color_picker::color_edit_button_srgba(ui, &mut c, Alpha::Opaque)
        .on_hover_text(rgb.to_string())
        .changed();
    if changed {
        *rgb = Rgb(c.r(), c.g(), c.b());
    }
    changed
}

/// A colour change never hides a button press from the same frame.
fn action_or_changed(action: Action) -> Action {
    if action == Action::None {
        Action::Changed
    } else {
        action
    }
}
