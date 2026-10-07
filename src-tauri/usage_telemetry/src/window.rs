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
}
