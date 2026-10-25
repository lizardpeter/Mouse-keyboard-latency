//! Per-device, strictly passive observation of host-delivered Raw Input messages.
//! Neither Raw Input nor Windows HID collection metadata measures all USB polls.
//! Keyboard key autorepeats are NOT hardware polling observations.
use std::collections::{BTreeMap,VecDeque};
use std::fmt::Write;

const MAX_DEVICES:usize=16;
const GAP_CAP:usize=8192;
const WINDOW_MS:f64=1000.0;

#[derive(Default)]
pub struct PollingDevice {
    pub packets:u64,
    pub movement:u64,
    pub buttons:u64,
    pub transitions:u64,
    pub key_repeats:u64,
    pub last_tick:Option<i64>,
    first_tick:Option<i64>,
    pub hid_polled_interval_ms:Option<u32>,
    pub hid_query_done:bool,
    last_motion_tick:Option<i64>,
    packet_gaps:VecDeque<f64>,
    motion_gaps:VecDeque<f64>,
    // Windows-delivered reports in the latest ~1s, not the USB poll schedule.
    recent_packets:VecDeque<i64>,
    recent_motion:VecDeque<i64>,
    // Per-device key state to distinguish physical down/up transitions from
    // software key repetition. Virtual keys may originate in different layouts.
    key_down:BTreeMap<(u16,u16),bool>,
}
#[derive(Clone,Copy)]
pub struct Rates {
    pub median_gap_ms:f64,
    pub p95_gap_ms:f64,
    pub p99_gap_ms:f64,
    pub min_gap_ms:f64,
    pub interval_estimate_hz:f64,
    pub one_second_rate_hz:Option<f64>,
    pub sample_count:usize,
}
fn percentiles(gaps:&VecDeque<f64>,one_second:Option<f64>)->Option<Rates>{
    if gaps.len()<5{return None}
    let mut g:Vec<f64>=gaps.iter().copied().filter(|x|x.is_finite()&&*x>0.0).collect();
    if g.len()<5{return None}
    g.sort_by(f64::total_cmp);
    let n=g.len();
    let at=|p:f64|g[((n-1) as f64*p).round() as usize];
    let median=at(0.5);
    Some(Rates{median_gap_ms:median,p95_gap_ms:at(0.95),p99_gap_ms:at(0.99),
        min_gap_ms:g[0],interval_estimate_hz:1000.0/median,
        one_second_rate_hz:one_second,sample_count:n})
}
fn save_gap(dest:&mut VecDeque<f64>,ms:f64){
    if !ms.is_finite()||ms<=0.0{return;}
    if dest.len()>=GAP_CAP{dest.pop_front();}
    dest.push_back(ms);
}
fn update_window(queue:&mut VecDeque<i64>,tick:i64,freq:i64){
    queue.push_back(tick);
    let oldest=tick.saturating_sub(freq);
    while queue.front().is_some_and(|t|*t<=oldest){queue.pop_front();}
}
impl PollingDevice {
    fn observe_packet(&mut self,tick:i64,freq:i64){
        if self.first_tick.is_none(){self.first_tick=Some(tick);}
        if let Some(last)=self.last_tick{
            if tick>=last {
                save_gap(&mut self.packet_gaps,(tick-last) as f64*1000.0/freq as f64);
            } else {
                // Discontinuous QPC time; do not create nonsense rates.
                self.packet_gaps.clear(); self.recent_packets.clear();
            }
        }
        self.last_tick=Some(tick);
        self.packets+=1;
        update_window(&mut self.recent_packets,tick,freq);
    }
    pub fn observe_mouse(&mut self,tick:i64,freq:i64,dx:i32,dy:i32,button_flags:u16){
        self.observe_packet(tick,freq);
        if button_flags!=0{self.buttons+=1;}
        if dx!=0||dy!=0 {
            self.movement+=1;
            if let Some(last)=self.last_motion_tick{
                if tick>=last {
                    save_gap(&mut self.motion_gaps,(tick-last) as f64*1000.0/freq as f64);
                }else{self.motion_gaps.clear();self.recent_motion.clear();}
            }
            self.last_motion_tick=Some(tick);
            update_window(&mut self.recent_motion,tick,freq);
        }
    }
    pub fn observe_key(&mut self,tick:i64,freq:i64,vk:u16,scan:u16,down:bool){
        self.observe_packet(tick,freq);
        let prev=self.key_down.get(&(vk,scan)).copied().unwrap_or(false);
        if down && prev{self.key_repeats+=1;}
        else {
            self.transitions+=1;
            self.key_down.insert((vk,scan),down);
        }
    }
    fn rate_of(&self,queue:&VecDeque<i64>,now:i64,freq:i64)->Option<f64>{
        let first=self.first_tick?;
        if now.saturating_sub(first)<freq{return None}
        let cutoff=now.saturating_sub(freq);
        let count=queue.iter().filter(|&&t|t>cutoff && t<=now).count();
        Some(count as f64*WINDOW_MS/1000.0)
    }
    pub fn packet_rates(&self,now:i64,freq:i64)->Option<Rates>{
        percentiles(&self.packet_gaps,self.rate_of(&self.recent_packets,now,freq))
    }
    pub fn movement_rates(&self,now:i64,freq:i64)->Option<Rates>{
        percentiles(&self.motion_gaps,self.rate_of(&self.recent_motion,now,freq))
    }
    pub fn last_seen_ms(&self,now:i64,freq:i64)->Option<f64>{
        self.last_tick.map(|t|now.saturating_sub(t) as f64*1000.0/freq as f64)
    }
}
pub struct PollingMonitor {
    pub frequency:i64,
    pub mice:BTreeMap<usize,PollingDevice>,
    pub keyboards:BTreeMap<usize,PollingDevice>,
    pub missing_device_capacity:u64,
}
impl PollingMonitor {
    pub fn new(frequency:i64)->Self{
        assert!(frequency>0);
        Self{frequency,mice:BTreeMap::new(),keyboards:BTreeMap::new(),
            missing_device_capacity:0}
    }
    fn find_or_add<'a>(map:&'a mut BTreeMap<usize,PollingDevice>,
                        handle:usize)->Option<&'a mut PollingDevice>{
        if !map.contains_key(&handle)&&map.len()>=MAX_DEVICES{return None;}
        Some(map.entry(handle).or_default())
    }
    pub fn mouse(&mut self,handle:usize,tick:i64,dx:i32,dy:i32,buttons:u16){
        if let Some(s)=Self::find_or_add(&mut self.mice,handle){
            s.observe_mouse(tick,self.frequency,dx,dy,buttons)
        }else{self.missing_device_capacity+=1}
    }
    pub fn keyboard(&mut self,handle:usize,tick:i64,vk:u16,scan:u16,down:bool){
        if let Some(s)=Self::find_or_add(&mut self.keyboards,handle){
            s.observe_key(tick,self.frequency,vk,scan,down);
        }else{self.missing_device_capacity+=1}
    }
    pub fn device_needing_hid_query(&self)->Option<(bool,usize)> {
        self.mice.iter().find(|(_,d)|!d.hid_query_done).map(|(&id,_)|(true,id))
            .or_else(||self.keyboards.iter().find(|(_,d)|!d.hid_query_done)
                .map(|(&id,_)|(false,id)))
    }
    pub fn set_hid_query(&mut self,mouse:bool,handle:usize,interval:Option<u32>){
        let devices=if mouse{&mut self.mice}else{&mut self.keyboards};
        if let Some(dev)=devices.get_mut(&handle){
            dev.hid_polled_interval_ms=interval.filter(|&i|i>0&&i<=1000);
            dev.hid_query_done=true;
        }
    }
    pub fn gui_mouse(&self,now:i64)->String {
        if self.mice.is_empty(){return "No Raw Input mouse devices observed yet.".into();}
        let mut out=String::new();
        for (&id,d) in &self.mice{
            let r=d.movement_rates(now,self.frequency);
            let packet=d.packet_rates(now,self.frequency);
            let active=d.last_seen_ms(now,self.frequency).is_some_and(|x|x<500.0);
            let _=writeln!(out,"Mouse device 0x{id:x}   {}",if active{"ACTIVE"}else{"IDLE"});
            let _=writeln!(out,"  Delivered motion: {} | buttons: {} | all Raw reports: {}",
                d.movement,d.buttons,d.packets);
            if let Some(r)=r {
                let _=writeln!(out,"  Motion inter-arrival: ~{:.0} Hz median-derived | median {:.3} ms | P95 {:.3} ms | P99 {:.3} ms",
                    r.interval_estimate_hz,r.median_gap_ms,r.p95_gap_ms,r.p99_gap_ms);
                if let Some(hz)=r.one_second_rate_hz{
                    let _=writeln!(out,"  Last 1-second moving events: {:.0}/s",hz);
                }
            }else{let _=writeln!(out,"  Need 6+ moving reports for interval statistics.");}
            if let Some(r)=packet {
                let _=writeln!(out,"  All Raw report gaps: median {:.3} ms | p99 {:.3} ms | {} gaps",
                    r.median_gap_ms,r.p99_gap_ms,r.sample_count);
            }
            let _=writeln!(out,"  HID class poll query: {}",
                d.hid_polled_interval_ms.map_or("unavailable".into(),|ms|
                    format!("{ms} ms ({} Hz, NOT USB endpoint bInterval)",1000/ms)));
        }
        out.push_str("Rates reflect host-delivered Raw Input. A resting mouse sends fewer motion reports.\n");
        out
    }
    pub fn gui_keyboard(&self,now:i64)->String {
        if self.keyboards.is_empty(){return "No Raw Input keyboard devices observed yet.".into();}
        let mut out=String::new();
        for (&id,d) in &self.keyboards{
            let _=writeln!(out,"Keyboard 0x{id:x}: {} key transitions, {} Windows repeats; {} total Raw messages",
                d.transitions,d.key_repeats,d.packets);
            if let Some(rate)=d.packet_rates(now,self.frequency){
                let _=writeln!(out,"  Message gaps: median {:.2} ms | P95 {:.2} ms | P99 {:.2} ms | min {:.3} ms",
                    rate.median_gap_ms,rate.p95_gap_ms,rate.p99_gap_ms,rate.min_gap_ms);
            }
            let _=writeln!(out,"  HID class poll query: {}",
                d.hid_polled_interval_ms.map_or("unavailable".into(),|ms|
                    format!("{ms} ms (not USB bInterval)")));
        }
        out.push_str("Key repeats are Windows typematic events, NOT keyboard USB polls.\n");
        out
    }
    pub fn append_report(&self,out:&mut String,now:i64){
        let _=writeln!(out,"\n\nPER-DEVICE POLLING ANALYSIS (PASSIVE)");
        let _=writeln!(out,"Raw Input captures host-delivered events, not every USB poll/idle report.");
        let _=writeln!(out,"HID_GET_POLL_FREQUENCY_MSEC may be unsupported and is NOT the USB interrupt endpoint bInterval.");
        let _=writeln!(out,"Below rates are independent per hDevice; no merging of unrelated mice or keyboards.");
        let _=writeln!(out,"Mouse devices:\n{}",self.gui_mouse(now));
        let _=writeln!(out,"Keyboard devices:\n{}",self.gui_keyboard(now));
        let _=writeln!(out,"Capacity-rejected input events: {}",self.missing_device_capacity);
    }
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn per_device_mouse_rates_do_not_mix(){
        let mut m=PollingMonitor::new(1_000_000);
        for t in 0..=2000_i64 {
            m.mouse(1,t*1000,1,1,0);
            if t%2==0{m.mouse(2,t*1000,2,2,0);}
        }
        let a=m.mice[&1].movement_rates(2_000_000,1_000_000).unwrap();
        let b=m.mice[&2].movement_rates(2_000_000,1_000_000).unwrap();
        assert_eq!(a.interval_estimate_hz,1000.0);
        assert_eq!(b.interval_estimate_hz,500.0);
        assert_eq!(a.one_second_rate_hz,Some(1000.0));
        assert_eq!(b.one_second_rate_hz,Some(500.0));
    }
    #[test]fn button_only_reports_dont_contribute_motion_rate(){
        let mut d=PollingDevice::default();
        for t in 0..=100_i64 {
            d.observe_mouse(t*1000,1_000_000,0,0,1);
        }
        assert_eq!(d.packets,101);assert_eq!(d.movement,0);
        assert!(d.movement_rates(101_000,1_000_000).is_none());
    }
    #[test]fn keyboard_repeats_not_usable_as_hardware_rate(){
        let mut d=PollingDevice::default();
        d.observe_key(0,1_000_000,68,32,true);
        for t in 1..=31{d.observe_key(t*32_000,1_000_000,68,32,true);}
        d.observe_key(1_000_000,1_000_000,68,32,false);
        assert_eq!(d.transitions,2);
        assert_eq!(d.key_repeats,31);
        assert!(!PollingMonitor::new(1_000_000).gui_keyboard(0).contains("8000"));
    }
}