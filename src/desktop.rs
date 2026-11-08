use crate::{
    collect::{
        Collector,
        cpu::CpuCollector,
        disk::DiskCollector,
        gpu::GpuCollector,
        memory::MemCollector,
        network::NetCollector,
        process::ProcCollector,
        statusbar::{StatusbarCollector, StatusbarInfo},
    },
    config::Config,
    domain::{
        config_enums::TempScale,
        cpu::CpuInfo,
        disk::{DiskFilter, DiskInfo},
        gpu::GpuInfo,
        memory::MemInfo,
        network::NetInfo,
        process::{IoSort, ProcInfo},
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
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::Relaxed},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

mod process_detail;
mod settings;
mod theme_editor;
mod theme_picker;
mod theme_preview;
mod title_bar;

/// Bumped whenever panels are added, so an older saved arrangement
/// cannot hide the new panels.
const LAYOUT: &str = "dock-layout-v3";
/// Storage key of the layout kept with `[layout save]`.
const SAVED_LAYOUT: &str = "saved-layout-v1";
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
    Net,
    CpuProcs,
    MemProcs,
    GpuProcs,
    VramProcs,
}

impl Panel {
    const ALL: [Panel; 10] = [
        Panel::Cpu,
        Panel::Gpu,
        Panel::Io,
        Panel::Mem,
        Panel::Disk,
        Panel::Net,
        Panel::CpuProcs,
        Panel::MemProcs,
        Panel::GpuProcs,
        Panel::VramProcs,
    ];

    fn caption(self) -> &'static str {
        match self {
            Panel::Cpu => "CPU",
            Panel::Gpu => "GPU",
            Panel::Io => "I/O",
            Panel::Mem => "Memory",
            Panel::Disk => "Disks",
            Panel::Net => "Network",
            Panel::CpuProcs => "CPU",
            Panel::MemProcs => "Memory",
            Panel::GpuProcs => "GPU",
            Panel::VramProcs => "VRAM",
        }
    }

    fn ranking(self) -> Option<Ranking> {
        match self {
            Panel::CpuProcs => Some(Ranking::Cpu),
            Panel::MemProcs => Some(Ranking::Mem),
            Panel::Io => Some(Ranking::Io),
            Panel::GpuProcs => Some(Ranking::Gpu),
            Panel::VramProcs => Some(Ranking::Vram),
            _ => None,
        }
    }

    fn index(self) -> usize {
        Panel::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// Vertical split fractions of the default layout: the CPU row against the
/// rest, then GPU, Memory and Network within what is left of the left column.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fractions {
    cpu: f32,
    gpu: f32,
    mem: f32,
    net: f32,
}

impl Fractions {
    /// Before any panel has been measured.
    const GUESS: Fractions = Fractions {
        cpu: 0.33,
        gpu: 0.25,
        mem: 0.38,
        net: 0.68,
    };
}

/// One panel's measured needs: the height of its fixed content (everything
/// but a graph that stretches to fill), and the tab bar and margins around it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fit {
    content: f32,
    /// Height of the panel body the content was drawn in.
    body: f32,
    chrome: f32,
}

/// Separator and rounding slack per panel, so content isn't clipped by a pixel.
const FIT_SLACK: f32 = 3.0;
/// Fitting passes after a reset: measure, fit, and settle once more.
const FIT_FRAMES: u8 = 3;

/// Fractions that give the CPU row and the GPU / Memory / Network / Disks
/// column exactly their fixed content; the column's leftover height goes to
/// the stretching graphs (Network twice as much as GPU and Memory). If the
/// window is too short, the graphs shrink first. `None` until every panel
/// involved has been measured.
fn fit_fractions(height: f32, fits: &[Fit; 10]) -> Option<Fractions> {
    let need = |panel: Panel| {
        let fit = fits[panel.index()];
        (fit.content > 0.0).then_some(fit.content + fit.chrome + FIT_SLACK)
    };
    let cpu = need(Panel::Cpu)?.min(height * 0.6);
    let column = height - cpu;
    let fixed = [need(Panel::Gpu)?, need(Panel::Mem)?, need(Panel::Net)?];
    let disk = need(Panel::Disk)?;
    let spare = column - fixed.iter().sum::<f32>() - disk;
    let [gpu, mem, net] = if spare >= 0.0 {
        let weights = [1.0, 1.0, 2.0];
        std::array::from_fn(|i| fixed[i] + spare * weights[i] / 4.0)
    } else {
        // Too short for everything: scale the stretchable panels down,
        // keeping Disks whole as long as possible.
        let room = (column - disk).max(0.0);
        let scale = room / fixed.iter().sum::<f32>();
        fixed.map(|h| h * scale)
    };
    let frac = |part: f32, whole: f32| (part / whole.max(1.0)).clamp(0.05, 0.95);
    Some(Fractions {
        cpu: frac(cpu, height),
        gpu: frac(gpu, column),
        mem: frac(mem, column - gpu),
        net: frac(net, column - gpu - mem),
    })
}

/// Mirrors the terminal's default arrangement: CPU across the top, device
/// panels on the left and the five process rankings stacked on the right.
fn initial_layout() -> DockState<Panel> {
    layout(Fractions::GUESS)
}

fn layout(f: Fractions) -> DockState<Panel> {
    let mut dock = DockState::new(vec![Panel::Cpu]);
    let tree = dock.main_surface_mut();
    let [_, gpu] = tree.split_below(NodeIndex::root(), f.cpu, vec![Panel::Gpu]);
    let [gpu, procs] = tree.split_right(gpu, 0.5, vec![Panel::CpuProcs]);
    let [_, mem] = tree.split_below(gpu, f.gpu, vec![Panel::Mem]);
    let [_, net] = tree.split_below(mem, f.mem, vec![Panel::Net]);
    tree.split_below(net, f.net, vec![Panel::Disk]);
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
    // Colour roles: a metric takes the gradient for what its number means,
    // whichever panel shows it. Each role maps onto one theme gradient.
    /// Utilization: CPU, cores, GPU, disk busy, process CPU/GPU, I/O total, ping.
    load: Gradient,
    /// Second load series: the system half of the CPU graph.
    load2: Gradient,
    /// CPU, GPU and per-core temperature, positioned by degrees Celsius.
    temp: Gradient,
    /// CPU and GPU power draw.
    power: Gradient,
    /// CPU and GPU clocks.
    clock: Gradient,
    /// How full: RAM used, commit, swap, VRAM, disk space, process memory.
    fill: Gradient,
    /// Memory breakdown bars, kept distinct from Used.
    mem_avail: Gradient,
    mem_cache: Gradient,
    /// Inbound data: network download, disk read, process I/O read.
    inbound: Gradient,
    /// Outbound data: network upload, disk write, process I/O write.
    outbound: Gradient,
    /// Warning text, e.g. the not-admin notice.
    warn: Color32,
    cpu_clock_max_mhz: i64,
    gpu_clock_max_mhz: i64,
    gpu_power_max_watts: i64,
    temp_scale: TempScale,
    swap_max_mib: i64,
    per_core: bool,
    base_10: bool,
    disk_filter: DiskFilter,
    hi: Color32,
    statusbar_bg: Color32,
    statusbar_fg: Color32,
    statusbar_hi: Color32,
    statusbar_sep: Color32,
    /// Fixed graph scales used when network auto-scaling is off.
    net_download: i64,
    net_upload: i64,
}

impl Look {
    fn new(config: &Config) -> Self {
        Self::with_palette(config, &crate::theme::palette(&config.ui.color_theme))
    }

    /// The config's scales and units with `palette`'s colours, for previewing
    /// a theme that is not saved yet.
    fn with_palette(config: &Config, palette: &crate::themes::ThemePalette) -> Self {
        let (c, g) = (&palette.colors, &palette.gradients);
        let bg = rgb(c.main_bg);
        Self {
            bg,
            fg: rgb(c.main_fg),
            label: rgb(c.data_label_fg),
            dim: rgb(c.graph_text),
            meter_off: mix(bg, rgb(c.meter_bg), 0.6),
            selected: rgb(c.selected_bg),
            load: Gradient::new(&g.cpu_upper),
            load2: Gradient::new(&g.cpu_lower),
            temp: Gradient::new(&g.temp),
            power: Gradient::new(&g.gpu_power),
            clock: Gradient::new(&g.gpu_clock),
            fill: Gradient::new(&g.used),
            mem_avail: Gradient::new(&g.available),
            mem_cache: Gradient::new(&g.cached),
            inbound: Gradient::new(&g.download),
            outbound: Gradient::new(&g.upload),
            // The temperature ramp passes yellow/orange around its middle.
            warn: Gradient::new(&g.temp).at(60),
            cpu_clock_max_mhz: config.cpu.cpu_clock_max_mhz,
            gpu_clock_max_mhz: config.gpu.gpu_clock_max_mhz,
            gpu_power_max_watts: config.gpu.gpu_power_max_watts,
            temp_scale: config.cpu.temp_scale,
            swap_max_mib: config.mem.swap_max_mib,
            per_core: config.view.proc_per_core,
            base_10: config.ui.base_10_sizes,
            disk_filter: DiskFilter::parse(&config.disk.disk_filter),
            hi: rgb(c.hi_fg),
            statusbar_bg: rgb(c.statusbar_bg),
            statusbar_fg: rgb(c.statusbar_fg),
            statusbar_hi: rgb(c.statusbar_hi),
            statusbar_sep: rgb(c.statusbar_sep),
            net_download: config.net.net_download,
            net_upload: config.net.net_upload,
        }
    }

    /// Byte size with a full unit. Binary (1024) values use the Windows /
    /// Task Manager labels KB, MB, GB rather than KiB, MiB, GiB, so the numbers
    /// match what Windows shows; `base_10_sizes` switches to 1000-based units.
    fn bytes(&self, value: u64) -> String {
        windows_units(crate::tools::floating_humanizer(
            value,
            false,
            0,
            false,
            false,
            self.base_10,
        ))
    }

    /// `used/total` with full units, sharing the unit when both match:
    /// `28.0/37GB`, but `154MB/5GB`. The total is a fixed capacity, so its
    /// trailing zeros are dropped; the live value keeps them so its width
    /// stays steady.
    fn pair(&self, used: u64, total: u64) -> String {
        let (used, total) = (self.bytes(used), self.bytes(total));
        let split = |s: &str| {
            let at = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
            (s[..at].to_owned(), s[at..].to_owned())
        };
        let ((a, unit), (b, other)) = (split(&used), split(&total));
        let b = trim_zeros(&b);
        if unit == other {
            format!("{a}/{b}{unit}")
        } else {
            format!("{used}/{b}{other}")
        }
    }

    fn rate(&self, value: u64) -> String {
        windows_units(crate::tools::floating_humanizer(
            value,
            false,
            0,
            false,
            true,
            self.base_10,
        ))
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
    nets: Vec<NetInfo>,
}

/// Run `step` every `interval` milliseconds on a background thread,
/// publishing each result and repainting, until `stop` fires. `make`
/// builds the collectors on the worker thread itself.
fn spawn_worker<S, T: Send + Sync + 'static>(
    ctx: egui::Context,
    slot: LatestSlot<T>,
    interval: Arc<AtomicU64>,
    stop: mpsc::Receiver<()>,
    make: impl FnOnce() -> S + Send + 'static,
    mut step: impl FnMut(&mut S) -> T + Send + 'static,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut state = make();
        loop {
            let start = Instant::now();
            slot.publish(step(&mut state));
            ctx.request_repaint();
            let period = Duration::from_millis(interval.load(Relaxed));
            if stop.recv_timeout(period.saturating_sub(start.elapsed()))
                != Err(mpsc::RecvTimeoutError::Timeout)
            {
                break;
            }
        }
    })
}

/// Background sampling. Ping can block for its whole timeout, so the
/// statusbar collector runs on its own thread.
struct Monitor {
    latest: LatestSlot<Sample>,
    status: LatestSlot<StatusbarInfo>,
    /// Shared refresh interval in milliseconds, changed from Settings.
    interval: Arc<AtomicU64>,
    /// Recent readings of every process, for the details windows.
    history: Arc<std::sync::Mutex<process_detail::History>>,
    stops: Vec<mpsc::Sender<()>>,
    workers: Vec<JoinHandle<()>>,
}

impl Monitor {
    fn new(ctx: egui::Context, interval_ms: u64) -> Self {
        let latest = LatestSlot::new();
        let status = LatestSlot::new();
        let interval = Arc::new(AtomicU64::new(interval_ms));
        let history = Arc::new(std::sync::Mutex::new(process_detail::History::default()));
        let recorder = history.clone();
        let (stop_sample, rx_sample) = mpsc::channel();
        let (stop_status, rx_status) = mpsc::channel();
        let sampler = spawn_worker(
            ctx.clone(),
            latest.clone(),
            interval.clone(),
            rx_sample,
            || {
                let mut processes = ProcCollector::new();
                processes.set_core_count(crate::collect::cpu::get_core_count());
                (
                    CpuCollector::new(),
                    GpuCollector::new(),
                    MemCollector::new(),
                    DiskCollector::new(),
                    NetCollector::new(),
                    processes,
                )
            },
            move |(cpu, gpu, mem, disk, net, processes)| {
                cpu.collect();
                gpu.collect();
                mem.collect();
                disk.collect();
                net.collect();
                processes.collect();
                let procs = processes.snapshot().procs;
                if let Ok(mut history) = recorder.lock() {
                    history.record(&procs);
                }
                Sample {
                    cpu: cpu.snapshot().info,
                    gpus: gpu.snapshot().devices,
                    mem: mem.snapshot().info,
                    disks: disk.snapshot().info.disks,
                    procs,
                    nets: net.snapshot().nets,
                }
            },
        );
        let statusbar = spawn_worker(
            ctx,
            status.clone(),
            interval.clone(),
            rx_status,
            StatusbarCollector::new,
            |collector| {
                collector.collect();
                collector.snapshot().info
            },
        );
        Self {
            latest,
            status,
            interval,
            history,
            stops: vec![stop_sample, stop_status],
            workers: vec![sampler, statusbar],
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        for stop in &self.stops {
            let _ = stop.send(());
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

/// `5.00` → `5`, `1.50` → `1.5`, `465` → `465`.
fn trim_zeros(number: &str) -> &str {
    if number.contains('.') {
        number.trim_end_matches('0').trim_end_matches('.')
    } else {
        number
    }
}

/// Relabel IEC binary units (KiB, MiB, …) with the symbols Windows uses for
/// the same 1024-based values (KB, MB, …), written tight like the terminal
/// (`16.0GB`). The numbers are unchanged.
fn windows_units(value: String) -> String {
    value.replace("iB", "B").replace(' ', "")
}

/// Start this executable again through the UAC prompt ("runas"). Returns
/// false when the user declines or the launch fails.
fn relaunch_as_admin() -> bool {
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::{HSTRING, PCWSTR, w},
    };
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let dir = exe.parent().map(HSTRING::from).unwrap_or_default();
    let exe = HSTRING::from(exe.as_path());
    // SAFETY: every string argument is a NUL-terminated wide string owned by
    // this frame (or a static literal) and outlives the call; no owner window
    // and no parameters are passed.
    let result =
        unsafe { ShellExecuteW(None, w!("runas"), &exe, PCWSTR::null(), &dir, SW_SHOWNORMAL) };
    // ShellExecuteW signals success with a value greater than 32.
    result.0 as usize > 32
}

/// Selected network adapter and the graph scaling toggles.
struct NetView {
    selected: String,
    auto: bool,
    sync: bool,
}

struct Desktop {
    monitor: Monitor,
    dock: DockState<Panel>,
    gpu: usize,
    net: NetView,
    sorts: Sorts,
    look: Look,
    /// Measured panel needs, for fitting the default layout.
    fits: [Fit; 10],
    /// Frames left in which to refit the default layout to measured panels.
    fitting: u8,
    /// The tab being dragged, which egui_dock draws on a floating layer.
    dragged: Option<Panel>,
    /// The layout and window placement stored with `[layout save]`.
    saved: Option<SavedLayout>,
    /// Write `saved` to storage at the end of this frame.
    store_saved: bool,
    /// The configuration in effect, edited by the Settings window.
    config: Config,
    config_path: std::path::PathBuf,
    settings: settings::Settings,
    /// Settings changed and not yet written to `rztop.toml`.
    config_dirty: bool,
    /// Outcome of the last write to `rztop.toml`, shown in Settings.
    config_saved: Option<Result<(), String>>,
    /// The title bar's copy of the app icon, redrawn when the theme changes.
    title_icon: Option<egui::TextureHandle>,
    /// Open process details windows.
    details: process_detail::Details,
    /// The process history length last applied: minutes and interval.
    history_len: Option<(i64, u64)>,
}

/// A layout the user chose to keep, with the window's position and size, so
/// `[layout load]` can return to it after experiments. The current layout is
/// still remembered across restarts separately.
#[derive(Clone, Serialize, Deserialize)]
struct SavedLayout {
    dock: DockState<Panel>,
    /// Outer top-left corner of the window, in points.
    position: Option<[f32; 2]>,
    /// Inner (client) size of the window, in points.
    size: Option<[f32; 2]>,
}

impl Desktop {
    /// Rebuild the default layout around the panels' measured needs. Runs for a
    /// few frames after a reset, since a panel's needs can change with the
    /// width it was given (the CPU core grid) and the first frames may come
    /// before any sample.
    fn refit(&mut self, height: f32) {
        for panel in Panel::ALL {
            let fit = &mut self.fits[panel.index()];
            if let Some((surface, node, _)) = self.dock.find_tab(&panel)
                && let Some(rect) = self.dock[surface][node].rect()
            {
                fit.chrome = (rect.height() - fit.body).max(0.0);
            }
        }
        if let Some(fractions) = fit_fractions(height, &self.fits) {
            self.dock = layout(fractions);
            self.fitting -= 1;
        }
    }

    /// The title bar, and whatever its buttons asked for.
    fn title_bar(&mut self, ui: &mut Ui, sample: Option<&Sample>, status: Option<&StatusbarInfo>) {
        let look = &self.look;
        let icon = self.title_icon.get_or_insert_with(|| {
            let size = 32;
            let image =
                egui::ColorImage::from_rgba_unmultiplied([size, size], &icon_rgba(look, size));
            ui.ctx()
                .load_texture("title-icon", image, egui::TextureOptions::LINEAR)
        });
        let own_cpu = sample.and_then(|s| {
            let me = s.procs.iter().find(|p| p.pid == std::process::id())?;
            let pct = crate::domain::process::display_cpu_percent(
                me.cpu_p,
                s.cpu.core_count,
                look.per_core,
            );
            Some(format!("{pct:.1}%"))
        });
        let mut state = title_bar::State {
            icon,
            own_cpu,
            uptime: status.map(|s| {
                crate::tools::uptime_label(s.uptime_seconds)
                    .trim_start_matches("up ")
                    .to_owned()
            }),
            not_admin: crate::collect::permissions::is_elevated() == Some(false),
            admin_limited: status.is_some_and(|s| s.admin_limited),
            has_saved_layout: self.saved.is_some(),
            filter: &mut self.sorts.filter,
        };
        let actions = title_bar::show(ui, look, &mut state);
        let ctx = ui.ctx().clone();
        if actions.settings {
            self.settings.toggle();
        }
        if actions.layout_save {
            let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
            self.saved = Some(SavedLayout {
                dock: self.dock.clone(),
                position: outer.map(|r| [r.min.x, r.min.y]),
                size: inner.map(|r| [r.width(), r.height()]),
            });
            self.store_saved = true;
            ctx.request_repaint();
        }
        if actions.layout_load
            && let Some(saved) = &self.saved
        {
            self.dock = saved.dock.clone();
            if let Some([x, y]) = saved.position {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(x, y)));
            }
            if let Some([w, h]) = saved.size {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(w, h)));
            }
        }
        if actions.layout_reset {
            self.dock = initial_layout();
            self.fitting = FIT_FRAMES;
        }
        // Closing saves the layout, so the elevated instance reopens it. If the
        // UAC prompt is declined this instance simply keeps running.
        if actions.elevate && relaunch_as_admin() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Show the Settings window and apply whatever it changed.
    fn settings_window(&mut self, ctx: &egui::Context, sample: Option<&Sample>) {
        let log_dir = crate::tools::data_dir();
        let info = settings::Context {
            nets: sample.map_or(&[], |s| s.nets.as_slice()),
            drives: sample.map_or_else(Vec::new, |s| {
                s.disks.iter().map(|d| d.name.clone()).collect()
            }),
            config_path: &self.config_path,
            log_dir: &log_dir,
            saved: self.config_saved.as_ref(),
            sample,
        };
        let changes = self.settings.show(ctx, &mut self.config, &self.look, &info);
        if changes.look || changes.preview {
            // The theme editor's draft wins over the configured theme while shown.
            self.look = match self.settings.preview() {
                Some(palette) => Look::with_palette(&self.config, palette),
                None => Look::new(&self.config),
            };
        }
        if changes.theme || changes.preview {
            apply_style(ctx, &self.look);
            self.title_icon = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(app_icon(
                &self.look,
            )))));
        }
        let wanted = (
            self.config.proc.proc_history_min,
            self.monitor.interval.load(Relaxed),
        );
        if self.history_len != Some(wanted)
            && let Ok(mut history) = self.monitor.history.lock()
        {
            history.set_length(wanted.0, wanted.1);
            self.history_len = Some(wanted);
        }
        if changes.interval {
            let ms = u64::try_from(self.config.refresh.update_ms).unwrap_or(1000);
            self.monitor
                .interval
                .store(ms.clamp(100, 86_400_000), Relaxed);
        }
        if changes.net {
            self.net.selected.clone_from(&self.config.view.net_iface);
            self.net.auto = self.config.view.net_auto;
            self.net.sync = self.config.view.net_sync;
        }
        if changes.log {
            crate::log::set_level(self.config.log.log_level);
        }
        self.config_dirty |= changes.any;
        // Save once a drag or click has finished, not on every step of it.
        if self.config_dirty && !ctx.input(|i| i.pointer.any_down()) {
            self.config_dirty = false;
            let result = self.config.save(&self.config_path);
            if let Err(e) = &result {
                tracing::warn!(subsystem = %crate::log::Subsystem::Config, error = %e, "config save failed");
            }
            self.config_saved = Some(result.map_err(|e| e.to_string()));
        }
    }
}

impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.store_saved)
            && let (Some(storage), Some(saved)) = (frame.storage_mut(), &self.saved)
        {
            // Written straight away rather than at the next autosave or exit.
            eframe::set_value(storage, SAVED_LAYOUT, saved);
            storage.flush();
        }
        let sample = self.monitor.latest.latest();
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.dragged = None;
        }
        let status = self.monitor.status.latest();
        egui::TopBottomPanel::top("title")
            .exact_height(title_bar::HEIGHT)
            .frame(
                egui::Frame::NONE
                    .fill(self.look.statusbar_bg)
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ctx, |ui| {
                self.title_bar(ui, sample.as_deref(), status.as_deref())
            });
        let mut open_proc = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let mut panels = Panels {
                    captions: Vec::new(),
                    sample,
                    status,
                    gpu: &mut self.gpu,
                    net: &mut self.net,
                    sorts: &mut self.sorts,
                    look: &self.look,
                    fits: &mut self.fits,
                    dragged: &mut self.dragged,
                    open_proc: &mut open_proc,
                };
                show_dock(ui, &mut self.dock, &mut panels);
                if self.fitting > 0 {
                    self.refit(ui.max_rect().height());
                    ui.ctx().request_repaint();
                }
            });
        let sample = self.monitor.latest.latest();
        if let Some(pid) = open_proc {
            self.details.open(ctx, &self.monitor.history, pid);
        }
        self.details.show(
            ctx,
            &self.look,
            &self.monitor.history,
            sample.as_deref(),
            self.monitor.interval.load(Relaxed),
        );
        self.settings_window(ctx, sample.as_deref());
        title_bar::edges(ctx, &self.look);
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAYOUT, &self.dock);
    }
}

fn show_dock(ui: &mut Ui, dock: &mut DockState<Panel>, panels: &mut Panels) {
    let mut style = egui_dock::Style::from_egui(ui.style());
    style.tab_bar.height = 22.0;
    // A splitter keeps this much of each side; egui_dock's 175 px default
    // stops short panels like Disks from shrinking to their content.
    style.separator.extra = 40.0;
    // Tabs share the whole bar, so a lone caption leaves no empty strip.
    style.tab_bar.fill_tab_bar = true;
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
    let ppp = painter.pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let cells = segment_cells(rect.left(), rect.width(), ppp);
    let lit = filled_cells(pct, cells.len());
    let last = cells.len().saturating_sub(1).max(1);
    let (top, bottom) = (snap(rect.top()), snap(rect.bottom()));
    for (i, (left, right)) in cells.into_iter().enumerate() {
        let color = if i < lit {
            grad.at((i * 100 / last) as i64)
        } else {
            off
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(left, top), pos2(right, bottom)),
            CornerRadius::same(1),
            color,
        );
    }
}

/// Left/right edges of each meter cell, in points but aligned to whole
/// physical pixels. Every cell and every gap has the same pixel width, so
/// the bar looks evenly spaced at any display scale; any remainder smaller
/// than one cell is left empty at the right end.
fn segment_cells(left: f32, width: f32, pixels_per_point: f32) -> Vec<(f32, f32)> {
    let ppp = pixels_per_point.max(0.25);
    let cell = (5.5 * ppp).round().max(2.0);
    let gap = (1.5 * ppp).round().max(1.0);
    let start = (left * ppp).round();
    let count = (((width * ppp).floor() + gap) / (cell + gap))
        .floor()
        .max(1.0) as usize;
    (0..count)
        .map(|i| {
            let x = start + i as f32 * (cell + gap);
            (x / ppp, (x + cell) / ppp)
        })
        .collect()
}

/// Characters reserved for meter values in this panel: the widest value seen
/// so far, so every bar in the panel ends at one column just before its
/// value. It only grows, so bars don't jump when a value gains a digit; a
/// row wider than the stored width asks for a redraw so earlier rows follow.
fn meter_value_chars(ui: &Ui, value: &str) -> usize {
    let id = ui.id().with("meter-value-chars");
    let need = value.chars().count().max(4);
    let (width, grew) = ui.data_mut(|data| {
        let width = data.get_temp_mut_or(id, 0usize);
        let grew = need > *width;
        *width = (*width).max(need);
        (*width, grew)
    });
    if grew {
        ui.ctx().request_repaint();
    }
    width
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
    let value_chars = meter_value_chars(ui, value);
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
        pos2(
            rect.right() - (value_chars as f32 + 1.0) * cw,
            rect.bottom() - 4.0,
        ),
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
/// Columns without history stay empty.
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
    // Columns before the first sample stay empty: dots appear as data does.
    let skip = series.len().saturating_sub(cols);
    let pad = cols - (series.len() - skip);
    for (col, value) in series.iter().skip(skip).copied().enumerate() {
        let col = pad + col;
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

/// Returns the height the stats column needs; the graph beside it stretches.
fn cpu_panel(ui: &mut Ui, look: &Look, cpu: &CpuInfo) -> f32 {
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
        stats.min_rect().bottom() - full.top()
    } else {
        let height = (full.height() * 0.35).clamp(56.0, 180.0);
        let (graph, _) = ui.allocate_exact_size(vec2(full.width(), height), Sense::hover());
        cpu_graph(ui, look, cpu, graph);
        ui.add_space(6.0);
        cpu_stats(ui, look, cpu);
        ui.min_rect().bottom() - full.top()
    }
}

/// User time grows up from the centre line, system time grows down.
fn cpu_graph(ui: &Ui, look: &Look, cpu: &CpuInfo, rect: Rect) {
    let painter = ui.painter_at(rect);
    let mid = rect.center().y;
    let (user, system) = (&cpu.cpu_percent.user, &cpu.cpu_percent.system);
    let upper = Rect::from_min_max(rect.min, pos2(rect.right(), mid - 1.0));
    let lower = Rect::from_min_max(pos2(rect.left(), mid + 1.0), rect.max);
    dot_graph(&painter, upper, user, Grow::Up, &look.load);
    dot_graph(&painter, lower, system, Grow::Down, &look.load2);
    caption(
        &painter,
        look,
        rect.right_top() + vec2(-2.0, 2.0),
        Align2::RIGHT_TOP,
        format!("user {}%", last(user)),
        look.load.at(last(user)),
    );
    caption(
        &painter,
        look,
        rect.right_bottom() + vec2(-2.0, -2.0),
        Align2::RIGHT_BOTTOM,
        format!("system {}%", last(system)),
        look.load2.at(last(system)),
    );
}

fn cpu_stats(ui: &mut Ui, look: &Look, cpu: &CpuInfo) {
    let total = cpu.cpu_percent.total.back().copied();
    let total_text = total.map_or_else(|| "N/A".to_owned(), |v| format!("{v}%"));
    meter_row(ui, look, "CPU", total, &total_text, &look.load);

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
    let clock_text = if clock.is_some() {
        cpu.cpu_hz.replace(' ', "")
    } else {
        "N/A".to_owned()
    };
    meter_row(ui, look, "Clock", clock, &clock_text, &look.clock);

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
                &format!("{watts:.0}/{max:.0}W"),
                &look.power,
            ),
            None => meter_row(
                ui,
                look,
                "Watts",
                None,
                &format!("{watts:.1}W"),
                &look.power,
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
                look.load.at(pct),
            );
            right -= 5.0 * cw;
            let graph = Rect::from_min_max(
                pos2(left + 3.5 * cw, line.top() + 1.0),
                pos2(right - cw, line.bottom() - 1.0),
            );
            if graph.width() > DOT {
                dot_graph(&ui.painter_at(graph), graph, history, Grow::Up, &look.load);
            }
        }
    }
}

fn mhz(value: u32) -> String {
    if value >= 1000 {
        format!("{:.2}GHz", f64::from(value) / 1000.0)
    } else {
        format!("{value}MHz")
    }
}

/// Returns the height of the meters; the utilisation graph below stretches.
/// The GPU name and its ◀ ▶ selector are in the tab bar (`tab_header`).
fn gpu_panel(ui: &mut Ui, look: &Look, gpus: &[GpuInfo], selected: usize) -> f32 {
    let top = ui.cursor().top();
    if gpus.is_empty() {
        ui.label(text("No GPU readings available", look.label));
        return ui.min_rect().bottom() - top;
    }
    let gpu = &gpus[selected % gpus.len()];

    let util = gpu.gpu_percent.utilization.back().copied();
    let util_text = util.map_or_else(|| "N/A".to_owned(), |v| format!("{v}%"));
    meter_row(ui, look, "GPU", util, &util_text, &look.load);

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
        &look.clock,
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
                &format!("{watts:.0}/{:.0}W", max_mw as f64 / 1000.0),
                &look.power,
            );
        } else {
            meter_row(
                ui,
                look,
                "Watts",
                None,
                &format!("{watts:.0}W"),
                &look.power,
            );
        }
    }

    if let Some(pct) = gpu.mem_used.saturating_mul(100).checked_div(gpu.mem_total) {
        let pct = pct.min(100) as i64;
        let vram = look.pair(gpu.mem_used, gpu.mem_total);
        meter_row(ui, look, "VRAM", Some(pct), &vram, &look.fill);
    }

    // Utilisation history fills whatever height remains.
    ui.add_space(6.0);
    let rest = ui.available_rect_before_wrap();
    let fixed = rest.top() - top;
    if rest.height() >= ROW + 3.0 * DOT && rest.width() > 4.0 * DOT {
        let (rect, _) = ui.allocate_exact_size(rest.size(), Sense::hover());
        let painter = ui.painter_at(rect);
        dot_graph(
            &painter,
            rect,
            &gpu.gpu_percent.utilization,
            Grow::Up,
            &look.load,
        );
        let now = util.unwrap_or(0);
        caption(
            &painter,
            look,
            rect.right_top() + vec2(-2.0, 2.0),
            Align2::RIGHT_TOP,
            format!("util {now}%"),
            look.load.at(now),
        );
    }
    fixed
}

fn pct_of(value: u64, capacity: u64) -> Option<i64> {
    value
        .saturating_mul(100)
        .checked_div(capacity)
        .map(|pct| pct.min(100) as i64)
}

/// Used / Avail / Cache / Commit / Swap meters, then the used-memory history.
/// Returns the height of the meters; the history graph below stretches.
fn mem_panel(ui: &mut Ui, look: &Look, mem: &MemInfo) -> f32 {
    let top = ui.cursor().top();
    let s = &mem.stats;
    let total = s.used + s.available;
    meter_row(
        ui,
        look,
        "Used",
        pct_of(s.used, total),
        &look.bytes(s.used),
        &look.fill,
    );
    meter_row(
        ui,
        look,
        "Avail",
        pct_of(s.available, total),
        &look.bytes(s.available),
        &look.mem_avail,
    );
    if s.cached > 0 {
        meter_row(
            ui,
            look,
            "Cache",
            pct_of(s.cached, total),
            &look.bytes(s.cached),
            &look.mem_cache,
        );
    }
    let commit = if s.commit_limit > 0 {
        look.pair(s.commit_used, s.commit_limit)
    } else {
        "N/A".to_owned()
    };
    let commit_pct = pct_of(s.commit_used, s.commit_limit);
    meter_row(ui, look, "Commit", commit_pct, &commit, &look.fill);

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
        let text = look.pair(value, capacity);
        meter_row(ui, look, "Swap", pct_of(value, capacity), &text, &look.fill);
    }

    ui.add_space(6.0);
    let rest = ui.available_rect_before_wrap();
    let fixed = rest.top() - top;
    if rest.height() >= ROW + 3.0 * DOT && rest.width() > 4.0 * DOT {
        let (rect, _) = ui.allocate_exact_size(rest.size(), Sense::hover());
        let painter = ui.painter_at(rect);
        dot_graph(&painter, rect, &mem.percent.used, Grow::Up, &look.fill);
        let now = last(&mem.percent.used);
        caption(
            &painter,
            look,
            rect.right_top() + vec2(-2.0, 2.0),
            Align2::RIGHT_TOP,
            format!("used {now}%"),
            look.fill.at(now),
        );
    }
    fixed
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
        let usage = look.pair(disk.used, disk.total);
        let used = i64::from(disk.used_percent.clamp(0, 100));
        meter_row(ui, look, &label, Some(used), &usage, &look.fill);

        let line = row(ui);
        let painter = ui.painter();
        let cy = line.center().y;
        let busy = i64::from(disk.busy_percent.clamp(0, 100));
        painter.text(
            line.right_center(),
            Align2::RIGHT_CENTER,
            format!("{busy}%"),
            mono(),
            look.load.at(busy),
        );
        painter.text(
            pos2(line.right() - 6.0 * cw, cy),
            Align2::LEFT_CENTER,
            "B",
            mono(),
            look.fg,
        );
        let speed_w = 11.0 * cw;
        let left_end = line.right() - 7.0 * cw;
        let graph_w = ((left_end - line.left() - 2.0 * (cw + speed_w) - 2.0 * cw) / 2.0).max(0.0);
        let mut x = line.left();
        let streams = [
            (
                "R",
                &disk.read_history,
                disk.read_bytes_per_sec,
                &look.inbound,
            ),
            (
                "W",
                &disk.write_history,
                disk.write_bytes_per_sec,
                &look.outbound,
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
                pos2(x, line.top() + 1.0),
                pos2(x + graph_w, line.bottom() - 1.0),
            );
            if graph.width() > DOT {
                dot_graph_scaled(&ui.painter_at(graph), graph, history, Grow::Up, grad, max);
            }
            x += graph_w + cw;
        }
        ui.add_space(4.0);
    }
}

fn link_speed(bps: u64) -> String {
    match bps {
        1_000_000_000.. => format!("{}Gbit/s", bps / 1_000_000_000),
        1_000_000.. => format!("{}Mbit/s", bps / 1_000_000),
        1_000.. => format!("{}kbit/s", bps / 1_000),
        _ => format!("{bps}bit/s"),
    }
}

/// A clickable `name*` toggle, starred while on.
fn toggle(ui: &mut Ui, look: &Look, name: &str, on: bool) -> bool {
    let label = format!("{name}{}", if on { "*" } else { "" });
    let color = if on { look.fg } else { look.label };
    ui.add(
        egui::Label::new(text(label, color))
            .selectable(false)
            .sense(Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .clicked()
}

/// The selected adapter, or the first one if the selection is gone.
fn net_index(nets: &[NetInfo], view: &NetView) -> usize {
    nets.iter()
        .position(|n| n.stable_id == view.selected)
        .unwrap_or(0)
}

fn adapter_name(net: &NetInfo) -> String {
    match net.link_speed {
        0 => net.description.clone(),
        bps => format!("{} ({})", net.description, link_speed(bps)),
    }
}

/// Download grows up from the centre line and upload grows down, as in the
/// terminal. Below: the ping history and the sync/auto toggles with totals.
fn net_panel(
    ui: &mut Ui,
    look: &Look,
    nets: &[NetInfo],
    status: Option<&StatusbarInfo>,
    view: &mut NetView,
) -> f32 {
    let top = ui.cursor().top();
    if nets.is_empty() {
        ui.label(text("No network adapters", look.label));
        return ui.min_rect().bottom() - top;
    }
    // The adapter name and its ◀ ▶ selector are in the tab bar (`tab_header`).
    let index = net_index(nets, view);
    view.selected.clone_from(&nets[index].stable_id);
    let net = &nets[index];

    let full = ui.available_rect_before_wrap();
    let footer = ROW * 2.0 + 4.0;
    let graph = Rect::from_min_max(
        full.min,
        pos2(full.right(), (full.bottom() - footer).max(full.top())),
    );
    ui.allocate_rect(graph, Sense::hover());
    if graph.height() >= 4.0 * DOT {
        let (down, up) = (&net.bandwidth.download, &net.bandwidth.upload);
        let scale = |series: &VecDeque<i64>, configured: i64| {
            if view.auto {
                visible_max(series, graph.width(), 0)
            } else {
                configured.saturating_mul(1024).max(1)
            }
        };
        let (mut down_max, mut up_max) =
            (scale(down, look.net_download), scale(up, look.net_upload));
        if view.sync {
            down_max = down_max.max(up_max);
            up_max = down_max;
        }
        let painter = ui.painter_at(graph);
        let mid = graph.center().y;
        let upper = Rect::from_min_max(graph.min, pos2(graph.right(), mid - 1.0));
        let lower = Rect::from_min_max(pos2(graph.left(), mid + 1.0), graph.max);
        dot_graph_scaled(&painter, upper, down, Grow::Up, &look.inbound, down_max);
        dot_graph_scaled(&painter, lower, up, Grow::Down, &look.outbound, up_max);
        let stat_pct = |speed: u64, top: u64| pct_of(speed, top).unwrap_or(0);
        let (d, u) = (&net.stat.download, &net.stat.upload);
        caption(
            &painter,
            look,
            graph.right_top() + vec2(-2.0, 2.0),
            Align2::RIGHT_TOP,
            format!("{} ▼", look.rate(d.speed)),
            look.inbound.at(stat_pct(d.speed, d.top)),
        );
        caption(
            &painter,
            look,
            graph.right_bottom() + vec2(-2.0, -2.0),
            Align2::RIGHT_BOTTOM,
            format!("{} ▲", look.rate(u.speed)),
            look.outbound.at(stat_pct(u.speed, u.top)),
        );
    }

    ui.add_space(4.0);
    if let Some(status) = status {
        ping_row(ui, look, status);
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if toggle(ui, look, "sync", view.sync) {
            view.sync = !view.sync;
        }
        if toggle(ui, look, "auto", view.auto) {
            view.auto = !view.auto;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let totals = format!(
                "↓ {} ↑ {}",
                look.bytes(net.stat.download.total),
                look.bytes(net.stat.upload.total)
            );
            ui.label(text(totals, look.fg));
        });
    });
    // Everything but the stretching graph.
    ui.min_rect().bottom() - top - graph.height()
}

/// `Ping ⣀⣀⣠⣀ 4ms`: latency history scaled to its visible maximum (at least
/// 10 ms). Failed probes are marked with a cross rather than drawn as zero.
fn ping_row(ui: &mut Ui, look: &Look, status: &StatusbarInfo) {
    let line = row(ui);
    let cw = char_width(ui);
    let cy = line.center().y;
    let painter = ui.painter();
    painter.text(
        line.left_center(),
        Align2::LEFT_CENTER,
        "Ping",
        mono(),
        look.fg,
    );
    let value = status.ping.strip_prefix("ping ").unwrap_or("...");
    // Like the meters: the graph ends one column before the widest value seen.
    let value_chars = meter_value_chars(ui, value) as f32;
    // Latency is a load: 0 ms takes the start colour, 100 ms and above the end.
    let value_color = status
        .ping_history
        .back()
        .copied()
        .flatten()
        .map_or(look.fg, |ms| look.load.at(ms));
    let painter = ui.painter();
    painter.text(
        line.right_center(),
        Align2::RIGHT_CENTER,
        value,
        mono(),
        value_color,
    );
    let graph = Rect::from_min_max(
        pos2(line.left() + 5.0 * cw, line.top() + 1.0),
        pos2(line.right() - (value_chars + 1.0) * cw, line.bottom() - 1.0),
    );
    let cols = graph_columns(graph.width());
    if cols == 0 {
        return;
    }
    let history = &status.ping_history;
    let skip = history.len().saturating_sub(cols);
    let max = history
        .iter()
        .skip(skip)
        .flatten()
        .copied()
        .max()
        .unwrap_or(10)
        .max(10);
    let values: VecDeque<i64> = history.iter().map(|v| v.unwrap_or(0)).collect();
    dot_graph_scaled(
        &ui.painter_at(graph),
        graph,
        &values,
        Grow::Up,
        &look.load,
        max,
    );
    let pad = cols - (history.len() - skip);
    for (i, sample) in history.iter().skip(skip).enumerate() {
        if sample.is_none() {
            let x = graph.right() - (cols - pad - i) as f32 * DOT + DOT / 2.0;
            painter.text(pos2(x, cy), Align2::CENTER_CENTER, "×", mono(), look.hi);
        }
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
/// built. Returns the row the user clicked, if any. `id` must differ between
/// tables: rows and scroll state are keyed by it.
fn table(
    ui: &mut Ui,
    id: egui::Id,
    look: &Look,
    columns: &[Column],
    rows: usize,
    cell: impl Fn(usize) -> (String, Vec<Cell>),
) -> Option<usize> {
    // The column headings are on the caption row (`ranking_header`).
    let cw = char_width(ui);
    let right = ui.max_rect().right();
    let edges = column_edges(right, columns, cw);
    let name_right = match (edges.first(), columns.first()) {
        (Some(edge), Some(first)) => edge - first.chars * cw - cw,
        _ => right,
    };
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink(false)
        .show_rows(ui, ROW, rows, |ui, range| {
            for index in range {
                let line = row(ui);
                let response = ui
                    .interact(line, id.with(index), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.hovered() {
                    ui.painter().rect_filled(
                        line.expand2(vec2(4.0, 0.0)),
                        CornerRadius::same(2),
                        mix(look.bg, look.fg, 0.08),
                    );
                }
                if response.clicked() {
                    clicked = Some(index);
                }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// Show only processes whose name contains this (any case), or whose
    /// PID is this. Set from the title bar.
    filter: String,
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
    let needle = sorts.filter.trim().to_lowercase();
    let mut list: Vec<&ProcInfo> = procs
        .iter()
        .filter(|p| p.pid != 0)
        .filter(|p| {
            needle.is_empty()
                || p.name.to_lowercase().contains(&needle)
                || p.pid.to_string() == needle
        })
        .collect();
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
/// Returns 0: rankings show as many rows as fit, so they take part in no fitting.
fn ranking_panel(
    ui: &mut Ui,
    look: &Look,
    sample: &Sample,
    ranking: Ranking,
    sorts: &mut Sorts,
    open: &mut Option<u32>,
) -> f32 {
    let list = ranked(&sample.procs, ranking, sorts);
    if list.is_empty() && !sorts.filter.trim().is_empty() {
        ui.label(text(
            format!("No process matches \"{}\"", sorts.filter.trim()),
            look.label,
        ));
        return 0.0;
    }
    let total_mem = sample.mem.stats.used + sample.mem.stats.available;
    let total_vram = sample.gpus.iter().map(|g| g.mem_total).max().unwrap_or(0);
    let cores = sample.cpu.core_count;
    // Coloured columns follow the colour roles: usage is load, memory is fill.
    let load = |pct: Option<i64>| pct.map_or(look.dim, |pct| look.load.at(pct));
    let fill = |pct: Option<i64>| pct.map_or(look.dim, |pct| look.fill.at(pct));
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
    let columns = ranking_columns(ranking, sorts);
    // Keyed by ranking and layer, so the Settings preview's table does not
    // share rows with the panel.
    let id = egui::Id::new(("process-table", ranking, ui.layer_id()));
    let clicked = table(ui, id, look, &columns, list.len(), |i| {
        let p = list[i];
        let values = match ranking {
            Ranking::Cpu => {
                let pct = cpu(p);
                vec![
                    (format!("{pct:.1}"), look.load.at(pct as i64)),
                    (look.bytes(p.ram), look.dim),
                    (look.bytes(p.mem), look.dim),
                ]
            }
            Ranking::Mem => vec![
                (format!("{:.1}", cpu(p)), look.dim),
                (look.bytes(p.ram), fill(pct_of(p.ram, total_mem))),
                (look.bytes(p.mem), fill(pct_of(p.mem, total_mem))),
            ],
            Ranking::Gpu => vec![
                (gpu(p), load(p.gpu_p.map(|v| v as i64))),
                (vram(p), look.dim),
            ],
            Ranking::Vram => vec![
                (gpu(p), look.dim),
                (vram(p), fill(p.gpu_mem.and_then(|v| pct_of(v, total_vram)))),
            ],
            Ranking::Io => [IoSort::Read, IoSort::Write, IoSort::Total]
                .into_iter()
                .map(|kind| match p.io_rates.map(|r| kind.value(r)) {
                    Some(v) => (
                        look.rate(v),
                        match kind {
                            IoSort::Read => &look.inbound,
                            IoSort::Write => &look.outbound,
                            IoSort::Total => &look.load,
                        }
                        .at((v.saturating_mul(100) / IO_FULL_SCALE).min(100) as i64),
                    ),
                    None => ("N/A".to_owned(), look.label),
                })
                .collect(),
        };
        (p.name.clone(), values)
    });
    if let Some(i) = clicked {
        *open = Some(list[i].pid);
    }
    0.0
}

/// Value columns of a ranking; the sorted one carries ▼. Memory's RAM /
/// Commit and all I/O columns are clickable to change the sort.
fn ranking_columns(ranking: Ranking, sorts: &Sorts) -> Vec<Column> {
    let arrow =
        |on: bool, with: &'static str, without: &'static str| if on { with } else { without };
    let mut columns = match ranking {
        Ranking::Cpu => vec![
            column("CPU%▼", 7.0),
            column("RAM", 11.0),
            column("Commit", 11.0),
        ],
        Ranking::Mem => vec![
            column("CPU%", 7.0),
            column(arrow(sorts.mem_ram, "RAM▼", "RAM"), 11.0),
            column(arrow(!sorts.mem_ram, "Commit▼", "Commit"), 11.0),
        ],
        Ranking::Gpu => vec![column("GPU%▼", 7.0), column("VRAM", 11.0)],
        Ranking::Vram => vec![column("GPU%", 7.0), column("VRAM▼", 11.0)],
        Ranking::Io => vec![
            column(arrow(sorts.io == IoSort::Read, "Read/s▼", "Read/s"), 12.0),
            column(
                arrow(sorts.io == IoSort::Write, "Write/s▼", "Write/s"),
                12.0,
            ),
            column(arrow(sorts.io == IoSort::Total, "Total▼", "Total"), 12.0),
        ],
    };
    match ranking {
        Ranking::Mem => columns[1..].iter_mut().for_each(|c| c.clickable = true),
        Ranking::Io => columns.iter_mut().for_each(|c| c.clickable = true),
        _ => {}
    }
    columns
}

/// A click on column `i` of a ranking's header.
fn sort_by_column(ranking: Ranking, sorts: &mut Sorts, i: usize) {
    match ranking {
        Ranking::Mem => sorts.mem_ram = i == 1,
        Ranking::Io => sorts.io = [IoSort::Read, IoSort::Write, IoSort::Total][i],
        _ => {}
    }
}

struct Panels<'a> {
    captions: Vec<(Panel, egui::Rect)>,
    sample: Option<Arc<Sample>>,
    status: Option<Arc<StatusbarInfo>>,
    gpu: &'a mut usize,
    net: &'a mut NetView,
    sorts: &'a mut Sorts,
    look: &'a Look,
    fits: &'a mut [Fit; 10],
    dragged: &'a mut Option<Panel>,
    /// A process row was clicked: open its details.
    open_proc: &'a mut Option<u32>,
}

impl Panels<'_> {
    /// A ranking's column headings on its caption row, aligned with the
    /// value columns below (the body is inset by the same 8 px margin). The
    /// caption stands where the terminal shows "Program". Sortable headings
    /// take clicks, read from the pointer since egui_dock owns tab presses.
    fn ranking_header(
        &mut self,
        ranking: Ranking,
        response: &egui::Response,
        painter: &egui::Painter,
    ) {
        let look = self.look;
        let columns = ranking_columns(ranking, self.sorts);
        let cw = painter
            .layout_no_wrap("0".to_owned(), mono(), look.fg)
            .size()
            .x;
        let tab = response.rect;
        let edges = column_edges(tab.right() - 8.0, &columns, cw);
        let cy = tab.center().y;
        let ctx = &response.ctx;
        let (click, hover) = ctx.input(|i| {
            (
                i.pointer
                    .primary_clicked()
                    .then(|| i.pointer.interact_pos())
                    .flatten(),
                i.pointer.hover_pos(),
            )
        });
        for (i, (column, &edge)) in columns.iter().zip(&edges).enumerate() {
            painter.text(
                pos2(edge, cy),
                Align2::RIGHT_CENTER,
                column.header,
                mono(),
                look.fg,
            );
            if column.clickable {
                let rect = Rect::from_min_max(
                    pos2(edge - column.chars * cw, tab.top()),
                    pos2(edge, tab.bottom()),
                );
                if hover.is_some_and(|p| rect.contains(p)) {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if click.is_some_and(|p| rect.contains(p)) {
                    sort_by_column(ranking, self.sorts, i);
                    ctx.request_repaint();
                }
            }
        }
    }

    /// Right-aligned panel info on the caption row: the CPU and GPU names,
    /// total memory and the network adapter. GPU and network get ◀ ▶
    /// selectors when there are several; a click on either arrow cycles. The
    /// header is skipped when the tab is too narrow to show it beside the
    /// caption.
    fn tab_header(
        &mut self,
        panel: Panel,
        response: &egui::Response,
        painter: &egui::Painter,
        caption_right: f32,
    ) {
        if let Some(ranking) = panel.ranking() {
            self.ranking_header(ranking, response, painter);
            return;
        }
        let Some(sample) = self.sample.clone() else {
            return;
        };
        let (name, count) = match panel {
            Panel::Cpu => (sample.cpu.cpu_name.clone(), 1),
            Panel::Gpu if !sample.gpus.is_empty() => {
                let gpus = &sample.gpus;
                (gpus[*self.gpu % gpus.len()].name.clone(), gpus.len())
            }
            Panel::Mem => {
                let s = &sample.mem.stats;
                (self.look.bytes(s.used + s.available), 1)
            }
            Panel::Net if !sample.nets.is_empty() => {
                let nets = &sample.nets;
                (adapter_name(&nets[net_index(nets, self.net)]), nets.len())
            }
            _ => return,
        };
        let look = self.look;
        let arrows = count > 1;
        let width = |s: &str| {
            painter
                .layout_no_wrap(s.to_owned(), mono(), look.fg)
                .size()
                .x
        };
        let gap = 6.0;
        let arrow_w = if arrows { width("◀") + gap } else { 0.0 };
        let right = response.rect.right() - 8.0;
        let total = width(&name) + 2.0 * arrow_w;
        if right - total < caption_right + 2.0 * gap {
            return;
        }
        let cy = response.rect.center().y;
        let mut x = right;
        let mut step = None;
        // egui_dock turns a press on a tab into its drag state, so the tab's
        // own `clicked()` never fires; read the click from the pointer.
        let click = response.ctx.input(|i| {
            i.pointer
                .primary_clicked()
                .then(|| i.pointer.interact_pos())
                .flatten()
        });
        let mut arrow = |x: f32, glyph: &str, delta: usize| {
            let rect = painter
                .text(pos2(x, cy), Align2::RIGHT_CENTER, glyph, mono(), look.label)
                .expand(3.0);
            if click.is_some_and(|p| rect.contains(p)) {
                step = Some(delta);
            }
            // Override the tab's grab cursor: this spot is a button, not a handle.
            let ctx = &response.ctx;
            if ctx.pointer_hover_pos().is_some_and(|p| rect.contains(p)) {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        };
        if arrows {
            arrow(x, "▶", 1);
            x -= arrow_w;
        }
        painter.text(pos2(x, cy), Align2::RIGHT_CENTER, &name, mono(), look.fg);
        x -= width(&name) + gap;
        if arrows {
            arrow(x, "◀", count - 1);
        }
        if let Some(delta) = step {
            match panel {
                Panel::Gpu => *self.gpu = (*self.gpu % count + delta) % count,
                Panel::Net => {
                    let next = (net_index(&sample.nets, self.net) + delta) % count;
                    self.net.selected.clone_from(&sample.nets[next].stable_id);
                }
                _ => {}
            }
            response.ctx.request_repaint();
        }
    }
}

impl TabViewer for Panels<'_> {
    type Tab = Panel;
    /// Blank, but as wide as the caption: egui_dock always centres tab text,
    /// so `on_tab_button` paints the real caption left-aligned instead.
    /// While a tab is dragged egui_dock draws it on its own floating layer, so
    /// it gets its real (centred) caption for the move.
    fn title(&mut self, panel: &mut Panel) -> egui::WidgetText {
        let caption = if *self.dragged == Some(*panel) {
            panel.caption().to_owned()
        } else {
            " ".repeat(panel.caption().chars().count())
        };
        text(caption, self.look.fg).strong().into()
    }
    fn on_tab_button(&mut self, tab: &mut Panel, response: &egui::Response) {
        self.captions.push((*tab, response.rect));
        // egui_dock swaps in a separate response for a dragged tab, so detect
        // the drag from where the press began instead.
        let dragging = response.ctx.input(|i| {
            i.pointer.is_decidedly_dragging()
                && i.pointer
                    .press_origin()
                    .is_some_and(|p| response.rect.contains(p))
        });
        if dragging {
            *self.dragged = Some(*tab);
        }
        if *self.dragged == Some(*tab) {
            return;
        }
        let painter = response
            .ctx
            .layer_painter(response.layer_id)
            .with_clip_rect(response.rect);
        // egui bundles only a regular monospace face, so the caption is
        // emboldened by drawing it twice, one physical pixel apart.
        let at = response.rect.left_center() + vec2(8.0, 0.0);
        let px = response.ctx.pixels_per_point().recip();
        painter.text(at, Align2::LEFT_CENTER, tab.caption(), mono(), self.look.fg);
        let caption = painter.text(
            at + vec2(px, 0.0),
            Align2::LEFT_CENTER,
            tab.caption(),
            mono(),
            self.look.fg,
        );
        self.tab_header(*tab, response, &painter, caption.right());
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
        let body = ui.max_rect();
        let content = match panel {
            Panel::Cpu => cpu_panel(ui, self.look, &sample.cpu),
            Panel::Gpu => gpu_panel(ui, self.look, &sample.gpus, *self.gpu),
            Panel::Mem => mem_panel(ui, self.look, &sample.mem),
            Panel::Disk => {
                disk_panel(ui, self.look, &sample.disks);
                ui.cursor().top() - body.top()
            }
            Panel::Net => net_panel(
                ui,
                self.look,
                &sample.nets,
                self.status.as_deref(),
                self.net,
            ),
            Panel::Io => ranking_panel(
                ui,
                self.look,
                &sample,
                Ranking::Io,
                self.sorts,
                self.open_proc,
            ),
            Panel::CpuProcs => ranking_panel(
                ui,
                self.look,
                &sample,
                Ranking::Cpu,
                self.sorts,
                self.open_proc,
            ),
            Panel::MemProcs => ranking_panel(
                ui,
                self.look,
                &sample,
                Ranking::Mem,
                self.sorts,
                self.open_proc,
            ),
            Panel::GpuProcs => ranking_panel(
                ui,
                self.look,
                &sample,
                Ranking::Gpu,
                self.sorts,
                self.open_proc,
            ),
            Panel::VramProcs => ranking_panel(
                ui,
                self.look,
                &sample,
                Ranking::Vram,
                self.sorts,
                self.open_proc,
            ),
        };
        if content > 0.0 {
            self.fits[panel.index()] = Fit {
                content,
                body: body.height(),
                chrome: self.fits[panel.index()].chrome,
            };
        }
    }
}

/// Load `rztop.toml` read-only and start the file logger. A GUI build has
/// no console, so config warnings and panics go to `rztop.log`.
fn config_path() -> std::path::PathBuf {
    crate::tools::config_dir().join("rztop.toml")
}

fn load_config() -> Config {
    let mut config = Config::new();
    let path = config_path();
    let warnings = if path.exists() {
        config.load(&path)
    } else {
        Vec::new()
    };
    let log_dir = crate::tools::data_dir();
    if crate::log::init(&log_dir, config.log.log_level).is_ok() {
        crate::log::startup_banner(
            config.log.log_level,
            &log_dir.join("rztop.log"),
            path.exists().then_some(path.as_path()),
        );
    }
    for warning in &warnings {
        tracing::warn!(subsystem = %crate::log::Subsystem::Config, %warning, "config load warning");
    }
    config
}

/// Window and taskbar icon: a few columns of graph dots on the theme
/// background, each dot coloured by its height with the gradient of the
/// Cores mini graphs, like a tiny CPU history.
fn app_icon(look: &Look) -> egui::IconData {
    const SIZE: usize = 64;
    egui::IconData {
        rgba: icon_rgba(look, SIZE),
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

/// The icon as `size`×`size` RGBA pixels. The design is laid out on a
/// 64-pixel grid and scaled, so every size shows the same picture.
fn icon_rgba(look: &Look, size: usize) -> Vec<u8> {
    const HEIGHTS: [usize; 6] = [2, 4, 3, 6, 4, 5];
    const ROWS: usize = 6;
    let s = size as f32 / 64.0;
    let (step, radius, corner) = (9.0 * s, 3.1 * s, 12.0 * s);
    // The faint border stays at least one pixel wide on small icons.
    let border = (1.5 * s).max(1.0);
    let margin = (size as f32 - step * HEIGHTS.len() as f32) / 2.0;
    // Coverage of a signed distance (negative inside), antialiased over a pixel.
    let cover = |d: f32| (0.5 - d).clamp(0.0, 1.0);
    let over = |dst: [f32; 4], src: Color32, a: f32| {
        let s = [src.r(), src.g(), src.b()].map(|c| f32::from(c) / 255.0);
        let out_a = a + dst[3] * (1.0 - a);
        let mut out = [0.0; 4];
        for i in 0..3 {
            out[i] = if out_a > 0.0 {
                (s[i] * a + dst[i] * dst[3] * (1.0 - a)) / out_a
            } else {
                0.0
            };
        }
        out[3] = out_a;
        out
    };
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded-square background with a faint border.
            let half = size as f32 / 2.0;
            let (qx, qy) = (
                (px - half).abs() - (half - corner),
                (py - half).abs() - (half - corner),
            );
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            let d = outside + qx.max(qy).min(0.0) - corner;
            let mut c = over([0.0; 4], look.dim, cover(d));
            c = over(c, look.bg, cover(d + border));
            for (col, &height) in HEIGHTS.iter().enumerate() {
                for row in 0..height {
                    let cx = margin + (col as f32 + 0.5) * step;
                    let cy = size as f32 - margin - (row as f32 + 0.5) * step;
                    let dist = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - radius;
                    if dist < 1.0 {
                        let color = look.load.at(((row + 1) * 100 / ROWS) as i64);
                        c = over(c, color, cover(dist));
                    }
                }
            }
            rgba.extend(c.map(|v| (v * 255.0).round() as u8));
        }
    }
    rgba
}

/// egui's own colours (panels, windows, widgets) from the theme.
fn apply_style(ctx: &egui::Context, look: &Look) {
    ctx.style_mut(|s| {
        s.visuals.panel_fill = look.bg;
        s.visuals.window_fill = look.bg;
        s.visuals.window_stroke.color = look.dim;
        s.visuals.extreme_bg_color = mix(look.bg, Color32::BLACK, 0.3);
        s.visuals.faint_bg_color = mix(look.bg, look.fg, 0.04);
        s.visuals.selection.bg_fill = look.selected;
        s.visuals.widgets.noninteractive.bg_stroke.color = look.dim;
        s.visuals.override_text_color = Some(look.fg);
        s.spacing.item_spacing = egui::vec2(4.0, 2.0);
    });
}

pub fn run_desktop() -> eframe::Result {
    let config = load_config();
    let look = Look::new(&config);
    let interval_ms = u64::try_from(config.refresh.update_ms)
        .unwrap_or(1000)
        .clamp(100, 86_400_000);
    let net = NetView {
        selected: config.view.net_iface.clone(),
        auto: config.view.net_auto,
        sync: config.view.net_sync,
    };
    let sorts = Sorts {
        mem_ram: config.proc.memory_sort_ram,
        io: config.proc.io_sort,
        filter: String::new(),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1150.0, 975.0])
            .with_min_inner_size([410.0, 320.0])
            .with_decorations(false)
            .with_icon(app_icon(&look)),
        ..Default::default()
    };
    eframe::run_native(
        "rztop",
        options,
        Box::new(move |cc| {
            apply_style(&cc.egui_ctx, &look);
            let last = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, LAYOUT));
            let saved = cc
                .storage
                .and_then(|storage| eframe::get_value::<SavedLayout>(storage, SAVED_LAYOUT));
            // A first run fits the default layout like Reset layout does.
            let fitting = if last.is_some() { 0 } else { FIT_FRAMES };
            let dock = last.unwrap_or_else(initial_layout);
            Ok(Box::new(Desktop {
                dock,
                saved,
                store_saved: false,
                monitor: Monitor::new(cc.egui_ctx.clone(), interval_ms),
                gpu: 0,
                net,
                sorts,
                fits: [Fit::default(); 10],
                fitting,
                dragged: None,
                look,
                config,
                config_path: config_path(),
                settings: settings::Settings::default(),
                config_dirty: false,
                config_saved: None,
                title_icon: None,
                details: process_detail::Details::default(),
                history_len: None,
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
    fn meter_cells_are_pixel_aligned_and_evenly_spaced() {
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let cells = segment_cells(70.3, 371.7, ppp);
            assert!(cells.len() > 10);
            let px = |v: f32| v * ppp;
            let widths: Vec<f32> = cells.iter().map(|(l, r)| px(r - l)).collect();
            let gaps: Vec<f32> = cells.windows(2).map(|w| px(w[1].0 - w[0].1)).collect();
            for v in widths.iter().chain(&gaps) {
                assert!(
                    (v - v.round()).abs() < 1e-3,
                    "ppp={ppp}: {v} not whole pixels"
                );
            }
            assert!(
                widths.iter().all(|w| (w - widths[0]).abs() < 1e-3),
                "ppp={ppp}"
            );
            assert!(gaps.iter().all(|g| (g - gaps[0]).abs() < 1e-3), "ppp={ppp}");
            let end = cells.last().unwrap().1;
            assert!(end <= 70.3 + 371.7 + 1.0 / ppp, "ppp={ppp}: overflows");
        }
    }

    #[test]
    fn fitted_layout_gives_fixed_content_its_height_and_graphs_the_rest() {
        let mut fits = [Fit::default(); 10];
        let mut set = |panel: Panel, content| {
            fits[panel.index()] = Fit {
                content,
                body: 0.0,
                chrome: 37.0,
            };
        };
        set(Panel::Cpu, 240.0);
        set(Panel::Gpu, 126.0);
        set(Panel::Mem, 126.0);
        set(Panel::Net, 66.0);
        set(Panel::Disk, 44.0);
        let f = fit_fractions(900.0, &fits).unwrap();
        let cpu = 900.0 * f.cpu;
        assert!((cpu - 280.0).abs() < 0.5, "{cpu}");
        let column = 900.0 - cpu;
        let gpu = column * f.gpu;
        let mem = (column - gpu) * f.mem;
        let net = (column - gpu - mem) * f.net;
        let disk = column - gpu - mem - net;
        // Disks gets exactly its content; the spare 620 - 522 = 98 is
        // shared 1:1:2 between the GPU, Memory and Network graphs.
        assert!((disk - 84.0).abs() < 0.5, "{disk}");
        assert!((gpu - (166.0 + 24.5)).abs() < 0.5, "{gpu}");
        assert!((net - (106.0 + 49.0)).abs() < 0.5, "{net}");
        // Nothing to fit until every left-column panel has been measured.
        fits[Panel::Disk.index()].content = 0.0;
        assert_eq!(fit_fractions(900.0, &fits), None);
    }

    #[test]
    fn icon_is_a_rounded_square_with_coloured_dots() {
        let look = Look::new(&Config::new());
        let icon = app_icon(&look);
        assert_eq!((icon.width, icon.height), (64, 64));
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        let alpha = |x: usize, y: usize| icon.rgba[(y * 64 + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "corners are transparent");
        assert_eq!(alpha(32, 32), 255, "body is opaque");
        if let Ok(path) = std::env::var("RZTOP_ICON_OUT") {
            std::fs::write(path, &icon.rgba).unwrap();
        }
    }

    #[test]
    fn sizes_and_rates_use_full_units() {
        const GIB: u64 = 1024 * 1024 * 1024;
        let look = Look::new(&Config::new());
        assert_eq!(look.bytes(16 * GIB), "16.0GB");
        assert_eq!(look.rate(1536), "1.50KB/s");
        assert_eq!(look.pair(28 * GIB, 37 * GIB), "28.0/37GB");
        assert_eq!(look.pair(154 * 1024 * 1024, 5 * GIB), "154MB/5GB");
        assert_eq!(look.pair(GIB / 2, 3 * GIB / 2), "512MB/1.5GB");
        assert_eq!(trim_zeros("465"), "465");
    }

    #[test]
    fn link_speeds_use_whole_units() {
        assert_eq!(link_speed(325_000_000), "325Mbit/s");
        assert_eq!(link_speed(2_500_000_000), "2Gbit/s");
        assert_eq!(link_speed(999), "999bit/s");
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
    fn filter_matches_name_in_any_case_or_exact_pid() {
        let named = |pid, name: &str| ProcInfo {
            pid,
            name: name.to_owned(),
            ..Default::default()
        };
        let procs = vec![
            named(4, "System"),
            named(120, "firefox.exe"),
            named(1200, "Code.exe"),
        ];
        let pids = |filter: &str| -> Vec<u32> {
            let sorts = Sorts {
                mem_ram: false,
                io: IoSort::Total,
                filter: filter.to_owned(),
            };
            let mut pids: Vec<u32> = ranked(&procs, Ranking::Cpu, &sorts)
                .iter()
                .map(|p| p.pid)
                .collect();
            pids.sort_unstable();
            pids
        };
        assert_eq!(pids(""), vec![4, 120, 1200]);
        assert_eq!(pids("FIRE"), vec![120]);
        assert_eq!(pids(" code "), vec![1200]);
        assert_eq!(pids("120"), vec![120]);
        assert!(pids("nothing").is_empty());
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
            filter: String::new(),
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
                            status: None,
                            gpu: &mut 0,
                            net: &mut NetView {
                                selected: String::new(),
                                auto: true,
                                sync: false,
                            },
                            sorts: &mut Sorts {
                                mem_ram: false,
                                io: IoSort::Total,
                                filter: String::new(),
                            },
                            look: &look,
                            fits: &mut [Fit::default(); 10],
                            dragged: &mut None,
                            open_proc: &mut None,
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

    /// A Windows `.ico` file holding each `(size, rgba)` image as a
    /// 32-bit bitmap with alpha.
    fn ico_bytes(images: &[(usize, Vec<u8>)]) -> Vec<u8> {
        let mut head = Vec::new();
        let mut body = Vec::new();
        head.extend(0u16.to_le_bytes());
        head.extend(1u16.to_le_bytes()); // icon
        head.extend((images.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * images.len();
        for (size, rgba) in images {
            let mask_row = size.div_ceil(32) * 4;
            let mut dib = Vec::new();
            for v in [40, *size as u32, 2 * *size as u32] {
                dib.extend(v.to_le_bytes());
            }
            dib.extend(1u16.to_le_bytes()); // planes
            dib.extend(32u16.to_le_bytes()); // bits per pixel
            dib.extend([0u8; 24]); // no compression; sizes and palette unused
            // Rows bottom-up, BGRA; then an all-clear AND mask (alpha decides).
            for row in rgba.chunks(size * 4).rev() {
                for px in row.chunks(4) {
                    dib.extend([px[2], px[1], px[0], px[3]]);
                }
            }
            dib.extend(vec![0u8; mask_row * size]);
            let dim = if *size >= 256 { 0 } else { *size as u8 };
            head.extend([dim, dim, 0, 0]);
            head.extend(1u16.to_le_bytes());
            head.extend(32u16.to_le_bytes());
            head.extend((dib.len() as u32).to_le_bytes());
            head.extend((offset as u32).to_le_bytes());
            offset += dib.len();
            body.extend(dib);
        }
        head.extend(body);
        head
    }

    #[test]
    fn icon_renders_every_size_and_ico_indexes_them() {
        let look = Look::new(&Config::new());
        let images: Vec<(usize, Vec<u8>)> = [16, 32, 256]
            .into_iter()
            .map(|s| (s, icon_rgba(&look, s)))
            .collect();
        for (size, rgba) in &images {
            assert_eq!(rgba.len(), size * size * 4);
        }
        let ico = ico_bytes(&images);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 3, 0]);
        // Third entry: 256 is written as 0; its data ends the file.
        let entry = &ico[6 + 32..6 + 48];
        assert_eq!(entry[0], 0);
        let len = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let at = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        assert_eq!(at + len, ico.len());
    }

    /// Regenerates `assets/rztop.ico`, the exe icon, from the default
    /// theme: `cargo test -- --ignored write_exe_icon`.
    #[test]
    #[ignore = "writes assets/rztop.ico"]
    fn write_exe_icon() {
        let look = Look::new(&Config::new());
        let images: Vec<(usize, Vec<u8>)> = [16, 20, 24, 32, 40, 48, 64, 128, 256]
            .into_iter()
            .map(|s| (s, icon_rgba(&look, s)))
            .collect();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/rztop.ico");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, ico_bytes(&images)).unwrap();
    }
}
