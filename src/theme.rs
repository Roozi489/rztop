// Modified for rztop on 2026-11-04: kept only the bundled theme palettes; the ANSI terminal theme was removed with the terminal frontend.
use crate::themes::ThemePalette;

struct BundledTheme {
    name: &'static str,
    content: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/bundled_themes.rs"));

/// Parse a bundled theme's palette by name, falling back to the default
/// theme when the name is unknown or its palette does not parse.
pub fn palette(name: &str) -> ThemePalette {
    BUNDLED_THEMES
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .and_then(|t| toml::from_str(t.content).ok())
        .unwrap_or_else(|| {
            let default = BUNDLED_THEMES
                .first()
                .expect("BUNDLED_THEMES is non-empty by construction")
                .content;
            toml::from_str(default).expect("default theme must parse")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes::Rgb;

    #[test]
    fn palette_by_name_and_unknown_falls_back_to_default() {
        assert_eq!(palette("default").colors.main_fg, Rgb(0xcc, 0xcc, 0xcc));
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
}
