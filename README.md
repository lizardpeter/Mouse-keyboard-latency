# Mouse / Keyboard Input Timing Lab

A native **Windows x64 GUI** built in Rust, with four interactive diagnostic modes. All results appear directly inside the application. CSV export is optional and **exports the currently selected mode**, including the complete keyboard event history.

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

## Collect one diagnostic report to investigate keyboard vs Windows

**The one file to send for analysis is `keyboard_diagnostic_report.txt`.**

1. Download and launch the latest Windows build from GitHub Actions.
2. Open **2. Keyboard Diagnostics**, then click **Reset mode**.
3. Try **individual keys**: press and fully release 20–30 times, including keys that feel unreliable.
4. Try **several keys simultaneously**, then release every key. Watch whether the onscreen held list clears.
5. Reproduce the suspected stuck-key / delayed-key event. After your fingers are off the keys, keep diagnostics running for at least **5 seconds**. This gives the monitor time to flag prolonged raw-vs-Windows state differences.
6. While the anomaly is visible, click **Save Diagnostic Report**, confirm the privacy warning, and save the proposed `keyboard_diagnostic_report.txt` file. You may also save it if the issue did not occur—that's still useful negative evidence.
7. **Read the file before sending it**. It records names of keys pressed, which can reveal typed content. Only use test inputs; avoid typing passwords, private messages or sensitive data while this mode is running. Then attach the report for analysis.

The text report includes:
- Windows version and QPC counter frequency;
- overall key-down / key-up / repeat / unmatched-release totals;
- the exact keys reported held *at the moment you clicked Save Diagnostic Report*;
- time held, hold durations, queue delay estimates, and simultaneous-key counts;
- **Raw Input versus `GetAsyncKeyState`** comparisons for keys that were observed during the session, with sustained (500+ ms) disagreements;
- **every recorded diagnostic key event** (subject to a 250,000-event safety cap) with timestamps;
- a place to record what you physically observed.

**Caveat:** Windows asynchronous key state and Raw Input both depend on the operating system; they are not independent physical switch measurements. Key state polling may be unavailable or misleading on a secure desktop. A normal report also cannot guarantee that the keyboard hardware has no fault.

## Mode 4: Live Typing / Character Burst Test

This mode is designed for symptoms such as delayed text appearing in **bursts** in Chrome or Notepad.

1. Select **4. Typing Test**. The focus automatically moves to the large typing box.
2. Click **Reset mode** and type test text at a normal pace, then type quickly and release each key. Try repeated sentences such as `the quick brown fox jumps over the lazy dog`.
3. Watch the **actual characters rendered by the Windows native EDIT control**, not guesses from scan codes. Under the box, live timing statistics show how far apart the textbox's **WM_CHAR** messages arrived and whether 3 or more consecutive character messages were handled less than 5 ms apart.
4. Compare the **Windows WM_CHAR** and **Raw Input** timelines shown underneath. The event time is when our GUI handles the messages, not an independently measured keyboard switch timestamp.
5. When typing feels delayed or appears all at once, do **not reset**; click **Save Diagnostic Report** and attach `keyboard_diagnostic_report.txt` for analysis. The report contains the test box's actual text, interleaved timestamps, key-down/up/repeats, and Windows key state readings. **Review before sharing.**
6. **Export CSV** in Typing Test saves the interleaved event timeline as `typing_event_timeline.csv`.

Important distinction: this tool *does not intercept Chrome/Notepad text output*; that would require hooking or inspecting other processes. Instead it provides an independent test of regular Windows character message delivery in our own focused textbox, with a simultaneous Raw Input stream. Multiple WM_CHAR messages handled close together can be caused by batching, fast software processing, IME or other mechanisms; a short gap alone does not prove a keyboard defect. Pasted text or IME composition changes do not necessarily produce one WM_CHAR per visible character.

For privacy, type **only artificial test sentences**, never passwords or sensitive material. This mode does not install any system-wide keyboard hooks, modify Windows settings, or send input records to a network service. The report and CSV are only created if you choose to save them.

## Mode 3: Mouse Reaction Time

1. Select **Mouse Reaction Time**, and click **Start trial**.
2. The large colored panel tells you to **WAIT**. An unpredictable delay of about **1.5–4 seconds** follows.
3. When the panel changes to **GREEN**, left-click inside it as quickly as possible.
4. Your response time immediately appears in **milliseconds**. The app also keeps a running average, best score, trial history, and false-start count.
5. Repeat as desired, or click **Reset mode**.

Clicking before green is a **false start** and does not create a valid time. Results are timed using `QueryPerformanceCounter` after the drawing submission of the green frame and the captured Raw Input mouse event. Physical monitor scan-out, pixel response and USB/mouse hardware latency are **not** separately measured, so times are display-submission-to-application estimates rather than lab-calibrated reaction times.

## Controls and privacy

- The app starts capturing immediately. **Pause** and **Reset mode** apply to the currently selected test. `F5` toggles pause in the comparison and reaction modes.
- **Export CSV** saves the **currently selected mode**, not just comparison trials. The Save dialog proposes a mode-specific filename:
  - **Input Comparison** → `mouse_keyboard_results.csv`: matched mouse/keyboard trials.
  - **Keyboard Diagnostics** → `keyboard_diagnostics.csv`: **all captured key events** (not merely the 14 shown on screen), per-event scan codes, key names, down/up/repeat status, held durations, concurrent held-key counts and Windows queue-age estimates, a snapshot of any still-held keys, and aggregate anomaly statistics.
  - **Reaction Time** → `reaction_times.csv`: each successful trial and summary statistics including false starts.
  - **Typing Test** → `typing_event_timeline.csv`: per-message WM_CHAR and Raw Input event times and gaps.
- Canceling export leaves results intact. The keyboard capture state is resynchronized after the Save dialog closes to avoid false stuck-key alerts.
- Input data remains on your PC and is not transmitted anywhere.
- No kernel drivers or background service.

## Development

Run `cargo test --all-targets` to execute platform-independent diagnostics/reaction/measurement logic tests. On Windows, `cargo run --release` launches the GUI. Every push to `main` triggers the Windows GitHub Actions release build and produces a downloadable executable.

**Compatibility:** Windows 10/11 x64. All modes capture Windows Raw Input data, not an electrical signal from the switches.
