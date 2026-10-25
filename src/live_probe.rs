//! Event-stream cross-checks aimed at a problem recurring *after* Windows boot.
//! This is a pure analysis layer: it cannot know physical key release or certify
//! that an idle mouse/keyboard has stopped sending input.
use std::collections::{BTreeMap,VecDeque};
use std::fmt::Write;

const MAX_NOTES:usize=128;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum IncidentKind{SlowRepeat,FastRepeat,MouseGap,ClickWhileHeld}
impl IncidentKind {
    fn label(self)->&'static str{
        match self {
            Self::SlowRepeat=>"Held-key repeats delivered slowly",
            Self::FastRepeat=>"Held-key repeats delivered as a burst",
            Self::MouseGap=>"Gap between mouse motion reports",
            Self::ClickWhileHeld=>"Mouse click arrived while Raw Input still showed key held",
        }
    }
}
#[derive(Clone,Copy,Debug)]
pub struct Incident{
    pub tick:i64,pub kind:IncidentKind,pub device:usize,pub vk:u16,
    pub gap_ms:f64,pub queue_age_ms:u32,pub gui_handoff_ms:f64,
    pub group:u16,pub cpu:u8,
}
struct Held{vk:u16,repeat_at:Option<i64>,fast:u8}
pub struct LiveProbe{
    freq:i64,epoch:i64,
    held:BTreeMap<(usize,u16,u8),Held>,
    mouse_motion:BTreeMap<usize,i64>,
    notes:VecDeque<Incident>,
    pub slow_repeats:u64,
    pub fast_repeat_bursts:u64,
    pub mouse_gaps:u64,
    pub click_while_held:u64,
    pub worst_repeat_ms:f64,
    pub worst_mouse_motion_gap_ms:f64,
}
impl LiveProbe{
    pub fn new(freq:i64,epoch:i64)->Self{
        assert!(freq>0);
        Self{freq,epoch,held:BTreeMap::new(),mouse_motion:BTreeMap::new(),
            notes:VecDeque::new(),slow_repeats:0,fast_repeat_bursts:0,
            mouse_gaps:0,click_while_held:0,worst_repeat_ms:0.0,
            worst_mouse_motion_gap_ms:0.0}
    }
    pub fn suspend(&mut self){self.held.clear();self.mouse_motion.clear();}
    fn gap(&self,a:i64,b:i64)->f64{b.saturating_sub(a) as f64*1000.0/self.freq as f64}
    fn note(&mut self,e:Incident){
        if self.notes.len()==MAX_NOTES{self.notes.pop_front();}
        self.notes.push_back(e);
    }
    /// Each Raw Keyboard make/break remains separate per device and scan/E0.
    /// We ignore the first repeat's Windows typematic delay (~500 ms).
    pub fn key(&mut self,tick:i64,device:usize,vk:u16,scan:u16,flags:u16,
               queue_age_ms:u32,handoff_ms:f64,group:u16,cpu:u8)->Option<IncidentKind>{
        let id=(device,if scan==0{vk}else{scan},(flags&0x06) as u8);
        if flags&1!=0{self.held.remove(&id);return None;}
        let (previous,fast)=if let Some(held)=self.held.get_mut(&id){
            let p=held.repeat_at.replace(tick);
            let fast=held.fast;
            (p,fast)
        }else{
            self.held.insert(id,Held{vk,repeat_at:None,fast:0});
            return None;
        };
        let Some(prev)=previous else{return None};
        if tick<prev{return None;}
        let gap=self.gap(prev,tick);
        let mut kind=None;
        if (150.0..=2000.0).contains(&gap){
            self.slow_repeats+=1;self.worst_repeat_ms=self.worst_repeat_ms.max(gap);
            kind=Some(IncidentKind::SlowRepeat);
        }else if gap<5.0 {
            let count=fast.saturating_add(1);
            if let Some(h)=self.held.get_mut(&id){h.fast=count;}
            if count==3{
                self.fast_repeat_bursts+=1;kind=Some(IncidentKind::FastRepeat);
            }
        }else if let Some(h)=self.held.get_mut(&id){h.fast=0;}
        if let Some(kind)=kind{
            self.note(Incident{tick,kind,device,vk,gap_ms:gap,
                queue_age_ms,gui_handoff_ms:handoff_ms,group,cpu});
        }
        kind
    }
    /// Only report gaps between *actual movement events*. An idle mouse cannot
    /// establish whether USB servicing stalled.
    pub fn mouse(&mut self,tick:i64,device:usize,dx:i32,dy:i32,
                 left_down:bool,queue_age_ms:u32,handoff_ms:f64,
                 group:u16,cpu:u8)->Option<IncidentKind>{
        if left_down && !self.held.is_empty(){
            self.click_while_held+=1;
            let vk=self.held.values().next().map_or(0,|h|h.vk);
            self.note(Incident{tick,kind:IncidentKind::ClickWhileHeld,
                device,vk,gap_ms:0.0,queue_age_ms,gui_handoff_ms:handoff_ms,group,cpu});
            return Some(IncidentKind::ClickWhileHeld);
        }
        if dx==0&&dy==0{return None;}
        let prior=self.mouse_motion.insert(device,tick);
        if let Some(last)=prior{
            let gap=self.gap(last,tick);
            // 100ms could simply be the user's hand pausing; signal only in
            // the independent report, never claim it is a USB/driver stall.
            if gap>=100.0 && gap<=5000.0{
                self.mouse_gaps+=1;
                self.worst_mouse_motion_gap_ms=self.worst_mouse_motion_gap_ms.max(gap);
                self.note(Incident{tick,kind:IncidentKind::MouseGap,
                    device,vk:0,gap_ms:gap,queue_age_ms,
                    gui_handoff_ms:handoff_ms,group,cpu});
                return Some(IncidentKind::MouseGap);
            }
        }
        None
    }
    pub fn short_status(&self)->String {
        format!("Slow held-key repeats: {} (worst {:.1} ms) | fast-repeat bursts: {} | mouse motion gaps: {} (may be idle) | click markers while key held: {}",
            self.slow_repeats,self.worst_repeat_ms,self.fast_repeat_bursts,
            self.mouse_gaps,self.click_while_held)
    }
    pub fn append_report(&self,out:&mut String){
        let _=writeln!(out,"\n\nLIVE INCIDENT CROSS-CHECK (RAW INPUT, HOST-TIMESTAMPED)");
        let _=writeln!(out,"First typematic delay is ignored. Slow repeat means consecutive observed repeats for an already-held key were >=150ms apart.");
        let _=writeln!(out,"Mouse motion gaps may simply be a pause in physical movement: NOT hardware stall evidence.");
        let _=writeln!(out,"Click while key held is an action marker, NOT proof the key was physically released before the click.");
        let _=writeln!(out,"{}",self.short_status());
        let _=writeln!(out,"time_ms | interpretation | device_handle | virtual_key | gap_ms | capture_queue_age_ms | GUI_handoff_ms | CPU_group | CPU_number");
        for n in &self.notes{
            let ms=n.tick.saturating_sub(self.epoch) as f64*1000.0/self.freq as f64;
            let _=writeln!(out,"{ms:.3} | {} | 0x{:x} | {} | {:.3} | {} | {:.3} | {} | {}",
                n.kind.label(),n.device,n.vk,n.gap_ms,n.queue_age_ms,
                n.gui_handoff_ms,n.group,n.cpu);
        }
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn first_repeat_delay_is_ignored_but_slow_subsequent_repeats_caught(){
        let mut p=LiveProbe::new(1_000_000,0);
        assert!(p.key(0,1,68,32,0,0,0.0,0,0).is_none());
        assert!(p.key(500_000,1,68,32,0,0,0.0,0,0).is_none());
        assert_eq!(p.key(810_000,1,68,32,0,312,2.0,0,1),Some(IncidentKind::SlowRepeat));
        assert_eq!(p.slow_repeats,1);
        p.key(811_000,1,68,32,1,0,0.0,0,0);
        p.key(1_100_000,1,68,32,0,0,0.0,0,0);
        assert_eq!(p.slow_repeats,1);
    }
    #[test]fn another_device_does_not_look_like_repeats(){
        let mut p=LiveProbe::new(1_000_000,0);
        p.key(0,1,68,32,0,0,0.0,0,0);
        p.key(400_000,2,68,32,0,0,0.0,0,0);
        p.key(600_000,2,68,32,0,0,0.0,0,0);
        assert_eq!(p.slow_repeats,0);
    }
    #[test]fn release_then_click_does_not_mark_still_held(){
        let mut p=LiveProbe::new(1_000_000,0);
        p.key(1,1,68,32,0,0,0.0,0,0);
        assert_eq!(p.mouse(2,9,0,0,true,0,0.0,0,0),Some(IncidentKind::ClickWhileHeld));
        p.key(3,1,68,32,1,0,0.0,0,0);
        assert!(p.mouse(4,9,0,0,true,0,0.0,0,0).is_none());
    }
}