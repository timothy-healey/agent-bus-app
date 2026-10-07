//! The auto-meter brake DECISION (D8/D9). Pure: given the current window pct and
//! whether the brake is already on for the auto-meter reason, decide whether to
//! set it on, release it, or do nothing. This crate never touches Runtime's
//! Brake — the composition root applies the decision. Hysteresis: trip at
//! >= brake_on_pct, release at < brake_off_pct (spec: 0.95 / 0.85).

use agent_bus_core::{LimitKind, UtilizationReading};

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

/// The highest Utilization (as a fraction) across the Limits the brake watches:
/// the session and weekly Limits. A Limit whose window has reset counts as 0.
/// None when the reading has neither Limit.
pub fn watched_fraction(reading: &UtilizationReading, now: i64) -> Option<f64> {
    reading
        .limits
        .iter()
        .filter(|l| matches!(l.kind, LimitKind::Session | LimitKind::Weekly))
        .map(|l| if now >= l.resets_at { 0.0 } else { l.utilization_pct / 100.0 })
        .fold(None, |acc: Option<f64>, x| Some(acc.map_or(x, |a| a.max(x))))
}

/// The auto-brake decision from a real reading. Without a reading, or when the
/// last poll failed, the brake is left as it is.
pub fn decide_utilization(
    reading: Option<&UtilizationReading>,
    available: bool,
    now: i64,
    auto_on: bool,
    brake_on_pct: f64,
    brake_off_pct: f64,
) -> BrakeDecision {
    if !available {
        return BrakeDecision::NoChange;
    }
    match reading.and_then(|r| watched_fraction(r, now)) {
        Some(f) => decide(f, auto_on, brake_on_pct, brake_off_pct),
        None => BrakeDecision::NoChange,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{LimitKind, LimitReading, UtilizationReading};

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

    fn r(session: f64, weekly: f64, fable: f64, resets_at: i64) -> UtilizationReading {
        UtilizationReading { observed_at: 0, limits: vec![
            LimitReading { kind: LimitKind::Session, utilization_pct: session, resets_at },
            LimitReading { kind: LimitKind::Weekly, utilization_pct: weekly, resets_at: 10_000 },
            LimitReading { kind: LimitKind::ModelWeekly { model: "Fable".into() }, utilization_pct: fable, resets_at: 10_000 },
        ]}
    }

    #[test]
    fn session_at_95_sets_the_brake() {
        assert_eq!(decide_utilization(Some(&r(95.0, 2.0, 0.0, 5_000)), true, 100, false, 0.95, 0.85), BrakeDecision::SetOn(AUTO_METER_REASON.into()));
    }

    #[test]
    fn weekly_at_95_sets_the_brake() {
        assert_eq!(decide_utilization(Some(&r(10.0, 96.0, 0.0, 5_000)), true, 100, false, 0.95, 0.85), BrakeDecision::SetOn(AUTO_METER_REASON.into()));
    }

    #[test]
    fn model_scoped_limits_never_brake() {
        assert_eq!(decide_utilization(Some(&r(10.0, 2.0, 100.0, 5_000)), true, 100, false, 0.95, 0.85), BrakeDecision::NoChange);
    }

    #[test]
    fn release_needs_both_watched_limits_below_85() {
        assert_eq!(decide_utilization(Some(&r(80.0, 90.0, 0.0, 5_000)), true, 100, true, 0.95, 0.85), BrakeDecision::NoChange);
        assert_eq!(decide_utilization(Some(&r(80.0, 84.0, 0.0, 5_000)), true, 100, true, 0.95, 0.85), BrakeDecision::Release);
    }

    #[test]
    fn an_expired_session_counts_as_zero_and_releases() {
        // session read 99% but its window reset at t=50; now is t=100.
        assert_eq!(decide_utilization(Some(&r(99.0, 2.0, 0.0, 50)), true, 100, true, 0.95, 0.85), BrakeDecision::Release);
    }

    #[test]
    fn no_reading_or_unavailable_changes_nothing() {
        assert_eq!(decide_utilization(None, true, 100, false, 0.95, 0.85), BrakeDecision::NoChange);
        assert_eq!(decide_utilization(Some(&r(99.0, 2.0, 0.0, 5_000)), false, 100, false, 0.95, 0.85), BrakeDecision::NoChange);
        assert_eq!(decide_utilization(Some(&r(10.0, 2.0, 0.0, 5_000)), false, 100, true, 0.95, 0.85), BrakeDecision::NoChange);
    }
}
