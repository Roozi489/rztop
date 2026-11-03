//! Zero-width control markers interpreted only at the terminal boundary.
// Modified for rztop on 2026-11-02.
use crate::domain::resizing::{Axis, Divider};
use std::collections::BTreeMap;

pub const MODAL: u32 = 1_000_000;
pub const OPTION_TAB: u32 = 2_000;
pub const OPTION_NAME: u32 = 3_000;
pub const OPTION_VALUE: u32 = 4_000;
pub const KEY_LEFT: u32 = 500;
pub const KEY_RIGHT: u32 = 501;
pub const KEY_ENTER: u32 = 502;
pub const KEY_ESCAPE: u32 = 503;
pub const KEY_PAGEUP: u32 = 504;
pub const KEY_PAGEDOWN: u32 = 505;

pub fn button(text: &str, id: u32) -> String {
    format!("\x1b[777;{id}z{text}\x1b[777;0z")
}

#[derive(Clone, Copy, Default)]
struct Cell {
    ch: char,
    control: u32,
    lead: usize,
    width: usize,
}

#[derive(Default)]
pub struct Screen {
    cells: BTreeMap<(usize, usize), Cell>,
    x: usize,
    y: usize,
    pub pointer: Option<(usize, usize)>,
    pub modal: bool,
    pub blocked: bool,
    pub valid: bool,
    pub dividers: Vec<Divider>,
}

impl Screen {
    pub fn invalidate(&mut self) {
        self.valid = false;
    }
    pub fn control_at(&self, x: usize, y: usize) -> Option<u32> {
        if !self.valid {
            return None;
        }
        let id = self.cells.get(&(x, y))?.control;
        if self.blocked
            && !(self.modal && matches!(id, v if v == MODAL + KEY_ENTER || v == MODAL + KEY_ESCAPE))
        {
            return None;
        }
        (id != 0 && (id >= MODAL) == self.modal).then_some(id)
    }
    pub fn divider_at(&self, x: usize, y: usize) -> Option<&Divider> {
        if !self.valid || self.modal || self.blocked || self.control_at(x, y).is_some() {
            return None;
        }
        self.dividers.iter().rev().find(|d| d.hit(x, y))
    }
    /// Consume partial output into the final cell map; overpainting always clears old controls.
    /// Markers never leave this function, including auxiliary terminal render paths.
    pub fn output(&mut self, input: &str) -> String {
        let cursor = (self.x, self.y);
        // Resolve hit ownership after every overpaint before deciding hover styling.
        self.parse(input, None);
        let hovered = self.pointer.and_then(|(x, y)| {
            let id = self.control_at(x, y)?;
            let mut left = x;
            let mut right = x;
            while left > 1 && self.control_at(left - 1, y) == Some(id) {
                left -= 1;
            }
            while self.control_at(right + 1, y) == Some(id) {
                right += 1;
            }
            Some((left, right, y, id))
        });
        (self.x, self.y) = cursor;
        let mut out = self.parse(input, hovered);
        out.push_str(&self.resize_feedback());
        out
    }

    fn parse(&mut self, input: &str, hovered: Option<(usize, usize, usize, u32)>) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        let mut control = 0;
        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                let mut sequence = String::new();
                for next in chars.by_ref() {
                    sequence.push(next);
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
                let command = sequence.pop().unwrap_or('m');
                if command == 'z' && sequence.starts_with("777;") {
                    control = sequence[4..].parse().unwrap_or(0);
                    continue;
                }
                let values: Vec<usize> = sequence
                    .split(';')
                    .map(|n| n.parse().unwrap_or(0))
                    .collect();
                let first = values.first().copied().unwrap_or(0);
                match command {
                    'H' | 'f' => {
                        self.y = first.max(1);
                        self.x = values.get(1).copied().unwrap_or(1).max(1);
                        control = 0;
                    }
                    'C' => self.x += first.max(1),
                    'D' => self.x = self.x.saturating_sub(first.max(1)).max(1),
                    'A' => self.y = self.y.saturating_sub(first.max(1)).max(1),
                    'B' => self.y += first.max(1),
                    'J' if first == 2 => self.cells.clear(),
                    _ => {}
                }
                out.push_str(&format!("\x1b[{sequence}{command}"));
                continue;
            }
            if ch == '\n' {
                self.y += 1;
                self.x = 1;
                control = 0;
                out.push(ch);
                continue;
            }
            if ch == '\r' {
                self.x = 1;
                out.push(ch);
                continue;
            }
            let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            let underline = hovered.is_some_and(|(left, right, row, id)| {
                control == id && self.y == row && self.x >= left && self.x <= right
            });
            if underline {
                out.push_str(crate::term::UNDERLINE);
            }
            out.push(ch);
            if underline {
                out.push_str(crate::term::UNDERLINE_OFF);
            }
            for offset in 0..width {
                if let Some(old) = self
                    .cells
                    .get(&(self.x + offset, self.y))
                    .copied()
                    .filter(|c| c.width > 1)
                {
                    for column in old.lead..old.lead + old.width {
                        self.cells.remove(&(column, self.y));
                    }
                }
            }
            for offset in 0..width {
                self.cells.insert(
                    (self.x + offset, self.y),
                    Cell {
                        ch,
                        control,
                        lead: self.x,
                        width,
                    },
                );
            }
            self.x += width;
        }
        out
    }

    fn resize_feedback(&self) -> String {
        let Some((x, y)) = self.pointer else {
            return String::new();
        };
        let Some(divider) = self.divider_at(x, y) else {
            return String::new();
        };
        let point = match divider.axis {
            Axis::Horizontal => x,
            Axis::Vertical => y,
        };
        let clear = |along, offset| {
            let key = match divider.axis {
                Axis::Horizontal => (along, divider.position + offset),
                Axis::Vertical => (divider.position + offset, along),
            };
            self.cells.get(&key).is_some_and(|c| {
                c.control == 0
                    && match divider.axis {
                        Axis::Horizontal => c.ch == '─' || c.ch == '━',
                        Axis::Vertical => c.ch == '│' || c.ch == '┃',
                    }
            })
        };
        let can_move = [
            divider.sizes[divider.before] > divider.minimums[divider.before],
            divider.sizes[divider.after] > divider.minimums[divider.after],
        ];
        let arrow = (divider.start..=divider.end)
            .filter(|&along| (0..2).all(|offset| !can_move[offset] || clear(along, offset)))
            .min_by_key(|p| p.abs_diff(point));
        let mut out = String::new();
        for along in divider.start..=divider.end {
            for (offset, movable) in can_move.iter().enumerate() {
                if !clear(along, offset) {
                    continue;
                }
                let (x, y, stroke) = match divider.axis {
                    Axis::Horizontal => (
                        along,
                        divider.position + offset,
                        if Some(along) == arrow && *movable {
                            if offset == 0 { "^" } else { "v" }
                        } else {
                            "="
                        },
                    ),
                    Axis::Vertical => (
                        divider.position + offset,
                        along,
                        if Some(along) == arrow && *movable {
                            if offset == 0 { "<" } else { ">" }
                        } else {
                            "║"
                        },
                    ),
                };
                out.push_str(&crate::term::mv(x, y));
                out.push_str(stroke);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pointer_aligned_arrows_avoid_labels_and_move_without_trails() {
        let mut screen = Screen {
            valid: true,
            pointer: Some((4, 2)),
            ..Default::default()
        };
        screen.dividers = vec![Divider {
            target: crate::domain::resizing::SplitTarget::Rankings,
            axis: Axis::Horizontal,
            position: 2,
            start: 1,
            end: 8,
            before: 0,
            after: 1,
            sizes: vec![10, 10],
            minimums: vec![5, 5],
        }];
        let raw = "\x1b[2;1H──XX────\x1b[3;1H────────";
        let output = screen.output(raw);
        assert!(output.contains("\x1b[2;5H^"));
        assert!(output.contains("\x1b[3;5Hv"));
        assert!(!output.contains("\x1b[2;4H^"));
        // A caption on one outline must not punch a hole in the other outline.
        assert!(output.contains("\x1b[3;3H="));
        assert!(output.contains("\x1b[3;4H="));
        assert!(!output.contains("\x1b[2;3H="));
        screen.pointer = Some((7, 2));
        let moved = screen.output(raw);
        assert!(moved.contains("\x1b[2;7H^"));
        assert!(moved.contains("\x1b[2;5H="));
        screen.pointer = None;
        assert_eq!(screen.output(raw), raw);
    }

    #[test]
    fn arrows_only_show_available_directions_on_both_axes() {
        for axis in [Axis::Horizontal, Axis::Vertical] {
            for (sizes, expected) in [
                ([5, 10], [false, true]),
                ([10, 5], [true, false]),
                ([5, 5], [false, false]),
            ] {
                let mut screen = Screen {
                    valid: true,
                    pointer: Some((2, 2)),
                    dividers: vec![Divider {
                        target: crate::domain::resizing::SplitTarget::Rankings,
                        axis,
                        position: 2,
                        start: 1,
                        end: 4,
                        before: 0,
                        after: 1,
                        sizes: sizes.to_vec(),
                        minimums: vec![5, 5],
                    }],
                    ..Default::default()
                };
                let raw = match axis {
                    Axis::Horizontal => "\x1b[2;1H────\x1b[3;1H────",
                    Axis::Vertical => "\x1b[1;2H││\x1b[2;2H││\x1b[3;2H││\x1b[4;2H││",
                };
                let output = screen.output(raw);
                let arrows = match axis {
                    Axis::Horizontal => ['^', 'v'],
                    Axis::Vertical => ['<', '>'],
                };
                for (arrow, visible) in arrows.into_iter().zip(expected) {
                    assert_eq!(output.contains(arrow), visible);
                }
            }
        }
    }
    #[test]
    fn hover_uses_final_overpaint_and_only_the_pointed_button() {
        let mut screen = Screen {
            valid: true,
            pointer: Some((1, 1)),
            ..Default::default()
        };
        let raw = format!(
            "\x1b[1;1H{}\x1b[2;1H{}",
            button("filter", 102),
            button("filter", 102)
        );
        let out = screen.output(&raw);
        assert_eq!(out.matches(crate::term::UNDERLINE).count(), 6);
        let overpaint = screen.output(&format!("{raw}\x1b[1;1H      "));
        assert!(!overpaint.contains(crate::term::UNDERLINE));
    }
    #[test]
    fn metadata_removed_and_unicode_overpainting_updates_hit_map() {
        let mut screen = Screen {
            valid: true,
            ..Default::default()
        };
        let output = screen.output(&format!("\x1b[2;3H{}", button("界x", 42)));
        assert!(!output.contains("777;"));
        assert_eq!(screen.control_at(3, 2), Some(42));
        assert_eq!(screen.control_at(4, 2), Some(42));
        screen.output("\x1b[2;3H ");
        assert_eq!(screen.control_at(3, 2), None);
        assert_eq!(screen.control_at(4, 2), None);
        screen.modal = true;
        assert_eq!(screen.control_at(4, 2), None);
    }
}
