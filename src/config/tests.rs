//! Integration tests for Config (load/write/validate and integer bounds).
//!
//! Lives in its own file purely so mod.rs does not carry ~700 lines of
//! tests on top of the production code; the test module remains gated
//! by `#[cfg(test)]` from mod.rs declaration.

use super::*;
use crate::collect::process_display::ProcSort;
use crate::domain::config_enums::GraphSymbol;
use tracing_subscriber::filter::LevelFilter;

#[test]
fn legacy_process_view_alias_loads_and_saves_under_new_name() {
    for enabled in [true, false] {
        let config: Config = toml::from_str(&format!("proc_dual_view = {enabled}")).unwrap();
        assert_eq!(config.proc.proc_rankings, enabled);
        let saved = toml::to_string(&config).unwrap();
        assert!(saved.contains(&format!("proc_rankings = {enabled}")));
        assert!(!saved.contains("proc_dual_view"));
        let restored: Config = toml::from_str(&saved).unwrap();
        assert_eq!(restored.proc.proc_rankings, enabled);
    }
}

#[test]
fn four_process_panels_default_on_but_respect_saved_choice() {
    assert!(Config::new().proc.proc_rankings);
    let fresh: Config = toml::from_str("").unwrap();
    assert!(fresh.proc.proc_rankings);
    let older: Config = toml::from_str("update_ms = 1000").unwrap();
    assert!(older.proc.proc_rankings);
    let single: Config = toml::from_str("proc_rankings = false").unwrap();
    assert!(!single.proc.proc_rankings);
    let four: Config = toml::from_str("proc_rankings = true").unwrap();
    assert!(four.proc.proc_rankings);
}

#[test]
fn swap_scale_round_trips_and_validates() {
    let mut config = Config::new();
    assert_eq!(config.mem.swap_max_mib, 0);
    IntKey::SwapMaxMib.set(&mut config, 59487);
    let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
    assert_eq!(restored.mem.swap_max_mib, 59487);
    assert!(!IntKey::SwapMaxMib.bounds().contains(&-1));
}

#[test]
fn gpu_power_scale_round_trips_and_validates() {
    let mut config = Config::new();
    assert_eq!(config.gpu.gpu_power_max_watts, 0);
    IntKey::GpuPowerMaxWatts.set(&mut config, 304);
    let saved = toml::to_string(&config).unwrap();
    let restored: Config = toml::from_str(&saved).unwrap();
    assert_eq!(restored.gpu.gpu_power_max_watts, 304);
    assert!(!IntKey::GpuPowerMaxWatts.bounds().contains(&-1));
    assert!(!IntKey::GpuPowerMaxWatts.bounds().contains(&10001));
}

#[test]
fn load_empty_file_uses_defaults() {
    let mut config = Config::new();
    let tmp = std::env::temp_dir().join("rtop_test_empty.toml");
    fs::write(&tmp, "").unwrap();
    let warnings = config.load(&tmp);
    assert!(warnings.is_empty());
    assert_eq!(config.refresh.update_ms, 2000);
    assert_eq!(config.ui.color_theme, "default");
    assert!(config.ui.theme_background);
    let _ = fs::remove_file(&tmp);
}

#[test]
fn load_valid_config_parses_all_types() {
    let mut config = Config::new();
    let tmp = std::env::temp_dir().join("rtop_test_valid.toml");
    fs::write(
        &tmp,
        "color_theme = \"dracula\"\ntheme_background = false\nupdate_ms = 500\n",
    )
    .unwrap();

    let warnings = config.load(&tmp);
    assert!(warnings.is_empty());
    assert_eq!(config.ui.color_theme, "dracula");
    assert!(!config.ui.theme_background);
    assert_eq!(config.refresh.update_ms, 500);
    let _ = fs::remove_file(&tmp);
}

#[test]
fn load_preserves_comments() {
    let mut config = Config::new();
    let tmp = std::env::temp_dir().join("rtop_test_comments.toml");
    fs::write(&tmp, "# this is a comment\nupdate_ms = 1000\n").unwrap();
    let warnings = config.load(&tmp);
    assert!(warnings.is_empty());
    assert_eq!(config.refresh.update_ms, 1000);
    let _ = fs::remove_file(&tmp);
}

#[test]
fn load_unknown_keys_are_ignored() {
    let mut config = Config::new();
    let tmp = std::env::temp_dir().join("rtop_test_unknown.toml");
    fs::write(&tmp, "nonexistent_key = \"value\"\n").unwrap();
    let warnings = config.load(&tmp);
    // serde with #[serde(default)] silently ignores unknown keys
    assert!(warnings.is_empty());
    let _ = fs::remove_file(&tmp);
}

#[test]
fn load_invalid_toml_generates_warning() {
    let mut config = Config::new();
    let tmp = std::env::temp_dir().join("rtop_test_badtoml.toml");
    fs::write(&tmp, "this is not valid toml {{{\n").unwrap();
    let warnings = config.load(&tmp);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("parse"));
    let _ = fs::remove_file(&tmp);
}

#[test]
fn load_out_of_range_int_generates_warning() {
    let mut config = Config::new();
    let tmp = std::env::temp_dir().join("rtop_test_out_of_range.toml");
    fs::write(&tmp, "update_ms = 50\n").unwrap();
    let warnings = config.load(&tmp);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("out of range"));
    assert_eq!(config.refresh.update_ms, 100);
    let _ = fs::remove_file(&tmp);
}

#[test]
fn load_invalid_typed_value_aborts_whole_parse() {
    // An invalid value for a typed enum field causes serde to
    // fail the whole `toml::from_str::<Config>` and the loader
    // returns a single "Failed to parse config" warning; the
    // entire config falls back to defaults.
    let mut config = Config::new();
    let tmp = std::env::temp_dir().join("rtop_test_bad_typed_value.toml");
    fs::write(&tmp, "graph_symbol = \"ascii\"\n").unwrap();
    let warnings = config.load(&tmp);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("Failed to parse config"));
    assert!(warnings[0].contains("graph_symbol"));
    assert_eq!(config.ui.graph_symbol, GraphSymbol::Braille);
    assert_eq!(config.ui.color_theme, "default");
    let _ = fs::remove_file(&tmp);
}

#[test]
fn load_invalid_string_choice_resets_just_that_field() {
    // String-typed fields validated by `validate_choice` reset
    // the offending field and emit a per-field warning while
    // the rest of the config loads normally.
    let mut config = Config::new();
    let tmp = std::env::temp_dir().join("rtop_test_bad_string_values.toml");
    fs::write(&tmp, "color_theme = \"foo\"\n").unwrap();
    let warnings = config.load(&tmp);
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("color_theme") && w.contains("foo"))
    );
    assert_eq!(config.ui.color_theme, "default");
    let _ = fs::remove_file(&tmp);
}

#[test]
fn write_roundtrip_preserves_values() {
    let mut config = Config::new();
    config.ui.color_theme = "nord".to_string();
    config.ui.vim_keys = true;
    config.refresh.update_ms = 1500;

    let tmp = std::env::temp_dir().join("rtop_test_roundtrip.toml");
    config.write(&tmp).unwrap();

    let mut config2 = Config::new();
    config2.load(&tmp);
    assert_eq!(config2.ui.color_theme, "nord");
    assert!(config2.ui.vim_keys);
    assert_eq!(config2.refresh.update_ms, 1500);
    let _ = fs::remove_file(&tmp);
}

#[test]
fn defaults_are_correct() {
    let config = Config::new();
    assert!(config.ui.theme_background);
    assert!(!config.ui.vim_keys);
    assert_eq!(config.refresh.update_ms, 2000);
    assert_eq!(config.ui.color_theme, "default");
    assert_eq!(config.ui.graph_symbol, GraphSymbol::Braille);
    assert_eq!(config.view.proc_sorting, ProcSort::Cpu);
}

/// Round-trip an existing-format `rtop.toml` through load to
/// verify the per-subsystem split preserves the on-disk
/// flat-key format. Asserts each section's fields populate
/// correctly from top-level TOML keys.
#[test]
fn load_existing_flat_format_populates_all_sections() {
    let mut config = Config::new();
    let tmp = std::env::temp_dir().join("rtop_test_flat_format.toml");
    fs::write(
        &tmp,
        r#"
color_theme = "dracula"
theme_background = false
vim_keys = true
update_ms = 500
cpu_update_ms = 250
cpu_invert_lower = false
statusbar_show_uptime = false
custom_cpu_name = "My CPU"
show_swap = false
io_mode = true
disk_filter = ["C:", "!D:"]
net_iface = "{12345678-1234-1234-1234-123456789012}"
net_download = 5000
proc_tree = true
proc_sorting = "memory"
proc_filter = "chrome"
gpu_iface = "NVIDIA:GPU-aaaa"
log_level = "info"
hidden_widgets = ["mem", "gpu"]
"#,
    )
    .unwrap();
    let warnings = config.load(&tmp);
    assert!(warnings.is_empty(), "warnings: {warnings:?}");

    assert_eq!(config.ui.color_theme, "dracula");
    assert!(!config.ui.theme_background);
    assert!(config.ui.vim_keys);

    assert_eq!(config.refresh.update_ms, 500);
    assert_eq!(config.refresh.cpu_update_ms, 250);

    assert!(!config.cpu.cpu_invert_lower);
    assert_eq!(config.cpu.custom_cpu_name, "My CPU");

    assert!(!config.statusbar.statusbar_show_uptime);

    assert!(!config.mem.show_swap);

    assert!(config.view.io_mode);
    assert_eq!(config.disk.disk_filter, vec!["C:", "!D:"]);

    assert_eq!(
        config.view.net_iface,
        "{12345678-1234-1234-1234-123456789012}"
    );
    assert_eq!(config.net.net_download, 5000);

    assert!(config.view.proc_tree);
    assert_eq!(config.view.proc_sorting, ProcSort::Memory);
    assert_eq!(config.view.proc_filter, "chrome");

    // GPU view (cycling-GPU widget persists the last-viewed
    // device's stable id; no per-device labels exist).
    assert_eq!(config.view.gpu_iface, "NVIDIA:GPU-aaaa");

    assert_eq!(config.log.log_level, LevelFilter::INFO);

    let _ = fs::remove_file(&tmp);
}

/// Round-trip a Config through write+load to verify the
/// serialised form is itself a valid input — the per-subsystem
/// split must not introduce any field name drift.
#[test]
fn write_and_reload_round_trips_cleanly() {
    let mut original = Config::new();
    original.ui.color_theme = "nord".to_string();
    original.ui.vim_keys = true;
    original.refresh.update_ms = 750;
    original.view.proc_tree = true;
    original.cpu.show_cpu_freq = false;
    original.view.gpu_iface = "AMD:UDID-deadbeef".to_string();
    original.log.log_level = LevelFilter::DEBUG;

    let tmp = std::env::temp_dir().join("rtop_test_subsystem_roundtrip.toml");
    original.write(&tmp).unwrap();

    let mut loaded = Config::new();
    let warnings = loaded.load(&tmp);
    assert!(warnings.is_empty(), "round-trip warnings: {warnings:?}");

    assert_eq!(loaded.ui.color_theme, "nord");
    assert!(loaded.ui.vim_keys);
    assert_eq!(loaded.refresh.update_ms, 750);
    assert!(loaded.view.proc_tree);
    assert!(!loaded.cpu.show_cpu_freq);
    assert_eq!(loaded.view.gpu_iface, "AMD:UDID-deadbeef");
    assert_eq!(loaded.log.log_level, LevelFilter::DEBUG);

    let _ = fs::remove_file(&tmp);
}

#[test]
fn validate_clamps_ints() {
    let mut config = Config::new();
    config.refresh.update_ms = 50;
    config.net.net_download = -1;
    let warnings = config.validate();
    assert_eq!(config.refresh.update_ms, 100);
    assert_eq!(config.net.net_download, 0);
    assert!(warnings.len() >= 2);
}

#[test]
fn validate_keeps_valid_disk_filter_unchanged() {
    let mut config = Config::new();
    config.disk.disk_filter = vec!["C:".into(), "!D:".into()];
    let warnings = config.validate();
    assert!(
        !warnings.iter().any(|w| w.contains("disk_filter")),
        "valid disk_filter should not warn, got: {warnings:?}"
    );
    assert_eq!(
        config.disk.disk_filter,
        vec!["C:".to_string(), "!D:".to_string()]
    );
}

#[test]
fn validate_warns_and_strips_invalid_disk_filter_entries() {
    let mut config = Config::new();
    config.disk.disk_filter = vec![
        "C:".into(),
        "abc".into(),
        "D:".into(),
        "3".into(),
        "!!".into(),
    ];
    let warnings = config.validate();
    let disks_warning = warnings
        .iter()
        .find(|w| w.contains("disk_filter"))
        .expect("validate must surface a disk_filter warning");
    assert!(disks_warning.contains("abc"));
    assert!(disks_warning.contains('3'));
    assert!(disks_warning.contains("!!"));
    assert_eq!(
        config.disk.disk_filter,
        vec!["C:".to_string(), "D:".to_string()]
    );
}

#[test]
fn validate_empty_disk_filter_does_not_warn() {
    let mut config = Config::new();
    config.disk.disk_filter = Vec::new();
    let warnings = config.validate();
    assert!(!warnings.iter().any(|w| w.contains("disk_filter")));
    assert!(config.disk.disk_filter.is_empty());
}

#[test]
fn gpu_update_ms_round_trips_through_field() {
    // The cycling-GPU widget uses a single global interval — the
    // schema exposes one `IntKey::GpuUpdateMs` reading and writing
    // the scalar `refresh.gpu_update_ms` field.
    let mut config = crate::config::Config::new();
    IntKey::GpuUpdateMs.set(&mut config, 1234);
    assert_eq!(IntKey::GpuUpdateMs.get(&config), 1234);
    assert_eq!(config.refresh.gpu_update_ms, 1234);
}

#[test]
fn save_rewrites_only_settings_keys_and_keeps_the_rest() {
    let dir = std::env::temp_dir().join(format!("rztop-save-{}", std::process::id()));
    let path = dir.join("rztop.toml");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &path,
        "# my settings\nvim_keys = true\ncolor_theme = \"nord\" # old\n\
         disk_filter = [\n  \"C:\",\n  \"D:\",\n]\nupdate_ms = 1000\n",
    )
    .unwrap();
    let mut config = Config::new();
    config.load(&path);
    config.ui.color_theme = "dracula".to_owned();
    config.refresh.update_ms = 2500;
    config.disk.disk_filter = vec!["!E:".to_owned()];
    config.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();

    assert!(text.starts_with("# my settings\nvim_keys = true\n"));
    assert!(text.contains("color_theme = \"dracula\"\n"));
    assert!(text.contains("disk_filter = [\"!E:\"]\nupdate_ms = 2500\n"));
    assert!(!text.contains("\"D:\""));
    // Keys the file did not have are appended once.
    assert_eq!(text.matches("temp_scale = \"celsius\"").count(), 1);
    assert!(text.contains("log_level = \"warn\""));
    let reloaded: Config = toml::from_str(&text).unwrap();
    assert!(reloaded.ui.vim_keys);
    assert_eq!(reloaded.refresh.update_ms, 2500);
}
