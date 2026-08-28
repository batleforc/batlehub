use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Why one version survived a retention run.
///
/// Reported per version rather than counted, because the question an operator
/// asks of a first dry run is not "how many" but "why is *that* one being
/// reclaimed" — and the answer has to be checkable against the policy they
/// wrote. Serialised in lowercase snake case, matching the config key each
/// variant corresponds to wherever there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeepReason {
    /// `retention_keep` is set on the version row — the version-tier pin, which
    /// outranks every policy above it (RFC 0016 §4.1).
    Pinned,
    /// Within `keep_versions`, counting back from the newest by publish date.
    KeepVersions,
    /// Published within `keep_for_days`.
    KeepFor,
    /// Downloaded within `keep_if_pulled_days`.
    KeepIfPulled,
    /// Yanked, and `keep_yanked` is on (the default).
    KeepYanked,
    /// No download record, and the version predates the floor before which an
    /// absence proves nothing (RFC 0016 §4.3).
    ///
    /// The one reason that is an *absence* of evidence rather than a presence of
    /// it, which is why it is named separately: an operator reading a report
    /// full of these is looking at a registry whose audit history is younger
    /// than its packages, not at a policy that is keeping too much.
    BeforeSignalFloor,
    /// No keep condition is configured at all, so nothing can be reclaimed.
    ///
    /// Distinct from the conditions above: it means the policy is inert, not
    /// that this version earned its survival.
    NoPolicy,
}

impl KeepReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pinned => "pinned",
            Self::KeepVersions => "keep_versions",
            Self::KeepFor => "keep_for",
            Self::KeepIfPulled => "keep_if_pulled",
            Self::KeepYanked => "keep_yanked",
            Self::BeforeSignalFloor => "before_signal_floor",
            Self::NoPolicy => "no_policy",
        }
    }
}

/// One version a retention run decided about.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetentionDecision {
    pub name: String,
    pub version: String,
    /// `None` when the version is to be reclaimed — every keep condition
    /// declined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_because: Option<KeepReason>,
}

/// What one retention run did, or — under `dry_run` — would have done.
///
/// The same structure either way, deliberately: an operator reads a dry run
/// against a real estate, turns `dry_run` off, and has to be able to compare the
/// two without translating between shapes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct RetentionReport {
    /// Versions examined.
    pub examined: u64,
    /// Versions reclaimed, or that would be under `dry_run`.
    pub reclaimed: u64,
    /// Versions kept.
    pub kept: u64,
    /// True when nothing was written.
    pub dry_run: bool,
    /// The coordinates reclaimed, `"{name}@{version}"`, sorted.
    ///
    /// The list an operator actually reads before turning `dry_run` off — a
    /// count alone cannot be checked against the policy that produced it.
    ///
    /// There is deliberately no `bytes_freed` beside it. A single-file
    /// ecosystem stores its artifact *at* `local:{reg}/{name}/{ver}` and the
    /// only cheap sizing the storage port offers is `stat_by_prefix`, for which
    /// `local:r/p/1.0` is a prefix of `local:r/p/1.0.1` — so the obvious
    /// implementation reports a sibling version's bytes as this one's. A number
    /// that is wrong in a way nobody would notice is worse than no number: an
    /// operator sizing a first live run reads this list.
    #[serde(default)]
    pub reclaimed_coordinates: Vec<String>,
    /// Per-version decisions, including the kept ones and why.
    ///
    /// Bounded by [`Self::decisions_truncated`]: a registry with two million
    /// versions must not serialise two million rows into one response.
    #[serde(default)]
    pub decisions: Vec<RetentionDecision>,
    /// How many decisions were dropped from [`Self::decisions`].
    ///
    /// Reported rather than silently omitted: a bounded list that does not say
    /// it is bounded reads as "this is everything", which is the reasoning that
    /// makes a truncated report dangerous to act on.
    pub decisions_truncated: u64,
    /// Set when the run stopped early because it hit a package it could not
    /// read. Partial results, and the report says so rather than looking
    /// complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_because: Option<String>,
}
