use std::collections::{HashMap, HashSet};

use crate::errors::{SchemaError, ToolCallError};
use crate::rename::RenameCandidate;
use crate::schema::{
    ConfidenceConfig, DeprecatedMetadata, DeprecationReason, Edge, EdgeDraft, GraphSnapshot,
    GraphVersion, Node, NodeId, NodeStatus, NodeSummary, SubgraphSummary,
};
use crate::validator::SchemaValidator;

#[derive(Clone, Debug)]
pub struct GraphEngine {
    nodes: HashMap<NodeId, Node>,
    edges: Vec<Edge>,
    deleted_in_overlay: HashSet<NodeId>,
    current_version: GraphVersion,
    validator: SchemaValidator,
}

impl GraphEngine {
    pub fn new(confidence_config: ConfidenceConfig) -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            deleted_in_overlay: HashSet::new(),
            current_version: GraphVersion::initial(),
            validator: SchemaValidator::new(confidence_config),
        }
    }

    pub fn confidence_config(&self) -> &ConfidenceConfig {
        self.validator.confidence_config()
    }

    pub fn set_confidence_config(&mut self, confidence_config: ConfidenceConfig) {
        self.validator.set_confidence_config(confidence_config);
    }

    pub fn current_version(&self) -> GraphVersion {
        self.current_version
    }

    pub fn increment_version(&mut self) -> GraphVersion {
        self.current_version = self.current_version.increment();
        self.current_version
    }

    pub fn upsert_node(&mut self, node: Node) -> Result<Node, SchemaError> {
        let node = node.validate()?;
        self.nodes.insert(node.id.clone(), node.clone());
        Ok(node)
    }

    pub fn write_edge(&mut self, draft: EdgeDraft) -> Result<Edge, SchemaError> {
        let edge = Edge::new(
            draft.source.clone(),
            draft.target.clone(),
            draft.kind,
            draft.confidence,
            draft.resolution,
            draft.introduced_at_version,
        )?;

        let Some(_source_node) = self.nodes.get(&draft.source) else {
            return Err(SchemaError::MissingNode(draft.source.to_string()));
        };
        let Some(target_node) = self.nodes.get(&draft.target) else {
            return Err(SchemaError::MissingNode(draft.target.to_string()));
        };

        self.validator.validate_edge_write(&edge, target_node)?;
        self.edges.push(edge.clone());
        Ok(edge)
    }

    pub fn deprecate_node(
        &mut self,
        node_id: &str,
        metadata: DeprecatedMetadata,
    ) -> Result<Node, SchemaError> {
        let node_id = NodeId::from_hex(node_id.to_string())?;
        let node = self
            .nodes
            .get_mut(&node_id)
            .ok_or_else(|| SchemaError::MissingNode(node_id.to_string()))?;
        node.status = NodeStatus::Deprecated(metadata);
        node.introduced_at_version = self.current_version;
        Ok(node.clone())
    }

    pub fn mark_node_deleted_in_overlay(&mut self, node_id: &str) -> Result<(), ToolCallError> {
        let node_id =
            NodeId::from_hex(node_id.to_string()).map_err(|_| ToolCallError::UnknownNode {
                requested_id: node_id.to_string(),
                suggestion: "Use find_callers('<name>') to search by human-readable name."
                    .to_string(),
            })?;
        if !self.nodes.contains_key(&node_id) {
            return Err(ToolCallError::UnknownNode {
                requested_id: node_id.to_string(),
                suggestion: "Use find_callers('<name>') to search by human-readable name."
                    .to_string(),
            });
        }
        self.deleted_in_overlay.insert(node_id);
        Ok(())
    }

    pub fn create_snapshot(&self, pr_identifier: String) -> GraphSnapshot {
        GraphSnapshot {
            baseline_version: self.current_version,
            current_baseline_version: self.current_version,
            pr_identifier,
        }
    }

    pub fn detect_rename(
        &self,
        deprecated_node_id: &str,
        candidate_node_id: &str,
    ) -> Result<RenameCandidate, ToolCallError> {
        let deprecated = self.resolve_node_or_error(deprecated_node_id)?;
        let candidate = self.resolve_node_or_error(candidate_node_id)?;
        Ok(RenameCandidate::from_nodes(deprecated, candidate))
    }

    pub fn apply_rename(
        &mut self,
        deprecated_node_id: &str,
        candidate_node_id: &str,
    ) -> Result<RenameCandidate, ToolCallError> {
        let candidate = self.detect_rename(deprecated_node_id, candidate_node_id)?;
        if candidate.auto_accept() {
            for edge in &mut self.edges {
                if edge.target == candidate.deprecated_node_id {
                    edge.target = candidate.candidate_node_id.clone();
                }
            }

            let deprecated_node = self
                .nodes
                .get_mut(&candidate.deprecated_node_id)
                .expect("deprecated node should exist");
            deprecated_node.status = NodeStatus::Deprecated(DeprecatedMetadata {
                expires_in_syncs: 1,
                successor_id: Some(candidate.candidate_node_id.clone()),
                reason: DeprecationReason::RenamedTo {
                    new_fqn: candidate.candidate_fqn.clone(),
                },
            });
        }
        Ok(candidate)
    }

    pub fn get_node_detail(&self, node_id: &str) -> Result<Node, ToolCallError> {
        let node = self.resolve_node_or_error(node_id)?;
        if self.deleted_in_overlay.contains(&node.id) {
            return Err(ToolCallError::NodeDeletedInPr {
                fqn: node.fqn.clone(),
                suggestion: "Flag callers of this node as potentially broken by the PR."
                    .to_string(),
            });
        }
        Ok(node.clone())
    }

    pub fn get_subgraph(
        &self,
        node_id: &str,
        escalation_confidence_threshold: f32,
        max_nodes: usize,
    ) -> Result<SubgraphSummary, ToolCallError> {
        if max_nodes == 0 {
            return Err(ToolCallError::invalid_schema(
                "max_nodes must be at least 1".to_string(),
            ));
        }
        if max_nodes > 25 {
            return Err(ToolCallError::invalid_schema(
                "max_nodes may not exceed the hard cap of 25".to_string(),
            ));
        }

        let queried_node = self.resolve_node_or_error(node_id)?;

        let mut callers = Vec::new();
        let mut callees = Vec::new();

        for edge in &self.edges {
            if edge.target == queried_node.id {
                if let Some(source_node) = self.nodes.get(&edge.source) {
                    callers.push((
                        summary_for(source_node, edge),
                        edge.requires_escalation(escalation_confidence_threshold),
                    ));
                }
            }
            if edge.source == queried_node.id {
                if let Some(target_node) = self.nodes.get(&edge.target) {
                    callees.push((
                        summary_for(target_node, edge),
                        edge.requires_escalation(escalation_confidence_threshold),
                    ));
                }
            }
        }

        let total_nodes = callers.len() + callees.len();
        if total_nodes > 50 {
            return Err(ToolCallError::SubgraphTooLarge {
                node_count: total_nodes,
                cap: max_nodes,
                suggestion: "Reduce traversal depth or review a more specific entry point."
                    .to_string(),
            });
        }

        let total_callers = callers.len();
        let total_callees = callees.len();
        let omitted_count = total_nodes.saturating_sub(max_nodes);
        let truncated = omitted_count > 0;

        let allowed_callers = total_callers.min(max_nodes);
        let remaining_for_callees = max_nodes.saturating_sub(allowed_callers);

        callers.truncate(allowed_callers);
        callees.truncate(remaining_for_callees);

        let (high_confidence_callers, low_confidence_callers): (Vec<_>, Vec<_>) = callers
            .into_iter()
            .partition(|(_, requires_escalation)| !requires_escalation);
        let (high_confidence_callees, low_confidence_callees): (Vec<_>, Vec<_>) = callees
            .into_iter()
            .partition(|(_, requires_escalation)| !requires_escalation);

        Ok(SubgraphSummary {
            queried_node_id: queried_node.id.clone(),
            queried_node_fqn: queried_node.fqn.clone(),
            total_callers,
            total_callees,
            high_confidence_callers: high_confidence_callers
                .into_iter()
                .map(|(summary, _)| summary)
                .collect(),
            low_confidence_callers: low_confidence_callers
                .into_iter()
                .map(|(summary, _)| summary)
                .collect(),
            high_confidence_callees: high_confidence_callees
                .into_iter()
                .map(|(summary, _)| summary)
                .collect(),
            low_confidence_callees: low_confidence_callees
                .into_iter()
                .map(|(summary, _)| summary)
                .collect(),
            truncated,
            omitted_count: truncated.then_some(omitted_count),
        })
    }

    fn resolve_node_or_error(&self, node_id: &str) -> Result<&Node, ToolCallError> {
        let node_id =
            NodeId::from_hex(node_id.to_string()).map_err(|_| ToolCallError::UnknownNode {
                requested_id: node_id.to_string(),
                suggestion: "Use find_callers('<name>') to search by human-readable name."
                    .to_string(),
            })?;

        self.nodes
            .get(&node_id)
            .ok_or_else(|| ToolCallError::UnknownNode {
                requested_id: node_id.as_str().to_string(),
                suggestion: "Use find_callers('<name>') to search by human-readable name."
                    .to_string(),
            })
    }
}

fn summary_for(node: &Node, edge: &Edge) -> NodeSummary {
    NodeSummary {
        id: node.id.clone(),
        fqn: node.fqn.clone(),
        kind: node.kind.clone(),
        confidence: edge.confidence(),
        resolution: edge.resolution,
        source_available: !node.status.is_deprecated(),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::schema::{
        EdgeDraft, EdgeKind, PartialParam, ResolutionMethod, Signature, SourceLocation,
    };

    fn node(language: &str, fqn: &str) -> Node {
        Node {
            id: NodeId::new(language, fqn),
            language: language.to_string(),
            name: fqn.rsplit('.').next().unwrap().to_string(),
            fqn: fqn.to_string(),
            kind: crate::schema::NodeKind::Function,
            file_path: format!("src/{}.py", fqn.replace('.', "/")),
            location: SourceLocation {
                start_line: 1,
                end_line: 10,
            },
            status: NodeStatus::Active,
            signature: Signature::PartiallyTyped {
                params: vec![PartialParam {
                    name: "config".to_string(),
                    type_annotation: None,
                    has_default: false,
                }],
                return_type: None,
                is_async: false,
            },
            body_hash: "abc123".repeat(10) + "ab",
            introduced_at_version: GraphVersion::initial(),
        }
    }

    #[test]
    fn subgraph_queries_partition_by_confidence() {
        let mut engine = GraphEngine::new(ConfidenceConfig::default());
        let target = node("python", "pkg.target");
        let caller = node("python", "pkg.caller");
        let callee = node("python", "pkg.callee");
        engine.upsert_node(target.clone()).unwrap();
        engine.upsert_node(caller.clone()).unwrap();
        engine.upsert_node(callee.clone()).unwrap();

        engine
            .write_edge(EdgeDraft {
                source: caller.id.clone(),
                target: target.id.clone(),
                kind: EdgeKind::Calls,
                confidence: 0.8,
                resolution: ResolutionMethod::TypeInferred,
                introduced_at_version: GraphVersion::initial(),
            })
            .unwrap();
        engine
            .write_edge(EdgeDraft {
                source: target.id.clone(),
                target: callee.id.clone(),
                kind: EdgeKind::Calls,
                confidence: 0.4,
                resolution: ResolutionMethod::Heuristic,
                introduced_at_version: GraphVersion::initial(),
            })
            .unwrap();

        let summary = engine.get_subgraph(target.id.as_str(), 0.7, 25).unwrap();
        assert_eq!(summary.total_callers, 1);
        assert_eq!(summary.total_callees, 1);
        assert_eq!(summary.high_confidence_callers.len(), 1);
        assert_eq!(summary.low_confidence_callees.len(), 1);
    }

    #[test]
    fn applying_high_confidence_rename_reroutes_edges() {
        let mut engine = GraphEngine::new(ConfidenceConfig::default());
        let caller = node("python", "pkg.caller");
        let mut deprecated = node("python", "pkg.old_name");
        let mut replacement = node("python", "pkg.new_name");
        deprecated.body_hash = "same".repeat(16);
        replacement.body_hash = "same".repeat(16);
        replacement.file_path = deprecated.file_path.clone();
        replacement.signature = deprecated.signature.clone();
        engine.upsert_node(caller.clone()).unwrap();
        engine.upsert_node(deprecated.clone()).unwrap();
        engine.upsert_node(replacement.clone()).unwrap();
        engine
            .write_edge(EdgeDraft {
                source: caller.id.clone(),
                target: deprecated.id.clone(),
                kind: EdgeKind::Calls,
                confidence: 1.0,
                resolution: ResolutionMethod::Static,
                introduced_at_version: GraphVersion::initial(),
            })
            .unwrap();

        let candidate = engine
            .apply_rename(deprecated.id.as_str(), replacement.id.as_str())
            .unwrap();
        assert!(candidate.auto_accept());
        assert_eq!(engine.edges[0].target, replacement.id);
        assert!(matches!(
            engine.nodes.get(&deprecated.id).unwrap().status,
            NodeStatus::Deprecated(_)
        ));
    }
}
