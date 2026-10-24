//! Win32 Raw Input capture. Callbacks only record timestamps, never print to console.

use crate::core::{Session, PAIR_WINDOW_MS};
use std::{
    cell::RefCell,
    ffi::c_void,
    io,
    mem::{size_of, zeroed},
    path::Path,
    ptr::{null, null_mut},
};

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;
type Wparam = usize;
type Lparam = isize;
type Lresult = isize;

const WM_INPUT: u32 = 0x00FF;
const RID_INPUT: u32 = 0x1000_0003;
const RIM_TYPEMOUSE: u32 = 0;
const RIM_TYPEKEYBOARD: u32 = 1;
const RIDEV_INPUTSINK: u32 = 0x0000_0100;
const RI_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x0001;
const RI_MOUSE_LEFT_BUTTON_UP: u16 = 0x0002;
const RI_KEY_BREAK: u16 = 0x0001;
const VK_SPACE: u16 = 0x20;
const VK_F11: u16 = 0x7A;
const VK_F12: u16 = 0x7B;

#[repr(C)]
struct WndClassW {
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: Hinstance,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

#[repr(C)]
struct RawInputDevice {
    usage_page: u16,
    usage: u16,
    flags: u32,
    hwnd_target: Hwnd,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawInputHeader {
    kind: u32,
    size: u32,
    device: *mut c_void,
    wparam: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawMouse {
    flags: u16,
    padding: u16,
    buttons: u32,
    raw_buttons: u32,
    last_x: i32,
    last_y: i32,
    extra_information: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawKeyboard {
    make_code: u16,
    flags: u16,
    reserved: u16,
    virtual_key: u16,
    message: u32,
    extra_information: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
union RawInputData {
    mouse: RawMouse,
    keyboard: RawKeyboard,
    storage: [u64; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawInput {
    header: RawInputHeader,
    data: RawInputData,
}

#[repr(C)]
#[derive(Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    wparam: Wparam,
    lparam: Lparam,
    time: u32,
    point: Point,
    private_data: u32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(class: *const WndClassW) -> u16;
    fn CreateWindowExW(
        ex_style: u32, class_name: *const u16, window_name: *const u16, style: u32,
        x: i32, y: i32, width: i32, height: i32,
        parent: Hwnd, menu: *mut c_void, instance: Hinstance, param: *const c_void,
    ) -> Hwnd;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
    fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> Lresult;
    fn PostQuitMessage(exit_code: i32);
    fn RegisterRawInputDevices(devices: *const RawInputDevice, count: u32, size: u32) -> i32;
    fn GetRawInputData(handle: *mut c_void, command: u32, data: *mut c_void,
                       bytes: *mut u32, header_size: u32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hinstance;
    fn QueryPerformanceCounter(value: *mut i64) -> i32;
    fn QueryPerformanceFrequency(value: *mut i64) -> i32;
}

struct Capture {
    session: Session,
    summary_requested: bool,
}

thread_local! {
    static CAPTURE: RefCell<Option<Capture>> = RefCell::new(None);
}

/// SAFETY: Windows invokes this on the same thread that owns the hidden window.
/// No I/O or complex processing occurs until the message has been dispatched.
unsafe extern "system" fn window_proc(
    hwnd: Hwnd, message: u32, wparam: Wparam, lparam: Lparam,
) -> Lresult {
    if message == WM_INPUT {
        let mut ticks = 0_i64;
        if QueryPerformanceCounter(&mut ticks) != 0 {
            let mut input: RawInput = zeroed();
            let mut byte_count = size_of::<RawInput>() as u32;
            let got = GetRawInputData(
                lparam as *mut c_void, RID_INPUT,
                (&mut input as *mut RawInput).cast::<c_void>(),
                &mut byte_count, size_of::<RawInputHeader>() as u32,
            );
            if got != u32::MAX {
                CAPTURE.with(|cell| {
                    if let Some(capture) = cell.borrow_mut().as_mut() {
                        match input.header.kind {
                            RIM_TYPEMOUSE if got as usize >= size_of::<RawInputHeader>() + size_of::<RawMouse>() => {
                                let flags = input.data.mouse.buttons as u16;
                                if flags & RI_MOUSE_LEFT_BUTTON_UP != 0 {
                                    capture.session.mouse_button(false, ticks);
                                }
                                if flags & RI_MOUSE_LEFT_BUTTON_DOWN != 0 {
                                    capture.session.mouse_button(true, ticks);
                                }
                            }
                            RIM_TYPEKEYBOARD if got as usize >= size_of::<RawInputHeader>() + size_of::<RawKeyboard>() => {
                                let key = input.data.keyboard;
                                let down = key.flags & RI_KEY_BREAK == 0;
                                match key.virtual_key {
                                    VK_SPACE => capture.session.space_key(down, ticks),
                                    VK_F11 if down => capture.summary_requested = true,
                                    VK_F12 if down => PostQuitMessage(0),
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
        }
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // This implementation and its Win32 structure checks target standard x64 Windows.
    if size_of::<usize>() != 8
        || size_of::<RawInputDevice>() != 16
        || size_of::<RawInputHeader>() != 24
        || size_of::<RawMouse>() != 24
        || size_of::<RawInput>() != 48
    {
        return Err("Unsupported Windows ABI layout; build for x86_64-pc-windows-msvc".into());
    }

    let (mut start, mut frequency) = (0_i64, 0_i64);
    unsafe {
        if QueryPerformanceFrequency(&mut frequency) == 0 || frequency <= 0 {
            return Err(io::Error::last_os_error().into());
        }
        if QueryPerformanceCounter(&mut start) == 0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    CAPTURE.with(|c| *c.borrow_mut() = Some(Capture {
        session: Session::new(start, frequency),
        summary_requested: false,
    }));

    let class_name: Vec<u16> = "MouseKeyboardTimingHiddenWindow\0".encode_utf16().collect();
    let window_name: Vec<u16> = "Mouse keyboard timing\0".encode_utf16().collect();

    unsafe {
        let instance = GetModuleHandleW(null());
        if instance.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let class = WndClassW {
            style: 0,
            wnd_proc: Some(window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: null_mut(),
            cursor: null_mut(),
            background: null_mut(),
            menu_name: null(),
            class_name: class_name.as_ptr(),
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error().into());
        }
        let hwnd = CreateWindowExW(
            0, class_name.as_ptr(), window_name.as_ptr(),
            0, 0, 0, 0, 0,
            null_mut(), null_mut(), instance, null(),
        );
        if hwnd.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let devices = [
            RawInputDevice {
                usage_page: 0x01, usage: 0x02,
                flags: RIDEV_INPUTSINK, hwnd_target: hwnd,
            },
            RawInputDevice {
                usage_page: 0x01, usage: 0x06,
                flags: RIDEV_INPUTSINK, hwnd_target: hwnd,
            },
        ];
        if RegisterRawInputDevices(
            devices.as_ptr(), devices.len() as u32, size_of::<RawInputDevice>() as u32
        ) == 0 {
            let error = io::Error::last_os_error();
            DestroyWindow(hwnd);
            return Err(error.into());
        }

        println!("======================================================");
        println!("     MOUSE CLICK vs KEYBOARD SPACE - RAW INPUT");
        println!("======================================================");
        println!("QPC frequency: {frequency} ticks/second");
        println!("Press LEFT CLICK and SPACE together, then release both.");
        println!("Repeat 30-100+ times. F11: interim stats. F12: save CSV and exit.");
        println!("Accepts pairs within {PAIR_WINDOW_MS:.0} ms. No output during capture.");
        println!("Measures arrival to this app, NOT physical switch-to-PC latency.");
        println!("A simultaneous actuator is needed to compare actual device latency.\n");

        let mut msg: Msg = zeroed();
        loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result == -1 {
                let error = io::Error::last_os_error();
                DestroyWindow(hwnd);
                return Err(error.into());
            }
            if result == 0 {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
            CAPTURE.with(|c| {
                if let Some(capture) = c.borrow_mut().as_mut() {
                    if capture.summary_requested {
                        capture.summary_requested = false;
                        capture.session.print_summary();
                    }
                }
            });
        }
        DestroyWindow(hwnd);
    }

    CAPTURE.with(|c| -> io::Result<()> {
        let Some(capture) = c.borrow_mut().take() else {
            return Ok(());
        };
        capture.session.print_summary();
        let path = Path::new("input_timing_results.csv");
        capture.session.save_csv(path)?;
        println!("Saved {} trials to {}", capture.session.trial_count(), path.display());
        Ok(())
    })?;
    Ok(())
}
