//! Worker output — the result a worker returns as **Structured output**: a
//! schema-validated object, one shape per stage kind. Shared by Runners (which
//! hands the schema to the CLI and deserialises the reply) and Runtime (which
//! acts on the result). Every field is a plain string or an enum, because the
//! CLI asks the model to repair a rejected object only once.

use crate::Verdict;
use serde::{Deserialize, Serialize};

/// What a generator (source) stage returns: the new candidates it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GeneratorOutput {
    /// One entry per NEW candidate. Empty when there is nothing new.
    pub items: Vec<GeneratedItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GeneratedItem {
    /// A stable, unique key for the candidate.
    pub key: String,
    /// A short (80 characters or fewer) plain-language summary.
    pub description: String,
    /// Path of the file written for this candidate, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

/// What a producer or implementer stage returns. There is no key: the item keeps
/// its parent's key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProducerOutput {
    /// Path of the file you wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    /// A short (80 characters or fewer) plain-language summary of the output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What a reviewer stage returns: a verdict, always with a reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReviewerOutput {
    /// approve, revise (send it back for another pass) or reject.
    pub verdict: Verdict,
    /// Why. On revise this is what the producer acts on.
    pub reason: String,
    /// Path of a file you wrote (for example a critique), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

/// Which output shape a stage returns: the source is a Generator, a reviewer
/// team a Reviewer, every other team a Producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputKind {
    Generator,
    Producer,
    Reviewer,
}

/// A worker's validated result, tagged by its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WorkerResult {
    Generator(GeneratorOutput),
    Producer(ProducerOutput),
    Reviewer(ReviewerOutput),
}

impl WorkerResult {
    pub fn kind(&self) -> OutputKind {
        match self {
            WorkerResult::Generator(_) => OutputKind::Generator,
            WorkerResult::Producer(_) => OutputKind::Producer,
            WorkerResult::Reviewer(_) => OutputKind::Reviewer,
        }
    }
}

impl OutputKind {
    /// The JSON Schema for this kind, generated from the Rust type. Inline (no
    /// `$ref`), no `$schema`, and optional fields are plain optional strings
    /// rather than nullable ones.
    pub fn schema(self) -> serde_json::Value {
        match self {
            OutputKind::Generator => schema_of::<GeneratorOutput>(),
            OutputKind::Producer => schema_of::<ProducerOutput>(),
            OutputKind::Reviewer => schema_of::<ReviewerOutput>(),
        }
    }

    /// Deserialise a structured-output object into this kind's result. An object
    /// of the wrong shape is an error.
    pub fn parse(self, v: &serde_json::Value) -> Result<WorkerResult, String> {
        let e = |e: serde_json::Error| e.to_string();
        Ok(match self {
            OutputKind::Generator => WorkerResult::Generator(GeneratorOutput::deserialize(v).map_err(e)?),
            OutputKind::Producer => WorkerResult::Producer(ProducerOutput::deserialize(v).map_err(e)?),
            OutputKind::Reviewer => WorkerResult::Reviewer(ReviewerOutput::deserialize(v).map_err(e)?),
        })
    }
}

fn schema_of<T: schemars::JsonSchema>() -> serde_json::Value {
    let settings = schemars::gen::SchemaSettings::draft07().with(|s| {
        s.option_add_null_type = false;
        s.option_nullable = false;
        s.inline_subschemas = true;
        s.meta_schema = None;
    });
    let root = settings.into_generator().into_root_schema_for::<T>();
    let mut v = serde_json::to_value(root).expect("a derived schema serialises");
    if let Some(o) = v.as_object_mut() {
        o.remove("title");
        o.remove("definitions");
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every `type` in the schema is string, array or object: the CLI nudges the
    /// model only once, so the shapes stay easy to satisfy.
    fn walk_types(v: &serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(t) = m.get("type") {
                    let t = t.as_str().unwrap_or_else(|| panic!("type must be one string: {t}"));
                    assert!(["string", "array", "object"].contains(&t), "type {t}");
                }
                m.values().for_each(walk_types);
            }
            serde_json::Value::Array(a) => a.iter().for_each(walk_types),
            _ => {}
        }
    }

    fn required(s: &serde_json::Value) -> Vec<String> {
        s["required"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn reviewer_schema_requires_verdict_and_reason_and_enumerates_verdicts() {
        let s = OutputKind::Reviewer.schema();
        assert_eq!(s["type"], "object");
        let req = required(&s);
        assert!(req.contains(&"verdict".to_string()) && req.contains(&"reason".to_string()));
        assert!(!req.contains(&"artifact".to_string()));
        assert_eq!(s["properties"]["verdict"]["enum"], json!(["approve", "revise", "reject"]));
        assert_eq!(s["properties"]["reason"]["type"], "string");
    }

    #[test]
    fn producer_schema_has_no_key_and_no_required_fields() {
        let s = OutputKind::Producer.schema();
        assert!(s["properties"].get("key").is_none());
        assert!(required(&s).is_empty());
        assert_eq!(s["properties"]["artifact"]["type"], "string");
        assert_eq!(s["properties"]["description"]["type"], "string");
    }

    #[test]
    fn generator_schema_inlines_the_item_shape() {
        let s = OutputKind::Generator.schema();
        assert_eq!(required(&s), vec!["items".to_string()]);
        let item = &s["properties"]["items"]["items"];
        assert_eq!(item["type"], "object");
        let req = required(item);
        assert!(req.contains(&"key".to_string()) && req.contains(&"description".to_string()));
        assert!(!req.contains(&"artifact".to_string()));
    }

    #[test]
    fn every_schema_uses_only_plain_types_and_no_refs_or_nulls() {
        for k in [OutputKind::Generator, OutputKind::Producer, OutputKind::Reviewer] {
            let s = k.schema();
            let text = serde_json::to_string(&s).unwrap();
            for banned in ["$ref", "$schema", "definitions", "null"] {
                assert!(!text.contains(banned), "{banned} in {text}");
            }
            assert!(!text.contains('\n'));
            walk_types(&s);
        }
    }

    /// The exact schemas handed to `--json-schema`. A change here changes what
    /// every worker is asked to return.
    #[test]
    fn schema_snapshots() {
        let snap = |k: OutputKind| serde_json::to_string(&k.schema()).unwrap();
        assert_eq!(
            snap(OutputKind::Generator),
            r#"{"description":"What a generator (source) stage returns: the new candidates it found.","properties":{"items":{"description":"One entry per NEW candidate. Empty when there is nothing new.","items":{"properties":{"artifact":{"description":"Path of the file written for this candidate, if any.","type":"string"},"description":{"description":"A short (80 characters or fewer) plain-language summary.","type":"string"},"key":{"description":"A stable, unique key for the candidate.","type":"string"}},"required":["description","key"],"type":"object"},"type":"array"}},"required":["items"],"type":"object"}"#
        );
        assert_eq!(
            snap(OutputKind::Producer),
            r#"{"description":"What a producer or implementer stage returns. There is no key: the item keeps its parent's key.","properties":{"artifact":{"description":"Path of the file you wrote.","type":"string"},"description":{"description":"A short (80 characters or fewer) plain-language summary of the output.","type":"string"}},"type":"object"}"#
        );
        assert_eq!(
            snap(OutputKind::Reviewer),
            r#"{"description":"What a reviewer stage returns: a verdict, always with a reason.","properties":{"artifact":{"description":"Path of a file you wrote (for example a critique), if any.","type":"string"},"reason":{"description":"Why. On revise this is what the producer acts on.","type":"string"},"verdict":{"description":"approve, revise (send it back for another pass) or reject.","enum":["approve","revise","reject"],"type":"string"}},"required":["reason","verdict"],"type":"object"}"#
        );
    }

    #[test]
    fn parse_maps_each_kind() {
        assert_eq!(
            OutputKind::Reviewer.parse(&json!({"verdict":"revise","reason":"thin"})).unwrap(),
            WorkerResult::Reviewer(ReviewerOutput { verdict: Verdict::Revise, reason: "thin".into(), artifact: None })
        );
        assert_eq!(
            OutputKind::Producer.parse(&json!({"artifact":"a.md"})).unwrap(),
            WorkerResult::Producer(ProducerOutput { artifact: Some("a.md".into()), description: None })
        );
        assert_eq!(
            OutputKind::Generator.parse(&json!({"items":[{"key":"a","description":"d"}]})).unwrap(),
            WorkerResult::Generator(GeneratorOutput {
                items: vec![GeneratedItem { key: "a".into(), description: "d".into(), artifact: None }]
            })
        );
        assert_eq!(OutputKind::Producer.parse(&json!({})).unwrap().kind(), OutputKind::Producer);
    }

    #[test]
    fn an_empty_items_list_is_a_valid_generator_result() {
        let r = OutputKind::Generator.parse(&json!({"items":[]})).unwrap();
        assert!(matches!(r, WorkerResult::Generator(g) if g.items.is_empty()));
    }

    #[test]
    fn parse_rejects_a_wrong_shape() {
        assert!(OutputKind::Reviewer.parse(&json!({"verdict":"maybe","reason":"x"})).is_err());
        assert!(OutputKind::Reviewer.parse(&json!({"verdict":"approve"})).is_err());
        assert!(OutputKind::Generator.parse(&json!({"items":[{"key":"a"}]})).is_err());
        assert!(OutputKind::Generator.parse(&json!({"keys":["a"]})).is_err());
        assert!(OutputKind::Producer.parse(&json!("just prose")).is_err());
    }
}
