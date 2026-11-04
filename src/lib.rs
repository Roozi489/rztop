// Modified for rztop on 2026-11-04: the desktop GUI is the only frontend.
mod collect;
mod config;
mod desktop;
mod domain;
mod log;
mod runner;
mod theme;
mod themes;
mod tools;

pub use desktop::run_desktop;
