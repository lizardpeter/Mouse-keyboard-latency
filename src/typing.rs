//! A focused-window typing test. Captures the EDIT control's WM_CHAR timing
//! and Raw Input arrival timing, not the contents of other applications.
use std::{fmt::Write as FmtWrite, fs::File, io::{self, BufWriter, Write}, path::Path};

const EVENT_CAP: usize = 50_000;

#[derive(Clone, Debug)]
pub struct TypingEvent {
    pub time_ms: f64,
    pub channel: &'static str,
    pub description: String,
    pub gap_ms: Option<f64>,
}

pub struct TypingMonitor {
    frequency: i64,
    epoch_ticks: i64,
    previous_char_ticks: Option<i64>,
    previous_raw_ticks: Option<i64>,
    consecutive_short_gaps: u32,
    pub chars: u64,
    pub raw_downs: u64,
    pub raw_ups: u64,
    pub raw_repeats: u64,
    pub short_char_gaps: u64,
    pub clusters: u64,
    pub largest_char_gap_ms: f64,
    pub latest_char_gap_ms: Option<f64>,
    pub last_raw_gap_ms: Option<f64>,
    pub dropped_rows: u64,
    pub events: Vec<TypingEvent>,
}
impl TypingMonitor {
    pub fn new(frequency:i64, now:i64)->Self {
        assert!(frequency > 0);
        Self {
            frequency,epoch_ticks:now,previous_char_ticks:None,
            previous_raw_ticks:None,consecutive_short_gaps:0,
            chars:0,raw_downs:0,raw_ups:0,raw_repeats:0,
            short_char_gaps:0,clusters:0,largest_char_gap_ms:0.0,
            latest_char_gap_ms:None,last_raw_gap_ms:None,
            dropped_rows:0,events:Vec::new(),
        }
    }
    fn ms(&self,ticks:i64)->f64 {
        ticks.max(0) as f64 * 1000.0 / self.frequency as f64
    }
    fn push(&mut self, event:TypingEvent) {
        if self.events.len()<EVENT_CAP {self.events.push(event);}
        else {self.dropped_rows+=1;}
    }
    pub fn raw(&mut self,label:&str,action:&'static str,ticks:i64) {
        match action {
            "DOWN"=>self.raw_downs+=1,
            "UP"=>self.raw_ups+=1,
            "REPEAT"=>self.raw_repeats+=1,
            _=>{}
        }
        let gap=self.previous_raw_ticks.map(|before|self.ms(ticks.saturating_sub(before)));
        self.previous_raw_ticks=Some(ticks);
        self.last_raw_gap_ms=gap;
        self.push(TypingEvent{
            time_ms:self.ms(ticks.saturating_sub(self.epoch_ticks)),
            channel:"RAW_INPUT",
            description:format!("{action} {label}"),
            gap_ms:gap,
        });
    }
    /// Received by the focused test EDIT window BEFORE it passes WM_CHAR to
    /// the original edit control procedure. UTF-16 code units are displayed
    /// explicitly for non-BMP characters.
    pub fn character(&mut self,utf16:u16,ticks:i64) {
        let gap=self.previous_char_ticks.map(|before|self.ms(ticks.saturating_sub(before)));
        self.previous_char_ticks=Some(ticks);
        self.chars+=1;
        if let Some(gap_ms)=gap {
            self.latest_char_gap_ms=Some(gap_ms);
            self.largest_char_gap_ms=self.largest_char_gap_ms.max(gap_ms);
            if gap_ms<5.0 {
                self.short_char_gaps+=1;
                self.consecutive_short_gaps+=1;
                if self.consecutive_short_gaps==2 {self.clusters+=1;}
            } else {
                self.consecutive_short_gaps=0;
            }
        }
        let description=match utf16 {
            8=>"[Backspace]".to_string(),
            9=>"[Tab]".to_string(),
            10|13=>"[Enter]".to_string(),
            27=>"[Escape]".to_string(),
            32=>"[Space]".to_string(),
            0..=31=>format!("[Control 0x{utf16:02X}]"),
            c if (0xD800..=0xDFFF).contains(&c)=>format!("[UTF-16 U+{c:04X}]"),
            c=>char::from_u32(c as u32).map_or_else(||format!("[U+{c:04X}]"),|v|v.to_string()),
        };
        self.push(TypingEvent{
            time_ms:self.ms(ticks.saturating_sub(self.epoch_ticks)),
            channel:"WM_CHAR",
            description,
            gap_ms:gap,
        });
    }
    pub fn recent(&self,channel:&str,count:usize)->String {
        let mut entries=self.events.iter().filter(|event|event.channel==channel)
            .rev().take(count).map(|event|format!(
                "{:>9.2} ms   {:<15}   gap: {}",
                event.time_ms,event.description,
                event.gap_ms.map_or_else(||"—".to_string(),|gap|format!("{gap:.3} ms"))
            )).collect::<Vec<_>>();
        entries.reverse();
        if entries.is_empty() {"No events yet. Click the text box and type test characters.".into()}
        else {entries.join("\r\n")}
    }
    pub fn stats(&self)->String {
        format!(
            "Actual WM_CHAR: {}   |   gaps <5 ms: {}   |   multi-char clusters: {}   |   Raw down/up/repeat: {}/{}/{}",
            self.chars,self.short_char_gaps,self.clusters,
            self.raw_downs,self.raw_ups,self.raw_repeats,
        )
    }
    pub fn append_report(&self, report:&mut String, visible_text:&str) {
        let _=writeln!(report,"\n\nFOCUSED TYPING TEST — WINDOWS TEXT OUTPUT vs RAW INPUT");
        let _=writeln!(report,"Text measured from the test box's WM_CHAR, not Chrome/Notepad.");
        let _=writeln!(report,"Timestamps are when this process handled messages, NOT keyboard hardware times.");
        let _=writeln!(report,"Gaps <5 ms and clusters are timing clues, NOT proof of bad hardware or delayed delivery.");
        let _=writeln!(report,"Only typed characters inside the test box are listed as WM_CHAR.");
        let _=writeln!(report,"Pasted text, IME/composition changes and programmatic edits are not individual WM_CHAR events.");
        let _=writeln!(report,"WM_CHAR message count: {}",self.chars);
        let _=writeln!(report,"Raw keyboard DOWN / UP / REPEAT: {} / {} / {}",
            self.raw_downs,self.raw_ups,self.raw_repeats);
        let _=writeln!(report,"Character gaps <5 ms: {}",self.short_char_gaps);
        let _=writeln!(report,"Clusters with >=3 WM_CHAR messages separated by <5 ms: {}",self.clusters);
        let _=writeln!(report,"Longest inter-character gap (includes idle pauses): {:.3} ms",
            self.largest_char_gap_ms);
        let _=writeln!(report,"Latest inter-character gap: {}",
            self.latest_char_gap_ms.map_or_else(||"N/A".into(),|v|format!("{v:.3} ms")));
        let _=writeln!(report,"Event rows saved: {} (omitted after capacity: {})",
            self.events.len(),self.dropped_rows);
        let _=writeln!(report,"\nACTUAL TEST BOX CONTENTS AT SAVE TIME (contains typed text; review before sharing)");
        let _=writeln!(report,"<BEGIN TEST TEXT>\n{visible_text}\n<END TEST TEXT>");
        let _=writeln!(report,"\nINTERLEAVED RAW INPUT AND TEXT MESSAGE TIMELINE");
        let _=writeln!(report,"time_ms | channel | action/character | preceding_same_channel_gap_ms");
        for event in &self.events {
            let _=writeln!(report,"{:.3} | {} | {} | {}",
                event.time_ms,event.channel,event.description,
                event.gap_ms.map_or_else(||"-".into(),|v|format!("{v:.3}")));
        }
    }
    pub fn save_csv(&self,path:&Path)->io::Result<()> {
        let mut file=BufWriter::new(File::create(path)?);
        writeln!(file,"time_ms,channel,action_or_character,preceding_channel_gap_ms")?;
        for e in &self.events {
            let quoted=e.description.replace('"',"\"\"");
            writeln!(file,"{:.3},{},\"{}\",{}",
                e.time_ms,e.channel,quoted,
                e.gap_ms.map_or_else(String::new,|v|format!("{v:.3}")))?;
        }
        file.flush()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_normal_typing_from_message_burst(){
        let mut t=TypingMonitor::new(1_000_000,0);
        t.raw("A","DOWN",90_000);
        t.character('a' as u16,100_000);
        t.character('b' as u16,145_000);
        t.character('c' as u16,145_700);
        t.character('d' as u16,146_100);
        t.character('e' as u16,186_000);
        assert_eq!(t.chars,5);
        assert_eq!(t.short_char_gaps,2);
        assert_eq!(t.clusters,1);
        assert_eq!(t.raw_downs,1);
        assert!(t.recent("WM_CHAR",5).contains("0.400 ms"));
    }
    #[test]
    fn report_contains_actual_text_and_does_not_infer_pastes(){
        let mut t=TypingMonitor::new(1_000_000,0);
        t.character('A' as u16,100_000);
        t.character(8,200_000);
        let mut report="header".to_string();
        t.append_report(&mut report,"test123");
        assert!(report.contains("test123"));
        assert!(report.contains("[Backspace]"));
        assert!(report.contains("NOT keyboard hardware times"));
    }
}
