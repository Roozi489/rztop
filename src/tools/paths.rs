// Modified for rztop on 2026-11-02: fork identity and configuration isolation.
/// Priority: `XDG_CONFIG_HOME/rztop` > `%APPDATA%/rztop` (via `directories` crate)
pub fn config_dir() -> std::path::PathBuf {
    config_dir_inner(std::env::var("XDG_CONFIG_HOME").ok().as_deref())
}

fn config_dir_inner(xdg: Option<&str>) -> std::path::PathBuf {
    if let Some(xdg) = xdg {
        let p = std::path::PathBuf::from(xdg);
        if p.is_absolute() {
            return p.join("rztop");
        }
    }
    directories::BaseDirs::new()
        .map(|d| d.config_dir().join("rztop"))
        .unwrap_or_else(|| std::path::PathBuf::from("rztop"))
}

/// Priority: `XDG_STATE_HOME/rztop` > `%LOCALAPPDATA%/rztop` (via `directories` crate)
pub fn data_dir() -> std::path::PathBuf {
    data_dir_inner(std::env::var("XDG_STATE_HOME").ok().as_deref())
}

fn data_dir_inner(xdg: Option<&str>) -> std::path::PathBuf {
    if let Some(xdg) = xdg {
        let p = std::path::PathBuf::from(xdg);
        if p.is_absolute() {
            return p.join("rztop");
        }
    }
    directories::BaseDirs::new()
        .map(|d| d.data_local_dir().join("rztop"))
        .unwrap_or_else(|| std::path::PathBuf::from("rztop"))
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

pub fn username() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_returns_path_with_rztop_suffix() {
        let dir = config_dir();
        assert!(
            dir.ends_with("rztop"),
            "expected path ending with 'rztop', got: {:?}",
            dir
        );
    }

    #[test]
    fn data_dir_returns_path_with_rztop_suffix() {
        let dir = data_dir();
        assert!(
            dir.ends_with("rztop"),
            "expected path ending with 'rztop', got: {:?}",
            dir
        );
    }

    #[test]
    fn config_dir_respects_xdg_env() {
        let dir = config_dir_inner(Some("C:\\custom\\xdg"));
        assert_eq!(dir, std::path::PathBuf::from("C:\\custom\\xdg\\rztop"));
    }

    #[test]
    fn config_dir_ignores_relative_xdg() {
        let dir = config_dir_inner(Some("relative/path"));
        assert_ne!(dir, std::path::PathBuf::from("relative/path/rztop"));
    }
}
