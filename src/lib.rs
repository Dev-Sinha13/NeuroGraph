mod errors;
mod graph;
mod parser;
mod rename;
mod schema;
mod validator;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde::de::DeserializeOwned;

use crate::errors::ToolCallError;
use crate::graph::GraphEngine;
use crate::schema::{ConfidenceConfig, EdgeDraft, GraphSnapshot, Node, OverlayReview};

#[pyclass(name = "GraphEngine")]
struct PyGraphEngine {
    inner: GraphEngine,
}

fn parse_json<T: DeserializeOwned>(payload: &str, context: &str) -> PyResult<T> {
    serde_json::from_str(payload).map_err(|error| {
        PyValueError::new_err(
            ToolCallError::invalid_schema(format!("{context}: {error}")).to_json(),
        )
    })
}

fn raise_tool_error(error: ToolCallError) -> PyErr {
    PyValueError::new_err(error.to_json())
}

#[pymethods]
impl PyGraphEngine {
    #[new]
    #[pyo3(signature = (confidence_config_json = None))]
    fn new(confidence_config_json: Option<String>) -> PyResult<Self> {
        let confidence_config = match confidence_config_json {
            Some(payload) => parse_json::<ConfidenceConfig>(&payload, "confidence_config")?,
            None => ConfidenceConfig::default(),
        };
        Ok(Self {
            inner: GraphEngine::new(confidence_config),
        })
    }

    fn startup_warning(&self) -> Option<String> {
        self.inner.confidence_config().startup_warning()
    }

    fn current_version(&self) -> u64 {
        self.inner.current_version().value()
    }

    fn increment_version(&mut self) -> u64 {
        self.inner.increment_version().value()
    }

    fn confidence_config_json(&self) -> PyResult<String> {
        serde_json::to_string_pretty(self.inner.confidence_config())
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn set_confidence_config(&mut self, payload: String) -> PyResult<()> {
        let config = parse_json::<ConfidenceConfig>(&payload, "confidence_config")?;
        self.inner.set_confidence_config(config);
        Ok(())
    }

    fn upsert_node(&mut self, payload: String) -> PyResult<String> {
        let node = parse_json::<Node>(&payload, "node")?;
        let stored = self
            .inner
            .upsert_node(node)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        serde_json::to_string_pretty(&stored)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn write_edge(&mut self, payload: String) -> PyResult<String> {
        let edge = parse_json::<EdgeDraft>(&payload, "edge")?;
        let stored = self
            .inner
            .write_edge(edge)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        serde_json::to_string_pretty(&stored)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn sync_python_project(&mut self, root: String) -> PyResult<String> {
        let report = self
            .inner
            .sync_python_project(&root)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        serde_json::to_string_pretty(&report)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn save_graph_state(&self, path: String) -> PyResult<()> {
        self.inner
            .save_to_path(std::path::Path::new(&path))
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn load_graph_state(&mut self, path: String) -> PyResult<String> {
        self.inner
            .load_from_path(std::path::Path::new(&path))
            .and_then(|state| {
                serde_json::to_string_pretty(&state).map_err(|error| error.to_string())
            })
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn analyze_diff(&self, diff_text: String) -> PyResult<String> {
        self.inner
            .analyze_diff(&diff_text)
            .and_then(|analysis| {
                serde_json::to_string_pretty(&analysis)
                    .map_err(|error| ToolCallError::invalid_schema(error.to_string()))
            })
            .map_err(raise_tool_error)
    }

    fn create_overlay_review(&self, pr_identifier: String, diff_text: String) -> PyResult<String> {
        self.inner
            .create_overlay_review(pr_identifier, &diff_text)
            .and_then(|overlay| {
                serde_json::to_string_pretty(&overlay)
                    .map_err(|error| ToolCallError::invalid_schema(error.to_string()))
            })
            .map_err(raise_tool_error)
    }

    fn deprecate_node(&mut self, node_id: String, payload: String) -> PyResult<String> {
        let metadata = parse_json(&payload, "deprecated_status")?;
        let node = self
            .inner
            .deprecate_node(&node_id, metadata)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        serde_json::to_string_pretty(&node)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn mark_node_deleted_in_overlay(&mut self, node_id: String) -> PyResult<()> {
        self.inner
            .mark_node_deleted_in_overlay(&node_id)
            .map_err(raise_tool_error)
    }

    #[pyo3(signature = (node_id, escalation_confidence_threshold, max_nodes = 25))]
    fn get_subgraph(
        &self,
        node_id: String,
        escalation_confidence_threshold: f32,
        max_nodes: usize,
    ) -> PyResult<String> {
        self.inner
            .get_subgraph(&node_id, escalation_confidence_threshold, max_nodes)
            .and_then(|summary| {
                serde_json::to_string_pretty(&summary)
                    .map_err(|error| ToolCallError::invalid_schema(error.to_string()))
            })
            .map_err(raise_tool_error)
    }

    #[pyo3(signature = (overlay_payload, node_id, escalation_confidence_threshold, max_nodes = 25))]
    fn get_overlay_subgraph(
        &self,
        overlay_payload: String,
        node_id: String,
        escalation_confidence_threshold: f32,
        max_nodes: usize,
    ) -> PyResult<String> {
        let overlay = parse_json::<OverlayReview>(&overlay_payload, "overlay_review")?;
        self.inner
            .get_overlay_subgraph(
                &overlay,
                &node_id,
                escalation_confidence_threshold,
                max_nodes,
            )
            .and_then(|summary| {
                serde_json::to_string_pretty(&summary)
                    .map_err(|error| ToolCallError::invalid_schema(error.to_string()))
            })
            .map_err(raise_tool_error)
    }

    fn get_node_detail(&self, node_id: String) -> PyResult<String> {
        self.inner
            .get_node_detail(&node_id)
            .and_then(|node| {
                serde_json::to_string_pretty(&node)
                    .map_err(|error| ToolCallError::invalid_schema(error.to_string()))
            })
            .map_err(raise_tool_error)
    }

    fn get_overlay_node_detail(
        &self,
        overlay_payload: String,
        node_id: String,
    ) -> PyResult<String> {
        let overlay = parse_json::<OverlayReview>(&overlay_payload, "overlay_review")?;
        self.inner
            .get_overlay_node_detail(&overlay, &node_id)
            .and_then(|node| {
                serde_json::to_string_pretty(&node)
                    .map_err(|error| ToolCallError::invalid_schema(error.to_string()))
            })
            .map_err(raise_tool_error)
    }

    fn get_unresolved_calls(&self, node_id: String) -> PyResult<String> {
        self.inner
            .get_unresolved_calls(&node_id)
            .and_then(|calls| {
                serde_json::to_string_pretty(&calls)
                    .map_err(|error| ToolCallError::invalid_schema(error.to_string()))
            })
            .map_err(raise_tool_error)
    }

    fn create_snapshot(&self, pr_identifier: String) -> PyResult<String> {
        let snapshot = self.inner.create_snapshot(pr_identifier);
        serde_json::to_string_pretty(&snapshot)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn snapshot_with_live_version(
        &self,
        payload: String,
        current_baseline_version: u64,
    ) -> PyResult<String> {
        let mut snapshot = parse_json::<GraphSnapshot>(&payload, "graph_snapshot")?;
        snapshot.current_baseline_version = current_baseline_version.into();
        serde_json::to_string_pretty(&snapshot)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn snapshot_is_stale(&self, payload: String, current_baseline_version: u64) -> PyResult<bool> {
        let mut snapshot = parse_json::<GraphSnapshot>(&payload, "graph_snapshot")?;
        snapshot.current_baseline_version = current_baseline_version.into();
        Ok(snapshot.is_stale())
    }

    fn detect_rename(
        &self,
        deprecated_node_id: String,
        candidate_node_id: String,
    ) -> PyResult<String> {
        self.inner
            .detect_rename(&deprecated_node_id, &candidate_node_id)
            .and_then(|candidate| {
                serde_json::to_string_pretty(&candidate)
                    .map_err(|error| ToolCallError::invalid_schema(error.to_string()))
            })
            .map_err(raise_tool_error)
    }

    fn apply_rename(
        &mut self,
        deprecated_node_id: String,
        candidate_node_id: String,
    ) -> PyResult<String> {
        self.inner
            .apply_rename(&deprecated_node_id, &candidate_node_id)
            .and_then(|candidate| {
                serde_json::to_string_pretty(&candidate)
                    .map_err(|error| ToolCallError::invalid_schema(error.to_string()))
            })
            .map_err(raise_tool_error)
    }
}

#[pymodule]
fn neurograph_engine(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyGraphEngine>()?;
    Ok(())
}
