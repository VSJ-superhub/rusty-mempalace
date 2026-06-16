//! Promotion gate — a structured gating function between transient observations
//! and durable writes.
//!
//! Before this module, `persist`/`store_fact` flowed straight into
//! `store_drawer`: every write was promoted unconditionally and `confidence`/
//! `source` were stored as inert attributes, not gates. That is the classic
//! place memory poisoning starts — an unattributed, low-confidence claim made
//! mid-conversation becomes a durable "fact" indistinguishable from a verified
//! one.
//!
//! The gate classifies each write into a [`MemoryKind`] and applies per-type
//! rules:
//!
//! | kind        | rule                                                        |
//! |-------------|-------------------------------------------------------------|
//! | observation | always promoted (lowest tier, raw scratch)                  |
//! | fact        | confidence ≥ 0.7 AND attributable (run id or non-conv src)  |
//! | episode     | only on task completion (`task_complete = true`)            |
//! | policy      | never auto-promotes; requires explicit `confirm = true`     |
//!
//! Rejections are returned to the caller with an actionable reason rather than
//! silently dropped or silently written.

use yourmemory_core::storage::{Confidence, Source};

/// The kind of memory being written. Selects which promotion rule applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    /// Raw scratch note. Lowest tier — always promoted.
    Observation,
    /// Durable, attributable claim about the world. Strictest gate.
    Fact,
    /// Summary of a completed task/session. Promoted only on task completion.
    Episode,
    /// Operating rule or preference. Never auto-promotes.
    Policy,
}

impl MemoryKind {
    /// Parse an explicit `kind` argument. Unknown / missing values fall back to
    /// [`MemoryKind::Observation`] — the safe default, never silently a fact.
    pub fn parse(s: Option<&str>) -> Self {
        match s.map(|x| x.trim().to_ascii_lowercase()).as_deref() {
            Some("fact") => MemoryKind::Fact,
            Some("episode") => MemoryKind::Episode,
            Some("policy") => MemoryKind::Policy,
            Some("observation") | Some("note") | Some("") | None => MemoryKind::Observation,
            _ => MemoryKind::Observation,
        }
    }
}

/// Minimum numeric confidence for a write classified as a fact.
pub const FACT_MIN_CONFIDENCE: f64 = 0.7;

/// Map the discrete [`Confidence`] enum onto a [0,1] score for threshold rules.
pub fn confidence_score(c: &Confidence) -> f64 {
    match c {
        Confidence::High => 0.9,
        Confidence::Medium => 0.7,
        Confidence::Low => 0.4,
        Confidence::Inferred => 0.2,
    }
}

/// A write request as seen by the gate. Borrows from the already-parsed tool
/// arguments so the gate stays allocation-free.
pub struct WriteRequest<'a> {
    pub kind: MemoryKind,
    pub confidence: &'a Confidence,
    pub source: &'a Source,
    /// Identifier of the run/session that produced this claim, if any.
    pub source_run_id: Option<&'a str>,
    /// Whether the originating task has completed (gates episodes).
    pub task_complete: bool,
    /// Explicit human-in-the-loop confirmation (gates policies).
    pub confirm: bool,
}

/// Outcome of running the gate.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// Write may proceed to durable storage.
    Promote,
    /// Write is refused; carries an actionable, human-readable reason.
    Reject(String),
}

/// Apply the per-type promotion rules to a write request.
pub fn evaluate(req: &WriteRequest) -> Decision {
    match req.kind {
        MemoryKind::Observation => Decision::Promote,

        MemoryKind::Fact => {
            let score = confidence_score(req.confidence);
            if score < FACT_MIN_CONFIDENCE {
                return Decision::Reject(format!(
                    "fact rejected: confidence {:.2} < {:.2} threshold — \
                     raise confidence or store as kind=observation",
                    score, FACT_MIN_CONFIDENCE
                ));
            }
            // A fact must be attributable: either an explicit run id, or a
            // non-conversational source (user/system/config). A bare claim
            // dropped mid-conversation is not promotable as a fact.
            let attributable =
                req.source_run_id.is_some() || !matches!(req.source, Source::Conversation);
            if !attributable {
                return Decision::Reject(
                    "fact rejected: unattributed — supply source_run_id, or set \
                     source to user/system/config"
                        .to_string(),
                );
            }
            Decision::Promote
        }

        MemoryKind::Episode => {
            if req.task_complete {
                Decision::Promote
            } else {
                Decision::Reject(
                    "episode rejected: episodes are promoted only on task completion — \
                     set task_complete=true"
                        .to_string(),
                )
            }
        }

        MemoryKind::Policy => {
            if req.confirm {
                Decision::Promote
            } else {
                Decision::Reject(
                    "policy rejected: policies never auto-promote — requires explicit \
                     confirm=true (human-in-the-loop)"
                        .to_string(),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req<'a>(
        kind: MemoryKind,
        confidence: &'a Confidence,
        source: &'a Source,
        source_run_id: Option<&'a str>,
        task_complete: bool,
        confirm: bool,
    ) -> WriteRequest<'a> {
        WriteRequest {
            kind,
            confidence,
            source,
            source_run_id,
            task_complete,
            confirm,
        }
    }

    #[test]
    fn observation_always_promotes() {
        let d = evaluate(&req(
            MemoryKind::Observation,
            &Confidence::Inferred,
            &Source::Conversation,
            None,
            false,
            false,
        ));
        assert_eq!(d, Decision::Promote);
    }

    #[test]
    fn fact_below_threshold_rejected() {
        let d = evaluate(&req(
            MemoryKind::Fact,
            &Confidence::Low,
            &Source::User,
            None,
            false,
            false,
        ));
        assert!(matches!(d, Decision::Reject(_)));
    }

    #[test]
    fn fact_unattributed_conversation_rejected() {
        let d = evaluate(&req(
            MemoryKind::Fact,
            &Confidence::High,
            &Source::Conversation,
            None,
            false,
            false,
        ));
        assert!(matches!(d, Decision::Reject(_)));
    }

    #[test]
    fn fact_with_run_id_promotes() {
        let d = evaluate(&req(
            MemoryKind::Fact,
            &Confidence::Medium,
            &Source::Conversation,
            Some("run-123"),
            false,
            false,
        ));
        assert_eq!(d, Decision::Promote);
    }

    #[test]
    fn fact_from_user_source_promotes() {
        let d = evaluate(&req(
            MemoryKind::Fact,
            &Confidence::High,
            &Source::User,
            None,
            false,
            false,
        ));
        assert_eq!(d, Decision::Promote);
    }

    #[test]
    fn episode_gated_on_completion() {
        let pending = evaluate(&req(
            MemoryKind::Episode,
            &Confidence::High,
            &Source::System,
            None,
            false,
            false,
        ));
        assert!(matches!(pending, Decision::Reject(_)));

        let done = evaluate(&req(
            MemoryKind::Episode,
            &Confidence::High,
            &Source::System,
            None,
            true,
            false,
        ));
        assert_eq!(done, Decision::Promote);
    }

    #[test]
    fn policy_never_auto_promotes() {
        let auto = evaluate(&req(
            MemoryKind::Policy,
            &Confidence::High,
            &Source::User,
            Some("run-1"),
            true,
            false,
        ));
        assert!(matches!(auto, Decision::Reject(_)));

        let confirmed = evaluate(&req(
            MemoryKind::Policy,
            &Confidence::High,
            &Source::User,
            None,
            false,
            true,
        ));
        assert_eq!(confirmed, Decision::Promote);
    }
}
