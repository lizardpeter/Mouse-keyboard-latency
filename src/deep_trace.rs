//! Passive, process-local Win32 message tracing. No hooks, ETW control,
//! USB filter drivers, registry changes, power-setting changes or system writes.
//! The only persisted output is an explicitly user-requested export.
use std::fmt::Write;

const CAP: usize = 100_000;
#[derive(Clone, Copy)]
pub struct TraceEvent {
    pub qpc: i64, pub kind: &'static str, pub vk: u16, pub scan: u16,
    pub repeats: u16, pub prev_down: bool, pub transition: bool,
    pub msg_age_ms: u32, pub queue_bits: u32, pub device: usize,
    pub dx: i32, pub dy: i32, pub cursor_x: i32, pub cursor_y: i32,
    pub buttons: u16,
}
impl TraceEvent {
    pub fn keyboard_msg(qpc:i64, kind:&'static str, vk:usize, lp:isize,
                        age:u32, queued:u32)->Self {
        let bits=lp as usize;
        Self {
            qpc,kind,vk:vk as u16,scan:((bits>>16)&255) as u16,
            repeats:(bits&0xffff) as u16,
            prev_down:(bits&(1<<30))!=0,transition:(bits&(1<<31))!=0,
            msg_age_ms:age,queue_bits:queued,device:0,dx:0,dy:0,
            cursor_x:0,cursor_y:0,buttons:0,
        }
    }
    pub fn raw(qpc:i64, kind:&'static str, vk:u16, scan:u16,
               age:u32, queued:u32,device:usize,dx:i32,dy:i32,
               cursor_x:i32,cursor_y:i32,buttons:u16)->Self {
        Self {qpc,kind,vk,scan,repeats:0,prev_down:false,transition:false,
            msg_age_ms:age,queue_bits:queued,device,dx,dy,cursor_x,cursor_y,buttons}
    }
}
pub struct DeepTrace {
    frequency: i64, epoch: i64,
    pub events: Vec<TraceEvent>, pub dropped:u64,
    pub key_msg_down:u64,pub key_msg_up:u64,
    pub combined_repeats:u64,pub raw_keyboard:u64,pub raw_mouse:u64,
    pub large_dispatch_gaps:u64,pub largest_dispatch_gap_ms:f64,
    last_dispatch:Option<i64>, pub current_queue_bits:u32,
}
impl DeepTrace {
    pub fn new(frequency:i64,epoch:i64)->Self {
        Self {frequency,epoch,events:Vec::with_capacity(CAP),dropped:0,
            key_msg_down:0,key_msg_up:0,combined_repeats:0,
            raw_keyboard:0,raw_mouse:0,large_dispatch_gaps:0,
            largest_dispatch_gap_ms:0.0,last_dispatch:None,current_queue_bits:0}
    }
    pub fn observe_dispatch(&mut self,qpc:i64,queued:u32) {
        if let Some(last)=self.last_dispatch {
            let gap=(qpc.saturating_sub(last)) as f64*1000.0/self.frequency as f64;
            if gap>=50.0 {self.large_dispatch_gaps+=1;}
            self.largest_dispatch_gap_ms=self.largest_dispatch_gap_ms.max(gap);
        }
        self.last_dispatch=Some(qpc);
        self.current_queue_bits=queued;
    }
    pub fn record(&mut self,event:TraceEvent){
        match event.kind {
            "WM_KEYDOWN"|"WM_SYSKEYDOWN"=>{
                self.key_msg_down+=1;
                self.combined_repeats+=event.repeats.saturating_sub(1) as u64;
            }
            "WM_KEYUP"|"WM_SYSKEYUP"=>self.key_msg_up+=1,
            "RAW_KEY"=>self.raw_keyboard+=1,
            "RAW_MOUSE"=>self.raw_mouse+=1,
            _=>{}
        }
        if self.events.len()<CAP { self.events.push(event); }
        else {self.dropped+=1;}
    }
    pub fn summary(&self)->String {
        format!("Win32 key down/up: {}/{}  |  Combined repeats: {}  |  Raw keyboard/mouse: {}/{}  |  Dispatch gaps >=50ms: {} (max {:.1}ms)",
            self.key_msg_down,self.key_msg_up,self.combined_repeats,
            self.raw_keyboard,self.raw_mouse,self.large_dispatch_gaps,
            self.largest_dispatch_gap_ms)
    }
    fn to_ms(&self,tick:i64)->f64 {
        tick.saturating_sub(self.epoch) as f64*1000.0/self.frequency as f64
    }
    pub fn append_report(&self,out:&mut String) {
        let _=writeln!(out,"\n\nDEEP TRACE — PROCESS-LOCAL OBSERVATIONS (READ ONLY)");
        let _=writeln!(out,"No system ETW session was started. No HID driver was installed.");
        let _=writeln!(out,"Keyboard MSG data is available only when THIS app owns keyboard focus.");
        let _=writeln!(out,"RawInput queue bits show queue state, not the contents of an OS/kernel buffer.");
        let _=writeln!(out,"MSG age is approximate, coarse, and not physical/USB latency.");
        let _=writeln!(out,"Long dispatch gaps include intentional inactivity and are not automatically stalls.");
        let _=writeln!(out,"{}",self.summary());
        let _=writeln!(out,"Events: {} (dropped after cap: {})",self.events.len(),self.dropped);
        let _=writeln!(out,"time_ms | type | vk | scan | key_repeat_count | prev_down | transition | queue_age_ms | queue_bits | device_handle | raw_dx | raw_dy | cursor_x | cursor_y | mouse_buttons");
        for e in &self.events {
            let _=writeln!(out,"{:.3} | {} | {} | {} | {} | {} | {} | {} | 0x{:04x} | 0x{:x} | {} | {} | {} | {} | 0x{:04x}",
                self.to_ms(e.qpc),e.kind,e.vk,e.scan,e.repeats,e.prev_down,
                e.transition,e.msg_age_ms,e.queue_bits,e.device,
                e.dx,e.dy,e.cursor_x,e.cursor_y,e.buttons);
        }
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn repeat_count_and_gap_observed_without_invented_hardware_times() {
        let mut t=DeepTrace::new(1_000_000,0);
        t.observe_dispatch(100,0);
        t.observe_dispatch(60_200,0x20);
        t.record(TraceEvent::keyboard_msg(60_200,"WM_KEYDOWN",68,0x4000_0004,1,0x20));
        assert_eq!(t.combined_repeats,3);
        assert_eq!(t.large_dispatch_gaps,1);
        assert!(t.summary().contains("Combined repeats: 3"));
    }
}