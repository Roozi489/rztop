// Modified for rztop on 2026-11-03: permission status, I/O ranking and pagefile clarity.
use crate::collect::CollectStatus;
use crate::domain::memory::MemInfo;
use crate::draw::box_drawing;
use crate::draw::buffer::AnsiBuffer;
use crate::draw::meter::Meter;
use crate::theme::{Theme, gradient_color};
use crate::theme_keys as tc;
use crate::tools;

use super::WidgetArea;

/// Per-frame view passed to [`draw`].
///
/// Borrows the [`MemConfig`] section directly (no field-by-field
/// copy). Adding a `MemConfig` field is automatically available to
/// the renderer with no `MemFrame` change.
///
/// `base_10` is read from `UiConfig` because byte-unit rendering is
/// a global UI concern, not a memory-widget setting; it's lifted
/// into this struct so the renderer doesn't need a `&UiConfig`
/// borrow just for the one field.
pub struct MemFrame<'a> {
    pub config: &'a crate::config::MemConfig,
    pub base_10: bool,
}

/// Preferred intrinsic height for the memory widget given the
/// snapshot hints, in rows (including borders).
///
/// 4 base rows (Used / Avail / Cached / Commit) + 1 swap row when
/// swap is user-visible + 2 borders.
pub fn preferred_height(hints: &crate::draw::layout::LayoutHints) -> usize {
    let content = 4 + if hints.has_swap { 1 } else { 0 };
    content + 2 // + top/bottom borders
}

/// Draw the memory widget into an ANSI string.
///
/// Layout:
/// ╭─ mem ──────────────────────╮
/// │ Used  ■■■■■■■■■■■░░░ 5.2G │
/// │ Avail ■■■■░░░░░░░░░░ 3.1G │
/// │ Cache ■■■░░░░░░░░░░░ 1.8G │
/// │ Commit ■■░░░░░░░░ 12G/34G │
/// │                            │
/// │ Swap  ■■░░░░░░░░░░░░ 1.0G │
/// │   1.0G / 8.0G              │
/// ╰────────────────────────────╯
pub fn draw(
    mem: &MemInfo,
    area: &WidgetArea,
    theme: &Theme,
    frame: &MemFrame<'_>,
    status: &CollectStatus,
) -> String {
    let x = area.x;
    let y = area.y;
    let width = area.width;
    let height = area.height;
    let rounded = area.rounded;
    let border_color = theme.color(tc::MEM_WIDGET);
    let fg = theme.color(tc::MAIN_FG);
    let title_color = theme.color(tc::TITLE);
    let hi = theme.color(tc::HI_FG);
    let used_grad = theme.gradient(tc::GRAD_USED);
    let cached_grad = theme.gradient(tc::GRAD_CACHED);
    let avail_grad = theme.gradient(tc::GRAD_AVAILABLE);

    let inner_h = height.saturating_sub(2);

    let mut buf = AnsiBuffer::new();
    buf.text(&box_drawing::create_box(&box_drawing::BoxConfig {
        x,
        y,
        width,
        height,
        line_color: border_color,
        fill: true,
        title: "mem",
        title2: "",
        num: super::MEM_KEY,
        rounded,
        hi_color: hi,
        title_color,
    }));

    super::draw_status_inset(&mut buf, status, "mem", x, y, border_color, title_color);

    let total_bytes = mem.stats.used + mem.stats.available;

    // Total memory inset on top border (like CPU frequency)
    let total_str = tools::floating_humanizer(total_bytes, true, 0, false, false, frame.base_10);
    let inset = box_drawing::title_inset(&total_str, border_color, title_color, false);
    let inset_x = box_drawing::right_inset_x(x, width, box_drawing::inset_width(&total_str));
    buf.mv(inset_x, y + 1).text(&inset);

    // Content area: width - 4 (border + space on each side)
    // Content starts at x + 3 (border at x+1, space at x+2)
    let val_w = 5; // right-aligned value column
    let label_w = 6; // "Used  ", "Avail ", etc.
    let inner_w = width.saturating_sub(4);
    let meter_w = inner_w.saturating_sub(label_w + val_w).max(5);
    let content_x = x + 3;
    let meter_bg = theme.color(tc::METER_BG);
    let used_meter = Meter::new(meter_w, used_grad, meter_bg);
    let avail_meter = Meter::new(meter_w, avail_grad, meter_bg);
    let cached_meter = Meter::new(meter_w, cached_grad, meter_bg);
    let mut row = 0;

    let render_row = |buf: &mut AnsiBuffer,
                      label: &str,
                      meter_str: &str,
                      value: &str,
                      label_color: &str,
                      val_color: &str,
                      rx: usize,
                      ry: usize| {
        buf.mv(rx, ry)
            .color(label_color)
            .text(label)
            .text(meter_str)
            .color(val_color)
            .text(&tools::rjust(value, val_w, false));
    };

    let used = mem.stats.used;
    let used_pct = (used * 100).checked_div(total_bytes).unwrap_or(0) as i32;
    let used_color = gradient_color(used_grad, used_pct);
    let used_str = tools::floating_humanizer(used, true, 0, false, false, frame.base_10);
    if row < inner_h {
        render_row(
            &mut buf,
            "Used  ",
            used_meter.render(used_pct),
            &used_str,
            fg,
            used_color,
            content_x,
            y + 2 + row,
        );
        row += 1;
    }

    let avail = mem.stats.available;
    let avail_pct = (avail * 100).checked_div(total_bytes).unwrap_or(0) as i32;
    let avail_color = gradient_color(avail_grad, avail_pct);
    let avail_str = tools::floating_humanizer(avail, true, 0, false, false, frame.base_10);
    if row < inner_h {
        render_row(
            &mut buf,
            "Avail ",
            avail_meter.render(avail_pct),
            &avail_str,
            fg,
            avail_color,
            content_x,
            y + 2 + row,
        );
        row += 1;
    }

    let cached = mem.stats.cached;
    if cached > 0 && row < inner_h {
        let cached_pct = (cached * 100).checked_div(total_bytes).unwrap_or(0) as i32;
        let cache_color = gradient_color(cached_grad, cached_pct);
        let cached_str = tools::floating_humanizer(cached, true, 0, false, false, frame.base_10);
        render_row(
            &mut buf,
            "Cache ",
            cached_meter.render(cached_pct),
            &cached_str,
            fg,
            cache_color,
            content_x,
            y + 2 + row,
        );
        row += 1;
    }

    if row < inner_h {
        let limit = mem.stats.commit_limit;
        let pct = mem
            .stats
            .commit_used
            .saturating_mul(100)
            .checked_div(limit)
            .unwrap_or(0)
            .min(100) as i32;
        let value = if limit > 0 {
            format!(
                " {}/{}",
                tools::floating_humanizer(
                    mem.stats.commit_used,
                    true,
                    0,
                    false,
                    false,
                    frame.base_10
                ),
                tools::floating_humanizer(limit, true, 0, false, false, frame.base_10)
            )
        } else {
            " N/A".into()
        };
        let gradient = theme.gradient(tc::GRAD_CPU_UPPER);
        let meter = Meter::new(
            inner_w.saturating_sub(label_w + tools::ulen(&value)),
            gradient,
            meter_bg,
        );
        buf.mv(content_x, y + 2 + row)
            .color(fg)
            .text("Commit")
            .text(meter.render(pct))
            .color(if limit > 0 {
                gradient_color(gradient, pct)
            } else {
                theme.color(tc::INACTIVE_FG)
            })
            .text(&value);
        row += 1;
    }

    // Swap (no blank line, no separate total line — meter shows ratio)
    if frame.config.show_swap {
        // A growth ceiling compares allocated capacity to potential capacity.
        // Without a ceiling, retain the standard used/allocated view.
        let swap_value = if frame.config.swap_max_mib > 0 {
            mem.stats.swap_total
        } else {
            mem.stats.swap_used
        };
        let swap_total = mem
            .stats
            .swap_total
            .max((frame.config.swap_max_mib.max(0) as u64).saturating_mul(1024 * 1024));
        let swap_pct = if swap_total > 0 {
            (swap_value.saturating_mul(100) / swap_total.max(1)).min(100) as i32
        } else {
            0
        };
        let swap_color = gradient_color(used_grad, swap_pct);
        let swap_str = tools::floating_humanizer(swap_value, true, 0, false, false, frame.base_10);
        let max_str = tools::floating_humanizer(swap_total, true, 0, false, false, frame.base_10);
        let value = format!(" {swap_str}/{max_str}");
        let swap_label = if frame.config.swap_max_mib > 0 {
            "Swap size "
        } else {
            "Swap used "
        };
        let swap_meter = Meter::new(
            inner_w.saturating_sub(tools::ulen(swap_label) + tools::ulen(&value)),
            used_grad,
            meter_bg,
        );
        if row < inner_h {
            buf.mv(content_x, y + 2 + row)
                .color(fg)
                .text(swap_label)
                .text(swap_meter.render(swap_pct))
                .color(swap_color)
                .text(&value);
        }
    }

    buf.finish()
}

/// Memory widget renderer. Unit struct — the widget has no per-
/// instance state.
pub struct MemWidget;

impl super::Widget for MemWidget {
    fn kinds(&self) -> &'static [crate::domain::widget_kind::WidgetKind] {
        const KINDS: &[crate::domain::widget_kind::WidgetKind] =
            &[crate::domain::widget_kind::WidgetKind::Mem];
        KINDS
    }

    fn preferred_height(&self, hints: &crate::draw::layout::LayoutHints) -> usize {
        preferred_height(hints)
    }

    fn min_width(&self, _: &crate::draw::layout::LayoutHints) -> usize {
        crate::draw::layout::MIN_MEM_WIDTH
    }

    fn min_height(&self, hints: &crate::draw::layout::LayoutHints) -> usize {
        preferred_height(hints)
    }

    fn render(&self, params: &crate::app::RenderParams<'_>, output: &mut String) {
        let Some(mem_dim) = params
            .layout
            .dims_for(crate::domain::widget_kind::WidgetKind::Mem)
        else {
            return;
        };
        let Some(mem) = params.mem else {
            return;
        };
        let area = super::WidgetArea::from_dim(mem_dim, params.rounded);
        let frame = MemFrame {
            config: &params.config.mem,
            base_10: params.config.ui.base_10_sizes,
        };
        output.push_str(&draw(&mem.info, &area, params.theme, &frame, &mem.status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::memory::{MemPercent, MemStats};

    fn strip_ansi(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let mut in_escape = false;
        for ch in s.chars() {
            if in_escape {
                if ch.is_ascii_alphabetic() || ch == 'm' {
                    in_escape = false;
                }
                continue;
            }
            if ch == '\x1b' {
                in_escape = true;
                continue;
            }
            result.push(ch);
        }
        result
    }

    const GIB: u64 = 1024 * 1024 * 1024;

    fn make_mem_info() -> MemInfo {
        MemInfo {
            stats: MemStats {
                used: 8 * GIB,
                available: 8 * GIB,
                cached: 2 * GIB,
                commit_used: 12 * GIB,
                commit_limit: 24 * GIB,
                swap_total: 4 * GIB,
                swap_used: GIB,
                swap_free: 3 * GIB,
            },
            percent: MemPercent::default(),
        }
    }

    fn make_area() -> WidgetArea {
        WidgetArea {
            x: 1,
            y: 1,
            width: 40,
            height: 12,
            rounded: true,
        }
    }

    #[test]
    fn commit_shows_current_limit_and_correct_gradient() {
        let theme = Theme::default();
        let mut info = make_mem_info();
        let config = crate::config::MemConfig::default();
        let frame = MemFrame {
            config: &config,
            base_10: false,
        };
        let output = draw(&info, &make_area(), &theme, &frame, &CollectStatus::Ok);
        let plain = strip_ansi(&output);
        assert!(plain.contains("Commit"));
        assert!(!plain.contains("Free"));
        assert!(output.contains(&format!(
            "{} 12G/24G",
            theme.gradient(tc::GRAD_CPU_UPPER)[50]
        )));
        info.stats.commit_limit = 0;
        let output = draw(&info, &make_area(), &theme, &frame, &CollectStatus::Ok);
        assert!(strip_ansi(&output).contains("N/A"));
    }

    #[test]
    fn draw_contains_mem_title() {
        let output = draw(
            &make_mem_info(),
            &make_area(),
            &Theme::default(),
            &MemFrame {
                config: &crate::config::MemConfig {
                    show_swap: true,
                    ..Default::default()
                },
                base_10: false,
            },
            &CollectStatus::Ok,
        );
        let plain = strip_ansi(&output);
        assert!(plain.contains("mem"), "output should contain 'mem' title");
    }

    #[test]
    fn draw_contains_used_label() {
        let output = draw(
            &make_mem_info(),
            &make_area(),
            &Theme::default(),
            &MemFrame {
                config: &crate::config::MemConfig {
                    show_swap: true,
                    ..Default::default()
                },
                base_10: false,
            },
            &CollectStatus::Ok,
        );
        let plain = strip_ansi(&output);
        assert!(plain.contains("Used"), "output should contain 'Used' label");
    }

    #[test]
    fn draw_shows_swap_when_enabled() {
        let output = draw(
            &make_mem_info(),
            &make_area(),
            &Theme::default(),
            &MemFrame {
                config: &crate::config::MemConfig {
                    show_swap: true,
                    ..Default::default()
                },
                base_10: false,
            },
            &CollectStatus::Ok,
        );
        let plain = strip_ansi(&output);
        assert!(
            plain.contains("Swap"),
            "output should contain 'Swap' when show_swap=true"
        );
    }

    #[test]
    fn draw_hides_swap_when_disabled() {
        let output = draw(
            &make_mem_info(),
            &make_area(),
            &Theme::default(),
            &MemFrame {
                config: &crate::config::MemConfig {
                    show_swap: false,
                    ..Default::default()
                },
                base_10: false,
            },
            &CollectStatus::Ok,
        );
        let plain = strip_ansi(&output);
        assert!(
            !plain.contains("Swap"),
            "output should not contain 'Swap' when show_swap=false"
        );
    }

    #[test]
    fn swap_value_is_colored_by_used_gradient() {
        // Defends Option A: every value with a meter takes the meter's
        // gradient at the value's pct. Swap meter uses GRAD_USED, so swap
        // value must too. Pre-fix the swap value used MAIN_FG while the four
        // rows above used their gradients — the only inconsistency in mem.
        let theme = Theme::default();
        // 1 GiB used / 4 GiB total = 25 %.
        let info = make_mem_info();
        let output = draw(
            &info,
            &make_area(),
            &theme,
            &MemFrame {
                config: &crate::config::MemConfig {
                    show_swap: true,
                    ..Default::default()
                },
                base_10: false,
            },
            &CollectStatus::Ok,
        );
        let used_grad = theme.gradient(tc::GRAD_USED);
        let expected = format!("{}{}", used_grad[25], " 1.0G");
        assert!(
            output.contains(&expected),
            "swap value cell should be GRAD_USED[25] immediately followed by ' 1.0G'; got:\n{output}"
        );
    }

    #[test]
    fn swap_uses_configured_scale_and_never_understates_allocation() {
        let theme = Theme::default();
        for (scale, expected, pct) in [(8192, "4.0G/8.0G", 50), (1024, "4.0G/4.0G", 100)] {
            let output = draw(
                &make_mem_info(),
                &make_area(),
                &theme,
                &MemFrame {
                    config: &crate::config::MemConfig {
                        show_swap: true,
                        swap_max_mib: scale,
                    },
                    base_10: false,
                },
                &CollectStatus::Ok,
            );
            assert!(output.contains(&format!(
                "{} {expected}",
                theme.gradient(tc::GRAD_USED)[pct]
            )));
            assert!(output.contains("Swap size "));
        }
    }

    #[test]
    fn swap_used_is_distinct_from_allocated_pagefile_size() {
        let mut mem = make_mem_info();
        mem.stats.swap_used = 78 * 1024 * 1024;
        mem.stats.swap_total = 5 * 1024 * 1024 * 1024;
        let output = draw(
            &mem,
            &make_area(),
            &Theme::default(),
            &MemFrame {
                config: &crate::config::MemConfig {
                    show_swap: true,
                    swap_max_mib: 0,
                },
                base_10: false,
            },
            &CollectStatus::Ok,
        );
        assert!(output.contains("Swap used "));
        assert!(output.contains("78M/5.0G"));
    }

    #[test]
    fn meter_row_labels_use_main_fg() {
        // Body label rule: meter row labels (Used, Avail, Cache, Free, Swap)
        // render in MAIN_FG so they distinguish from structural text (TITLE
        // border insets and section dividers). Pre-shift these were TITLE.
        let theme = Theme::default();
        let output = draw(
            &make_mem_info(),
            &make_area(),
            &theme,
            &MemFrame {
                config: &crate::config::MemConfig {
                    show_swap: true,
                    ..Default::default()
                },
                base_10: false,
            },
            &CollectStatus::Ok,
        );
        let fg = theme.color(tc::MAIN_FG);
        for label in &["Used  ", "Avail ", "Cache ", "Commit", "Swap used "] {
            assert!(
                output.contains(&format!("{fg}{label}")),
                "mem label {label:?} should be preceded by MAIN_FG"
            );
        }
    }
}
