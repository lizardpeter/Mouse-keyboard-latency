//! Native Win32 GUI and Windows Raw Input capture. No external dependencies.
//! All UI events and Raw Input callbacks run on the owning GUI thread.

use crate::core::Session;
use std::{cell::RefCell, ffi::c_void, io, mem::{size_of, zeroed}, path::Path,
    ptr::{null, null_mut}};

type H = *mut c_void;
type WP = usize;
type LP = isize;

const WM_CREATE:u32=1; const WM_DESTROY:u32=2; const WM_COMMAND:u32=0x111;
const WM_INPUT:u32=0xFF; const WM_TIMER:u32=0x113; const WM_SETFONT:u32=0x30;
const WM_CTLCOLORSTATIC:u32=0x138; const WM_ERASEBKGND:u32=0x14;
const WM_SIZE:u32=5; const WM_KEYDOWN:u32=0x100;
const RIDEV_INPUTSINK:u32=0x100; const RID_INPUT:u32=0x10000003;
const RI_MOUSE_LEFT_BUTTON_DOWN:u16=1; const RI_MOUSE_LEFT_BUTTON_UP:u16=2;
const RI_KEY_BREAK:u16=1; const VK_SPACE:u16=0x20; const VK_F5:u16=0x74;
const WS_CHILD:u32=0x40000000; const WS_VISIBLE:u32=0x10000000;
const WS_TABSTOP:u32=0x10000; const WS_BORDER:u32=0x800000;
const WS_CAPTION:u32=0xC00000; const WS_SYSMENU:u32=0x80000;
const WS_MINIMIZEBOX:u32=0x20000; const SS_LEFT:u32=0;
const BS_PUSHBUTTON:u32=0; const SW_SHOW:i32=5;
const ID_START:usize=101; const ID_RESET:usize=102; const ID_SAVE:usize=103;
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
    fn SetWindowTextW(hwnd:H,text:*const u16)->i32;
    fn SendMessageW(hwnd:H,message:u32,wp:WP,lp:LP)->LP;
    fn SetTimer(hwnd:H,id:usize,period:u32,callback:H)->usize;
    fn KillTimer(hwnd:H,id:usize)->i32;
    fn ShowWindow(hwnd:H,cmd:i32)->i32;
    fn UpdateWindow(hwnd:H)->i32;
    fn GetCursorPos(out:*mut Point)->i32;
    fn ScreenToClient(hwnd:H,point:*mut Point)->i32;
    fn MessageBoxW(hwnd:H,text:*const u16,caption:*const u16,flags:u32)->i32;
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
#[link(name="kernel32")]
extern "system" {
    fn GetModuleHandleW(name:*const u16)->H;
    fn QueryPerformanceFrequency(out:*mut i64)->i32;
    fn QueryPerformanceCounter(out:*mut i64)->i32;
}

struct Labels {
    start:H, recording:H, count:H, mouse:H, keyboard:H, median:H,
    secondary:H, recent:H, notification:H,
}
struct State { session:Session, active:bool, labels:Option<Labels>, note:String }
thread_local! { static STATE:RefCell<Option<State>>=const{RefCell::new(None)}; }

fn now()->i64 {let mut v=0; unsafe{QueryPerformanceCounter(&mut v);} v}
unsafe fn control(parent:H,instance:H,class:&str,text:&str,id:usize,
    x:i32,y:i32,w:i32,h:i32,style:u32,font:H)->H {
    let hwnd=CreateWindowExW(0,wide(class).as_ptr(),wide(text).as_ptr(),
        WS_VISIBLE|WS_CHILD|style,x,y,w,h,parent,id as H,instance,null());
    if !font.is_null(){SendMessageW(hwnd,WM_SETFONT,font as usize,1);}
    hwnd
}
unsafe fn text(parent:H,instance:H,value:&str,x:i32,y:i32,w:i32,h:i32,font:H)->H{
    control(parent,instance,"STATIC",value,0,x,y,w,h,SS_LEFT,font)
}
fn set_text(hwnd:H,value:&str){if !hwnd.is_null(){unsafe{SetWindowTextW(hwnd,wide(value).as_ptr());}}}
unsafe fn font(height:i32,weight:i32)->H {
    CreateFontW(-height,0,0,0,weight,0,0,0,1,0,0,5,0,wide("Segoe UI").as_ptr())
}

fn refresh() {
    STATE.with(|cell|{
        let state=cell.borrow();
        let Some(app)=state.as_ref() else{return;};
        let Some(l)=app.labels.as_ref() else{return;};
        set_text(l.start,if app.active {"Pause"} else {"Start recording"});
        set_text(l.recording,if app.active {"●  RECORDING  ·  Input is being captured"}
            else {"●  PAUSED  ·  Press Start recording"});
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
        let mut lines=String::from("TRIAL              FIRST INPUT              SIGNED GAP\r\n\
            ─────────────────────────────────────────────────\r\n");
        for t in app.session.trials().iter().rev().take(9) {
            let first=if t.keyboard_minus_mouse_ms>0.0{"MOUSE"}
                else if t.keyboard_minus_mouse_ms<0.0{"KEYBOARD"}else{"TIE"};
            lines.push_str(&format!("{:>4}                     {:<12}           {:+.3} ms\r\n",
                t.number,first,t.keyboard_minus_mouse_ms));
        }
        if app.session.trial_count()==0{
            lines.push_str("\r\nClick Start recording, then click the mouse and press Space closely together.");
        }
        set_text(l.recent,&lines);
        set_text(l.notification,&app.note);
    });
}
fn toggle(){
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            app.session.break_pair();
            app.active=!app.active;
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
            app.session.reset(now());
            app.note="All recorded pairs cleared.".into();
        }
    });
    refresh();
}
fn error(hwnd:H,message:&str){
    unsafe{MessageBoxW(hwnd,wide(message).as_ptr(),wide("Input Timing Lab").as_ptr(),ERROR_ICON);}
}
fn save(hwnd:H){
    let before=STATE.with(|cell|{
        let mut state=cell.borrow_mut();
        let Some(app)=state.as_mut()else{return false};
        let prior=app.active;
        app.active=false;app.session.break_pair();
        prior
    });
    let mut path=[0u16;1024];
    let suggested=wide("mouse_keyboard_results.csv");
    path[..suggested.len()].copy_from_slice(&suggested);
    let filter=wide("CSV files (*.csv)\0*.csv\0All files (*.*)\0*.*\0");
    let title=wide("Save input timing results");
    let ext=wide("csv");
    let mut details=SaveFile{
        size:size_of::<SaveFile>() as u32,owner:hwnd,instance:null_mut(),
        filter:filter.as_ptr(),custom:null_mut(),custom_max:0,filter_index:1,
        file:path.as_mut_ptr(),file_max:path.len() as u32,
        file_title:null_mut(),file_title_max:0,initial_dir:null(),
        title:title.as_ptr(),flags:OFN_OVERWRITEPROMPT|OFN_PATHMUSTEXIST,
        file_offset:0,file_extension:0,default_ext:ext.as_ptr(),custom_data:0,
        hook:null_mut(),template:null(),reserved:null_mut(),reserved_num:0,flags_ex:0,
    };
    if unsafe{GetSaveFileNameW(&mut details)}!=0{
        let end=path.iter().position(|&c|c==0).unwrap_or(path.len());
        let filename=String::from_utf16_lossy(&path[..end]);
        let result=STATE.with(|cell|{
            cell.borrow().as_ref().unwrap().session.save_csv(Path::new(&filename))
        });
        match result{
            Ok(())=>STATE.with(|cell|{
                if let Some(app)=cell.borrow_mut().as_mut(){
                    app.note=format!("Saved CSV to {}",filename);
                }
            }),
            Err(e)=>error(hwnd,&format!("Unable to save CSV: {e}")),
        }
    }
    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            app.session.break_pair();app.active=before;
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
    text(hwnd,instance,"INPUT TIMING LAB",30,21,745,47,f_title);
    text(hwnd,instance,
        "MOUSE LEFT CLICK  vs.  KEYBOARD SPACE   •   Windows Raw Input / QueryPerformanceCounter",
        32,71,735,22,f_small);
    let start=control(hwnd,instance,"BUTTON","Start recording",ID_START,
        31,111,175,42,WS_TABSTOP|BS_PUSHBUTTON,f_medium);
    control(hwnd,instance,"BUTTON","Reset",ID_RESET,
        220,111,110,42,WS_TABSTOP|BS_PUSHBUTTON,f_medium);
    control(hwnd,instance,"BUTTON","Save CSV...",ID_SAVE,
        343,111,130,42,WS_TABSTOP|BS_PUSHBUTTON,f_medium);
    let recording=text(hwnd,instance,"",494,117,350,29,f_medium);

    text(hwnd,instance,"COMPLETED PAIRS",32,184,200,23,f_small);
    text(hwnd,instance,"MOUSE FIRST",230,184,190,23,f_small);
    text(hwnd,instance,"KEYBOARD FIRST",428,184,190,23,f_small);
    text(hwnd,instance,"MEDIAN ABSOLUTE GAP",631,184,213,23,f_small);
    let count=text(hwnd,instance,"0",32,215,184,53,f_large);
    let mouse=text(hwnd,instance,"0",230,215,184,53,f_large);
    let keyboard=text(hwnd,instance,"0",428,215,184,53,f_large);
    let median=text(hwnd,instance,"—",631,215,213,53,f_large);

    text(hwnd,instance,"DETAILED STATISTICS",32,291,400,25,f_medium);
    let secondary=text(hwnd,instance,"",32,327,812,33,f_small);
    text(hwnd,instance,"MOST RECENT INPUT PAIRS",32,375,400,25,f_medium);
    let recent=text(hwnd,instance,"",32,410,812,235,f_small);
    text(hwnd,instance,
        "Positive gap = MOUSE first   •   Negative gap = KEYBOARD first   •   F5 = pause/resume",
        32,652,810,25,f_small);
    let notification=text(hwnd,instance,"",32,687,800,33,f_small);
    text(hwnd,instance,
        "Measured: application-observed arrival time. This is not absolute hardware latency.",
        32,731,812,24,f_small);

    STATE.with(|cell|{
        if let Some(app)=cell.borrow_mut().as_mut(){
            app.labels=Some(Labels{
                start,recording,count,mouse,keyboard,median,secondary,recent,notification
            });
        }
    });
    refresh();
    // Fonts are intentionally process-lifetime resources; statics retain references.
}

unsafe extern "system" fn wnd_proc(hwnd:H,msg:u32,wp:WP,lp:LP)->LP{
    match msg{
        WM_CREATE=>{make_gui(hwnd);return 0;}
        WM_INPUT=>{
            // Timestamp BEFORE parsing packet or updating UI. No rendering in callback.
            let tick=now();
            let mut raw:RawInput=zeroed();
            let mut size=size_of::<RawInput>() as u32;
            let got=GetRawInputData(lp as H,RID_INPUT,
                (&mut raw as *mut RawInput).cast(),&mut size,size_of::<RawHeader>() as u32);
            if got!=u32::MAX{
                STATE.with(|cell|{
                    let mut state=cell.borrow_mut();
                    let Some(app)=state.as_mut()else{return};
                    if !app.active{return};
                    match raw.header.kind{
                        0 if got as usize>=size_of::<RawHeader>()+size_of::<RawMouse>()=>{
                            let bits=raw.data.mouse.buttons as u16;
                            // Ignore clicks on the GUI controls. Elsewhere or background: capture.
                            let mut p=Point::default();
                            let over_controls=GetCursorPos(&mut p)!=0
                                && ScreenToClient(hwnd,&mut p)!=0
                                && p.x>=0&&p.x<850&&p.y>=0&&p.y<173;
                            if bits&RI_MOUSE_LEFT_BUTTON_UP!=0{
                                app.session.mouse_button(false,tick);
                            }
                            if bits&RI_MOUSE_LEFT_BUTTON_DOWN!=0&&!over_controls{
                                app.session.mouse_button(true,tick);
                            }
                        }
                        1 if got as usize>=size_of::<RawHeader>()+size_of::<RawKeyboard>()=>{
                            let key=raw.data.key;
                            if key.key==VK_SPACE {
                                app.session.space_key(key.flags&RI_KEY_BREAK==0,tick);
                            }
                        }
                        _=>{}
                    }
                });
            }
            return DefWindowProcW(hwnd,msg,wp,lp);
        }
        WM_COMMAND=>{
            match wp&0xFFFF{
                ID_START=>toggle(),ID_RESET=>reset(),ID_SAVE=>save(hwnd),_=>{},
            }
            return 0;
        }
        WM_KEYDOWN if wp as u16==VK_F5=>{toggle();return 0;}
        WM_TIMER=>{refresh();return 0;}
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
            session:Session::new(now(),frequency),active:false,labels:None,
            note:"Ready. Your capture remains on this computer.".into(),
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
            100,90,884,810,null_mut(),null_mut(),instance,null());
        if hwnd.is_null(){return Err(io::Error::last_os_error().into());}
        let devices=[
            RawDevice{page:1,usage:2,flags:RIDEV_INPUTSINK,hwnd},
            RawDevice{page:1,usage:6,flags:RIDEV_INPUTSINK,hwnd},
        ];
        if RegisterRawInputDevices(devices.as_ptr(),2,size_of::<RawDevice>() as u32)==0{
            let e=io::Error::last_os_error();error(hwnd,&format!("Raw Input: {e}"));
            DestroyWindow(hwnd);return Err(e.into());
        }
        SetTimer(hwnd,1,120,null_mut());
        ShowWindow(hwnd,SW_SHOW);UpdateWindow(hwnd);
        let mut m:Msg=zeroed();
        loop{
            let result=GetMessageW(&mut m,null_mut(),0,0);
            if result==0{break;}
            if result<0{return Err(io::Error::last_os_error().into());}
            TranslateMessage(&m);DispatchMessageW(&m);
        }
    }
    Ok(())
}
