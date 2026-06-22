//! Rolling-window math — pure functions over already-summed token counts. No DB,
//! no clock: `now` and the raw sums are passed in (D5). The stores supply the
//! sums; this turns them into the meter's numbers + the threshold band.

use serde::{Deserialize, Serialize};

/// The meter's colour state (DOMAIN.md + topbar-usage mockup).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdBand {
    Safe,   // < 60%
    Warn,   // 60–85%
    Hot,    // >= 85%
    Braked, // brake is on (overrides the others for display)
}

/// Band from a fraction in [0, ∞). `braked` forces Braked.
pub fn band(pct: f64, braked: bool) -> ThresholdBand {
    if braked {
        return ThresholdBand::Braked;
    }
    if pct >= 0.85 {
        ThresholdBand::Hot
    } else if pct >= 0.60 {
        ThresholdBand::Warn
    } else {
        ThresholdBand::Safe
    }
}

/// window_pct = total / budget, clamped to [0, ∞) (can exceed 1.0). budget 0 →
/// 0.0 (avoid div-by-zero; the config CHECK keeps budget > 0 in practice).
pub fn window_pct(total: u64, budget: u64) -> f64 {
    if budget == 0 {
        0.0
    } else {
        total as f64 / budget as f64
    }
}

/// Burn rate in tokens/minute from tokens seen in the last `window_secs` seconds.
pub fn burn_per_min(recent_tokens: u64, window_secs: i64) -> f64 {
    if window_secs <= 0 {
        0.0
    } else {
        recent_tokens as f64 / (window_secs as f64 / 60.0)
    }
}

/// Seconds until the window has room again = (oldest_in_window + window_secs) -
/// now. None when nothing is in the window. Clamped at 0 (never negative).
pub fn reset_in_secs(oldest_in_window: Option<i64>, window_secs: i64, now: i64) -> Option<i64> {
    oldest_in_window.map(|oldest| ((oldest + window_secs) - now).max(0))
}

/// Estimated unix-seconds at which the window crosses `brake_on_pct` of budget,
/// given the current total + burn. None when burn is 0 or already past the
/// threshold (D7). Best-effort tooltip hint only.
pub fn est_brake_at(
    total: u64,
    budget: u64,
    brake_on_pct: f64,
    burn_per_min: f64,
    now: i64,
) -> Option<i64> {
    if burn_per_min <= 0.0 || budget == 0 {
        return None;
    }
    let threshold = budget as f64 * brake_on_pct;
    let remaining = threshold - total as f64;
    if remaining <= 0.0 {
        return None;
    }
    let secs = (remaining / (burn_per_min / 60.0)).round() as i64;
    Some(now + secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_match_the_spec_thresholds() {
        assert_eq!(band(0.0, false), ThresholdBand::Safe);
        assert_eq!(band(0.59, false), ThresholdBand::Safe);
        assert_eq!(band(0.60, false), ThresholdBand::Warn);
        assert_eq!(band(0.84, false), ThresholdBand::Warn);
        assert_eq!(band(0.85, false), ThresholdBand::Hot);
        assert_eq!(band(1.5, false), ThresholdBand::Hot);
        assert_eq!(band(0.10, true), ThresholdBand::Braked); // braked overrides
    }

    #[test]
    fn pct_divides_by_budget_and_guards_zero() {
        assert!((window_pct(1_300_000, 2_600_000) - 0.5).abs() < 1e-9);
        assert_eq!(window_pct(100, 0), 0.0);
    }

    #[test]
    fn burn_per_min_from_one_minute_window() {
        // 18_000 tokens in 60s -> 18_000/min
        assert!((burn_per_min(18_000, 60) - 18_000.0).abs() < 1e-9);
        assert_eq!(burn_per_min(10, 0), 0.0);
    }

    #[test]
    fn reset_counts_down_from_oldest_plus_window() {
        // oldest at t=1000, 5h window (18000s), now=2000 -> 17000 left
        assert_eq!(reset_in_secs(Some(1000), 18000, 2000), Some(17000));
        // already elapsed -> clamped to 0
        assert_eq!(reset_in_secs(Some(1000), 18000, 100000), Some(0));
        assert_eq!(reset_in_secs(None, 18000, 2000), None);
    }

    #[test]
    fn est_brake_at_projects_forward_or_none() {
        // total 1.0M, budget 2.6M, threshold .95 -> 2.47M; remaining 1.47M;
        // burn 60k/min -> 1k/s -> ~1470s
        let at = est_brake_at(1_000_000, 2_600_000, 0.95, 60_000.0, 1000).unwrap();
        assert_eq!(at, 1000 + 1470);
        // no burn -> None
        assert!(est_brake_at(1_000_000, 2_600_000, 0.95, 0.0, 1000).is_none());
        // already past threshold -> None
        assert!(est_brake_at(2_600_000, 2_600_000, 0.95, 10.0, 1000).is_none());
    }
}
