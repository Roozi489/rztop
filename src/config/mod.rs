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

/// Keys the desktop Settings window edits, and so the only keys
/// [`Config::save`] writes.
pub const SETTINGS_KEYS: &[&str] = &[
    "color_theme",
    "temp_scale",
    "base_10_sizes",
    "update_ms",
    "cpu_clock_max_mhz",
    "gpu_clock_max_mhz",
    "gpu_power_max_watts",
    "swap_max_mib",
    "proc_per_core",
    "net_iface",
    "net_auto",
    "net_sync",
    "net_download",
    "net_upload",
    "disk_filter",
    "log_level",
];

/// Replace the `key = value` lines of `keys` in a flat TOML document with
/// the values from `table`, keeping every other line. A multi-line array
/// value is replaced whole. Keys not yet in the document are added before
/// its first table header, or at the end.
fn update_keys(existing: &str, table: &toml::Table, keys: &[&str]) -> String {
    let line_for = |key: &str| {
        table
            .get(key)
            .map(|value| format!("{key} = {}", value_text(value)))
    };
    let mut out: Vec<String> = Vec::new();
    let mut written: Vec<&str> = Vec::new();
    let mut header_at = None;
    let mut lines = existing.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if header_at.is_none() && trimmed.starts_with('[') {
            header_at = Some(out.len());
        }
        let key = header_at
            .is_none()
            .then(|| top_level_key(trimmed))
            .flatten()
            .and_then(|k| keys.iter().copied().find(|&name| name == k));
        let Some(key) = key else {
            out.push(line.to_owned());
            continue;
        };
        // Skip the rest of a value that spans several lines.
        let value = trimmed.split_once('=').map_or("", |(_, v)| v);
        let mut depth = bracket_depth(value);
        while depth > 0 {
            match lines.next() {
                Some(next) => depth += bracket_depth(next),
                None => break,
            }
        }
        if !written.contains(&key)
            && let Some(new) = line_for(key)
        {
            out.push(new);
            written.push(key);
        }
    }
    let missing: Vec<String> = keys
        .iter()
        .filter(|k| !written.contains(k))
        .filter_map(|k| line_for(k))
        .collect();
    let at = header_at.unwrap_or(out.len());
    out.splice(at..at, missing);
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// The bare key of a `key = value` line, if the line is one.
fn top_level_key(line: &str) -> Option<&str> {
    let (key, _) = line.split_once('=')?;
    let key = key.trim();
    (!key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    .then_some(key)
}

/// Net `[` minus `]` outside strings, to find where an array value ends.
fn bracket_depth(text: &str) -> i32 {
    let mut depth = 0;
    let mut quote = None;
    for c in text.chars() {
        match (quote, c) {
            (None, '#') => break,
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '[') => depth += 1,
            (None, ']') => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Inline TOML for one value.
fn value_text(value: &toml::Value) -> String {
    match value {
        toml::Value::Array(items) => {
            let items: Vec<String> = items.iter().map(value_text).collect();
            format!("[{}]", items.join(", "))
        }
        other => other.to_string(),
    }
}

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

    /// Save the keys the desktop Settings window edits ([`SETTINGS_KEYS`])
    /// into the TOML file at `path`, creating it if needed. Only those
    /// keys' lines are rewritten: comments, other keys and their order are
    /// kept, and keys missing from the file are appended.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let table = toml::Table::try_from(self).map_err(std::io::Error::other)?;
        let existing = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let updated = update_keys(&existing, &table, SETTINGS_KEYS);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Write a sibling file first so a failed write cannot truncate the config.
        let temp = path.with_extension("toml.tmp");
        fs::write(&temp, updated)?;
        fs::rename(&temp, path)
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

        // `color_theme` names a bundled theme or one of the user's own;
        // anything else falls back to the default. The other closed-set
        // fields are typed enums that serde validates on load.
        if !crate::theme::exists(&self.ui.color_theme) {
            validate_choice(
                &mut self.ui.color_theme,
                "default",
                crate::theme::THEME_NAMES,
                "color_theme",
                &mut warnings,
            );
        }

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
