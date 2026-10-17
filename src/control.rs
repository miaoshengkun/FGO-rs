use anyhow::{Result, bail};
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlError {
    Stopped,
    TimeLimitReached,
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stopped => "execution stopped",
            Self::TimeLimitReached => "execution time limit reached",
        })
    }
}

impl Error for ControlError {}

#[derive(Clone)]
enum Clock {
    Wall(Instant),
    Simulated(Arc<AtomicU64>),
}

#[derive(Clone)]
pub struct Control {
    stopped: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    pause_acknowledged: Arc<AtomicBool>,
    clock: Clock,
    limit: Duration,
}

impl Control {
    pub fn new(limit: Duration) -> Self {
        Self {
            stopped: Arc::default(),
            paused: Arc::default(),
            pause_acknowledged: Arc::default(),
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
            pause_acknowledged: Arc::default(),
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
    pub fn is_stop_requested(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
    pub fn is_pause_requested(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
    /// True only after a worker has entered a paused checkpoint.
    pub fn is_paused(&self) -> bool {
        self.pause_acknowledged.load(Ordering::SeqCst)
    }
    pub fn elapsed(&self) -> Duration {
        match &self.clock {
            Clock::Wall(started) => started.elapsed(),
            Clock::Simulated(elapsed_ms) => {
                Duration::from_millis(elapsed_ms.load(Ordering::SeqCst))
            }
        }
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
            return Err(ControlError::Stopped.into());
        }
        if self.elapsed() >= self.limit {
            return Err(ControlError::TimeLimitReached.into());
        }
        Ok(())
    }

    pub fn checkpoint(&self) -> Result<()> {
        loop {
            if let Err(error) = self.check_cancel() {
                self.pause_acknowledged.store(false, Ordering::SeqCst);
                return Err(error);
            }
            if !self.paused.load(Ordering::SeqCst) {
                if self.pause_acknowledged.load(Ordering::Relaxed) {
                    self.pause_acknowledged.store(false, Ordering::SeqCst);
                }
                return Ok(());
            }
            self.pause_acknowledged.store(true, Ordering::SeqCst);
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

    fn wait_until_paused(control: &Control) {
        let started = Instant::now();
        while !control.is_paused() {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "worker did not acknowledge pause"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn pause_is_acknowledged_by_worker_and_cleared_after_resume() {
        let control = Control::new(Duration::from_secs(5));
        control.pause();
        assert!(control.is_pause_requested());
        assert!(!control.is_paused());
        let worker_control = control.clone();
        let worker = thread::spawn(move || worker_control.checkpoint());
        wait_until_paused(&control);
        assert!(!worker.is_finished());
        control.resume();
        worker.join().unwrap().unwrap();
        assert!(!control.is_paused());
    }

    #[test]
    fn stop_while_paused_finishes_without_a_resume() {
        let control = Control::new(Duration::from_secs(5));
        control.pause();
        let worker_control = control.clone();
        let worker = thread::spawn(move || worker_control.checkpoint());
        wait_until_paused(&control);
        control.stop();
        assert!(
            worker
                .join()
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("execution stopped")
        );
        assert!(!control.is_paused());
    }

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
