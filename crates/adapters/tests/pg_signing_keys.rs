//! RFC 0015 §4.2 — provider signing keys, asserted against **both** stores.
//!
//! One body of assertions, run over Postgres and the in-memory double, for the
//! reason §13.5 gives about `pg_grants.rs`: agreement between an adapter and its
//! double is not evidence. Survey finding 2 shipped when an empty list meant
//! "everything" in four repository implementations *"that all agreed with each
//! other"*.
//!
//! Requires a running PostgreSQL instance; set `DATABASE_URL` to opt in. Without
//! it the Postgres arm is skipped and the in-memory arm still runs, so the file
//! is never silently green.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use batlehub_adapters::db::packages::PoolOptions;
use batlehub_adapters::db::{PgPackageRepository, PgSigningKeyStore};
use batlehub_adapters::in_memory::InMemorySigningKeyStore;
use batlehub_core::{entities::SigningKey, ports::SigningKeyPort};

static TEST_ID: AtomicU64 = AtomicU64::new(0);

fn registry() -> String {
    format!(
        "keys-{}-{}",
        std::process::id(),
        TEST_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn key(id: &str) -> SigningKey {
    SigningKey {
        key_id: id.to_owned(),
        ascii_armor: format!("-----BEGIN PGP PUBLIC KEY BLOCK-----\n{id}\n-----END-----"),
        trust_signature: None,
        source: None,
        source_url: None,
    }
}

/// Both stores, so one body of assertions covers the pair.
async fn stores() -> Vec<(&'static str, Arc<dyn SigningKeyPort>)> {
    let mut out: Vec<(&'static str, Arc<dyn SigningKeyPort>)> =
        vec![("in-memory", InMemorySigningKeyStore::new())];
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let repo = PgPackageRepository::new(
            &url,
            PoolOptions {
                max_connections: 4,
                min_connections: 1,
                acquire_timeout_secs: 10,
            },
        )
        .await
        .expect("connect to postgres");
        repo.run_migrations().await.expect("run migrations");
        out.push(("postgres", Arc::new(PgSigningKeyStore::new(repo.pool()))));
    } else {
        eprintln!("skipping the postgres arm: DATABASE_URL not set");
    }
    out
}

/// A namespace with no key registered answers an empty list, not an error.
///
/// This is the shipped state of every estate before the store existed, and the
/// download path serves it exactly as it did — §10's promise, on the one path
/// that was already returning this value from a hardcoded literal.
#[tokio::test]
async fn a_namespace_with_no_keys_answers_empty() {
    for (label, store) in stores().await {
        let reg = registry();
        assert!(
            store
                .list_signing_keys(&reg, "hashicorp")
                .await
                .unwrap()
                .is_empty(),
            "{label}"
        );
    }
}

/// A key round-trips, and reaches only the namespace it was registered for.
#[tokio::test]
async fn a_key_is_scoped_to_its_namespace() {
    for (label, store) in stores().await {
        let reg = registry();
        store
            .set_signing_key(&reg, "hashicorp", key("AAAA"))
            .await
            .unwrap();

        let keys = store.list_signing_keys(&reg, "hashicorp").await.unwrap();
        assert_eq!(keys.len(), 1, "{label}");
        assert_eq!(keys[0].key_id, "AAAA", "{label}");
        assert!(
            keys[0].ascii_armor.contains("BEGIN PGP PUBLIC KEY BLOCK"),
            "{label}"
        );

        assert!(
            store
                .list_signing_keys(&reg, "someone-else")
                .await
                .unwrap()
                .is_empty(),
            "{label}: a key must not leak to a neighbouring namespace"
        );
    }
}

/// Re-registering the same id **replaces** the armour rather than appending.
///
/// That is what a rotation keeping its key id looks like, and the alternative is
/// two rows for one id — which would make the download response's key list depend
/// on read order. Asserted on both stores because it is the one place the
/// in-memory double could plausibly have been written to append.
#[tokio::test]
async fn re_registering_an_id_replaces_rather_than_appends() {
    for (label, store) in stores().await {
        let reg = registry();
        store
            .set_signing_key(&reg, "ns", key("AAAA"))
            .await
            .unwrap();

        let mut rotated = key("AAAA");
        rotated.ascii_armor = "-----BEGIN PGP PUBLIC KEY BLOCK-----\nrotated\n-----END-----".into();
        store.set_signing_key(&reg, "ns", rotated).await.unwrap();

        let keys = store.list_signing_keys(&reg, "ns").await.unwrap();
        assert_eq!(keys.len(), 1, "{label}: one id, one key");
        assert!(keys[0].ascii_armor.contains("rotated"), "{label}");
    }
}

/// Several keys coexist, in registration order, and one can be removed.
///
/// Order matters because the two stores have to agree about it: Postgres orders
/// by `id` and the double preserves insertion, and a client reading the list has
/// no way to tell which store answered.
#[tokio::test]
async fn keys_keep_their_order_and_delete_individually() {
    for (label, store) in stores().await {
        let reg = registry();
        for id in ["AAAA", "BBBB", "CCCC"] {
            store.set_signing_key(&reg, "ns", key(id)).await.unwrap();
        }
        let ids: Vec<String> = store
            .list_signing_keys(&reg, "ns")
            .await
            .unwrap()
            .into_iter()
            .map(|k| k.key_id)
            .collect();
        assert_eq!(ids, vec!["AAAA", "BBBB", "CCCC"], "{label}");

        store.delete_signing_key(&reg, "ns", "BBBB").await.unwrap();
        let ids: Vec<String> = store
            .list_signing_keys(&reg, "ns")
            .await
            .unwrap()
            .into_iter()
            .map(|k| k.key_id)
            .collect();
        assert_eq!(ids, vec!["AAAA", "CCCC"], "{label}");

        // Absent is not an error — a delete is idempotent, like every other one
        // in this tree.
        store.delete_signing_key(&reg, "ns", "BBBB").await.unwrap();
    }
}

/// A key that verifies nothing is refused before it is stored.
///
/// Not a cryptographic check: this server does not parse the key, and pretending
/// to would be worse than not. It refuses the two shapes that are definitely
/// useless, because a registry serving either tells Terraform to verify against
/// nothing *while looking configured* — which is the empty-placeholder state this
/// whole feature exists to leave.
#[test]
fn a_key_that_verifies_nothing_is_refused() {
    assert!(key("AAAA").validate().is_ok());

    let mut no_id = key("AAAA");
    no_id.key_id = "  ".to_owned();
    assert!(no_id.validate().is_err());

    let mut not_armoured = key("AAAA");
    not_armoured.ascii_armor = "just some text".to_owned();
    assert!(not_armoured.validate().is_err());
}
