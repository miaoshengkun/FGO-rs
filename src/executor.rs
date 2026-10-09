use crate::{
    domain::{Command, VerificationStatus},
    verification::{ActionVerificationContext, VerificationOrigin},
    vision::Point,
};
use anyhow::{Result, bail};

/// Shared, auditable execution surface used by battle and farming drivers.
pub trait Executor {
    fn tap(&mut self, point: Point) -> Result<()>;
    fn swipe(&mut self, _from: Point, _to: Point, _ms: u64) -> Result<()> {
        bail!("swipe is not supported by this executor")
    }
    fn sleep(&mut self, ms: u64) -> Result<()>;
    fn elapsed_ms(&self) -> u64;
    fn current_frame_id(&self) -> u64 {
        0
    }
    fn verification_origin(&self) -> VerificationOrigin {
        VerificationOrigin::default()
    }
    fn log(&mut self, message: &str) -> Result<()>;
    fn decision(&mut self, message: &str) -> Result<()> {
        self.log(message)
    }
    fn verification(
        &mut self,
        _context: &ActionVerificationContext,
        _status: VerificationStatus,
    ) -> Result<()> {
        Ok(())
    }
    fn command(&mut self, _command: Command) -> Result<()> {
        Ok(())
    }
}
