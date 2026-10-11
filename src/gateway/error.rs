//! Structured error models and Actionable Remediation Envelopes (INV-8, INV-9, D-96, D-97).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Actionable Remediation Envelope returned when relational edge target validation fails (INV-8, INV-9, D-96, D-97).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvalidEdgeTargetEnvelope {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub message: String,
    pub invalid_field: String,
    pub invalid_value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failing_entity: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation_action: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_repairs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub valid_target_candidates: Vec<Value>,
}

impl InvalidEdgeTargetEnvelope {
    /// Creates a new `InvalidEdgeTargetEnvelope` with standardized error taxonomy and suggested repairs.
    #[must_use]
    pub fn new(
        invalid_field: impl Into<String>,
        invalid_value: impl Into<String>,
        message: impl Into<String>,
        suggested_repairs: Vec<Value>,
        valid_target_candidates: Vec<Value>,
    ) -> Self {
        let inv_field = invalid_field.into();
        let inv_val = invalid_value.into();
        Self {
            code: "ERR_INVALID_EDGE_TARGET".to_string(),
            error_code: Some("ERR_INVALID_EDGE_TARGET".to_string()),
            message: message.into(),
            invalid_field: inv_field.clone(),
            invalid_value: inv_val.clone(),
            failing_entity: Some(serde_json::json!({
                "attempted_target_id": inv_val,
                "field": inv_field,
            })),
            remediation_action: Some("SELECT_ACTIVE_TARGET".to_string()),
            suggested_repairs,
            valid_target_candidates,
        }
    }
}
