// Modified for rztop on 2026-11-04: kept only the bundled theme palettes; the ANSI terminal theme was removed with the terminal frontend.
// Modified for rztop on 2026-11-07: user themes from the theme editor, saved beside the config.
use crate::themes::ThemePalette;
use std::path::PathBuf;

struct BundledTheme {
    name: &'static str,
    content: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/bundled_themes.rs"));

/// Parse a theme's palette by name: a bundled theme, else a user theme
/// from [`user_dir`]. Falls back to the default theme when the name is
/// unknown or its palette does not parse.
pub fn palette(name: &str) -> ThemePalette {
    BUNDLED_THEMES
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .and_then(|t| toml::from_str(t.content).ok())
        .or_else(|| user_palette(name))
        .unwrap_or_else(|| {
            let default = BUNDLED_THEMES
                .first()
                .expect("BUNDLED_THEMES is non-empty by construction")
                .content;
            toml::from_str(default).expect("default theme must parse")
        })
}

/// Folder of the user's own themes: one rtop-format `<name>.toml` each.
pub fn user_dir() -> PathBuf {
    crate::tools::config_dir().join("themes")
}

pub fn is_bundled(name: &str) -> bool {
    THEME_NAMES.iter().any(|t| t.eq_ignore_ascii_case(name))
}

/// Whether `name` is a bundled theme or a user theme that parses.
pub fn exists(name: &str) -> bool {
    is_bundled(name) || user_palette(name).is_some()
}

/// Every theme name: the bundled ones, then the user's own in name order.
/// A user theme named like a bundled one is skipped; the bundled one wins.
pub fn names() -> Vec<String> {
    let mut user: Vec<String> = std::fs::read_dir(user_dir())
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let path = e.ok()?.path();
            (path.extension()? == "toml").then_some(())?;
            let name = path.file_stem()?.to_str()?.to_owned();
            (valid_name(&name) && !is_bundled(&name)).then_some(name)
        })
        .collect();
    user.sort_by_key(|n| n.to_ascii_lowercase());
    THEME_NAMES
        .iter()
        .map(|&n| n.to_owned())
        .chain(user)
        .collect()
}

/// A theme name usable as a file name: letters, digits, `_` and `-`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn user_palette(name: &str) -> Option<ThemePalette> {
    if !valid_name(name) {
        return None;
    }
    let text = std::fs::read_to_string(user_dir().join(format!("{name}.toml"))).ok()?;
    toml::from_str(&text).ok()
}

/// Write a user theme, replacing one of the same name. Bundled names are
/// refused so a user theme can never hide a bundled one.
pub fn save_user(name: &str, palette: &ThemePalette) -> std::io::Result<()> {
    if !valid_name(name) {
        return Err(std::io::Error::other(
            "use letters, digits, _ or - for the name",
        ));
    }
    if is_bundled(name) {
        return Err(std::io::Error::other(
            "a bundled theme has that name; pick another",
        ));
    }
    let body = toml::to_string_pretty(palette).map_err(std::io::Error::other)?;
    let dir = user_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join(format!("{name}.toml")),
        format!("# rztop theme, made with the theme editor. Also loads in rtop.\n\n{body}"),
    )
}

pub fn delete_user(name: &str) -> std::io::Result<()> {
    if !valid_name(name) || is_bundled(name) {
        return Err(std::io::Error::other("not a user theme"));
    }
    std::fs::remove_file(user_dir().join(format!("{name}.toml")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes::Rgb;

    #[test]
    fn palette_by_name_and_unknown_falls_back_to_default() {
        assert_eq!(palette("default").colors.main_fg, Rgb(0xd4, 0xbe, 0x98));
        assert_eq!(
            palette("no-such-theme").colors.main_fg,
            palette("default").colors.main_fg
        );
    }

    #[test]
    fn all_bundled_themes_parse_successfully() {
        for bt in BUNDLED_THEMES {
            let result = toml::from_str::<ThemePalette>(bt.content);
            assert!(
                result.is_ok(),
                "bundled theme '{}' failed to parse: {}",
                bt.name,
                result.unwrap_err()
            );
        }
    }

    #[test]
    fn every_bundled_theme_declares_main_bg() {
        // `main_bg` is required — every theme must declare it.
        // This pins the contract so no future theme contribution
        // can omit the field and have it silently default.
        for bt in BUNDLED_THEMES {
            let palette: ThemePalette = toml::from_str(bt.content)
                .unwrap_or_else(|e| panic!("bundled theme '{}' failed to parse: {e}", bt.name));
            // If parsing succeeded, `main_bg: Rgb` is structurally
            // present. Sanity-check by reading it.
            let _ = palette.colors.main_bg;
        }
    }

    #[test]
    fn every_bundled_theme_gives_data_label_fg_its_own_shade() {
        // `data_label_fg` only earns its place if it is visually
        // distinct from the value colour beside it and from the
        // background behind it. Guard both ends for every theme so a
        // future contribution cannot copy `main_fg` verbatim and
        // silently undo the label/value hierarchy.
        for bt in BUNDLED_THEMES {
            let palette: ThemePalette = toml::from_str(bt.content)
                .unwrap_or_else(|e| panic!("bundled theme '{}' failed to parse: {e}", bt.name));
            let c = &palette.colors;
            assert_ne!(
                c.data_label_fg, c.main_fg,
                "theme '{}': data_label_fg must differ from main_fg",
                bt.name,
            );
            assert_ne!(
                c.data_label_fg, c.main_bg,
                "theme '{}': data_label_fg must differ from main_bg",
                bt.name,
            );
        }
    }

    #[test]
    fn every_bundled_theme_keeps_labels_brighter_than_dead_rows() {
        // Brightness order is value > label > dead row. Compare each
        // against `main_bg` so the check holds for light themes, where
        // "brighter" means closer to a dark foreground.
        let distance = |a: Rgb, b: Rgb| {
            let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).abs();
            d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
        };
        for bt in BUNDLED_THEMES {
            let palette: ThemePalette = toml::from_str(bt.content)
                .unwrap_or_else(|e| panic!("bundled theme '{}' failed to parse: {e}", bt.name));
            let c = &palette.colors;
            let label = distance(c.data_label_fg, c.main_bg);
            let value = distance(c.main_fg, c.main_bg);
            assert!(
                label < value,
                "theme '{}': data_label_fg must sit closer to the background than main_fg \
                 (label {label}, value {value})",
                bt.name,
            );
        }
    }

    #[test]
    fn theme_names_matches_bundled_themes() {
        assert_eq!(
            THEME_NAMES.len(),
            BUNDLED_THEMES.len(),
            "THEME_NAMES and BUNDLED_THEMES must have the same length"
        );
        for (i, bt) in BUNDLED_THEMES.iter().enumerate() {
            assert_eq!(
                THEME_NAMES[i], bt.name,
                "THEME_NAMES[{i}] does not match BUNDLED_THEMES"
            );
        }
    }

    #[test]
    fn saved_palette_reads_back_identically() {
        for bt in BUNDLED_THEMES {
            let palette: ThemePalette = toml::from_str(bt.content).unwrap();
            let text = toml::to_string_pretty(&palette).unwrap();
            let again: ThemePalette = toml::from_str(&text).unwrap();
            assert_eq!(
                toml::to_string(&again).unwrap(),
                toml::to_string(&palette).unwrap(),
                "theme '{}' changed in a save round trip",
                bt.name,
            );
        }
    }

    #[test]
    fn theme_names_must_be_file_safe() {
        assert!(valid_name("my_theme-2"));
        assert!(!valid_name(""));
        assert!(!valid_name("../evil"));
        assert!(!valid_name("has space"));
        assert!(
            names()
                .iter()
                .take(THEME_NAMES.len())
                .eq(THEME_NAMES.iter())
        );
    }
}
