//! The auto-meter brake DECISION (D8/D9). Pure: given the current window pct and
//! whether the brake is already on for the auto-meter reason, decide whether to
//! set it on, release it, or do nothing. This crate never touches Runtime's
//! Brake — the composition root applies the decision. Hysteresis: trip at
//! >= brake_on_pct, release at < brake_off_pct (spec: 0.95 / 0.85).

/// What the composition root should do to the Runtime brake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrakeDecision {
    /// Set the brake on with this reason (the spec's `auto-meter`).
    SetOn(String),
    /// Release the brake (window dropped below the off threshold).
    Release,
    /// Leave the brake as-is.
    NoChange,
}

pub const AUTO_METER_REASON: &str = "auto-meter";

/// Decide. `auto_on` = the brake is currently on *because of the auto-meter*
/// (the root tracks the reason, so a manual/reactive brake is not auto-released).
pub fn decide(pct: f64, auto_on: bool, brake_on_pct: f64, brake_off_pct: f64) -> BrakeDecision {
    if !auto_on && pct >= brake_on_pct {
        BrakeDecision::SetOn(AUTO_METER_REASON.to_string())
    } else if auto_on && pct < brake_off_pct {
        BrakeDecision::Release
    } else {
        BrakeDecision::NoChange
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trips_on_at_or_above_on_threshold() {
        assert_eq!(decide(0.95, false, 0.95, 0.85), BrakeDecision::SetOn("auto-meter".into()));
        assert_eq!(decide(0.99, false, 0.95, 0.85), BrakeDecision::SetOn("auto-meter".into()));
    }

    #[test]
    fn does_not_re_trip_when_already_auto_on() {
        assert_eq!(decide(0.96, true, 0.95, 0.85), BrakeDecision::NoChange);
    }

    #[test]
    fn releases_below_off_threshold_only_when_auto_on() {
        assert_eq!(decide(0.84, true, 0.95, 0.85), BrakeDecision::Release);
        // not auto-on -> never auto-releases (manual/reactive brake stays)
        assert_eq!(decide(0.10, false, 0.95, 0.85), BrakeDecision::NoChange);
    }

    #[test]
    fn hysteresis_band_holds_between_off_and_on() {
        // auto-on, sitting at 0.90 (between .85 and .95): stay on
        assert_eq!(decide(0.90, true, 0.95, 0.85), BrakeDecision::NoChange);
        // not on, sitting at 0.90: don't trip yet
        assert_eq!(decide(0.90, false, 0.95, 0.85), BrakeDecision::NoChange);
    }
}
