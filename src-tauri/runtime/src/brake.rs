//! The system-wide brake (spec: Brake). When on, the WorkerPool stops issuing
//! new claims; in-flight workers complete. Thread-safe; shared via Arc.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrakeState {
    pub on: bool,
    pub reason: Option<String>,
}

#[derive(Default)]
pub struct Brake {
    on: AtomicBool,
    reason: Mutex<Option<String>>,
}

impl Brake {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_on(&self, reason: impl Into<String>) {
        *self.reason.lock().unwrap() = Some(reason.into());
        self.on.store(true, Ordering::SeqCst);
    }

    pub fn set_off(&self) {
        self.on.store(false, Ordering::SeqCst);
        *self.reason.lock().unwrap() = None;
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }

    pub fn state(&self) -> BrakeState {
        BrakeState { on: self.is_on(), reason: self.reason.lock().unwrap().clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_off_with_no_reason() {
        let b = Brake::new();
        assert!(!b.is_on());
        assert_eq!(b.state(), BrakeState { on: false, reason: None });
    }

    #[test]
    fn set_on_records_reason_then_off_clears_it() {
        let b = Brake::new();
        b.set_on("rate-limit");
        assert!(b.is_on());
        assert_eq!(b.state().reason.as_deref(), Some("rate-limit"));
        b.set_off();
        assert!(!b.is_on());
        assert_eq!(b.state().reason, None);
    }
}
