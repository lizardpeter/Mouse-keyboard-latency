//! Native Win32 GUI and Windows Raw Input capture. No external dependencies.
//! All UI events and Raw Input callbacks run on the owning GUI thread.

use crate::core::{Kind, Session, DeviceSnapshot};
use crate::keyboard::{KeyboardDiagnostics,KeyId,OsKeyStateMonitor};
use crate::reaction::{ReactionTest,Phase};
use crate::typing::TypingMonitor;
use crate::deep_trace::{DeepTrace,TraceEvent};
use crate::mouse_diagnostics::MouseDiagnostics;
use crate::polling::PollingMonitor;
use crate::flight::{FlightRecorder,FlightEvent,Lane,LANES};
use crate::watchdog::Watchdog;
use crate::input_capture::{InputCapture,CapturedInput,Kind as CapturedKind,MAX_DRAIN_PER_PULSE,current_processor};
use crate::thread_metrics::{ThreadMetrics,TimingSample};
use crate::hid_poll::get_hid_polled_interval_ms;
use std::fmt::Write as _;
use std::cell::Cell;
use std::{cell::RefCell, ffi::c_void, io, mem::{size_of, zeroed}, path::Path,
    ptr::{null, null_mut}};

type H = *mut c_void;
type WP = usize;
type LP = isize;

const WM_CREATE:u32=1; const WM_DESTROY:u32=2; const WM_COMMAND:u32=0x111;
const WM_INPUT:u32=0xFF; const WM_TIMER:u32=0x113; const WM_SETFONT:u32=0x30;
const WM_CTLCOLORSTATIC:u32=0x138; const WM_ERASEBKGND:u32=0x14;
const WM_SIZE:u32=5; const WM_KEYDOWN:u32=0x100; const WM_PAINT:u32=0x0F;
const WM_CHAR:u32=0x102; const GWLP_WNDPROC:i32=-4;
const WM_KEYUP:u32=0x101; const WM_SYSKEYDOWN:u32=0x104; const WM_SYSKEYUP:u32=0x105;
const QS_KEY:u32=1; const QS_RAWINPUT:u32=0x400;
const RIDI_DEVICENAME:u32=0x20000007;
const WS_VSCROLL:u32=0x00200000;
const ES_MULTILINE:u32=0x0004; const ES_AUTOVSCROLL:u32=0x0040;
const ES_WANTRETURN:u32=0x1000; const ES_READONLY:u32=0x0800;
const RIDEV_INPUTSINK:u32=0x100; const RID_INPUT:u32=0x10000003;
const RI_MOUSE_LEFT_BUTTON_DOWN:u16=1; const RI_MOUSE_LEFT_BUTTON_UP:u16=2;
const RI_KEY_BREAK:u16=1; const VK_SPACE:u16=0x20; const VK_F5:u16=0x74;
const WS_CHILD:u32=0x40000000; const WS_VISIBLE:u32=0x10000000;
const WS_TABSTOP:u32=0x10000; const WS_BORDER:u32=0x800000;
const WS_CAPTION:u32=0xC00000; const WS_SYSMENU:u32=0x80000;
const WS_MINIMIZEBOX:u32=0x20000; const SS_LEFT:u32=0;
const BS_PUSHBUTTON:u32=0; const SW_SHOW:i32=5;
const ID_START:usize=101; const ID_RESET:usize=102; const ID_SAVE:usize=103;
const ID_DIAGNOSTIC_REPORT:usize=105;
const ID_TYPING_INPUT:usize=306;
const ID_TYPING_TEXTLOG:usize=307; const ID_TYPING_RAWLOG:usize=308;
const ID_MODE_COMPARE:usize=201; const ID_MODE_KEYBOARD:usize=202;
const ID_MODE_REACTION:usize=203; const ID_REACTION_ARM:usize=204;
const ID_MODE_TYPING:usize=205;
const ID_MODE_MOUSE:usize=206; const ID_VULKAN_SCENE:usize=207;
const ID_QUERY_HID:usize=208;
const ID_MODE_FLIGHT:usize=209; const ID_FLIGHT_MARK:usize=210;
const ID_FLIGHT_ZOOM_IN:usize=211; const ID_FLIGHT_ZOOM_OUT:usize=212;
const ID_FLIGHT_BACK:usize=213; const ID_FLIGHT_FORWARD:usize=214;
const FONT_NORMAL:i32=400; const FONT_BOLD:i32=700;
const OFN_OVERWRITEPROMPT:u32=2; const OFN_PATHMUSTEXIST:u32=0x800;
const ERROR_ICON:u32=0x10; const BLACK:u32=0x00251D18;
const BG_COLOR:u32=0x00F8F4F1;
fn wide(s:&str)->Vec<u16>{s.encode_utf16().chain(std::iter::once(0)).collect()}

#[repr(C)]
struct WndClass { style:u32, proc:Option<unsafe extern "system" fn(H,u32,WP,LP)->LP>,
    cls_extra:i32, wnd_extra:i32, instance:H, icon:H, cursor:H, brush:H,
    menu:*const u16, class:*const u16 }
#[repr(C)]
struct RawDevice{page:u16,usage:u16,flags:u32,hwnd:H}
#[repr(C)]
#[derive(Copy,Clone)]
struct RawHeader{kind:u32,size:u32,device:H,wp:usize}
#[repr(C)]
#[derive(Copy,Clone)]
struct RawMouse { flags:u16,padding:u16,buttons:u32,raw_buttons:u32,
    last_x:i32,last_y:i32,extra:u32 }
#[repr(C)]
#[derive(Copy,Clone)]
struct RawKeyboard{make:u16,flags:u16,reserved:u16,key:u16,message:u32,extra:u32}
#[repr(C)]
#[derive(Copy,Clone)]
union RawData{mouse:RawMouse,key:RawKeyboard,padding:[u64;3]}
#[repr(C)]
#[derive(Copy,Clone)]
struct RawInput{header:RawHeader,data:RawData}
#[repr(C)]
#[derive(Default)]
struct Point{x:i32,y:i32}
#[repr(C)]
#[derive(Clone, Copy)]
struct Rect { left:i32,top:i32,right:i32,bottom:i32 }
#[repr(C)]
struct Paint {
    dc:H, erase:i32, region:Rect, restore:i32, update:i32, reserved:[u8;32]
}
#[repr(C)]
struct Msg{hwnd:H,msg:u32,wp:WP,lp:LP,time:u32,point:Point,private:u32}
#[repr(C)]
struct SaveFile {
    size:u32,owner:H,instance:H,filter:*const u16,custom:*mut u16,
    custom_max:u32,filter_index:u32,file:*mut u16,file_max:u32,
    file_title:*mut u16,file_title_max:u32,initial_dir:*const u16,
    title:*const u16,flags:u32,file_offset:u16,file_extension:u16,
    default_ext:*const u16,custom_data:isize,hook:H,template:*const u16,
    reserved:H,reserved_num:u32,flags_ex:u32,
}

#[link(name="user32")]
extern "system" {
    fn RegisterClassW(p:*const WndClass)->u16;
    fn CreateWindowExW(ex:u32,class:*const u16,text:*const u16,style:u32,
        x:i32,y:i32,w:i32,h:i32,parent:H,menu:H,instance:H,param:*const c_void)->H;
    fn DefWindowProcW(hwnd:H,msg:u32,wp:WP,lp:LP)->LP;
    fn GetMessageW(msg:*mut Msg,hwnd:H,min:u32,max:u32)->i32;
    fn TranslateMessage(msg:*const Msg)->i32;
    fn DispatchMessageW(msg:*const Msg)->LP;
    fn DestroyWindow(hwnd:H)->i32;
    fn PostQuitMessage(code:i32);
    fn RegisterRawInputDevices(devices:*const RawDevice,count:u32,size:u32)->i32;
    fn GetRawInputData(handle:H,command:u32,out:H,bytes:*mut u32,header:u32)->u32;
    fn GetRawInputBuffer(data:H,bytes:*mut u32,header:u32)->u32;
    fn SetWindowTextW(hwnd:H,text:*const u16)->i32;
    fn SetWindowLongPtrW(hwnd:H,index:i32,new_ptr:isize)->isize;
    fn CallWindowProcW(prev:isize,hwnd:H,msg:u32,wp:WP,lp:LP)->LP;
    fn GetWindowTextLengthW(hwnd:H)->i32;
    fn GetWindowTextW(hwnd:H,out:*mut u16,length:i32)->i32;
    fn SendMessageW(hwnd:H,message:u32,wp:WP,lp:LP)->LP;
    fn SetTimer(hwnd:H,id:usize,period:u32,callback:H)->usize;
    fn KillTimer(hwnd:H,id:usize)->i32;
    fn SetFocus(hwnd:H)->H;
    fn ShowWindow(hwnd:H,cmd:i32)->i32;
    fn BeginPaint(hwnd:H,paint:*mut Paint)->H;
    fn EndPaint(hwnd:H,paint:*const Paint)->i32;
    fn InvalidateRect(hwnd:H,rect:*const Rect,erase:i32)->i32;
    fn FillRect(dc:H,rect:*const Rect,brush:H)->i32;
    fn DrawTextW(dc:H,text:*const u16,n:i32,rect:*mut Rect,format:u32)->i32;
    fn GetClientRect(hwnd:H,r:*mut Rect)->i32;
    fn UpdateWindow(hwnd:H)->i32;
    fn GetCursorPos(out:*mut Point)->i32;
    fn ScreenToClient(hwnd:H,point:*mut Point)->i32;
    fn MessageBoxW(hwnd:H,text:*const u16,caption:*const u16,flags:u32)->i32;
    fn GetAsyncKeyState(virtual_key:i32)->i16;
    fn GetMessageTime()->i32;
    fn GetQueueStatus(flags:u32)->u32;
    fn GetRawInputDeviceInfoW(device:H,command:u32,out:H,size:*mut u32)->u32;
    fn SetBkMode(dc:H,mode:i32)->i32;
    fn SetTextColor(dc:H,color:u32)->u32;
}
#[link(name="gdi32")]
extern "system" {
    fn CreateFontW(height:i32,width:i32,escape:i32,orientation:i32,weight:i32,
        italic:u32,underline:u32,strike:u32,charset:u32,out_precision:u32,
        clip:u32,quality:u32,family:u32,face:*const u16)->H;
    fn CreateSolidBrush(rgb:u32)->H;
    fn DeleteObject(obj:H)->i32;
}
#[link(name="comdlg32")]
extern "system" { fn GetSaveFileNameW(data:*mut SaveFile)->i32; }
#[repr(C)]
struct OsVersionInfo {
    struct_size:u32, major:u32, minor:u32, build:u32, csd:[u16;128],
}
#[link(name="ntdll")]
extern "system" {
    fn RtlGetVersion(details:*mut OsVersionInfo)->i32;
}
fn system_description()->String {
    let mut details=OsVersionInfo {
        struct_size:size_of::<OsVersionInfo>() as u32, major:0, minor:0, build:0,
        csd:[0;128],
    };
    let status=unsafe{RtlGetVersion(&mut details)};
    if status==0 {
        format!("Windows {}.{} build {} (x64)",details.major,details.minor,details.build)
    } else {
        "Windows x64 (version unavailable)".into()
    }
}

#[link(name="kernel32")]
extern "system" {
    fn GetModuleHandleW(name:*const u16)->H;
    fn GetTickCount64()->u64;
    fn QueryPerformanceFrequency(out:*mut i64)->i32;
    fn QueryPerformanceCounter(out:*mut i64)->i32;
}

struct Labels {
    start:H, recording:H,
    mouse_count:H, mouse_last:H, mouse_age:H, mouse_interval:H,
    key_count:H, key_last:H, key_age:H, key_interval:H,
    count:H, mouse:H, keyboard:H, median:H,
    secondary:H, latest_pair:H, recent:H, notification:H,
}
struct KeyboardViews {
    counters:H, held:H, alerts:H, durations:H, queue:H, os_state:H, recent:H, polling:H,
}
struct ReactionViews {
    action:H, status:H, stats:H, last:H, recent:H,
}
struct TypingViews {
    input:H, stats:H, last_gap:H, text_log:H, raw_log:H,
}
struct MouseViews{stats:H,recent:H,deep:H}
struct FlightViews{status:H,markers:H,window:H,threads:H}
struct State {
    session:Session, active:bool, labels:Option<Labels>, note:String,
    mode:usize, groups:[Vec<H>;7],
    frequency:i64,
    keyboard:KeyboardDiagnostics, os_keys:OsKeyStateMonitor,
    key_views:Option<KeyboardViews>,
    reaction:ReactionTest, reaction_views:Option<ReactionViews>,
    typing:TypingMonitor, typing_views:Option<TypingViews>,
    trace:DeepTrace, mouse_diag:MouseDiagnostics, mouse_views:Option<MouseViews>,
    polling:PollingMonitor,
    flight:FlightRecorder,flight_views:Option<FlightViews>,watchdog:Watchdog,
    capture_watchdog:Watchdog,capture:Option<InputCapture>,thread_metrics:ThreadMetrics,
}
thread_local! {
    static STATE:RefCell<Option<State>>=const{RefCell::new(None)};
    static CREATE_GROUP:Cell<usize>=const{Cell::new(0)};
    static ORIGINAL_TYPING_PROC:Cell<isize>=const{Cell::new(0)};
    static GUI_HANDLER_MAX:Cell<i64>=const{Cell::new(0)};
}

fn now()->i64 {let mut v=0; unsafe{QueryPerformanceCounter(&mut v);} v}
struct HandlerCost(i64);
impl Drop for HandlerCost{
    fn drop(&mut self){
        let spent=now().saturating_sub(self.0);
        GUI_HANDLER_MAX.with(|max|{if spent>max.get(){max.set(spent);}});
    }
}
fn sync_capture(app:&mut State){
    if let Some(c)=app.capture.as_ref(){c.set_mode(app.mode);c.set_enabled(app.active && app.mode!=6);}
    let enabled=app.active && matches!(app.mode,2|4|5);
    app.watchdog.set_active(enabled,now());
    app.capture_watchdog.set_active(enabled,now());
}
fn queue_bits()->u32 {
    // High word shows present queue bits. GetQueueStatus does not remove messages.
    unsafe{GetQueueStatus(QS_KEY|QS_RAWINPUT)>>16}
}
fn first_pending_raw_size()->u32{
    // Null-output size QUERY ONLY: does not drain/remove any Raw Input records.
    // Reports size of the FIRST waiting record, not number or total bytes.
    let mut bytes=0u32;
    unsafe{GetRawInputBuffer(null_mut(),&mut bytes,size_of::<RawHeader>() as u32);}
    bytes
}
fn message_age()->u32{
    unsafe{(GetTickCount64() as u32).wrapping_sub(GetMessageTime() as u32)}
}
fn record_key_message(msg:u32,wp:WP,lp:LP){
    let tag=match msg {
        WM_KEYDOWN=>"WM_KEYDOWN",WM_KEYUP=>"WM_KEYUP",
        WM_SYSKEYDOWN=>"WM_SYSKEYDOWN",WM_SYSKEYUP=>"WM_SYSKEYUP",
        _=>return
    };
    let t=now();let queued=queue_bits();let age=message_age();
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            if app.active && (app.mode==2 || app.mode==4 || app.mode==5) {
                // Mouse mode records mouse motion, not a second typed-key history.
                if app.mode==2 || app.mode==4 {
                    app.flight.observe(FlightEvent::new(t,Lane::WinKey,tag,wp as u16,0,
                        (lp as u64 & 0xffff) as i32,age));
                }
                app.trace.record(TraceEvent::keyboard_msg(t,tag,wp,lp,age,queued,first_pending_raw_size()));
            }
        }
    });
}
fn append_device_inventory(trace:&DeepTrace,out:&mut String){
    use std::collections::BTreeSet;
    let devices: BTreeSet<usize>=trace.events.iter().filter_map(|e|
        (e.device!=0).then_some(e.device)).collect();
    let _=writeln!(out,"\nRAW INPUT DEVICE IDENTIFIERS (device serial/path redacted)");
    for device in devices {
        let mut size=0u32;
        let rc=unsafe{GetRawInputDeviceInfoW(device as H,RIDI_DEVICENAME,null_mut(),&mut size)};
        if rc==u32::MAX || size==0 || size>8192 {
            let _=writeln!(out,"device_handle=0x{device:x} VID/PID unavailable");
            continue;
        }
        let mut buffer=vec![0u16;size as usize];
        let read=unsafe{GetRawInputDeviceInfoW(device as H,RIDI_DEVICENAME,
            buffer.as_mut_ptr().cast(),&mut size)};
        if read==u32::MAX {
            let _=writeln!(out,"device_handle=0x{device:x} VID/PID unavailable");
            continue;
        }
        let path=String::from_utf16_lossy(&buffer[..(read as usize).min(buffer.len())]);
        let up=path.to_ascii_uppercase();
        let token=|tag:&str|{
            up.find(tag).and_then(|start|up.get(start+tag.len()..start+tag.len()+4))
                .filter(|x|x.chars().all(|c|c.is_ascii_hexdigit()))
                .unwrap_or("unknown")
                .to_string()
        };
        let _=writeln!(out,"device_handle=0x{device:x} VID={} PID={}",token("VID_"),token("PID_"));
    }
}

unsafe fn control(parent:H,instance:H,class:&str,text:&str,id:usize,
    x:i32,y:i32,w:i32,h:i32,style:u32,font:H)->H {
    let hwnd=CreateWindowExW(0,wide(class).as_ptr(),wide(text).as_ptr(),
        WS_VISIBLE|WS_CHILD|style,x,y,w,h,parent,id as H,instance,null());
    if !font.is_null(){SendMessageW(hwnd,WM_SETFONT,font as usize,1);}
    if !hwnd.is_null() {
        CREATE_GROUP.with(|group|{
            STATE.with(|cell|{
                if let Some(app)=cell.borrow_mut().as_mut(){
                    app.groups[group.get()].push(hwnd);
                }
            });
        });
    }
    hwnd
}
unsafe fn text(parent:H,instance:H,value:&str,x:i32,y:i32,w:i32,h:i32,font:H)->H{
    control(parent,instance,"STATIC",value,0,x,y,w,h,SS_LEFT,font)
}
fn set_text(hwnd:H,value:&str){if !hwnd.is_null(){unsafe{SetWindowTextW(hwnd,wide(value).as_ptr());}}}
fn get_text(hwnd:H)->String {
    if hwnd.is_null(){return String::new();}
    let cap=unsafe{GetWindowTextLengthW(hwnd)}.max(0) as usize;
    let mut buf=vec![0u16;cap.min(32768)+1];
    let read=unsafe{GetWindowTextW(hwnd,buf.as_mut_ptr(),buf.len() as i32)}.max(0) as usize;
    String::from_utf16_lossy(&buf[..read])
}
/// Observe real character messages delivered to the native test EDIT control.
/// Only this specific, focused textbox is subclassed; no global Windows hooks.
unsafe extern "system" fn typing_edit_proc(hwnd:H,msg:u32,wp:WP,lp:LP)->LP {
    STATE.with(|cell|{
        if let Ok(state)=cell.try_borrow(){
            if let Some(app)=state.as_ref(){app.watchdog.beat(now());}
        }
    });
    if matches!(msg,WM_KEYDOWN|WM_KEYUP|WM_SYSKEYDOWN|WM_SYSKEYUP){
        record_key_message(msg,wp,lp);
    }
    if msg==WM_CHAR {
        let tick=now();
        STATE.with(|cell|{
            if let Some(app)=cell.borrow_mut().as_mut(){
                if app.mode==4 && app.active {
                    app.flight.observe(FlightEvent::new(
                        tick,Lane::Char,"WM_CHAR",0,0,0,0));
                    app.typing.character(wp as u16,tick);
                }
            }
        });
    }
    let original=ORIGINAL_TYPING_PROC.with(Cell::get);
    if original!=0 {CallWindowProcW(original,hwnd,msg,wp,lp)}
    else {DefWindowProcW(hwnd,msg,wp,lp)}
}
unsafe fn font(height:i32,weight:i32)->H {
    CreateFontW(-height,0,0,0,weight,0,0,0,1,0,0,5,0,wide("Segoe UI").as_ptr())
}


fn format_age(value: Option<f64>) -> String {
    match value {
        Some(ms) if ms < 1000.0 => format!("{ms:.0} ms ago"),
        Some(ms) => format!("{:.2} s ago", ms / 1000.0),
        None => "No input yet".into(),
    }
}

fn show_device(snapshot: DeviceSnapshot, count:H, last:H, age:H, interval:H, unit:&str) {
    set_text(count, &format!("{} {}", snapshot.press_count, unit));
    set_text(last, &snapshot.last_event_ms.map_or_else(|| "Last event: —".to_string(),
        |ms| format!("Last event: {ms:.3} ms after start")));
    set_text(age, &format!("Seen: {}", format_age(snapshot.age_ms)));
    set_text(interval, &snapshot.last_interval_ms.map_or_else(
        || "Press-to-press interval: —".to_string(),
        |ms| format!("Press-to-press interval: {ms:.3} ms")));
}


fn key_name(vk:u16,scan:u16,flags:u16)->String {
    // Keep a useful label even if the active keyboard layout cannot name the key.
    if (0x41..=0x5a).contains(&vk) || (0x30..=0x39).contains(&vk) {
        return char::from_u32(vk as u32).unwrap().to_string();
    }
    match vk {
        0x20=>"Space".into(),0x08=>"Backspace".into(),0x0d=>"Enter".into(),
        0x09=>"Tab".into(),0x1b=>"Esc".into(),0x10=>"Shift".into(),
        0x11=>"Ctrl".into(),0x12=>"Alt".into(),
        0x25=>"Left".into(),0x26=>"Up".into(),0x27=>"Right".into(),0x28=>"Down".into(),
        _=>{
            let suffix=if flags&2!=0 {" E0"} else if flags&4!=0 {" E1"} else {""};
            format!("VK 0x{vk:02X} / scan {scan:02X}{suffix}")
        },
    }
}

fn mode_title(m:usize)->&'static str {
    match m { 1=>"1 · Input comparison",2=>"2 · Keyboard diagnostics",
        3=>"3 · Mouse reaction time",4=>"4 · Typing test",5=>"5 · Mouse diagnostics",6=>"6 · Flight recorder",_=>"Unknown" }
}
fn select_mode(hwnd:H,new_mode:usize){
    STATE.with(|cell|{
        let mut cell=cell.borrow_mut();
        let Some(app)=cell.as_mut()else{return};
        app.session.break_pair();
        if app.mode!=new_mode && matches!(app.mode,2|4|5) {
            app.flight.finish_early();
        }
        if app.mode!=new_mode {
            // Explicit diagnostic-mode selection is the privacy boundary:
            // do not carry key histories between unrelated capture sessions.
            app.trace=DeepTrace::new(app.frequency,now());
            app.polling=PollingMonitor::new(app.frequency);
            if new_mode==5 {
                app.mouse_diag=MouseDiagnostics::new(app.frequency,now());
            }
        }
        app.watchdog.set_active(app.active && matches!(new_mode,2|4|5),now());
        app.capture_watchdog.set_active(app.active && matches!(new_mode,2|4|5),now());
        if app.mode!=new_mode && (app.mode==2 || app.mode==4 || new_mode==2 || new_mode==4) {
            app.keyboard.suspend();
            app.os_keys.clear_pending();
            app.typing.suspend();
        }
        app.mode=new_mode;
        sync_capture(app);
        app.note=match new_mode{
            1=>"Mouse and Space events update live on this page.".into(),
            2=>"Test key combinations; press and release keys to watch their states.".into(),
            3=>"Click Start trial, wait for GREEN, then click the colored panel.".into(),
            4=>"Click the typing box and type test text. Watch both WM_CHAR and Raw Input timings.".into(),
            5=>"Move the mouse continuously. Compare raw report spacing with sampled Windows pointer movement.".into(),
            6=>"Inspect frozen evidence. Zoom or pan. Export only after reviewing privacy notice.".into(),
            _=>String::new(),
        };
        for (i,group) in app.groups.iter().enumerate(){
            if i==0{continue;}
            for &handle in group {
                unsafe{ShowWindow(handle,if i==new_mode {5}else{0});}
            }
        }
    });
    unsafe{InvalidateRect(hwnd,null(),1);}
    if new_mode==4 {
        STATE.with(|cell|{
            if let Some(app)=cell.borrow().as_ref(){
                if let Some(view)=app.typing_views.as_ref() {
                    unsafe{SetFocus(view.input);}
                }
            }
        });
    }
    refresh();
}
fn query_hid() {
    // An explicit button press initiates only GET operations on device metadata.
    // Handles are opened with ZERO requested access; unsupported is not an error.
    let devices=STATE.with(|cell|{
        let c=cell.borrow();
        c.as_ref().map_or_else(Vec::new,|app|{
            let mut handles=Vec::new();
            for &h in app.polling.mice.keys(){handles.push((true,h));}
            for &h in app.polling.keyboards.keys(){handles.push((false,h));}
            handles
        })
    });
    let count=devices.len();
    let mut succeeded=0;
    let mut results=Vec::with_capacity(count);
    for (mouse,handle) in devices {
        let ms=get_hid_polled_interval_ms(handle);
        if ms.is_some(){succeeded+=1;}
        results.push((mouse,handle,ms));
    }
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            for (mouse,handle,ms) in results{
                app.polling.set_hid_query(mouse,handle,ms);
            }
            app.note=format!("Read-only HID query: {succeeded}/{count} collections supplied an interval. This is NOT USB endpoint bInterval; unsupported means unknown.");
        }
    });
    refresh();
}
fn arm_reaction(hwnd:H){
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            app.reaction.arm(now(),app.frequency);
        }
    });
    unsafe{InvalidateRect(hwnd,null(),1);}
    refresh();
}
fn draw_flight_chart(dc:H){
    STATE.with(|cell|{
        let state=cell.borrow();
        let Some(app)=state.as_ref() else{return};
        if app.mode!=6{return;}
        let width=735usize;
        let end=app.flight.current_end(now());
        let histogram=app.flight.histogram(end,width);
        let colors=[0x00b78516u32,0x00a65f76,0x00a53d16,
            0x00a29030,0x00664bd3,0x00169add];
        let brushes:Vec<H>=colors.iter().map(|c|unsafe{CreateSolidBrush(*c)}).collect();
        let backdrop=unsafe{CreateSolidBrush(0x00f1eae3)};
        let grid=unsafe{CreateSolidBrush(0x00dfd8d2)};
        let tick=(app.flight.width_ms/1000.0)/6.0;
        for lane in 0..LANES{
            let y=395+(lane as i32)*61;
            let bg=Rect{left:155,top:y,right:895,bottom:y+43};
            unsafe{FillRect(dc,&bg,backdrop);}
            let label=["RAW KEY","WIN32 KEY","WM_CHAR",
                "MOUSE","GUI GAP","ANOMALY"][lane];
            let mut name=Rect{left:33,top:y+8,right:150,bottom:y+41};
            unsafe{
                SetBkMode(dc,1);SetTextColor(dc,0x0043362e);
                DrawTextW(dc,wide(label).as_ptr(),-1,&mut name,0);
            }
            for x in 0..=6{
                let gx=155+((width*x)/6) as i32;
                unsafe{FillRect(dc,&Rect{left:gx,top:y,right:gx+1,bottom:y+43},grid);}
            }
            for (col,row) in histogram.iter().enumerate(){
                let count=row[lane];
                if count==0{continue;}
                let height=((count as i32)*4).clamp(4,40);
                let gx=155+col as i32;
                let bar=Rect{left:gx,top:y+42-height,right:gx+1,bottom:y+42};
                unsafe{FillRect(dc,&bar,brushes[lane]);}
            }
        }
        for i in 0..=6{
            let x=155+(width*i/6) as i32;
            let offset=(6-i) as f64*tick;
            let mut rect=Rect{left:x.saturating_sub(24),top:765,right:x+42,bottom:786};
            unsafe{
                SetTextColor(dc,0x0043362e);
                DrawTextW(dc,wide(&format!("-{offset:.2}s")).as_ptr(),-1,&mut rect,0);
            }
        }
        let mut info=Rect{left:155,top:363,right:890,bottom:388};
        unsafe{
            DrawTextW(dc,wide("Left: older  /  Right: newer  |  bar height = events per pixel").as_ptr(),
                -1,&mut info,0);
            for b in brushes {DeleteObject(b);}
            DeleteObject(backdrop);DeleteObject(grid);
        }
    });
}
fn flight_action(hwnd:H,id:usize){
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            match id{
                ID_FLIGHT_MARK=>{
                    if app.active && matches!(app.mode,2|4|5|6){
                        app.flight.manual_capture(now());
                        if app.mode==6{app.flight.finish_early();}
                        app.note="Manual flight marker recorded in memory. Keep diagnostic test running for 5s, then open Flight tab.".into();
                    }else{app.note="Select Keyboard, Typing, or Mouse to start a diagnostic capture.".into();}
                },
                ID_FLIGHT_ZOOM_IN=>app.flight.zoom(true),
                ID_FLIGHT_ZOOM_OUT=>app.flight.zoom(false),
                ID_FLIGHT_BACK=>app.flight.pan(true),
                ID_FLIGHT_FORWARD=>app.flight.pan(false),
                _=>{},
            }
        }
    });
    unsafe{InvalidateRect(hwnd,null(),0);}
    refresh();
}
fn paint_reaction(hwnd:H) {
    // Paint submission is our local timing reference; physical display scan-out
    // may occur later, so this is NOT photon-verified reaction time.
    let mut ps:Paint=unsafe{zeroed()};
    let dc=unsafe{BeginPaint(hwnd,&mut ps)};
    if dc.is_null(){return;}
    let (mode,phase)=STATE.with(|cell|{
        let c=cell.borrow();
        c.as_ref().map(|a|(a.mode,a.reaction.phase)).unwrap_or((0,Phase::Ready))
    });
    if mode==3 {
        let client=unsafe{
            let mut r:Rect=zeroed();
            GetClientRect(hwnd,&mut r);
            r
        };
        let mut rect=Rect{left:32,top:315,right:client.right-32,bottom:590};
        let (bg,caption)=match phase {
            Phase::Ready=>(0x00403532,"CLICK START TRIAL TO BEGIN"),
            Phase::Waiting{..}=>(0x003C799C,"WAIT FOR GREEN — DO NOT CLICK"),
            Phase::PendingPaint|Phase::Visible{..}=>(0x004EBC23,"CLICK NOW!"),
            Phase::Complete{..}=>(0x004D443C,"TRIAL COMPLETE — START AGAIN"),
            Phase::FalseStart=>(0x004C42DD,"TOO EARLY — FALSE START"),
        };
        unsafe{
            let brush=CreateSolidBrush(bg);
            FillRect(dc,&rect,brush);
            DeleteObject(brush);
            SetBkMode(dc,1);
            SetTextColor(dc,0x00FFFFFF);
            DrawTextW(dc,wide(caption).as_ptr(),-1,&mut rect,0x21|0x4);
        }
    }
    if mode==6{draw_flight_chart(dc);}
    unsafe{EndPaint(hwnd,&ps);}
    if mode==3 && phase==Phase::PendingPaint{
        STATE.with(|cell|{
            if let Some(app)=cell.borrow_mut().as_mut(){
                app.reaction.painted(now());
            }
        });
    }
}

fn refresh() {
    STATE.with(|cell|{
        let state=cell.borrow();
        let Some(app)=state.as_ref() else{return;};
        let Some(l)=app.labels.as_ref() else{return;};
        let timestamp = now();
        set_text(l.start,if app.active {"Pause"} else {"Start recording"});
        set_text(l.recording,if app.active {"●  RECORDING  ·  Input is being captured"}
            else {"●  PAUSED  ·  Press Start recording"});
        // Hidden Win32 controls should never incur periodic formatting,
        // SetWindowText or layout work during sensitive high-frequency input.
        if app.mode==1{
        show_device(app.session.device_snapshot(Kind::Mouse, timestamp),
            l.mouse_count,l.mouse_last,l.mouse_age,l.mouse_interval,"clicks");
        show_device(app.session.device_snapshot(Kind::Keyboard, timestamp),
            l.key_count,l.key_last,l.key_age,l.key_interval,"key presses");
        set_text(l.count,&app.session.trial_count().to_string());
        let s=app.session.stats();
        set_text(l.mouse,&s.as_ref().map_or(0,|s|s.mouse_first).to_string());
        set_text(l.keyboard,&s.as_ref().map_or(0,|s|s.keyboard_first).to_string());
        set_text(l.median,&s.as_ref().map_or_else(||"—".into(),
            |s|format!("{:.3} ms",s.median_absolute_ms)));
        let details=s.as_ref().map_or_else(||{
            "Complete 30–100+ pairs for a more useful comparison.".to_string()
        },|s|format!("Mean signed (keyboard - mouse): {:+.3} ms     \
            P95 absolute gap: {:.3} ms     Mouse first: {:.1}%     Keyboard first: {:.1}%",
            s.mean_signed_ms,s.p95_absolute_ms,
            100.0*s.mouse_first as f64/s.count as f64,
            100.0*s.keyboard_first as f64/s.count as f64));

        set_text(l.secondary,&details);
        let latest = app.session.trials().last().map_or_else(
            || "Latest matched pair: waiting for a click + Space".to_string(),
            |t| {
                let delta = t.keyboard_minus_mouse_ms;
                let first = if delta > 0.0 {"mouse first"} else if delta < 0.0 {"keyboard first"}
                    else {"same timestamp"};
                format!("Latest pair #{}: {:+.3} ms   ({first})",t.number,delta)
            },
        );
        set_text(l.latest_pair,&latest);
        let mut lines=String::from("TRIAL              FIRST INPUT              SIGNED GAP\r\n\
            ─────────────────────────────────────────────────\r\n");
        for t in app.session.trials().iter().rev().take(9) {
            let first=if t.keyboard_minus_mouse_ms>0.0{"MOUSE"}
                else if t.keyboard_minus_mouse_ms<0.0{"KEYBOARD"}else{"TIE"};
            lines.push_str(&format!("{:>4}                     {:<12}           {:+.3} ms\r\n",
                t.number,first,t.keyboard_minus_mouse_ms));
        }
        if app.session.trial_count()==0{
            lines.push_str("\r\nPress the left mouse button and Space close together to compare arrival times.");
        }
        set_text(l.recent,&lines);
        set_text(l.notification,&app.note);
        }
        if app.mode==4 {
        if let Some(view)=app.typing_views.as_ref() {
            set_text(view.stats,&app.typing.stats());
            set_text(view.last_gap,&format!(
                "Last text gap: {}   |   largest gap (includes thinking/pauses): {:.1} ms   |   last raw gap: {}",
                app.typing.latest_char_gap_ms.map_or_else(||"—".into(),|ms|format!("{ms:.3} ms")),
                app.typing.largest_char_gap_ms,
                app.typing.last_raw_gap_ms.map_or_else(||"—".into(),|ms|format!("{ms:.3} ms"))
            ));
            set_text(view.text_log,&app.typing.recent("WM_CHAR",11));
            set_text(view.raw_log,&app.typing.recent("RAW_INPUT",11));
        }
        }
        if let Some(v)=app.flight_views.as_ref(){
            if app.mode==6 {
                set_text(v.status,&app.flight.summary());
                let dropped=app.capture.as_ref().map_or(0,InputCapture::dropped);
                set_text(v.threads,&app.thread_metrics.short_status(dropped));
                set_text(v.markers,&app.flight.marker_list());
                let (start,end)=app.flight.view_bounds_ms(timestamp);
                set_text(v.window,&format!(
                    "View {:.3}–{:.3}s | span {:.2}s | pan {:.2}s",
                    start/1000.0,end/1000.0,
                    app.flight.width_ms/1000.0,app.flight.pan_ms/1000.0));
            }
        }
        if let Some(v)=app.mouse_views.as_ref(){
            if app.mode==5 {
                set_text(v.stats,&app.polling.gui_mouse(timestamp));
                set_text(v.recent,&app.mouse_diag.recent());
                set_text(v.deep,&format!("{}\r\nRaw cursor comparison and full per-device data are included in saved report.",app.trace.summary()));
            }
        }
        if app.mode==2 {
        if let Some(k)=app.key_views.as_ref(){
            let kd=&app.keyboard;
            set_text(k.counters,&format!(
                "Down: {}     Up: {}     Auto-repeats: {}     Unmatched key-ups: {}     Max held together: {}",
                kd.downs,kd.ups,kd.repeats,kd.orphan_ups,kd.maximum_simultaneous));
            set_text(k.held,&kd.held_display(timestamp));
            set_text(k.alerts,&kd.alert_display(timestamp));
            set_text(k.durations,&format!(
                "Currently held: {}  |  Last down→up: {}  |  Longest: {:.1} ms  |  Shortest event gap: {}",
                kd.held_count(),
                kd.last_release_duration_ms.map_or_else(||"—".into(),|x|format!("{x:.1} ms")),
                kd.max_release_duration_ms,
                kd.shortest_gap_ms.map_or_else(||"—".into(),|x|format!("{x:.2} ms"))));
            set_text(k.queue,&format!(
                "Windows message queue age (approx.): latest {}  |  worst {} ms  |  ≥32 ms delays: {}",
                kd.latest_queue_age_ms.map_or_else(||"—".into(),|x|format!("{x} ms")),
                kd.largest_queue_age_ms, kd.slow_queue_events));
            set_text(k.os_state,&format!(
                "Raw vs Windows key-state checks: {} samples  |  sustained disagreements: {}",
                app.os_keys.samples,app.os_keys.confirmed_disagreements));
            if app.mode==2 {set_text(k.polling,&app.polling.gui_keyboard(timestamp));}
            set_text(k.recent,&kd.log_display());
        }
        }
        if app.mode==3 {
        if let Some(r)=app.reaction_views.as_ref(){
            let rx=&app.reaction;
            let (status,caption)=match rx.phase {
                Phase::Ready=>("Ready. Click Start trial to begin.","Start trial"),
                Phase::Waiting{..}=>("Wait for GREEN; do not click early.","Waiting..."),
                Phase::PendingPaint=>("Preparing the green signal...","Waiting..."),
                Phase::Visible{..}=>("GREEN! Click the colored area NOW.","Waiting..."),
                Phase::Complete{milliseconds}=>(
                    if milliseconds < 100.0 {"Result recorded (very fast — check for anticipation)."}
                    else {"Result recorded. Start another trial."},"Start next trial"),
                Phase::FalseStart=>("False start: mouse clicked before green.","Try again"),
            };
            set_text(r.status,status);
            set_text(r.action,caption);
            set_text(r.last,&rx.last().map_or_else(||"Last result: —".into(),
                |v|format!("Last result: {v:.1} ms")));
            set_text(r.stats,&format!(
                "Trials: {}       Average: {}       Best: {}       False starts: {}",
                rx.results_ms.len(),
                rx.average().map_or_else(||"—".into(),|v|format!("{v:.1} ms")),
                rx.best().map_or_else(||"—".into(),|v|format!("{v:.1} ms")),
                rx.false_starts));
            let history=rx.results_ms.iter().enumerate().rev().take(10)
                .map(|(i,v)|format!("Trial {:>2}      {:.1} ms",i+1,v))
                .collect::<Vec<_>>().join("\r\n");
            set_text(r.recent,&history);
        }
        }
    });
}
fn toggle(){
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            // Both watchdogs are reset after changing active state.
            app.flight.clear_temporal_state();
            app.session.break_pair();
            app.keyboard.suspend();
            app.os_keys.clear_pending();
            app.typing.suspend();
            app.active=!app.active;
            sync_capture(app);
            if !app.active{app.flight.finish_early();}
            app.note=if app.active{
                "Recording: press LEFT CLICK + SPACE, releasing both before repeating.".into()
            }else{"Paused. Click Start recording when ready.".into()};
        }
    });
    refresh();
}
fn reset(){
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            if matches!(app.mode,2|4|5|6){
                app.flight=FlightRecorder::new(app.frequency,now());
                app.watchdog.drain();
                sync_capture(app);
            }
            match app.mode{
                6=>{app.note="Flight recorder cleared; switch to Keyboard, Typing, or Mouse to arm again.".into();},
                2=>{app.polling=PollingMonitor::new(app.frequency);
                     app.trace=DeepTrace::new(app.frequency,now());
                     app.keyboard=KeyboardDiagnostics::new(app.frequency);
                    app.os_keys=OsKeyStateMonitor::new(now(),app.frequency);
                    app.note="Keyboard diagnostics and Windows state checks cleared.".into();},
                5=>{app.polling=PollingMonitor::new(app.frequency);
                     app.mouse_diag=MouseDiagnostics::new(app.frequency,now());
                     app.trace=DeepTrace::new(app.frequency,now());
                     app.note="Mouse motion and queue observations reset.".into();},
                3=>{app.reaction.reset();
                    app.note="Reaction results cleared.".into();},
                4=>{
                    app.polling=PollingMonitor::new(app.frequency);
                    app.trace=DeepTrace::new(app.frequency,now());
                    app.typing=TypingMonitor::new(app.frequency,now());
                    app.keyboard=KeyboardDiagnostics::new(app.frequency);
                    app.os_keys=OsKeyStateMonitor::new(now(),app.frequency);
                    if let Some(view)=app.typing_views.as_ref() {
                        set_text(view.input,"");
                    }
                    app.note="Typing text and timing history cleared.".into();
                },
                _=>{app.session.reset(now());app.note="Input comparison reset.".into();}
            }
        }
    });
    refresh();
}
fn error(hwnd:H,message:&str){
    unsafe{MessageBoxW(hwnd,wide(message).as_ptr(),wide("Input Timing Lab").as_ptr(),ERROR_ICON);}
}
fn save(hwnd:H){
    // Freeze capture while the Windows Save dialog is open. The active mode
    // chooses both the output filename and the serialized data.
    let (was_active, mode, snapshot_ticks)=STATE.with(|cell|{
        let mut state=cell.borrow_mut();
        let Some(app)=state.as_mut() else {return (false,1,now())};
        let prior=app.active;
        app.active=false;
        app.flight.finish_early();
        sync_capture(app);
        app.session.break_pair();
        (prior,app.mode,now())
    });
    let (suggested_file, title_text)=match mode {
        2=>("keyboard_diagnostics.csv","Export complete keyboard diagnostic history"),
        3=>("reaction_times.csv","Export mouse reaction-time trials"),
        4=>("typing_event_timeline.csv","Export focused typing events and timings"),
        5=>("mouse_diagnostics.csv","Export mouse polling and pointer observations"),
        6=>("flight_recorder.csv","Export frozen or rolling flight recorder timeline"),
        _=>("mouse_keyboard_results.csv","Export mouse / keyboard comparison"),
    };
    let mut path=[0u16;1024];
    let suggested=wide(suggested_file);
    path[..suggested.len()].copy_from_slice(&suggested);
    let filter=wide("CSV files (*.csv)\0*.csv\0All files (*.*)\0*.*\0");
    let title=wide(title_text);
    let ext=wide("csv");
    let mut details=SaveFile {
        size:size_of::<SaveFile>() as u32,owner:hwnd,instance:null_mut(),
        filter:filter.as_ptr(),custom:null_mut(),custom_max:0,filter_index:1,
        file:path.as_mut_ptr(),file_max:path.len() as u32,
        file_title:null_mut(),file_title_max:0,initial_dir:null(),
        title:title.as_ptr(),flags:OFN_OVERWRITEPROMPT|OFN_PATHMUSTEXIST,
        file_offset:0,file_extension:0,default_ext:ext.as_ptr(),custom_data:0,
        hook:null_mut(),template:null(),reserved:null_mut(),reserved_num:0,flags_ex:0,
    };
    if unsafe{GetSaveFileNameW(&mut details)}!=0 {
        let end=path.iter().position(|&c|c==0).unwrap_or(path.len());
        let filename=String::from_utf16_lossy(&path[..end]);
        let result=STATE.with(|cell|{
            let state=cell.borrow();
            let app=state.as_ref().expect("GUI state initialized");
            match mode {
                2=>app.keyboard.save_csv(Path::new(&filename),snapshot_ticks),
                3=>app.reaction.save_csv(Path::new(&filename)),
                4=>app.typing.save_csv(Path::new(&filename)),
                5=>app.mouse_diag.save_csv(Path::new(&filename)),
                6=>app.flight.save_csv(Path::new(&filename)),
                _=>app.session.save_csv(Path::new(&filename)),
            }
        });
        match result{
            Ok(())=>STATE.with(|cell|{
                if let Some(app)=cell.borrow_mut().as_mut(){
                    app.note=format!("Saved {} data to {}",mode_title(mode),filename);
                }
            }),
            Err(e)=>error(hwnd,&format!("Unable to save CSV: {e}")),
        }
    }
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            // A key released while the modal dialog had focus must not
            // appear stuck after returning to the diagnostics screen.
            app.keyboard.suspend();
            app.os_keys.clear_pending();
            app.typing.suspend();
            app.session.break_pair();
            app.active=was_active;
            sync_capture(app);
        }
    });
    refresh();
}

fn save_report(hwnd:H) {
    // The user is about to share key-name history. Warn explicitly first.
    let warning=wide("The diagnostic report includes EVERY KEY NAME pressed during this test.\n\n\
        Only record test inputs, not passwords or private messages.\n\
        Review the text file before sharing it.\n\nSave report?");
    let heading=wide("Privacy notice — keyboard diagnostic report");
    // MB_OKCANCEL | MB_ICONWARNING. 1 means IDOK.
    if unsafe{MessageBoxW(hwnd,warning.as_ptr(),heading.as_ptr(),0x31)} != 1 {
        return;
    }

    // Freeze capture and build a consistent snapshot BEFORE the modal Save
    // dialog begins processing its own keyboard messages.
    let (was_active, contents)=STATE.with(|cell|{
        let mut state=cell.borrow_mut();
        let Some(app)=state.as_mut() else {return (false,String::new());};
        if app.mode!=2 && app.mode!=4 && app.mode!=5 && app.mode!=6{return (false,String::new());}
        let previous=app.active;
        app.active=false;
        app.flight.finish_early();
        sync_capture(app);
        let mut snapshot=if app.mode==6 {
            String::new()
        }else if app.mode==5 {
            format!("INPUT TIMING LAB — MOUSE DIAGNOSTIC REPORT\nPlatform: {}\n",system_description())
        }else{
            app.keyboard.build_report(now(),&app.os_keys,&system_description())
        };
        if app.typing.chars>0 || app.mode==4 {
            let contents=app.typing_views.as_ref()
                .map_or_else(String::new,|view|get_text(view.input));
            app.typing.append_report(&mut snapshot,&contents);
        }
        if app.mode==5{app.mouse_diag.append_report(&mut snapshot);}
        if app.mode==6 {
            app.flight.append_report(&mut snapshot);
        }else{
            app.flight.append_report(&mut snapshot);
            app.trace.append_report(&mut snapshot);
        }
        if app.mode!=6 {
            app.polling.append_report(&mut snapshot,now());
            append_device_inventory(&app.trace,&mut snapshot);
        }
        app.thread_metrics.append_report(&mut snapshot,
            app.capture.as_ref().map_or(0,InputCapture::dropped));
        (previous,snapshot)
    });
    if contents.is_empty(){return;}

    let mut name=[0u16;1024];
    let default_name=wide("keyboard_diagnostic_report.txt");
    name[..default_name.len()].copy_from_slice(&default_name);
    let filter=wide("Text report (*.txt)\0*.txt\0All files (*.*)\0*.*\0");
    let title=wide("Save diagnostic report to share for analysis");
    let extension=wide("txt");
    let mut details=SaveFile {
        size:size_of::<SaveFile>() as u32,owner:hwnd,instance:null_mut(),
        filter:filter.as_ptr(),custom:null_mut(),custom_max:0,filter_index:1,
        file:name.as_mut_ptr(),file_max:name.len() as u32,
        file_title:null_mut(),file_title_max:0,initial_dir:null(),
        title:title.as_ptr(),flags:OFN_OVERWRITEPROMPT|OFN_PATHMUSTEXIST,
        file_offset:0,file_extension:0,default_ext:extension.as_ptr(),
        custom_data:0,hook:null_mut(),template:null(),
        reserved:null_mut(),reserved_num:0,flags_ex:0,
    };
    let confirmed=unsafe{GetSaveFileNameW(&mut details)}!=0;
    if confirmed {
        let end=name.iter().position(|&n|n==0).unwrap_or(name.len());
        let filename=String::from_utf16_lossy(&name[..end]);
        match std::fs::write(Path::new(&filename),contents) {
            Ok(())=>STATE.with(|cell|{
                if let Some(app)=cell.borrow_mut().as_mut(){
                    app.note=format!("Report saved: {} (review before sharing)",filename);
                }
            }),
            Err(io_error)=>error(hwnd,&format!("Unable to save diagnostic report: {io_error}")),
        };
    }
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            app.keyboard.suspend();
            app.os_keys.clear_pending();
            app.typing.suspend();
            app.session.break_pair();
            app.active=was_active;
            sync_capture(app);
        }
    });
    refresh();
}

unsafe fn make_gui(hwnd:H){
    let instance=GetModuleHandleW(null());
    let f_small=font(14,FONT_NORMAL);
    let f_medium=font(17,FONT_BOLD);
    let f_large=font(29,FONT_BOLD);
    let f_title=font(28,FONT_BOLD);

    CREATE_GROUP.with(|g|g.set(0));
    text(hwnd,instance,"INPUT TIMING LAB",30,16,860,45,f_title);
    text(hwnd,instance,"Six tests  •  Read-only input flight recorder  •  Vulkan game scene",
        32,62,860,24,f_small);
    control(hwnd,instance,"BUTTON","1. Compare",ID_MODE_COMPARE,
        32,102,138,42,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","2. Keyboard",ID_MODE_KEYBOARD,
        178,102,138,42,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","3. Reaction",ID_MODE_REACTION,
        324,102,138,42,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","4. Typing",ID_MODE_TYPING,
        470,102,138,42,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","5. Mouse",ID_MODE_MOUSE,
        616,102,138,42,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","6. Flight",ID_MODE_FLIGHT,
        762,102,142,42,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    let start=control(hwnd,instance,"BUTTON","Pause",ID_START,
        31,163,110,38,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","Reset mode",ID_RESET,
        150,163,110,38,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","Export CSV",ID_SAVE,
        268,163,109,38,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","Mark Now",ID_FLIGHT_MARK,
        385,163,104,38,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    let recording=text(hwnd,instance,"",665,167,255,32,f_small);
    control(hwnd,instance,"BUTTON","Open Vulkan Scene",ID_VULKAN_SCENE,
        498,163,158,38,WS_TABSTOP|BS_PUSHBUTTON,f_small);

    CREATE_GROUP.with(|g|g.set(1));
    text(hwnd,instance,"LIVE MOUSE · LEFT CLICK",32,229,410,27,f_medium);
    text(hwnd,instance,"LIVE KEYBOARD · SPACE",492,229,410,27,f_medium);
    let mouse_count=text(hwnd,instance,"0 clicks",32,261,410,46,f_large);
    let key_count=text(hwnd,instance,"0 key presses",492,261,410,46,f_large);
    let mouse_last=text(hwnd,instance,"Last event: —",32,316,410,25,f_small);
    let key_last=text(hwnd,instance,"Last event: —",492,316,410,25,f_small);
    let mouse_age=text(hwnd,instance,"Seen: No input yet",32,343,410,25,f_small);
    let key_age=text(hwnd,instance,"Seen: No input yet",492,343,410,25,f_small);
    let mouse_interval=text(hwnd,instance,"Press-to-press interval: —",32,370,410,25,f_small);
    let key_interval=text(hwnd,instance,"Press-to-press interval: —",492,370,410,25,f_small);
    text(hwnd,instance,"LIVE ARRIVAL-TIME COMPARISON",32,409,835,30,f_medium);
    text(hwnd,instance,"PAIRED",32,450,180,22,f_small);
    text(hwnd,instance,"MOUSE FIRST",240,450,190,22,f_small);
    text(hwnd,instance,"KEYBOARD FIRST",472,450,190,22,f_small);
    text(hwnd,instance,"MEDIAN GAP",700,450,192,22,f_small);
    let count=text(hwnd,instance,"0",32,478,180,47,f_large);
    let mouse=text(hwnd,instance,"0",240,478,180,47,f_large);
    let keyboard=text(hwnd,instance,"0",472,478,180,47,f_large);
    let median=text(hwnd,instance,"—",700,478,192,47,f_large);
    let latest_pair=text(hwnd,instance,"Latest matched pair: waiting for input",32,533,855,30,f_medium);
    let secondary=text(hwnd,instance,"",32,570,855,30,f_small);
    text(hwnd,instance,"RECENT PAIRED RESULTS",32,612,855,27,f_medium);
    let recent=text(hwnd,instance,"",32,646,858,178,f_small);
    text(hwnd,instance,"Positive = mouse first  •  Negative = keyboard first",
        32,826,858,23,f_small);
    let notification=text(hwnd,instance,"",32,851,858,23,f_small);
    text(hwnd,instance,"Arrival times are not absolute input latency. A synchronized actuator is required for hardware comparisons.",
        32,880,858,22,f_small);

    CREATE_GROUP.with(|g|g.set(2));
    text(hwnd,instance,"KEYBOARD EVENT DIAGNOSTICS",32,230,870,34,f_title);
    control(hwnd,instance,"BUTTON","Read HID interval (GET)",ID_QUERY_HID,
        32,273,212,37,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    text(hwnd,instance,"Reproduce the issue; Save Report for full timing.",
        252,277,410,24,f_small);
    control(hwnd,instance,"BUTTON","Save Diagnostic Report",ID_DIAGNOSTIC_REPORT,
        670,269,232,41,WS_TABSTOP|BS_PUSHBUTTON,f_medium);
    text(hwnd,instance,"RAW KEY-DOWN / KEY-UP COUNTS",32,314,870,25,f_medium);
    let counters=text(hwnd,instance,"",32,351,870,31,f_small);
    text(hwnd,instance,"KEYS RAW INPUT STILL REPORTS AS HELD",32,397,870,25,f_medium);
    let held=text(hwnd,instance,"",32,430,870,76,f_medium);
    text(hwnd,instance,"SUSPECTED EVENT ANOMALIES",32,527,870,28,f_medium);
    let alerts=text(hwnd,instance,"",32,562,870,40,f_small);
    let durations=text(hwnd,instance,"",32,606,870,31,f_small);
    let queue=text(hwnd,instance,"",32,636,870,26,f_small);
    let os_state=text(hwnd,instance,"",32,665,870,25,f_small);
    let key_poll=text(hwnd,instance,"",32,692,870,85,f_small);
    text(hwnd,instance,"RECENT RAW KEY EVENTS",32,779,870,25,f_medium);
    let key_recent=text(hwnd,instance,"",32,808,870,79,f_small);
    text(hwnd,instance,"Only type test keys, never passwords. The saved report contains every key name observed.",
        32,891,870,24,f_small);

    CREATE_GROUP.with(|g|g.set(3));
    text(hwnd,instance,"MOUSE REACTION TIME",32,232,800,40,f_title);
    let arm_button=control(hwnd,instance,"BUTTON","Start trial",ID_REACTION_ARM,
        32,274,185,37,WS_TABSTOP|BS_PUSHBUTTON,f_medium);
    let status=text(hwnd,instance,"Ready. Click Start trial.",231,277,675,32,f_medium);
    text(hwnd,instance,"Native Vulkan scene: Open Vulkan Scene (above), F1–F8 adjust FPS cap.",
        32,318,870,26,f_small);
    // The colored test area is painted by WM_PAINT, not a STATIC child.
    text(hwnd,instance,"REACTION RESULTS — milliseconds",32,620,830,28,f_medium);
    let last=text(hwnd,instance,"Last result: —",32,662,410,36,f_large);
    let rx_stats=text(hwnd,instance,"",32,710,850,30,f_medium);
    text(hwnd,instance,"RECENT TRIALS",32,754,850,24,f_medium);
    let rx_recent=text(hwnd,instance,"",32,789,850,95,f_small);
    text(hwnd,instance,"The timer starts after the green frame's paint submission; monitor scan-out time is not measured.",
        32,891,875,23,f_small);

    CREATE_GROUP.with(|g|g.set(4));
    text(hwnd,instance,"LIVE TYPING / CHARACTER BURST TEST",32,230,870,35,f_title);
    text(hwnd,instance,"Type into THIS box. It displays actual Windows text input (not a guessed Raw Input mapping).",
        32,276,622,26,f_small);
    control(hwnd,instance,"BUTTON","Save Diagnostic Report",ID_DIAGNOSTIC_REPORT,
        670,269,232,41,WS_TABSTOP|BS_PUSHBUTTON,f_medium);
    text(hwnd,instance,"WINDOWS TEXTBOX — type a test sentence repeatedly, including rapid typing",
        32,315,870,26,f_medium);
    let typing_input=control(hwnd,instance,"EDIT","",ID_TYPING_INPUT,
        32,349,870,157,WS_TABSTOP|WS_BORDER|WS_VSCROLL|ES_MULTILINE|
            ES_AUTOVSCROLL|ES_WANTRETURN,f_medium);
    // Subclass only this test field. No system-wide keyboard hook or injection.
    let old_proc=SetWindowLongPtrW(typing_input,GWLP_WNDPROC,typing_edit_proc as usize as isize);
    if old_proc!=0 {ORIGINAL_TYPING_PROC.with(|cell|cell.set(old_proc));}
    text(hwnd,instance,"LIVE CHARACTER DELIVERY / INPUT COUNTS",32,519,870,26,f_medium);
    let typing_stats=text(hwnd,instance,"",32,550,870,27,f_small);
    let typing_gap=text(hwnd,instance,"",32,579,870,27,f_small);
    text(hwnd,instance,"ACTUAL WINDOWS WM_CHAR ARRIVAL TIMELINE (latest below)",32,621,870,27,f_medium);
    let typed_chars=control(hwnd,instance,"EDIT","",ID_TYPING_TEXTLOG,
        32,653,870,115,WS_BORDER|ES_MULTILINE|ES_AUTOVSCROLL|WS_VSCROLL|ES_READONLY,f_small);
    text(hwnd,instance,"RAW INPUT ARRIVALS — compare key-down / key-up / repeat timing",32,782,870,26,f_medium);
    let typed_raw=control(hwnd,instance,"EDIT","",ID_TYPING_RAWLOG,
        32,813,870,105,WS_BORDER|ES_MULTILINE|ES_AUTOVSCROLL|WS_VSCROLL|ES_READONLY,f_small);

    CREATE_GROUP.with(|g|g.set(5));
    text(hwnd,instance,"RAW MOUSE POLLING / WINDOWS CURSOR",32,230,870,35,f_title);
    control(hwnd,instance,"BUTTON","Read HID interval (GET)",ID_QUERY_HID,
        32,276,212,37,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    text(hwnd,instance,"Move mouse continuously; Raw Input rate != hardware bus polls.",
        255,276,414,38,f_small);
    control(hwnd,instance,"BUTTON","Save Diagnostic Report",ID_DIAGNOSTIC_REPORT,
        672,275,232,39,WS_TABSTOP|BS_PUSHBUTTON,f_medium);
    text(hwnd,instance,"LIVE RAW MOUSE STATISTICS",32,333,870,28,f_medium);
    let mouse_stats=text(hwnd,instance,"",32,370,860,134,f_medium);
    text(hwnd,instance,"LATEST RAW DELTAS vs SAMPLED WINDOWS CURSOR COORDINATES",
        32,505,870,28,f_medium);
    let mouse_recent=control(hwnd,instance,"EDIT","",0,
        32,550,870,285,WS_BORDER|ES_MULTILINE|ES_AUTOVSCROLL|WS_VSCROLL|ES_READONLY,f_small);
    let mouse_deep=text(hwnd,instance,"",32,847,870,50,f_small);
    text(hwnd,instance,"Open Vulkan Scene, then M for live cyan raw-counts vs orange Windows pointer markers.",
        32,905,870,25,f_small);
    CREATE_GROUP.with(|g|g.set(6));
    text(hwnd,instance,"ANOMALY FLIGHT RECORDER",32,230,870,35,f_title);
    text(hwnd,instance,"Capture is memory-only. Start on Keyboard, Typing or Mouse; switch here to inspect.",
        32,271,640,25,f_small);
    control(hwnd,instance,"BUTTON","Save Flight Report",ID_DIAGNOSTIC_REPORT,
        685,264,218,41,WS_TABSTOP|BS_PUSHBUTTON,f_medium);
    let flight_status=text(hwnd,instance,"",32,310,860,25,f_small);
    control(hwnd,instance,"BUTTON","Zoom +",ID_FLIGHT_ZOOM_IN,
        32,337,104,30,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","Zoom -",ID_FLIGHT_ZOOM_OUT,
        144,337,104,30,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","Older <<",ID_FLIGHT_BACK,
        256,337,112,30,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    control(hwnd,instance,"BUTTON","Newer >>",ID_FLIGHT_FORWARD,
        376,337,112,30,WS_TABSTOP|BS_PUSHBUTTON,f_small);
    let flight_window=text(hwnd,instance,"",496,339,406,27,f_small);
    let flight_threads=text(hwnd,instance,"",32,371,860,24,f_small);
    text(hwnd,instance,"CAPTURE MARKERS / AUTOMATIC DETECTIONS",32,794,865,24,f_medium);
    let flight_markers=control(hwnd,instance,"EDIT","",0,
        32,822,870,80,WS_BORDER|ES_MULTILINE|ES_AUTOVSCROLL|WS_VSCROLL|ES_READONLY,f_small);
    text(hwnd,instance,"Bars are host-delivery timestamps, not USB packet timestamps. Save only after reviewing key identifiers.",
        32,908,870,23,f_small);

    CREATE_GROUP.with(|g|g.set(0));
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            app.flight_views=Some(FlightViews{status:flight_status,markers:flight_markers,window:flight_window,threads:flight_threads});
            app.labels=Some(Labels{
                start,recording,
                mouse_count,mouse_last,mouse_age,mouse_interval,
                key_count,key_last,key_age,key_interval,
                count,mouse,keyboard,median,secondary,latest_pair,recent,notification,
            });
            app.key_views=Some(KeyboardViews{counters,held,alerts,durations,queue,os_state,recent:key_recent,polling:key_poll});
            app.reaction_views=Some(ReactionViews{
                action:arm_button,
                status,stats:rx_stats,last,recent:rx_recent,
            });
            app.mouse_views=Some(MouseViews{stats:mouse_stats,recent:mouse_recent,deep:mouse_deep});
            app.typing_views=Some(TypingViews{
                input:typing_input,stats:typing_stats,last_gap:typing_gap,
                text_log:typed_chars,raw_log:typed_raw,
            });
        }
    });
    select_mode(hwnd,1);
}
/// The GUI consumes bounded capture batches. The original QPC timestamps and
/// queue ages were taken BEFORE this thread got CPU time; GUI handoff is
/// measured independently and cannot change the actual Raw Input timestamps.
fn consume_captured(hwnd:H,app:&mut State,e:CapturedInput)->bool{
    if !app.active || app.mode==6{return false;}
    let tick=e.tick;let age=e.queue_age_ms;let bits=e.queue_bits;
    let elapsed=now().saturating_sub(tick) as f64*1000.0/app.frequency as f64;
    let keyboard=matches!(e.kind,CapturedKind::Keyboard{..});
    app.thread_metrics.raw(TimingSample{
        tick,device:e.device,keyboard,
        captured_age_ms:age,handoff_ms:elapsed,callback_us:e.callback_us,
        group:e.cpu.group,processor:e.cpu.number,
    });
    if matches!(app.mode,2|4|5){app.trace.observe_dispatch(tick,bits);}
    match e.kind{
        CapturedKind::Mouse{flags,buttons,dx,dy,cursor_x,cursor_y}=>{
            let mut p=Point{x:cursor_x,y:cursor_y};
            unsafe{ScreenToClient(hwnd,&mut p);}
            if app.mode==5{
                app.polling.mouse(e.device,tick,dx,dy,buttons);
                app.mouse_diag.observe(tick,e.device,dx,dy,
                    flags&1!=0,buttons,p.x,p.y);
            }
            if app.mode==5 || (matches!(app.mode,2|4)&&buttons!=0) {
                app.flight.observe(FlightEvent::new(tick,Lane::Mouse,
                    if buttons!=0{"BUTTON"}else{"MOVE"},0,e.device,
                    dx.saturating_abs().saturating_add(dy.saturating_abs()),age));
                app.trace.record(TraceEvent::raw(tick,"RAW_MOUSE",0,0,
                    age,bits,e.device,dx,dy,p.x,p.y,buttons,0));
            }
            if app.mode==1{
                let over_toolbar=p.x>=0&&p.x<930&&p.y>=0&&p.y<215;
                if buttons&RI_MOUSE_LEFT_BUTTON_UP!=0{
                    app.session.mouse_button(false,tick);
                }
                if buttons&RI_MOUSE_LEFT_BUTTON_DOWN!=0&&!over_toolbar{
                    app.session.mouse_button(true,tick);
                }
            } else if app.mode==3 && buttons&RI_MOUSE_LEFT_BUTTON_DOWN!=0 {
                if p.x>=32&&p.x<915&&p.y>=315&&p.y<590{
                    app.reaction.click(tick,app.frequency);
                    return true;
                }
            }
        },
        CapturedKind::Keyboard{vk,scan,flags}=>{
            if vk==0xff{return false;}
            let down=flags&RI_KEY_BREAK==0;
            if matches!(app.mode,2|4|5){
                if matches!(app.mode,2|4){
                    app.polling.keyboard(e.device,tick,vk,scan,down);
                }
                app.trace.record(TraceEvent::raw(tick,"RAW_KEY",
                    vk,scan,age,bits,e.device,0,0,0,0,0,0));
            }
            if app.mode==1 && vk==VK_SPACE {
                app.session.space_key(down,tick);
            }else if matches!(app.mode,2|4){
                let sc=if scan==0{vk}else{scan};
                let id=KeyId::new(sc,flags);
                let repeat=down && app.keyboard.observed_vk_states()
                    .iter().any(|(key,_,held)|*key==vk&&*held);
                let label=key_name(vk,sc,flags);
                app.flight.observe(FlightEvent::new(tick,Lane::RawKey,
                    if !down{"UP"}else if repeat{"REPEAT"}else{"DOWN"},
                    vk,e.device,0,age));
                app.keyboard.observe_queue_age(age);
                if app.mode==4{
                    app.typing.raw(&label,
                        if !down{"UP"}else if repeat{"REPEAT"}else{"DOWN"},tick);
                }
                app.keyboard.event_vk(id,label,down,tick,vk);
            }
        }
    }
    false
}

unsafe extern "system" fn wnd_proc(hwnd:H,msg:u32,wp:WP,lp:LP)->LP{
    let _cost=HandlerCost(now());
    // A heartbeat is published on each GUI callback, not just WM_TIMER.
    // WM_TIMER is low priority when input is saturated.
    STATE.with(|cell|{
        if let Ok(state)=cell.try_borrow(){
            if let Some(app)=state.as_ref(){app.watchdog.beat(now());}
        }
    });
    if matches!(msg,WM_KEYDOWN|WM_KEYUP|WM_SYSKEYDOWN|WM_SYSKEYUP) {
        record_key_message(msg,wp,lp);
    }
    match msg{
        WM_CREATE=>{make_gui(hwnd);return 0;}
        // WM_INPUT now belongs exclusively to the independent message-only
        // capture window. This GUI thread never receives Raw Input directly.
        WM_INPUT=>{return DefWindowProcW(hwnd,msg,wp,lp);}
        WM_COMMAND=>{
            let id=wp&0xFFFF;
            if id==ID_TYPING_INPUT || id==ID_TYPING_TEXTLOG || id==ID_TYPING_RAWLOG {
                // EN_CHANGE and other notifications must NOT steal textbox
                // focus. The user must be able to type continuously.
                return 0;
            }
            match id {
                ID_START=>toggle(),
                ID_RESET=>reset(),
                ID_SAVE=>save(hwnd),
                ID_DIAGNOSTIC_REPORT=>save_report(hwnd),
                ID_MODE_COMPARE=>select_mode(hwnd,1),
                ID_MODE_KEYBOARD=>select_mode(hwnd,2),
                ID_MODE_REACTION=>select_mode(hwnd,3),
                ID_MODE_TYPING=>select_mode(hwnd,4),
                ID_REACTION_ARM=>arm_reaction(hwnd),
                ID_MODE_MOUSE=>select_mode(hwnd,5),
                ID_MODE_FLIGHT=>select_mode(hwnd,6),
                ID_FLIGHT_MARK|ID_FLIGHT_ZOOM_IN|ID_FLIGHT_ZOOM_OUT|
                    ID_FLIGHT_BACK|ID_FLIGHT_FORWARD=>flight_action(hwnd,id),
                ID_QUERY_HID=>query_hid(),
                ID_VULKAN_SCENE=>{
                    match std::env::current_exe().and_then(|exe|
                        std::process::Command::new(exe).arg("--vulkan-scene").spawn()) {
                        Ok(_)=>{},
                        Err(e)=>error(hwnd,&format!("Could not start Vulkan scene: {e}")),
                    }
                },
                _=>{},
            }
            // Keep the test textbox focused after toolbar commands. Other
            // modes focus the parent so Space won't activate a button.
            let field=STATE.with(|cell|{
                cell.borrow().as_ref().and_then(|app|
                    (app.mode==4).then(||app.typing_views.as_ref().map(|v|v.input)).flatten())
            });
            if let Some(field)=field {SetFocus(field);} else {SetFocus(hwnd);}
            return 0;
        }
        WM_KEYDOWN if wp as u16==VK_F5=>{
            // F5 acts as pause/resume except in the keyboard key-troubleshooting mode.
            let mode=STATE.with(|cell|cell.borrow().as_ref().map_or(1,|s|s.mode));
            if mode!=2 && mode!=4 {toggle();}
            return 0;
        }
        WM_PAINT=>{paint_reaction(hwnd);return 0;}
        WM_TIMER=>{
            let changed=STATE.with(|cell|{
                let mut state=cell.borrow_mut();
                let Some(app)=state.as_mut()else{return false;};
                let when=now();
                let cpu=current_processor();
                let handler_ticks=GUI_HANDLER_MAX.with(|x|x.replace(0));
                let handler_max=handler_ticks as f64*1000.0/app.frequency as f64;
                app.thread_metrics.gui_pulse(when,handler_max,cpu.group,cpu.number);
                // Bounded: a burst can never force an unbounded UI callback.
                // Generation IDs prevent cross-mode/stale keyboard captures.
                let (events,generation)=app.capture.as_ref()
                    .map_or_else(||(Vec::new(),0),|c|
                        (c.drain(MAX_DRAIN_PER_PULSE),c.generation()));
                if !app.active{
                    app.watchdog.drain();app.capture_watchdog.drain();
                    return false;
                }
                let mut changed=false;
                for event in events{
                    if event.generation==generation {
                        changed|=consume_captured(hwnd,app,event);
                    }
                }
                app.flight.finish(now());
                for gap in app.watchdog.drain(){
                    if matches!(app.mode,2|4|5){
                        app.thread_metrics.gap(false,gap.ended,gap.duration_ms);
                        app.flight.observe(FlightEvent::new(gap.ended,
                            Lane::Watchdog,"GUI_STALL",0,0,
                            gap.duration_ms.round() as i32,0));
                    }
                }
                for gap in app.capture_watchdog.drain(){
                    if matches!(app.mode,2|4|5){
                        app.thread_metrics.gap(true,gap.ended,gap.duration_ms);
                        app.flight.observe(FlightEvent::new(gap.ended,
                            Lane::Watchdog,"CAPTURE_STALL",0,0,
                            gap.duration_ms.round() as i32,0));
                    }
                }
                if app.mode==2 || app.mode==4 {
                    let when=now();
                    for (vk,label,raw_down) in app.keyboard.observed_vk_states(){
                        let os_down=GetAsyncKeyState(vk as i32)<0;
                        app.os_keys.sample(vk,&label,raw_down,os_down,when);
                    }
                }
                if app.mode==3{app.reaction.tick(now())|changed}else{changed}
            });
            if changed{InvalidateRect(hwnd,null(),0);}
            refresh();
            return 0;
        }
        WM_DESTROY=>{KillTimer(hwnd,1);PostQuitMessage(0);return 0;}
        _=>{}
    }
    DefWindowProcW(hwnd,msg,wp,lp)
}

pub fn run()->Result<(),Box<dyn std::error::Error>>{
    if size_of::<usize>()!=8 || size_of::<RawHeader>()!=24
        || size_of::<RawInput>()!=48 || size_of::<RawMouse>()!=24
        || size_of::<RawDevice>()!=16 || size_of::<SaveFile>()!=152 {
        return Err("Windows x64 ABI required".into());
    }
    let mut frequency=0i64;
    unsafe{
        if QueryPerformanceFrequency(&mut frequency)==0||frequency<=0{
            return Err(io::Error::last_os_error().into());
        }
    }
    STATE.with(|cell|{
        *cell.borrow_mut()=Some(State{
            session:Session::new(now(),frequency),active:true,labels:None,
            note:"Live capture active. Select a diagnostic mode or open Vulkan Scene.".into(),
            mode:1,groups:std::array::from_fn(|_|Vec::new()),frequency,
            keyboard:KeyboardDiagnostics::new(frequency),
            os_keys:OsKeyStateMonitor::new(now(),frequency),key_views:None,
            reaction:ReactionTest::default(),reaction_views:None,
            typing:TypingMonitor::new(frequency,now()),typing_views:None,
            trace:DeepTrace::new(frequency,now()),
            mouse_diag:MouseDiagnostics::new(frequency,now()),mouse_views:None,
            polling:PollingMonitor::new(frequency),
            flight:FlightRecorder::new(frequency,now()),flight_views:None,
            watchdog:Watchdog::new(frequency,now),
            capture_watchdog:Watchdog::new(frequency,now),capture:None,
            thread_metrics:ThreadMetrics::new(frequency,now()),
        });
    });
    let cls=wide("MouseKeyboardLatencyGui");
    let title=wide("Mouse / Keyboard Latency — Input Timing Lab");
    unsafe{
        let instance=GetModuleHandleW(null());
        let class=WndClass{style:0,proc:Some(wnd_proc),cls_extra:0,wnd_extra:0,
            instance,icon:null_mut(),cursor:null_mut(),brush:6 as H,
            menu:null(),class:cls.as_ptr()};
        if RegisterClassW(&class)==0{
            return Err(io::Error::last_os_error().into());
        }
        let hwnd=CreateWindowExW(0,cls.as_ptr(),title.as_ptr(),
            WS_CAPTION|WS_SYSMENU|WS_MINIMIZEBOX|WS_VISIBLE,
            100,45,950,966,null_mut(),null_mut(),instance,null());
        if hwnd.is_null(){return Err(io::Error::last_os_error().into());}
        // Register Raw Input on a separate message-only window/thread.
        // Never also register it on the GUI: Windows permits only one receiver
        // per input class per process, and the last registration wins.
        let heartbeat=STATE.with(|cell|{
            cell.borrow().as_ref().unwrap().capture_watchdog.heartbeat()
        });
        let capture=match InputCapture::start(frequency,heartbeat){
            Ok(worker)=>worker,
            Err(e)=>{
                error(hwnd,&format!("Could not start isolated Raw Input receiver: {e}"));
                DestroyWindow(hwnd);return Err(e.into());
            }
        };
        STATE.with(|cell|{
            let mut state=cell.borrow_mut();
            let app=state.as_mut().unwrap();
            app.capture=Some(capture);
            sync_capture(app);
        });
        // 20 Hz display refresh; timestamps remain captured immediately on input.
        SetTimer(hwnd,1,50,null_mut());
        ShowWindow(hwnd,SW_SHOW);UpdateWindow(hwnd);
        let mut m:Msg=zeroed();
        loop{
            let result=GetMessageW(&mut m,null_mut(),0,0);
            if result==0{break;}
            if result<0{return Err(io::Error::last_os_error().into());}
            TranslateMessage(&m);DispatchMessageW(&m);
        }
    }
    // Stop the independent capture receiver before returning from run().
    STATE.with(|cell|{*cell.borrow_mut()=None;});
    Ok(())
}
