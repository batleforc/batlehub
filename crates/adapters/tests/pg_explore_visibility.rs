//! Integration tests for per-package visibility filtering in the explore
//! queries (`PgPackageRepository::explore_packages` / `count_explore_packages`).
//!
//! **These cannot be written against the in-memory harness.**
//! `PackageRepository::explore_packages` has a default trait impl returning
//! `Ok(vec![])`, so `InMemoryPackageRepository` — the backend behind
//! `crates/web/tests/explore.rs` — returns an empty list for every query and
//! would pass whether or not the filter exists. The leak this guards against is
//! only observable against real SQL.
//!
//! Requires a running PostgreSQL instance. Set `DATABASE_URL` to opt in:
//!
//!   task test:pg-explore                             # starts Postgres via Podman automatically
//!   DATABASE_URL=postgresql://batlehub:changeme@localhost/batlehub \
//!     cargo test -p batlehub-adapters --test pg_explore_visibility

use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use sqlx::PgPool;

use batlehub_adapters::db::packages::PoolOptions;
use batlehub_adapters::db::{PgPackageRepository, PgTeamNamespaceStore};
use batlehub_adapters::local_registry::PostgresLocalRegistry;
use batlehub_core::{
    entities::{
        ExploreFilter, ExploreSortBy, ExploreViewer, PublishedPackage, TeamNamespace, Visibility,
    },
    ports::{LocalRegistryBackend, PackageRepository, TeamNamespacePort},
};

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

static TEST_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    repo: PgPackageRepository,
    local: PostgresLocalRegistry,
    namespaces: PgTeamNamespaceStore,
    /// Unique per test so parallel runs cannot see each other's rows.
    registry: String,
}

async fn fixture(url: &str) -> Fixture {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let repo = PgPackageRepository::new(
        url,
        PoolOptions {
            max_connections: 4,
            min_connections: 1,
            acquire_timeout_secs: 10,
        },
    )
    .await
    .expect("connect to postgres");
    repo.run_migrations().await.expect("run migrations");
    let pool: PgPool = repo.pool();
    Fixture {
        local: PostgresLocalRegistry::new(pool.clone()),
        namespaces: PgTeamNamespaceStore::new(pool),
        repo,
        registry: format!("vis-{pid}-{id}"),
    }
}

impl Fixture {
    async fn publish(&self, name: &str, visibility: Visibility) {
        let pkg = PublishedPackage {
            registry: self.registry.clone(),
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            checksum: format!("{:064x}", 0u64),
            yanked: false,
            deprecated: false,
            deprecation_message: None,
            unlisted: false,
            index_metadata: serde_json::json!({ "name": name, "version": "1.0.0" }),
            published_at: Utc::now(),
            published_by: Some("test-user".to_owned()),
            signature_bytes: None,
            signature_type: None,
            visibility,
        };
        // `publish` inserts in the *pending* state, which the explore queries
        // filter out via `status = 'published'`; `commit_publish` is what makes
        // the row visible. Skipping it would make every one of these tests pass
        // vacuously against an empty result set.
        self.local.publish(pkg).await.expect("publish");
        self.local
            .commit_publish(&self.registry, name, "1.0.0")
            .await
            .expect("commit publish");
    }

    async fn claim(&self, prefix: &str, group_id: &str) {
        self.namespaces
            .claim_namespace(TeamNamespace {
                registry: self.registry.clone(),
                prefix: prefix.to_owned(),
                group_id: group_id.to_owned(),
                claimed_by: Some("test-admin".to_owned()),
            })
            .await
            .expect("claim namespace");
    }

    fn filter(&self, viewer: ExploreViewer) -> ExploreFilter {
        ExploreFilter {
            registry: Some(self.registry.clone()),
            registries: vec![],
            name_contains: None,
            sort_by: ExploreSortBy::Name,
            limit: 100,
            offset: 0,
            viewer,
        }
    }

    /// Package names this viewer can see, sorted. Also asserts the paired count
    /// query agrees — a total that disagrees with the page is its own bug.
    async fn visible_to(&self, viewer: ExploreViewer) -> Vec<String> {
        let entries = self
            .repo
            .explore_packages(self.filter(viewer.clone()))
            .await
            .expect("explore_packages");
        let count = self
            .repo
            .count_explore_packages(self.filter(viewer))
            .await
            .expect("count_explore_packages");

        let mut names: Vec<String> = entries.into_iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(
            count as usize,
            names.len(),
            "count query must apply the same visibility predicate as the list query"
        );
        names
    }
}

fn anonymous() -> ExploreViewer {
    ExploreViewer::default()
}

fn user(groups: &[&str]) -> ExploreViewer {
    ExploreViewer {
        is_admin: false,
        is_authenticated: true,
        groups: groups.iter().map(|g| (*g).to_owned()).collect(),
    }
}

fn admin() -> ExploreViewer {
    ExploreViewer {
        is_admin: true,
        is_authenticated: true,
        groups: vec![],
    }
}

macro_rules! require_db {
    () => {
        match db_url() {
            Some(url) => url,
            None => {
                eprintln!("skipping: DATABASE_URL not set");
                return;
            }
        }
    };
}

#[tokio::test]
async fn public_is_visible_to_everyone_including_anonymous() {
    let url = require_db!();
    let f = fixture(&url).await;
    f.publish("open-pkg", Visibility::Public).await;

    assert_eq!(f.visible_to(anonymous()).await, vec!["open-pkg"]);
    assert_eq!(f.visible_to(user(&[])).await, vec!["open-pkg"]);
    assert_eq!(f.visible_to(admin()).await, vec!["open-pkg"]);
}

#[tokio::test]
async fn internal_is_hidden_from_anonymous_but_visible_once_authenticated() {
    let url = require_db!();
    let f = fixture(&url).await;
    f.publish("internal-pkg", Visibility::Internal).await;

    assert!(
        f.visible_to(anonymous()).await.is_empty(),
        "an internal package must not appear in an anonymous listing"
    );
    assert_eq!(f.visible_to(user(&[])).await, vec!["internal-pkg"]);
    assert_eq!(f.visible_to(admin()).await, vec!["internal-pkg"]);
}

#[tokio::test]
async fn team_is_visible_only_to_members_of_the_owning_group() {
    let url = require_db!();
    let f = fixture(&url).await;
    f.publish("team-a/secret", Visibility::Team).await;
    f.claim("team-a", "group-a").await;

    assert!(f.visible_to(anonymous()).await.is_empty());
    assert!(
        f.visible_to(user(&[])).await.is_empty(),
        "merely being authenticated must not reveal a team package"
    );
    assert!(
        f.visible_to(user(&["group-b"])).await.is_empty(),
        "membership of an unrelated group must not reveal a team package"
    );
    assert_eq!(
        f.visible_to(user(&["group-a"])).await,
        vec!["team-a/secret"]
    );
    assert_eq!(f.visible_to(admin()).await, vec!["team-a/secret"]);
}

/// Mirrors `check_team_visibility`'s `None` arm, which denies rather than
/// falling back to `Internal`. A deleted or never-created claim must not quietly
/// widen a team package to every authenticated user.
#[tokio::test]
async fn team_with_no_namespace_claim_is_visible_to_nobody_but_admins() {
    let url = require_db!();
    let f = fixture(&url).await;
    f.publish("unclaimed/pkg", Visibility::Team).await;
    // deliberately no claim_namespace call

    assert!(f.visible_to(anonymous()).await.is_empty());
    assert!(f.visible_to(user(&[])).await.is_empty());
    assert!(f.visible_to(user(&["group-a"])).await.is_empty());
    assert_eq!(f.visible_to(admin()).await, vec!["unclaimed/pkg"]);
}

/// The longest matching prefix wins **outright**: if the most specific claim
/// belongs to a group the viewer is not in, access is denied even though a
/// shorter claim they *are* in also matches. An `EXISTS` over all matching
/// claims would wrongly allow this.
#[tokio::test]
async fn longest_prefix_claim_wins_even_when_a_shorter_one_would_match() {
    let url = require_db!();
    let f = fixture(&url).await;
    f.publish("org/team-a/pkg", Visibility::Team).await;
    f.claim("org", "group-outer").await;
    f.claim("org/team-a", "group-inner").await;

    assert!(
        f.visible_to(user(&["group-outer"])).await.is_empty(),
        "the shorter 'org' claim must not grant access when 'org/team-a' is more specific"
    );
    assert_eq!(
        f.visible_to(user(&["group-inner"])).await,
        vec!["org/team-a/pkg"]
    );
}

/// `check_team_visibility` compares group ids with spaces stripped from both
/// sides; the SQL predicate has to do the same or a group named "Team A" would
/// match in one place and not the other.
#[tokio::test]
async fn group_comparison_ignores_spaces_on_both_sides() {
    let url = require_db!();
    let f = fixture(&url).await;
    f.publish("spaced/pkg", Visibility::Team).await;
    f.claim("spaced", "Team A").await;

    assert_eq!(f.visible_to(user(&["TeamA"])).await, vec!["spaced/pkg"]);
    assert_eq!(f.visible_to(user(&["Team A"])).await, vec!["spaced/pkg"]);
}

/// A namespace prefix containing a SQL `LIKE` metacharacter must be matched
/// literally. `LIKE prefix || '/%'` would let `a_c` claim `abc/...`, making the
/// listing more permissive than the download path.
#[tokio::test]
async fn like_metacharacters_in_a_prefix_are_matched_literally() {
    let url = require_db!();
    let f = fixture(&url).await;
    f.publish("abc/pkg", Visibility::Team).await;
    f.claim("a_c", "group-wildcard").await;

    assert!(
        f.visible_to(user(&["group-wildcard"])).await.is_empty(),
        "'a_c' must not match 'abc' — the underscore is a literal, not a wildcard"
    );
}

#[tokio::test]
async fn mixed_visibilities_are_filtered_independently() {
    let url = require_db!();
    let f = fixture(&url).await;
    f.publish("pub-pkg", Visibility::Public).await;
    f.publish("int-pkg", Visibility::Internal).await;
    f.publish("team-a/secret", Visibility::Team).await;
    f.claim("team-a", "group-a").await;

    assert_eq!(f.visible_to(anonymous()).await, vec!["pub-pkg"]);
    assert_eq!(f.visible_to(user(&[])).await, vec!["int-pkg", "pub-pkg"]);
    assert_eq!(
        f.visible_to(user(&["group-a"])).await,
        vec!["int-pkg", "pub-pkg", "team-a/secret"]
    );
    assert_eq!(
        f.visible_to(admin()).await,
        vec!["int-pkg", "pub-pkg", "team-a/secret"]
    );
}
