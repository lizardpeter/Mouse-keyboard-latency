#![cfg_attr(windows, windows_subsystem = "windows")]

mod core;
#[cfg(windows)] mod windows;

#[cfg(windows)] fn main() { if let Err(error) = windows::run() { eprintln!("GUI error: {error}"); std::process::exit(1); } }
#[cfg(not(windows))] fn main() { eprintln!("Windows x64 desktop application. The timing logic tests are portable."); }
