//! Reclaiming the bytes of locally published versions nobody is using
//! (RFC 0016, phases 3–4).
//!
//! # Why this shares no code with `services::eviction`
//!
//! The two look alike enough that the first implementation instinct is to widen
//! `EvictionConfig` to reach local rows. That instinct is the one thing this
//! module exists to resist:
//!
//! | | Cache eviction | Retention |
//! | --- | --- | --- |
//! | Governs | proxy-cached artifacts | locally published versions |
//! | Another copy exists | yes, upstream | **frequently not** |
//! | Cost of a wrong reclaim | a re-fetch | the artifact |
//! | Default | configured per registry | keep everything, forever |
//!
//! That asymmetry sets every default here. A shared implementation would inherit
//! eviction's defaults, its reporting and its silence, and would apply a policy
//! calibrated for recoverable data to data that is not (RFC 0016 §5.1).
//!
//! # Keep conditions are a union of vetoes
//!
//! **A version survives if *any* configured condition matches.** There is no
//! expression to evaluate and no ordering to get wrong: the only way to reclaim
//! a version is for every configured condition to decline. Wrong configuration
//! therefore fails toward keeping, which is the direction that is recoverable.
//!
//! # Retention reclaims bytes, not names
//!
//! A reclaimed version leaves a tombstone and its number can never be taken
//! again. Freeing disk must not free the *namespace*, or retention becomes a
//! supply-chain mechanism by accident — which is why this calls the same
//! `LocalRegistryService::delete_version` a human deletion goes through rather
//! than reaching into the backend itself.

mod report;
pub use report::{KeepReason, RetentionDecision, RetentionReport};

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::entities::{Identity, PublishedPackage};
use crate::error::CoreError;
use crate::ports::PackageRepository;
use crate::services::LocalRegistryService;

/// The date before which an absent download record proves nothing, when the
/// operator has not said otherwise (RFC 0016 §4.3).
///
/// 2026-08-27, the day after the survey remediation that gave the Maven and
/// NuGet local artifact paths a download event at all. Before it the audit trail
/// is silent for those ecosystems, and a retention run that read the silence as
/// disuse would reclaim versions the estate was using every day.
///
/// A constant rather than a config default because it is a fact about this
/// software's history, not a preference: every instance upgraded through that
/// version has the same gap in the same place.
pub const DEFAULT_DOWNLOAD_SIGNAL_FLOOR: &str = "2026-08-27T00:00:00Z";

/// How many per-version decisions one report carries.
///
/// A registry with two million versions must not serialise two million rows into
/// one response. What is dropped is *counted* and reported, because a bounded
/// list that does not say it is bounded reads as "this is everything".
pub const MAX_REPORTED_DECISIONS: usize = 2_000;

/// The resolved keep conditions for one registry, as plain data.
///
/// Mirrors the config block, with the days already turned into instants against
/// a single `now` — so every version in one run is judged against the same
/// clock, and a sweep that takes ten minutes cannot decide two identically-aged
/// versions differently.
#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub keep_versions: Option<u32>,
    pub keep_for: Option<Duration>,
    pub keep_if_pulled: Option<Duration>,
    pub keep_yanked: bool,
    pub download_signal_floor: DateTime<Utc>,
    pub reclaim_delay: Duration,
    pub dry_run: bool,
}

impl RetentionPolicy {
    /// Whether any reclamation condition is configured — i.e. whether a run
    /// would do anything at all. `keep_yanked` does not count: it only ever
    /// vetoes.
    pub fn reclaims_anything(&self) -> bool {
        self.keep_versions.is_some() || self.keep_for.is_some() || self.keep_if_pulled.is_some()
    }
}

/// Drives retention over one registry's locally published versions.
pub struct RetentionService {
    pub local: Arc<LocalRegistryService>,
    /// The download signal `keep_if_pulled` reads. `None` disables that
    /// condition entirely — and, because the conditions are a union of vetoes,
    /// disabling a veto makes the policy *more* aggressive. So a run with a
    /// configured `keep_if_pulled` and no repository refuses rather than
    /// silently reclaiming what it cannot prove is idle.
    pub packages: Option<Arc<dyn PackageRepository>>,
}

/// One version and everything the decision needs, gathered before any of the
/// conditions are asked.
struct Candidate<'a> {
    pkg: &'a PublishedPackage,
    /// Index from the newest by publish date: 0 is the newest.
    rank_from_newest: usize,
    last_download: Option<DateTime<Utc>>,
}

impl RetentionService {
    pub fn new(
        local: Arc<LocalRegistryService>,
        packages: Option<Arc<dyn PackageRepository>>,
    ) -> Self {
        Self { local, packages }
    }

    /// Run retention over every locally published version in `registry`.
    ///
    /// Under `policy.dry_run` nothing is written and the report says what would
    /// have happened. That is the default and the only mode an operator should
    /// meet first.
    pub async fn run(
        &self,
        registry: &str,
        policy: &RetentionPolicy,
        identity: &Identity,
    ) -> Result<RetentionReport, CoreError> {
        if policy.keep_if_pulled.is_some() && self.packages.is_none() {
            return Err(CoreError::Config(format!(
                "registry '{registry}': keep_if_pulled_days is configured but this deployment has \
                 no package repository, so there is no download signal to read. Retention would \
                 reclaim versions it cannot prove are idle; refusing to run"
            )));
        }

        let mut report = RetentionReport {
            dry_run: policy.dry_run,
            ..Default::default()
        };

        // No keep condition means nothing is reclaimable. Reported as an
        // ordinary run over the whole registry rather than short-circuited, so
        // the operator sees the version count they are protecting and a
        // `no_policy` reason against it, instead of an empty answer they have to
        // interpret.
        let names = self.local.backend.list_package_names(registry).await?;
        let now = Utc::now();

        for name in names {
            let versions = self.local.backend.get_versions(registry, &name).await?;
            if versions.is_empty() {
                continue;
            }
            let downloads = self.last_downloads(registry, &name).await?;

            // `get_versions` is `published_at ASC`; rank 0 must be the newest.
            let total = versions.len();
            let mut doomed: Vec<&PublishedPackage> = Vec::new();

            for (i, pkg) in versions.iter().enumerate() {
                let candidate = Candidate {
                    pkg,
                    rank_from_newest: total - 1 - i,
                    last_download: downloads
                        .iter()
                        .find(|(v, _)| *v == pkg.version)
                        .map(|(_, t)| *t),
                };
                report.examined += 1;
                let decision = Self::decide(&candidate, policy, now);
                if decision.is_none() {
                    doomed.push(pkg);
                } else {
                    report.kept += 1;
                }
                if report.decisions.len() < MAX_REPORTED_DECISIONS {
                    report.decisions.push(RetentionDecision {
                        name: pkg.name.clone(),
                        version: pkg.version.clone(),
                        kept_because: decision,
                    });
                } else {
                    report.decisions_truncated += 1;
                }
            }

            for pkg in doomed {
                if policy.dry_run {
                    report.reclaimed += 1;
                    report
                        .reclaimed_coordinates
                        .push(format!("{}@{}", pkg.name, pkg.version));
                    continue;
                }
                // The same call a human deletion goes through, so a reclaimed
                // version leaves the same tombstone, drops the same bytes and
                // records the same audit event — with the run's identity as the
                // subject, which is what tells the two apart in the trail.
                match self
                    .local
                    .delete_version(registry, &pkg.name, &pkg.version, identity)
                    .await
                {
                    Ok(_) => {
                        report.reclaimed += 1;
                        report
                            .reclaimed_coordinates
                            .push(format!("{}@{}", pkg.name, pkg.version));
                    }
                    // **Stop, and say so.** Not `?`: by the time one delete
                    // fails the run has already reclaimed real versions, and
                    // returning an error would throw away the only record of
                    // which ones — leaving an operator to work out what happened
                    // from the audit log. Not "continue", either: a delete that
                    // failed is a storage or database problem, and grinding
                    // through 200 000 more packages against a broken backend
                    // turns one fault into a very long one.
                    Err(e) => {
                        report.incomplete_because = Some(format!(
                            "stopped at {}@{}: {e}. Everything listed above was reclaimed; \
                             nothing after it was attempted.",
                            pkg.name, pkg.version
                        ));
                        report.reclaimed_coordinates.sort();
                        return Ok(report);
                    }
                }
                if !policy.reclaim_delay.is_zero() {
                    tokio::time::sleep(policy.reclaim_delay).await;
                }
            }
        }

        report.reclaimed_coordinates.sort();
        Ok(report)
    }

    /// **The union of vetoes.** `Some(reason)` keeps, `None` reclaims.
    ///
    /// Ordered so the cheapest and most absolute conditions come first, and so
    /// the reported reason is the most informative one when several apply: an
    /// operator told a version survived because it is *pinned* learns more than
    /// one told it survived because it is recent, when both are true.
    ///
    /// The floor-date check sits **last** on purpose. It is the one that turns
    /// *absence of evidence* into a keep, and putting it after the positive
    /// conditions makes it obvious that it can only ever add survivors
    /// (RFC 0016 §5.1).
    fn decide(
        candidate: &Candidate<'_>,
        policy: &RetentionPolicy,
        now: DateTime<Utc>,
    ) -> Option<KeepReason> {
        // An inert policy keeps everything. Checked first so a registry with a
        // block that configures only compaction never reclaims by accident.
        if !policy.reclaims_anything() {
            return Some(KeepReason::NoPolicy);
        }

        // The version-tier pin outranks every policy above it.
        if candidate.pkg.retention_keep {
            return Some(KeepReason::Pinned);
        }

        if let Some(n) = policy.keep_versions {
            if candidate.rank_from_newest < n as usize {
                return Some(KeepReason::KeepVersions);
            }
        }

        if let Some(window) = policy.keep_for {
            if within(candidate.pkg.published_at, window, now) {
                return Some(KeepReason::KeepFor);
            }
        }

        if let Some(window) = policy.keep_if_pulled {
            if let Some(last) = candidate.last_download {
                if within(last, window, now) {
                    return Some(KeepReason::KeepIfPulled);
                }
            }
        }

        if policy.keep_yanked && candidate.pkg.yanked {
            return Some(KeepReason::KeepYanked);
        }

        // Absence of evidence, last. A version with no download record that was
        // published before the floor might have been pulled every day of its
        // life without anything writing it down.
        //
        // Only consulted when `keep_if_pulled` is configured: with no download
        // condition in the policy, the signal is not being read at all and its
        // gaps are nobody's business.
        if policy.keep_if_pulled.is_some()
            && candidate.last_download.is_none()
            && candidate.pkg.published_at < policy.download_signal_floor
        {
            return Some(KeepReason::BeforeSignalFloor);
        }

        None
    }

    /// The download signal for one package, or an empty list when the policy
    /// does not read it.
    ///
    /// Skipped rather than fetched-and-ignored when `keep_if_pulled` is unset:
    /// this is a query per package over a table with the whole estate's access
    /// history in it, and a policy that does not consult the signal should not
    /// pay for it.
    async fn last_downloads(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<(String, DateTime<Utc>)>, CoreError> {
        let Some(ref repo) = self.packages else {
            return Ok(vec![]);
        };
        repo.last_downloads(registry, name).await
    }
}

/// Whether `then` is within `window` of `now`.
///
/// Saturating rather than signed: a `published_at` in the future — a clock skew,
/// a restored backup — is "within" every window rather than outside all of them,
/// which keeps rather than reclaims.
fn within(then: DateTime<Utc>, window: Duration, now: DateTime<Utc>) -> bool {
    let Ok(window) = chrono::Duration::from_std(window) else {
        // An absurd window (>292 years) is not a reason to reclaim.
        return true;
    };
    then >= now - window
}

#[cfg(test)]
mod tests;
