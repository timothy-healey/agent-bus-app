use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Approve,
    Revise,
    Reject,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_serialises_as_lowercase_string() {
        assert_eq!(serde_json::to_string(&Verdict::Approve).unwrap(), "\"approve\"");
        assert_eq!(serde_json::to_string(&Verdict::Revise).unwrap(),  "\"revise\"");
        assert_eq!(serde_json::to_string(&Verdict::Reject).unwrap(),  "\"reject\"");
    }

    #[test]
    fn verdict_round_trips() {
        for v in [Verdict::Approve, Verdict::Revise, Verdict::Reject] {
            let s = serde_json::to_string(&v).unwrap();
            let back: Verdict = serde_json::from_str(&s).unwrap();
            assert_eq!(v, back);
        }
    }
}
