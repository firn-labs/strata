//! Flow execution with a step-by-step run trace (WORKFLOW-05).
//!
//! [`execute`] walks a flow graph from a trigger node and records every
//! visited node into a [`RunRecord`]: what ran, with which configuration,
//! which branch a condition chose and why, and when each step started and
//! finished. The trace is the deliverable — it is what makes runs
//! diagnosable after the fact and lets processing status be communicated
//! to third parties.
//!
//! Step nodes do not yet perform real actions: calling the core server's
//! API from a step lands with the WORKFLOW-08 integration. Until then the
//! engine records each step as executed with its configuration, so the
//! logging machinery, traversal, and condition decisions are real today
//! and the action executor slots in behind the same trace.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::flow::{FlowDefinition, FlowNode, NodeKind};

/// Ceiling on recorded steps per run — a guard against cyclic graphs, not a
/// tuning knob. Hitting it fails the run with a diagnosable trace instead of
/// looping forever.
pub const MAX_STEPS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub Uuid);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Failed,
}

/// One complete execution of a flow: trigger, inputs, every step in order,
/// and the overall outcome (WORKFLOW-05).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: RunId,
    pub flow: crate::flow::FlowId,
    /// Who (or, later, which event source) started the run.
    pub triggered_by: String,
    /// The trigger node the run entered through.
    pub trigger_node: String,
    /// Input payload handed to the trigger, available to condition nodes.
    pub input: Value,
    pub status: RunStatus,
    /// Present when the run failed: what went wrong, pointing at the step
    /// trace for the details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
    pub steps: Vec<StepRecord>,
}

/// One visited node in a run, in execution order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    /// Position within the run, starting at 0.
    pub seq: usize,
    pub node: String,
    pub kind: NodeKind,
    /// The node configuration the engine acted on, copied into the trace so
    /// the log stays meaningful even after the flow definition is edited.
    pub config: Value,
    pub outcome: StepOutcome,
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum StepOutcome {
    /// A trigger node accepted the run's input.
    Triggered,
    /// A step node ran (recorded only, until WORKFLOW-08 wires real actions).
    Executed,
    /// A condition node evaluated its expression and chose a branch.
    Decided { branch: String, reason: String },
    /// The node could not be executed; the run stops here as failed.
    Failed { error: String },
}

/// Execute `flow` from `trigger_node` and return the full trace.
///
/// Traversal is depth-first following edge definition order; a condition
/// node only follows edges whose `branch` label matches its decision.
pub fn execute(flow: &FlowDefinition, trigger_node: &str, input: Value, actor: &str) -> RunRecord {
    let started_at = Timestamp::now();
    let mut steps = Vec::new();
    let mut error = None;

    // Nodes still to visit, depth-first: pushed in reverse edge order so the
    // first-defined edge is executed first.
    let mut pending = vec![trigger_node.to_owned()];

    while let Some(node_id) = pending.pop() {
        if steps.len() >= MAX_STEPS {
            error = Some(format!(
                "run exceeded {MAX_STEPS} steps — the flow graph likely contains a cycle"
            ));
            break;
        }

        // Registration validated all edge endpoints, so the node exists.
        let node = flow.node(&node_id).expect("edge points at a known node");
        let step_started = Timestamp::now();
        let outcome = run_node(node, &input);

        let failed = matches!(outcome, StepOutcome::Failed { .. });
        let chosen_branch = match &outcome {
            StepOutcome::Decided { branch, .. } => Some(branch.clone()),
            _ => None,
        };

        steps.push(StepRecord {
            seq: steps.len(),
            node: node_id.clone(),
            kind: node.kind,
            config: node.config.clone(),
            outcome,
            started_at: step_started,
            finished_at: Timestamp::now(),
        });

        if failed {
            error = Some(format!("step {node_id:?} failed; see its trace entry"));
            break;
        }

        let followers: Vec<_> = flow
            .edges_from(&node_id)
            .filter(|edge| match (&chosen_branch, &edge.branch) {
                // Conditions follow only their chosen branch; unlabeled
                // edges out of a condition are never taken.
                (Some(chosen), Some(label)) => chosen == label,
                (Some(_), None) => false,
                // Everything else follows all outgoing edges.
                (None, _) => true,
            })
            .map(|edge| edge.to.clone())
            .collect();
        pending.extend(followers.into_iter().rev());
    }

    RunRecord {
        id: RunId::new(),
        flow: flow.id,
        triggered_by: actor.to_owned(),
        trigger_node: trigger_node.to_owned(),
        input,
        status: if error.is_some() {
            RunStatus::Failed
        } else {
            RunStatus::Completed
        },
        error,
        started_at,
        finished_at: Timestamp::now(),
        steps,
    }
}

fn run_node(node: &FlowNode, input: &Value) -> StepOutcome {
    match node.kind {
        NodeKind::Trigger => StepOutcome::Triggered,
        NodeKind::Step => StepOutcome::Executed,
        NodeKind::Condition => match decide(&node.config, input) {
            Ok((branch, reason)) => StepOutcome::Decided { branch, reason },
            Err(error) => StepOutcome::Failed { error },
        },
    }
}

/// A condition node's configuration: compare one field of the run input
/// against a constant. Deliberately small — richer expressions can extend
/// this without changing the trace format, which only records the decision.
#[derive(Debug, Deserialize)]
struct Condition {
    /// Key into the run input object.
    input: String,
    operator: Operator,
    #[serde(default)]
    value: Value,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operator {
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    Exists,
}

/// Evaluate a condition, returning the branch label (`"true"`/`"false"`)
/// plus a human-readable reason recorded in the trace.
fn decide(config: &Value, input: &Value) -> Result<(String, String), String> {
    let condition: Condition = serde_json::from_value(config.clone())
        .map_err(|e| format!("invalid condition config: {e}"))?;

    let field = input.get(&condition.input);

    let verdict = match condition.operator {
        Operator::Exists => field.is_some_and(|v| !v.is_null()),
        _ => {
            let Some(actual) = field else {
                return Err(format!(
                    "condition reads input field {:?}, which the run input does not contain",
                    condition.input
                ));
            };
            match condition.operator {
                Operator::Equals => *actual == condition.value,
                Operator::NotEquals => *actual != condition.value,
                Operator::GreaterThan | Operator::LessThan => {
                    let (Some(a), Some(b)) = (actual.as_f64(), condition.value.as_f64()) else {
                        return Err(format!(
                            "condition compares {:?} numerically, but got {actual} vs {}",
                            condition.input, condition.value
                        ));
                    };
                    if matches!(condition.operator, Operator::GreaterThan) {
                        a > b
                    } else {
                        a < b
                    }
                }
                Operator::Exists => unreachable!("handled above"),
            }
        }
    };

    let reason = format!(
        "input.{} = {} {} {}",
        condition.input,
        field.unwrap_or(&Value::Null),
        match condition.operator {
            Operator::Equals => "equals",
            Operator::NotEquals => "not_equals",
            Operator::GreaterThan => "greater_than",
            Operator::LessThan => "less_than",
            Operator::Exists => "exists",
        },
        condition.value
    );
    Ok((verdict.to_string(), reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{FlowEdge, FlowId, FlowNode};
    use serde_json::json;

    fn node(id: &str, kind: NodeKind, config: Value) -> FlowNode {
        FlowNode {
            id: id.into(),
            kind,
            config,
        }
    }

    fn edge(from: &str, to: &str, branch: Option<&str>) -> FlowEdge {
        FlowEdge {
            from: from.into(),
            to: to.into(),
            branch: branch.map(Into::into),
        }
    }

    fn branching_flow() -> FlowDefinition {
        FlowDefinition {
            id: FlowId::new(),
            name: "invoice routing".into(),
            owner: "accounting".into(),
            nodes: vec![
                node("in", NodeKind::Trigger, json!({})),
                node(
                    "big?",
                    NodeKind::Condition,
                    json!({"input": "amount", "operator": "greater_than", "value": 1000}),
                ),
                node(
                    "approve",
                    NodeKind::Step,
                    json!({"action": "request_approval"}),
                ),
                node("file", NodeKind::Step, json!({"action": "move"})),
            ],
            edges: vec![
                edge("in", "big?", None),
                edge("big?", "approve", Some("true")),
                edge("big?", "file", Some("false")),
            ],
        }
    }

    #[test]
    fn a_run_traces_every_step_with_timestamps() {
        let flow = branching_flow();
        let run = execute(&flow, "in", json!({"amount": 5000}), "alice");

        assert_eq!(run.status, RunStatus::Completed);
        assert_eq!(run.triggered_by, "alice");
        let visited: Vec<_> = run.steps.iter().map(|s| s.node.as_str()).collect();
        assert_eq!(visited, ["in", "big?", "approve"]);
        for (i, step) in run.steps.iter().enumerate() {
            assert_eq!(step.seq, i);
            assert!(step.started_at <= step.finished_at);
        }
        assert!(run.started_at <= run.steps[0].started_at);
        assert!(run.finished_at >= run.steps[2].finished_at);
    }

    #[test]
    fn conditions_record_their_decision_and_take_only_the_chosen_branch() {
        let flow = branching_flow();
        let run = execute(&flow, "in", json!({"amount": 20}), "alice");

        let visited: Vec<_> = run.steps.iter().map(|s| s.node.as_str()).collect();
        assert_eq!(visited, ["in", "big?", "file"]);
        let StepOutcome::Decided { branch, reason } = &run.steps[1].outcome else {
            panic!("condition step must record a decision");
        };
        assert_eq!(branch, "false");
        assert!(reason.contains("amount"));
        assert!(reason.contains("greater_than"));
    }

    #[test]
    fn a_condition_on_a_missing_input_fails_the_run_diagnosably() {
        let flow = branching_flow();
        let run = execute(&flow, "in", json!({"vendor": "acme"}), "alice");

        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.error.as_deref().unwrap().contains("big?"));
        let StepOutcome::Failed { error } = &run.steps.last().unwrap().outcome else {
            panic!("failing step must record its error");
        };
        assert!(error.contains("amount"));
    }

    #[test]
    fn cyclic_graphs_hit_the_step_ceiling_instead_of_hanging() {
        let flow = FlowDefinition {
            id: FlowId::new(),
            name: "loop".into(),
            owner: "qa".into(),
            nodes: vec![
                node("in", NodeKind::Trigger, json!({})),
                node("again", NodeKind::Step, json!({})),
            ],
            edges: vec![edge("in", "again", None), edge("again", "again", None)],
        };
        let run = execute(&flow, "in", json!({}), "alice");
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.steps.len(), MAX_STEPS);
        assert!(run.error.unwrap().contains("cycle"));
    }

    #[test]
    fn fan_out_follows_edges_in_definition_order() {
        let flow = FlowDefinition {
            id: FlowId::new(),
            name: "fan out".into(),
            owner: "qa".into(),
            nodes: vec![
                node("in", NodeKind::Trigger, json!({})),
                node("first", NodeKind::Step, json!({})),
                node("second", NodeKind::Step, json!({})),
            ],
            edges: vec![edge("in", "first", None), edge("in", "second", None)],
        };
        let run = execute(&flow, "in", json!({}), "alice");
        let visited: Vec<_> = run.steps.iter().map(|s| s.node.as_str()).collect();
        assert_eq!(visited, ["in", "first", "second"]);
    }
}
