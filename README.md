# Mouse / Keyboard Input Timing Lab

A native **Windows x64 GUI** built in Rust, with six interactive diagnostic modes and an optional native Vulkan scene. All results appear directly inside the application, including a zoomable in-memory anomaly timeline. CSV export is optional and **exports the currently selected mode**, including the complete keyboard event history.

## Download the executable (no Rust required)

1. Open [GitHub Actions — Build Windows GUI](../../actions/workflows/ci.yml).
2. Select the newest **successful** run for `main`.
3. Under **Artifacts**, download `Mouse-Keyboard-Latency-Windows-GUI`.
4. Extract the ZIP and double-click `Mouse-Keyboard-Latency.exe`.

The Windows build runs automatically when code is pushed to `main`. It produces a normal desktop GUI `.exe`, not a console program. No driver installation, Rust installation, or Vulkan SDK is required. The optional Vulkan scene dynamically loads the Vulkan runtime provided by the installed graphics driver; if unavailable, the original Win32 diagnostics still run.

## Mode 6: Flight Recorder — anomaly capture and timeline

**The new flight recorder is strictly passive and in-memory.** It keeps the most recent **10 seconds** of input events while the application is recording in **Keyboard (2)**, **Typing (4)**, or **Mouse (5)**. It automatically marks and snapshots a time window when it sees one of:

- **Raw repeat burst:** four repeat keydown events for the same key/device with three successive gaps under 5 ms. Normal 33 ms typematic repeats should not trigger this.
- **Character burst:** at least three `WM_CHAR` events with two consecutive inter-character gaps under 5 ms in the test box.
- **Approximate queue-age alert:** at least 48 ms apparent Raw Input queue age (the Windows tick-based estimate is coarse).
- **GUI heartbeat stall:** the application's own GUI-message callbacks stopped publishing a heartbeat for at least 200 ms, as observed by a separate in-process watchdog thread.

The recorder copies up to 10 seconds **before** the trigger and collects up to **5 seconds after** it. Press the global **Mark Now** button while in an active diagnostic mode to create a manual snapshot even if no automatic anomaly is detected. Remain in that mode for five seconds after marking to collect the full post-trigger portion, then select **6. Flight** to inspect.

The Flight screen includes a native Win32 **six-track density timeline**: Raw keyboard, Win32 key messages, characters, mouse reports, watchdog gaps, and anomaly markers. **Zoom + / Zoom -** change the visible span; **Older / Newer** pan through history. Each column represents a small time range; taller bars mean more events in that column. The UI never pretends that application-dispatch timestamps are physical USB report or switch timestamps.

Select **Save Flight Report** or **Export CSV** to write the explicitly chosen file. Without that action, captures remain only in memory. **Reset mode** on Flight discards the current buffered recording; switching diagnostic modes does not silently transfer keyboard history into ordinary comparison/reaction modes. If you switch to Flight before post-roll completes, it freezes early and cannot recover the remaining post-event time.

**Recording privacy:** Raw Input can be delivered to this process while it is not focused when one of the diagnostic modes is selected. Use **test keys only**, never passwords or private chat messages during keyboard/typing sessions. The Flight Recorder saves key identifiers and device handles, *not typed WM_CHAR text*, but those identifiers may still reveal typing. Review exported files before sharing.

**Watchdog limitations:** The watchdog uses a 10 ms sleep in a normal user-mode thread and an atomic heartbeat; it does not set global timer resolution, load a driver, register hooks or create ETW sessions. A long gap indicates that the app's GUI callbacks were not observed promptly; it does **not** establish that USB delivery or the Windows kernel was stalled. An overloaded system may also delay the watchdog itself. The watchdog is enabled only while recording diagnostic modes and is suspended around the application's modal Save dialogs.

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

## Polling-rate analysis: what reviewers' tools actually measure

This release takes inspiration from two public codebases: [microe1's MouseTester](https://github.com/microe1/MouseTester) (Win32 Raw Input and QPC) and [ClickSyncMouseTester](https://github.com/Nuitfanee/ClickSyncMouseTester) (Rust, per-device windows, and motion-vs-packet analysis). The implementation here is original.

**Three values must never be confused:**

1. **USB endpoint-advertised polling interval:** the `bInterval` from a USB interrupt-IN endpoint configuration descriptor, interpreted at the device's USB speed. This is the firmware-advertised service interval. **Input Timing Lab does not automatically retrieve it**, because Microsoft USBView's descriptor query walks host/hub IOCTLs, typically opening hub handles with write access; we deliberately refuse that under our strict zero-access policy. Read it independently using [Microsoft's USBView](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/usbview) if desired.
2. **HID class polled-collection interval:** an optional Windows `IOCTL_HID_GET_POLL_FREQUENCY_MSEC` result. Clicking **Read HID interval (GET)** opens the device with `dwDesiredAccess=0` and submits **only** the documented GET command. An unsupported/denied request returns **unavailable**. A 1 ms result does not certify a USB endpoint polls at 1000 Hz, and the API's millisecond integer cannot accurately express 8000 Hz.
3. **Host-delivered effective event rate:** per-device QPC timestamps from `WM_INPUT`. The mouse screen now displays independent estimates per `hDevice`: median-derived motion-event rate, actual last-1-second motion event count, median/P95/P99 motion intervals, and all-Raw-report spacing. Motion output depends on continuous physical movement and can differ from USB service intervals, particularly on high-report-rate mice.

Keyboard repeats (**typematic** events) are explicitly counted separately from genuine down/up transitions. The keyboard view shows per-device key transitions, Windows repeat counts, and raw-message gap statistics. **No Windows user-mode Raw Input algorithm can recover a keyboard's 1000/8000 Hz USB polling rate merely by holding a key**; it receives changes and autorepeats, not every empty USB scan. Keyboard USB polling interval requires actual USB endpoint descriptors or external bus instrumentation.

The optional HID metadata query never changes the polling configuration; there is **no SET IOCTL** in the app, no system-file or registry write, and no background scan. Measurements and device paths remain in memory; full device paths and serial numbers are not exported. Saved reports are created only after you explicitly choose a path.

## Mode 5: Mouse Polling / Raw vs Windows Pointer

Move the mouse continuously while keeping **5. Mouse** selected. The app records Windows `WM_INPUT` movement events, their QPC inter-arrival intervals, raw X/Y counts, mouse-button flags, device handles and the Windows cursor coordinates sampled at dispatch. It displays **per-mouse** median-derived motion event rate, latest 1-second count, median/P95/P99 intervals, and a live event timeline. The HID GET-only metadata button is optional. You can export the observations as CSV or save a report.

**This is not an absolute measurement of the mouse's USB polling rate.** Idle sensors send fewer movement reports, multiple devices can be present, high-frequency arrivals can be batched, and cursor coordinates are affected by pointer settings and DWM. Raw motion counts and cursor pixels cannot be subtracted to calculate input delay.

## Passive Deep Keyboard Trace

While the application is recording, it also captures the `WM_KEYDOWN`/`WM_KEYUP` and `WM_SYSKEYDOWN`/`WM_SYSKEYUP` messages addressed to its own main window or focused test text box, along with:

- QPC event-handling timestamps, approximate `GetMessageTime` age, queued `QS_KEY`/`QS_RAWINPUT` flags and per-message `lParam` repeat count / previous-key-state bits.
- Matching Win32 Raw Input keyboard and mouse events, raw scan codes, device handles, mouse deltas, mouse buttons (useful as an **independent physical action marker**) and current Windows cursor samples.
- Up to 100,000 preallocated trace records in memory. The report includes the complete retained timeline.
- Read-only device identity lookup at report time, exported as **VID/PID only** so keyboard/mouse serial-number paths are not shared.

Observe keyboard message data **only within our own focused app**. There is no mechanism to inspect arbitrary Chrome/CS2 message queues or kernel HID driver buffers from this normal user-mode application. The Raw Input and queue observations cannot time the electrical actuation or switch release.

**Important: read-only policy.** The executable never edits the registry, Windows files, drivers, services, USB controller settings, keyboard-repeat configuration, GPU settings, or power settings. It does not start/stop ETW sessions or install a USB filter. Raw Input registration is process-scoped and stops when the process exits. Exporting CSV/reports writes **only the file you explicitly select** in a Save dialog, not a system file. Graphics drivers may independently maintain their own runtime caches; the application does not write them.

We deliberately **do not auto-start USB ETW traces**, because enabling an ETW session changes live system tracing state and writes an ETL recording, contrary to strict read-only operation. Analysis of USB reports requires an external, explicitly user-authorized capture or hardware analyzer.

## Native Vulkan Game-style Reaction and Mouse Scene

Click **Open Vulkan Scene** in the main program. It launches a temporary, independent copy of the executable using `--vulkan-scene`, so registering mouse Raw Input for the Vulkan window does not steal registration from the main diagnostic application. The scene renders with **native Vulkan through ash**, not DirectX, OpenGL, GDI, or an embedded browser.

- Native Win32 window and fullscreen-style GPU-rendered color scene with a centered target.
- Press **R** for reaction test, **SPACE** to arm, wait for the **green target/background**, then left-click. Early clicks count as false starts. The estimated reaction time starts just after the green frame's Vulkan `queue_present` submission; actual photon emission/scan-out is **not measured**.
- Press **M** for the mouse motion scene: **cyan** shows raw-count integration, **orange** the sampled Windows cursor, and a central target supplies spatial reference. These two markers use different units/acceleration behavior.
- Press **F1**=60 FPS, **F2**=120, **F3**=144, **F4**=240, **F5**=360, **F6**=500, **F7**=1000, **F8**=uncapped, **ESC**=close. Current FPS and approximate mouse movement-event frequency are shown in the window title.
- Uses `VK_PRESENT_MODE_IMMEDIATE_KHR` **only** (no silent V-sync fallback), one CPU/GPU frame in flight and no shader compilation in the hot path. Vulkan still requires **at least two swapchain images**; Windows composition, driver queueing, and the monitor's scan-out timing cannot be forced to zero. Unsupported immediate presentation causes an explanatory error rather than silently enabling V-sync.

This new scene is experimental and has not been validated with physical photodiode measurements. The standard Win32 reaction test remains available.

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
