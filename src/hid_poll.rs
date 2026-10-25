//! Explicit, *read-only* Windows HID-class GET request, never a SET request.
//! Important: GET_POLL_FREQUENCY_MSEC describes a HID polled collection when
//! implemented. It is NOT necessarily USB interrupt endpoint bInterval,
//! especially for USB 2.0 high-speed 2000/4000/8000 Hz devices.
use std::ffi::c_void;
use std::ptr::{null,null_mut};
type H=*mut c_void;
const RIDI_DEVICENAME:u32=0x2000_0007;
const IOCTL_HID_GET_POLL_FREQUENCY_MSEC:u32=0x000B_0198;
const OPEN_EXISTING:u32=3;
const FILE_SHARE_READ:u32=1;const FILE_SHARE_WRITE:u32=2;
const INVALID_HANDLE_VALUE:H=(-1isize) as H;
#[link(name="user32")]
unsafe extern "system" {
    fn GetRawInputDeviceInfoW(device:H,command:u32,out:H,size:*mut u32)->u32;
}
#[link(name="kernel32")]
unsafe extern "system"{
    fn CreateFileW(name:*const u16,access:u32,share:u32,security:H,
        creation:u32,flags:u32,template:H)->H;
    fn DeviceIoControl(file:H,control:u32,in_buf:H,in_size:u32,
        out_buf:H,out_size:u32,returned:*mut u32,overlapped:H)->i32;
    fn CloseHandle(handle:H)->i32;
}
pub fn get_hid_polled_interval_ms(raw_handle:usize)->Option<u32> {
    // Raw Input hDevice -> device path. Avoid storing or exporting the path;
    // it can contain vendor identifiers and potentially unique serial data.
    let mut count=0u32;
    let rc=unsafe{GetRawInputDeviceInfoW(raw_handle as H,RIDI_DEVICENAME,
        null_mut(),&mut count)};
    if rc==u32::MAX||count==0||count>8192{return None}
    let mut path=vec![0u16;count as usize];
    let n=unsafe{GetRawInputDeviceInfoW(raw_handle as H,RIDI_DEVICENAME,
        path.as_mut_ptr().cast(),&mut count)};
    if n==u32::MAX{return None}
    let end=path.iter().position(|&ch|ch==0).unwrap_or(path.len());
    path.truncate(end);
    path.push(0);
    // Zero desired access: this is a read-only metadata query. If the driver
    // refuses access, do NOT retry with GENERIC_READ or GENERIC_WRITE.
    let file=unsafe{CreateFileW(path.as_ptr(),0,FILE_SHARE_READ|FILE_SHARE_WRITE,
        null_mut(),OPEN_EXISTING,0,null_mut())};
    if file==INVALID_HANDLE_VALUE||file.is_null(){return None}
    let mut interval=0u32;
    let mut returned=0u32;
    let ok=unsafe{DeviceIoControl(file,IOCTL_HID_GET_POLL_FREQUENCY_MSEC,
        null_mut(),0,(&mut interval as *mut u32).cast(),4,
        &mut returned,null_mut())};
    unsafe{CloseHandle(file);}
    if ok==0||returned<4||interval==0||interval>1000{return None}
    Some(interval)
}
