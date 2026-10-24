//! Pure reaction-time trial state machine.
//! Stimulus time is the moment GUI drawing was submitted, NOT photons on screen.

use std::{fs::File, io::{self, BufWriter, Write}, path::Path};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Ready,
    Waiting { target_ticks: i64 },
    PendingPaint,
    Visible { shown_ticks: i64 },
    Complete { milliseconds: f64 },
    FalseStart,
}
impl Default for Phase { fn default()->Self { Self::Ready } }

#[derive(Default)]
pub struct ReactionTest {
    pub phase: Phase,
    pub results_ms: Vec<f64>,
    pub false_starts: u32,
}
impl ReactionTest {
    pub fn reset(&mut self) { *self = Self::default(); }
    pub fn arm(&mut self, now_ticks:i64, frequency:i64) {
        assert!(frequency>0);
        // Unpredictable-ish varying wait (not cryptographic randomness).
        let mut state=now_ticks as u64 ^ (now_ticks as u64).rotate_left(19)
            ^ (self.results_ms.len() as u64).wrapping_mul(0x9e3779b97f4a7c15);
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let wait_ms=1500+(state%2501) as i64;
        self.arm_with_delay(now_ticks, frequency, wait_ms);
    }
    pub fn arm_with_delay(&mut self, now:i64, frequency:i64, delay_ms:i64) {
        self.phase=Phase::Waiting {
            target_ticks: now.saturating_add(delay_ms.saturating_mul(frequency)/1000)
        };
    }
    pub fn tick(&mut self, now:i64)->bool {
        if let Phase::Waiting {target_ticks} = self.phase {
            if now>=target_ticks {
                self.phase=Phase::PendingPaint;
                return true;
            }
        }
        false
    }
    pub fn painted(&mut self, now:i64) {
        if self.phase==Phase::PendingPaint {
            self.phase=Phase::Visible{shown_ticks:now};
        }
    }
    /// Called only for Raw Input left mouse down, outside the controls.
    pub fn click(&mut self, now:i64, frequency:i64) {
        match self.phase {
            Phase::Waiting{..} | Phase::PendingPaint => {
                self.false_starts+=1;
                self.phase=Phase::FalseStart;
            }
            Phase::Visible{shown_ticks} => {
                let delay=now.saturating_sub(shown_ticks) as f64*1000.0/frequency as f64;
                self.results_ms.push(delay);
                self.phase=Phase::Complete{milliseconds:delay};
            }
            _ => {}
        }
    }
    pub fn average(&self)->Option<f64> {
        if self.results_ms.is_empty() {None} else {
            Some(self.results_ms.iter().sum::<f64>()/self.results_ms.len() as f64)
        }
    }
    pub fn best(&self)->Option<f64> {
        self.results_ms.iter().copied().reduce(f64::min)
    }
    pub fn save_csv(&self, path: &Path) -> io::Result<()> {
        let mut out = BufWriter::new(File::create(path)?);
        writeln!(out, "record_type,trial_number,reaction_time_ms,metric,value")?;
        for (index, value) in self.results_ms.iter().enumerate() {
            writeln!(out, "trial,{},{:.3},,", index + 1, value)?;
        }
        writeln!(out, "summary,,,valid_trials,{}", self.results_ms.len())?;
        writeln!(out, "summary,,,false_starts,{}", self.false_starts)?;
        writeln!(out, "summary,,,mean_reaction_ms,{}", self.average().map_or(String::new(),|n|format!("{n:.3}")))?;
        writeln!(out, "summary,,,best_reaction_ms,{}", self.best().map_or(String::new(),|n|format!("{n:.3}")))?;
        out.flush()
    }
    pub fn last(&self)->Option<f64> { self.results_ms.last().copied() }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn exports_reaction_trials_and_false_starts() {
        let mut r=ReactionTest::default();
        r.arm_with_delay(0,1_000_000,1000);
        r.tick(1_000_000);r.painted(1_000_000);
        r.click(1_247_000,1_000_000);
        r.arm_with_delay(2_000_000,1_000_000,1500);
        r.click(2_100_000,1_000_000);
        let file=std::env::temp_dir().join(format!("reaction_export_{}.csv",std::process::id()));
        r.save_csv(&file).unwrap();
        let contents=std::fs::read_to_string(&file).unwrap();
        let _=std::fs::remove_file(file);
        assert!(contents.contains("trial,1,247.000"));
        assert!(contents.contains("summary,,,false_starts,1"));
    }
    #[test]
    fn stimulus_requires_actual_paint_mark_before_valid_click(){
        let mut r=ReactionTest::default();
        r.arm_with_delay(10_000,1_000_000,2000);
        assert!(!r.tick(2_009_999));
        assert!(r.tick(2_010_000));
        assert_eq!(r.phase,Phase::PendingPaint);
        r.painted(2_025_000);
        r.click(2_275_000,1_000_000);
        assert_eq!(r.last(),Some(250.0));
        assert_eq!(r.average(),Some(250.0));
    }
    #[test]
    fn early_click_counts_false_start_and_no_result(){
        let mut r=ReactionTest::default();
        r.arm_with_delay(0,1_000_000,1500);
        r.click(50_000,1_000_000);
        assert_eq!(r.phase,Phase::FalseStart);
        assert_eq!(r.false_starts,1);
        assert!(r.results_ms.is_empty());
    }
    #[test]
    fn results_and_reset(){
        let mut r=ReactionTest::default();
        r.arm_with_delay(0,1_000_000,2000);
        r.tick(2_000_000);r.painted(2_000_000);r.click(2_300_000,1_000_000);
        r.arm_with_delay(3_000_000,1_000_000,1000);
        r.tick(4_000_000);r.painted(4_000_000);r.click(4_200_000,1_000_000);
        assert_eq!(r.best(),Some(200.0));
        assert_eq!(r.average(),Some(250.0));
        r.reset();
        assert_eq!(r.phase,Phase::Ready);
    }
    #[test]
    fn paint_delayed_click_counts_false_start(){
        let mut r=ReactionTest::default();
        r.arm_with_delay(0,1_000_000,2000);
        r.tick(2_000_000);
        r.click(2_001_000,1_000_000);
        r.painted(2_050_000);
        assert_eq!(r.phase,Phase::FalseStart);
    }
}
