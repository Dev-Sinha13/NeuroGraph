use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::errors::SchemaError;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(String);

impl NodeId {
    pub fn new(language: &str, fully_qualified_name: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(format!("{language}::{fully_qualified_name}"));
        Self(hex::encode(hasher.finalize()))
    }

    pub fn from_hex(value: impl Into<String>) -> Result<Self, SchemaError> {
        let value = value.into();
        if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
            return Err(SchemaError::InvalidNodeId(value));
        }
        Ok(Self(value.to_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphVersion(u64);

impl GraphVersion {
    pub fn new(value: u64) -> Result<Self, SchemaError> {
        if value == 0 {
            return Err(SchemaError::InvalidGraphVersion);
        }
        Ok(Self(value))
    }

    pub fn initial() -> Self {
        Self(1)
    }

    pub fn increment(self) -> Self {
        Self(self.0 + 1)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl Default for GraphVersion {
    fn default() -> Self {
        Self::initial()
    }
}

impl From<u64> for GraphVersion {
    fn from(value: u64) -> Self {
        Self::new(value).expect("GraphVersion cannot be 0")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    Function,
    Method { parent_fqn: String },
    Class,
    Struct,
    Interface,
    Trait,
    Module,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DeprecationReason {
    RenamedTo { new_fqn: String },
    Unresolved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeprecatedMetadata {
    pub expires_in_syncs: u8,
    pub successor_id: Option<NodeId>,
    pub reason: DeprecationReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeStatus {
    Active,
    Deprecated(DeprecatedMetadata),
}

impl NodeStatus {
    pub fn is_deprecated(&self) -> bool {
        matches!(self, Self::Deprecated(_))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypedParam {
    pub name: String,
    pub type_annotation: String,
    pub has_default: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PartialParam {
    pub name: String,
    pub type_annotation: Option<String>,
    pub has_default: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Signature {
    Typed {
        params: Vec<TypedParam>,
        return_type: String,
        is_async: bool,
        is_generic: bool,
    },
    PartiallyTyped {
        params: Vec<PartialParam>,
        return_type: Option<String>,
        is_async: bool,
    },
    Untyped {
        arity: Option<u8>,
    },
}

impl Signature {
    pub fn is_structurally_comparable(&self) -> bool {
        matches!(self, Self::Typed { .. } | Self::PartiallyTyped { .. })
    }

    pub fn arity(&self) -> Option<usize> {
        match self {
            Self::Typed { params, .. } => Some(params.len()),
            Self::PartiallyTyped { params, .. } => Some(params.len()),
            Self::Untyped { arity } => arity.map(usize::from),
        }
    }

    pub fn param_names(&self) -> Option<Vec<&str>> {
        match self {
            Self::Typed { params, .. } => {
                Some(params.iter().map(|param| param.name.as_str()).collect())
            }
            Self::PartiallyTyped { params, .. } => {
                Some(params.iter().map(|param| param.name.as_str()).collect())
            }
            Self::Untyped { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub language: String,
    pub name: String,
    pub fqn: String,
    pub kind: NodeKind,
    pub file_path: String,
    pub location: SourceLocation,
    pub status: NodeStatus,
    pub signature: Signature,
    pub body_hash: String,
    pub introduced_at_version: GraphVersion,
}

impl Node {
    pub fn validate(self) -> Result<Self, SchemaError> {
        let expected = NodeId::new(&self.language, &self.fqn);
        if self.id != expected {
            return Err(SchemaError::NodeIdMismatch {
                provided: self.id,
                expected,
            });
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum EdgeKind {
    Calls,
    Imports,
    Inherits,
    Implements,
    Instantiates,
    RuntimeVerified,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ResolutionMethod {
    Static,
    TypeInferred,
    Heuristic,
    Runtime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub source: NodeId,
    pub target: NodeId,
    pub kind: EdgeKind,
    confidence: f32,
    pub resolution: ResolutionMethod,
    pub introduced_at_version: GraphVersion,
}

impl Edge {
    pub fn new(
        source: NodeId,
        target: NodeId,
        kind: EdgeKind,
        confidence: f32,
        resolution: ResolutionMethod,
        introduced_at_version: GraphVersion,
    ) -> Result<Self, SchemaError> {
        if !(0.0..=1.0).contains(&confidence) {
            return Err(SchemaError::InvalidConfidence(confidence));
        }

        if matches!(
            resolution,
            ResolutionMethod::Static | ResolutionMethod::Runtime
        ) && (confidence - 1.0).abs() > f32::EPSILON
        {
            return Err(SchemaError::ConfidenceMismatch {
                resolution,
                confidence,
            });
        }

        Ok(Self {
            source,
            target,
            kind,
            confidence,
            resolution,
            introduced_at_version,
        })
    }

    pub fn confidence(&self) -> f32 {
        self.confidence
    }

    pub fn requires_escalation(&self, escalation_confidence_threshold: f32) -> bool {
        self.confidence < escalation_confidence_threshold
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeDraft {
    pub source: NodeId,
    pub target: NodeId,
    pub kind: EdgeKind,
    pub confidence: f32,
    pub resolution: ResolutionMethod,
    pub introduced_at_version: GraphVersion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceRange {
    pub min: f32,
    pub max: f32,
}

impl ConfidenceRange {
    pub fn new(min: f32, max: f32) -> Result<Self, SchemaError> {
        if min > max {
            return Err(SchemaError::InvalidConfidenceRange { min, max });
        }
        Ok(Self { min, max })
    }

    pub fn contains(&self, value: f32) -> bool {
        self.min <= value && value <= self.max
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceConfig {
    pub type_inferred_range: ConfidenceRange,
    pub heuristic_range: ConfidenceRange,
    pub is_calibrated: bool,
    pub calibration_sample_size: u32,
    pub calibrated_at: Option<String>,
}

impl ConfidenceConfig {
    pub fn startup_warning(&self) -> Option<String> {
        if self.is_calibrated {
            None
        } else {
            Some(
                "ConfidenceConfig is still using default feasibility-spike ranges. Run calibration before relying on escalation routing.".to_string(),
            )
        }
    }
}

impl Default for ConfidenceConfig {
    fn default() -> Self {
        Self {
            type_inferred_range: ConfidenceRange::new(0.7, 0.9)
                .expect("default type-inferred range should be valid"),
            heuristic_range: ConfidenceRange::new(0.3, 0.6)
                .expect("default heuristic range should be valid"),
            is_calibrated: false,
            calibration_sample_size: 0,
            calibrated_at: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphSnapshot {
    pub baseline_version: GraphVersion,
    pub current_baseline_version: GraphVersion,
    pub pr_identifier: String,
}

impl GraphSnapshot {
    pub fn is_stale(&self) -> bool {
        self.current_baseline_version > self.baseline_version
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeSummary {
    pub id: NodeId,
    pub fqn: String,
    pub kind: NodeKind,
    pub confidence: f32,
    pub resolution: ResolutionMethod,
    pub source_available: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubgraphSummary {
    pub queried_node_id: NodeId,
    pub queried_node_fqn: String,
    pub total_callers: usize,
    pub total_callees: usize,
    pub high_confidence_callers: Vec<NodeSummary>,
    pub low_confidence_callers: Vec<NodeSummary>,
    pub high_confidence_callees: Vec<NodeSummary>,
    pub low_confidence_callees: Vec<NodeSummary>,
    pub truncated: bool,
    pub omitted_count: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_round_trip_and_validation_work() {
        let node_id = NodeId::new("python", "pkg.parse_config");
        let parsed = NodeId::from_hex(node_id.as_str().to_string()).unwrap();
        assert_eq!(node_id, parsed);
        assert!(NodeId::from_hex("not-a-hash").is_err());
    }

    #[test]
    fn graph_version_starts_at_one_and_increments() {
        assert!(GraphVersion::new(0).is_err());
        assert_eq!(GraphVersion::initial().value(), 1);
        assert_eq!(GraphVersion::initial().increment().value(), 2);
    }

    #[test]
    fn signature_helpers_reflect_variant_shape() {
        let typed = Signature::Typed {
            params: vec![TypedParam {
                name: "value".to_string(),
                type_annotation: "str".to_string(),
                has_default: false,
            }],
            return_type: "str".to_string(),
            is_async: false,
            is_generic: false,
        };
        let untyped = Signature::Untyped { arity: None };

        assert!(typed.is_structurally_comparable());
        assert_eq!(typed.arity(), Some(1));
        assert_eq!(typed.param_names(), Some(vec!["value"]));
        assert!(!untyped.is_structurally_comparable());
        assert_eq!(untyped.arity(), None);
    }

    #[test]
    fn edge_constructor_enforces_fixed_confidence_rules() {
        let source = NodeId::new("python", "pkg.a");
        let target = NodeId::new("python", "pkg.b");
        assert!(
            Edge::new(
                source.clone(),
                target.clone(),
                EdgeKind::Calls,
                1.1,
                ResolutionMethod::Heuristic,
                GraphVersion::initial(),
            )
            .is_err()
        );
        assert!(
            Edge::new(
                source,
                target,
                EdgeKind::Calls,
                0.9,
                ResolutionMethod::Static,
                GraphVersion::initial(),
            )
            .is_err()
        );
    }

    #[test]
    fn confidence_config_warning_and_snapshot_staleness_are_exposed() {
        let config = ConfidenceConfig::default();
        assert!(config.startup_warning().is_some());

        let snapshot = GraphSnapshot {
            baseline_version: GraphVersion::initial(),
            current_baseline_version: GraphVersion::initial().increment(),
            pr_identifier: "42".to_string(),
        };
        assert!(snapshot.is_stale());
    }
}
