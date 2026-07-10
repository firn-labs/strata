//! Flow definitions — the data model behind the visual editor.
//!
//! A flow is a directed graph: trigger nodes start an execution, step nodes
//! transform or route documents, and edges define the order. The frontend's
//! visual editor produces exactly this structure as JSON, so the same
//! definition supports the graphical, textual, and semantic representations
//! required of workflows.

#![allow(dead_code)] // Consumed once the engine's execution loop lands.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FlowId(pub Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDefinition {
    pub id: FlowId,
    pub name: String,
    /// Department or team that owns and may edit this flow.
    pub owner: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: String,
    pub kind: NodeKind,
    /// Node-specific configuration, defined by the node's kind.
    #[serde(default)]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    #[test]
    fn flow_definition_roundtrips_through_json() {
        let flow = FlowDefinition {
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
        };

        let json = serde_json::to_string(&flow).unwrap();
        let back: FlowDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(back.nodes.len(), 2);
        assert_eq!(back.edges[0].from, "upload");
    }
}
