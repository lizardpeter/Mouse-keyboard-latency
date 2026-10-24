#![cfg_attr(windows, windows_subsystem = "windows")]

mod core;
mod keyboard;
mod reaction;
mod typing;
#[cfg(windows)] mod windows;

#[cfg(windows)] fn main() { if let Err(error) = windows::run() { eprintln!("GUI error: {error}"); std::process::exit(1); } }
#[cfg(not(windows))] fn main() { eprintln!("Windows x64 desktop application. Cross-platform diagnostic tests are available."); }
