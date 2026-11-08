//! Process details: a window per process opened from any process table,
//! with what is known about the process and graphs of its CPU, memory,
//! I/O and GPU use. The sampler records a short history for every
//! process, so a window shows the recent past as soon as it opens.

use super::{Gradient, Grow, Look, Sample, caption, dot_graph_scaled, graph_columns, mix, text};
use crate::domain::process::{ProcInfo, display_cpu_percent};
use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use std::collections::{HashMap, HashSet, VecDeque};

/// Minutes of history kept for a process with an open window when
/// background recording is off.
pub(super) const OPEN_WINDOW_MIN: i64 = 15;
const GRAPH_H: f32 = 64.0;

/// One process's recent readings, oldest first. Every process gets one,
/// so readings are stored as `f32` (exact to about seven significant
/// digits, which a graph cannot show anyway) and the GPU series only once
/// the process has used the GPU.
#[derive(Default)]
pub(super) struct Track {
    /// The latest reading; kept after the process exits.
    pub(super) last: ProcInfo,
    /// The process was missing from the latest sample.
    pub(super) exited: bool,
    /// CPU in hundredths of a percent of one core.
    cpu: VecDeque<f32>,
    ram: VecDeque<f32>,
    commit: VecDeque<f32>,
    read: VecDeque<f32>,
    write: VecDeque<f32>,
    /// GPU in hundredths of a percent; empty until `has_gpu`.
    gpu: VecDeque<f32>,
    /// Empty until `has_gpu`.
    vram: VecDeque<f32>,
    has_gpu: bool,
}

impl Track {
    /// Record `p`, keeping at most `limit` readings per series.
    fn push(&mut self, p: &ProcInfo, limit: usize) {
        self.last = p.clone();
        self.exited = false;
        if limit == 0 {
            return;
        }
        let io = p
            .io_rates
            .unwrap_or(crate::domain::process::IoRates { read: 0, write: 0 });
        for (series, value) in [
            (&mut self.cpu, (p.cpu_p.max(0.0) * 100.0) as f32),
            (&mut self.ram, p.ram as f32),
            (&mut self.commit, p.mem as f32),
            (&mut self.read, io.read as f32),
            (&mut self.write, io.write as f32),
        ] {
            append(series, value, limit);
        }
        self.has_gpu |= p.gpu_p.is_some_and(|v| v > 0.0) || p.gpu_mem.is_some_and(|v| v > 0);
        if self.has_gpu {
            // Every earlier reading was zero, or `has_gpu` would be set.
            let earlier = self.cpu.len() - 1;
            for (series, value) in [
                (
                    &mut self.gpu,
                    (p.gpu_p.unwrap_or(0.0).max(0.0) * 100.0) as f32,
                ),
                (&mut self.vram, p.gpu_mem.unwrap_or(0) as f32),
            ] {
                if series.len() < earlier {
                    series.reserve_exact(earlier - series.len() + 1);
                    series.resize(earlier, 0.0);
                }
                append(series, value, limit);
            }
        }
    }

    /// Drop readings beyond `limit` and give back the spare memory.
    fn trim(&mut self, limit: usize) {
        for series in [
            &mut self.cpu,
            &mut self.ram,
            &mut self.commit,
            &mut self.read,
            &mut self.write,
            &mut self.gpu,
            &mut self.vram,
        ] {
            let excess = series.len().saturating_sub(limit);
            series.drain(..excess);
            series.shrink_to_fit();
        }
    }
}

/// Add `value` to a series, dropping the oldest past `limit`. Grows the
/// buffer by doubling, but never past `limit`.
fn append(series: &mut VecDeque<f32>, value: f32, limit: usize) {
    while series.len() >= limit {
        series.pop_front();
    }
    if series.len() == series.capacity() {
        let grow = series.len().max(8).min(limit - series.len());
        series.reserve_exact(grow);
    }
    series.push_back(value);
}

/// A series as whole numbers, for the graphs.
fn decode(series: &VecDeque<f32>) -> VecDeque<i64> {
    series.iter().map(|&v| v as i64).collect()
}

/// Every running process's track, written by the sampler thread.
#[derive(Default)]
pub(super) struct History {
    tracks: HashMap<u32, Track>,
    /// Processes with a details window open: their tracks outlive them.
    keep: HashSet<u32>,
    /// Readings kept per series for every process; 0 records none.
    limit: usize,
    /// Readings kept for processes in `keep`.
    open_limit: usize,
}

/// How many readings `minutes` of history is at `interval_ms`.
pub(super) fn samples_for(minutes: i64, interval_ms: u64) -> usize {
    let ms = u64::try_from(minutes.max(0)).unwrap_or(0) * 60_000;
    usize::try_from(ms.div_ceil(interval_ms.max(1))).unwrap_or(usize::MAX)
}

impl History {
    /// Keep `minutes` of readings for every process at `interval_ms`;
    /// 0 keeps them only for processes with a window open.
    pub(super) fn set_length(&mut self, minutes: i64, interval_ms: u64) {
        let limit = samples_for(minutes, interval_ms);
        let open_limit = if limit == 0 {
            samples_for(OPEN_WINDOW_MIN, interval_ms)
        } else {
            limit
        };
        if (limit, open_limit) == (self.limit, self.open_limit) {
            return;
        }
        self.limit = limit;
        self.open_limit = open_limit;
        for (pid, track) in &mut self.tracks {
            track.trim(if self.keep.contains(pid) {
                open_limit
            } else {
                limit
            });
        }
    }

    /// Recording for every process is off.
    pub(super) fn off(&self) -> bool {
        self.limit == 0
    }

    /// Add one sample. A PID reused by a new process starts a new track.
    pub(super) fn record(&mut self, procs: &[ProcInfo]) {
        let mut seen = HashSet::with_capacity(procs.len());
        for p in procs {
            seen.insert(p.pid);
            let track = self.tracks.entry(p.pid).or_default();
            if track.last.started != p.started || track.last.name != p.name || track.exited {
                *track = Track::default();
            }
            let limit = if self.keep.contains(&p.pid) {
                self.open_limit
            } else {
                self.limit
            };
            track.push(p, limit);
        }
        let keep = &self.keep;
        self.tracks.retain(|pid, track| {
            if seen.contains(pid) {
                return true;
            }
            track.exited = true;
            keep.contains(pid)
        });
    }

    fn track(&self, pid: u32) -> Option<&Track> {
        self.tracks.get(&pid)
    }
}

/// A details window: the process it follows and facts read when it opened.
struct Window {
    pid: u32,
    /// Full path of the executable, read once.
    path: Option<String>,
    /// Why the last End process failed.
    end_error: Option<String>,
}

/// The open details windows.
#[derive(Default)]
pub(super) struct Details {
    windows: Vec<Window>,
}

impl Details {
    /// Open a window for `pid`, or bring its window forward.
    pub(super) fn open(
        &mut self,
        ctx: &egui::Context,
        history: &std::sync::Mutex<History>,
        pid: u32,
    ) {
        if !self.windows.iter().any(|w| w.pid == pid) {
            self.windows.push(Window {
                pid,
                path: image_path(pid),
                end_error: None,
            });
            if let Ok(mut history) = history.lock() {
                history.keep.insert(pid);
            }
        }
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, window_id(pid)));
    }

    /// Draw every open window.
    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        look: &Look,
        history: &std::sync::Mutex<History>,
        sample: Option<&Sample>,
        interval_ms: u64,
    ) {
        if self.windows.is_empty() {
            return;
        }
        let Ok(mut guard) = history.lock() else {
            return;
        };
        let cores = sample.map_or(1, |s| s.cpu.core_count);
        let off = guard.off();
        let mut closed = Vec::new();
        let mut open_parent = None;
        let screen = ctx.screen_rect();
        for (i, window) in self.windows.iter_mut().enumerate() {
            let Some(track) = guard.track(window.pid) else {
                closed.push(window.pid);
                continue;
            };
            let p = &track.last;
            let mut open = true;
            let title = format!("{} ({})", p.name, p.pid);
            egui::Window::new(text(title, look.fg).strong())
                .id(window_id(window.pid))
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_size(vec2(600.0, 720.0))
                .default_pos(screen.center() + vec2(24.0, 24.0) * i as f32 - vec2(300.0, 360.0))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // Keep content clear of the floating scroll bar.
                            ui.set_max_width(ui.available_width() - 14.0);
                            header(ui, look, track, window);
                            ui.add_space(6.0);
                            graphs(ui, look, track, cores, interval_ms, off);
                            ui.add_space(4.0);
                            ui.label(text("Details", look.fg).strong());
                            ui.add_space(4.0);
                            if let Some(parent) = facts(ui, look, track, window, sample, cores) {
                                open_parent = Some(parent);
                            }
                        });
                });
            if !open {
                closed.push(window.pid);
            }
        }
        let limit = guard.limit;
        for pid in &closed {
            guard.keep.remove(pid);
            if let Some(track) = guard.tracks.get_mut(pid) {
                track.trim(limit);
            }
        }
        drop(guard);
        self.windows.retain(|w| !closed.contains(&w.pid));
        if let Some(parent) = open_parent {
            self.open(ctx, history, parent);
        }
    }
}

fn window_id(pid: u32) -> egui::Id {
    egui::Id::new(("process-details", pid))
}

/// The process name, whether it is still running, and End process.
fn header(ui: &mut Ui, look: &Look, track: &Track, window: &mut Window) {
    let p = &track.last;
    ui.horizontal(|ui| {
        ui.label(text(&p.name, look.fg).strong().size(15.0));
        ui.add_space(8.0);
        let (state, color) = if track.exited {
            ("exited".to_owned(), look.warn)
        } else {
            (p.state.to_string().to_lowercase(), look.label)
        };
        ui.label(text(state, color));
        if track.exited {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(text("End process", look.warn))
                .on_hover_text("Stop this process immediately, like Task Manager's End task")
                .clicked()
            {
                window.end_error = end_process(p.pid).err();
            }
        });
    });
    if let Some(error) = &window.end_error {
        ui.label(text(
            format!("Could not end the process: {error}"),
            look.warn,
        ));
    }
}

/// Terminate `pid`. Refuses the System pseudo-processes and Windows'
/// critical processes, whose end would crash the machine.
fn end_process(pid: u32) -> Result<(), String> {
    use windows::Win32::Foundation::E_ACCESSDENIED;
    use windows::Win32::System::Threading::{
        IsProcessCritical, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        TerminateProcess,
    };
    if pid == 0 || pid == 4 {
        return Err("Windows does not allow ending this process".to_owned());
    }
    let explain = |e: windows::core::Error| {
        if e.code() == E_ACCESSDENIED {
            "access denied. It may belong to another user or run elevated; \
             run rztop as administrator to end it."
                .to_owned()
        } else {
            e.message()
        }
    };
    // SAFETY: the handle is checked and closed by OwnedHandle; `critical`
    // outlives the call that writes it.
    unsafe {
        let handle = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )
        .map_err(explain)?;
        let handle = crate::collect::win::OwnedHandle::new(handle)
            .ok_or_else(|| "the process could not be opened".to_owned())?;
        let mut critical = windows::core::BOOL::default();
        if IsProcessCritical(handle.get(), &mut critical).is_ok() && critical.as_bool() {
            return Err(
                "it is a critical Windows process; ending it would crash Windows".to_owned(),
            );
        }
        TerminateProcess(handle.get(), 1).map_err(explain)
    }
}

/// The facts table. Returns the parent's PID when its link was clicked.
fn facts(
    ui: &mut Ui,
    look: &Look,
    track: &Track,
    window: &Window,
    sample: Option<&Sample>,
    cores: usize,
) -> Option<u32> {
    let p = &track.last;
    let mut parent_clicked = None;
    egui::Grid::new(("process-facts", p.pid))
        .num_columns(2)
        .spacing([16.0, 5.0])
        .striped(false)
        .show(ui, |ui| {
            let row = |ui: &mut Ui, name: &str, value: String| {
                ui.label(text(name, look.label));
                ui.label(text(value, look.fg));
                ui.end_row();
            };
            row(ui, "PID", p.pid.to_string());

            ui.label(text("Parent", look.label));
            let parent = sample.and_then(|s| s.procs.iter().find(|q| q.pid == p.ppid));
            match parent {
                Some(parent) if p.ppid != 0 => {
                    let link = ui
                        .add(
                            egui::Label::new(
                                text(format!("{} ({})", parent.name, parent.pid), look.fg)
                                    .underline(),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Show the parent's details");
                    if link.clicked() {
                        parent_clicked = Some(parent.pid);
                    }
                }
                _ => {
                    ui.label(text(format!("{} (not running)", p.ppid), look.label));
                }
            }
            ui.end_row();

            let user = if p.user.is_empty() {
                NEEDS_ADMIN.to_owned()
            } else {
                p.user.clone()
            };
            row(ui, "User", user);
            if let Some(path) = &window.path {
                ui.label(text("Path", look.label));
                if one_line(ui, look, path, "Open folder") {
                    let _ = std::process::Command::new("explorer")
                        .arg(format!("/select,{path}"))
                        .spawn();
                }
                ui.end_row();
            }
            ui.label(text("Command line", look.label));
            if one_line(ui, look, &p.cmd, "Copy") {
                ui.ctx().copy_text(p.cmd.clone());
            }
            ui.end_row();
            if let Some(started) = p.started
                && let Some(now) = now_filetime()
            {
                let seconds = now.saturating_sub(started) / 10_000_000;
                row(
                    ui,
                    "Running for",
                    crate::tools::sec_to_dhms(seconds, false, false),
                );
            }
            row(ui, "Priority", p.priority.to_string());
            row(ui, "Threads", p.threads.to_string());
            row(
                ui,
                "CPU time",
                crate::tools::sec_to_dhms(p.cpu_time / 10_000_000, false, false),
            );
            let cpu = display_cpu_percent(p.cpu_p, cores, look.per_core);
            row(ui, "CPU", format!("{cpu:.1}%"));
            if memory_hidden(p) {
                ui.label(text("Memory", look.label));
                ui.label(text(NEEDS_ADMIN, look.warn));
                ui.end_row();
            } else {
                row(
                    ui,
                    "Memory",
                    format!("{} RAM, {} commit", look.bytes(p.ram), look.bytes(p.mem)),
                );
            }
            row(
                ui,
                "I/O total",
                format!(
                    "{} read, {} written",
                    look.bytes(p.io_read),
                    look.bytes(p.io_write)
                ),
            );
            if track.has_gpu {
                row(
                    ui,
                    "GPU",
                    format!(
                        "{:.1}%, {} VRAM",
                        p.gpu_p.unwrap_or(0.0),
                        look.bytes(p.gpu_mem.unwrap_or(0))
                    ),
                );
            }
        });
    parent_clicked
}

/// One line of monospace text, cut short with an ellipsis when it does not
/// fit, then a small button. True when clicked.
fn one_line(ui: &mut Ui, look: &Look, value: &str, button: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        let button_w = ui.fonts(|f| {
            f.layout_no_wrap(
                button.to_owned(),
                egui::TextStyle::Button.resolve(ui.style()),
                look.fg,
            )
            .size()
            .x
        }) + 2.0 * ui.spacing().button_padding.x
            + ui.spacing().item_spacing.x
            + 4.0;
        let width = (ui.available_width() - button_w).max(40.0);
        ui.allocate_ui(vec2(width, ui.spacing().interact_size.y), |ui| {
            // egui shows the whole text on hover when it is cut short.
            ui.add(egui::Label::new(text(value, look.fg).monospace()).truncate());
        });
        clicked = ui.small_button(button).clicked();
    });
    clicked
}
/// The graphs: CPU, memory, I/O and, when the process uses one, the GPU.
/// `off`: only processes with a window open are recorded.
fn graphs(ui: &mut Ui, look: &Look, track: &Track, cores: usize, interval_ms: u64, off: bool) {
    let width = ui.available_width();
    let cols = graph_columns(width);
    let span = cols.min(track.cpu.len()) as u64 * interval_ms / 1000;
    let span = crate::tools::sec_to_dhms(span, true, false);
    // Until the graphs fill, they hold everything rztop has seen of it.
    let caption = if off && track.cpu.len() < cols {
        format!("Last {span} · since this window opened")
    } else if track.cpu.len() < cols {
        format!("Last {span} · since the process appeared or rztop started")
    } else {
        format!("Last {span}")
    };
    ui.label(text(caption, look.label).size(11.5));
    ui.add_space(6.0);

    // CPU in the same units as the tables (one core or all cores).
    let scale = if look.per_core {
        1
    } else {
        cores.max(1) as i64
    };
    let cpu: VecDeque<i64> = track.cpu.iter().map(|&v| v as i64 / scale).collect();
    let pct = |v: i64| format!("{:.1}%", v as f64 / 100.0);
    // Per-core readings can reach 100% for every core.
    let full = 100
        * 100
        * if look.per_core {
            cores.max(1) as i64
        } else {
            1
        };
    single(ui, look, "CPU", &cpu, &look.load, &pct, full);

    mirrored(
        ui,
        look,
        "Memory",
        [
            ("RAM", &decode(&track.ram), &look.fill),
            ("Commit", &decode(&track.commit), &look.mem_cache),
        ],
        &|v| look.bytes(v.max(0) as u64),
        true,
        memory_hidden(&track.last),
    );
    mirrored(
        ui,
        look,
        "I/O",
        [
            ("Read", &decode(&track.read), &look.inbound),
            ("Write", &decode(&track.write), &look.outbound),
        ],
        &|v| look.rate(v.max(0) as u64),
        false,
        false,
    );
    if track.has_gpu {
        let gpu_pct = |v: i64| format!("{:.1}%", v as f64 / 100.0);
        single(
            ui,
            look,
            "GPU",
            &decode(&track.gpu),
            &look.load,
            &gpu_pct,
            100 * 100,
        );
        single(
            ui,
            look,
            "VRAM",
            &decode(&track.vram),
            &look.fill,
            &|v| look.bytes(v.max(0) as u64),
            0,
        );
    }
}

const NEEDS_ADMIN: &str = "unknown (needs administrator)";

/// Windows hides the memory of some processes (services, other users'
/// processes) from a non-elevated rztop; both counters then read zero,
/// which no running process really has.
fn memory_hidden(p: &ProcInfo) -> bool {
    p.ram == 0 && p.mem == 0
}

/// Largest value in the part of `series` a graph `width` wide shows.
fn peak(series: &VecDeque<i64>, width: f32) -> i64 {
    let cols = graph_columns(width);
    series
        .iter()
        .skip(series.len().saturating_sub(cols))
        .copied()
        .max()
        .unwrap_or(0)
}

/// A heading row: the name on the left, readouts on the right.
fn heading(ui: &mut Ui, look: &Look, name: &str, parts: &[(String, Color32)]) {
    ui.horizontal(|ui| {
        ui.label(text(name, look.fg).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (part, color) in parts.iter().rev() {
                ui.label(text(part, *color));
            }
        });
    });
}

fn graph_rect(ui: &mut Ui, look: &Look) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), GRAPH_H), Sense::hover());
    let painter = ui.painter_at(rect.expand(1.0));
    painter.rect_filled(
        rect,
        CornerRadius::same(3),
        mix(look.bg, Color32::BLACK, 0.25),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(1.0_f32, mix(look.bg, look.dim, 0.6)),
        egui::StrokeKind::Outside,
    );
    rect
}

/// One series growing up. `full` is the value that fills the graph, or 0
/// for a byte series scaled to its visible peak; small loads scale up so
/// they stay visible.
fn single(
    ui: &mut Ui,
    look: &Look,
    name: &str,
    series: &VecDeque<i64>,
    grad: &Gradient,
    format: &dyn Fn(i64) -> String,
    full: i64,
) {
    let now = series.back().copied().unwrap_or(0);
    let width = ui.available_width();
    let top = peak(series, width);
    // Scale to the peak, rounded up to a step, but never above `full`.
    let max = if full > 0 {
        nice_ceiling(top.max(full / 100)).min(full)
    } else {
        byte_ceiling(top)
    };
    heading(
        ui,
        look,
        name,
        &[
            (format(now), grad.at(pct(now, max))),
            (format!("  peak {}", format(top)), look.label),
        ],
    );
    let rect = graph_rect(ui, look);
    let painter = ui.painter_at(rect);
    dot_graph_scaled(
        &painter,
        rect.shrink2(vec2(2.0, 2.0)),
        series,
        Grow::Up,
        grad,
        max,
    );
    caption(
        &painter,
        look,
        rect.left_top() + vec2(4.0, 3.0),
        Align2::LEFT_TOP,
        format(max),
        look.dim,
    );
    ui.add_space(10.0);
}

/// Two series sharing a graph: the first grows up from the middle, the
/// second down, like the network panel. `shared` gives both one scale.
fn mirrored(
    ui: &mut Ui,
    look: &Look,
    name: &str,
    series: [(&str, &VecDeque<i64>, &Gradient); 2],
    format: &dyn Fn(i64) -> String,
    shared: bool,
    hidden: bool,
) {
    if hidden {
        heading(ui, look, name, &[(NEEDS_ADMIN.to_owned(), look.warn)]);
        let rect = graph_rect(ui, look);
        ui.painter_at(rect).text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Run rztop as administrator to see its memory",
            super::mono(),
            look.label,
        );
        ui.add_space(10.0);
        return;
    }
    let width = ui.available_width();
    let [(a_name, a, a_grad), (b_name, b, b_grad)] = series;
    let (a_top, b_top) = (peak(a, width), peak(b, width));
    let (a_max, b_max) = if shared {
        let m = byte_ceiling(a_top.max(b_top));
        (m, m)
    } else {
        (byte_ceiling(a_top), byte_ceiling(b_top))
    };
    let (a_now, b_now) = (
        a.back().copied().unwrap_or(0),
        b.back().copied().unwrap_or(0),
    );
    heading(
        ui,
        look,
        name,
        &[
            (format!("{a_name} "), look.label),
            (format(a_now), a_grad.at(pct(a_now, a_max))),
            (format!("   {b_name} "), look.label),
            (format(b_now), b_grad.at(pct(b_now, b_max))),
        ],
    );
    let rect = graph_rect(ui, look);
    let painter = ui.painter_at(rect);
    let inner = rect.shrink2(vec2(2.0, 2.0));
    let mid = inner.center().y;
    let upper = Rect::from_min_max(inner.min, pos2(inner.right(), mid - 1.0));
    let lower = Rect::from_min_max(pos2(inner.left(), mid + 1.0), inner.max);
    dot_graph_scaled(&painter, upper, a, Grow::Up, a_grad, a_max);
    dot_graph_scaled(&painter, lower, b, Grow::Down, b_grad, b_max);
    caption(
        &painter,
        look,
        rect.left_top() + vec2(4.0, 3.0),
        Align2::LEFT_TOP,
        format!("{a_name} ▲ {}", format(a_max)),
        look.dim,
    );
    caption(
        &painter,
        look,
        rect.left_bottom() + vec2(4.0, -3.0),
        Align2::LEFT_BOTTOM,
        format!("{b_name} ▼ {}", format(b_max)),
        look.dim,
    );
    ui.add_space(10.0);
}

fn pct(value: i64, max: i64) -> i64 {
    (value.max(0).saturating_mul(100) / max.max(1)).min(100)
}

/// The next 1, 2 or 5 times a power of ten at or above `value` (at least 1),
/// so graph scales read as round numbers and do not jitter.
fn nice_ceiling(value: i64) -> i64 {
    let value = value.max(1);
    let mut step = 1_i64;
    loop {
        for m in [1, 2, 5] {
            let candidate = step.saturating_mul(m);
            if candidate >= value {
                return candidate;
            }
        }
        if step > i64::MAX / 10 {
            return i64::MAX;
        }
        step *= 10;
    }
}

/// The next power of two at or above `value` (at least 1 KiB), so byte
/// scales read as 512MB, 1GB, 2GB rather than odd numbers.
fn byte_ceiling(value: i64) -> i64 {
    let value = u64::try_from(value).unwrap_or(0).max(1024);
    i64::try_from(value.next_power_of_two()).unwrap_or(i64::MAX)
}

/// The time now as a FILETIME (100 ns ticks since 1601, UTC).
fn now_filetime() -> Option<u64> {
    // SAFETY: GetSystemTimeAsFileTime takes no arguments and cannot fail.
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    Some((u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime))
}

/// The executable's full path, if the process can be opened.
fn image_path(pid: u32) -> Option<String> {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::core::PWSTR;
    // SAFETY: the handle is checked and closed by OwnedHandle; the buffer
    // outlives the call and `len` holds its capacity in characters.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let handle = crate::collect::win::OwnedHandle::new(handle)?;
        let mut buffer = vec![0_u16; 32_768];
        let mut len = buffer.len() as u32;
        QueryFullProcessImageNameW(
            handle.get(),
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .ok()?;
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A history keeping 15 minutes at 1 s.
    fn recording() -> History {
        let mut history = History::default();
        history.set_length(15, 1000);
        history
    }

    #[test]
    fn shorter_history_trims_and_off_records_only_open_windows() {
        let mut history = recording();
        for _ in 0..100 {
            history.record(&[proc(4, "a.exe", 1, 1.0), proc(8, "b.exe", 1, 1.0)]);
        }
        // One minute at 2 s is 30 readings.
        history.set_length(1, 2000);
        assert_eq!(history.track(4).map(|t| t.cpu.len()), Some(30));

        history.keep.insert(8);
        history.set_length(0, 1000);
        assert!(history.off());
        assert_eq!(history.track(4).map(|t| t.cpu.len()), Some(0));
        assert_eq!(history.track(8).map(|t| t.cpu.len()), Some(30));
        history.record(&[proc(4, "a.exe", 1, 1.0), proc(8, "b.exe", 1, 1.0)]);
        // Still known, for opening a window, but nothing recorded.
        assert!(history.track(4).is_some_and(|t| t.cpu.is_empty()));
        assert_eq!(history.track(8).map(|t| t.cpu.len()), Some(31));
    }

    fn proc(pid: u32, name: &str, started: u64, cpu_p: f64) -> ProcInfo {
        ProcInfo {
            pid,
            name: name.to_owned(),
            started: Some(started),
            cpu_p,
            ..Default::default()
        }
    }

    #[test]
    fn history_follows_processes_and_resets_on_pid_reuse() {
        let mut history = recording();
        history.record(&[proc(4, "a.exe", 1, 10.0), proc(8, "b.exe", 1, 0.0)]);
        history.record(&[proc(4, "a.exe", 1, 20.0), proc(8, "b.exe", 1, 0.0)]);
        assert_eq!(history.track(4).map(|t| t.cpu.len()), Some(2));
        assert_eq!(
            history.track(4).and_then(|t| t.cpu.back().copied()),
            Some(2000.0)
        );

        // b.exe exits: dropped, unless a window keeps it.
        history.keep.insert(8);
        history.record(&[proc(4, "a.exe", 1, 0.0)]);
        assert!(history.track(8).is_some_and(|t| t.exited));
        history.keep.clear();
        history.record(&[proc(4, "a.exe", 1, 0.0)]);
        assert!(history.track(8).is_none());

        // A new process with PID 4 starts a fresh track.
        history.record(&[proc(4, "c.exe", 2, 5.0)]);
        assert_eq!(history.track(4).map(|t| t.cpu.len()), Some(1));
    }

    #[test]
    fn history_is_capped() {
        let mut history = recording();
        let keep = samples_for(15, 1000);
        assert_eq!(keep, 900);
        for _ in 0..keep + 10 {
            history.record(&[proc(4, "a.exe", 1, 1.0)]);
        }
        let track = history.track(4).expect("tracked");
        assert_eq!(track.ram.len(), keep);
        // Grown to exactly the cap, not the next power of two.
        assert_eq!(track.ram.capacity(), keep);
    }

    #[test]
    fn gpu_series_start_when_the_gpu_is_used() {
        let mut history = recording();
        for _ in 0..3 {
            history.record(&[proc(4, "a.exe", 1, 1.0)]);
        }
        let track = history.track(4).expect("tracked");
        assert!(track.gpu.is_empty() && track.vram.capacity() == 0);

        let mut busy = proc(4, "a.exe", 1, 1.0);
        busy.gpu_p = Some(12.5);
        busy.gpu_mem = Some(1 << 20);
        history.record(&[busy]);
        let track = history.track(4).expect("tracked");
        // Earlier samples read as zero, aligned with the other series.
        assert_eq!(track.gpu.len(), track.cpu.len());
        assert_eq!(
            track.gpu.iter().copied().collect::<Vec<_>>(),
            [0.0, 0.0, 0.0, 1250.0]
        );
        assert_eq!(track.vram.back().copied(), Some(1_048_576.0));
    }

    #[test]
    fn scales_round_up_to_1_2_5_steps() {
        assert_eq!(nice_ceiling(0), 1);
        assert_eq!(nice_ceiling(3), 5);
        assert_eq!(nice_ceiling(12), 20);
        assert_eq!(nice_ceiling(200), 200);
        assert_eq!(nice_ceiling(501), 1000);
        assert_eq!(byte_ceiling(0), 1024);
        assert_eq!(byte_ceiling(1500), 2048);
        assert_eq!(byte_ceiling(1 << 30), 1 << 30);
    }
}
