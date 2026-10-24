//! Measure application-observed Windows Raw Input arrival time differences.
//! This is NOT absolute hardware latency without a shared physical trigger.

mod core;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn main() {
    if let Err(error) = windows::run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This app runs on Windows. The timing model tests are portable.");
    std::process::exit(1);
}
