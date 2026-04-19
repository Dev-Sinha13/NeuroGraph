use crate::errors::SchemaError;
use crate::schema::{ConfidenceConfig, Edge, Node, ResolutionMethod};

#[derive(Clone, Debug)]
pub struct SchemaValidator {
    confidence_config: ConfidenceConfig,
}

impl SchemaValidator {
    pub fn new(confidence_config: ConfidenceConfig) -> Self {
        Self { confidence_config }
    }

    pub fn confidence_config(&self) -> &ConfidenceConfig {
        &self.confidence_config
    }

    pub fn set_confidence_config(&mut self, confidence_config: ConfidenceConfig) {
        self.confidence_config = confidence_config;
    }

    pub fn validate_edge_write(&self, edge: &Edge, target: &Node) -> Result<(), SchemaError> {
        match edge.resolution {
            ResolutionMethod::Static | ResolutionMethod::Runtime => {}
            ResolutionMethod::TypeInferred => {
                let range = &self.confidence_config.type_inferred_range;
                if !range.contains(edge.confidence()) {
                    return Err(SchemaError::ConfidenceOutOfCalibratedRange {
                        resolution: edge.resolution,
                        confidence: edge.confidence(),
                        min: range.min,
                        max: range.max,
                    });
                }
            }
            ResolutionMethod::Heuristic => {
                let range = &self.confidence_config.heuristic_range;
                if !range.contains(edge.confidence()) {
                    return Err(SchemaError::ConfidenceOutOfCalibratedRange {
                        resolution: edge.resolution,
                        confidence: edge.confidence(),
                        min: range.min,
                        max: range.max,
                    });
                }
            }
        }

        if target.status.is_deprecated() {
            return Err(SchemaError::EdgeTargetsDeprecatedNode {
                target: target.id.clone(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        Edge, EdgeKind, GraphVersion, Node, NodeId, NodeKind, NodeStatus, PartialParam, Signature,
        SourceLocation,
    };

    fn target_node(status: NodeStatus) -> Node {
        Node {
            id: NodeId::new("python", "pkg.target"),
            language: "python".to_string(),
            name: "target".to_string(),
            fqn: "pkg.target".to_string(),
            kind: NodeKind::Function,
            file_path: "pkg/target.py".to_string(),
            location: SourceLocation {
                start_line: 1,
                end_line: 2,
            },
            status,
            signature: Signature::PartiallyTyped {
                params: vec![PartialParam {
                    name: "value".to_string(),
                    type_annotation: None,
                    has_default: false,
                }],
                return_type: None,
                is_async: false,
            },
            body_hash: "a".repeat(64),
            introduced_at_version: GraphVersion::initial(),
        }
    }

    #[test]
    fn validator_rejects_uncalibrated_range_violations_and_deprecated_targets() {
        let validator = SchemaValidator::new(ConfidenceConfig::default());
        let edge = Edge::new(
            NodeId::new("python", "pkg.source"),
            NodeId::new("python", "pkg.target"),
            EdgeKind::Calls,
            0.95,
            ResolutionMethod::TypeInferred,
            GraphVersion::initial(),
        )
        .unwrap();

        assert!(matches!(
            validator.validate_edge_write(&edge, &target_node(NodeStatus::Active)),
            Err(SchemaError::ConfidenceOutOfCalibratedRange { .. })
        ));
        assert!(matches!(
            validator.validate_edge_write(
                &Edge::new(
                    NodeId::new("python", "pkg.source"),
                    NodeId::new("python", "pkg.target"),
                    EdgeKind::Calls,
                    0.4,
                    ResolutionMethod::Heuristic,
                    GraphVersion::initial(),
                )
                .unwrap(),
                &target_node(NodeStatus::Deprecated(crate::schema::DeprecatedMetadata {
                    expires_in_syncs: 1,
                    successor_id: None,
                    reason: crate::schema::DeprecationReason::Unresolved,
                })),
            ),
            Err(SchemaError::EdgeTargetsDeprecatedNode { .. })
        ));
    }
}
