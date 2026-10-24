# Mouse / Keyboard Latency — Live Windows GUI

A standalone Windows desktop application, written in Rust, showing **mouse and keyboard Raw Input activity live inside the window**. No CSV export or Rust installation is required to see results.

## Download the Windows application

1. Open **[Build Windows GUI on GitHub Actions](../../actions/workflows/ci.yml)**.
2. Open the latest successful workflow run on **main**.
3. Download the artifact **Mouse-Keyboard-Latency-Windows-GUI**.
4. Extract its ZIP and launch **Mouse-Keyboard-Latency.exe** directly.

The app runs as a normal Windows x64 GUI, not a command prompt. GitHub Actions builds a fresh executable automatically on each push to main.

## Live results (automatic)

**Capture begins as soon as the app launches.** Left-click the mouse and/or press **Space** on the keyboard to see two separate panels update automatically, even if only one input device is used.

**Mouse live panel**
- Total left-button presses
- Most recent event arrival time measured since the recording began
- How recently the last click occurred
- Time interval between the last two mouse clicks

**Keyboard live panel**
- Total Space presses
- Most recent event arrival time measured since the recording began
- How recently the last Space press occurred
- Time interval between the last two Space presses

**Live comparison panel**
- Total paired events (left-click + Space within 300 ms)
- Mouse-first vs keyboard-first count
- Median absolute arrival-time difference
- Latest pair's signed arrival difference
- Mean signed difference, 95th percentile separation, and recent paired measurements

**Positive** signed gap means the mouse arrived first; **negative** means keyboard first. Event data is timestamped on receipt, while the screen updates approximately every 50 ms (20 times per second). No file save is needed to see these metrics.

**Controls:** **Pause**, **Reset**, and **Export CSV** (optional). **F5** also pauses/resumes if this app has focus. Click outside the toolbar to count mouse clicks. Raw Input works even if the app is not focused, so you can test while using another window.

## Accurate interpretation

This program compares event arrival time **at the Windows application**, not the independent absolute hardware input latency of a mouse or keyboard. An isolated mouse click has no known physical starting timestamp, so it is scientifically impossible to derive the mouse's hardware latency simply from the Windows event timestamp. The per-device press intervals displayed above are **time between separate presses**, not an input-lag measurement.

The relative timing between two manually pressed devices also includes human finger timing. An externally synchronized actuator is required to meaningfully compare hardware delay; an external timestamped sensor/reference is required for absolute latency.

The application requires no kernel driver and sends no input data over the network. CSV export only happens when you select it.

## Development

Build locally on Windows x64 with `cargo run --release`. Run pairing and device-timing unit tests with `cargo test`. This application uses only native Windows APIs with no third-party Rust dependencies. GitHub Actions runs tests on Linux and Windows, compiles the release GUI, and uploads it to the workflow run.
