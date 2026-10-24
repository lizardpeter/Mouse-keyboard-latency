//! Pure, platform-independent keyboard event diagnostics.
//! This diagnoses the Raw Input stream, NOT switch-to-PC absolute latency.

use std::collections::{BTreeMap, VecDeque};

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
    pub last_event_ms: Option<f64>,
    pub last_release_duration_ms: Option<f64>,
    pub max_release_duration_ms: f64,
    pub shortest_gap_ms: Option<f64>,
    pub latest_gap_ms: Option<f64>,
    log: VecDeque<KeyEventLog>,
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

        if is_down {
            if let Some(key) = self.held.get_mut(&id) {
                key.repeats += 1;
                self.repeats += 1;
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
                    self.last_release_duration_ms = Some(duration);
                    self.max_release_duration_ms = self.max_release_duration_ms.max(duration);
                    self.append_log(stamp, format!("UP      {label}  ·  held {duration:.1} ms"));
                }
                None => {
                    self.orphan_ups += 1;
                    self.append_log(stamp, format!("UNMATCHED UP  {label}"));
                }
            }
        }
    }

    /// Explicit capture boundaries cannot be interpreted as lost key-up events.
    /// Forget any held keys when pausing or leaving the diagnostics tab.
    pub fn suspend(&mut self) {
        self.held.clear();
        self.previous_ticks = None;
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
