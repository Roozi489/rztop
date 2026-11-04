// Modified for rztop on 2026-11-04: the desktop GUI is the only frontend.
#![windows_subsystem = "windows"]

fn main() -> eframe::Result {
    rztop::run_desktop()
}
