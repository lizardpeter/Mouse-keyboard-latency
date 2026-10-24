//! Platform-independent event pairing, reporting, and CSV export.

use std::{
    cmp::Ordering,
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

pub const PAIR_WINDOW_MS: f64 = 300.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Mouse,
    Keyboard,
}

#[derive(Clone, Copy, Debug)]
struct Event {
    kind: Kind,
    ticks: i64,
}

#[derive(Clone, Debug)]
pub struct Trial {
    pub number: usize,
    pub mouse_ms: f64,
    pub keyboard_ms: f64,
    /// Positive if the mouse arrived first.
    pub keyboard_minus_mouse_ms: f64,
}

#[derive(Debug, PartialEq)]
pub struct Stats {
    pub count: usize,
    pub mouse_first: usize,
    pub keyboard_first: usize,
    pub ties: usize,
    pub mean_signed_ms: f64,
    pub median_signed_ms: f64,
    pub median_absolute_ms: f64,
    pub p95_absolute_ms: f64,
    pub minimum_absolute_ms: f64,
    pub maximum_absolute_ms: f64,
}

/// Independent per-device receipt statistics (not absolute input latency).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeviceSnapshot {
    pub press_count: usize,
    pub last_event_ms: Option<f64>,
    pub age_ms: Option<f64>,
    pub last_interval_ms: Option<f64>,
    pub mean_interval_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct DeviceAccumulator {
    count: usize,
    last_ticks: Option<i64>,
    last_interval_ticks: Option<i64>,
    total_interval_ticks: i64,
}

impl DeviceAccumulator {
    fn record(&mut self, ticks: i64) {
        if let Some(previous) = self.last_ticks {
            // Reordered timestamps should not corrupt the interval average.
            if ticks >= previous {
                let difference = ticks - previous;
                self.last_interval_ticks = Some(difference);
                self.total_interval_ticks += difference;
            }
        }
        self.count += 1;
        self.last_ticks = Some(ticks);
    }
}

pub struct Session {
    start_ticks: i64,
    frequency: i64,
    pending: Option<Event>,
    mouse_held: bool,
    keyboard_held: bool,
    trials: Vec<Trial>,
    mouse_live: DeviceAccumulator,
    keyboard_live: DeviceAccumulator,
}

impl Session {
    pub fn new(start_ticks: i64, frequency: i64) -> Self {
        assert!(frequency > 0, "QPC frequency must be positive");
        Self {
            start_ticks,
            frequency,
            pending: None,
            mouse_held: false,
            keyboard_held: false,
            trials: Vec::new(),
            mouse_live: DeviceAccumulator::default(),
            keyboard_live: DeviceAccumulator::default(),
        }
    }


    /// Return captured pairs for display.
    pub fn trials(&self) -> &[Trial] { &self.trials }

    /// Prevent a partial pair carrying across pause or save operations.
    pub fn break_pair(&mut self) {
        self.pending = None;
        self.mouse_held = false;
        self.keyboard_held = false;
    }

    pub fn reset(&mut self, now_ticks: i64) {
        *self = Session::new(now_ticks, self.frequency);
    }

    /// See the independent activity for either device, whether paired or not.
    pub fn device_snapshot(&self, kind: Kind, now_ticks: i64) -> DeviceSnapshot {
        let acc = match kind {
            Kind::Mouse => &self.mouse_live,
            Kind::Keyboard => &self.keyboard_live,
        };
        let to_ms = |v: i64| self.ms(v);
        DeviceSnapshot {
            press_count: acc.count,
            last_event_ms: acc.last_ticks.map(|v| to_ms(v - self.start_ticks)),
            age_ms: acc.last_ticks.map(|v| to_ms(now_ticks.saturating_sub(v))),
            last_interval_ms: acc.last_interval_ticks.map(to_ms),
            mean_interval_ms: if acc.count >= 2 {
                Some(to_ms(acc.total_interval_ticks) / (acc.count - 1) as f64)
            } else {
                None
            },
        }
    }

    pub fn trial_count(&self) -> usize {
        self.trials.len()
    }

    pub fn mouse_button(&mut self, down: bool, ticks: i64) {
        if down {
            if !self.mouse_held {
                self.mouse_held = true;
                self.add_event(Kind::Mouse, ticks);
            }
        } else {
            self.mouse_held = false;
        }
    }

    pub fn space_key(&mut self, down: bool, ticks: i64) {
        if down {
            if !self.keyboard_held {
                self.keyboard_held = true;
                self.add_event(Kind::Keyboard, ticks);
            }
        } else {
            self.keyboard_held = false;
        }
    }

    fn ms(&self, ticks: i64) -> f64 {
        ticks as f64 * 1000.0 / self.frequency as f64
    }

    fn add_event(&mut self, kind: Kind, ticks: i64) {
        match kind {
            Kind::Mouse => self.mouse_live.record(ticks),
            Kind::Keyboard => self.keyboard_live.record(ticks),
        }
        let next = Event { kind, ticks };
        let Some(previous) = self.pending.take() else {
            self.pending = Some(next);
            return;
        };

        let elapsed_ms = self.ms(ticks - previous.ticks);
        if previous.kind == kind || !(0.0..=PAIR_WINDOW_MS).contains(&elapsed_ms) {
            self.pending = Some(next);
            return;
        }

        let (mouse, keyboard) = if kind == Kind::Mouse {
            (next, previous)
        } else {
            (previous, next)
        };
        self.trials.push(Trial {
            number: self.trials.len() + 1,
            mouse_ms: self.ms(mouse.ticks - self.start_ticks),
            keyboard_ms: self.ms(keyboard.ticks - self.start_ticks),
            keyboard_minus_mouse_ms: self.ms(keyboard.ticks - mouse.ticks),
        });
    }

    pub fn stats(&self) -> Option<Stats> {
        let count = self.trials.len();
        if count == 0 {
            return None;
        }

        let mut signed: Vec<f64> = self.trials.iter().map(|t| t.keyboard_minus_mouse_ms).collect();
        let mut absolute: Vec<f64> = signed.iter().map(|t| t.abs()).collect();
        let mouse_first = signed.iter().filter(|&&v| v > 0.0).count();
        let keyboard_first = signed.iter().filter(|&&v| v < 0.0).count();
        let mean_signed_ms = signed.iter().sum::<f64>() / count as f64;
        signed.sort_by(f64::total_cmp);
        absolute.sort_by(f64::total_cmp);

        Some(Stats {
            count,
            mouse_first,
            keyboard_first,
            ties: count - mouse_first - keyboard_first,
            mean_signed_ms,
            median_signed_ms: percentile(&signed, 0.50),
            median_absolute_ms: percentile(&absolute, 0.50),
            p95_absolute_ms: percentile(&absolute, 0.95),
            minimum_absolute_ms: absolute[0],
            maximum_absolute_ms: absolute[count - 1],
        })
    }

    pub fn print_summary(&self) {
        let Some(s) = self.stats() else {
            println!("No completed trials yet. Press left mouse and Space close together.");
            return;
        };
        println!("\n------------- RESULTS ({} pairs) -------------", s.count);
        println!(
            "Mouse first: {} | Keyboard first: {} | Ties: {}",
            s.mouse_first, s.keyboard_first, s.ties
        );
        println!("Mean signed (keyboard - mouse): {:+.3} ms", s.mean_signed_ms);
        println!("Median signed:                 {:+.3} ms", s.median_signed_ms);
        println!("Median absolute separation:    {:.3} ms", s.median_absolute_ms);
        println!("95th percentile separation:    {:.3} ms", s.p95_absolute_ms);
        println!(
            "Minimum / maximum separation: {:.3} / {:.3} ms\n",
            s.minimum_absolute_ms, s.maximum_absolute_ms
        );
        println!("Positive signed values mean the mouse event arrived first.");
    }

    pub fn save_csv(&self, path: &Path) -> io::Result<()> {
        let mut writer = BufWriter::new(File::create(path)?);
        writeln!(
            writer,
            "trial,mouse_time_ms,keyboard_time_ms,keyboard_minus_mouse_ms,first_arrival"
        )?;
        for t in &self.trials {
            let winner = match t.keyboard_minus_mouse_ms.partial_cmp(&0.0) {
                Some(Ordering::Greater) => "mouse",
                Some(Ordering::Less) => "keyboard",
                _ => "tie",
            };
            writeln!(
                writer,
                "{},{:.6},{:.6},{:.6},{}",
                t.number, t.mouse_ms, t.keyboard_ms, t.keyboard_minus_mouse_ms, winner
            )?;
        }
        writer.flush()
    }
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let pos = (sorted.len() - 1) as f64 * fraction;
    let lower = pos.floor() as usize;
    let upper = pos.ceil() as usize;
    sorted[lower] + (sorted[upper] - sorted[lower]) * (pos - lower as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_live_mouse_and_keyboard_activity() {
        let mut s = Session::new(50_000, 1_000_000);
        s.mouse_button(true, 60_000);
        let mouse = s.device_snapshot(Kind::Mouse, 61_000);
        let keyboard = s.device_snapshot(Kind::Keyboard, 61_000);
        assert_eq!(mouse.press_count, 1);
        assert_eq!(mouse.last_event_ms, Some(10.0));
        assert_eq!(mouse.age_ms, Some(1.0));
        assert_eq!(mouse.last_interval_ms, None);
        assert_eq!(keyboard.press_count, 0);
        assert_eq!(keyboard.age_ms, None);
        assert_eq!(s.trial_count(), 0);
        s.space_key(true, 62_000);
        assert_eq!(s.device_snapshot(Kind::Keyboard, 63_000).press_count, 1);
        assert_eq!(s.trial_count(), 1);
    }

    #[test]
    fn repeat_inputs_and_pause_do_not_add_phantom_presses() {
        let mut s = Session::new(0, 1_000_000);
        s.mouse_button(true, 1_000);
        s.mouse_button(true, 2_000);
        assert_eq!(s.device_snapshot(Kind::Mouse, 5_000).press_count, 1);
        s.mouse_button(false, 3_000);
        s.mouse_button(true, 6_000);
        let view = s.device_snapshot(Kind::Mouse, 7_000);
        assert_eq!(view.press_count, 2);
        assert_eq!(view.last_interval_ms, Some(5.0));
        assert_eq!(view.mean_interval_ms, Some(5.0));
        s.break_pair();
        assert_eq!(s.device_snapshot(Kind::Mouse, 7_000).press_count, 2);
        s.reset(20_000);
        assert_eq!(s.device_snapshot(Kind::Mouse, 21_000).press_count, 0);
    }


    #[test]
    fn mouse_first_is_positive() {
        let mut s = Session::new(0, 1_000_000);
        s.mouse_button(true, 100_000);
        s.space_key(true, 102_500);
        assert_eq!(s.trial_count(), 1);
        assert_eq!(s.stats().unwrap().median_signed_ms, 2.5);
    }

    #[test]
    fn keyboard_first_is_negative() {
        let mut s = Session::new(0, 1_000_000);
        s.space_key(true, 100_000);
        s.mouse_button(true, 104_000);
        assert_eq!(s.stats().unwrap().median_signed_ms, -4.0);
    }

    #[test]
    fn ignores_autorepeat_until_release() {
        let mut s = Session::new(0, 1_000_000);
        s.space_key(true, 1_000);
        s.space_key(true, 2_000);
        s.mouse_button(true, 3_000);
        assert_eq!(s.trial_count(), 1);
        s.mouse_button(true, 4_000);
        s.space_key(true, 5_000);
        assert_eq!(s.trial_count(), 1);
        s.mouse_button(false, 6_000);
        s.space_key(false, 7_000);
        s.mouse_button(true, 8_000);
        s.space_key(true, 9_000);
        assert_eq!(s.trial_count(), 2);
    }

    #[test]
    fn old_unmatched_events_do_not_pair() {
        let mut s = Session::new(0, 1_000_000);
        s.mouse_button(true, 1);
        s.space_key(true, 400_002);
        assert_eq!(s.trial_count(), 0);
    }

    #[test]
    fn duplicate_kinds_replace_pending() {
        let mut s = Session::new(0, 1_000_000);
        s.mouse_button(true, 1_000);
        s.mouse_button(false, 2_000);
        s.mouse_button(true, 3_000);
        s.space_key(true, 4_000);
        assert_eq!(s.trial_count(), 1);
        assert_eq!(s.stats().unwrap().median_signed_ms, 1.0);
    }

    #[test]
    fn statistics_interpolate_percentile() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&a, 0.5), 3.0);
        assert_eq!(percentile(&a, 0.95), 4.8);
    }
}
