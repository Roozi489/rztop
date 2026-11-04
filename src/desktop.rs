use crate::domain::process::IoSort;
use crate::{
    collect::{
        Collector, cpu::CpuCollector, disk::DiskCollector, gpu::GpuCollector, memory::MemCollector,
        process::ProcCollector,
    },
    config::Config,
    domain::{
        config_enums::TempScale,
        cpu::CpuInfo,
        disk::{DiskFilter, DiskInfo},
        gpu::GpuInfo,
        memory::MemInfo,
        process::ProcInfo,
    },
    runner::LatestSlot,
    themes::{GradientDef, Rgb},
};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, RichText, Sense, Ui, pos2,
    vec2,
};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{Arc, mpsc},
    thread::JoinHandle,
    time::Duration,
};

const TICK: Duration = Duration::from_secs(1);
/// Bumped whenever panels are added, so an older saved arrangement
/// cannot hide the new panels.
const LAYOUT: &str = "dock-layout-v2";
/// Height of one text, meter or core row.
const ROW: f32 = 18.0;
/// Spacing between graph dots, horizontally and vertically.
const DOT: f32 = 4.0;
const FONT_SIZE: f32 = 13.0;
/// Absolute full-colour I/O rate, matching the terminal rankings.
const IO_FULL_SCALE: u64 = 5_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Panel {
    Cpu,
    Gpu,
    Io,
    Mem,
    Disk,
    CpuProcs,
    MemProcs,
    GpuProcs,
    VramProcs,
}

impl Panel {
    const ALL: [Panel; 9] = [
        Panel::Cpu,
        Panel::Gpu,
        Panel::Io,
        Panel::Mem,
        Panel::Disk,
        Panel::CpuProcs,
        Panel::MemProcs,
        Panel::GpuProcs,
        Panel::VramProcs,
    ];
}

/// Mirrors the terminal's default arrangement: CPU across the top, device
/// panels on the left and the five process rankings stacked on the right.
fn initial_layout() -> DockState<Panel> {
    let mut dock = DockState::new(vec![Panel::Cpu]);
    let tree = dock.main_surface_mut();
    let [_, gpu] = tree.split_below(NodeIndex::root(), 0.33, vec![Panel::Gpu]);
    let [gpu, procs] = tree.split_right(gpu, 0.5, vec![Panel::CpuProcs]);
    let [_, mem] = tree.split_below(gpu, 0.34, vec![Panel::Mem]);
    tree.split_below(mem, 0.62, vec![Panel::Disk]);
    let [_, rest] = tree.split_below(procs, 0.2, vec![Panel::MemProcs]);
    let [_, rest] = tree.split_below(rest, 0.25, vec![Panel::Io]);
    let [_, rest] = tree.split_below(rest, 1.0 / 3.0, vec![Panel::GpuProcs]);
    tree.split_below(rest, 0.5, vec![Panel::VramProcs]);
    dock
}

fn rgb(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// 101-step theme gradient: start → mid over 0–50 %, mid → end over 50–100 %.
struct Gradient([Color32; 101]);

impl Gradient {
    fn new(def: &GradientDef) -> Self {
        let (start, mid, end) = (rgb(def.start), rgb(def.mid), rgb(def.end));
        Self(std::array::from_fn(|i| {
            if i <= 50 {
                mix(start, mid, i as f32 / 50.0)
            } else {
                mix(mid, end, (i - 50) as f32 / 50.0)
            }
        }))
    }

    fn at(&self, pct: i64) -> Color32 {
        self.0[pct.clamp(0, 100) as usize]
    }
}

/// Colours and meter scales taken from the user's rztop configuration.
struct Look {
    bg: Color32,
    fg: Color32,
    label: Color32,
    dim: Color32,
    meter_off: Color32,
    selected: Color32,
    cpu_upper: Gradient,
    cpu_lower: Gradient,
    temp: Gradient,
    gpu: Gradient,
    gpu_clock: Gradient,
    gpu_power: Gradient,
    gpu_vram: Gradient,
    used: Gradient,
    available: Gradient,
    cached: Gradient,
    disk_read: Gradient,
    disk_write: Gradient,
    disk_busy: Gradient,
    cpu_clock_max_mhz: i64,
    gpu_clock_max_mhz: i64,
    gpu_power_max_watts: i64,
    temp_scale: TempScale,
    swap_max_mib: i64,
    per_core: bool,
    base_10: bool,
    disk_filter: DiskFilter,
}

impl Look {
    fn new(config: &Config) -> Self {
        let palette = crate::theme::palette(&config.ui.color_theme);
        let (c, g) = (&palette.colors, &palette.gradients);
        let bg = rgb(c.main_bg);
        Self {
            bg,
            fg: rgb(c.main_fg),
            label: rgb(c.data_label_fg),
            dim: rgb(c.graph_text),
            meter_off: mix(bg, rgb(c.meter_bg), 0.6),
            selected: rgb(c.selected_bg),
            cpu_upper: Gradient::new(&g.cpu_upper),
            cpu_lower: Gradient::new(&g.cpu_lower),
            temp: Gradient::new(&g.temp),
            gpu: Gradient::new(&g.gpu),
            gpu_clock: Gradient::new(&g.gpu_clock),
            gpu_power: Gradient::new(&g.gpu_power),
            gpu_vram: Gradient::new(&g.gpu_vram),
            used: Gradient::new(&g.used),
            available: Gradient::new(&g.available),
            cached: Gradient::new(&g.cached),
            disk_read: Gradient::new(&g.disk_read),
            disk_write: Gradient::new(&g.disk_write),
            disk_busy: Gradient::new(&g.disk_busy),
            cpu_clock_max_mhz: config.cpu.cpu_clock_max_mhz,
            gpu_clock_max_mhz: config.gpu.gpu_clock_max_mhz,
            gpu_power_max_watts: config.gpu.gpu_power_max_watts,
            temp_scale: config.cpu.temp_scale,
            swap_max_mib: config.mem.swap_max_mib,
            per_core: config.view.proc_per_core,
            base_10: config.ui.base_10_sizes,
            disk_filter: DiskFilter::parse(&config.disk.disk_filter),
        }
    }

    fn bytes(&self, value: u64) -> String {
        crate::tools::floating_humanizer(value, true, 0, false, false, self.base_10)
    }

    fn rate(&self, value: u64) -> String {
        crate::tools::floating_humanizer(value, true, 0, false, true, self.base_10)
    }

    fn temp(&self, celsius: i64) -> String {
        let (value, unit) = crate::tools::celsius_to(celsius, self.temp_scale);
        format!("{value}{unit}")
    }
}

struct Sample {
    cpu: CpuInfo,
    gpus: Vec<GpuInfo>,
    mem: MemInfo,
    disks: Vec<DiskInfo>,
    procs: Vec<ProcInfo>,
}

struct Monitor {
    latest: LatestSlot<Sample>,
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl Monitor {
    fn new(ctx: egui::Context) -> Self {
        let latest = LatestSlot::new();
        let slot = latest.clone();
        let (stop, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut cpu = CpuCollector::new();
            let mut gpu = GpuCollector::new();
            let mut mem = MemCollector::new();
            let mut disk = DiskCollector::new();
            let mut processes = ProcCollector::new();
            processes.set_core_count(crate::collect::cpu::get_core_count());
            loop {
                let start = std::time::Instant::now();
                cpu.collect();
                gpu.collect();
                mem.collect();
                disk.collect();
                processes.collect();
                slot.publish(Sample {
                    cpu: cpu.snapshot().info,
                    gpus: gpu.snapshot().devices,
                    mem: mem.snapshot().info,
                    disks: disk.snapshot().info.disks,
                    procs: processes.snapshot().procs,
                });
                ctx.request_repaint();
                if rx.recv_timeout(TICK.saturating_sub(start.elapsed()))
                    != Err(mpsc::RecvTimeoutError::Timeout)
                {
                    break;
                }
            }
        });
        Self {
            latest,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Desktop {
    monitor: Monitor,
    dock: DockState<Panel>,
    gpu: usize,
    sorts: Sorts,
    look: Look,
}

impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(text("rztop", self.look.cpu_upper.at(0)));
                ui.label(text("Drag a caption to dock", self.look.label));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Reset layout").clicked() {
                        self.dock = initial_layout();
                    }
                });
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let mut panels = Panels {
                    captions: Vec::new(),
                    sample: self.monitor.latest.latest(),
                    gpu: &mut self.gpu,
                    sorts: &mut self.sorts,
                    look: &self.look,
                };
                show_dock(ui, &mut self.dock, &mut panels);
            });
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAYOUT, &self.dock);
    }
}

fn show_dock(ui: &mut Ui, dock: &mut DockState<Panel>, panels: &mut Panels) {
    let mut style = egui_dock::Style::from_egui(ui.style());
    style.tab_bar.height = 22.0;
    style.tab.tab_body.inner_margin = egui::Margin::same(8);
    style.overlay.overlay_type = egui_dock::OverlayType::HighlightedAreas;
    // A completed library drag can renumber the entire panel tree.
    let positions = |dock: &DockState<Panel>| Panel::ALL.map(|panel| dock.find_tab(&panel));
    let before = positions(dock);
    DockArea::new(dock)
        .style(style)
        .show_close_buttons(false)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(false)
        .show_inside(ui, panels);
    if before == positions(dock) && dock.surfaces_count() == 1 {
        finish_quick_drop(ui, dock, &panels.captions);
    } else {
        ui.data_mut(|data| data.remove::<(Panel, egui::Pos2)>(ui.id().with("caption-press")));
    }
}

/// Some Windows input devices deliver the press, move, and release between paints.
/// Let the regular docking overlay own any drag it tracks across frames. For a
/// quick sweep, resolve the original caption and the destination from raw events.
fn finish_quick_drop(ui: &Ui, dock: &mut DockState<Panel>, captions: &[(Panel, egui::Rect)]) {
    use egui_dock::{Split, TabDestination, TabInsert};
    let id = ui.id().with("caption-press");
    let mut press: Option<(Panel, egui::Pos2)> = ui.data(|data| data.get_temp(id));
    let mut release = None;
    ui.input(|input| {
        for event in &input.events {
            if let egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                ..
            } = event
            {
                if *pressed {
                    press = captions
                        .iter()
                        .find(|(_, rect)| rect.contains(*pos))
                        .map(|(panel, _)| (*panel, *pos));
                } else {
                    release = press
                        .take()
                        .filter(|(_, origin)| origin.distance(*pos) > 8.0)
                        .map(|(panel, _)| (panel, *pos));
                }
            }
        }
    });

    ui.data_mut(|data| {
        if let Some(p) = press {
            data.insert_temp(id, p);
        } else {
            data.remove::<(Panel, egui::Pos2)>(id);
        }
    });
    let Some((panel, pos)) = release else { return };
    let Some(src) = dock.find_tab(&panel) else {
        return;
    };
    let target = dock.iter_all_tabs().find_map(|((surface, node), _)| {
        let rect = dock[surface][node].rect()?;
        ((surface, node) != (src.0, src.1) && rect.contains(pos)).then_some((surface, node, rect))
    });
    if let Some((surface, node, rect)) = target {
        let x = (pos.x - rect.left()) / rect.width();
        let y = (pos.y - rect.top()) / rect.height();
        let (dist, side) = [
            (x, Split::Left),
            (1.0 - x, Split::Right),
            (y, Split::Above),
            (1.0 - y, Split::Below),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .expect("four edges");
        let how = if dist < 0.25 {
            TabInsert::Split(side)
        } else {
            TabInsert::Append
        };
        dock.move_tab(src, TabDestination::Node(surface, node, how));
        ui.ctx().request_repaint();
    }
}

fn text(value: impl Into<String>, color: Color32) -> RichText {
    RichText::new(value)
        .monospace()
        .size(FONT_SIZE)
        .color(color)
}

fn mono() -> FontId {
    FontId::monospace(FONT_SIZE)
}

fn char_width(ui: &Ui) -> f32 {
    ui.fonts(|fonts| fonts.glyph_width(&mono(), '0'))
}

/// Reserve one full-width row and return its rectangle.
fn row(ui: &mut Ui) -> Rect {
    let width = ui.available_width();
    ui.allocate_exact_size(vec2(width, ROW), Sense::hover()).0
}

/// Number of lit meter cells, rounded to the nearest cell.
fn filled_cells(pct: i64, cells: usize) -> usize {
    (pct.clamp(0, 100) as usize * cells + 50) / 100
}

/// Segmented meter. Lit cells take the gradient colour of their position,
/// so a full bar runs from the gradient start to its end.
fn segments(painter: &Painter, rect: Rect, pct: i64, grad: &Gradient, off: Color32) {
    const PITCH: f32 = 7.0;
    const GAP: f32 = 1.5;
    let cells = ((rect.width() + GAP) / PITCH).floor().max(1.0) as usize;
    let width = (rect.width() - GAP * (cells - 1) as f32) / cells as f32;
    let lit = filled_cells(pct, cells);
    let last = cells.saturating_sub(1).max(1);
    for i in 0..cells {
        let color = if i < lit {
            grad.at((i * 100 / last) as i64)
        } else {
            off
        };
        let x = rect.left() + i as f32 * (width + GAP);
        painter.rect_filled(
            Rect::from_min_size(pos2(x, rect.top()), vec2(width, rect.height())),
            CornerRadius::same(1),
            color,
        );
    }
}

/// `Label ■■■■■■■····  value` row. A missing percentage draws no meter.
fn meter_row(
    ui: &mut Ui,
    look: &Look,
    label: &str,
    pct: Option<i64>,
    value: &str,
    grad: &Gradient,
) {
    let rect = row(ui);
    let cw = char_width(ui);
    let painter = ui.painter();
    painter.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        label,
        mono(),
        look.fg,
    );
    let value_color = pct.map_or(look.label, |pct| grad.at(pct));
    painter.text(
        rect.right_center(),
        Align2::RIGHT_CENTER,
        value,
        mono(),
        value_color,
    );
    // Six-character labels share one meter start; longer ones (disk names) push it right.
    let label_chars = label.chars().count().max(6) + 1;
    let bar = Rect::from_min_max(
        pos2(rect.left() + label_chars as f32 * cw, rect.top() + 4.0),
        pos2(rect.right() - 11.0 * cw, rect.bottom() - 4.0),
    );
    if let Some(pct) = pct
        && bar.width() > 8.0
    {
        segments(painter, bar, pct, grad, look.meter_off);
    }
}

#[derive(Clone, Copy)]
enum Grow {
    Up,
    Down,
}

/// Braille-style dot graph with the newest sample at the right edge. Each
/// column is one sample; each dot takes the gradient colour of its height.
/// Columns without history show the zero baseline.
fn dot_graph(painter: &Painter, rect: Rect, series: &VecDeque<i64>, grow: Grow, grad: &Gradient) {
    dot_graph_scaled(painter, rect, series, grow, grad, 100);
}

fn graph_columns(width: f32) -> usize {
    (width / DOT).floor().max(0.0) as usize
}

/// Largest value in the visible window, so throughput graphs auto-scale
/// like the terminal's disk rows. Never below 1.
fn visible_max(series: &VecDeque<i64>, width: f32, current: u64) -> i64 {
    let cols = graph_columns(width);
    series
        .iter()
        .skip(series.len().saturating_sub(cols))
        .copied()
        .chain(std::iter::once(i64::try_from(current).unwrap_or(i64::MAX)))
        .max()
        .unwrap_or(0)
        .max(1)
}

/// [`dot_graph`] for a series whose full scale is `max` rather than 100.
fn dot_graph_scaled(
    painter: &Painter,
    rect: Rect,
    series: &VecDeque<i64>,
    grow: Grow,
    grad: &Gradient,
    max: i64,
) {
    let cols = graph_columns(rect.width());
    let rows = ((rect.height() / DOT).floor() as usize).max(1);
    if cols == 0 {
        return;
    }
    let max = max.max(1) as f32;
    let skip = series.len().saturating_sub(cols);
    let pad = cols - (series.len() - skip);
    let values = std::iter::repeat_n(0, pad).chain(series.iter().skip(skip).copied());
    for (col, value) in values.enumerate() {
        let x = rect.right() - (cols - col) as f32 * DOT + DOT / 2.0;
        let fraction = (value.max(0) as f32 / max).min(1.0);
        let height = (fraction * rows as f32).round() as usize;
        for r in 0..height.clamp(1, rows) {
            let offset = (r as f32 + 0.5) * DOT;
            let y = match grow {
                Grow::Up => rect.bottom() - offset,
                Grow::Down => rect.top() + offset,
            };
            let color = grad.at(((r + 1) * 100 / rows) as i64);
            painter.circle_filled(pos2(x, y), 1.1, color);
        }
    }
}

/// Text drawn over a graph, with a background patch so dots never cross it.
fn caption(
    painter: &Painter,
    look: &Look,
    pos: Pos2,
    align: Align2,
    value: String,
    color: Color32,
) {
    let galley = painter.layout_no_wrap(value, mono(), color);
    let rect = align.anchor_size(pos, galley.size());
    painter.rect_filled(rect.expand(2.0), CornerRadius::same(2), look.bg);
    painter.galley(rect.min, galley, color);
}

fn last(series: &VecDeque<i64>) -> i64 {
    series.back().copied().unwrap_or(0)
}

fn cpu_panel(ui: &mut Ui, look: &Look, cpu: &CpuInfo) {
    let full = ui.available_rect_before_wrap();
    if full.width() >= 620.0 {
        let split = (full.width() * 0.42).round();
        cpu_graph(
            ui,
            look,
            cpu,
            Rect::from_min_size(full.min, vec2(split, full.height())),
        );
        let stats_rect = Rect::from_min_max(pos2(full.left() + split + 14.0, full.top()), full.max);
        let mut stats = ui.new_child(egui::UiBuilder::new().max_rect(stats_rect));
        cpu_stats(&mut stats, look, cpu);
        ui.allocate_rect(full, Sense::hover());
    } else {
        let height = (full.height() * 0.35).clamp(56.0, 180.0);
        let (graph, _) = ui.allocate_exact_size(vec2(full.width(), height), Sense::hover());
        cpu_graph(ui, look, cpu, graph);
        ui.add_space(6.0);
        cpu_stats(ui, look, cpu);
    }
}

/// User time grows up from the centre line, system time grows down.
fn cpu_graph(ui: &Ui, look: &Look, cpu: &CpuInfo, rect: Rect) {
    let painter = ui.painter_at(rect);
    let mid = rect.center().y;
    let (user, system) = (&cpu.cpu_percent.user, &cpu.cpu_percent.system);
    let upper = Rect::from_min_max(rect.min, pos2(rect.right(), mid - 1.0));
    let lower = Rect::from_min_max(pos2(rect.left(), mid + 1.0), rect.max);
    dot_graph(&painter, upper, user, Grow::Up, &look.cpu_upper);
    dot_graph(&painter, lower, system, Grow::Down, &look.cpu_lower);
    caption(
        &painter,
        look,
        rect.right_top() + vec2(-2.0, 2.0),
        Align2::RIGHT_TOP,
        format!("user {}%", last(user)),
        look.cpu_upper.at(last(user)),
    );
    caption(
        &painter,
        look,
        rect.right_bottom() + vec2(-2.0, -2.0),
        Align2::RIGHT_BOTTOM,
        format!("system {}%", last(system)),
        look.cpu_lower.at(last(system)),
    );
}

fn cpu_stats(ui: &mut Ui, look: &Look, cpu: &CpuInfo) {
    let title = row(ui);
    ui.painter().text(
        title.right_center(),
        Align2::RIGHT_CENTER,
        &cpu.cpu_name,
        mono(),
        look.fg,
    );

    let total = cpu.cpu_percent.total.back().copied();
    let total_text = total.map_or_else(|| "N/A".to_owned(), |v| format!("{v}%"));
    meter_row(ui, look, "CPU", total, &total_text, &look.cpu_upper);

    let scale = if look.cpu_clock_max_mhz > 0 {
        look.cpu_clock_max_mhz as u64
    } else {
        u64::from(cpu.peak_clock_mhz)
    };
    let clock = (cpu.clock_mhz > 0).then(|| {
        (u64::from(cpu.clock_mhz) * 100)
            .checked_div(scale)
            .unwrap_or(0)
            .min(100) as i64
    });
    let clock_text = if clock.is_some() { &cpu.cpu_hz } else { "N/A" };
    meter_row(ui, look, "Clock", clock, clock_text, &look.cpu_upper);

    if let Some(t) = cpu
        .temp
        .first()
        .and_then(|t| t.back())
        .copied()
        .filter(|t| *t > 0)
    {
        meter_row(ui, look, "Temp", Some(t), &look.temp(t), &look.temp);
    }
    if let Some(watts) = cpu.cpu_watts {
        match cpu.cpu_max_watts.filter(|max| *max > 0.0) {
            Some(max) => meter_row(
                ui,
                look,
                "Watts",
                Some((watts / max * 100.0).clamp(0.0, 100.0) as i64),
                &format!("{watts:.0}W/{max:.0}W"),
                &look.cpu_upper,
            ),
            None => meter_row(
                ui,
                look,
                "Watts",
                None,
                &format!("{watts:.1} W"),
                &look.cpu_upper,
            ),
        }
    }

    ui.add_space(4.0);
    let header = row(ui);
    ui.painter().text(
        header.left_center(),
        Align2::LEFT_CENTER,
        "Cores",
        mono(),
        look.label,
    );
    cores(ui, look, cpu);
}

/// Per-core rows: index, mini history graph, usage and (when known) temperature.
/// Cores fill columns top to bottom, matching the terminal layout.
fn cores(ui: &mut Ui, look: &Look, cpu: &CpuInfo) {
    let count = cpu.core_percent.len();
    if count == 0 {
        return;
    }
    let cw = char_width(ui);
    let width = ui.available_width();
    let cols = ((width / (30.0 * cw)) as usize).clamp(1, 4).min(count);
    let per_col = count.div_ceil(cols);
    let temp_of = |i: usize| {
        cpu.temp
            .get(i + 1)
            .and_then(|t| t.back())
            .copied()
            .filter(|t| *t > 0)
    };
    let show_temp = (0..count).any(|i| temp_of(i).is_some());
    let gap = 2.0 * cw;
    let cell_w = (width - gap * (cols - 1) as f32) / cols as f32;
    for r in 0..per_col {
        let line = row(ui);
        let painter = ui.painter();
        let cy = line.center().y;
        for c in 0..cols {
            let i = c * per_col + r;
            let Some(history) = cpu.core_percent.get(i) else {
                continue;
            };
            let left = line.left() + c as f32 * (cell_w + gap);
            let mut right = left + cell_w;
            painter.text(
                pos2(left, cy),
                Align2::LEFT_CENTER,
                format!("{i:02}"),
                mono(),
                look.fg,
            );
            if show_temp {
                if let Some(t) = temp_of(i) {
                    painter.text(
                        pos2(right, cy),
                        Align2::RIGHT_CENTER,
                        look.temp(t),
                        mono(),
                        look.temp.at(t),
                    );
                }
                right -= 6.0 * cw;
            }
            let pct = last(history).clamp(0, 100);
            painter.text(
                pos2(right, cy),
                Align2::RIGHT_CENTER,
                format!("{pct}%"),
                mono(),
                look.cpu_upper.at(pct),
            );
            right -= 5.0 * cw;
            let graph = Rect::from_min_max(
                pos2(left + 3.5 * cw, line.top() + 2.0),
                pos2(right - cw, line.bottom() - 2.0),
            );
            if graph.width() > DOT {
                dot_graph(
                    &ui.painter_at(graph),
                    graph,
                    history,
                    Grow::Up,
                    &look.cpu_upper,
                );
            }
        }
    }
}

fn mhz(value: u32) -> String {
    if value >= 1000 {
        format!("{:.1}GHz", f64::from(value) / 1000.0)
    } else {
        format!("{value}MHz")
    }
}

fn gpu_panel(ui: &mut Ui, look: &Look, gpus: &[GpuInfo], selected: &mut usize) {
    if gpus.is_empty() {
        ui.label(text("No GPU readings available", look.label));
        return;
    }
    *selected %= gpus.len();
    let gpu = &gpus[*selected];
    let count = gpus.len();
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if count > 1 && ui.small_button("▶").clicked() {
                *selected = (*selected + 1) % count;
            }
            ui.label(text(&gpu.name, look.fg));
            if count > 1 && ui.small_button("◀").clicked() {
                *selected = (*selected + count - 1) % count;
            }
        });
    });

    let util = gpu.gpu_percent.utilization.back().copied();
    let util_text = util.map_or_else(|| "N/A".to_owned(), |v| format!("{v}%"));
    meter_row(ui, look, "GPU", util, &util_text, &look.gpu);

    let max_clock = if look.gpu_clock_max_mhz > 0 {
        look.gpu_clock_max_mhz as u64
    } else {
        u64::from(gpu.gpu_max_clock_speed)
    };
    let clock = (u64::from(gpu.gpu_clock_speed) * 100)
        .checked_div(max_clock)
        .map(|pct| pct.min(100) as i64);
    meter_row(
        ui,
        look,
        "Clock",
        clock,
        &mhz(gpu.gpu_clock_speed),
        &look.gpu_clock,
    );

    if let Some(t) = gpu.temp.back().copied().filter(|t| *t > 0) {
        meter_row(ui, look, "Temp", Some(t), &look.temp(t), &look.temp);
    }

    let max_mw = if look.gpu_power_max_watts > 0 {
        look.gpu_power_max_watts.saturating_mul(1000)
    } else {
        gpu.pwr_max_usage
    };
    if gpu.pwr_usage > 0 {
        let watts = gpu.pwr_usage as f64 / 1000.0;
        if max_mw > 0 {
            meter_row(
                ui,
                look,
                "Watts",
                Some((gpu.pwr_usage.saturating_mul(100) / max_mw).clamp(0, 100)),
                &format!("{watts:.0}W/{:.0}W", max_mw as f64 / 1000.0),
                &look.gpu_power,
            );
        } else {
            meter_row(
                ui,
                look,
                "Watts",
                None,
                &format!("{watts:.0}W"),
                &look.gpu_power,
            );
        }
    }

    if let Some(pct) = gpu.mem_used.saturating_mul(100).checked_div(gpu.mem_total) {
        let pct = pct.min(100) as i64;
        let vram = format!("{}/{}", look.bytes(gpu.mem_used), look.bytes(gpu.mem_total));
        meter_row(ui, look, "VRAM", Some(pct), &vram, &look.gpu_vram);
    }

    // Utilisation history fills whatever height remains.
    ui.add_space(6.0);
    let rest = ui.available_rect_before_wrap();
    if rest.height() >= ROW + 3.0 * DOT && rest.width() > 4.0 * DOT {
        let (rect, _) = ui.allocate_exact_size(rest.size(), Sense::hover());
        let painter = ui.painter_at(rect);
        dot_graph(
            &painter,
            rect,
            &gpu.gpu_percent.utilization,
            Grow::Up,
            &look.gpu,
        );
        let now = util.unwrap_or(0);
        caption(
            &painter,
            look,
            rect.right_top() + vec2(-2.0, 2.0),
            Align2::RIGHT_TOP,
            format!("util {now}%"),
            look.gpu.at(now),
        );
    }
}

fn pct_of(value: u64, capacity: u64) -> Option<i64> {
    value
        .saturating_mul(100)
        .checked_div(capacity)
        .map(|pct| pct.min(100) as i64)
}

/// Used / Avail / Cache / Commit / Swap meters, then the used-memory history.
fn mem_panel(ui: &mut Ui, look: &Look, mem: &MemInfo) {
    let s = &mem.stats;
    let total = s.used + s.available;
    let title = row(ui);
    ui.painter().text(
        title.right_center(),
        Align2::RIGHT_CENTER,
        look.bytes(total),
        mono(),
        look.fg,
    );
    meter_row(
        ui,
        look,
        "Used",
        pct_of(s.used, total),
        &look.bytes(s.used),
        &look.used,
    );
    meter_row(
        ui,
        look,
        "Avail",
        pct_of(s.available, total),
        &look.bytes(s.available),
        &look.available,
    );
    if s.cached > 0 {
        meter_row(
            ui,
            look,
            "Cache",
            pct_of(s.cached, total),
            &look.bytes(s.cached),
            &look.cached,
        );
    }
    let commit = if s.commit_limit > 0 {
        format!(
            "{}/{}",
            look.bytes(s.commit_used),
            look.bytes(s.commit_limit)
        )
    } else {
        "N/A".to_owned()
    };
    let commit_pct = pct_of(s.commit_used, s.commit_limit);
    meter_row(ui, look, "Commit", commit_pct, &commit, &look.cpu_upper);

    // A positive swap_max_mib shows allocation against a growth ceiling;
    // otherwise usage against current allocation (see README "Swap").
    let ceiling = u64::try_from(look.swap_max_mib.max(0)).unwrap_or(0) * 1024 * 1024;
    if s.swap_total > 0 || ceiling > 0 {
        let value = if ceiling > 0 {
            s.swap_total
        } else {
            s.swap_used
        };
        let capacity = s.swap_total.max(ceiling);
        let text = format!("{}/{}", look.bytes(value), look.bytes(capacity));
        meter_row(ui, look, "Swap", pct_of(value, capacity), &text, &look.used);
    }

    ui.add_space(6.0);
    let rest = ui.available_rect_before_wrap();
    if rest.height() >= ROW + 3.0 * DOT && rest.width() > 4.0 * DOT {
        let (rect, _) = ui.allocate_exact_size(rest.size(), Sense::hover());
        let painter = ui.painter_at(rect);
        dot_graph(&painter, rect, &mem.percent.used, Grow::Up, &look.used);
        let now = last(&mem.percent.used);
        caption(
            &painter,
            look,
            rect.right_top() + vec2(-2.0, 2.0),
            Align2::RIGHT_TOP,
            format!("used {now}%"),
            look.used.at(now),
        );
    }
}

/// Per drive: a usage meter, then read/write throughput graphs and busy time.
fn disk_panel(ui: &mut Ui, look: &Look, disks: &[DiskInfo]) {
    let disks = look.disk_filter.apply(disks);
    if disks.is_empty() {
        ui.label(text("No disks", look.label));
        return;
    }
    let cw = char_width(ui);
    for disk in disks {
        let label = format!("{} {}", disk.name, disk.fstype);
        let usage = format!("{}/{}", look.bytes(disk.used), look.bytes(disk.total));
        let used = i64::from(disk.used_percent.clamp(0, 100));
        meter_row(ui, look, &label, Some(used), &usage, &look.available);

        let line = row(ui);
        let painter = ui.painter();
        let cy = line.center().y;
        let busy = i64::from(disk.busy_percent.clamp(0, 100));
        painter.text(
            line.right_center(),
            Align2::RIGHT_CENTER,
            format!("{busy}%"),
            mono(),
            look.disk_busy.at(busy),
        );
        painter.text(
            pos2(line.right() - 6.0 * cw, cy),
            Align2::LEFT_CENTER,
            "B",
            mono(),
            look.fg,
        );
        let speed_w = 9.0 * cw;
        let left_end = line.right() - 7.0 * cw;
        let graph_w = ((left_end - line.left() - 2.0 * (cw + speed_w) - 2.0 * cw) / 2.0).max(0.0);
        let mut x = line.left();
        let streams = [
            (
                "R",
                &disk.read_history,
                disk.read_bytes_per_sec,
                &look.disk_read,
            ),
            (
                "W",
                &disk.write_history,
                disk.write_bytes_per_sec,
                &look.disk_write,
            ),
        ];
        for (letter, history, current, grad) in streams {
            let max = visible_max(history, graph_w, current);
            let now = (i64::try_from(current)
                .unwrap_or(i64::MAX)
                .saturating_mul(100)
                / max)
                .min(100);
            painter.text(pos2(x, cy), Align2::LEFT_CENTER, letter, mono(), look.fg);
            painter.text(
                pos2(x + cw + speed_w, cy),
                Align2::RIGHT_CENTER,
                look.rate(current),
                mono(),
                grad.at(now),
            );
            x += cw + speed_w + cw / 2.0;
            let graph = Rect::from_min_max(
                pos2(x, line.top() + 2.0),
                pos2(x + graph_w, line.bottom() - 2.0),
            );
            if graph.width() > DOT {
                dot_graph_scaled(&ui.painter_at(graph), graph, history, Grow::Up, grad, max);
            }
            x += graph_w + cw;
        }
        ui.add_space(4.0);
    }
}

/// One right-aligned value column of a process table.
struct Column {
    header: &'static str,
    chars: f32,
    clickable: bool,
}

fn column(header: &'static str, chars: f32) -> Column {
    Column {
        header,
        chars,
        clickable: false,
    }
}

/// Right edge of each value column, packed against `right`.
fn column_edges(right: f32, columns: &[Column], cw: f32) -> Vec<f32> {
    let mut edges = vec![0.0; columns.len()];
    let mut x = right;
    for (edge, column) in edges.iter_mut().zip(columns).rev() {
        *edge = x;
        x -= column.chars * cw;
    }
    edges
}

type Cell = (String, Color32);

/// Program column plus fixed-width value columns; only visible rows are
/// built. Returns the index of a clicked sortable header.
fn table(
    ui: &mut Ui,
    look: &Look,
    columns: &[Column],
    rows: usize,
    cell: impl Fn(usize) -> (String, Vec<Cell>),
) -> Option<usize> {
    let cw = char_width(ui);
    let header = row(ui);
    let edges = column_edges(header.right(), columns, cw);
    let cy = header.center().y;
    ui.painter().text(
        header.left_center(),
        Align2::LEFT_CENTER,
        "Program",
        mono(),
        look.fg,
    );
    let mut clicked = None;
    for (i, (column, &edge)) in columns.iter().zip(&edges).enumerate() {
        ui.painter().text(
            pos2(edge, cy),
            Align2::RIGHT_CENTER,
            column.header,
            mono(),
            look.fg,
        );
        if column.clickable {
            let rect = Rect::from_min_max(
                pos2(edge - column.chars * cw, header.top()),
                pos2(edge, header.bottom()),
            );
            let response = ui
                .interact(rect, ui.id().with(("sort", i)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                clicked = Some(i);
            }
        }
    }
    let name_right = match (edges.first(), columns.first()) {
        (Some(edge), Some(first)) => edge - first.chars * cw - cw,
        _ => header.right(),
    };
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show_rows(ui, ROW, rows, |ui, range| {
            for index in range {
                let line = row(ui);
                let (name, values) = cell(index);
                let painter = ui.painter();
                let cy = line.center().y;
                let chars = ((name_right - line.left()) / cw).max(1.0) as usize;
                painter.text(
                    line.left_center(),
                    Align2::LEFT_CENTER,
                    crate::tools::ljust(&name, chars, true),
                    mono(),
                    look.fg,
                );
                for ((value, color), &edge) in values.into_iter().zip(&edges) {
                    painter.text(pos2(edge, cy), Align2::RIGHT_CENTER, value, mono(), color);
                }
            }
        });
    clicked
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ranking {
    Cpu,
    Mem,
    Gpu,
    Vram,
    Io,
}

/// Sort choices the user can change by clicking headers, seeded from config.
struct Sorts {
    mem_ram: bool,
    io: IoSort,
}

fn desc_f64(a: Option<f64>, b: Option<f64>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Some(a), Some(b)) => b.total_cmp(&a),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Processes for one ranking, highest first, unknown values last, ties by
/// PID. The System Idle Process (PID 0) is not a consumer and is omitted.
fn ranked<'a>(procs: &'a [ProcInfo], ranking: Ranking, sorts: &Sorts) -> Vec<&'a ProcInfo> {
    let mut list: Vec<&ProcInfo> = procs.iter().filter(|p| p.pid != 0).collect();
    let io = |p: &ProcInfo| p.io_rates.map(|r| sorts.io.value(r));
    list.sort_by(|a, b| {
        let order = match ranking {
            Ranking::Cpu => b.cpu_p.total_cmp(&a.cpu_p),
            Ranking::Mem if sorts.mem_ram => b.ram.cmp(&a.ram),
            Ranking::Mem => b.mem.cmp(&a.mem),
            Ranking::Gpu => desc_f64(a.gpu_p, b.gpu_p),
            Ranking::Vram => b.gpu_mem.cmp(&a.gpu_mem),
            Ranking::Io => io(b).cmp(&io(a)),
        };
        order.then(a.pid.cmp(&b.pid))
    });
    list
}

/// The five process rankings. Primary columns use the CPU gradient; the
/// other columns are dimmed, as in the terminal.
fn ranking_panel(ui: &mut Ui, look: &Look, sample: &Sample, ranking: Ranking, sorts: &mut Sorts) {
    let list = ranked(&sample.procs, ranking, sorts);
    let total_mem = sample.mem.stats.used + sample.mem.stats.available;
    let total_vram = sample.gpus.iter().map(|g| g.mem_total).max().unwrap_or(0);
    let cores = sample.cpu.core_count;
    let ramp = |pct: Option<i64>| pct.map_or(look.dim, |pct| look.cpu_upper.at(pct));
    let cpu =
        |p: &ProcInfo| crate::domain::process::display_cpu_percent(p.cpu_p, cores, look.per_core);
    let gpu = |p: &ProcInfo| {
        p.gpu_p
            .map_or_else(|| "N/A".to_owned(), |v| format!("{v:.1}"))
    };
    let vram = |p: &ProcInfo| {
        p.gpu_mem
            .map_or_else(|| "N/A".to_owned(), |v| look.bytes(v))
    };
    let arrow =
        |on: bool, with: &'static str, without: &'static str| if on { with } else { without };

    let mut columns = match ranking {
        Ranking::Cpu => vec![
            column("CPU%▼", 7.0),
            column("RAM", 7.0),
            column("Commit", 9.0),
        ],
        Ranking::Mem => vec![
            column("CPU%", 7.0),
            column(arrow(sorts.mem_ram, "RAM▼", "RAM"), 7.0),
            column(arrow(!sorts.mem_ram, "Commit▼", "Commit"), 9.0),
        ],
        Ranking::Gpu => vec![column("GPU%▼", 7.0), column("VRAM", 8.0)],
        Ranking::Vram => vec![column("GPU%", 7.0), column("VRAM▼", 8.0)],
        Ranking::Io => vec![
            column(arrow(sorts.io == IoSort::Read, "Read/s▼", "Read/s"), 10.0),
            column(
                arrow(sorts.io == IoSort::Write, "Write/s▼", "Write/s"),
                10.0,
            ),
            column(arrow(sorts.io == IoSort::Total, "Total▼", "Total"), 10.0),
        ],
    };
    match ranking {
        Ranking::Mem => columns[1..].iter_mut().for_each(|c| c.clickable = true),
        Ranking::Io => columns.iter_mut().for_each(|c| c.clickable = true),
        _ => {}
    }

    let clicked = table(ui, look, &columns, list.len(), |i| {
        let p = list[i];
        let values = match ranking {
            Ranking::Cpu => {
                let pct = cpu(p);
                vec![
                    (format!("{pct:.1}"), look.cpu_upper.at(pct as i64)),
                    (look.bytes(p.ram), look.dim),
                    (look.bytes(p.mem), look.dim),
                ]
            }
            Ranking::Mem => vec![
                (format!("{:.1}", cpu(p)), look.dim),
                (look.bytes(p.ram), ramp(pct_of(p.ram, total_mem))),
                (look.bytes(p.mem), ramp(pct_of(p.mem, total_mem))),
            ],
            Ranking::Gpu => vec![
                (gpu(p), ramp(p.gpu_p.map(|v| v as i64))),
                (vram(p), look.dim),
            ],
            Ranking::Vram => vec![
                (gpu(p), look.dim),
                (vram(p), ramp(p.gpu_mem.and_then(|v| pct_of(v, total_vram)))),
            ],
            Ranking::Io => [IoSort::Read, IoSort::Write, IoSort::Total]
                .into_iter()
                .map(|kind| match p.io_rates.map(|r| kind.value(r)) {
                    Some(v) => (
                        look.rate(v),
                        look.cpu_upper
                            .at((v.saturating_mul(100) / IO_FULL_SCALE).min(100) as i64),
                    ),
                    None => ("N/A".to_owned(), look.label),
                })
                .collect(),
        };
        (p.name.clone(), values)
    });
    match (ranking, clicked) {
        (Ranking::Mem, Some(i)) => sorts.mem_ram = i == 1,
        (Ranking::Io, Some(i)) => sorts.io = [IoSort::Read, IoSort::Write, IoSort::Total][i],
        _ => {}
    }
}

struct Panels<'a> {
    captions: Vec<(Panel, egui::Rect)>,
    sample: Option<Arc<Sample>>,
    gpu: &'a mut usize,
    sorts: &'a mut Sorts,
    look: &'a Look,
}

impl TabViewer for Panels<'_> {
    type Tab = Panel;
    fn title(&mut self, panel: &mut Panel) -> egui::WidgetText {
        text(
            match panel {
                Panel::Cpu => "┤ CPU ├",
                Panel::Gpu => "┤ GPU ├",
                Panel::Io => "┤ I/O processes ├",
                Panel::Mem => "┤ Memory ├",
                Panel::Disk => "┤ Disks ├",
                Panel::CpuProcs => "┤ CPU processes ├",
                Panel::MemProcs => "┤ Memory processes ├",
                Panel::GpuProcs => "┤ GPU processes ├",
                Panel::VramProcs => "┤ VRAM processes ├",
            },
            self.look.fg,
        )
        .strong()
        .into()
    }
    fn on_tab_button(&mut self, tab: &mut Panel, response: &egui::Response) {
        self.captions.push((*tab, response.rect));
    }
    fn closeable(&mut self, _: &mut Panel) -> bool {
        false
    }
    fn scroll_bars(&self, _: &Panel) -> [bool; 2] {
        // Panels size their graphs to the visible area; I/O scrolls its own rows.
        [false, false]
    }
    fn ui(&mut self, ui: &mut Ui, panel: &mut Panel) {
        let Some(sample) = self.sample.clone() else {
            ui.label(text("Sampling…", self.look.label));
            return;
        };
        match panel {
            Panel::Cpu => cpu_panel(ui, self.look, &sample.cpu),
            Panel::Gpu => gpu_panel(ui, self.look, &sample.gpus, self.gpu),
            Panel::Mem => mem_panel(ui, self.look, &sample.mem),
            Panel::Disk => disk_panel(ui, self.look, &sample.disks),
            Panel::Io => ranking_panel(ui, self.look, &sample, Ranking::Io, self.sorts),
            Panel::CpuProcs => ranking_panel(ui, self.look, &sample, Ranking::Cpu, self.sorts),
            Panel::MemProcs => ranking_panel(ui, self.look, &sample, Ranking::Mem, self.sorts),
            Panel::GpuProcs => ranking_panel(ui, self.look, &sample, Ranking::Gpu, self.sorts),
            Panel::VramProcs => ranking_panel(ui, self.look, &sample, Ranking::Vram, self.sorts),
        }
    }
}

fn load_config() -> Config {
    let mut config = Config::new();
    let path = crate::tools::config_dir().join("rztop.toml");
    if path.exists() {
        // Read-only: warnings are reported by the terminal frontend, which owns the file.
        let _ = config.load(&path);
    }
    config
}

pub fn run_desktop() -> eframe::Result {
    let config = load_config();
    let look = Look::new(&config);
    let sorts = Sorts {
        mem_ram: config.proc.memory_sort_ram,
        io: config.proc.io_sort,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1150.0, 975.0])
            .with_min_inner_size([410.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rztop desktop",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.style_mut(|s| {
                s.visuals.panel_fill = look.bg;
                s.visuals.window_fill = look.bg;
                s.visuals.extreme_bg_color = mix(look.bg, Color32::BLACK, 0.3);
                s.visuals.faint_bg_color = mix(look.bg, look.fg, 0.04);
                s.visuals.selection.bg_fill = look.selected;
                s.visuals.widgets.noninteractive.bg_stroke.color = look.dim;
                s.visuals.override_text_color = Some(look.fg);
                s.spacing.item_spacing = egui::vec2(4.0, 2.0);
            });
            let dock = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, LAYOUT))
                .unwrap_or_else(initial_layout);
            Ok(Box::new(Desktop {
                dock,
                monitor: Monitor::new(cc.egui_ctx.clone()),
                gpu: 0,
                sorts,
                look,
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, Rect};

    #[test]
    fn gradient_spans_theme_stops() {
        let def = GradientDef {
            start: Rgb(0, 0, 0),
            mid: Rgb(100, 100, 100),
            end: Rgb(200, 0, 0),
        };
        let grad = Gradient::new(&def);
        assert_eq!(grad.at(-5), Color32::from_rgb(0, 0, 0));
        assert_eq!(grad.at(50), Color32::from_rgb(100, 100, 100));
        assert_eq!(grad.at(250), Color32::from_rgb(200, 0, 0));
    }

    #[test]
    fn meter_cells_round_to_nearest() {
        assert_eq!(filled_cells(0, 40), 0);
        assert_eq!(filled_cells(1, 40), 0);
        assert_eq!(filled_cells(2, 40), 1);
        assert_eq!(filled_cells(50, 40), 20);
        assert_eq!(filled_cells(100, 40), 40);
        assert_eq!(filled_cells(150, 40), 40);
    }

    #[test]
    fn rankings_sort_descending_with_unknowns_last_and_skip_idle() {
        use crate::domain::process::IoRates;
        let proc = |pid, cpu_p, ram, mem, gpu_p, io: Option<(u64, u64)>| ProcInfo {
            pid,
            cpu_p,
            ram,
            mem,
            gpu_p,
            io_rates: io.map(|(read, write)| IoRates { read, write }),
            ..Default::default()
        };
        let procs = vec![
            proc(0, 99.0, 0, 0, Some(99.0), Some((99, 99))),
            proc(4, 5.0, 100, 900, None, None),
            proc(8, 20.0, 700, 200, Some(1.0), Some((10, 500))),
            proc(12, 20.0, 300, 300, Some(3.0), Some((400, 0))),
        ];
        let mut sorts = Sorts {
            mem_ram: false,
            io: IoSort::Total,
        };
        let pids = |ranking, sorts: &Sorts| -> Vec<u32> {
            ranked(&procs, ranking, sorts)
                .iter()
                .map(|p| p.pid)
                .collect()
        };
        assert_eq!(pids(Ranking::Cpu, &sorts), [8, 12, 4]);
        assert_eq!(pids(Ranking::Mem, &sorts), [4, 12, 8]);
        assert_eq!(pids(Ranking::Gpu, &sorts), [12, 8, 4]);
        assert_eq!(pids(Ranking::Io, &sorts), [8, 12, 4]);
        sorts.mem_ram = true;
        sorts.io = IoSort::Read;
        assert_eq!(pids(Ranking::Mem, &sorts), [8, 12, 4]);
        assert_eq!(pids(Ranking::Io, &sorts), [12, 8, 4]);
    }

    #[test]
    fn dragging_io_caption_to_cpu_right_edge_creates_a_new_split() {
        let ctx = egui::Context::default();
        let look = Look::new(&Config::new());
        let mut dock = initial_layout();
        let mut time = 0.0;
        let mut frame = |dock: &mut DockState<Panel>, events| {
            time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(740.0, 620.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show_dock(
                        ui,
                        dock,
                        &mut Panels {
                            captions: Vec::new(),
                            sample: None,
                            gpu: &mut 0,
                            sorts: &mut Sorts {
                                mem_ram: false,
                                io: IoSort::Total,
                            },
                            look: &look,
                        },
                    );
                });
            });
        };
        let rect = |dock: &DockState<Panel>, panel| {
            let (surface, node, _) = dock.find_tab(&panel).unwrap();
            dock[surface][node].rect().unwrap()
        };
        frame(&mut dock, vec![]);
        frame(&mut dock, vec![]);
        let source = rect(&dock, Panel::Io).min + egui::vec2(30.0, 10.0);
        let target = rect(&dock, Panel::Cpu).lerp_inside(egui::vec2(0.85, 0.5));
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut dock, vec![Event::PointerMoved(source)]);
        frame(&mut dock, vec![button(source, true)]);
        for step in 1..=12 {
            frame(
                &mut dock,
                vec![Event::PointerMoved(source.lerp(target, step as f32 / 12.0))],
            );
        }
        frame(&mut dock, vec![button(target, false)]);
        frame(&mut dock, vec![]);
        let io = rect(&dock, Panel::Io);
        let cpu = rect(&dock, Panel::Cpu);
        assert!(
            (io.top() - cpu.top()).abs() < 2.0 && io.left() >= cpu.right() - 2.0,
            "cpu={cpu:?} io={io:?}"
        );
        assert_eq!(dock.iter_all_tabs().count(), Panel::ALL.len());
        // A rapid gesture can arrive entirely within one Windows repaint.
        dock = initial_layout();
        frame(&mut dock, vec![]);
        frame(
            &mut dock,
            vec![
                Event::PointerMoved(source),
                button(source, true),
                Event::PointerMoved(target),
                button(target, false),
            ],
        );
        frame(&mut dock, vec![]);
        let io = rect(&dock, Panel::Io);
        let cpu = rect(&dock, Panel::Cpu);
        assert!((io.top() - cpu.top()).abs() < 2.0 && io.left() >= cpu.right() - 2.0);
    }
}
