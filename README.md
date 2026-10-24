# Mouse-keyboard-latency

A lightweight **Rust + Windows Raw Input** tester that compares when left mouse clicks and **Space** keypresses reach a Windows application. Uses `QueryPerformanceCounter` for high-resolution timestamps. No Rust dependencies, drivers, network connections, or input injection.

## Run on Windows 10/11 (x64)

1. Install [Rust](https://rustup.rs/).
2. Clone this repository or download its ZIP.
3. Double-click `run_windows.bat`, or run `cargo run --release` from the repository root.
4. Press **left mouse button** and **Space** close together, then release both. Repeat 30-100 or more times.
5. Press **F11** to print intermediate statistics. Press **F12** to finish and save `input_timing_results.csv`.

Raw Input is received by a hidden window even when the console is not focused. Both events must arrive within 300 ms to count as a trial. Held buttons and repeated Space key events are ignored. Input timestamps are captured inside the Win32 message callback; console printing is deferred.

## Reading the results

Signed difference is **keyboard arrival time minus mouse arrival time**, in milliseconds.

- **Positive** means the mouse event arrived first.
- **Negative** means the keyboard event arrived first.
- **Zero** means their software timestamps matched.

Reports number of pairs, mouse-first/keyboard-first counts, signed mean/median, median and 95th-percentile absolute separation, and minimum/maximum absolute separation. The CSV includes per-trial event arrival offsets relative to startup.

**Critical measurement limitation:** This is not a direct measurement of device hardware input latency. A hand-pressed mouse and keyboard are not triggered at exactly the same time. Measurements also include USB polling, OS queueing, thread scheduling, and callback timing. To compare actual hardware latencies, use a common physical/electrical trigger for both switches; for absolute latency, also measure the physical activation time with external hardware.

Use a wired device pair, a quiet system, and repeated measurements for best repeatability. Disconnect other input devices: this version captures all registered mice/keyboards rather than filtering by device.

## Building and testing

```powershell
cargo test
cargo build --release
```

GitHub Actions runs Rust unit tests on Windows and Linux, builds a Windows x64 executable, and uploads it as a workflow artifact. The unit tests cover event ordering, stale pairs, debouncing, and statistical percentiles.

No Windows hardware measurement has been performed in the authoring environment yet. Windows CI confirms compilation, but actual device comparisons must be performed on a Windows PC.
