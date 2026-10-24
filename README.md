# Mouse / Keyboard Input Timing Lab

A native **Windows x64 GUI** built in Rust, with three interactive diagnostic modes. All results appear directly inside the application; CSV export is strictly optional.

## Download the executable (no Rust required)

1. Open [GitHub Actions — Build Windows GUI](../../actions/workflows/ci.yml).
2. Select the newest **successful** run for `main`.
3. Under **Artifacts**, download `Mouse-Keyboard-Latency-Windows-GUI`.
4. Extract the ZIP and double-click `Mouse-Keyboard-Latency.exe`.

The Windows build runs automatically when code is pushed to `main`. It produces a normal desktop GUI `.exe`, not a console program. No drivers, extra DLLs, or Rust installation are needed to run the packaged application.

## Mode 1: Input Comparison

- Tracks left mouse button and Space bar arrivals continuously through Windows Raw Input.
- Shows **mouse presses**, **Space presses**, the most recent timestamps and times since the latest events.
- Pairs events within 300 ms and reports mouse-first / keyboard-first, signed timing difference, median gap, mean, P95, and a live recent-trial history.
- Positive (keyboard arrival minus mouse arrival) means **mouse first**; negative means **keyboard first**.

## Mode 2: Keyboard Diagnostics

Use this mode when the keyboard seems to repeat, miss releases, stick, or have key-rollover problems. Press single keys and try pressing several keys together, then releasing all of them.

The display shows:

- Key-down, key-up and repeat counts.
- **Every key Windows currently considers held** and how long it has been held (including simultaneous keys).
- **Potential stuck-key warnings** for keys reporting held longer than 3 seconds.
- Orphan/unmatched key-up events, which may be caused by a missing preceding press, an event arriving outside the observation window, or a mode switch.
- Maximum simultaneously held keys reported by Windows, useful for checking rollover.
- Last press duration, longest held duration, shortest inter-event gap and a live event timeline.
- **Approximate Windows Raw Input message-queue age**, including worst observed delay and number of arrivals with 32 ms or more of queue delay.

**Interpret alerts as observations, not definitive proof of hardware defects.** Holding a key intentionally triggers a long-held warning. A software observer cannot know that a physical key was pressed if the keyboard never sends an event, nor can it measure physical switch-to-USB latency without an external reference. Raw Input message queue age is an approximate Windows-side scheduling measurement, not hardware input latency.

Pausing and changing modes clear tracked held states so a key released outside the observation window will not incorrectly stay flagged forever. Reset mode also clears the diagnostic counts.

## Mode 3: Mouse Reaction Time

1. Select **Mouse Reaction Time**, and click **Start trial**.
2. The large colored panel tells you to **WAIT**. An unpredictable delay of about **1.5–4 seconds** follows.
3. When the panel changes to **GREEN**, left-click inside it as quickly as possible.
4. Your response time immediately appears in **milliseconds**. The app also keeps a running average, best score, trial history, and false-start count.
5. Repeat as desired, or click **Reset mode**.

Clicking before green is a **false start** and does not create a valid time. Results are timed using `QueryPerformanceCounter` after the drawing submission of the green frame and the captured Raw Input mouse event. Physical monitor scan-out, pixel response and USB/mouse hardware latency are **not** separately measured, so times are display-submission-to-application estimates rather than lab-calibrated reaction times.

## Controls and privacy

- The app starts capturing immediately. **Pause** and **Reset mode** apply to the currently selected test. `F5` toggles pause in the comparison and reaction modes.
- **Export CSV** preserves the input-comparison pairs only, and is never required for live results.
- Input data remains on your PC and is not transmitted anywhere.
- No kernel drivers or background service.

## Development

Run `cargo test --all-targets` to execute platform-independent diagnostics/reaction/measurement logic tests. On Windows, `cargo run --release` launches the GUI. Every push to `main` triggers the Windows GitHub Actions release build and produces a downloadable executable.

**Compatibility:** Windows 10/11 x64. All modes capture Windows Raw Input data, not an electrical signal from the switches.
