use serde::{Deserialize, Serialize};

use crate::schema::{Node, NodeId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenameEvidence {
    pub body_hash_match: bool,
    pub arity_match: bool,
    pub param_names_match: bool,
    pub same_directory: bool,
}

impl RenameEvidence {
    pub fn confidence(&self) -> f32 {
        let mut confidence: f32 = 0.0;
        if self.body_hash_match {
            confidence += 0.70;
        }
        if self.arity_match {
            confidence += 0.15;
        }
        if self.param_names_match {
            confidence += 0.10;
        }
        if self.same_directory {
            confidence += 0.05;
        }
        confidence.min(1.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenameCandidate {
    pub deprecated_node_id: NodeId,
    pub deprecated_fqn: String,
    pub candidate_node_id: NodeId,
    pub candidate_fqn: String,
    pub evidence: RenameEvidence,
    pub confidence: f32,
}

impl RenameCandidate {
    pub const AUTO_ACCEPT_THRESHOLD: f32 = 0.70;

    pub fn from_nodes(deprecated: &Node, candidate: &Node) -> Self {
        let structurally_comparable = deprecated.signature.is_structurally_comparable()
            && candidate.signature.is_structurally_comparable();
        let evidence = RenameEvidence {
            body_hash_match: deprecated.body_hash == candidate.body_hash,
            arity_match: deprecated.signature.arity() == candidate.signature.arity(),
            param_names_match: structurally_comparable
                && deprecated.signature.param_names() == candidate.signature.param_names(),
            same_directory: parent_directory(&deprecated.file_path)
                == parent_directory(&candidate.file_path),
        };

        let confidence = evidence.confidence();
        Self {
            deprecated_node_id: deprecated.id.clone(),
            deprecated_fqn: deprecated.fqn.clone(),
            candidate_node_id: candidate.id.clone(),
            candidate_fqn: candidate.fqn.clone(),
            evidence,
            confidence,
        }
    }

    pub fn auto_accept(&self) -> bool {
        self.confidence >= Self::AUTO_ACCEPT_THRESHOLD
    }
}

fn parent_directory(path: &str) -> &str {
    path.rsplit_once(['/', '\\'])
        .map(|(directory, _)| directory)
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        GraphVersion, Node, NodeKind, NodeStatus, PartialParam, Signature, SourceLocation,
    };

    fn node(fqn: &str, file_path: &str, body_hash: &str) -> Node {
        Node {
            id: NodeId::new("python", fqn),
            language: "python".to_string(),
            name: fqn.rsplit('.').next().unwrap().to_string(),
            fqn: fqn.to_string(),
            kind: NodeKind::Function,
            file_path: file_path.to_string(),
            location: SourceLocation {
                start_line: 1,
                end_line: 3,
            },
            status: NodeStatus::Active,
            signature: Signature::PartiallyTyped {
                params: vec![PartialParam {
                    name: "value".to_string(),
                    type_annotation: None,
                    has_default: false,
                }],
                return_type: None,
                is_async: false,
            },
            body_hash: body_hash.to_string(),
            introduced_at_version: GraphVersion::initial(),
        }
    }

    #[test]
    fn rename_confidence_uses_weighted_evidence() {
        let deprecated = node("pkg.old", "pkg/old.py", "same");
        let candidate = node("pkg.new", "other/new.py", "same");
        let rename = RenameCandidate::from_nodes(&deprecated, &candidate);
        assert!((rename.confidence - 0.95).abs() < f32::EPSILON);
        assert!(rename.auto_accept());
    }
}
