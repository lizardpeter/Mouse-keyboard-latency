//! Independent passive GUI-message-pump heartbeat observer.
//! A private in-process thread reads an atomic QPC heartbeat every 10ms;
//! it changes NO Windows timer resolution, service, registry, system file,
//! driver configuration or global process priority.
use std::collections::VecDeque;
use std::sync::{Arc,atomic::{AtomicBool,AtomicI64,Ordering},Mutex};
use std::thread::{self,JoinHandle};
use std::time::Duration;
#[derive(Debug,Clone,Copy)]
pub struct GuiGap{pub started:i64,pub ended:i64,pub duration_ms:f64}
struct Shared{
    running:AtomicBool,
    enabled:AtomicBool,
    latest:AtomicI64,
    gaps:Mutex<VecDeque<GuiGap>>,
}
#[derive(Clone)]
pub struct Heartbeat{shared:Arc<Shared>}
impl Heartbeat{
    pub fn beat(&self,tick:i64){self.shared.latest.store(tick,Ordering::Release);}
    pub fn last(&self)->i64{self.shared.latest.load(Ordering::Acquire)}
}
pub struct Watchdog{
    shared:Arc<Shared>,
    join:Option<JoinHandle<()>>,
}
impl Watchdog{
    pub fn new(freq:i64,clock:fn()->i64)->Self{
        assert!(freq>0);
        let shared=Arc::new(Shared{
            running:AtomicBool::new(true),
            enabled:AtomicBool::new(false),
            latest:AtomicI64::new(clock()),
            gaps:Mutex::new(VecDeque::with_capacity(32)),
        });
        let worker=shared.clone();
        let join=thread::spawn(move ||{
            let threshold=(freq as f64*0.2) as i64;
            let mut pending:Option<i64>=None;
            while worker.running.load(Ordering::Relaxed){
                thread::sleep(Duration::from_millis(10));
                if !worker.enabled.load(Ordering::Acquire){
                    pending=None;continue;
                }
                let now=clock();
                let beat=worker.latest.load(Ordering::Acquire);
                if pending.is_none() && now.saturating_sub(beat)>=threshold{
                    pending=Some(beat);
                }
                if let Some(start)=pending {
                    if beat>start {
                        let ms=(beat-start) as f64*1000.0/freq as f64;
                        if let Ok(mut queue)=worker.gaps.lock(){
                            if queue.len()==32{queue.pop_front();}
                            queue.push_back(GuiGap{
                                started:start,ended:beat,duration_ms:ms
                            });
                        }
                        pending=None;
                    }
                }
            }
        });
        Self{shared,join:Some(join)}
    }
    pub fn heartbeat(&self)->Heartbeat{Heartbeat{shared:self.shared.clone()}}
    pub fn last_beat(&self)->i64{self.shared.latest.load(Ordering::Acquire)}
    pub fn set_active(&self,enabled:bool,now:i64){
        self.shared.latest.store(now,Ordering::Release);
        self.shared.enabled.store(enabled,Ordering::Release);
    }
    pub fn beat(&self,tick:i64){
        self.shared.latest.store(tick,Ordering::Release);
    }
    pub fn drain(&self)->Vec<GuiGap>{
        self.shared.gaps.lock().map(|mut g|g.drain(..).collect())
            .unwrap_or_default()
    }
}
impl Drop for Watchdog{
    fn drop(&mut self){
        self.shared.running.store(false,Ordering::Release);
        if let Some(join)=self.join.take(){let _=join.join();}
    }
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn no_automatic_configuration_or_file_operations(){
        fn clock()->i64{1_000_000}
        let w=Watchdog::new(1_000_000,clock);
        w.set_active(false,clock());
        assert!(w.drain().is_empty());
    }
}
