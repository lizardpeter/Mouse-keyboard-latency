//! Pure, platform-independent keyboard event diagnostics.
//! This diagnoses the Raw Input stream, NOT switch-to-PC absolute latency.

use std::{
    collections::{BTreeMap, VecDeque},
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

pub const SUSPECT_HELD_MS: f64 = 3000.0;
const LOG_LIMIT: usize = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct KeyId {
    pub scan: u16,
    pub extended: u8,
}
impl KeyId {
    pub fn new(scan: u16, flags: u16) -> Self {
        Self { scan, extended: ((flags & 0x06) >> 1) as u8 }
    }
}

#[derive(Clone, Debug)]
pub struct KeyEventLog {
    pub when_ms: f64,
    pub text: String,
}

/// Structured data for every recorded key event. The small on-screen log is
/// deliberately independent of this complete export history.
#[derive(Clone, Debug)]
pub struct DiagnosticEvent {
    pub time_ms: f64,
    pub key_id: KeyId,
    pub label: String,
    pub action: &'static str,
    pub held_duration_ms: Option<f64>,
    pub simultaneous_held: usize,
    pub queue_age_ms: Option<u32>,
}

/// Prevent pathological multi-hour sessions from exhausting memory.
const MAX_EXPORTED_EVENTS: usize = 250_000;

fn csv_field(value: &str) -> String {
    let escaped = value.replace('"', """");
    format!(""{}"", escaped)
}

#[derive(Clone)]
pub struct HeldKey {
    pub label: String,
    pub since: i64,
    pub repeats: u32,
}
#[derive(Default)]
pub struct KeyboardDiagnostics {
    held: BTreeMap<KeyId, HeldKey>,
    pub downs: u64,
    pub ups: u64,
    pub repeats: u64,
    pub orphan_ups: u64,
    pub maximum_simultaneous: usize,
    /// Approximate age of Raw Input's Windows message when the handler runs.
    pub latest_queue_age_ms: Option<u32>,
    pub largest_queue_age_ms: u32,
    pub slow_queue_events: u64,
    pub last_event_ms: Option<f64>,
    pub last_release_duration_ms: Option<f64>,
    pub max_release_duration_ms: f64,
    pub shortest_gap_ms: Option<f64>,
    pub latest_gap_ms: Option<f64>,
    log: VecDeque<KeyEventLog>,
    events: Vec<DiagnosticEvent>,
    pub truncated_events: u64,
    previous_ticks: Option<i64>,
    base_ticks: Option<i64>,
    frequency: i64,
}

impl KeyboardDiagnostics {
    pub fn new(frequency: i64) -> Self {
        assert!(frequency > 0);
        Self { frequency, ..Self::default() }
    }

    fn ms(&self, ticks: i64) -> f64 {
        ticks as f64 * 1000.0 / self.frequency as f64
    }
    fn append_log(&mut self, time_ms: f64, text: String) {
        self.log.push_front(KeyEventLog { when_ms: time_ms, text });
        self.log.truncate(LOG_LIMIT);
    }

    /// Process a decoded Windows Raw Keyboard record.
    /// KeyId contains the scan code and E0/E1 distinction to separate left/right.
    pub fn event(&mut self, id: KeyId, label: String, is_down: bool, ticks: i64) {
        let base = *self.base_ticks.get_or_insert(ticks);
        let stamp = self.ms(ticks.saturating_sub(base));
        if let Some(before) = self.previous_ticks {
            if ticks >= before {
                let gap = self.ms(ticks - before);
                self.latest_gap_ms = Some(gap);
                self.shortest_gap_ms = Some(self.shortest_gap_ms.map_or(gap, |x| x.min(gap)));
            }
        }
        self.previous_ticks = Some(ticks);
        self.last_event_ms = Some(stamp);

        let mut action = "down";
        let mut duration_ms = None;
        if is_down {
            if let Some(key) = self.held.get_mut(&id) {
                key.repeats += 1;
                self.repeats += 1;
                action = "repeat";
                self.append_log(stamp, format!("REPEAT  {label} (held)"));
            } else {
                self.downs += 1;
                self.held.insert(id, HeldKey { label: label.clone(), since: ticks, repeats: 0 });
                self.maximum_simultaneous = self.maximum_simultaneous.max(self.held.len());
                self.append_log(stamp, format!("DOWN    {label}"));
            }
        } else {
            match self.held.remove(&id) {
                Some(key) => {
                    self.ups += 1;
                    let duration = self.ms(ticks.saturating_sub(key.since));
                    action = "up";
                    duration_ms = Some(duration);
                    self.last_release_duration_ms = Some(duration);
                    self.max_release_duration_ms = self.max_release_duration_ms.max(duration);
                    self.append_log(stamp, format!("UP      {label}  ·  held {duration:.1} ms"));
                }
                None => {
                    self.orphan_ups += 1;
                    action = "unmatched_up";
                    self.append_log(stamp, format!("UNMATCHED UP  {label}"));
                }
            }
        }
        if self.events.len() < MAX_EXPORTED_EVENTS {
            self.events.push(DiagnosticEvent {
                time_ms: stamp,
                key_id: id,
                label,
                action,
                held_duration_ms: duration_ms,
                simultaneous_held: self.held.len(),
                queue_age_ms: self.latest_queue_age_ms,
            });
        } else {
            self.truncated_events += 1;
        }
    }

    /// Explicit capture boundaries cannot be interpreted as lost key-up events.
    /// Forget any held keys when pausing or leaving the diagnostics tab.
    pub fn suspend(&mut self) {
        self.held.clear();
        self.previous_ticks = None;
    }

    pub fn observe_queue_age(&mut self, milliseconds: u32) {
        // GETMESSAGETIME is only millisecond resolution and may be platform dependent.
        // Discard unreasonable values caused by wrap/timestamp mismatch.
        if milliseconds > 5_000 { return; }
        self.latest_queue_age_ms = Some(milliseconds);
        self.largest_queue_age_ms = self.largest_queue_age_ms.max(milliseconds);
        if milliseconds >= 32 { self.slow_queue_events += 1; }
    }

    /// Export the full diagnostic event history, current held-key snapshot, and
    /// aggregate counters. Unlike the GUI preview, this does not stop at 14 events.
    pub fn save_csv(&self, path: &Path, now_ticks: i64) -> io::Result<()> {
        let mut out = BufWriter::new(File::create(path)?);
        writeln!(out, "record_type,time_ms,key,scan_code,extended,event_type,held_duration_ms,simultaneous_held,queue_age_ms,metric,value")?;
        for event in &self.events {
            writeln!(
                out,
                "event,{:.3},{},{},{},{},{},{},{},,",
                event.time_ms,
                csv_field(&event.label),
                event.key_id.scan,
                event.key_id.extended,
                event.action,
                event.held_duration_ms.map_or(String::new(), |v| format!("{v:.3}")),
                event.simultaneous_held,
                event.queue_age_ms.map_or(String::new(), |v| v.to_string()),
            )?;
        }
        // Current down states must be part of the export: these are often the
        // evidence of a key-up that never reached this process.
        for (id, key) in &self.held {
            writeln!(
                out,
                "held_snapshot,,{},{},{},held,{:.3},,,,",
                csv_field(&key.label),
                id.scan,
                id.extended,
                self.ms(now_ticks.saturating_sub(key.since)),
            )?;
        }

        for (name, value) in [
            ("key_down_events", self.downs.to_string()),
            ("key_up_events", self.ups.to_string()),
            ("auto_repeats", self.repeats.to_string()),
            ("unmatched_key_ups", self.orphan_ups.to_string()),
            ("currently_held_keys", self.held.len().to_string()),
            ("max_simultaneous_keys", self.maximum_simultaneous.to_string()),
            ("slow_queue_events_32ms_or_more", self.slow_queue_events.to_string()),
            ("largest_queue_age_ms", self.largest_queue_age_ms.to_string()),
            ("longest_held_duration_ms", format!("{:.3}", self.max_release_duration_ms)),
            ("latest_held_duration_ms", self.last_release_duration_ms.map_or(String::new(), |v| format!("{v:.3}"))),
            ("shortest_inter_event_gap_ms", self.shortest_gap_ms.map_or(String::new(), |v| format!("{v:.3}"))),
            ("latest_inter_event_gap_ms", self.latest_gap_ms.map_or(String::new(), |v| format!("{v:.3}"))),
            ("latest_queue_age_ms", self.latest_queue_age_ms.map_or(String::new(), |v| v.to_string())),
            ("exported_event_count", self.events.len().to_string()),
            ("dropped_events_after_capacity", self.truncated_events.to_string()),
            ("suspected_long_held_keys", self.stuck_candidates(now_ticks).len().to_string()),
        ] {
            writeln!(out, "summary,,,,,,,,,{},{}", csv_field(name), csv_field(&value))?;
        }
        out.flush()
    }

    pub fn held_count(&self) -> usize { self.held.len() }
    pub fn stuck_candidates(&self, now: i64) -> Vec<(String, f64)> {
        self.held.values().filter_map(|key| {
            let duration = self.ms(now.saturating_sub(key.since));
            (duration >= SUSPECT_HELD_MS).then(|| (key.label.clone(), duration))
        }).collect()
    }
    pub fn held_display(&self, now: i64) -> String {
        if self.held.is_empty() { return "None — all observed key-downs have key-ups".into(); }
        self.held.values().take(24).map(|key| {
            format!("{}  ({:.0} ms)", key.label, self.ms(now.saturating_sub(key.since)))
        }).collect::<Vec<_>>().join("   |   ")
    }
    pub fn log_display(&self) -> String {
        if self.log.is_empty() { return "No keyboard events yet. Press and release keys to begin.".into(); }
        self.log.iter().map(|item| format!("{:>9.2} ms   {}", item.when_ms, item.text))
            .collect::<Vec<_>>().join("\r\n")
    }
    pub fn alert_display(&self, now: i64) -> String {
        let stuck = self.stuck_candidates(now);
        let mut alerts = Vec::new();
        if !stuck.is_empty() {
            alerts.push(format!("{} long-held key(s) over {:.0}s — may be intentional; check release",
                stuck.len(), SUSPECT_HELD_MS / 1000.0));
        }
        if self.orphan_ups > 0 {
            alerts.push(format!("{} key-up event(s) without a matching down (possibly started before capture)",
                self.orphan_ups));
        }
        if alerts.is_empty() {
            "No suspicious key-up/down mismatches currently detected.".into()
        } else { alerts.join("  •  ") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(scan:u16) -> KeyId { KeyId::new(scan,0) }

    #[test]
    fn csv_export_contains_every_event_and_still_held_keys() {
        let mut d = KeyboardDiagnostics::new(1_000_000);
        for i in 0..30 {
            let tick = i as i64 * 100_000;
            d.observe_queue_age(4);
            d.event(key(30), "A".into(), true, tick);
            d.event(key(30), "A".into(), false, tick+50_000);
        }
        d.event(key(31), "S,quoted\"test".into(), true, 3_100_000);
        let path = std::env::temp_dir().join(format!("keyboard_diagnostic_export_{}.csv", std::process::id()));
        d.save_csv(&path, 3_200_000).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(contents.lines().filter(|line| line.starts_with("event,")).count(), 61);
        assert!(contents.contains("held_snapshot"));
        assert!(contents.contains("\"S,quoted\"\"test\""));
        assert!(contents.contains("unmatched_key_ups"));
        assert!(contents.contains("max_simultaneous_keys"));
        assert!(contents.contains("dropped_events_after_capacity"));
        assert!(contents.contains("queue_age_ms"));
    }
    #[test]
    fn balanced_key_down_up_tracks_hold_time() {
        let mut d = KeyboardDiagnostics::new(1_000_000);
        d.event(key(30),"A".into(),true,1_000_000);
        assert_eq!(d.held_count(),1);
        d.event(key(30),"A".into(),false,1_125_000);
        assert_eq!((d.downs,d.ups,d.held_count()),(1,1,0));
        assert_eq!(d.last_release_duration_ms,Some(125.0));
        assert!(d.stuck_candidates(10_000_000).is_empty());
    }
    #[test]
    fn repeat_down_is_not_extra_pressed_key() {
        let mut d=KeyboardDiagnostics::new(1_000_000);
        d.event(key(30),"A".into(),true,1);
        d.event(key(30),"A".into(),true,2);
        d.event(key(30),"A".into(),true,3);
        assert_eq!((d.downs,d.repeats,d.held_count()),(1,2,1));
        d.event(key(30),"A".into(),false,4);
        assert_eq!((d.ups,d.held_count()),(1,0));
    }
    #[test]
    fn missing_release_is_suspected_and_not_claimed_certain() {
        let mut d=KeyboardDiagnostics::new(1_000_000);
        d.event(key(30),"A".into(),true,2);
        assert!(d.stuck_candidates(2_999_999).is_empty());
        assert_eq!(d.stuck_candidates(3_000_002).len(),1);
        d.event(key(30),"A".into(),false,3_200_002);
        assert!(d.stuck_candidates(10_000_000).is_empty());
    }
    #[test]
    fn unmatched_release_and_multikey_rollover() {
        let mut d=KeyboardDiagnostics::new(1_000_000);
        d.event(key(30),"A".into(),false,1);
        assert_eq!(d.orphan_ups,1);
        for x in 30..38 { d.event(key(x),format!("K{x}"),true,x as i64+1); }
        assert_eq!(d.maximum_simultaneous,8);
        assert_eq!(d.held_count(),8);
    }
    #[test]
    fn queue_age_is_separate_from_hardware_latency() {
        let mut d=KeyboardDiagnostics::new(1_000_000);
        d.observe_queue_age(4);
        d.observe_queue_age(36);
        d.observe_queue_age(u32::MAX);
        assert_eq!(d.latest_queue_age_ms,Some(36));
        assert_eq!(d.largest_queue_age_ms,36);
        assert_eq!(d.slow_queue_events,1);
    }
    #[test]
    fn capture_pause_does_not_report_phantom_stuck_key() {
        let mut d = KeyboardDiagnostics::new(1_000_000);
        d.event(key(30), "A".into(), true, 1_000_000);
        d.suspend();
        assert_eq!(d.held_count(), 0);
        assert!(d.stuck_candidates(9_000_000).is_empty());
        assert_eq!(d.downs, 1);
    }
    #[test]
    fn extended_scancode_keeps_keys_separate() {
        let mut d=KeyboardDiagnostics::new(1_000_000);
        d.event(KeyId::new(29,0),"LCTRL".into(),true,10);
        d.event(KeyId::new(29,2),"RCTRL".into(),true,11);
        assert_eq!(d.held_count(),2);
    }
}
