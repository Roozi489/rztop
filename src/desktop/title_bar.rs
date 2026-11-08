//! The window's own title bar, drawn in the theme's colours in place of the
//! system one: app icon and name, rztop's own CPU usage and
//! the uptime, then icon buttons for elevation, layout and
//! settings, and the minimize / maximize / close buttons. Also lets the
//! window be resized from its edges, which a window without system
//! decorations no longer gets from Windows.

use super::{Look, mix, text};
use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, Pos2, Rect, Sense, Shape, Stroke, TextureHandle, Ui,
    ViewportCommand, pos2, vec2,
};

pub(super) const HEIGHT: f32 = 30.0;
/// Width of a minimize / maximize / close button, like Windows' own.
const CONTROL_W: f32 = 46.0;
/// Width of an icon button.
const ICON_W: f32 = 30.0;
/// How far in from the window edge a press starts a resize.
const EDGE: f32 = 5.0;

/// What the user asked for in the title bar this frame.
#[derive(Default)]
pub(super) struct Actions {
    pub(super) settings: bool,
    pub(super) layout_save: bool,
    pub(super) layout_load: bool,
    pub(super) layout_reset: bool,
    pub(super) elevate: bool,
}

/// What the title bar shows besides its buttons.
pub(super) struct State<'a> {
    pub(super) icon: &'a TextureHandle,
    /// rztop's own CPU usage, already formatted.
    pub(super) own_cpu: Option<String>,
    pub(super) uptime: Option<String>,
    /// Not elevated; the shield icon offers a relaunch.
    pub(super) not_admin: bool,
    /// Some data is missing for lack of elevation; the shield warns.
    pub(super) admin_limited: bool,
    pub(super) has_saved_layout: bool,
}

pub(super) fn show(ui: &mut Ui, look: &Look, state: &State) -> Actions {
    let mut actions = Actions::default();
    let bar = ui.max_rect();
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));

    // The background drags the window; widgets added later sit on top of it.
    let drag = ui.interact(bar, ui.id().with("title-drag"), Sense::click_and_drag());
    if drag.double_clicked() {
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    } else if drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }

    ui.spacing_mut().item_spacing.x = 0.0;
    ui.horizontal_centered(|ui| {
        ui.add_space(8.0);
        ui.add(egui::Image::new(state.icon).fit_to_exact_size(vec2(16.0, 16.0)));
        ui.add_space(8.0);
        ui.add(egui::Label::new(text("rztop", look.statusbar_fg)).selectable(false));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Right to left: the first item drawn is the rightmost.
            if control(ui, look, Control::Close).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
            let max = if maximized {
                Control::Restore
            } else {
                Control::Maximize
            };
            if control(ui, look, max).clicked() {
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            }
            if control(ui, look, Control::Minimize).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
            }
            ui.add_space(8.0);

            actions.settings = icon_button(ui, look, Icon::Settings, look.statusbar_fg)
                .on_hover_text("Settings")
                .clicked();
            layout_menu(ui, look, state.has_saved_layout, &mut actions);
            if state.not_admin {
                let (color, tip) = if state.admin_limited {
                    (
                        look.warn,
                        "Not running as administrator, so some data is missing \
                         (CPU temperature and power, some processes).\n\
                         Click to restart rztop as administrator.",
                    )
                } else {
                    (
                        look.statusbar_fg,
                        "Not running as administrator.\nClick to restart rztop as administrator.",
                    )
                };
                actions.elevate = icon_button(ui, look, Icon::Shield, color)
                    .on_hover_text(tip)
                    .clicked();
            }

            // Readouts, dropped when the window is too narrow for them.
            for (label, value) in [
                ("up ", state.uptime.as_deref()),
                ("rztop CPU ", state.own_cpu.as_deref()),
            ] {
                let Some(value) = value else { continue };
                let mut job = egui::text::LayoutJob::default();
                for (part, color) in [(label, look.statusbar_sep), (value, look.statusbar_fg)] {
                    job.append(part, 0.0, egui::TextFormat::simple(super::mono(), color));
                }
                let galley = ui.fonts(|f| f.layout_job(job));
                if ui.available_width() < galley.size().x + 24.0 {
                    break;
                }
                ui.add_space(14.0);
                ui.add(egui::Label::new(galley).selectable(false));
            }
        });
    });
    actions
}

/// The layout icon and its menu: save, load and reset.
fn layout_menu(ui: &mut Ui, look: &Look, has_saved: bool, actions: &mut Actions) {
    let mut response = icon_button(ui, look, Icon::Layout, look.statusbar_fg);
    let popup = ui.id().with("layout-menu");
    if response.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup));
    }
    if !ui.memory(|m| m.is_popup_open(popup)) {
        response = response.on_hover_text("Layout");
    }
    egui::popup::popup_above_or_below_widget(
        ui,
        popup,
        &response,
        egui::AboveOrBelow::Below,
        egui::PopupCloseBehavior::CloseOnClick,
        |ui| {
            ui.set_min_width(170.0);
            // Plain rows like a menu, highlighted on hover.
            ui.visuals_mut().button_frame = false;
            ui.style_mut().spacing.button_padding = vec2(8.0, 4.0);
            if ui
                .button("Save layout")
                .on_hover_text("Keep the panels and the window position and size")
                .clicked()
            {
                actions.layout_save = true;
            }
            if ui
                .add_enabled(has_saved, egui::Button::new("Load saved layout"))
                .on_disabled_hover_text("Nothing saved yet")
                .clicked()
            {
                actions.layout_load = true;
            }
            if ui
                .button("Reset layout")
                .on_hover_text("The default arrangement, sized to fit the window")
                .clicked()
            {
                actions.layout_reset = true;
            }
        },
    );
}

#[derive(Clone, Copy)]
enum Icon {
    Settings,
    Layout,
    Shield,
}

fn icon_button(ui: &mut Ui, look: &Look, icon: Icon, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ICON_W, HEIGHT), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let painter = ui.painter_at(rect);
    let color = if response.hovered() {
        painter.rect_filled(
            rect.shrink2(vec2(2.0, 3.0)),
            CornerRadius::same(4),
            mix(look.statusbar_bg, look.statusbar_fg, 0.12),
        );
        look.statusbar_hi
    } else {
        color
    };
    let c = rect.center();
    let stroke = Stroke::new(1.4_f32, color);
    match icon {
        Icon::Settings => gear(&painter, c, stroke),
        Icon::Layout => {
            // A window split into panels, like the dock.
            let r = Rect::from_center_size(c, vec2(15.0, 13.0));
            painter.rect_stroke(r, CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            let split = r.left() + r.width() * 0.42;
            painter.line_segment([pos2(split, r.top()), pos2(split, r.bottom())], stroke);
            painter.line_segment(
                [pos2(split, r.center().y), pos2(r.right(), r.center().y)],
                stroke,
            );
        }
        Icon::Shield => {
            let p = |x: f32, y: f32| c + vec2(x, y);
            painter.add(Shape::closed_line(
                vec![
                    p(0.0, -7.5),
                    p(6.0, -5.0),
                    p(6.0, 0.0),
                    p(4.5, 4.0),
                    p(0.0, 7.5),
                    p(-4.5, 4.0),
                    p(-6.0, 0.0),
                    p(-6.0, -5.0),
                ],
                stroke,
            ));
            painter.line_segment([p(0.0, -3.5), p(0.0, 1.0)], Stroke::new(1.6_f32, color));
            painter.circle_filled(p(0.0, 3.6), 0.95, color);
        }
    }
    response
}

/// A gear: a ring with eight teeth around a hub.
fn gear(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    painter.circle_stroke(c, 4.8, stroke);
    painter.circle_stroke(c, 1.8, stroke);
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let dir = vec2(a.cos(), a.sin());
        painter.line_segment(
            [c + dir * 4.8, c + dir * 7.2],
            Stroke::new(2.4_f32, stroke.color),
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// A Windows-style caption button: flat, full height, red when hovering close.
fn control(ui: &mut Ui, look: &Look, control: Control) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(CONTROL_W, HEIGHT), Sense::click());
    let painter = ui.painter_at(rect);
    let mut color = look.statusbar_fg;
    if response.hovered() {
        let bg = if control == Control::Close {
            color = Color32::WHITE;
            Color32::from_rgb(0xc4, 0x2b, 0x1c)
        } else {
            mix(look.statusbar_bg, look.statusbar_fg, 0.12)
        };
        painter.rect_filled(rect, CornerRadius::ZERO, bg);
    }
    let c = rect.center();
    let stroke = Stroke::new(1.0_f32, color);
    let h = 5.0;
    match control {
        Control::Minimize => {
            painter.line_segment([c + vec2(-h, 0.0), c + vec2(h, 0.0)], stroke);
        }
        Control::Maximize => {
            painter.rect_stroke(
                Rect::from_center_size(c, vec2(2.0 * h, 2.0 * h)),
                CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        Control::Restore => {
            // Two overlapping windows.
            let front = Rect::from_center_size(c + vec2(-1.0, 1.0), vec2(8.0, 8.0));
            painter.rect_stroke(
                front,
                CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Inside,
            );
            let back = front.translate(vec2(2.0, -2.0));
            painter.line_segment([pos2(back.left(), back.top()), back.right_top()], stroke);
            painter.line_segment(
                [back.right_top(), pos2(back.right(), back.bottom())],
                stroke,
            );
        }
        Control::Close => {
            painter.line_segment([c + vec2(-h, -h), c + vec2(h, h)], stroke);
            painter.line_segment([c + vec2(-h, h), c + vec2(h, -h)], stroke);
        }
    }
    let tip = match control {
        Control::Minimize => "Minimize",
        Control::Maximize => "Maximize",
        Control::Restore => "Restore down",
        Control::Close => "Close",
    };
    response.on_hover_text(tip)
}

/// Resize the window by dragging its edges and corners, and draw a thin
/// border so the window stands apart from what is behind it. Call after
/// everything else so the resize cursor wins.
pub(super) fn edges(ctx: &egui::Context, look: &Look) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let screen = ctx.screen_rect();
    if !maximized {
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("window-border"),
        ))
        .rect_stroke(
            screen,
            CornerRadius::ZERO,
            Stroke::new(1.0_f32, look.dim),
            egui::StrokeKind::Inside,
        );
    }
    if maximized {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let Some(direction) = edge_at(screen, pos) else {
        return;
    };
    // Leave a drag that started elsewhere (a separator, a tab) alone.
    if ctx.input(|i| i.pointer.any_down() && !i.pointer.any_pressed()) {
        return;
    }
    use egui::ResizeDirection as D;
    ctx.set_cursor_icon(match direction {
        D::North | D::South => CursorIcon::ResizeVertical,
        D::East | D::West => CursorIcon::ResizeHorizontal,
        D::NorthWest | D::SouthEast => CursorIcon::ResizeNwSe,
        D::NorthEast | D::SouthWest => CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

/// Which edge or corner of `screen` the point is on, if any. Corners are
/// a little larger than the edges so they are easy to hit.
fn edge_at(screen: Rect, pos: Pos2) -> Option<egui::ResizeDirection> {
    use egui::ResizeDirection as D;
    if !screen.contains(pos) {
        return None;
    }
    let corner = EDGE * 3.0;
    let left = pos.x - screen.left() < EDGE;
    let right = screen.right() - pos.x < EDGE;
    let top = pos.y - screen.top() < EDGE;
    let bottom = screen.bottom() - pos.y < EDGE;
    let near_left = pos.x - screen.left() < corner;
    let near_right = screen.right() - pos.x < corner;
    let near_top = pos.y - screen.top() < corner;
    let near_bottom = screen.bottom() - pos.y < corner;
    Some(match () {
        _ if (top && near_left) || (left && near_top) => D::NorthWest,
        _ if (top && near_right) || (right && near_top) => D::NorthEast,
        _ if (bottom && near_left) || (left && near_bottom) => D::SouthWest,
        _ if (bottom && near_right) || (right && near_bottom) => D::SouthEast,
        _ if top => D::North,
        _ if bottom => D::South,
        _ if left => D::West,
        _ if right => D::East,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::ResizeDirection as D;

    #[test]
    fn edges_and_corners_map_to_resize_directions() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        assert!(matches!(edge_at(screen, pos2(1.0, 300.0)), Some(D::West)));
        assert!(matches!(edge_at(screen, pos2(799.0, 300.0)), Some(D::East)));
        assert!(matches!(edge_at(screen, pos2(400.0, 1.0)), Some(D::North)));
        assert!(matches!(
            edge_at(screen, pos2(400.0, 599.0)),
            Some(D::South)
        ));
        assert!(matches!(
            edge_at(screen, pos2(2.0, 10.0)),
            Some(D::NorthWest)
        ));
        assert!(matches!(
            edge_at(screen, pos2(795.0, 598.0)),
            Some(D::SouthEast)
        ));
        assert!(edge_at(screen, pos2(400.0, 300.0)).is_none());
        assert!(edge_at(screen, pos2(-1.0, 300.0)).is_none());
    }
}
