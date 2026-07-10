//! Flow definitions — the data model behind the visual editor.
//!
//! A flow is a directed graph: trigger nodes start an execution, step nodes
//! transform or route documents, and edges define the order. The frontend's
//! visual editor produces exactly this structure as JSON, so the same
//! definition supports the graphical, textual, and semantic representations
//! required of workflows.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FlowId(pub Uuid);

impl FlowId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for FlowId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for FlowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDefinition {
    pub id: FlowId,
    pub name: String,
    /// Department or team that owns and may edit this flow.
    pub owner: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

impl FlowDefinition {
    pub fn node(&self, id: &str) -> Option<&FlowNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// Edges leaving `node`, in definition order — the engine follows them
    /// in exactly this order, so the visual layout stays authoritative.
    pub fn edges_from<'a>(&'a self, node: &'a str) -> impl Iterator<Item = &'a FlowEdge> {
        self.edges.iter().filter(move |edge| edge.from == node)
    }

    /// Structural validation applied at registration time, so the engine
    /// never has to execute a graph with dangling references.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for node in &self.nodes {
            if node.id.trim().is_empty() {
                return Err("node ids must not be empty".into());
            }
            if !seen.insert(node.id.as_str()) {
                return Err(format!("duplicate node id {:?}", node.id));
            }
        }
        if !self.nodes.iter().any(|n| n.kind == NodeKind::Trigger) {
            return Err("a flow needs at least one trigger node".into());
        }
        for edge in &self.edges {
            for end in [&edge.from, &edge.to] {
                if !seen.contains(end.as_str()) {
                    return Err(format!("edge references unknown node {end:?}"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: String,
    pub kind: NodeKind,
    /// Node-specific configuration, defined by the node's kind.
    #[serde(default)]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// Starts an execution (document uploaded, schedule fired, ...).
    Trigger,
    /// Performs an action (classify, move, OCR, notify, call API, ...).
    Step,
    /// Routes to different branches based on a condition.
    Condition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    /// Optional branch label for edges leaving a condition node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_node_flow() -> FlowDefinition {
        FlowDefinition {
            id: FlowId(Uuid::new_v4()),
            name: "Incoming invoices".into(),
            owner: "accounting".into(),
            nodes: vec![
                FlowNode {
                    id: "upload".into(),
                    kind: NodeKind::Trigger,
                    config: serde_json::json!({"source": "capture"}),
                },
                FlowNode {
                    id: "file-it".into(),
                    kind: NodeKind::Step,
                    config: serde_json::json!({"action": "move", "target": "invoices/"}),
                },
            ],
            edges: vec![FlowEdge {
                from: "upload".into(),
                to: "file-it".into(),
                branch: None,
            }],
        }
    }

    #[test]
    fn flow_definition_roundtrips_through_json() {
        let flow = two_node_flow();
        let json = serde_json::to_string(&flow).unwrap();
        let back: FlowDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(back.nodes.len(), 2);
        assert_eq!(back.edges[0].from, "upload");
    }

    #[test]
    fn validation_accepts_a_well_formed_flow() {
        assert_eq!(two_node_flow().validate(), Ok(()));
    }

    #[test]
    fn validation_rejects_structural_defects() {
        let mut no_trigger = two_node_flow();
        no_trigger.nodes.remove(0);
        no_trigger.edges.clear();
        assert!(no_trigger.validate().unwrap_err().contains("trigger"));

        let mut dangling = two_node_flow();
        dangling.edges[0].to = "nowhere".into();
        assert!(dangling.validate().unwrap_err().contains("nowhere"));

        let mut duplicate = two_node_flow();
        duplicate.nodes[1].id = "upload".into();
        assert!(duplicate.validate().unwrap_err().contains("duplicate"));
    }
}
