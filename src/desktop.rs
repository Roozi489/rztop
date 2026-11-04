use crate::{
    collect::{Collector, cpu::CpuCollector, gpu::GpuCollector, process::ProcCollector},
    domain::{cpu::CpuInfo, gpu::GpuInfo, process::IoRates},
    runner::LatestSlot,
};
use eframe::egui::{self, Color32, RichText, Ui};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, mpsc},
    thread::JoinHandle,
    time::Duration,
};

const TICK: Duration = Duration::from_secs(1);
const LAYOUT: &str = "dock-layout-v1";
const GREEN: Color32 = Color32::from_rgb(170, 206, 113);
const BLUE: Color32 = Color32::from_rgb(139, 178, 225);
const GOLD: Color32 = Color32::from_rgb(229, 186, 109);
const FG: Color32 = Color32::from_rgb(224, 219, 199);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Panel {
    Cpu,
    Gpu,
    Io,
}

fn initial_layout() -> DockState<Panel> {
    let mut dock = DockState::new(vec![Panel::Cpu]);
    let tree = dock.main_surface_mut();
    let [_, lower] = tree.split_below(NodeIndex::root(), 0.47, vec![Panel::Gpu]);
    tree.split_right(lower, 0.45, vec![Panel::Io]);
    dock
}

struct Sample {
    cpu: CpuInfo,
    gpus: Vec<GpuInfo>,
    io: Vec<(String, Option<IoRates>)>,
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
            let mut processes = ProcCollector::new();
            processes.set_core_count(crate::collect::cpu::get_core_count());
            loop {
                let start = std::time::Instant::now();
                cpu.collect();
                gpu.collect();
                processes.collect();
                let mut procs = processes.snapshot().procs;
                procs.sort_by_key(|p| (std::cmp::Reverse(p.io_rates.map(|r| r.total())), p.pid));
                let io = procs.into_iter().map(|p| (p.name, p.io_rates)).collect();
                slot.publish(Sample {
                    cpu: cpu.snapshot().info,
                    gpus: gpu.snapshot().devices,
                    io,
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
}

impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(text("rztop", GREEN));
                ui.label(text("Drag a caption to dock", FG));
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
    // A completed library drag can renumber the entire three-panel tree.
    let positions = |dock: &DockState<Panel>| {
        [Panel::Cpu, Panel::Gpu, Panel::Io].map(|panel| dock.find_tab(&panel))
    };
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
    RichText::new(value).monospace().size(14.0).color(color)
}

struct Panels<'a> {
    captions: Vec<(Panel, egui::Rect)>,
    sample: Option<Arc<Sample>>,
    gpu: &'a mut usize,
}

impl TabViewer for Panels<'_> {
    type Tab = Panel;
    fn title(&mut self, panel: &mut Panel) -> egui::WidgetText {
        text(
            match panel {
                Panel::Cpu => "┤ CPU ├",
                Panel::Gpu => "┤ GPU ├",
                Panel::Io => "┤ I/O processes ├",
            },
            FG,
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
    fn scroll_bars(&self, panel: &Panel) -> [bool; 2] {
        [false, *panel != Panel::Io]
    }
    fn ui(&mut self, ui: &mut Ui, panel: &mut Panel) {
        let Some(s) = &self.sample else {
            ui.label(text("Sampling…", BLUE));
            return;
        };
        match panel {
            Panel::Cpu => {
                ui.label(text(&s.cpu.cpu_name, FG).strong());
                meter(ui, "CPU", s.cpu.cpu_percent.total.back().copied());
                let n = (ui.available_width() / 8.5).clamp(1.0, 160.0) as usize;
                let history: String = s
                    .cpu
                    .cpu_percent
                    .total
                    .iter()
                    .skip(s.cpu.cpu_percent.total.len().saturating_sub(n))
                    .map(|v| {
                        ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']
                            [(v.clamp(&0, &100) * 7 / 100) as usize]
                    })
                    .collect();
                ui.label(text(history, BLUE));
                ui.label(text(
                    format!("Clock  {}    Cores  {}", s.cpu.cpu_hz, s.cpu.core_count),
                    GOLD,
                ));
                let cols = ((ui.available_width() / 166.0) as usize).clamp(1, 4);
                egui::Grid::new("cores")
                    .num_columns(cols)
                    .spacing([16.0, 2.0])
                    .show(ui, |ui| {
                        for (i, core) in s.cpu.core_percent.iter().enumerate() {
                            let pct = core.back().copied().unwrap_or(0).clamp(0, 100);
                            ui.label(text(
                                format!(
                                    "{i:02} {:·<7} {pct:3}%",
                                    "▪".repeat((pct * 7 / 100) as usize)
                                ),
                                GREEN,
                            ));
                            if (i + 1) % cols == 0 {
                                ui.end_row();
                            }
                        }
                    });
            }
            Panel::Gpu => {
                if s.gpus.is_empty() {
                    ui.label(text("No GPU readings available", BLUE));
                    return;
                }
                *self.gpu %= s.gpus.len();
                if s.gpus.len() > 1 {
                    ui.horizontal(|ui| {
                        if ui.small_button("◀").clicked() {
                            *self.gpu = (*self.gpu + s.gpus.len() - 1) % s.gpus.len();
                        }
                        if ui.small_button("▶").clicked() {
                            *self.gpu = (*self.gpu + 1) % s.gpus.len();
                        }
                    });
                }
                let g = &s.gpus[*self.gpu];
                ui.label(text(&g.name, FG).strong());
                meter(ui, "GPU", g.gpu_percent.utilization.back().copied());
                ui.label(text(format!("Clock  {} MHz", g.gpu_clock_speed), BLUE));
                if let Some(t) = g.temp.back().filter(|t| **t > 0) {
                    ui.label(text(format!("Temp   {t} °C"), GOLD));
                }
                if g.pwr_usage > 0 {
                    ui.label(text(
                        format!("Watts  {:.0} W", g.pwr_usage as f64 / 1000.0),
                        GOLD,
                    ));
                }
                if g.mem_total > 0 {
                    ui.label(text(
                        format!("VRAM   {} / {}", bytes(g.mem_used), bytes(g.mem_total)),
                        BLUE,
                    ));
                }
            }
            Panel::Io => {
                let width = ((ui.available_width() / 8.5) as usize)
                    .saturating_sub(24)
                    .clamp(1, 40);
                ui.label(text(
                    format!(
                        "{:<width$}{:>8}{:>8}{:>8}",
                        "Program", "Read/s", "Write/s", "Total▼"
                    ),
                    GOLD,
                ));
                egui::ScrollArea::vertical().show_rows(ui, 16.0, s.io.len(), |ui, rows| {
                    for (name, rates) in &s.io[rows] {
                        let name = crate::tools::ljust(name, width, true);
                        let fmt = |v: Option<u64>| v.map(bytes).unwrap_or_else(|| "N/A".into());
                        ui.label(text(
                            format!(
                                "{name}{:>8}{:>8}{:>8}",
                                fmt(rates.map(|r| r.read)),
                                fmt(rates.map(|r| r.write)),
                                fmt(rates.map(|r| r.total()))
                            ),
                            GREEN,
                        ));
                    }
                });
            }
        }
    }
}

fn bytes(value: u64) -> String {
    crate::tools::floating_humanizer(value, true, 0, false, false, false)
}

fn meter(ui: &mut Ui, name: &str, value: Option<i64>) {
    let Some(value) = value else {
        ui.label(text(format!("{name}  N/A"), BLUE));
        return;
    };
    let cells = ((ui.available_width() / 8.5) as usize)
        .saturating_sub(12)
        .clamp(1, 72);
    let filled = value.clamp(0, 100) as usize * cells / 100;
    ui.label(text(
        format!(
            "{name:<6}{}{} {value:3}%",
            "▪".repeat(filled),
            "·".repeat(cells - filled)
        ),
        GREEN,
    ));
}

pub fn run_desktop() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([740.0, 620.0])
            .with_min_inner_size([410.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rztop desktop",
        options,
        Box::new(|cc| {
            cc.egui_ctx.style_mut(|s| {
                s.visuals.panel_fill = Color32::from_rgb(22, 22, 30);
                s.visuals.extreme_bg_color = Color32::from_rgb(18, 18, 25);
                s.visuals.selection.bg_fill = Color32::from_rgb(60, 78, 99);
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
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, Rect};

    #[test]
    fn dragging_io_caption_to_cpu_right_edge_creates_a_new_split() {
        let ctx = egui::Context::default();
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
        assert_eq!(dock.iter_all_tabs().count(), 3);
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
