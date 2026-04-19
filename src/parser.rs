use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::schema::{
    EdgeDraft, EdgeKind, GraphVersion, Node, NodeId, NodeKind, NodeStatus, PartialParam,
    ResolutionMethod, Signature, SourceLocation,
};

#[derive(Debug)]
pub struct ParsedProject {
    pub nodes: Vec<Node>,
    pub edge_drafts: Vec<EdgeDraft>,
    pub unresolved_calls: HashMap<NodeId, Vec<String>>,
    pub scanned_files: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
struct ParsedFile {
    nodes: Vec<Node>,
    callsites: Vec<CallsiteRecord>,
    import_records: Vec<ImportRecord>,
}

#[derive(Clone, Debug)]
struct CallsiteRecord {
    source: NodeId,
    module_fqn: String,
    parent_class_fqn: Option<String>,
    token: String,
    aliases: HashMap<String, String>,
}

#[derive(Clone, Debug)]
struct ImportRecord {
    source_module: NodeId,
    target_fqn: String,
}

#[derive(Debug)]
struct OpenEntity {
    node: Node,
    indent: usize,
    kind: OpenEntityKind,
    body_lines: Vec<String>,
    call_tokens: Vec<String>,
    aliases: HashMap<String, String>,
    last_line: u32,
}

#[derive(Clone, Debug)]
enum OpenEntityKind {
    Class,
    Callable {
        module_fqn: String,
        parent_class_fqn: Option<String>,
    },
}

pub fn scan_python_project(root: &Path, version: GraphVersion) -> io::Result<ParsedProject> {
    let mut all_nodes = Vec::new();
    let mut all_callsites = Vec::new();
    let mut all_imports = Vec::new();
    let mut warnings = Vec::new();
    let mut scanned_files = 0;

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| should_visit_path(entry.path()))
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("py") {
            continue;
        }

        scanned_files += 1;
        let parsed = parse_python_file(root, entry.path(), version)?;
        if parsed.nodes.is_empty() {
            warnings.push(format!(
                "No addressable nodes were extracted from {}",
                normalize_rel_path(root, entry.path())?
            ));
        }
        all_nodes.extend(parsed.nodes);
        all_callsites.extend(parsed.callsites);
        all_imports.extend(parsed.import_records);
    }

    let node_by_fqn: HashMap<String, Node> = all_nodes
        .iter()
        .cloned()
        .map(|node| (node.fqn.clone(), node))
        .collect();
    let node_by_id: HashMap<NodeId, Node> = all_nodes
        .iter()
        .cloned()
        .map(|node| (node.id.clone(), node))
        .collect();
    let mut short_name_index: HashMap<String, Vec<NodeId>> = HashMap::new();
    for node in &all_nodes {
        short_name_index
            .entry(node.name.clone())
            .or_default()
            .push(node.id.clone());
    }

    let mut edge_keys: HashSet<(NodeId, NodeId, EdgeKind)> = HashSet::new();
    let mut edge_drafts = Vec::new();
    for import in all_imports {
        if let Some(target) = resolve_import_target(&import.target_fqn, &node_by_fqn) {
            let key = (
                import.source_module.clone(),
                target.id.clone(),
                EdgeKind::Imports,
            );
            if edge_keys.insert(key) {
                edge_drafts.push(EdgeDraft {
                    source: import.source_module.clone(),
                    target: target.id.clone(),
                    kind: EdgeKind::Imports,
                    confidence: 1.0,
                    resolution: ResolutionMethod::Static,
                    introduced_at_version: version,
                });
            }
        }
    }

    let mut unresolved_calls: HashMap<NodeId, Vec<String>> = HashMap::new();
    for callsite in all_callsites {
        if let Some((target, kind, confidence, resolution)) =
            resolve_callsite(&callsite, &node_by_fqn, &node_by_id, &short_name_index)
        {
            let key = (callsite.source.clone(), target.id.clone(), kind);
            if edge_keys.insert(key) {
                edge_drafts.push(EdgeDraft {
                    source: callsite.source.clone(),
                    target: target.id.clone(),
                    kind,
                    confidence,
                    resolution,
                    introduced_at_version: version,
                });
            }
        } else {
            unresolved_calls
                .entry(callsite.source.clone())
                .or_default()
                .push(short_token(&callsite.token));
        }
    }

    Ok(ParsedProject {
        nodes: all_nodes,
        edge_drafts,
        unresolved_calls,
        scanned_files,
        warnings,
    })
}

fn parse_python_file(root: &Path, path: &Path, version: GraphVersion) -> io::Result<ParsedFile> {
    let source = std::fs::read_to_string(path)?;
    let relative_path = normalize_rel_path(root, path)?;
    let module_fqn = module_fqn_for_path(root, path);
    let total_lines = source.lines().count() as u32;

    let module_node = Node {
        id: NodeId::new("python", &module_fqn),
        language: "python".to_string(),
        name: module_fqn
            .rsplit('.')
            .next()
            .unwrap_or(module_fqn.as_str())
            .to_string(),
        fqn: module_fqn.clone(),
        kind: NodeKind::Module,
        file_path: relative_path.clone(),
        location: SourceLocation {
            start_line: 1,
            end_line: total_lines.max(1),
        },
        status: NodeStatus::Active,
        signature: Signature::Untyped { arity: None },
        body_hash: hash_normalized_lines(&source.lines().map(str::to_string).collect::<Vec<_>>()),
        introduced_at_version: version,
    };

    let mut nodes = vec![module_node.clone()];
    let mut callsites = Vec::new();
    let mut import_records = Vec::new();
    let mut aliases = HashMap::new();
    let mut open_entities: Vec<OpenEntity> = Vec::new();

    for (index, line) in source.lines().enumerate() {
        let line_number = index as u32 + 1;
        let indent = leading_spaces(line);
        let trimmed = line.trim_start();
        let significant = !trimmed.is_empty() && !trimmed.starts_with('#');

        if significant {
            while let Some(entity) = open_entities.last() {
                if indent <= entity.indent {
                    close_entity(
                        open_entities.pop().expect("open entity should exist"),
                        line_number - 1,
                        &mut nodes,
                        &mut callsites,
                    );
                } else {
                    break;
                }
            }
        }

        for entity in &mut open_entities {
            entity.body_lines.push(line.to_string());
            entity.last_line = line_number;
        }

        if significant && open_entities.is_empty() {
            for (alias, target) in parse_import_aliases(trimmed) {
                aliases.insert(alias, target.clone());
                import_records.push(ImportRecord {
                    source_module: module_node.id.clone(),
                    target_fqn: target,
                });
            }
        }

        if let Some(class_name) = parse_class_name(trimmed) {
            if open_entities
                .iter()
                .any(|entity| matches!(entity.kind, OpenEntityKind::Callable { .. }))
            {
                continue;
            }
            let class_fqn = format!("{module_fqn}.{class_name}");
            open_entities.push(OpenEntity {
                node: Node {
                    id: NodeId::new("python", &class_fqn),
                    language: "python".to_string(),
                    name: class_name.to_string(),
                    fqn: class_fqn,
                    kind: NodeKind::Class,
                    file_path: relative_path.clone(),
                    location: SourceLocation {
                        start_line: line_number,
                        end_line: line_number,
                    },
                    status: NodeStatus::Active,
                    signature: Signature::Untyped { arity: None },
                    body_hash: String::new(),
                    introduced_at_version: version,
                },
                indent,
                kind: OpenEntityKind::Class,
                body_lines: Vec::new(),
                call_tokens: Vec::new(),
                aliases: aliases.clone(),
                last_line: line_number,
            });
            continue;
        }

        if let Some((name, signature)) = parse_def_signature(trimmed) {
            if open_entities
                .iter()
                .any(|entity| matches!(entity.kind, OpenEntityKind::Callable { .. }))
            {
                continue;
            }
            let parent_class_fqn =
                open_entities
                    .iter()
                    .rev()
                    .find_map(|entity| match &entity.kind {
                        OpenEntityKind::Class => Some(entity.node.fqn.clone()),
                        OpenEntityKind::Callable { .. } => None,
                    });
            let kind = if let Some(parent_fqn) = &parent_class_fqn {
                NodeKind::Method {
                    parent_fqn: parent_fqn.clone(),
                }
            } else {
                NodeKind::Function
            };
            let fqn = if let Some(parent_fqn) = &parent_class_fqn {
                format!("{parent_fqn}.{name}")
            } else {
                format!("{module_fqn}.{name}")
            };
            open_entities.push(OpenEntity {
                node: Node {
                    id: NodeId::new("python", &fqn),
                    language: "python".to_string(),
                    name: name.to_string(),
                    fqn,
                    kind,
                    file_path: relative_path.clone(),
                    location: SourceLocation {
                        start_line: line_number,
                        end_line: line_number,
                    },
                    status: NodeStatus::Active,
                    signature,
                    body_hash: String::new(),
                    introduced_at_version: version,
                },
                indent,
                kind: OpenEntityKind::Callable {
                    module_fqn: module_fqn.clone(),
                    parent_class_fqn,
                },
                body_lines: Vec::new(),
                call_tokens: Vec::new(),
                aliases: aliases.clone(),
                last_line: line_number,
            });
            continue;
        }

        if significant
            && !trimmed.starts_with('@')
            && !trimmed.starts_with("class ")
            && !trimmed.starts_with("def ")
            && !trimmed.starts_with("async def ")
        {
            if let Some(callable) = open_entities
                .iter_mut()
                .rev()
                .find(|entity| matches!(entity.kind, OpenEntityKind::Callable { .. }))
            {
                callable.call_tokens.extend(extract_call_tokens(trimmed));
            }
        }
    }

    for entity in open_entities.drain(..).rev() {
        close_entity(entity, total_lines.max(1), &mut nodes, &mut callsites);
    }

    Ok(ParsedFile {
        nodes,
        callsites,
        import_records,
    })
}

fn close_entity(
    mut entity: OpenEntity,
    end_line: u32,
    nodes: &mut Vec<Node>,
    callsites: &mut Vec<CallsiteRecord>,
) {
    entity.node.location.end_line = end_line.max(entity.node.location.start_line);
    entity.node.body_hash = hash_normalized_lines(&entity.body_lines);
    match entity.kind {
        OpenEntityKind::Class => nodes.push(entity.node),
        OpenEntityKind::Callable {
            module_fqn,
            parent_class_fqn,
        } => {
            for token in entity.call_tokens {
                callsites.push(CallsiteRecord {
                    source: entity.node.id.clone(),
                    module_fqn: module_fqn.clone(),
                    parent_class_fqn: parent_class_fqn.clone(),
                    token,
                    aliases: entity.aliases.clone(),
                });
            }
            nodes.push(entity.node);
        }
    }
}

fn resolve_import_target<'a>(
    target_fqn: &str,
    node_by_fqn: &'a HashMap<String, Node>,
) -> Option<&'a Node> {
    if let Some(node) = node_by_fqn.get(target_fqn) {
        return Some(node);
    }
    target_fqn
        .rsplit_once('.')
        .and_then(|(module_fqn, _)| node_by_fqn.get(module_fqn))
}

fn resolve_callsite<'a>(
    callsite: &CallsiteRecord,
    node_by_fqn: &'a HashMap<String, Node>,
    node_by_id: &'a HashMap<NodeId, Node>,
    short_name_index: &HashMap<String, Vec<NodeId>>,
) -> Option<(&'a Node, EdgeKind, f32, ResolutionMethod)> {
    let token = callsite.token.as_str();

    if let Some(parent_fqn) = &callsite.parent_class_fqn {
        if let Some(method_name) = token.strip_prefix("self.") {
            let candidate = format!("{parent_fqn}.{method_name}");
            if let Some(node) = node_by_fqn.get(&candidate) {
                return Some((node, EdgeKind::Calls, 0.75, ResolutionMethod::TypeInferred));
            }
        }
    }

    if !token.contains('.') {
        if let Some(parent_fqn) = &callsite.parent_class_fqn {
            let candidate = format!("{parent_fqn}.{token}");
            if let Some(node) = node_by_fqn.get(&candidate) {
                return Some((node, EdgeKind::Calls, 0.75, ResolutionMethod::TypeInferred));
            }
        }

        if let Some(alias_target) = callsite.aliases.get(token) {
            if let Some(node) = node_by_fqn.get(alias_target) {
                return Some((
                    node,
                    edge_kind_for_target(node),
                    0.55,
                    ResolutionMethod::Heuristic,
                ));
            }
        }

        let same_module = format!("{}.{}", callsite.module_fqn, token);
        if let Some(node) = node_by_fqn.get(&same_module) {
            return Some((
                node,
                edge_kind_for_target(node),
                0.55,
                ResolutionMethod::Heuristic,
            ));
        }

        if let Some(matches) = short_name_index.get(token) {
            if matches.len() == 1 {
                if let Some(node) = node_by_id.get(&matches[0]) {
                    return Some((
                        node,
                        edge_kind_for_target(node),
                        0.35,
                        ResolutionMethod::Heuristic,
                    ));
                }
            }
        }
        return None;
    }

    let (head, tail) = token.split_once('.')?;
    if let Some(alias_target) = callsite.aliases.get(head) {
        let candidate = format!("{alias_target}.{tail}");
        if let Some(node) = node_by_fqn.get(&candidate) {
            return Some((
                node,
                edge_kind_for_target(node),
                0.55,
                ResolutionMethod::Heuristic,
            ));
        }
    }

    node_by_fqn.get(token).map(|node| {
        (
            node,
            edge_kind_for_target(node),
            0.55,
            ResolutionMethod::Heuristic,
        )
    })
}

fn edge_kind_for_target(node: &Node) -> EdgeKind {
    match node.kind {
        NodeKind::Class | NodeKind::Struct => EdgeKind::Instantiates,
        _ => EdgeKind::Calls,
    }
}

fn parse_import_aliases(trimmed: &str) -> Vec<(String, String)> {
    if let Some(rest) = trimmed.strip_prefix("import ") {
        return split_top_level(rest)
            .into_iter()
            .filter_map(|part| {
                let (module_name, alias) = parse_import_part(&part)?;
                let alias = alias.unwrap_or_else(|| {
                    module_name
                        .rsplit('.')
                        .next()
                        .unwrap_or(module_name.as_str())
                        .to_string()
                });
                Some((alias, module_name))
            })
            .collect();
    }

    if let Some(rest) = trimmed.strip_prefix("from ") {
        if let Some((module_name, imports)) = rest.split_once(" import ") {
            return split_top_level(imports)
                .into_iter()
                .filter_map(|part| {
                    let (symbol_name, alias) = parse_import_part(&part)?;
                    if symbol_name == "*" {
                        return None;
                    }
                    let alias = alias.unwrap_or_else(|| symbol_name.clone());
                    Some((alias, format!("{}.{}", module_name.trim(), symbol_name)))
                })
                .collect();
        }
    }

    Vec::new()
}

fn parse_import_part(part: &str) -> Option<(String, Option<String>)> {
    let item = part.trim();
    if item.is_empty() {
        return None;
    }
    if let Some((name, alias)) = item.split_once(" as ") {
        return Some((name.trim().to_string(), Some(alias.trim().to_string())));
    }
    Some((item.to_string(), None))
}

fn parse_class_name(trimmed: &str) -> Option<&str> {
    static CLASS_RE: OnceLock<Regex> = OnceLock::new();
    let regex = CLASS_RE.get_or_init(|| Regex::new(r"^class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
    regex
        .captures(trimmed)
        .and_then(|captures| captures.get(1).map(|value| value.as_str()))
}

fn parse_def_signature(trimmed: &str) -> Option<(&str, Signature)> {
    static DEF_RE: OnceLock<Regex> = OnceLock::new();
    let regex = DEF_RE.get_or_init(|| {
        Regex::new(
            r"^(?:async\s+def|def)\s+([A-Za-z_][A-Za-z0-9_]*)\s*\((.*)\)\s*(?:->\s*([^:]+))?:",
        )
        .unwrap()
    });
    let captures = regex.captures(trimmed)?;
    let name = captures.get(1)?.as_str();
    let params = captures.get(2).map(|value| value.as_str()).unwrap_or("");
    let return_type = captures
        .get(3)
        .map(|value| value.as_str().trim().to_string());
    let is_async = trimmed.starts_with("async def ");
    Some((name, build_signature(params, return_type, is_async)))
}

fn build_signature(params: &str, return_type: Option<String>, is_async: bool) -> Signature {
    let parts = split_top_level(params);
    let mut partial_params = Vec::new();
    let mut typed_params = Vec::new();
    let mut fully_typed = return_type.is_some() || parts.is_empty();

    for part in parts {
        let item = part.trim();
        if item.is_empty() || item == "/" || item == "*" {
            continue;
        }
        let has_default = split_top_level_assignment(item).1.is_some();
        let left = split_top_level_assignment(item)
            .0
            .trim()
            .trim_start_matches('*')
            .trim();
        let (name, annotation) = if let Some((name, annotation)) = split_top_level_annotation(left)
        {
            (name.trim().to_string(), Some(annotation.trim().to_string()))
        } else {
            fully_typed = false;
            (left.to_string(), None)
        };

        partial_params.push(PartialParam {
            name: name.clone(),
            type_annotation: annotation.clone(),
            has_default,
        });
        if let Some(annotation) = annotation {
            typed_params.push(crate::schema::TypedParam {
                name,
                type_annotation: annotation,
                has_default,
            });
        } else {
            fully_typed = false;
        }
    }

    if fully_typed {
        Signature::Typed {
            params: typed_params,
            return_type: return_type.unwrap_or_else(|| "None".to_string()),
            is_async,
            is_generic: params.contains('['),
        }
    } else {
        Signature::PartiallyTyped {
            params: partial_params,
            return_type,
            is_async,
        }
    }
}

fn extract_call_tokens(trimmed: &str) -> Vec<String> {
    static CALL_RE: OnceLock<Regex> = OnceLock::new();
    let regex = CALL_RE.get_or_init(|| Regex::new(r"([A-Za-z_][A-Za-z0-9_\.]*)\s*\(").unwrap());
    regex
        .captures_iter(trimmed)
        .filter_map(|captures| captures.get(1).map(|value| value.as_str().to_string()))
        .filter(|token| {
            !matches!(
                token.as_str(),
                "if" | "for" | "while" | "return" | "with" | "match" | "except"
            )
        })
        .collect()
}

fn hash_normalized_lines(lines: &[String]) -> String {
    let mut hasher = Sha256::new();
    let normalized = lines
        .iter()
        .map(|line| {
            strip_comment(line)
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>()
        })
        .collect::<String>();
    hasher.update(normalized);
    hex::encode(hasher.finalize())
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#')
        .map(|(prefix, _)| prefix)
        .unwrap_or(line)
}

fn short_token(token: &str) -> String {
    token.rsplit('.').next().unwrap_or(token).to_string()
}

fn split_top_level(input: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in input.chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                items.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        items.push(current.trim().to_string());
    }
    items
}

fn split_top_level_assignment(input: &str) -> (&str, Option<&str>) {
    split_on_top_level_char(input, '=').unwrap_or((input, None))
}

fn split_top_level_annotation(input: &str) -> Option<(&str, &str)> {
    split_on_top_level_char(input, ':').map(|(left, right)| (left, right.unwrap_or("")))
}

fn split_on_top_level_char(input: &str, delimiter: char) -> Option<(&str, Option<&str>)> {
    let mut depth = 0i32;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ if ch == delimiter && depth == 0 => {
                return Some((&input[..index], Some(&input[index + ch.len_utf8()..])));
            }
            _ => {}
        }
    }
    None
}

fn module_fqn_for_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let mut components: Vec<String> = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .map(|component| component.trim_end_matches(".py").replace('-', "_"))
        .collect();
    if components.last().map(String::as_str) == Some("__init__") {
        components.pop();
    }
    if components.is_empty() {
        return root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project")
            .replace('-', "_");
    }
    components.join(".")
}

fn normalize_rel_path(root: &Path, path: &Path) -> io::Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    Ok(relative
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string())
}

fn leading_spaces(line: &str) -> usize {
    line.chars()
        .take_while(|character| *character == ' ')
        .count()
}

fn should_visit_path(path: &Path) -> bool {
    static IGNORED: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let ignored = IGNORED.get_or_init(|| {
        HashSet::from([
            ".git",
            "__pycache__",
            ".venv",
            "node_modules",
            "target",
            "dist",
            "build",
        ])
    });
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|name| !ignored.contains(name))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn scanner_extracts_python_entities_and_edges() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(
            temp_dir.path().join("app.py"),
            r#"
import helpers as helpers

class Parser:
    def parse(self, value: str) -> str:
        cleaned = helpers.clean(value)
        return self.render(cleaned)

    def render(self, value):
        return value

def run(config):
    parser = Parser()
    return parser.parse(config)
"#,
        )
        .unwrap();
        fs::write(
            temp_dir.path().join("helpers.py"),
            "def clean(value: str) -> str:\n    return value.strip()\n",
        )
        .unwrap();

        let parsed = scan_python_project(temp_dir.path(), GraphVersion::initial()).unwrap();
        assert!(
            parsed
                .nodes
                .iter()
                .any(|node| node.fqn == "app.Parser.parse")
        );
        assert!(parsed.nodes.iter().any(|node| node.fqn == "helpers.clean"));
        assert!(
            parsed
                .edge_drafts
                .iter()
                .any(|edge| edge.kind == EdgeKind::Imports)
        );
        assert!(
            parsed
                .edge_drafts
                .iter()
                .any(|edge| edge.kind == EdgeKind::Calls)
        );
        assert!(
            parsed
                .edge_drafts
                .iter()
                .any(|edge| edge.kind == EdgeKind::Instantiates)
        );
    }

    #[test]
    fn scanner_tracks_unresolved_calls() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(
            temp_dir.path().join("app.py"),
            "def run(value):\n    return missing_call(value)\n",
        )
        .unwrap();
        let parsed = scan_python_project(temp_dir.path(), GraphVersion::initial()).unwrap();
        assert_eq!(parsed.unresolved_calls.len(), 1);
        assert_eq!(
            parsed.unresolved_calls.values().next().unwrap(),
            &vec!["missing_call".to_string()]
        );
    }
}
