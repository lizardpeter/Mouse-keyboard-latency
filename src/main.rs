#![cfg_attr(windows, windows_subsystem = "windows")]

mod core;
mod keyboard;
mod reaction;
mod typing;
mod deep_trace;
mod mouse_diagnostics;
#[cfg(windows)] mod vulkan_scene;
#[cfg(windows)] mod windows;

#[cfg(windows)]
fn main() {
    let vulkan=std::env::args_os().any(|a|a=="--vulkan-scene");
    let result=if vulkan{vulkan_scene::run()}else{windows::run()};
    if let Err(error)=result {
        if vulkan {
            #[link(name="user32")]
            unsafe extern "system" {fn MessageBoxW(hwnd:*mut std::ffi::c_void,text:*const u16,title:*const u16,flags:u32)->i32;}
            let msg=format!("Unable to start or continue Vulkan scene:\n{error}");
            let wide=|s:&str|s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<_>>();
            unsafe{MessageBoxW(std::ptr::null_mut(),wide(&msg).as_ptr(),wide("Input Timing Lab").as_ptr(),0x10);}
        }
        eprintln!("GUI error: {error}");
        std::process::exit(1);
    }
}
#[cfg(not(windows))]
fn main() { eprintln!("Windows x64 desktop application. Cross-platform diagnostic tests are available."); }
