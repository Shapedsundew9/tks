//! Per-node governance policy evaluation, permission validation,
//! and mutation pathway determination (WP-2.2, INV-5, D-57, D-81, TB-7.7).
//!
//! Enforces:
//! - Per-node governance policies: `AUTONOMOUS_ELABORATION`, `HUMAN_REVIEW_REQUIRED`, `LOCKED`.
//! - Absolute precedence of node-level policy over global caller permissions (INV-5).
//! - Pathway 1 (Autonomous Task Elaboration): permits verified agents to create non-normative
//!   execution tasks (`TASK`) directly in `ACTIVE` state under `AUTONOMOUS_ELABORATION` parents.
//! - Pathway 2 (Active Leaf Task Updates): row-level locked status updates on active tasks
//!   committing discrete audit events without touching draft revisions (D-30, D-57).
//! - Pathway 3 (Normative Requirement Proposals): immutability of active normative specifications
//!   (`REQUIREMENT`, `SPECIFICATION`), intercepting mutations into candidate `DRAFT` nodes (INV-2).
//! - Pathway 4 (Candidate Draft Evolution): intermediate edits append to ephemeral
//!   `attributes->'draft_revisions'` without polluting the audit ledger (D-16, D-61).

use serde::{Deserialize, Serialize};

use crate::storage::mutation::MutationError;

/// Per-node governance policy governing alteration and elaboration permissions (INV-5, D-81).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GovernancePolicy {
    /// External verified agents may elaborate execution tasks directly into `ACTIVE` state.
    AutonomousElaboration,
    /// Changes require supervisory staging approval; normative mutations create candidate drafts.
    HumanReviewRequired,
    /// Safety-critical locked node; all mutations and elaborations are strictly rejected.
    Locked,
}

impl GovernancePolicy {
    /// Returns the uppercase canonical string representation matching database CHECK constraints.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AutonomousElaboration => "AUTONOMOUS_ELABORATION",
            Self::HumanReviewRequired => "HUMAN_REVIEW_REQUIRED",
            Self::Locked => "LOCKED",
        }
    }
}

impl std::fmt::Display for GovernancePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for GovernancePolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_uppercase().as_str() {
            "AUTONOMOUS_ELABORATION" => Ok(Self::AutonomousElaboration),
            "HUMAN_REVIEW_REQUIRED" => Ok(Self::HumanReviewRequired),
            "LOCKED" => Ok(Self::Locked),
            other => Err(format!(
                "Unknown governance policy '{other}': expected AUTONOMOUS_ELABORATION, HUMAN_REVIEW_REQUIRED, or LOCKED"
            )),
        }
    }
}

impl Serialize for GovernancePolicy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for GovernancePolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<Self>().map_err(serde::de::Error::custom)
    }
}

/// Execution status states for active leaf tasks (`node_type = 'TASK'`) (D-57).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskStatus {
    /// Task has been created and is waiting to be claimed or started.
    Open,
    /// Implementation work is actively underway.
    InProgress,
    /// Execution is blocked by external dependencies or failed reverification.
    Blocked,
    /// Task execution has completed successfully.
    Completed,
}

impl TaskStatus {
    /// Returns the uppercase canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::InProgress => "IN_PROGRESS",
            Self::Blocked => "BLOCKED",
            Self::Completed => "COMPLETED",
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_uppercase().as_str() {
            "OPEN" => Ok(Self::Open),
            "IN_PROGRESS" => Ok(Self::InProgress),
            "BLOCKED" => Ok(Self::Blocked),
            "COMPLETED" => Ok(Self::Completed),
            other => Err(format!(
                "Unknown task status '{other}': expected OPEN, IN_PROGRESS, BLOCKED, or COMPLETED"
            )),
        }
    }
}

impl Serialize for TaskStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TaskStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<Self>().map_err(serde::de::Error::custom)
    }
}

/// Disambiguated mutation pathway classifications (D-57).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MutationPathway {
    /// Pathway 1: Autonomous task elaboration under `AUTONOMOUS_ELABORATION` parent node.
    AutonomousTaskElaboration,
    /// Pathway 2: In-place row-level locked status updates on active execution tasks (`TASK`).
    ActiveTaskUpdate,
    /// Pathway 3: Intercepted normative mutations on active requirements, creating candidate `DRAFT` entities.
    NormativeDraftProposal,
    /// Pathway 4: Ephemeral draft evolution appending to `attributes->'draft_revisions'`.
    CandidateDraftEvolution,
}

/// Requested governance operation being validated against node policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GovernanceAction {
    /// Sub-task elaboration under target parent node.
    ElaborateTask,
    /// Execution status transition on an active task.
    UpdateTaskStatus,
    /// Modification to requirement, specification, or node content.
    ProposeNormativeMutation,
    /// Intermediate edit on an unapproved candidate draft entity.
    MutateDraft,
}

/// Evaluates a proposed governance action against a node's policy, type, and lifecycle state.
///
/// # Errors
///
/// - Returns `MutationError::GovernanceLocked` if target node policy is `LOCKED`.
/// - Returns `MutationError::GovernanceRejected` if action is prohibited by policy or state.
pub fn evaluate_governance_action(
    policy: GovernancePolicy,
    action: GovernanceAction,
    node_type: &str,
    lifecycle_state: &str,
) -> Result<MutationPathway, MutationError> {
    // 1. Strict invariant: LOCKED nodes reject all mutations and elaborations unconditionally
    if policy == GovernancePolicy::Locked {
        return Err(MutationError::GovernanceLocked(
            "Node governance policy is LOCKED; operation rejected".to_string(),
        ));
    }

    match action {
        GovernanceAction::ElaborateTask => {
            if policy == GovernancePolicy::AutonomousElaboration {
                Ok(MutationPathway::AutonomousTaskElaboration)
            } else {
                Err(MutationError::GovernanceRejected(format!(
                    "Autonomous task elaboration requires parent governance policy 'AUTONOMOUS_ELABORATION' (current: '{policy}')"
                )))
            }
        }
        GovernanceAction::UpdateTaskStatus => {
            if node_type != "TASK" {
                return Err(MutationError::GovernanceRejected(format!(
                    "Cannot update task status on non-task node (node_type: '{node_type}')"
                )));
            }
            if lifecycle_state != "ACTIVE" {
                return Err(MutationError::GovernanceRejected(format!(
                    "Cannot update task status on non-active node (lifecycle_state: '{lifecycle_state}')"
                )));
            }
            Ok(MutationPathway::ActiveTaskUpdate)
        }
        GovernanceAction::ProposeNormativeMutation => {
            if lifecycle_state == "ACTIVE"
                && (node_type == "REQUIREMENT" || node_type == "SPECIFICATION")
            {
                Ok(MutationPathway::NormativeDraftProposal)
            } else if lifecycle_state == "DRAFT" {
                Ok(MutationPathway::CandidateDraftEvolution)
            } else {
                Ok(MutationPathway::NormativeDraftProposal)
            }
        }
        GovernanceAction::MutateDraft => {
            if lifecycle_state != "DRAFT" {
                return Err(MutationError::GovernanceRejected(format!(
                    "Draft mutation pathway only applies to nodes in 'DRAFT' state (current: '{lifecycle_state}')"
                )));
            }
            Ok(MutationPathway::CandidateDraftEvolution)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_governance_policy_parsing_and_display() {
        assert_eq!(
            "AUTONOMOUS_ELABORATION"
                .parse::<GovernancePolicy>()
                .unwrap(),
            GovernancePolicy::AutonomousElaboration
        );
        assert_eq!(
            "human_review_required".parse::<GovernancePolicy>().unwrap(),
            GovernancePolicy::HumanReviewRequired
        );
        assert_eq!(
            "Locked".parse::<GovernancePolicy>().unwrap(),
            GovernancePolicy::Locked
        );
        assert_eq!(
            GovernancePolicy::AutonomousElaboration.to_string(),
            "AUTONOMOUS_ELABORATION"
        );
        assert!("INVALID".parse::<GovernancePolicy>().is_err());
    }

    #[test]
    fn test_task_status_parsing_and_display() {
        assert_eq!("OPEN".parse::<TaskStatus>().unwrap(), TaskStatus::Open);
        assert_eq!(
            "in_progress".parse::<TaskStatus>().unwrap(),
            TaskStatus::InProgress
        );
        assert_eq!(
            "Blocked".parse::<TaskStatus>().unwrap(),
            TaskStatus::Blocked
        );
        assert_eq!(
            "COMPLETED".parse::<TaskStatus>().unwrap(),
            TaskStatus::Completed
        );
        assert_eq!(TaskStatus::InProgress.to_string(), "IN_PROGRESS");
        assert!("UNKNOWN".parse::<TaskStatus>().is_err());
    }

    #[test]
    fn test_evaluate_locked_policy_rejection() {
        let res = evaluate_governance_action(
            GovernancePolicy::Locked,
            GovernanceAction::ElaborateTask,
            "TASK",
            "ACTIVE",
        );
        assert!(matches!(res, Err(MutationError::GovernanceLocked(_))));

        let res2 = evaluate_governance_action(
            GovernancePolicy::Locked,
            GovernanceAction::UpdateTaskStatus,
            "TASK",
            "ACTIVE",
        );
        assert!(matches!(res2, Err(MutationError::GovernanceLocked(_))));
    }

    #[test]
    fn test_evaluate_autonomous_elaboration_policy() {
        let ok = evaluate_governance_action(
            GovernancePolicy::AutonomousElaboration,
            GovernanceAction::ElaborateTask,
            "REQUIREMENT",
            "ACTIVE",
        );
        assert_eq!(ok.unwrap(), MutationPathway::AutonomousTaskElaboration);

        let rejected = evaluate_governance_action(
            GovernancePolicy::HumanReviewRequired,
            GovernanceAction::ElaborateTask,
            "REQUIREMENT",
            "ACTIVE",
        );
        assert!(matches!(
            rejected,
            Err(MutationError::GovernanceRejected(_))
        ));
    }

    #[test]
    fn test_evaluate_update_task_status_validations() {
        let ok = evaluate_governance_action(
            GovernancePolicy::AutonomousElaboration,
            GovernanceAction::UpdateTaskStatus,
            "TASK",
            "ACTIVE",
        );
        assert_eq!(ok.unwrap(), MutationPathway::ActiveTaskUpdate);

        let not_task = evaluate_governance_action(
            GovernancePolicy::AutonomousElaboration,
            GovernanceAction::UpdateTaskStatus,
            "REQUIREMENT",
            "ACTIVE",
        );
        assert!(matches!(
            not_task,
            Err(MutationError::GovernanceRejected(_))
        ));

        let not_active = evaluate_governance_action(
            GovernancePolicy::AutonomousElaboration,
            GovernanceAction::UpdateTaskStatus,
            "TASK",
            "DRAFT",
        );
        assert!(matches!(
            not_active,
            Err(MutationError::GovernanceRejected(_))
        ));
    }

    #[test]
    fn test_evaluate_normative_proposals_and_draft_evolution() {
        let normative = evaluate_governance_action(
            GovernancePolicy::HumanReviewRequired,
            GovernanceAction::ProposeNormativeMutation,
            "REQUIREMENT",
            "ACTIVE",
        );
        assert_eq!(normative.unwrap(), MutationPathway::NormativeDraftProposal);

        let draft_evolution = evaluate_governance_action(
            GovernancePolicy::HumanReviewRequired,
            GovernanceAction::MutateDraft,
            "REQUIREMENT",
            "DRAFT",
        );
        assert_eq!(
            draft_evolution.unwrap(),
            MutationPathway::CandidateDraftEvolution
        );
    }
}
