//! Platform-independent bounded diagnostics of the independent Raw Input and GUI
//! scheduling paths. First-minute buckets do not require ETW or system changes.
use std::collections::VecDeque;
use std::fmt::Write;
#[derive(Copy,Clone,Default)]
pub struct SecondBin{
    pub keyboard:u32,pub mouse:u32,
    pub max_capture_queue_age_ms:u32,
    pub max_gui_handoff_ms:f64,
    pub max_capture_handler_us:f64,
    pub max_gui_handler_ms:f64,
    pub gui_stalls:u32,pub capture_stalls:u32,
}
#[derive(Copy,Clone,Default)]
pub struct TimingSample{
    pub tick:i64,pub device:usize,pub keyboard:bool,
    pub captured_age_ms:u32,pub handoff_ms:f64,
    pub callback_us:f64,
    pub group:u16,pub processor:u8,
}
pub struct ThreadMetrics{
    freq:i64,epoch:i64,
    pub first_minute:[SecondBin;60],
    pub first_minute_samples:u32,
    pub gui_stalls:u64,pub capture_stalls:u64,
    pub last_gui_gap_ms:f64,pub last_capture_gap_ms:f64,
    pub max_gui_gap_ms:f64,pub max_capture_gap_ms:f64,
    pub max_gui_handoff_ms:f64,pub max_capture_handler_us:f64,
    pub max_gui_handler_ms:f64,
    pub latest_capture_cpu:Option<(u16,u8)>,
    pub latest_gui_cpu:Option<(u16,u8)>,
    pub raw_received:u64,pub raw_keyboard:u64,pub raw_mouse:u64,
    pub newest_raw_tick:Option<i64>,
    recent:VecDeque<TimingSample>,last_recent_tick:i64,
}
impl ThreadMetrics{
    pub fn new(freq:i64,epoch:i64)->Self{
        assert!(freq>0);
        Self{freq,epoch,first_minute:[SecondBin::default();60],
            first_minute_samples:0,
            gui_stalls:0,capture_stalls:0,
            last_gui_gap_ms:0.0,last_capture_gap_ms:0.0,
            max_gui_gap_ms:0.0,max_capture_gap_ms:0.0,
            max_gui_handoff_ms:0.0,max_capture_handler_us:0.0,
            max_gui_handler_ms:0.0,
            latest_capture_cpu:None,latest_gui_cpu:None,
            raw_received:0,raw_keyboard:0,raw_mouse:0,
            newest_raw_tick:None,recent:VecDeque::with_capacity(2048),
            last_recent_tick:0}
    }
    fn second(&self,tick:i64)->Option<usize>{
        let elapsed=tick.saturating_sub(self.epoch);
        if elapsed<0{return None}
        let sec=(elapsed as u64)/(self.freq as u64);
        (sec<60).then_some(sec as usize)
    }
    pub fn raw(&mut self,s:TimingSample){
        self.raw_received+=1;
        if s.keyboard{self.raw_keyboard+=1;}else{self.raw_mouse+=1;}
        self.newest_raw_tick=Some(s.tick);
        self.latest_capture_cpu=Some((s.group,s.processor));
        self.max_gui_handoff_ms=self.max_gui_handoff_ms.max(s.handoff_ms);
        self.max_capture_handler_us=self.max_capture_handler_us.max(s.callback_us);
        if let Some(i)=self.second(s.tick){
            let b=&mut self.first_minute[i];
            if s.keyboard{b.keyboard+=1;}else{b.mouse+=1;}
            b.max_capture_queue_age_ms=b.max_capture_queue_age_ms.max(s.captured_age_ms);
            b.max_gui_handoff_ms=b.max_gui_handoff_ms.max(s.handoff_ms);
            b.max_capture_handler_us=b.max_capture_handler_us.max(s.callback_us);
            self.first_minute_samples+=1;
        }
        // Cap observation rate and memory independently of 8-kHz mouse packets.
        if self.last_recent_tick==0 ||
            s.tick.saturating_sub(self.last_recent_tick)>=self.freq/20 ||
            s.handoff_ms>=32.0
        {
            self.last_recent_tick=s.tick;
            if self.recent.len()>=2048{self.recent.pop_front();}
            self.recent.push_back(s);
        }
        let oldest=s.tick.saturating_sub(60*self.freq);
        while self.recent.front().is_some_and(|x|x.tick<oldest){
            self.recent.pop_front();
        }
    }
    pub fn gui_pulse(&mut self,tick:i64,handler_max_ms:f64,group:u16,cpu:u8){
        self.latest_gui_cpu=Some((group,cpu));
        self.max_gui_handler_ms=self.max_gui_handler_ms.max(handler_max_ms);
        if let Some(i)=self.second(tick){
            let b=&mut self.first_minute[i];
            b.max_gui_handler_ms=b.max_gui_handler_ms.max(handler_max_ms);
        }
    }
    pub fn gap(&mut self,capture:bool,tick:i64,ms:f64){
        if capture{
            self.capture_stalls+=1;self.last_capture_gap_ms=ms;
            self.max_capture_gap_ms=self.max_capture_gap_ms.max(ms);
        }else{
            self.gui_stalls+=1;self.last_gui_gap_ms=ms;
            self.max_gui_gap_ms=self.max_gui_gap_ms.max(ms);
        }
        if let Some(i)=self.second(tick){
            let b=&mut self.first_minute[i];
            if capture{b.capture_stalls+=1;}else{b.gui_stalls+=1;}
        }
    }
    pub fn status(&self,dropped:u64)->String{
        let cpu=|p:Option<(u16,u8)>|p.map_or_else(||"unknown".into(),
            |(g,n)|format!("group {g} / CPU {n}"));
        format!(
            "Raw Input receiver: {} events (keyboard {}, mouse {}); {} overflow-dropped\n\
             Capture callback max {:.3} ms | GUI handoff max {:.3} ms | GUI handler max {:.3} ms\n\
             GUI stalls {} (max {:.1}ms), independent input stalls {} (max {:.1}ms)\n\
             Last capture processor: {}; GUI processor: {}",
            self.raw_received,self.raw_keyboard,self.raw_mouse,dropped,
            self.max_capture_handler_us/1000.0,self.max_gui_handoff_ms,
            self.max_gui_handler_ms,
            self.gui_stalls,self.max_gui_gap_ms,self.capture_stalls,self.max_capture_gap_ms,
            cpu(self.latest_capture_cpu),cpu(self.latest_gui_cpu))
    }
    pub fn append_report(&self,out:&mut String,dropped:u64){
        let _=writeln!(out,"\n\nISOLATED INPUT CAPTURE / GUI SCHEDULING DIAGNOSTICS");
        let _=writeln!(out,"Input Raw WM_INPUT timestamp is from separate message-only-window thread.");
        let _=writeln!(out,"GUI handoff = GUI processing timestamp - capture callback entry QPC.");
        let _=writeln!(out,"Capture queue age uses GetMessageTime (coarse). Neither value is switch-to-photon latency.");
        let _=writeln!(out,"Watchdog gaps are observed thread heartbeats, not proven CPU starvation or kernel blocking.");
        let _=writeln!(out,"CPU identifiers are snapshots and may change because threads migrate.");
        let _=writeln!(out,"{}",self.status(dropped));
        let _=writeln!(out,"\nFIRST 60 SECONDS AFTER APPLICATION LAUNCH (not Windows boot time)");
        let _=writeln!(out,"second | raw_keyboard | raw_mouse | max_input_msg_age_ms | max_handoff_ms | max_capture_callback_us | max_gui_handler_ms | gui_stalls | input_stalls");
        for (i,b) in self.first_minute.iter().enumerate(){
            let _=writeln!(out,"{} | {} | {} | {} | {:.3} | {:.3} | {:.3} | {} | {}",
                i,b.keyboard,b.mouse,b.max_capture_queue_age_ms,
                b.max_gui_handoff_ms,b.max_capture_handler_us,b.max_gui_handler_ms,
                b.gui_stalls,b.capture_stalls);
        }
        let _=writeln!(out,"\nRECENT 60-SECOND SAMPLED INPUT ARRIVALS (bounded 20Hz normal, higher on GUI lag)");
        let _=writeln!(out,"t_since_program_launch_ms | device | kind | capture_queue_age_ms | GUI_handoff_ms | capture_callback_us | processor_group | logical_processor");
        for s in &self.recent{
            let ms=s.tick.saturating_sub(self.epoch) as f64*1000.0/self.freq as f64;
            let _=writeln!(out,"{ms:.3} | 0x{:x} | {} | {} | {:.3} | {:.3} | {} | {}",
                s.device,if s.keyboard{"KEY"}else{"MOUSE"},s.captured_age_ms,
                s.handoff_ms,s.callback_us,s.group,s.processor);
        }
    }
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn first_minute_is_bounded_and_reports_correct_second(){
        let mut t=ThreadMetrics::new(1_000_000,5_000_000);
        t.raw(TimingSample{tick:5_000_000,device:4,keyboard:true,
            captured_age_ms:312,handoff_ms:3.0,callback_us:11.0,
            group:1,processor:3});
        t.gap(false,5_250_000,305.0);
        t.raw(TimingSample{tick:66_000_000,device:4,keyboard:true,
            captured_age_ms:0,handoff_ms:0.1,callback_us:1.0,
            group:1,processor:5});
        assert_eq!(t.first_minute[0].keyboard,1);
        assert_eq!(t.first_minute[0].gui_stalls,1);
        assert_eq!(t.first_minute.iter().map(|b|b.keyboard).sum::<u32>(),1);
        assert!(t.status(0).contains("group 1 / CPU 5"));
    }
}
