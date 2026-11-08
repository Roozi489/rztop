//! Theme thumbnails for the Settings window: one per bundled theme, each
//! drawn in that theme's own colours: its background and text, a meter
//! strip for every colour role, and a miniature title bar.

use super::{Gradient, Look, segments};
use crate::config::Config;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui,
    text::{LayoutJob, TextFormat},
    vec2,
};

const THUMB: egui::Vec2 = vec2(232.0, 92.0);
const PAD: f32 = 8.0;
const STRIP_H: f32 = 7.0;
const ROW_H: f32 = 12.0;
const BAR_H: f32 = 18.0;

/// Every theme's colours, built the first time the grid is shown and
/// again after a user theme is saved or deleted.
#[derive(Default)]
pub(super) struct ThemeGrid {
    looks: Vec<(String, Look)>,
}

impl ThemeGrid {
    /// Rebuild the thumbnails next time, to pick up user theme changes.
    pub(super) fn refresh(&mut self) {
        self.looks.clear();
    }

    /// A theme's colours, once the grid has been shown.
    pub(super) fn look(&self, name: &str) -> Option<&Look> {
        self.looks
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, look)| look)
    }

    /// Draw the thumbnails, wrapping to the available width. Returns the
    /// theme the user clicked and the one under the pointer.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        config: &Config,
        current: &str,
    ) -> (Option<String>, Option<String>) {
        if self.looks.is_empty() {
            self.looks = crate::theme::names()
                .into_iter()
                .map(|name| {
                    let look = Look::with_palette(config, &crate::theme::palette(&name));
                    (name, look)
                })
                .collect();
        }
        let mut picked = None;
        let mut hovered = None;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = vec2(PAD, PAD);
            ui.horizontal_wrapped(|ui| {
                for (name, look) in &self.looks {
                    let selected = name.eq_ignore_ascii_case(current);
                    let response = thumbnail(ui, name, look, selected);
                    if response.clicked() {
                        picked = Some(name.clone());
                    }
                    if response.hovered() {
                        hovered = Some(name.clone());
                    }
                }
            });
        });
        (picked, hovered)
    }
}

/// The colour roles in the order the thumbnail shows them.
fn roles(look: &Look) -> [(&'static str, &Gradient); 9] {
    [
        ("Load", &look.load),
        ("Sys", &look.load2),
        ("Temp", &look.temp),
        ("Power", &look.power),
        ("Clock", &look.clock),
        ("Fill", &look.fill),
        ("Comprs", &look.mem_cache),
        ("In", &look.inbound),
        ("Out", &look.outbound),
    ]
}

fn thumbnail(ui: &mut Ui, name: &str, look: &Look, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(THUMB, Sense::click());
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(name);
    let painter = ui.painter_at(rect.expand(2.0));
    let small = FontId::monospace(10.0);
    painter.rect_filled(rect, CornerRadius::same(4), look.bg);

    // Miniature title bar: the theme's name, the user's own themes tagged,
    // and a `CPU 1.2%` readout in the bar's colours.
    let bar = Rect::from_min_max(
        rect.left_top(),
        egui::pos2(rect.right(), rect.top() + BAR_H),
    );
    painter.rect_filled(
        bar,
        CornerRadius {
            nw: 4,
            ne: 4,
            ..Default::default()
        },
        look.statusbar_bg,
    );
    let mut job = LayoutJob::default();
    job.append(
        name,
        0.0,
        TextFormat::simple(FontId::monospace(11.5), look.statusbar_fg),
    );
    if !crate::theme::is_bundled(name) {
        job.append(
            " custom",
            0.0,
            TextFormat::simple(small.clone(), look.statusbar_sep),
        );
    }
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(bar.left() + PAD, bar.center().y - galley.size().y / 2.0),
        galley,
        look.statusbar_fg,
    );
    let mut job = LayoutJob::default();
    let parts: [(&str, Color32); 2] = [("CPU ", look.statusbar_sep), ("1.2%", look.statusbar_fg)];
    for (text, color) in parts {
        job.append(text, 0.0, TextFormat::simple(small.clone(), color));
    }
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(
            bar.right() - PAD - galley.size().x,
            bar.center().y - galley.size().y / 2.0,
        ),
        galley,
        look.statusbar_fg,
    );

    // Two columns of role strips.
    let top = bar.bottom() + 5.0;
    let col_w = (rect.width() - 3.0 * PAD) / 2.0;
    let label_w = 36.0;
    for (i, (label, grad)) in roles(look).into_iter().enumerate() {
        let (col, row) = (i / 5, i % 5);
        let left = rect.left() + PAD + col as f32 * (col_w + PAD);
        let cy = top + row as f32 * ROW_H + ROW_H / 2.0;
        painter.text(
            egui::pos2(left, cy),
            Align2::LEFT_CENTER,
            label,
            small.clone(),
            look.label,
        );
        let strip = Rect::from_min_max(
            egui::pos2(left + label_w, cy - STRIP_H / 2.0),
            egui::pos2(left + col_w, cy + STRIP_H / 2.0),
        );
        segments(&painter, strip, 100, grad, look.meter_off);
    }

    // Border: the theme's highlight when current, its text colour on hover.
    let stroke = if selected {
        Stroke::new(2.0_f32, look.hi)
    } else if response.hovered() {
        Stroke::new(1.5_f32, look.fg)
    } else {
        Stroke::new(1.0_f32, look.dim)
    };
    painter.rect_stroke(rect, CornerRadius::same(4), stroke, StrokeKind::Outside);
    response
}
