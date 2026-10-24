# Mouse / Keyboard Latency — Windows Desktop App

A standalone, native Windows x64 desktop application written in Rust. It compares **when mouse left clicks and Space keyboard presses arrive in Windows Raw Input**.

## Download and run (no Rust required)

1. Open the [Build Windows GUI workflow](../../actions/workflows/ci.yml).
2. Open the most recent successful run on `main`.
3. Under **Artifacts**, download `Mouse-Keyboard-Latency-Windows-GUI`.
4. Extract the ZIP and **double-click `Mouse-Keyboard-Latency.exe`**.

GitHub Actions automatically runs the tests and compiles the Windows release executable whenever `main` is updated. This is a standard GUI application: it does **not** open a command prompt. No driver, service, or Rust installation is required on the receiving computer.

## Use

- Click **Start recording** to begin listening to mouse and keyboard Raw Input.
- Left click and press **Space** close together, then release both. Repeat for 30–100+ samples.
- The dashboard shows completed pairs, mouse-first and keyboard-first totals, median absolute separation, mean signed separation, 95th percentile gap and the 9 latest pairs.
- Click **Pause**, **Reset**, or **Save CSV...** at any time. CSV export uses the normal Windows Save As dialog. **F5** also toggles recording while the window has keyboard focus.
- A positive signed difference means the mouse was observed first. A negative difference means the keyboard was observed first.
- Clicks on the GUI toolbar are ignored. Click elsewhere in the window or outside the window to take measurements. Background Raw Input registration permits capture even when another app is in focus.
- Pairs farther apart than 300 ms are excluded. Held-button autorepeats are excluded.

## Measurement limitation

**Results are differences between input arrival times at this application, not actual hardware latency.** Clicking two different physical devices by hand introduces uncertain finger timing, and Windows message scheduling and USB polling affect the timestamps. To compare real hardware input lag, actuate both devices with a common physical trigger. To measure absolute latency, include an external reference for the activation time.

Events stay on the PC. Data is written only when you click **Save CSV...**. No device driver or network connection is used.

## Development

Build locally on Windows x64 with `cargo run --release`. Run cross-platform pairing/statistics tests with `cargo test`. The native GUI deliberately uses only Win32/GDI, so the Rust crate has no external dependencies.
