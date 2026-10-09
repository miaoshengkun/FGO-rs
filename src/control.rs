use anyhow::{Result, bail};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
enum Clock {
    Wall(Instant),
    Simulated(Arc<AtomicU64>),
}

#[derive(Clone)]
pub struct Control {
    stopped: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    clock: Clock,
    limit: Duration,
}

impl Control {
    pub fn new(limit: Duration) -> Self {
        Self {
            stopped: Arc::default(),
            paused: Arc::default(),
            clock: Clock::Wall(Instant::now()),
            limit,
        }
    }

    /// A deterministic clock for replay and transition tests. Calls to sleep
    /// advance logical time without blocking the current thread.
    pub fn simulated(limit: Duration) -> Self {
        Self {
            stopped: Arc::default(),
            paused: Arc::default(),
            clock: Clock::Simulated(Arc::default()),
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
        match &self.clock {
            Clock::Wall(started) => started.elapsed(),
            Clock::Simulated(elapsed_ms) => {
                Duration::from_millis(elapsed_ms.load(Ordering::SeqCst))
            }
        }
    }

    pub fn is_simulated(&self) -> bool {
        matches!(self.clock, Clock::Simulated(_))
    }

    /// Move simulated replay time forward to a recorded capture timestamp.
    /// Wall-clock controls reject this operation so production time cannot be
    /// changed accidentally.
    pub fn advance_to(&self, elapsed: Duration) -> Result<()> {
        let Clock::Simulated(current) = &self.clock else {
            bail!("cannot advance a wall clock")
        };
        let target = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let previous = current.load(Ordering::SeqCst);
        if target < previous {
            bail!("replay timestamp moved backwards from {previous} ms to {target} ms");
        }
        current.fetch_max(target, Ordering::SeqCst);
        self.check_cancel()
    }

    /// Wall-clock limits include paused time. Simulated limits advance with
    /// replay timestamps and logical sleeps. This never waits for resume.
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
        if let Clock::Simulated(elapsed_ms) = &self.clock {
            self.checkpoint()?;
            let delta = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
            let _ = elapsed_ms.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                Some(current.saturating_add(delta))
            });
            return self.checkpoint();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_sleep_and_capture_timestamps_advance_without_wall_time() {
        let control = Control::simulated(Duration::from_secs(10));
        let wall = Instant::now();
        control.sleep(Duration::from_secs(5)).unwrap();
        control.advance_to(Duration::from_millis(5900)).unwrap();
        assert_eq!(control.elapsed(), Duration::from_millis(5900));
        assert!(control.advance_to(Duration::from_millis(5800)).is_err());
        assert!(wall.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn simulated_time_still_enforces_the_execution_limit() {
        let control = Control::simulated(Duration::from_millis(100));
        let error = control.sleep(Duration::from_millis(100)).unwrap_err();
        assert!(error.to_string().contains("time limit"));
    }

    #[test]
    fn wall_clock_cannot_be_advanced_by_replay_data() {
        let control = Control::new(Duration::from_secs(1));
        assert!(control.advance_to(Duration::from_millis(500)).is_err());
    }
}
