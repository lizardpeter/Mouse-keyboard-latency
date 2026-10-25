//! Passive background-thread wakeup probe.
//! Observes its *own* timing slippage, independently of either Win32 message
//! loop. A late wakeup is not proof of DPC activity, a specific CPU hog, or
//! an input/USB fault. No scheduler settings are changed.
use std::sync::{Arc,atomic::{AtomicBool,Ordering},mpsc::{self,Receiver,SyncSender,TrySendError}};
use std::thread::{self,JoinHandle};
use std::time::Duration;

#[derive(Clone,Copy,Debug)]
pub struct WakeGap{pub tick:i64,pub elapsed_ms:f64}
pub struct SchedulerProbe{
    running:Arc<AtomicBool>,
    active:Arc<AtomicBool>,
    rx:Receiver<WakeGap>,
    worker:Option<JoinHandle<()>>,
}
impl SchedulerProbe{
    pub fn new(freq:i64,clock:fn()->i64)->Self{
        let (tx,rx):(SyncSender<WakeGap>,Receiver<WakeGap>)=mpsc::sync_channel(128);
        let running=Arc::new(AtomicBool::new(true));
        let active=Arc::new(AtomicBool::new(false));
        let r=running.clone();let a=active.clone();
        let worker=thread::spawn(move ||{
            let mut last=clock();
            while r.load(Ordering::Acquire){
                thread::sleep(Duration::from_millis(10));
                let tick=clock();
                let enabled=a.load(Ordering::Acquire);
                let gap=tick.saturating_sub(last) as f64*1000.0/freq as f64;
                last=tick;
                if enabled && gap>=100.0{
                    match tx.try_send(WakeGap{tick,elapsed_ms:gap}){
                        Ok(())|Err(TrySendError::Full(_))=>{},
                        Err(TrySendError::Disconnected(_))=>break,
                    }
                }
            }
        });
        Self{running,active,rx,worker:Some(worker)}
    }
    pub fn set_active(&self,value:bool){self.active.store(value,Ordering::Release);}
    pub fn drain(&self)->Vec<WakeGap>{
        self.rx.try_iter().take(128).collect()
    }
}
impl Drop for SchedulerProbe{
    fn drop(&mut self){
        self.running.store(false,Ordering::Release);
        if let Some(t)=self.worker.take(){let _=t.join();}
    }
}
