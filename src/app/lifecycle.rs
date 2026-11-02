// Modified for rztop on 2026-11-02: fork identity and configuration isolation.
//! Process-lifecycle helpers: input thread, terminal IO operations,
//! theme styling, and config save-on-exit.
//!
//! These are stateless, IO-adjacent helpers that the event loop
//! invokes at well-defined moments (startup, per-keystroke,
//! shutdown). They are intentionally kept apart from `state.rs` and
//! `dirty_exec.rs` so the data-pipeline modules don't pull in
//! terminal types.

use crate::config;
use crate::event::AppEvent;
use crate::input;
use crate::theme;
use crossterm::event::Event;

/// Spawn a thread that blocks on `crossterm::event::read()` and forwards
/// key presses and resize events through the event channel.
///
/// The thread is not joined — it exits when `tx` is dropped (all senders
/// gone) or when the process exits.
pub(crate) fn spawn_input_thread(tx: std::sync::mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        loop {
            match crossterm::event::read() {
                Ok(Event::Key(key)) => {
                    if let Some(k) = input::translate_key(key)
                        && tx.send(AppEvent::Key(k)).is_err()
                    {
                        break;
                    }
                }
                Ok(Event::Resize(_, _)) if tx.send(AppEvent::Resize).is_err() => {
                    break;
                }
                Ok(Event::Mouse(mouse)) if tx.send(AppEvent::Mouse(mouse)).is_err() => {
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        subsystem = %crate::log::Subsystem::Input,
                        error = %e,
                        "crossterm::event::read failed",
                    );
                }
            }
        }
    });
}

pub(crate) fn style_terminal_output(
    output: &str,
    config: &config::Config,
    theme: &theme::Theme,
) -> String {
    theme.style_output(output, config.ui.theme_background)
}

/// Last successfully persisted content. Serialization owns a string so failed writes
/// can be retried without treating unsaved changes as the disk baseline.
pub(crate) struct SettingsPersistence {
    saved: Option<String>,
}

impl SettingsPersistence {
    pub(crate) fn new(config: &config::Config) -> Self {
        Self {
            saved: config
                .save_path()
                .exists()
                .then(|| toml::to_string_pretty(config).ok())
                .flatten(),
        }
    }

    pub(crate) fn save(&mut self, config: &mut config::Config, state: &super::AppState) {
        // Inline editors are previews until confirmed. Mouse drags also gate this
        // policy at their event-dispatch call site until release.
        if matches!(
            state.overlay.active.kind(),
            crate::overlay::OverlayKind::Filter | crate::overlay::OverlayKind::OptionsEdit
        ) {
            return;
        }
        config.hidden_widgets = state.filter.hidden;
        state.view.sync_to_config(&mut config.view);
        if let Err(error) = self.save_changed(config) {
            tracing::warn!(subsystem = %crate::log::Subsystem::Config, %error,
                path = %config.save_path().display(), "config save failed");
        }
    }

    fn save_changed(&mut self, config: &config::Config) -> std::io::Result<bool> {
        let content = toml::to_string_pretty(config).map_err(std::io::Error::other)?;
        if self.saved.as_deref() == Some(content.as_str()) {
            return Ok(false);
        }
        config.write(&config.save_path())?;
        self.saved = Some(content);
        Ok(true)
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    #[test]
    fn first_run_saves_defaults_once() {
        let path =
            std::env::temp_dir().join(format!("rztop-first-save-{}.toml", std::process::id()));
        assert!(!path.exists());
        let mut config = config::Config::new();
        config.set_save_path(path.clone());
        let mut persistence = SettingsPersistence::new(&config);
        assert!(persistence.save_changed(&config).unwrap());
        assert!(!persistence.save_changed(&config).unwrap());
        let mut loaded = config::Config::new();
        assert!(loaded.load(&path).is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn changed_settings_use_custom_path_and_retry_failed_write() {
        let dir = std::env::temp_dir().join(format!("rztop-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = config::Config::new();
        config.set_save_path(dir.clone()); // A directory cannot be written as a file.
        let mut persistence = SettingsPersistence::new(&config);
        assert!(!persistence.save_changed(&config).unwrap());
        config.refresh.update_ms = 1234;
        assert!(persistence.save_changed(&config).is_err());
        let path = dir.join("custom.toml");
        config.set_save_path(path.clone());
        assert!(persistence.save_changed(&config).unwrap());
        assert!(!persistence.save_changed(&config).unwrap());
        let mut loaded = config::Config::new();
        assert!(loaded.load(&path).is_empty());
        assert_eq!(loaded.refresh.update_ms, 1234);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
