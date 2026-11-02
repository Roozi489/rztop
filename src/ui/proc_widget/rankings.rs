// Modified for rztop on 2026-11-02: monitoring, layout and interaction enhancements.
//! Independent CPU, commit size, GPU usage and VRAM rankings.
use super::*;
use crate::collect::process_display::{ProcSort, build_proc_display_entries};
use crate::tools;

pub(super) fn draw(
    procs: &[ProcInfo],
    area: &WidgetArea,
    theme: &Theme,
    settings: &ProcFrame,
    view: &ProcView,
    status: &CollectStatus,
) -> String {
    let mut buf = AnsiBuffer::new();
    let heights =
        crate::domain::resizing::allocate(area.height, &settings.ranking_weights, &[5; 5]);
    for (index, title) in [
        "CPU processes",
        "Memory processes",
        "I/O processes (R+W▼)",
        "GPU processes",
        "VRAM processes",
    ]
    .into_iter()
    .enumerate()
    {
        let y = area.y + heights[..index].iter().sum::<usize>();
        let height = heights[index];
        let sort = if index == 0 {
            ProcSort::Cpu
        } else {
            ProcSort::Memory
        };
        let gpu = index >= 3;
        let usage_primary = index == 0 || index == 3;
        let title = if view.paused {
            format!("{title} [paused]")
        } else {
            title.into()
        };
        buf.text(&box_drawing::create_box(&box_drawing::BoxConfig {
            x: area.x,
            y,
            width: area.width,
            height,
            line_color: theme.color(tc::PROC_WIDGET),
            fill: true,
            title: &title,
            title2: "",
            num: crate::ui::PROC_KEY,
            rounded: area.rounded,
            hi_color: theme.color(tc::HI_FG),
            title_color: theme.color(tc::TITLE),
        }));
        if height < 4 || area.width < if gpu { 24 } else { 28 } {
            continue;
        }
        super::super::draw_status_inset(
            &mut buf,
            status,
            &title,
            area.x,
            y,
            theme.color(tc::PROC_WIDGET),
            theme.color(tc::TITLE),
        );
        if index == 2 {
            draw_io(
                &mut buf,
                procs,
                &WidgetArea { y, height, ..*area },
                theme,
                settings,
                view,
            );
            draw_footer(&mut buf, area, y, height, theme, view);
            continue;
        }
        let show_ram = !gpu;
        let name_width = area.width.saturating_sub(if show_ram { 28 } else { 21 });
        let cpu_x = area.x + 3 + name_width + 1;
        let ram_x = cpu_x + 7;
        let mem_x = ram_x + if show_ram { 7 } else { 0 };
        let cpu_color = theme.color(if usage_primary {
            tc::HI_FG
        } else {
            tc::INACTIVE_FG
        });
        let mem_color = theme.color(if !usage_primary {
            tc::HI_FG
        } else {
            tc::INACTIVE_FG
        });
        buf.mv(area.x + 3, y + 2)
            .color(theme.color(tc::TITLE))
            .text(&tools::uresize("Program", name_width));
        buf.mv(cpu_x, y + 2)
            .color(theme.color(tc::TITLE))
            .text(if usage_primary {
                if gpu { " GPU%▼" } else { " CPU%▼" }
            } else {
                if gpu { "  GPU%" } else { "  CPU%" }
            });
        let mem_label = match (gpu, usage_primary) {
            (true, false) => "VRAM▼",
            (true, true) => "VRAM",
            (false, false) if !settings.memory_sort_ram => "Commit▼",
            (false, false) => "Commit",
            (false, true) => "Commit",
        };
        if show_ram {
            buf.mv(ram_x, y + 2)
                .color(theme.color(tc::TITLE))
                .text(&if index == 1 {
                    crate::draw::controls::button(
                        if settings.memory_sort_ram {
                            "  RAM▼"
                        } else {
                            "   RAM"
                        },
                        'M' as u32,
                    )
                } else {
                    "   RAM".into()
                });
        }
        buf.mv(mem_x, y + 2)
            .color(theme.color(tc::TITLE))
            .text(&if index == 1 {
                crate::draw::controls::button(&format!("{mem_label:>7}"), 'C' as u32)
            } else {
                format!("{mem_label:>7}")
            });
        let mut entries = build_proc_display_entries(procs, sort, true, view.filter, false, false);
        if index == 1 && settings.memory_sort_ram {
            entries.sort_by(|a, b| {
                let (a, b) = (&procs[a.proc_index], &procs[b.proc_index]);
                b.ram.cmp(&a.ram).then(a.pid.cmp(&b.pid))
            });
        }
        if gpu {
            entries.sort_by(|a, b| {
                let (a, b) = (&procs[a.proc_index], &procs[b.proc_index]);
                let order = if usage_primary {
                    b.gpu_p
                        .partial_cmp(&a.gpu_p)
                        .unwrap_or(std::cmp::Ordering::Equal)
                } else {
                    b.gpu_mem.cmp(&a.gpu_mem)
                };
                order.then(a.pid.cmp(&b.pid))
            });
        }
        for (row, entry) in entries.iter().take(height.saturating_sub(4)).enumerate() {
            let proc = &procs[entry.proc_index];
            let cpu = if gpu {
                proc.gpu_p
                    .map(|p| format!("{p:.1}"))
                    .unwrap_or_else(|| "N/A".into())
            } else {
                format!("{:.1}", rows::display_proc_cpu(proc.cpu_p, settings))
            };
            let mem = if gpu {
                proc.gpu_mem
                    .map(|m| tools::floating_humanizer(m, true, 0, false, false, settings.base_10))
                    .unwrap_or_else(|| "N/A".into())
            } else {
                tools::floating_humanizer(proc.mem, true, 0, false, false, settings.base_10)
            };
            let cpu_value = if gpu {
                proc.gpu_p
            } else {
                Some(rows::display_proc_cpu(proc.cpu_p, settings))
            };
            let value_cpu_color = if usage_primary && settings.proc_colors {
                cpu_value.map_or(theme.color(tc::INACTIVE_FG), |pct| {
                    crate::theme::gradient_color(
                        theme.gradient(tc::GRAD_CPU_UPPER),
                        pct.clamp(0.0, 100.0) as i32,
                    )
                })
            } else {
                cpu_color
            };
            let capacity = if gpu {
                settings.total_vram
            } else {
                settings.total_mem
            };
            let bytes = if gpu { proc.gpu_mem } else { Some(proc.mem) };
            let value_mem_color = if !usage_primary && settings.proc_colors {
                if capacity > 0 {
                    bytes.map_or(theme.color(tc::INACTIVE_FG), |value| {
                        crate::theme::gradient_color(
                            theme.gradient(tc::GRAD_CPU_UPPER),
                            (value.saturating_mul(100) / capacity).min(100) as i32,
                        )
                    })
                } else {
                    theme.color(tc::INACTIVE_FG)
                }
            } else {
                mem_color
            };
            if show_ram {
                let ram =
                    tools::floating_humanizer(proc.ram, true, 0, false, false, settings.base_10);
                let ram_color = if index == 1 {
                    if settings.proc_colors && settings.total_mem > 0 {
                        crate::theme::gradient_color(
                            theme.gradient(tc::GRAD_CPU_UPPER),
                            (proc.ram.saturating_mul(100) / settings.total_mem).min(100) as i32,
                        )
                    } else {
                        theme.color(tc::MAIN_FG)
                    }
                } else {
                    theme.color(tc::INACTIVE_FG)
                };
                buf.mv(ram_x, y + 3 + row)
                    .color(ram_color)
                    .text(&tools::rjust(&ram, 6, true));
            }
            let name = if view.dead_pids.contains(&proc.pid) {
                format!("✗ {}", proc.name)
            } else {
                proc.name.clone()
            };
            buf.mv(area.x + 3, y + 3 + row)
                .color(theme.color(tc::MAIN_FG))
                .text(&tools::ljust(&name, name_width, true));
            buf.mv(cpu_x, y + 3 + row)
                .color(value_cpu_color)
                .text(&tools::rjust(&cpu, 6, true));
            buf.mv(mem_x, y + 3 + row)
                .color(value_mem_color)
                .text(&tools::rjust(&mem, 7, true));
        }
        draw_footer(&mut buf, area, y, height, theme, view);
    }
    buf.finish()
}

fn draw_footer(
    buf: &mut AnsiBuffer,
    area: &WidgetArea,
    y: usize,
    height: usize,
    theme: &Theme,
    view: &ProcView,
) {
    let footer = if view.filtering {
        format!("filter: {}_", view.filter)
    } else if !view.filter.is_empty() {
        crate::draw::controls::button(&format!("filter: {}", view.filter), 'f' as u32)
    } else {
        format!(
            "{}  {}",
            crate::draw::controls::button("f filter", 'f' as u32),
            crate::draw::controls::button("Space pause", 32)
        )
    };
    buf.mv(area.x + 3, y + height)
        .color(theme.color(tc::TITLE))
        .text(&tools::uresize(&footer, area.width.saturating_sub(6)))
        .reset();
}

fn draw_io(
    buf: &mut AnsiBuffer,
    procs: &[ProcInfo],
    area: &WidgetArea,
    theme: &Theme,
    settings: &ProcFrame,
    view: &ProcView,
) {
    let mut entries =
        build_proc_display_entries(procs, ProcSort::Cpu, true, view.filter, false, false);
    entries.sort_by(|a, b| {
        let (a, b) = (&procs[a.proc_index], &procs[b.proc_index]);
        b.io_rates
            .map(|r| r.total())
            .cmp(&a.io_rates.map(|r| r.total()))
            .then(a.pid.cmp(&b.pid))
    });
    let maximum = entries
        .iter()
        .filter_map(|e| procs[e.proc_index].io_rates.map(|r| r.total()))
        .max()
        .unwrap_or(0);
    let name_width = area.width.saturating_sub(25);
    let read_x = area.x + 4 + name_width;
    let write_x = read_x + 10;
    buf.mv(area.x + 3, area.y + 2)
        .color(theme.color(tc::TITLE))
        .text(&tools::uresize("Program", name_width));
    buf.mv(read_x, area.y + 2).text("    Read/s");
    buf.mv(write_x, area.y + 2).text("   Write/s");
    for (row, entry) in entries
        .iter()
        .take(area.height.saturating_sub(4))
        .enumerate()
    {
        let proc = &procs[entry.proc_index];
        let color = proc.io_rates.map_or(theme.color(tc::INACTIVE_FG), |rates| {
            if settings.proc_colors {
                crate::theme::gradient_color(
                    theme.gradient(tc::GRAD_CPU_UPPER),
                    rates
                        .total()
                        .saturating_mul(100)
                        .checked_div(maximum)
                        .unwrap_or(0)
                        .min(100) as i32,
                )
            } else {
                theme.color(tc::MAIN_FG)
            }
        });
        buf.mv(area.x + 3, area.y + 3 + row)
            .color(theme.color(tc::MAIN_FG))
            .text(&tools::ljust(&proc.name, name_width, true));
        for (x, rate) in [
            (read_x, proc.io_rates.map(|r| r.read)),
            (write_x, proc.io_rates.map(|r| r.write)),
        ] {
            let text = rate.map_or_else(
                || "N/A".into(),
                |value| {
                    format!(
                        "{}/s",
                        tools::floating_humanizer(value, true, 0, false, false, settings.base_10)
                    )
                },
            );
            buf.mv(x, area.y + 3 + row)
                .color(color)
                .text(&tools::rjust(&text, 10, true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{make_frame, make_view, strip_ansi};
    use super::*;
    #[test]
    fn ram_selection_and_io_known_zero_precede_unknown() {
        let procs = vec![
            ProcInfo {
                pid: 3,
                name: "unknown".into(),
                mem: 1000,
                ram: 1,
                ..Default::default()
            },
            ProcInfo {
                pid: 2,
                name: "resident".into(),
                mem: 1,
                ram: 2000,
                io_rates: Some(crate::domain::process::IoRates { read: 0, write: 0 }),
                ..Default::default()
            },
            ProcInfo {
                pid: 1,
                name: "active".into(),
                mem: 2,
                ram: 2,
                io_rates: Some(crate::domain::process::IoRates {
                    read: 10,
                    write: 20,
                }),
                ..Default::default()
            },
        ];
        let mut frame = make_frame();
        frame.memory_sort_ram = true;
        let area = WidgetArea {
            x: 0,
            y: 0,
            width: 44,
            height: 40,
            rounded: false,
        };
        let out = strip_ansi(&draw(
            &procs,
            &area,
            &Theme::default(),
            &frame,
            &make_view(),
            &CollectStatus::Ok,
        ));
        let memory = out
            .split_once("Memory processes")
            .unwrap()
            .1
            .split_once("I/O processes")
            .unwrap()
            .0;
        assert!(memory.contains("RAM▼"));
        assert!(!memory.contains("Commit▼"));
        assert!(memory.find("resident").unwrap() < memory.find("unknown").unwrap());
        let io = out
            .split_once("I/O processes")
            .unwrap()
            .1
            .split_once("GPU processes")
            .unwrap()
            .0;
        assert!(io.find("active").unwrap() < io.find("resident").unwrap());
        assert!(io.find("resident").unwrap() < io.find("unknown").unwrap());
        assert!(io.contains("N/A"));
    }

    #[test]
    fn arrows_ram_and_primary_gradients() {
        let procs = vec![ProcInfo {
            name: "sample".into(),
            cpu_p: 25.0,
            mem: 512 * 1024 * 1024,
            ram: 128 * 1024 * 1024,
            gpu_p: Some(75.0),
            gpu_mem: Some(256 * 1024 * 1024),
            ..Default::default()
        }];
        let theme = Theme::default();
        let mut frame = make_frame();
        frame.proc_colors = true;
        frame.proc_per_core = false;
        frame.core_count = 1;
        let area = WidgetArea {
            x: 0,
            y: 0,
            width: 44,
            height: 32,
            rounded: true,
        };
        let output = draw(
            &procs,
            &area,
            &theme,
            &frame,
            &make_view(),
            &CollectStatus::Ok,
        );
        let output = crate::draw::controls::Screen::default().output(&output);
        let plain = strip_ansi(&output);
        for label in ["CPU%▼", "Commit▼", "GPU%▼", "VRAM▼"] {
            assert!(plain.contains(label), "{label}");
        }
        assert_eq!(plain.matches("RAM").count(), 5); // two RAM columns, two VRAM headers and VRAM title
        assert_eq!(plain.matches("128M").count(), 2);
        let (cpu, rest) = output.split_once("Memory processes").unwrap();
        let (commit, rest) = rest.split_once("GPU processes").unwrap();
        let (gpu, vram) = rest.split_once("VRAM processes").unwrap();
        assert!(cpu.contains(&format!("{}  25.0", theme.gradient(tc::GRAD_CPU_UPPER)[25])));
        assert!(commit.contains(&format!(
            "{}   512M",
            theme.gradient(tc::GRAD_CPU_UPPER)[50]
        )));
        assert!(gpu.contains(&format!("{}  75.0", theme.gradient(tc::GRAD_CPU_UPPER)[75])));
        assert!(vram.contains(&format!(
            "{}   256M",
            theme.gradient(tc::GRAD_CPU_UPPER)[25]
        )));
        assert!(cpu.contains(&format!("{}  128M", theme.color(tc::INACTIVE_FG))));
        assert!(commit.contains(&format!("{}  128M", theme.gradient(tc::GRAD_CPU_UPPER)[12])));
        let header = theme.color(tc::TITLE);
        assert!(cpu.contains(&format!("{header} CPU%▼")));
        assert!(commit.contains(&format!("{header}Commit▼")));
        assert!(commit.contains(&format!("{header}   RAM")));
    }

    #[test]
    fn independent_rankings_are_descending_and_compact() {
        let procs = vec![
            ProcInfo {
                name: "cpu-heavy".into(),
                cpu_p: 95.0,
                mem: 1024,
                gpu_p: Some(10.0),
                gpu_mem: Some(99999),
                ..Default::default()
            },
            ProcInfo {
                name: "mem-heavy".into(),
                cpu_p: 1.0,
                mem: 999999,
                gpu_p: Some(80.0),
                gpu_mem: Some(100),
                ..Default::default()
            },
        ];
        let area = WidgetArea {
            x: 0,
            y: 0,
            width: 44,
            height: 32,
            rounded: true,
        };
        let output = strip_ansi(&draw(
            &procs,
            &area,
            &Theme::default(),
            &make_frame(),
            &make_view(),
            &CollectStatus::Ok,
        ));
        let (cpu, mem) = output.split_once("Memory processes").unwrap();
        assert!(cpu.find("cpu-heavy").unwrap() < cpu.find("mem-heavy").unwrap());
        assert!(mem.find("mem-heavy").unwrap() < mem.find("cpu-heavy").unwrap());
        let (_, gpu) = output.split_once("GPU processes").unwrap();
        let (gpu, vram) = gpu.split_once("VRAM processes").unwrap();
        assert!(gpu.find("mem-heavy").unwrap() < gpu.find("cpu-heavy").unwrap());
        assert!(vram.find("cpu-heavy").unwrap() < vram.find("mem-heavy").unwrap());
        assert!(!output.contains("PID"));
        assert!(!output.contains("Command Line"));
    }

    #[test]
    fn filter_and_pause_apply_to_both_rankings() {
        let procs = vec![
            ProcInfo {
                name: "keep-me".into(),
                ..Default::default()
            },
            ProcInfo {
                name: "exclude-me".into(),
                ..Default::default()
            },
        ];
        let mut view = make_view();
        view.filter = "keep";
        view.paused = true;
        let area = WidgetArea {
            x: 0,
            y: 0,
            width: 44,
            height: 32,
            rounded: false,
        };
        let output = strip_ansi(&draw(
            &procs,
            &area,
            &Theme::default(),
            &make_frame(),
            &view,
            &CollectStatus::Ok,
        ));
        assert_eq!(output.matches("keep-me").count(), 5);
        assert_eq!(output.matches("[paused]").count(), 5);
        assert!(!output.contains("exclude-me"));
    }
}
