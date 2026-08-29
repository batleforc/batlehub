//! What shadow mode would have refused — RFC 0015 §4.7.
//!
//! `grants.dry_run` is *"the most useful setting in this document and the most
//! dangerous"*. It is what makes §10's migration survivable in practice — enable
//! the new model in shadow, watch a week of real traffic, then enforce — and it
//! is also, if forgotten, an authorization bypass configured on purpose.
//!
//! A shadow that produces nothing to read is only the dangerous half. §4.7 asks
//! for three records of every would-have-been:
//!
//! - a structured log line,
//! - a `batlehub_policy_dryrun_total` counter labelled by policy and node,
//! - an admin endpoint listing recent would-have-beens, so the console can show
//!   them.
//!
//! This is the third. The first two happen at the same call site.
//!
//! # Why a bounded buffer and not a table
//!
//! A would-have-been is *operational* rather than evidential: an operator
//! watching a migration wants the last few hundred, grouped by node, to decide
//! whether it is safe to enforce. The audit log is where a permanent record
//! belongs, and a denial that actually happened is already there — this is the
//! set that, by construction, produced no denial to record.
//!
//! Writing every shadowed request to Postgres would also put a write on the read
//! path of a registry that is, by definition, serving everything it is asked
//! for. A shadow is enabled precisely when traffic is highest and the
//! consequences of slowing it down are worst.

use std::collections::VecDeque;

use chrono::{DateTime, NaiveDate, Utc};
use tokio::sync::RwLock;

/// How many would-have-beens to keep.
///
/// Enough to see a pattern, not enough to matter. An operator reading this is
/// asking "what breaks if I enforce?", and the answer is a *set of nodes and
/// verbs* rather than a request log — the same few coordinates recur.
const MAX_ENTRIES: usize = 500;

/// One request that shadow mode served and enforcement would have refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowedDenial {
    pub at: DateTime<Utc>,
    pub registry: String,
    pub package: String,
    pub version: String,
    /// The verb the caller did not hold.
    pub action: String,
    /// The subject, in the spelling a grant would be written in — so an operator
    /// can copy it into the block that would fix this.
    pub subject: String,
    /// The node whose shadow served this request, and the date it expires.
    pub node: String,
    pub shadow_until: NaiveDate,
}

/// Recent would-have-beens, newest last.
#[derive(Default)]
pub struct ShadowLog {
    entries: RwLock<VecDeque<ShadowedDenial>>,
}

impl ShadowLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many entries the buffer keeps at most.
    ///
    /// Reported to the admin endpoint rather than assumed by it, because
    /// `recent.len() == capacity()` is the one state where an operator must not
    /// read the list as complete.
    pub fn capacity(&self) -> usize {
        MAX_ENTRIES
    }

    pub async fn record(&self, denial: ShadowedDenial) {
        let mut entries = self.entries.write().await;
        if entries.len() >= MAX_ENTRIES {
            entries.pop_front();
        }
        entries.push_back(denial);
    }

    /// Everything recorded, newest first.
    pub async fn recent(&self, limit: usize) -> Vec<ShadowedDenial> {
        let entries = self.entries.read().await;
        entries.iter().rev().take(limit).cloned().collect()
    }

    /// Would-have-beens grouped by node, with a count and the distinct verbs.
    ///
    /// The shape the question actually has. "Can I enforce this namespace yet?"
    /// is answered by *which verbs are still missing and for whom*, not by a
    /// list of requests — and a busy registry produces thousands of the latter
    /// for a handful of the former.
    pub async fn by_node(&self) -> Vec<ShadowSummary> {
        let entries = self.entries.read().await;
        let mut out: Vec<ShadowSummary> = Vec::new();
        for e in entries.iter() {
            match out.iter_mut().find(|s| s.node == e.node) {
                Some(existing) => {
                    existing.count += 1;
                    if !existing.actions.contains(&e.action) {
                        existing.actions.push(e.action.clone());
                    }
                    if !existing.subjects.contains(&e.subject) {
                        existing.subjects.push(e.subject.clone());
                    }
                    existing.last_seen = existing.last_seen.max(e.at);
                }
                None => out.push(ShadowSummary {
                    node: e.node.clone(),
                    shadow_until: e.shadow_until,
                    count: 1,
                    actions: vec![e.action.clone()],
                    subjects: vec![e.subject.clone()],
                    last_seen: e.at,
                }),
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.count));
        out
    }
}

/// One node's shadow, summarised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowSummary {
    pub node: String,
    pub shadow_until: NaiveDate,
    pub count: u64,
    pub actions: Vec<String>,
    pub subjects: Vec<String>,
    pub last_seen: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denial(node: &str, action: &str, subject: &str) -> ShadowedDenial {
        ShadowedDenial {
            at: Utc::now(),
            registry: "npm1".to_owned(),
            package: "pkg".to_owned(),
            version: "1.0.0".to_owned(),
            action: action.to_owned(),
            subject: subject.to_owned(),
            node: node.to_owned(),
            shadow_until: NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(),
        }
    }

    #[tokio::test]
    async fn recent_is_newest_first() {
        let log = ShadowLog::new();
        log.record(denial("registry:npm1", "releases:read", "*"))
            .await;
        log.record(denial("registry:npm1", "releases:publish", "*"))
            .await;

        let recent = log.recent(10).await;
        assert_eq!(recent[0].action, "releases:publish");
        assert_eq!(recent[1].action, "releases:read");
    }

    /// The buffer is bounded: a shadow is enabled when traffic is highest.
    #[tokio::test]
    async fn the_log_is_bounded() {
        let log = ShadowLog::new();
        for i in 0..(MAX_ENTRIES + 50) {
            log.record(denial("n", &format!("a{i}"), "*")).await;
        }
        assert_eq!(log.recent(usize::MAX).await.len(), MAX_ENTRIES);
        // …and it drops the *oldest*, so what is left is what is happening now.
        assert_eq!(
            log.recent(1).await[0].action,
            format!("a{}", MAX_ENTRIES + 49)
        );
    }

    /// The summary is the shape the question has: which verbs are still missing
    /// and for whom, not a list of requests.
    #[tokio::test]
    async fn by_node_groups_and_deduplicates() {
        let log = ShadowLog::new();
        log.record(denial("namespace:@acme", "releases:read", "role:user"))
            .await;
        log.record(denial("namespace:@acme", "releases:read", "role:user"))
            .await;
        log.record(denial("namespace:@acme", "releases:publish", "user:bob"))
            .await;
        log.record(denial("registry:npm1", "releases:read", "*"))
            .await;

        let summary = log.by_node().await;
        assert_eq!(summary.len(), 2);
        // Busiest first: an operator triaging a migration reads top-down.
        assert_eq!(summary[0].node, "namespace:@acme");
        assert_eq!(summary[0].count, 3);
        assert_eq!(
            summary[0].actions,
            vec!["releases:read", "releases:publish"],
            "distinct verbs, in the order first seen"
        );
        assert_eq!(summary[0].subjects, vec!["role:user", "user:bob"]);
    }

    #[tokio::test]
    async fn an_empty_log_summarises_to_nothing() {
        assert!(ShadowLog::new().by_node().await.is_empty());
    }
}
