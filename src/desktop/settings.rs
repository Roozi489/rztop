//! Settings window. Edits the shared [`Config`]; the caller applies what
//! changed and saves the editable keys to `rztop.toml`.

use super::theme_editor::{Action, ThemeEditor};
use super::theme_picker::ThemeGrid;
use super::{Look, adapter_name, text};
use crate::config::Config;
use crate::domain::{config_enums::TempScale, disk::DiskFilter, network::NetInfo};
use eframe::egui::{self, Align2, Ui, vec2};
use std::path::Path;
use tracing_subscriber::filter::LevelFilter;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Section {
    #[default]
    Appearance,
    Data,
    Network,
    Logging,
}

impl Section {
    const ALL: [Section; 4] = [
        Section::Appearance,
        Section::Data,
        Section::Network,
        Section::Logging,
    ];

    fn title(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Data => "Data & scales",
            Section::Network => "Network & disks",
            Section::Logging => "Logging",
        }
    }
}

/// What a frame of the Settings window changed, so the caller can apply it.
#[derive(Default)]
pub(super) struct Changes {
    /// Anything that needs saving.
    pub(super) any: bool,
    /// Colours or meter scales: rebuild the look.
    pub(super) look: bool,
    /// The theme itself: restyle the window and its icon too.
    pub(super) theme: bool,
    pub(super) interval: bool,
    pub(super) net: bool,
    pub(super) log: bool,
    /// The theme editor's preview started, changed or ended: rebuild the
    /// look from [`Settings::preview`] and restyle.
    pub(super) preview: bool,
}

/// Facts the window shows but does not edit.
pub(super) struct Context<'a> {
    pub(super) nets: &'a [NetInfo],
    pub(super) drives: Vec<String>,
    pub(super) config_path: &'a Path,
    pub(super) log_dir: &'a Path,
    /// Outcome of the last save: `None` before any, `Some(Err)` on failure.
    pub(super) saved: Option<&'a Result<(), String>>,
}

#[derive(Default)]
pub(super) struct Settings {
    open: bool,
    section: Section,
    themes: ThemeGrid,
    /// The disk filter as typed, kept while editing so invalid or partial
    /// entries are not wiped out mid-word.
    disk_filter: Option<String>,
    editor: Option<ThemeEditor>,
    /// The app is showing the editor's colours instead of the saved theme.
    previewing: bool,
}

impl Settings {
    /// The colours the app should show instead of the configured theme.
    pub(super) fn preview(&self) -> Option<&crate::themes::ThemePalette> {
        self.editor
            .as_ref()
            .filter(|_| self.previewing)
            .map(|e| &e.palette)
    }

    /// End the edit: the app returns to the configured theme, and the
    /// thumbnails pick up a saved or deleted user theme.
    fn close_editor(&mut self, ch: &mut Changes) {
        self.editor = None;
        self.themes.refresh();
        self.previewing = false;
        ch.preview = true;
        ch.look = true;
        ch.theme = true;
    }

    /// Preview the draft exactly while the editor is visible.
    fn sync_preview(&mut self, ch: &mut Changes) {
        let on = self.open && self.section == Section::Appearance && self.editor.is_some();
        if on != self.previewing {
            self.previewing = on;
            ch.preview = true;
        }
    }

    pub(super) fn toggle(&mut self) {
        self.open = !self.open;
        self.disk_filter = None;
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        config: &mut Config,
        look: &Look,
        info: &Context,
    ) -> Changes {
        let mut changes = Changes::default();
        if self.open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.open = false;
        }
        // A draft survives closing the window or switching section, but the
        // app only shows its colours while the editor is on screen.
        self.sync_preview(&mut changes);
        if !self.open {
            return changes;
        }
        let screen = ctx.screen_rect();
        let mut open = self.open;
        egui::Window::new(text("Settings", look.fg).strong())
            .id(egui::Id::new("settings"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .default_size(vec2(
                (screen.width() - 80.0).min(1040.0),
                (screen.height() - 120.0).min(640.0),
            ))
            .show(ctx, |ui| {
                egui::TopBottomPanel::bottom("settings-footer")
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 6)))
                    .show_inside(ui, |ui| footer(ui, look, info));
                egui::SidePanel::left("settings-nav")
                    .resizable(false)
                    .exact_width(150.0)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 4)))
                    .show_inside(ui, |ui| {
                        for section in Section::ALL {
                            let on = self.section == section;
                            let color = if on { look.fg } else { look.label };
                            if ui
                                .selectable_label(on, text(section.title(), color))
                                .clicked()
                            {
                                self.section = section;
                            }
                        }
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                        left: 14,
                        ..Default::default()
                    }))
                    .show_inside(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| match self.section {
                                Section::Appearance => {
                                    self.appearance(ui, config, look, &mut changes)
                                }
                                Section::Data => data(ui, config, look, &mut changes),
                                Section::Network => {
                                    self.network(ui, config, look, info, &mut changes)
                                }
                                Section::Logging => logging(ui, config, look, info, &mut changes),
                            });
                    });
            });
        self.open = open;
        self.sync_preview(&mut changes);
        changes.any |= changes.look || changes.interval || changes.net || changes.log;
        changes
    }

    fn appearance(&mut self, ui: &mut Ui, config: &mut Config, look: &Look, ch: &mut Changes) {
        if let Some(editor) = &mut self.editor {
            heading(ui, look, "Theme editor");
            match editor.show(ui, look) {
                Action::None => {}
                Action::Changed => ch.preview = true,
                Action::Save => match crate::theme::save_user(&editor.name, &editor.palette) {
                    Ok(()) => {
                        config.ui.color_theme = editor.name.clone();
                        self.close_editor(ch);
                    }
                    Err(e) => editor.error = Some(format!("Could not save: {e}")),
                },
                Action::Cancel => self.close_editor(ch),
                Action::Delete => match crate::theme::delete_user(&editor.name) {
                    Ok(()) => {
                        if config.ui.color_theme.eq_ignore_ascii_case(&editor.name) {
                            config.ui.color_theme = "default".to_owned();
                        }
                        self.close_editor(ch);
                    }
                    Err(e) => editor.error = Some(format!("Could not delete: {e}")),
                },
            }
            return;
        }

        ui.horizontal(|ui| {
            heading(ui, look, "Theme");
            ui.add_space(12.0);
            let current = &config.ui.color_theme;
            let label = if crate::theme::is_bundled(current) {
                format!("Customize {current}…")
            } else {
                format!("Edit {current}…")
            };
            if ui.button(label).clicked() {
                self.editor = Some(ThemeEditor::new(current));
            }
        });
        let current = config.ui.color_theme.clone();
        if let Some(name) = self.themes.show(ui, config, &current)
            && !name.eq_ignore_ascii_case(&current)
        {
            config.ui.color_theme = name;
            ch.look = true;
            ch.theme = true;
        }
        ui.add_space(14.0);
        heading(ui, look, "Units");
        grid(ui, "units", |ui| {
            label(ui, look, "Temperature");
            egui::ComboBox::from_id_salt("temp-scale")
                .selected_text(temp_name(config.cpu.temp_scale))
                .show_ui(ui, |ui| {
                    for &scale in TempScale::ALL {
                        ch.look |= ui
                            .selectable_value(&mut config.cpu.temp_scale, scale, temp_name(scale))
                            .changed();
                    }
                });
            ui.end_row();

            label(ui, look, "Sizes");
            ui.horizontal(|ui| {
                ch.look |= ui
                    .radio_value(
                        &mut config.ui.base_10_sizes,
                        false,
                        "1024-based, like Task Manager",
                    )
                    .changed();
                ch.look |= ui
                    .radio_value(&mut config.ui.base_10_sizes, true, "1000-based")
                    .changed();
            });
            ui.end_row();
        });
    }

    fn network(
        &mut self,
        ui: &mut Ui,
        config: &mut Config,
        look: &Look,
        info: &Context,
        ch: &mut Changes,
    ) {
        heading(ui, look, "Network");
        grid(ui, "network", |ui| {
            label(ui, look, "Adapter");
            let current = info
                .nets
                .iter()
                .find(|n| n.stable_id == config.view.net_iface);
            let selected = match current {
                Some(net) => adapter_name(net),
                None if config.view.net_iface == "auto" || config.view.net_iface.is_empty() => {
                    "First available".to_owned()
                }
                None => "Saved adapter (not present)".to_owned(),
            };
            egui::ComboBox::from_id_salt("net-adapter")
                .selected_text(selected)
                .width(320.0)
                .show_ui(ui, |ui| {
                    ch.net |= ui
                        .selectable_value(
                            &mut config.view.net_iface,
                            "auto".to_owned(),
                            "First available",
                        )
                        .changed();
                    for net in info.nets {
                        ch.net |= ui
                            .selectable_value(
                                &mut config.view.net_iface,
                                net.stable_id.clone(),
                                adapter_name(net),
                            )
                            .changed();
                    }
                });
            ui.end_row();

            label(ui, look, "Graph scale");
            ui.vertical(|ui| {
                ch.net |= ui
                    .checkbox(&mut config.view.net_auto, "Auto-scale to recent traffic")
                    .changed();
                ch.net |= ui
                    .checkbox(
                        &mut config.view.net_sync,
                        "Same scale for download and upload",
                    )
                    .changed();
            });
            ui.end_row();

            let fixed = !config.view.net_auto;
            label(ui, look, "Fixed download");
            ch.look |= ui
                .add_enabled(fixed, kib(&mut config.net.net_download))
                .changed();
            ui.end_row();
            label(ui, look, "Fixed upload");
            ch.look |= ui
                .add_enabled(fixed, kib(&mut config.net.net_upload))
                .changed();
            ui.end_row();
            hint_row(ui, look, "Used when auto-scale is off");
        });

        ui.add_space(14.0);
        heading(ui, look, "Disks");
        grid(ui, "disks", |ui| {
            label(ui, look, "Filter");
            let typed = self
                .disk_filter
                .get_or_insert_with(|| config.disk.disk_filter.join(" "));
            let edited = ui
                .add(
                    egui::TextEdit::singleline(typed)
                        .desired_width(220.0)
                        .hint_text("show all")
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
            ui.end_row();
            let entries: Vec<String> = typed
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            let filter = DiskFilter::parse(&entries);
            if edited {
                config.disk.disk_filter = entries
                    .iter()
                    .filter(|e| !filter.invalid().contains(e))
                    .cloned()
                    .collect();
                ch.look = true;
            }
            if !filter.invalid().is_empty() {
                ui.label("");
                ui.label(text(
                    format!("Ignored: {}", filter.invalid().join(" ")),
                    look.warn,
                ));
                ui.end_row();
            }
            hint_row(
                ui,
                look,
                "C: D: shows only those; !D: hides D:. Empty shows all",
            );
            if !info.drives.is_empty() {
                hint_row(ui, look, &format!("Drives now: {}", info.drives.join(" ")));
            }
        });
    }
}

fn data(ui: &mut Ui, config: &mut Config, look: &Look, ch: &mut Changes) {
    heading(ui, look, "Refresh");
    grid(ui, "refresh", |ui| {
        label(ui, look, "Interval");
        ch.interval |= ui
            .add(
                egui::DragValue::new(&mut config.refresh.update_ms)
                    .range(100..=86_400_000)
                    .speed(10)
                    .suffix(" ms"),
            )
            .changed();
        ui.end_row();
        hint_row(ui, look, "Same as - / + in the status bar");
    });

    ui.add_space(14.0);
    heading(ui, look, "Meter scales");
    grid(ui, "scales", |ui| {
        let rows: [(&str, &mut i64, &str, &str); 4] = [
            (
                "CPU clock",
                &mut config.cpu.cpu_clock_max_mhz,
                " MHz",
                "0 = highest clock seen this session",
            ),
            (
                "GPU clock",
                &mut config.gpu.gpu_clock_max_mhz,
                " MHz",
                "0 = the GPU's reported maximum",
            ),
            (
                "GPU power",
                &mut config.gpu.gpu_power_max_watts,
                " W",
                "0 = the driver's power limit; e.g. your card's board power",
            ),
            (
                "Swap",
                &mut config.mem.swap_max_mib,
                " MiB",
                "0 = the current pagefile allocation",
            ),
        ];
        for (name, value, unit, hint) in rows {
            let max = if unit == " MiB" {
                1_000_000_000
            } else {
                10_000
            };
            label(ui, look, name);
            ui.horizontal(|ui| {
                ch.look |= ui
                    .add(
                        egui::DragValue::new(value)
                            .range(0..=max)
                            .speed(10)
                            .suffix(unit),
                    )
                    .changed();
                ui.label(text(hint, look.label));
            });
            ui.end_row();
        }
        hint_row(
            ui,
            look,
            "Scales only change the display, never hardware settings",
        );
    });

    ui.add_space(14.0);
    heading(ui, look, "Processes");
    grid(ui, "processes", |ui| {
        label(ui, look, "CPU %");
        ch.look |= ui
            .checkbox(
                &mut config.view.proc_per_core,
                "100% = one logical core (otherwise all cores)",
            )
            .changed();
        ui.end_row();
    });
}

fn logging(ui: &mut Ui, config: &mut Config, look: &Look, info: &Context, ch: &mut Changes) {
    heading(ui, look, "Logging");
    grid(ui, "logging", |ui| {
        label(ui, look, "Level");
        egui::ComboBox::from_id_salt("log-level")
            .selected_text(config.log.log_level.to_string())
            .show_ui(ui, |ui| {
                for level in [
                    LevelFilter::OFF,
                    LevelFilter::ERROR,
                    LevelFilter::WARN,
                    LevelFilter::INFO,
                    LevelFilter::DEBUG,
                    LevelFilter::TRACE,
                ] {
                    ch.log |= ui
                        .selectable_value(&mut config.log.log_level, level, level.to_string())
                        .changed();
                }
            });
        ui.end_row();
        hint_row(ui, look, "debug and trace write a lot; warn is the default");

        label(ui, look, "Log file");
        path_row(ui, look, &info.log_dir.join("rztop.log"), info.log_dir);
        ui.end_row();
        label(ui, look, "Config file");
        let config_dir = info.config_path.parent().unwrap_or(info.config_path);
        path_row(ui, look, info.config_path, config_dir);
        ui.end_row();
    });
}

fn footer(ui: &mut Ui, look: &Look, info: &Context) {
    let (message, color) = match info.saved {
        None => (
            format!("Changes save to {}", info.config_path.display()),
            look.label,
        ),
        Some(Ok(())) => (
            format!("Saved to {}", info.config_path.display()),
            look.label,
        ),
        Some(Err(e)) => (format!("Could not save: {e}"), look.warn),
    };
    ui.label(text(message, color));
}

fn heading(ui: &mut Ui, look: &Look, title: &str) {
    ui.label(text(title, look.fg).strong());
    ui.add_space(4.0);
}

fn grid(ui: &mut Ui, id: &str, body: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([18.0, 8.0])
        .min_col_width(120.0)
        .show(ui, body);
}

fn label(ui: &mut Ui, look: &Look, name: &str) {
    ui.label(text(name, look.fg));
}

/// A muted explanation under the control in the second column.
fn hint_row(ui: &mut Ui, look: &Look, hint: &str) {
    ui.label("");
    ui.label(text(hint, look.label).size(11.5));
    ui.end_row();
}

fn path_row(ui: &mut Ui, look: &Look, path: &Path, folder: &Path) {
    ui.horizontal(|ui| {
        ui.label(text(path.display().to_string(), look.fg));
        if ui.button("Open folder").clicked() {
            // Explorer opens the folder; a missing folder opens Documents.
            let _ = std::process::Command::new("explorer").arg(folder).spawn();
        }
    });
}

fn kib(value: &mut i64) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .range(0..=10_000_000)
        .speed(10)
        .suffix(" KiB/s")
}

fn temp_name(scale: TempScale) -> &'static str {
    match scale {
        TempScale::Celsius => "Celsius (°C)",
        TempScale::Fahrenheit => "Fahrenheit (°F)",
        TempScale::Kelvin => "Kelvin (K)",
        TempScale::Rankine => "Rankine (°R)",
    }
}
