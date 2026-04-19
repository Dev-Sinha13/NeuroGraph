use serde_json::{Value, json};
use thiserror::Error;

use crate::schema::{NodeId, ResolutionMethod};

#[derive(Debug, Error, PartialEq)]
pub enum SchemaError {
    #[error("NodeId must be a 64-character SHA-256 hex string, received `{0}`")]
    InvalidNodeId(String),
    #[error("GraphVersion must start at 1 and never be 0")]
    InvalidGraphVersion,
    #[error("confidence {0} must be within [0.0, 1.0]")]
    InvalidConfidence(f32),
    #[error("resolution `{resolution:?}` requires confidence 1.0, received {confidence}")]
    ConfidenceMismatch {
        resolution: ResolutionMethod,
        confidence: f32,
    },
    #[error(
        "confidence {confidence} for `{resolution:?}` is outside the calibrated range [{min}, {max}]"
    )]
    ConfidenceOutOfCalibratedRange {
        resolution: ResolutionMethod,
        confidence: f32,
        min: f32,
        max: f32,
    },
    #[error("target node `{target}` is deprecated and cannot receive new active edges")]
    EdgeTargetsDeprecatedNode { target: NodeId },
    #[error("confidence range min {min} cannot exceed max {max}")]
    InvalidConfidenceRange { min: f32, max: f32 },
    #[error("node id `{provided}` does not match the expected value `{expected}`")]
    NodeIdMismatch { provided: NodeId, expected: NodeId },
    #[error("edge endpoint `{0}` does not exist")]
    MissingNode(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolCallError {
    UnknownNode {
        requested_id: String,
        suggestion: String,
    },
    NodeDeletedInPr {
        fqn: String,
        suggestion: String,
    },
    #[allow(dead_code)]
    QueryTimeout {
        timeout_ms: u64,
        suggestion: String,
    },
    InvalidSchema {
        detail: String,
    },
    SubgraphTooLarge {
        node_count: usize,
        cap: usize,
        suggestion: String,
    },
}

impl ToolCallError {
    pub fn invalid_schema(detail: String) -> Self {
        Self::InvalidSchema { detail }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownNode { .. } => "UNKNOWN_NODE",
            Self::NodeDeletedInPr { .. } => "NODE_DELETED_IN_PR",
            Self::QueryTimeout { .. } => "QUERY_TIMEOUT",
            Self::InvalidSchema { .. } => "INVALID_SCHEMA",
            Self::SubgraphTooLarge { .. } => "SUBGRAPH_TOO_LARGE",
        }
    }

    pub fn payload(&self) -> Value {
        match self {
            Self::UnknownNode {
                requested_id,
                suggestion,
            } => json!({
                "error": self.code(),
                "detail": format!("The requested node `{requested_id}` does not exist in the current graph."),
                "suggestion": suggestion,
            }),
            Self::NodeDeletedInPr { fqn, suggestion } => json!({
                "error": self.code(),
                "detail": format!("The node `{fqn}` exists in the baseline graph but was removed in the PR overlay."),
                "suggestion": suggestion,
            }),
            Self::QueryTimeout {
                timeout_ms,
                suggestion,
            } => json!({
                "error": self.code(),
                "detail": format!("The graph query exceeded the timeout threshold of {timeout_ms} ms."),
                "suggestion": suggestion,
            }),
            Self::InvalidSchema { detail } => json!({
                "error": self.code(),
                "detail": detail,
            }),
            Self::SubgraphTooLarge {
                node_count,
                cap,
                suggestion,
            } => json!({
                "error": self.code(),
                "detail": format!("The requested subgraph would return {node_count} nodes which exceeds the cap of {cap}."),
                "suggestion": suggestion,
            }),
        }
    }

    pub fn to_json(&self) -> String {
        self.payload().to_string()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::ToolCallError;

    #[test]
    fn tool_error_payload_contains_required_fields() {
        let payload = ToolCallError::UnknownNode {
            requested_id: "abc".to_string(),
            suggestion: "Use find_callers('bar') to search by name.".to_string(),
        }
        .payload();

        assert_eq!(payload["error"], Value::String("UNKNOWN_NODE".to_string()));
        assert!(payload["detail"].as_str().unwrap().contains("abc"));
        assert!(
            payload["suggestion"]
                .as_str()
                .unwrap()
                .contains("find_callers")
        );
    }
}
