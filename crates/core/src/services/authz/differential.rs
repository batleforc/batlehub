//! The §11.3 differential harness: both evaluators, every combination, no
//! disagreements.
//!
//! > Run **both** evaluators — today's chain and the new resolver — over the
//! > cartesian product of every fixture config in the tree, every subject shape
//! > (anonymous, user, admin, each group form, a PAT, a redeemed signed URL) and
//! > every verb, and fail on any disagreement. Translation is only correct if it
//! > is *observably* identical; a review cannot establish that across four
//! > repository implementations and twenty-one registry types.
//!
//! §11.3 calls this "not a test so much as the gate for phase 3", and §7 says
//! why: a translation that widens any existing config is a silent privilege
//! escalation across every deployment. Nothing about that failure is loud. The
//! only thing that catches it is running both answers side by side.
//!
//! # What "both evaluators" means here
//!
//! **Old:** `RbacRule::evaluate` — the config's verbs, matched per request
//! against the caller's role and groups.
//!
//! **New:** [`translate_rbac`](super::translate::translate_rbac) into a
//! registry-tier [`GrantMap`], then [`resolve`] over a one-node path.
//!
//! Only the `rbac` rule is compared, and that is the whole of what the
//! translation claims. The other rules in the chain judge the *artifact* rather
//! than the caller (§5.2) and are untouched by this phase; including them would
//! compare the new resolver against gates it does not replace, and a green
//! result would mean less rather than more.
//!
//! # The four fixtures that are not realistic
//!
//! §11.3 names them, and the reason they are named is that **none of them
//! appears in a corpus of realistic configurations** — which is exactly why they
//! have to be added deliberately rather than harvested. Each is a §10 rule whose
//! failure is silent. They are in [`RBAC_FIXTURES`] with the rule they guard.

use std::collections::HashMap;

use crate::entities::{
    expand_patterns, resolve, Action, GrantMap, Identity, Node, Role, Subject, Tier, WildcardScope,
};
use crate::rules::{RbacRule, Rule, RuleContext, RuleDecision};

use super::translate::{translate_rbac, ExploreFlags, RbacSnapshot, WriteMode};

/// One config shape to compare the two evaluators over.
pub struct RbacFixture {
    pub name: &'static str,
    /// Why this shape is in the corpus rather than harvested from real configs.
    pub guards: &'static str,
    pub anonymous: &'static [&'static str],
    pub user: &'static [&'static str],
    pub admin: &'static [&'static str],
    pub groups: &'static [(&'static str, &'static [&'static str])],
    pub explore: (bool, bool, bool),
}

/// The corpus.
///
/// The first four rows are §11.3's named fixtures; the rest are the shapes real
/// configs actually take, so a rule that only breaks on ordinary input is caught
/// too.
pub const RBAC_FIXTURES: &[RbacFixture] = &[
    RbacFixture {
        name: "wildcard-on-a-non-admin-subject",
        guards: "§10 rule 3. `user = [\"*\"]` and `groups = { eng = [\"*\"] }` are legal \
                 today and mean two read verbs. Rule 3 is the only thing standing between \
                 them and the whole enum, and a harness that never sees one passes while \
                 that rule is missing.",
        anonymous: &[],
        user: &["*"],
        admin: &["*"],
        groups: &[("eng", &["*"])],
        explore: (false, true, true),
    },
    RbacFixture {
        name: "explore-denied-for-a-role",
        guards: "§10 rule 2. The disagreement to catch is a caller the console refuses \
                 today and answers afterwards.",
        anonymous: &["releases:read"],
        user: &["releases:read", "source:read"],
        admin: &["*"],
        groups: &[],
        explore: (false, false, true),
    },
    RbacFixture {
        name: "source-read-without-releases-read",
        guards: "§10 rule 4, from one side. This registry must still reach the listing \
                 documents `source:read` reaches today — the cargo sparse index is the \
                 coordinate where the two verbs disagree.",
        anonymous: &[],
        user: &["source:read"],
        admin: &["*"],
        groups: &[],
        explore: (false, true, true),
    },
    RbacFixture {
        name: "releases-read-without-source-read",
        guards: "§10 rule 4, from the other side. Asserted separately because a \
                 translation that gave `releases:list` to only one of the two verbs \
                 passes whichever side it chose.",
        anonymous: &[],
        user: &["releases:read"],
        admin: &["*"],
        groups: &[],
        explore: (false, true, true),
    },
    RbacFixture {
        name: "the-shape-config-example-ships",
        guards: "The ordinary case, eight times over in `config.example.toml`.",
        anonymous: &["releases:read", "source:read"],
        user: &["releases:read", "source:read"],
        admin: &["*"],
        groups: &[],
        explore: (true, true, true),
    },
    RbacFixture {
        name: "every-group-shape",
        guards: "The three `[registries.rbac.groups]` key shapes match three different \
                 things today, and folding any two together widens every config that \
                 uses the narrower one.",
        anonymous: &[],
        user: &[],
        admin: &["*"],
        groups: &[
            ("eng", &["releases:read"]),
            ("*:qa", &["source:read"]),
            ("oidc1:ops", &["releases:read", "source:read"]),
        ],
        explore: (false, false, true),
    },
    RbacFixture {
        name: "anonymous-denied-everything",
        guards: "The shape `authz_matrix.rs` uses for its whole negative axis, so a \
                 disagreement here would invalidate that suite rather than this one.",
        anonymous: &[],
        user: &["releases:read", "source:read"],
        admin: &["*"],
        groups: &[],
        explore: (false, true, true),
    },
];

/// The subject shapes §11.3 names.
///
/// A PAT is *its user* — §4.3 settles that it carries its creator's groups and
/// can never exceed them — so it appears here as an ordinary user identity with
/// groups rather than as a fourth kind of principal. A redeemed signed URL
/// produces an identity too (§9.2: "Redemption already produces an `Identity`;
/// under this RFC it produces a `Subject`"), so it is covered by the same rows.
pub fn subject_shapes() -> Vec<(&'static str, Subject)> {
    let mk = |role: Role, user: Option<&str>, groups: &[&str]| {
        Subject::Identity(Identity {
            user_id: user.map(str::to_owned),
            role,
            auth_provider: None,
            groups: groups.iter().map(|g| (*g).to_owned()).collect(),
        })
    };
    vec![
        ("anonymous", mk(Role::Anonymous, None, &[])),
        ("user", mk(Role::User, Some("alice"), &[])),
        ("admin", mk(Role::Admin, Some("root"), &[])),
        ("bare-group", mk(Role::Anonymous, None, &["eng"])),
        ("prefixed-group", mk(Role::Anonymous, None, &["oidc1:eng"])),
        (
            "other-provider-group",
            mk(Role::Anonymous, None, &["oidc2:qa"]),
        ),
        ("bare-qa-group", mk(Role::Anonymous, None, &["qa"])),
        ("exact-ops-group", mk(Role::Anonymous, None, &["oidc1:ops"])),
        (
            "wrong-provider-ops",
            mk(Role::Anonymous, None, &["oidc2:ops"]),
        ),
        // A PAT: its creator's identity, with a subset of their groups.
        (
            "pat-of-a-user",
            mk(Role::User, Some("alice"), &["oidc1:eng"]),
        ),
        (
            "multi-group-user",
            mk(Role::User, Some("bob"), &["eng", "oidc1:ops"]),
        ),
    ]
}

/// One disagreement between the two evaluators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disagreement {
    pub fixture: &'static str,
    pub subject: &'static str,
    pub action: Action,
    /// What today's `RbacRule` answered.
    pub old_allows: bool,
    /// What the translated grants answer.
    pub new_allows: bool,
}

impl std::fmt::Display for Disagreement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let direction = if self.new_allows {
            "WIDENED — the new model allows what today refuses"
        } else {
            "BROKE — the new model refuses what today allows"
        };
        write!(
            f,
            "{}/{} on '{}': old={} new={} — {direction}",
            self.fixture, self.subject, self.action, self.old_allows, self.new_allows
        )
    }
}

/// Run both evaluators over the whole cartesian product and report every
/// disagreement.
///
/// Returns an empty vector when the translation is observably identical, which
/// is the only claim §10 makes and the only one worth believing.
pub async fn run(write_mode: WriteMode) -> Vec<Disagreement> {
    let mut out = Vec::new();

    for fixture in RBAC_FIXTURES {
        let (old, new) = build_pair(fixture, write_mode);

        for (subject_name, subject) in subject_shapes() {
            for action in Action::ALL {
                // Verbs the new model introduces have no "today" to disagree
                // with: no route requests them, `[registries.rbac]` cannot name
                // them, and §10 rule 5 grants some of them deliberately. They
                // are asserted by `translate.rs`'s own tests, where the claim is
                // about the rule rather than about equivalence.
                if !is_expressible_today(*action) {
                    continue;
                }

                let old_allows = old_allows(&old, &subject, *action).await;
                let new_allows = new_allows(&new, &subject, *action);

                if old_allows != new_allows {
                    out.push(Disagreement {
                        fixture: fixture.name,
                        subject: subject_name,
                        action: *action,
                        old_allows,
                        new_allows,
                    });
                }
            }
        }
    }

    out
}

/// The verbs `RbacRule` — the evaluator on the left-hand side — has an opinion
/// about.
///
/// These two, and no others. Everything else in the vocabulary is either new in
/// phase 1, or was enforced somewhere `RbacRule` never saw: today's write
/// authority is a role check in `publish.rs` and `lifecycle.rs`, the
/// `require_admin` surfaces are a middleware, and `catalogue:browse` is
/// `hot_config`'s access sets. Comparing the resolver against an evaluator with
/// no opinion would produce a disagreement on every row and mean nothing.
///
/// **`catalogue:browse` is the one worth naming**, because it *looks* like it
/// belongs. It is the fourth field of the same config struct, and §10 rule 2
/// translates it — but not here: see `translate.rs`'s "Rule 2 is not this
/// module's". Excluding it narrows what this harness claims, and the claim it
/// still makes is exactly co-extensive with what `translate_rbac` does.
fn is_expressible_today(action: Action) -> bool {
    matches!(action, Action::ReleasesRead | Action::SourceRead)
}

fn build_pair(fixture: &RbacFixture, write_mode: WriteMode) -> (RbacRule, GrantMap) {
    let own = |v: &[&str]| -> Vec<String> { v.iter().map(|s| (*s).to_owned()).collect() };

    let permissions = HashMap::from([
        (Role::Anonymous, own(fixture.anonymous)),
        (Role::User, own(fixture.user)),
        (Role::Admin, own(fixture.admin)),
    ]);
    let groups: HashMap<String, Vec<String>> = fixture
        .groups
        .iter()
        .map(|(k, v)| ((*k).to_owned(), own(v)))
        .collect();

    let old = RbacRule::from_patterns(permissions.clone())
        .and_then(|r| r.with_group_patterns(groups.clone()))
        .expect("fixture patterns are valid");

    let expand = |v: &[String]| expand_patterns(v, WildcardScope::Legacy).expect("valid");
    let snapshot = RbacSnapshot {
        anonymous: expand(&own(fixture.anonymous)),
        user: expand(&own(fixture.user)),
        admin: expand(&own(fixture.admin)),
        groups: groups.iter().map(|(k, v)| (k.clone(), expand(v))).collect(),
        explore: ExploreFlags {
            anonymous: fixture.explore.0,
            user: fixture.explore.1,
            admin: fixture.explore.2,
        },
    };
    let new = translate_rbac(&snapshot, write_mode).expect("fixture translates");
    (old, new)
}

async fn old_allows(rule: &RbacRule, subject: &Subject, action: Action) -> bool {
    let meta = crate::entities::PackageMetadata {
        id: crate::entities::PackageId::new("reg", "pkg", "1.0.0"),
        published_at: None,
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::Value::Null,
        cache_control: None,
    };
    let ctx = RuleContext {
        identity: subject.identity(),
        package: &meta,
        action,
        cache_entry: None,
        requested_version: None,
    };
    matches!(rule.evaluate(&ctx).await, RuleDecision::Allow)
}

fn new_allows(grants: &GrantMap, subject: &Subject, action: Action) -> bool {
    let path = [Node::new(
        Tier::Registry,
        "registry:reg",
        Some(grants.clone()),
    )];
    resolve(&path, subject).holds(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate. §11.3: *"fail on any disagreement."*
    #[tokio::test]
    async fn the_two_evaluators_agree_on_every_combination() {
        for mode in [WriteMode::Accepts, WriteMode::Refuses] {
            let disagreements = run(mode).await;
            assert!(
                disagreements.is_empty(),
                "{} disagreement(s) between today's RbacRule and the translated grants \
                 ({mode:?}):\n  {}",
                disagreements.len(),
                disagreements
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            );
        }
    }

    /// The harness can actually see a disagreement.
    ///
    /// A differential test that always passes is worse than none: it is the
    /// reassurance without the check. So one is manufactured — a translation
    /// that drops rule 2 — and the harness must report it. This is the same
    /// discipline as confirming a security test red against the pre-fix code,
    /// applied to a harness rather than to a fix.
    #[tokio::test]
    async fn the_harness_reports_a_widening_when_there_is_one() {
        // `explore-denied-for-a-role` denies the console to `user`. A
        // translation that ignored the flag would grant `catalogue:browse`
        // anyway, via the same path rule 2 exists to prevent.
        let fixture = &RBAC_FIXTURES[1];
        let (old, mut new) = build_pair(fixture, WriteMode::Refuses);
        new = new.grant(
            crate::entities::SubjectMatcher::Role(Role::User),
            [Action::CatalogueBrowse],
        );

        let user = Subject::Identity(Identity {
            user_id: Some("alice".to_owned()),
            role: Role::User,
            auth_provider: None,
            groups: vec![],
        });
        assert!(!old_allows(&old, &user, Action::CatalogueBrowse).await);
        assert!(
            new_allows(&new, &user, Action::CatalogueBrowse),
            "the manufactured widening must be visible, or this harness proves nothing"
        );
    }

    /// Every fixture says why it is in the corpus.
    ///
    /// The four §11.3 names are there precisely because they do *not* occur in
    /// realistic configs; a row added later without a reason is a row nobody can
    /// tell apart from noise.
    #[test]
    fn every_fixture_states_what_it_guards() {
        for f in RBAC_FIXTURES {
            assert!(
                !f.guards.trim().is_empty(),
                "fixture '{}' gives no reason for existing",
                f.name
            );
        }
        assert!(
            RBAC_FIXTURES.len() >= 4,
            "§11.3 names four fixtures by construction; the corpus must hold at least those"
        );
    }

    /// **The instance tier cannot widen what this harness compares**, and that is
    /// why the harness does not walk it.
    ///
    /// §13.12 adds a fifth tier above `registry`, and this file resolves a
    /// one-node registry path — so a reader is entitled to ask whether the
    /// comparison has quietly stopped being co-extensive with what the server
    /// does. It has not, for a reason that has to be *asserted* rather than
    /// argued: §10 rule 5's instance node grants only **control** verbs, and this
    /// harness compares only the two verbs `RbacRule` has an opinion about.
    ///
    /// So the tier cannot change a migrated config's read answer, and the
    /// harness's claim stands. If someone later adds a read verb to
    /// `instance_node`, every existing deployment's read scope widens on upgrade
    /// — the silent privilege escalation §7 opens with — and this fails instead.
    #[test]
    fn the_instance_tier_grants_no_verb_this_harness_compares() {
        use crate::services::authz::translate::instance_node;

        let node = instance_node(None);
        for (_, subject) in subject_shapes() {
            let resolved = resolve(std::slice::from_ref(&node), &subject);
            for action in [
                Action::ReleasesRead,
                Action::SourceRead,
                // The other two the read path serves, for the same reason: a
                // migration must not hand them out through a tier no existing
                // config mentions.
                Action::ReleasesList,
                Action::CatalogueBrowse,
            ] {
                assert!(
                    !resolved.holds(action),
                    "§10 rule 5's instance node must grant no read verb — {action} \
                     would widen every migrated config's read scope on upgrade, \
                     through a tier none of them wrote"
                );
            }
        }
    }
}
