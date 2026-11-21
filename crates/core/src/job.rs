//! Progress reporting and cancellation for long operations.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::error::{Error, Result};

const PROGRESS_SCALE: f32 = 1000.0;

#[derive(Debug, Default)]
struct Inner {
    cancelled: AtomicBool,
    progress: AtomicU32,
}

/// Shared handle: the tool reports progress and polls for cancellation,
/// the UI/CLI keeps a clone to cancel and to read progress.
#[derive(Debug, Clone, Default)]
pub struct Job {
    inner: Arc<Inner>,
}

impl Job {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Relaxed)
    }

    /// Returns `Err(Error::Cancelled)` if the job was cancelled; call it in loops.
    pub fn check_cancelled(&self) -> Result<()> {
        if self.is_cancelled() { Err(Error::Cancelled) } else { Ok(()) }
    }

    /// Sets progress as a fraction in `0.0..=1.0` (clamped, NaN ignored).
    pub fn set_progress(&self, fraction: f32) {
        if fraction.is_nan() {
            return;
        }
        let permille = (fraction.clamp(0.0, 1.0) * PROGRESS_SCALE).round() as u32;
        self.inner.progress.store(permille, Ordering::Relaxed);
    }

    pub fn progress(&self) -> f32 {
        self.inner.progress.load(Ordering::Relaxed) as f32 / PROGRESS_SCALE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_idle() {
        let job = Job::new();
        assert!(!job.is_cancelled());
        assert_eq!(job.progress(), 0.0);
        assert!(job.check_cancelled().is_ok());
    }

    #[test]
    fn clones_share_state() {
        let job = Job::new();
        let other = job.clone();
        other.cancel();
        assert!(job.is_cancelled());
        assert!(matches!(job.check_cancelled(), Err(Error::Cancelled)));
        job.set_progress(0.5);
        assert_eq!(other.progress(), 0.5);
    }

    #[test]
    fn progress_is_clamped_and_ignores_nan() {
        let job = Job::new();
        job.set_progress(2.0);
        assert_eq!(job.progress(), 1.0);
        job.set_progress(-1.0);
        assert_eq!(job.progress(), 0.0);
        job.set_progress(0.25);
        job.set_progress(f32::NAN);
        assert_eq!(job.progress(), 0.25);
    }
}
