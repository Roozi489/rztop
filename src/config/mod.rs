// Modified for rztop on 2026-11-04: dropped the terminal layout presets, widget toggles and pane resizing.
//! Persisted configuration state and the integer bounds for validating it.
//!
//! The persisted shape is composed of per-subsystem sections (see
//! [`sections`] — `UiConfig`, `RefreshConfig`, ..., `ViewConfig`,
//! `LogConfig`) flattened into a single TOML root.
//!
//! Integer bounds live in [`schema`] ([`IntKey`]). Adding or modifying
//! a key is a one-table edit in the `int_keys!` invocation in
//! `schema.rs`.

mod schema;
mod sections;
#[cfg(test)]
mod tests;

pub use schema::*;
pub use sections::*;

use sections::validate_choice;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// All persisted configuration state for rztop.
///
/// Composed of per-subsystem sections (see [`sections`]) flattened
/// into a single TOML root via `#[serde(flatten)]`. Keys left over from
/// the terminal frontend (`preset`, `custom_layout`, `hidden_widgets`,
/// `resizing`) are ignored on load.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(flatten)]
    pub ui: UiConfig,
    #[serde(flatten)]
    pub refresh: RefreshConfig,
    #[serde(flatten)]
    pub cpu: CpuConfig,
    #[serde(flatten)]
    pub mem: MemConfig,
    #[serde(flatten)]
    pub net: NetConfig,
    #[serde(flatten)]
    pub proc: ProcConfig,
    #[serde(flatten)]
    pub gpu: GpuConfig,
    #[serde(flatten)]
    pub disk: DiskConfig,
    #[serde(flatten)]
    pub log: LogConfig,
    /// View-state section; see [`ViewConfig`].
    #[serde(flatten)]
    pub view: ViewConfig,

    /// Statusbar widget settings (master visibility, per-item
    /// visibility, clock format). See [`StatusbarConfig`].
    #[serde(flatten)]
    pub statusbar: StatusbarConfig,
}

impl Config {
    /// Create a default config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load config from a TOML file. Returns warnings for invalid values.
    pub fn load(&mut self, path: &Path) -> Vec<String> {
        let mut warnings = Vec::new();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return warnings,
        };

        if content.trim().is_empty() {
            return warnings;
        }

        match toml::from_str::<Config>(&content) {
            Ok(loaded) => *self = loaded,
            Err(e) => {
                warnings.push(format!("Failed to parse config: {e}"));
                return warnings;
            }
        }

        warnings.append(&mut self.validate());
        warnings
    }

    /// Write config to a TOML file.
    #[cfg(test)]
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let content =
            toml::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)
    }

    /// Validate all config values, clamping ints and resetting invalid strings.
    pub fn validate(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();

        // Clamp every integer field to its declared bounds. Each
        // IntKey owns its `[min, max]` range in one place; this loop
        // is the only consumer that mutates out-of-range values
        // back into range.
        for &key in IntKey::ALL {
            let old = key.get(self);
            let new = key.clamp_to_bounds(old);
            if new != old {
                warnings.push(format!(
                    "Value for '{}' out of range ({old}), clamped to {new}",
                    key.name(),
                ));
                key.set(self, new);
            }
        }

        // The only remaining `validate_choice` call covers
        // `color_theme` — the single string-typed field with a
        // closed set of choices. The other browsable config
        // fields are typed enums whose validity is enforced by
        // serde at deserialise time.
        validate_choice(
            &mut self.ui.color_theme,
            "default",
            crate::theme::THEME_NAMES,
            "color_theme",
            &mut warnings,
        );

        // Validate disk_filter: surface invalid drive entries as
        // warnings and drop them in place so the saved config
        // matches what the runtime actually uses.
        let filter = crate::domain::disk::DiskFilter::parse(&self.disk.disk_filter);
        if !filter.invalid().is_empty() {
            warnings.push(format!(
                "Invalid drive entry/entries in 'disk_filter': {}",
                filter.invalid().join(", ")
            ));
            let invalid_tokens = filter.invalid().to_vec();
            self.disk
                .disk_filter
                .retain(|tok| !invalid_tokens.contains(tok));
        }

        warnings
    }
}
