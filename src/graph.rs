use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::errors::{SchemaError, ToolCallError};
use crate::parser::scan_python_project;
use crate::rename::RenameCandidate;
use crate::schema::{
    ChangedNode, ConfidenceConfig, DeprecatedMetadata, DeprecationReason, DiffAnalysis,
    DiffFileSummary, Edge, EdgeDraft, GraphSnapshot, GraphVersion, Node, NodeId, NodeStatus,
    NodeSummary, SubgraphSummary, SyncReport,
};
use crate::validator::SchemaValidator;

#[derive(Clone, Debug)]
pub struct GraphEngine {
    nodes: HashMap<NodeId, Node>,
    edges: Vec<Edge>,
    deleted_in_overlay: HashSet<NodeId>,
    unresolved_calls: HashMap<NodeId, Vec<String>>,
    file_index: HashMap<String, Vec<NodeId>>,
    current_version: GraphVersion,
    validator: SchemaValidator,
}

impl GraphEngine {
    pub fn new(confidence_config: ConfidenceConfig) -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            deleted_in_overlay: HashSet::new(),
            unresolved_calls: HashMap::new(),
            file_index: HashMap::new(),
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
        self.rebuild_indexes();
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

    pub fn sync_python_project(&mut self, root: &str) -> Result<SyncReport, String> {
        let root_path = Path::new(root);
        if !root_path.exists() {
            return Err(format!("Project root `{root}` does not exist"));
        }

        let version = self.next_sync_version();
        let garbage_collected_nodes = self.age_out_deprecated_nodes();
        let parsed = scan_python_project(root_path, version).map_err(|error| error.to_string())?;
        let existing_active = self.active_nodes();
        let new_node_map: HashMap<NodeId, Node> = parsed
            .nodes
            .iter()
            .cloned()
            .map(|node| (node.id.clone(), node))
            .collect();
        let removed_ids: Vec<NodeId> = existing_active
            .keys()
            .filter(|node_id| !new_node_map.contains_key(*node_id))
            .cloned()
            .collect();
        let added_ids: HashSet<NodeId> = new_node_map
            .keys()
            .filter(|node_id| !existing_active.contains_key(*node_id))
            .cloned()
            .collect();

        let mut renamed_nodes = Vec::new();
        let mut unresolved_deprecations = Vec::new();
        let mut used_successors = HashSet::new();

        for removed_id in &removed_ids {
            let removed_node = existing_active
                .get(removed_id)
                .expect("removed node should exist")
                .clone();

            let mut best_candidate: Option<RenameCandidate> = None;
            for added_id in &added_ids {
                if used_successors.contains(added_id) {
                    continue;
                }
                let Some(candidate_node) = new_node_map.get(added_id) else {
                    continue;
                };
                let candidate = RenameCandidate::from_nodes(&removed_node, candidate_node);
                if best_candidate
                    .as_ref()
                    .map(|current| candidate.confidence > current.confidence)
                    .unwrap_or(true)
                {
                    best_candidate = Some(candidate);
                }
            }

            if let Some(candidate) = best_candidate {
                if candidate.auto_accept() {
                    used_successors.insert(candidate.candidate_node_id.clone());
                    renamed_nodes.push(candidate.clone());
                    self.reroute_inbound_edges(
                        &candidate.deprecated_node_id,
                        &candidate.candidate_node_id,
                    );
                    self.nodes.insert(
                        removed_node.id.clone(),
                        deprecated_copy(
                            removed_node,
                            version,
                            Some(candidate.candidate_node_id.clone()),
                            DeprecationReason::RenamedTo {
                                new_fqn: candidate.candidate_fqn.clone(),
                            },
                        ),
                    );
                    continue;
                }
            }

            unresolved_deprecations.push(removed_node.fqn.clone());
            self.nodes.insert(
                removed_node.id.clone(),
                deprecated_copy(removed_node, version, None, DeprecationReason::Unresolved),
            );
        }

        for parsed_node in parsed.nodes {
            let parsed_id = parsed_node.id.clone();
            let next_node = if let Some(existing_node) = self.nodes.get(&parsed_id) {
                if existing_node.status.is_deprecated()
                    || !node_semantically_equal(existing_node, &parsed_node)
                {
                    Node {
                        introduced_at_version: version,
                        ..parsed_node
                    }
                } else {
                    Node {
                        introduced_at_version: existing_node.introduced_at_version,
                        ..parsed_node
                    }
                }
            } else {
                parsed_node
            };
            self.nodes.insert(parsed_id, next_node);
        }

        self.edges.clear();
        for draft in parsed.edge_drafts {
            self.write_edge(draft).map_err(|error| error.to_string())?;
        }
        self.unresolved_calls = parsed.unresolved_calls;
        self.current_version = version;
        self.rebuild_indexes();

        Ok(SyncReport {
            root: root.to_string(),
            version,
            scanned_files: parsed.scanned_files,
            active_nodes: self
                .nodes
                .values()
                .filter(|node| matches!(node.status, NodeStatus::Active))
                .count(),
            deprecated_nodes: self
                .nodes
                .values()
                .filter(|node| matches!(node.status, NodeStatus::Deprecated(_)))
                .count(),
            garbage_collected_nodes,
            renamed_nodes,
            unresolved_deprecations,
            warnings: parsed.warnings,
        })
    }

    pub fn analyze_diff(&self, diff_text: &str) -> Result<DiffAnalysis, ToolCallError> {
        let raw_files = parse_unified_diff(diff_text);
        let mut changed_node_ids = Vec::new();
        let mut seen_changed_nodes = HashSet::new();
        let mut changed_files = Vec::new();
        let mut deleted_symbols = Vec::new();
        let mut added_symbols = Vec::new();

        for raw_file in raw_files {
            deleted_symbols.extend(raw_file.deleted_symbols.clone());
            added_symbols.extend(raw_file.added_symbols.clone());

            let mut changed_nodes = Vec::new();
            if let Some(node_ids) = self.file_index.get(&raw_file.file_path) {
                for node_id in node_ids {
                    let Some(node) = self.nodes.get(node_id) else {
                        continue;
                    };
                    let changed = overlaps_lines(&raw_file.added_lines, &node.location)
                        || overlaps_lines(&raw_file.removed_lines, &node.location)
                        || (raw_file.added_lines.is_empty()
                            && raw_file.removed_lines.is_empty()
                            && matches!(node.kind, crate::schema::NodeKind::Module));
                    if changed {
                        changed_nodes.push(ChangedNode {
                            id: node.id.clone(),
                            fqn: node.fqn.clone(),
                            file_path: node.file_path.clone(),
                            start_line: node.location.start_line,
                            end_line: node.location.end_line,
                        });
                    }
                }
            }

            let has_specific_nodes = changed_nodes.iter().any(|node| {
                self.nodes
                    .get(&node.id)
                    .map(|graph_node| !matches!(graph_node.kind, crate::schema::NodeKind::Module))
                    .unwrap_or(false)
            });
            if has_specific_nodes {
                changed_nodes.retain(|node| {
                    self.nodes
                        .get(&node.id)
                        .map(|graph_node| {
                            !matches!(graph_node.kind, crate::schema::NodeKind::Module)
                        })
                        .unwrap_or(true)
                });
            }

            for node in &changed_nodes {
                if seen_changed_nodes.insert(node.id.clone()) {
                    changed_node_ids.push(node.id.clone());
                }
            }

            changed_files.push(DiffFileSummary {
                file_path: raw_file.file_path,
                added_lines: raw_file.added_lines,
                removed_lines: raw_file.removed_lines,
                added_symbols: raw_file.added_symbols,
                deleted_symbols: raw_file.deleted_symbols,
                changed_nodes,
            });
        }

        Ok(DiffAnalysis {
            changed_files,
            changed_node_ids,
            deleted_symbols,
            added_symbols,
        })
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
        let result = node.clone();
        self.rebuild_indexes();
        Ok(result)
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
            self.reroute_inbound_edges(&candidate.deprecated_node_id, &candidate.candidate_node_id);
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

    pub fn get_unresolved_calls(&self, node_id: &str) -> Result<Vec<String>, ToolCallError> {
        let node = self.resolve_node_or_error(node_id)?;
        Ok(self
            .unresolved_calls
            .get(&node.id)
            .cloned()
            .unwrap_or_default())
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

    fn next_sync_version(&self) -> GraphVersion {
        if self.nodes.is_empty() && self.current_version == GraphVersion::initial() {
            self.current_version
        } else {
            self.current_version.increment()
        }
    }

    fn age_out_deprecated_nodes(&mut self) -> usize {
        let mut removed = Vec::new();
        for (node_id, node) in &mut self.nodes {
            if let NodeStatus::Deprecated(metadata) = &mut node.status {
                if metadata.expires_in_syncs > 0 {
                    metadata.expires_in_syncs -= 1;
                }
                if metadata.expires_in_syncs == 0 {
                    removed.push(node_id.clone());
                }
            }
        }

        for node_id in &removed {
            self.nodes.remove(node_id);
            self.unresolved_calls.remove(node_id);
            self.deleted_in_overlay.remove(node_id);
            self.edges
                .retain(|edge| &edge.source != node_id && &edge.target != node_id);
        }
        removed.len()
    }

    fn reroute_inbound_edges(&mut self, from: &NodeId, to: &NodeId) {
        for edge in &mut self.edges {
            if &edge.target == from {
                edge.target = to.clone();
            }
        }
    }

    fn active_nodes(&self) -> HashMap<NodeId, Node> {
        self.nodes
            .iter()
            .filter(|(_, node)| matches!(node.status, NodeStatus::Active))
            .map(|(node_id, node)| (node_id.clone(), node.clone()))
            .collect()
    }

    fn rebuild_indexes(&mut self) {
        self.file_index.clear();
        for node in self
            .nodes
            .values()
            .filter(|node| matches!(node.status, NodeStatus::Active))
        {
            self.file_index
                .entry(node.file_path.clone())
                .or_default()
                .push(node.id.clone());
        }
    }
}

fn deprecated_copy(
    mut node: Node,
    version: GraphVersion,
    successor_id: Option<NodeId>,
    reason: DeprecationReason,
) -> Node {
    node.status = NodeStatus::Deprecated(DeprecatedMetadata {
        expires_in_syncs: 1,
        successor_id,
        reason,
    });
    node.introduced_at_version = version;
    node
}

fn node_semantically_equal(existing: &Node, next: &Node) -> bool {
    existing.language == next.language
        && existing.name == next.name
        && existing.fqn == next.fqn
        && existing.kind == next.kind
        && existing.file_path == next.file_path
        && existing.location == next.location
        && existing.signature == next.signature
        && existing.body_hash == next.body_hash
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

#[derive(Default)]
struct RawDiffFile {
    file_path: String,
    added_lines: Vec<u32>,
    removed_lines: Vec<u32>,
    added_symbols: Vec<String>,
    deleted_symbols: Vec<String>,
}

fn parse_unified_diff(diff_text: &str) -> Vec<RawDiffFile> {
    let mut files = Vec::new();
    let mut current: Option<RawDiffFile> = None;
    let mut old_line = 0u32;
    let mut new_line = 0u32;

    for line in diff_text.lines() {
        if let Some(path) = line.strip_prefix("diff --git ") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            current = Some(RawDiffFile {
                file_path: diff_header_path(path),
                ..RawDiffFile::default()
            });
            continue;
        }

        if let Some(hunk) = line.strip_prefix("@@ ") {
            if let Some((old_start, new_start)) = parse_hunk_positions(hunk) {
                old_line = old_start;
                new_line = new_start;
            }
            continue;
        }

        let Some(file) = current.as_mut() else {
            continue;
        };

        if let Some(path) = line.strip_prefix("+++ ") {
            if path != "/dev/null" {
                file.file_path = normalize_diff_path(path);
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("--- ") {
            if file.file_path.is_empty() && path != "/dev/null" {
                file.file_path = normalize_diff_path(path);
            }
            continue;
        }

        if line.starts_with('+') && !line.starts_with("+++") {
            file.added_lines.push(new_line);
            if let Some(symbol) = extract_symbol_name(&line[1..]) {
                file.added_symbols.push(symbol);
            }
            new_line += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            file.removed_lines.push(old_line);
            if let Some(symbol) = extract_symbol_name(&line[1..]) {
                file.deleted_symbols.push(symbol);
            }
            old_line += 1;
        } else if line.starts_with(' ') {
            old_line += 1;
            new_line += 1;
        }
    }

    if let Some(file) = current {
        files.push(file);
    }

    files
}

fn diff_header_path(path: &str) -> String {
    let mut parts = path.split_whitespace();
    let _old = parts.next();
    let new_path = parts.next().unwrap_or_default();
    normalize_diff_path(new_path)
}

fn normalize_diff_path(path: &str) -> String {
    path.trim_start_matches("a/")
        .trim_start_matches("b/")
        .replace('\\', "/")
}

fn parse_hunk_positions(hunk: &str) -> Option<(u32, u32)> {
    static HUNK_RE: OnceLock<Regex> = OnceLock::new();
    let regex = HUNK_RE
        .get_or_init(|| Regex::new(r"-([0-9]+)(?:,[0-9]+)? \+([0-9]+)(?:,[0-9]+)?").unwrap());
    let captures = regex.captures(hunk)?;
    let old_start = captures.get(1)?.as_str().parse().ok()?;
    let new_start = captures.get(2)?.as_str().parse().ok()?;
    Some((old_start, new_start))
}

fn extract_symbol_name(line: &str) -> Option<String> {
    static SYMBOL_RE: OnceLock<Regex> = OnceLock::new();
    let regex = SYMBOL_RE.get_or_init(|| {
        Regex::new(r"^\s*(?:async\s+def|def|class)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
    });
    regex
        .captures(line)
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_string()))
}

fn overlaps_lines(lines: &[u32], location: &crate::schema::SourceLocation) -> bool {
    lines
        .iter()
        .any(|line| *line >= location.start_line && *line <= location.end_line)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    fn write_project(temp_dir: &TempDir, files: &[(&str, &str)]) {
        for (path, contents) in files {
            let full_path = temp_dir.path().join(path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full_path, contents).unwrap();
        }
    }

    #[test]
    fn sync_project_extracts_nodes_edges_and_unresolved_calls() {
        let temp_dir = TempDir::new().unwrap();
        write_project(
            &temp_dir,
            &[
                (
                    "app.py",
                    r#"
import helpers

class Parser:
    def parse(self, value: str) -> str:
        cleaned = helpers.clean(value)
        return self.render(cleaned)

    def render(self, value):
        return format_result(value)

def format_result(value):
    return value
"#,
                ),
                (
                    "helpers.py",
                    "def clean(value: str) -> str:\n    return value.strip()\n",
                ),
            ],
        );

        let mut engine = GraphEngine::new(ConfidenceConfig::default());
        let report = engine
            .sync_python_project(temp_dir.path().to_string_lossy().as_ref())
            .unwrap();
        assert_eq!(report.scanned_files, 2);
        assert!(
            engine
                .nodes
                .values()
                .any(|node| node.fqn == "app.Parser.parse")
        );
        assert!(
            engine
                .edges
                .iter()
                .any(|edge| matches!(edge.kind, crate::schema::EdgeKind::Imports))
        );
        assert!(
            engine
                .edges
                .iter()
                .any(|edge| matches!(edge.kind, crate::schema::EdgeKind::Calls))
        );
        assert_eq!(
            engine
                .unresolved_calls
                .values()
                .flat_map(|calls| calls.iter())
                .count(),
            1
        );
    }

    #[test]
    fn sync_project_marks_renames_and_garbage_collects() {
        let temp_dir = TempDir::new().unwrap();
        write_project(
            &temp_dir,
            &[(
                "service.py",
                "def old_name(value):\n    cleaned = value.strip()\n    return cleaned\n",
            )],
        );

        let mut engine = GraphEngine::new(ConfidenceConfig::default());
        engine
            .sync_python_project(temp_dir.path().to_string_lossy().as_ref())
            .unwrap();

        write_project(
            &temp_dir,
            &[(
                "service.py",
                "def new_name(value):\n    cleaned = value.strip()\n    return cleaned\n",
            )],
        );
        let second = engine
            .sync_python_project(temp_dir.path().to_string_lossy().as_ref())
            .unwrap();
        assert_eq!(second.renamed_nodes.len(), 1);
        assert!(
            engine
                .nodes
                .values()
                .any(|node| node.fqn == "service.old_name" && node.status.is_deprecated())
        );

        let third = engine
            .sync_python_project(temp_dir.path().to_string_lossy().as_ref())
            .unwrap();
        assert_eq!(third.garbage_collected_nodes, 1);
        assert!(
            !engine
                .nodes
                .values()
                .any(|node| node.fqn == "service.old_name")
        );
    }

    #[test]
    fn diff_analysis_maps_changed_nodes() {
        let temp_dir = TempDir::new().unwrap();
        write_project(
            &temp_dir,
            &[(
                "app.py",
                "def run(value):\n    return helper(value)\n\ndef helper(value):\n    return value\n",
            )],
        );
        let mut engine = GraphEngine::new(ConfidenceConfig::default());
        engine
            .sync_python_project(temp_dir.path().to_string_lossy().as_ref())
            .unwrap();

        let diff = r#"diff --git a/app.py b/app.py
--- a/app.py
+++ b/app.py
@@ -1,4 +1,5 @@
 def run(value):
-    return helper(value)
+    return helper(value).strip()
 
 def helper(value):
     return value
"#;
        let analysis = engine.analyze_diff(diff).unwrap();
        assert_eq!(analysis.changed_files.len(), 1);
        assert_eq!(analysis.changed_node_ids.len(), 1);
        assert_eq!(analysis.changed_files[0].file_path, "app.py");
    }
}
