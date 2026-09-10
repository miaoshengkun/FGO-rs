use anyhow::{Result, bail};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Control {
    stopped: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    started: Instant,
    limit: Duration,
}

impl Control {
    pub fn new(limit: Duration) -> Self {
        Self {
            stopped: Arc::default(),
            paused: Arc::default(),
            started: Instant::now(),
            limit,
        }
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Wall-clock limits include paused time. This never waits for resume.
    pub fn check_cancel(&self) -> Result<()> {
        if self.stopped.load(Ordering::SeqCst) {
            bail!("execution stopped");
        }
        if self.elapsed() >= self.limit {
            bail!("execution time limit reached");
        }
        Ok(())
    }

    pub fn checkpoint(&self) -> Result<()> {
        loop {
            self.check_cancel()?;
            if !self.paused.load(Ordering::SeqCst) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn sleep(&self, duration: Duration) -> Result<()> {
        let mut remaining = duration;
        while !remaining.is_zero() {
            self.checkpoint()?;
            let part = remaining.min(Duration::from_millis(20));
            thread::sleep(part);
            remaining = remaining.saturating_sub(part);
        }
        self.checkpoint()
    }
}
