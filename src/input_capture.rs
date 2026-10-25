//! Independent Windows Raw Input receiver.
//! One dedicated message-only window/thread timestamps keyboard and mouse WM_INPUT.
//! Its WndProc only copies a fixed-size event into a bounded nonblocking queue;
//! it never touches GUI controls, files, registry, timers' resolution or drivers.
use crate::watchdog::Heartbeat;
use std::{cell::RefCell,ffi::c_void,io,mem::{size_of,zeroed},
    ptr::{null,null_mut},sync::{Arc,atomic::{AtomicBool,AtomicU64,Ordering},
    mpsc::{self,Receiver,SyncSender,TrySendError}},thread::{self,JoinHandle}};

type H=*mut c_void;
const WM_INPUT:u32=0x00FF;
const WM_TIMER:u32=0x0113;
const WM_CLOSE:u32=0x0010;
const WM_DESTROY:u32=0x0002;
const RID_INPUT:u32=0x10000003;
const RIDEV_INPUTSINK:u32=0x100;
const QS_KEY:u32=1;
const QS_RAWINPUT:u32=0x400;
const HWND_MESSAGE:H=(-3isize) as H;
const MAX_QUEUE:usize=65_536;
pub const MAX_DRAIN_PER_PULSE:usize=4096;
#[repr(C)]
struct WndClass{style:u32,proc:Option<unsafe extern "system" fn(H,u32,usize,isize)->isize>,
    cls_extra:i32,wnd_extra:i32,instance:H,icon:H,cursor:H,brush:H,
    menu:*const u16,class:*const u16}
#[repr(C)]
struct RawDevice{page:u16,usage:u16,flags:u32,hwnd:H}
#[repr(C)]
#[derive(Clone,Copy)]
struct RawHeader{kind:u32,size:u32,device:H,wp:usize}
#[repr(C)]
#[derive(Clone,Copy)]
struct RawMouse{flags:u16,padding:u16,buttons:u32,raw_buttons:u32,
    last_x:i32,last_y:i32,extra:u32}
#[repr(C)]
#[derive(Clone,Copy)]
struct RawKeyboard{make:u16,flags:u16,reserved:u16,key:u16,message:u32,extra:u32}
#[repr(C)]
#[derive(Clone,Copy)]
union RawData{mouse:RawMouse,key:RawKeyboard,padding:[u64;3]}
#[repr(C)]
#[derive(Clone,Copy)]
struct RawInput{header:RawHeader,data:RawData}
#[repr(C)]
struct Msg{hwnd:H,msg:u32,wp:usize,lp:isize,time:u32,
    point:[i32;2],private:u32}
#[repr(C)]
#[derive(Clone,Copy,Default)]
pub struct Processor{pub group:u16,pub number:u8}
#[repr(C)]
struct ProcessorNumber{group:u16,number:u8,reserved:u8}
#[derive(Clone,Copy)]
pub enum Kind {
    Mouse{flags:u16,buttons:u16,dx:i32,dy:i32,cursor_x:i32,cursor_y:i32},
    Keyboard{vk:u16,scan:u16,flags:u16},
}
#[derive(Clone,Copy)]
pub struct CapturedInput{
    pub tick:i64,
    pub generation:u64,
    pub device:usize,
    pub kind:Kind,
    pub queue_age_ms:u32,
    pub queue_bits:u32,
    pub cpu:Processor,
    pub callback_us:f64,
}
#[link(name="user32")]
unsafe extern "system"{
    fn RegisterClassW(class:*const WndClass)->u16;
    fn CreateWindowExW(ex:u32,class:*const u16,name:*const u16,style:u32,
        x:i32,y:i32,w:i32,h:i32,parent:H,menu:H,instance:H,param:*const c_void)->H;
    fn DefWindowProcW(hwnd:H,msg:u32,wp:usize,lp:isize)->isize;
    fn DestroyWindow(hwnd:H)->i32;
    fn GetMessageW(msg:*mut Msg,hwnd:H,min:u32,max:u32)->i32;
    fn TranslateMessage(msg:*const Msg)->i32;
    fn DispatchMessageW(msg:*const Msg)->isize;
    fn PostMessageW(hwnd:H,msg:u32,wp:usize,lp:isize)->i32;
    fn PostQuitMessage(code:i32);
    fn RegisterRawInputDevices(devices:*const RawDevice,count:u32,size:u32)->i32;
    fn GetRawInputData(handle:H,cmd:u32,out:H,bytes:*mut u32,header:u32)->u32;
    fn GetQueueStatus(flags:u32)->u32;
    fn GetMessageTime()->i32;
    fn SetTimer(hwnd:H,id:usize,ms:u32,callback:H)->usize;
    fn KillTimer(hwnd:H,id:usize)->i32;
    fn GetCursorPos(p:*mut [i32;2])->i32;
}
#[link(name="kernel32")]
unsafe extern "system"{
    fn GetModuleHandleW(name:*const u16)->H;
    fn QueryPerformanceCounter(t:*mut i64)->i32;
    fn GetTickCount64()->u64;
    fn GetCurrentProcessorNumberEx(out:*mut ProcessorNumber);
}
fn qpc()->i64{let mut t=0;unsafe{QueryPerformanceCounter(&mut t);}t}
pub fn current_processor()->Processor{
    let mut p=ProcessorNumber{group:0,number:0,reserved:0};
    unsafe{GetCurrentProcessorNumberEx(&mut p);}
    Processor{group:p.group,number:p.number}
}
fn wide(s:&str)->Vec<u16>{s.encode_utf16().chain(Some(0)).collect()}
struct Context {
    tx:SyncSender<CapturedInput>,
    enabled:Arc<AtomicBool>,
    generation:Arc<AtomicU64>,
    dropped:Arc<AtomicU64>,
    heartbeat:Heartbeat,
    freq:i64,
    // Data needed only on the input thread, not referenced from the GUI.
}
thread_local!{static CONTEXT:RefCell<Option<Context>>=const{RefCell::new(None)};}
unsafe extern "system" fn input_proc(hwnd:H,msg:u32,wp:usize,lp:isize)->isize{
    match msg{
        WM_INPUT=>{
            let tick=qpc();
            CONTEXT.with(|slot|{
                let ctx=slot.borrow();
                let Some(ctx)=ctx.as_ref()else{return;};
                ctx.heartbeat.beat(tick);
                if !ctx.enabled.load(Ordering::Acquire){return;}
                let generation=ctx.generation.load(Ordering::Acquire);
                let mut raw:RawInput=zeroed();
                let mut bytes=size_of::<RawInput>() as u32;
                let got=GetRawInputData(lp as H,RID_INPUT,
                    (&mut raw as *mut RawInput).cast(),&mut bytes,
                    size_of::<RawHeader>() as u32);
                if got==u32::MAX||(got as usize)<size_of::<RawHeader>(){return;}
                let kind=match raw.header.kind{
                    0 if got as usize>=size_of::<RawHeader>()+size_of::<RawMouse>()=>{
                        let m=raw.data.mouse;
                        let mut p=[0,0];
                        let _=GetCursorPos(&mut p);
                        Kind::Mouse{flags:m.flags,buttons:m.buttons as u16,
                            dx:m.last_x,dy:m.last_y,cursor_x:p[0],cursor_y:p[1]}
                    },
                    1 if got as usize>=size_of::<RawHeader>()+size_of::<RawKeyboard>()=>{
                        let k=raw.data.key;
                        Kind::Keyboard{vk:k.key,scan:k.make,flags:k.flags}
                    },
                    _=>return
                };
                let age=(GetTickCount64() as u32).wrapping_sub(GetMessageTime() as u32);
                let age=if age<=5000{age}else{0};
                let bits=GetQueueStatus(QS_KEY|QS_RAWINPUT)>>16;
                let cpu=current_processor();
                let callback_us=qpc().saturating_sub(tick) as f64*1_000_000.0/ctx.freq as f64;
                let event=CapturedInput{
                    tick,generation,device:raw.header.device as usize,kind,
                    queue_age_ms:age,queue_bits:bits,cpu,callback_us
                };
                if let Err(TrySendError::Full(_))=ctx.tx.try_send(event){
                    ctx.dropped.fetch_add(1,Ordering::Relaxed);
                }
            });
            // RIM_INPUT cleanup must still be performed by DefWindowProcW.
            DefWindowProcW(hwnd,msg,wp,lp)
        },
        WM_TIMER=>{
            CONTEXT.with(|slot|{
                if let Some(ctx)=slot.borrow().as_ref(){ctx.heartbeat.beat(qpc());}
            });
            0
        },
        WM_CLOSE=>{DestroyWindow(hwnd);0},
        WM_DESTROY=>{KillTimer(hwnd,1);PostQuitMessage(0);0},
        _=>DefWindowProcW(hwnd,msg,wp,lp),
    }
}
pub struct InputCapture{
    rx:Receiver<CapturedInput>,
    hwnd:isize,
    worker:Option<JoinHandle<()>>,
    enabled:Arc<AtomicBool>,
    generation:Arc<AtomicU64>,
    dropped:Arc<AtomicU64>,
}
impl InputCapture{
    pub fn start(freq:i64,beat:Heartbeat)->io::Result<Self>{
        let (tx,rx)=mpsc::sync_channel(MAX_QUEUE);
        let (init_tx,init_rx)=mpsc::sync_channel(1);
        let enabled=Arc::new(AtomicBool::new(false));
        let generation=Arc::new(AtomicU64::new(1));
        let dropped=Arc::new(AtomicU64::new(0));
        let e=enabled.clone();let g=generation.clone();let d=dropped.clone();
        let worker=thread::Builder::new()
            .name("input-timing-raw-capture".into()).spawn(move ||{
                // This thread owns the only Raw Input registration for the GUI process.
                // There is no UI work in its message pump.
                let cls=wide("InputTimingLabPassiveRawCapture");
                unsafe {
                    let instance=GetModuleHandleW(null());
                    let wc=WndClass{style:0,proc:Some(input_proc),cls_extra:0,
                        wnd_extra:0,instance,icon:null_mut(),cursor:null_mut(),
                        brush:null_mut(),menu:null(),class:cls.as_ptr()};
                    if RegisterClassW(&wc)==0{
                        let _=init_tx.send(Err(io::Error::last_os_error().to_string()));
                        return;
                    }
                    CONTEXT.with(|slot|*slot.borrow_mut()=Some(Context{
                        tx,enabled:e,generation:g,dropped:d,heartbeat:beat,freq
                    }));
                    let hwnd=CreateWindowExW(0,cls.as_ptr(),cls.as_ptr(),0,
                        0,0,0,0,HWND_MESSAGE,null_mut(),instance,null());
                    if hwnd.is_null(){
                        let _=init_tx.send(Err(io::Error::last_os_error().to_string()));
                        return;
                    }
                    let devices=[
                        RawDevice{page:1,usage:2,flags:RIDEV_INPUTSINK,hwnd},
                        RawDevice{page:1,usage:6,flags:RIDEV_INPUTSINK,hwnd},
                    ];
                    if RegisterRawInputDevices(devices.as_ptr(),2,size_of::<RawDevice>() as u32)==0{
                        let err=io::Error::last_os_error().to_string();
                        DestroyWindow(hwnd);
                        let _=init_tx.send(Err(err));return;
                    }
                    if SetTimer(hwnd,1,20,null_mut())==0{
                        let err=io::Error::last_os_error().to_string();
                        DestroyWindow(hwnd);
                        let _=init_tx.send(Err(err));return;
                    }
                    if init_tx.send(Ok(hwnd as isize)).is_err(){
                        DestroyWindow(hwnd);return;
                    }
                    let mut message:Msg=zeroed();
                    while GetMessageW(&mut message,null_mut(),0,0)>0{
                        TranslateMessage(&message);DispatchMessageW(&message);
                    }
                    CONTEXT.with(|slot|slot.borrow_mut().take());
                }
            })?;
        let hwnd=match init_rx.recv(){
            Ok(Ok(hwnd))=>hwnd,
            Ok(Err(why))=>{
                let _=worker.join();
                return Err(io::Error::other(format!("Raw Input capture thread: {why}")));
            },
            Err(_)=>{
                let _=worker.join();
                return Err(io::Error::other("Raw Input thread exited before starting"));
            }
        };
        Ok(Self{rx,hwnd,worker:Some(worker),enabled,generation,dropped})
    }
    pub fn set_enabled(&self,enabled:bool){
        self.generation.fetch_add(1,Ordering::SeqCst);
        self.enabled.store(enabled,Ordering::SeqCst);
    }
    pub fn generation(&self)->u64{self.generation.load(Ordering::SeqCst)}
    pub fn drain(&self,max:usize)->Vec<CapturedInput>{
        let mut result=Vec::with_capacity(max.min(512));
        while result.len()<max {
            match self.rx.try_recv(){Ok(event)=>result.push(event),Err(_)=>break}
        }
        result
    }
    pub fn dropped(&self)->u64{self.dropped.load(Ordering::Relaxed)}
}
impl Drop for InputCapture{
    fn drop(&mut self){
        self.enabled.store(false,Ordering::Release);
        unsafe{PostMessageW(self.hwnd as H,WM_CLOSE,0,0);}
        if let Some(worker)=self.worker.take(){let _=worker.join();}
    }
}
