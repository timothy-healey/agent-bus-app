//! The account's plan Limits as Claude reports them. Usage Telemetry consumes a
//! `UtilizationSource`; the Runners ACL implements it, so neither depends on the
//! other's crate.

use serde::{Deserialize, Serialize};

/// Which plan Limit a reading is for. Only `Session` and `Weekly` drive the brake.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LimitKind {
    Session,
    Weekly,
    ModelWeekly { model: String },
}

/// One Limit's Utilization (0–100, as reported) and its reset time (unix seconds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitReading {
    pub kind: LimitKind,
    pub utilization_pct: f64,
    pub resets_at: i64,
}

/// Every Limit reported at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UtilizationReading {
    pub observed_at: i64,
    pub limits: Vec<LimitReading>,
}

/// One fresh reading of the account's Limits, or why there isn't one.
pub trait UtilizationSource: Send + Sync {
    fn fetch(&self) -> Result<UtilizationReading, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_round_trips_through_json() {
        let r = UtilizationReading {
            observed_at: 100,
            limits: vec![
                LimitReading { kind: LimitKind::Session, utilization_pct: 41.0, resets_at: 200 },
                LimitReading { kind: LimitKind::Weekly, utilization_pct: 2.0, resets_at: 300 },
                LimitReading { kind: LimitKind::ModelWeekly { model: "Fable".into() }, utilization_pct: 0.0, resets_at: 400 },
            ],
        };
        let back: UtilizationReading = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn utilization_source_is_object_safe() {
        struct Fixed;
        impl UtilizationSource for Fixed {
            fn fetch(&self) -> Result<UtilizationReading, String> {
                Ok(UtilizationReading { observed_at: 1, limits: vec![] })
            }
        }
        let s: std::sync::Arc<dyn UtilizationSource> = std::sync::Arc::new(Fixed);
        assert_eq!(s.fetch().unwrap().observed_at, 1);
    }
}
