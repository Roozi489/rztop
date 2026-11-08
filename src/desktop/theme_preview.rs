//! A large preview of a theme for the Settings window: a few of rztop's
//! panels, drawn by the real panels from the live sample, in that theme's
//! colours. The Settings window covers most of the app, so this is where
//! a theme can be judged before and while it is applied.

use super::{
    Look, Ranking, Sample, Sorts, cpu_panel, gpu_panel, mem_panel, mix, mono, ranking_panel, text,
};
use crate::config::Config;
use eframe::egui::{self, Align2, CornerRadius, Rect, Stroke, StrokeKind, Ui, pos2, vec2};

pub(super) const HEIGHT: f32 = 340.0;
const CAPTION_H: f32 = 20.0;
const GAP: f32 = 3.0;

/// Draw the preview across the available width.
pub(super) fn show(ui: &mut Ui, look: &Look, sample: Option<&Sample>, config: &Config) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), look.bg);

    let body = rect;
    let Some(sample) = sample else {
        painter.text(
            body.center(),
            Align2::CENTER_CENTER,
            "Sampling…",
            mono(),
            look.label,
        );
        border(&painter, rect, look);
        return;
    };

    // CPU across the top, then Memory, GPU and the CPU process ranking.
    let top_h = (body.height() * 0.5).round();
    let cpu = Rect::from_min_size(body.min, vec2(body.width(), top_h));
    panel(ui, look, cpu, "CPU", |ui| {
        cpu_panel(ui, look, &sample.cpu);
    });
    let row = Rect::from_min_max(pos2(body.left(), cpu.bottom() + GAP), body.max);
    // The process table gets the most room so its names stay readable.
    let space = row.width() - 2.0 * GAP;
    let widths = [space * 0.29, space * 0.29, space * 0.42];
    let column = |i: usize| {
        let left = row.left() + widths[..i].iter().sum::<f32>() + i as f32 * GAP;
        Rect::from_min_size(pos2(left, row.top()), vec2(widths[i], row.height()))
    };
    panel(ui, look, column(0), "Memory", |ui| {
        mem_panel(ui, look, &sample.mem);
    });
    panel(ui, look, column(1), "GPU", |ui| {
        gpu_panel(ui, look, &sample.gpus, 0);
    });
    let mut sorts = Sorts {
        mem_ram: config.proc.memory_sort_ram,
        io: config.proc.io_sort,
    };
    panel(ui, look, column(2), "CPU", |ui| {
        ranking_panel(ui, look, sample, Ranking::Cpu, &mut sorts, &mut None);
    });
    border(&painter, rect, look);
}

/// One panel: its caption strip and its body, clipped to `rect`.
fn panel(ui: &mut Ui, look: &Look, rect: Rect, caption: &str, body: impl FnOnce(&mut Ui)) {
    let painter = ui.painter_at(rect);
    let strip = Rect::from_min_size(rect.min, vec2(rect.width(), CAPTION_H));
    painter.rect_filled(strip, CornerRadius::ZERO, mix(look.bg, look.fg, 0.05));
    painter.text(
        pos2(strip.left() + 8.0, strip.center().y),
        Align2::LEFT_CENTER,
        caption,
        mono(),
        look.fg,
    );
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0_f32, mix(look.bg, look.dim, 0.6)),
        StrokeKind::Inside,
    );
    let inner = Rect::from_min_max(
        pos2(rect.left() + 8.0, strip.bottom() + 6.0),
        pos2(rect.right() - 8.0, rect.bottom() - 4.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(inner.intersect(ui.clip_rect()));
    // Left enabled: a disabled Ui fades its colours. A click on a column
    // header only changes this preview's own copy of the sort order.
    body(&mut child);
}

fn border(painter: &egui::Painter, rect: Rect, look: &Look) {
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, look.dim),
        StrokeKind::Outside,
    );
}

/// Text explaining what the preview shows.
pub(super) fn hint(look: &Look, name: &str, hovered: bool) -> egui::RichText {
    let message = if hovered {
        format!("Previewing {name}; click it to apply")
    } else {
        format!("{name}. Hover a theme to preview it; click to apply")
    };
    text(message, look.label).size(11.5)
}
