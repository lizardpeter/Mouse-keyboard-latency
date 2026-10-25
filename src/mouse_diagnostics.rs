//! Passive Raw Input mouse sampling compared to sampled Windows cursor movement.
//! Raw counts and pointer pixels are different coordinate systems when pointer
//! acceleration/scaling is active; never label their difference as input latency.
use std::fmt::Write;
const CAP:usize=100_000;
#[derive(Clone,Copy)]
pub struct MouseSample {
    pub time:i64,pub device:usize,pub dx:i32,pub dy:i32,
    pub cursor_x:i32,pub cursor_y:i32,pub absolute:bool,
    pub buttons:u16, pub interval_ms:Option<f64>,
}
pub struct MouseDiagnostics {
    frequency:i64,epoch:i64,last_motion:Option<i64>,
    pub samples:Vec<MouseSample>,pub dropped:u64,
    pub motion:u64,pub buttons:u64,pub accumulated_x:i64,pub accumulated_y:i64,
    interval_ms:Vec<f64>,
}
impl MouseDiagnostics {
    pub fn new(frequency:i64,epoch:i64)->Self{
        Self{frequency,epoch,last_motion:None,samples:Vec::with_capacity(CAP),
             dropped:0,motion:0,buttons:0,accumulated_x:0,accumulated_y:0,
             interval_ms:Vec::with_capacity(CAP)}
    }
    pub fn observe(&mut self,time:i64,device:usize,dx:i32,dy:i32,
                   absolute:bool,buttons:u16,x:i32,y:i32) {
        let movement=dx!=0 || dy!=0;
        let interval=if movement {
            let gap=self.last_motion.map(|last|
                time.saturating_sub(last) as f64 * 1000.0/self.frequency as f64);
            self.last_motion=Some(time); self.motion+=1;
            self.accumulated_x+=dx as i64; self.accumulated_y+=dy as i64;
            if let Some(ms)=gap {
                if ms>0.0 && self.interval_ms.len()<CAP {self.interval_ms.push(ms);}
            }
            gap
        } else {None};
        if buttons!=0 {self.buttons+=1;}
        let sample=MouseSample{time,device,dx,dy,cursor_x:x,cursor_y:y,absolute,
            buttons,interval_ms:interval};
        if self.samples.len()<CAP{self.samples.push(sample)}
        else{self.dropped+=1}
    }
    pub fn frequency_stats(&self)->Option<(f64,f64,f64)> {
        if self.interval_ms.is_empty(){return None;}
        let mut values=self.interval_ms.clone();
        values.sort_by(|a,b|a.total_cmp(b));
        let median=values[values.len()/2];
        let p95=values[((values.len()-1) as f64*0.95).round() as usize];
        let hz=1000.0/median;
        Some((hz,median,p95))
    }
    pub fn live_stats(&self)->String {
        let estimate=self.frequency_stats()
            .map_or_else(||"Move the mouse to measure report spacing".to_string(),
                |(hz,median,p95)|format!(
                    "Observed movement-event rate ~{hz:.0} Hz  |  median {median:.3}ms  |  p95 gap {p95:.3}ms"));
        format!("{estimate}\r\nRaw movement events: {}  |  button reports: {}  |  raw accumulated: ({},{})\r\nWindows cursor coordinates are sampled at Raw Input dispatch; do not equate them to raw counts or physical movement.",
            self.motion,self.buttons,self.accumulated_x,self.accumulated_y)
    }
    pub fn recent(&self)->String {
        let mut lines=self.samples.iter().rev().take(15).map(|s|{
            let ms=s.time.saturating_sub(self.epoch) as f64*1000.0/self.frequency as f64;
            format!("{ms:>9.2}ms   raw ({:>5},{:>5})  pointer ({:>5},{:>5})  gap {}",
                s.dx,s.dy,s.cursor_x,s.cursor_y,
                s.interval_ms.map_or_else(||"-".into(),|v|format!("{v:.3}ms")))
        }).collect::<Vec<_>>();
        lines.reverse();
        if lines.is_empty(){"No mouse events yet. Move the mouse.".into()}
        else{lines.join("\r\n")}
    }
    pub fn append_report(&self,out:&mut String){
        let _=writeln!(out,"\n\nMOUSE RAW INPUT vs SAMPLED WINDOWS CURSOR");
        let _=writeln!(out,"Rate is an observed moving-event estimate, NOT a certified hardware polling rate.");
        let _=writeln!(out,"Moving too slowly or stationary produces fewer motion reports; acceleration/scaling changes pixel movement.");
        let _=writeln!(out,"{}",self.live_stats().replace("\r\n","\n"));
        let _=writeln!(out,"Rows: {} (dropped {})",self.samples.len(),self.dropped);
        let _=writeln!(out,"time_ms | device_handle | raw_dx | raw_dy | cursor_x | cursor_y | absolute | button_flags | movement_gap_ms");
        for s in &self.samples{
            let ms=s.time.saturating_sub(self.epoch) as f64*1000.0/self.frequency as f64;
            let _=writeln!(out,"{ms:.3} | 0x{:x} | {} | {} | {} | {} | {} | 0x{:x} | {}",
                s.device,s.dx,s.dy,s.cursor_x,s.cursor_y,s.absolute,s.buttons,
                s.interval_ms.map_or_else(||"-".into(),|v|format!("{v:.4}")));
        }
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn polling_estimate_ignores_button_only_events(){
        let mut d=MouseDiagnostics::new(1_000_000,0);
        for i in 0..101 {
            let t=i*1_000;
            d.observe(t,3,4,-1,false,0,i as i32,2);
            d.observe(t+500,3,0,0,false,1,0,0);
        }
        let (hz,median,p95)=d.frequency_stats().unwrap();
        assert_eq!(median,1.0);assert_eq!(p95,1.0);
        assert_eq!(hz,1000.0);assert_eq!(d.buttons,101);
    }
}