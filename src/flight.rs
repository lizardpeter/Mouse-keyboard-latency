//! Bounded flight recorder. Records only during explicitly selected diagnostic
//! modes. Never touches files or system configuration unless user exports.
use std::collections::VecDeque;
use std::fmt::Write;
use std::io::{self,Write as IoWrite};
use std::path::Path;

const PRE_MS: f64=10_000.0;
const POST_MS: f64=5_000.0;
const ROLL_CAP:usize=100_000;
const CAPTURE_CAP:usize=125_000;
const ANOMALY_CAP:usize=128;
pub const LANES:usize=6;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Lane {RawKey=0,WinKey=1,Char=2,Mouse=3,Watchdog=4,Marker=5}
impl Lane {
    pub fn label(self)->&'static str{
        match self{
            Self::RawKey=>"RAW KEY",Self::WinKey=>"WIN32 KEY",
            Self::Char=>"WM_CHAR",Self::Mouse=>"MOUSE",Self::Watchdog=>"WATCHDOG",
            Self::Marker=>"ANOMALY"
        }
    }
}
#[derive(Clone,Copy,Debug)]
pub struct FlightEvent {
    pub tick:i64, pub lane:Lane, pub tag:&'static str,
    pub vk:u16, pub device:usize, pub value:i32,
    pub queue_ms:u32,
}
impl FlightEvent {
    pub fn new(tick:i64,lane:Lane,tag:&'static str,vk:u16,device:usize,
               value:i32,queue_ms:u32)->Self{
        Self{tick,lane,tag,vk,device,value,queue_ms}
    }
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Reason{RepeatBurst,CharacterBurst,QueueAge,GuiGap,InputGap,Manual}
impl Reason{
    pub fn label(self)->&'static str {
        match self{
            Self::RepeatBurst=>"Raw repeat burst",
            Self::CharacterBurst=>"WM_CHAR burst",
            Self::QueueAge=>"Coarse message-queue age",
            Self::GuiGap=>"GUI heartbeat stall",
            Self::InputGap=>"Raw Input receiver heartbeat stall",
            Self::Manual=>"Manual capture"
        }
    }
}
#[derive(Clone,Copy,Debug)]
pub struct Marker{pub tick:i64,pub reason:Reason,pub detail_ms:f64}

pub struct FlightRecorder {
    frequency:i64,
    epoch:i64,
    rolling:VecDeque<FlightEvent>,
    frozen:Vec<FlightEvent>,
    pub markers:Vec<Marker>,
    pub dropped:u64,
    pub recording:bool,
    pub completed:bool,
    deadline:i64,
    last_fast_repeat:Option<(u16,usize,i64,u8)>,
    last_fast_char:Option<(i64,u8)>,
    last_signal_tick:Option<i64>,
    pub width_ms:f64,
    pub pan_ms:f64,
    pub anomalies:u64,
    pub watchdog_gaps:u64,
}
impl FlightRecorder {
    pub fn new(frequency:i64,epoch:i64)->Self{
        assert!(frequency>0);
        Self{frequency,epoch,rolling:VecDeque::with_capacity(ROLL_CAP),
            frozen:Vec::with_capacity(CAPTURE_CAP),markers:Vec::new(),
            dropped:0,recording:false,completed:false,deadline:0,
            last_fast_repeat:None,last_fast_char:None,last_signal_tick:None,
            width_ms:15_000.0,pan_ms:0.0,anomalies:0,watchdog_gaps:0}
    }
    fn ticks(&self,ms:f64)->i64{(ms*self.frequency as f64/1000.0) as i64}
    fn ms(&self,t:i64)->f64{t.saturating_sub(self.epoch) as f64*1000.0/self.frequency as f64}
    pub fn clear_temporal_state(&mut self){
        self.last_fast_char=None;self.last_fast_repeat=None;self.last_signal_tick=None;
    }
    fn signal(&mut self,tick:i64,reason:Reason,detail_ms:f64){
        // Rate-limit coarse queue alerts so 50 samples aren't all "anomalies."
        let debounce=self.ticks(250.0);
        if self.last_signal_tick.is_some_and(|t|tick.saturating_sub(t)<debounce)
            && reason!=Reason::Manual{return;}
        self.last_signal_tick=Some(tick);
        self.anomalies+=1;
        if self.markers.len()<ANOMALY_CAP{
            self.markers.push(Marker{tick,reason,detail_ms});
        }
        if !self.recording && !self.completed{
            self.start(tick);
        }
    }
    fn start(&mut self,tick:i64){
        let min=tick.saturating_sub(self.ticks(PRE_MS));
        self.frozen.clear();
        self.frozen.extend(self.rolling.iter().filter(|e|e.tick>=min)
            .take(CAPTURE_CAP).copied());
        self.recording=true;
        self.deadline=tick.saturating_add(self.ticks(POST_MS));
        self.width_ms=PRE_MS+POST_MS;self.pan_ms=0.0;
    }
    pub fn manual_capture(&mut self,tick:i64){
        if !self.recording && !self.completed{
            self.signal(tick,Reason::Manual,0.0);
        }
    }
    pub fn finish(&mut self,now:i64){
        if self.recording && now>=self.deadline{
            self.recording=false;self.completed=true;
        }
    }
    /// When switching from a capture mode to the viewer before post-roll ends,
    /// finish early rather than presenting a perpetually "recording" snapshot.
    pub fn finish_early(&mut self){
        if self.recording{self.recording=false;self.completed=true;}
        self.clear_temporal_state();
    }
    pub fn observe(&mut self,event:FlightEvent){
        self.finish(event.tick);
        if self.rolling.len()==ROLL_CAP{self.rolling.pop_front();self.dropped+=1;}
        self.rolling.push_back(event);
        let min=event.tick.saturating_sub(self.ticks(PRE_MS));
        while self.rolling.front().is_some_and(|e|e.tick<min){
            self.rolling.pop_front();
        }
        if self.recording && event.tick<=self.deadline {
            if self.frozen.len()<CAPTURE_CAP{self.frozen.push(event);}
            else{self.dropped+=1;}
        }
        match event.lane{
            Lane::RawKey=>{
                if event.tag=="REPEAT"{
                    let (count,dt)=self.last_fast_repeat
                        .filter(|(vk,dev,last,_)|*vk==event.vk
                            && *dev==event.device && event.tick>=*last)
                        .map(|(_,_,last,n)| {
                            let gap=(event.tick-last) as f64*1000.0/self.frequency as f64;
                            (if gap<5.0{n.saturating_add(1)}else{0},gap)
                        }).unwrap_or((0,0.0));
                    self.last_fast_repeat=Some((event.vk,event.device,event.tick,count));
                    if count==3 {self.signal(event.tick,Reason::RepeatBurst,dt);}
                }else{self.last_fast_repeat=None;}
            },
            Lane::Char=>{
                let fast=self.last_fast_char
                    .filter(|(t,_)|event.tick>=*t)
                    .map(|(t,n)| {
                        let gap=(event.tick-t) as f64*1000.0/self.frequency as f64;
                        if gap<5.0 {n.saturating_add(1)}else{0}
                    }).unwrap_or(0);
                self.last_fast_char=Some((event.tick,fast));
                if fast==2{self.signal(event.tick,Reason::CharacterBurst,0.0);}
            },
            Lane::Watchdog=>{
                if event.value>=200{
                    self.watchdog_gaps+=1;
                    let reason=if event.tag=="CAPTURE_STALL"{Reason::InputGap}else{Reason::GuiGap};
                    self.signal(event.tick,reason,event.value as f64);
                }
            },
            _=>{},
        }
        if event.queue_ms>=48 && event.lane==Lane::RawKey {
            self.signal(event.tick,Reason::QueueAge,event.queue_ms as f64);
        }
    }
    pub fn view_bounds_ms(&self,now:i64)->(f64,f64){
        let end=self.current_end(now);
        let start=end.saturating_sub(self.ticks(self.width_ms));
        (self.ms(start),self.ms(end))
    }
    pub fn current_end(&self,now:i64)->i64{
        let latest=if self.recording||self.completed{
            self.frozen.last().map(|e|e.tick).unwrap_or(now)
        }else{self.rolling.back().map(|e|e.tick).unwrap_or(now)};
        latest.saturating_sub(self.ticks(self.pan_ms))
    }
    pub fn zoom(&mut self,zoom_in:bool){
        self.width_ms=if zoom_in{(self.width_ms/2.0).max(125.0)}
            else{(self.width_ms*2.0).min(30_000.0)};
    }
    pub fn pan(&mut self,back:bool){
        let shift=self.width_ms/2.0;
        self.pan_ms=(self.pan_ms+if back{shift}else{-shift}).clamp(0.0,120_000.0);
    }
    pub fn histogram(&self,end:i64,columns:usize)->Vec<[u8;LANES]>{
        let mut histogram=vec![[0u8;LANES];columns];
        if columns==0{return histogram;}
        let start=end.saturating_sub(self.ticks(self.width_ms));
        let fill=|e:&FlightEvent,h:&mut Vec<[u8;LANES]>|{
            if e.tick<start||e.tick>end{return;}
            let frac=(e.tick-start) as f64/(end-start).max(1) as f64;
            let index=((frac*columns as f64).floor() as usize).min(columns-1);
            let lane=e.lane as usize;
            h[index][lane]=h[index][lane].saturating_add(1);
        };
        if self.recording||self.completed{
            for e in &self.frozen{fill(e,&mut histogram);}
        }else{
            for e in &self.rolling{fill(e,&mut histogram);}
        }
        for m in &self.markers{
            if m.tick>=start&&m.tick<=end{
                fill(&FlightEvent::new(m.tick,Lane::Marker,"MARKER",0,0,0,0),&mut histogram);
            }
        }
        histogram
    }
    pub fn summary(&self)->String{
        let state=if self.recording{"Collecting 5s post-trigger"}
            else if self.completed{"FROZEN — ready to export"}
            else{"Armed — 10s rolling memory only"};
        format!("{state} | anomalies {} | heartbeat stalls {} | samples {} | dropped {}",
            self.anomalies,self.watchdog_gaps,
            if self.recording||self.completed{self.frozen.len()}else{self.rolling.len()},
            self.dropped)
    }
    pub fn marker_list(&self)->String{
        if self.markers.is_empty(){return "No anomaly marker yet. Use Manual Capture to preserve a segment.".into();}
        self.markers.iter().rev().take(12)
            .map(|m|format!("{:>9.2} ms — {} (value {:.1} ms)",
                self.ms(m.tick),m.reason.label(),m.detail_ms))
            .collect::<Vec<_>>().join("\r\n")
    }
    pub fn append_report(&self,out:&mut String) {
        let _=writeln!(out,"\nINPUT TIMING LAB — IN-MEMORY FLIGHT RECORDER");
        let _=writeln!(out,"Keyboard key names/codes can reveal test inputs. Review before sharing.");
        let _=writeln!(out,"Capture uses user-mode process observations, not physical switch/USB latency.");
        let _=writeln!(out,"10-second preroll; 5-second postroll if diagnostics stays active.");
        let _=writeln!(out,"{}",self.summary());
        let _=writeln!(out,"Markers:");
        for m in &self.markers {
            let _=writeln!(out,"{:.3} ms | {} | {:.3}",self.ms(m.tick),m.reason.label(),m.detail_ms);
        }
        let _=writeln!(out,"time_ms | channel | action | vk | device_handle | value | approximate_queue_age_ms");
        if self.recording||self.completed{
            for e in &self.frozen{self.write_event(out,e);}
        }else{
            for e in &self.rolling{self.write_event(out,e);}
        }
    }
    fn write_event(&self,out:&mut String,e:&FlightEvent){
        let _=writeln!(out,"{:.3} | {} | {} | {} | 0x{:x} | {} | {}",
            self.ms(e.tick),e.lane.label(),e.tag,e.vk,e.device,e.value,e.queue_ms);
    }
    pub fn save_csv(&self,path:&Path)->io::Result<()>{
        let mut out=io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(out,"time_ms,channel,action,vk,device_handle,value,approx_queue_age_ms")?;
        let dump=|e:&FlightEvent,out:&mut io::BufWriter<std::fs::File>|{
            writeln!(out,"{:.3},{},{},{},0x{:x},{},{}",
                self.ms(e.tick),e.lane.label(),e.tag,e.vk,e.device,e.value,e.queue_ms)
        };
        if self.recording||self.completed{
            for e in &self.frozen{dump(e,&mut out)?;}
        }else{for e in &self.rolling{dump(e,&mut out)?;}}
        out.flush()
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    fn raw(t:i64,kind:&'static str)->FlightEvent{
        FlightEvent::new(t,Lane::RawKey,kind,68,3,0,0)
    }
    #[test]fn burst_captures_pre_and_post_and_freezes(){
        let mut r=FlightRecorder::new(1_000_000,0);
        r.observe(raw(0,"DOWN"));
        for t in [1_000_000,2_000_000,2_001_000,2_002_000,2_003_000]{
            r.observe(raw(t,"REPEAT"));
        }
        assert!(r.recording);assert_eq!(r.markers.len(),1);
        assert_eq!(r.markers[0].reason,Reason::RepeatBurst);
        r.observe(raw(5_000_000,"UP"));
        r.finish(8_000_000);
        assert!(r.completed);assert!(!r.recording);
        assert_eq!(r.frozen.len(),7);
        let mut out=String::new();r.append_report(&mut out);
        assert!(out.contains("Raw repeat burst"));
    }
    #[test]fn normal_repeats_do_not_trigger(){
        let mut r=FlightRecorder::new(1_000_000,0);
        r.observe(raw(0,"DOWN"));
        for i in 1..100{r.observe(raw(i*33_000,"REPEAT"));}
        assert!(r.markers.is_empty());
    }
    #[test]fn char_burst_and_manual_marker(){
        let mut r=FlightRecorder::new(1_000_000,0);
        for t in [0,1_000,2_000]{
            r.observe(FlightEvent::new(t,Lane::Char,"WM_CHAR",0,0,0,0));
        }
        assert_eq!(r.markers[0].reason,Reason::CharacterBurst);
        r.finish_early();assert!(r.completed);
        let h=r.histogram(10_000,100);
        assert!(h.iter().any(|b|b[Lane::Char as usize]>0));
    }
    #[test]fn no_markers_without_anomaly_and_queue_age_debounced(){
        let mut r=FlightRecorder::new(1_000_000,0);
        for i in 0..10 {
            r.observe(FlightEvent::new(i*10_000,Lane::RawKey,"DOWN",10,4,0,64));
        }
        assert_eq!(r.markers.len(),1);
    }
}
