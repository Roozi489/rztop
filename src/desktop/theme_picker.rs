//! Theme thumbnails for the Settings window: one per bundled theme, each
//! drawn in that theme's own colours: its background and text, a meter
//! strip for every colour role, and a miniature status bar.

use super::{Gradient, Look, segments};
use crate::config::Config;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui,
    text::{LayoutJob, TextFormat},
    vec2,
};

const THUMB: egui::Vec2 = vec2(232.0, 112.0);
const PAD: f32 = 8.0;
const STRIP_H: f32 = 7.0;
const ROW_H: f32 = 12.0;

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

    /// Draw the thumbnails, wrapping to the available width. Returns the
    /// theme the user clicked.
    pub(super) fn show(&mut self, ui: &mut Ui, config: &Config, current: &str) -> Option<String> {
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
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = vec2(PAD, PAD);
            ui.horizontal_wrapped(|ui| {
                for (name, look) in &self.looks {
                    let selected = name.eq_ignore_ascii_case(current);
                    if thumbnail(ui, name, look, selected).clicked() {
                        picked = Some(name.clone());
                    }
                }
            });
        });
        picked
    }
}

/// The colour roles in the order the thumbnail shows them.
fn roles(look: &Look) -> [(&'static str, &Gradient); 10] {
    [
        ("Load", &look.load),
        ("Sys", &look.load2),
        ("Temp", &look.temp),
        ("Power", &look.power),
        ("Clock", &look.clock),
        ("Fill", &look.fill),
        ("Avail", &look.mem_avail),
        ("Cache", &look.mem_cache),
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

    // Title row, like a panel caption; the user's own themes are tagged.
    painter.text(
        rect.left_top() + vec2(PAD, 5.0),
        Align2::LEFT_TOP,
        name,
        FontId::monospace(11.5),
        look.fg,
    );
    if !crate::theme::is_bundled(name) {
        painter.text(
            rect.right_top() + vec2(-PAD, 6.0),
            Align2::RIGHT_TOP,
            "custom",
            small.clone(),
            look.label,
        );
    }

    // Two columns of role strips.
    let top = rect.top() + 22.0;
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

    // Miniature status bar: `[layout save] [- 2000ms +]` in the theme's bar colours.
    let bar = Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - 18.0),
        rect.right_bottom(),
    );
    painter.rect_filled(
        bar,
        CornerRadius {
            sw: 4,
            se: 4,
            ..Default::default()
        },
        look.statusbar_bg,
    );
    let mut job = LayoutJob::default();
    let parts: [(&str, Color32); 10] = [
        ("[", look.statusbar_sep),
        ("layout ", look.statusbar_fg),
        ("save", look.statusbar_hi),
        ("] ", look.statusbar_sep),
        ("[", look.statusbar_sep),
        ("-", look.statusbar_hi),
        (" 2000ms ", look.statusbar_fg),
        ("+", look.statusbar_hi),
        ("] ", look.statusbar_sep),
        ("warn", look.warn),
    ];
    for (text, color) in parts {
        job.append(text, 0.0, TextFormat::simple(small.clone(), color));
    }
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(bar.left() + PAD, bar.center().y - galley.size().y / 2.0),
        galley,
        look.statusbar_fg,
    );

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
