//! Print the listing-filter coverage table for `docs/guide/admin-policies.md`.
//!
//! The table used to be a warning box in the admin guide naming npm as the only
//! filtered ecosystem — a fact about the code that nothing kept true. It is now
//! generated from [`RegistryKind::listing_filter`], which is an exhaustive match
//! that a new registry kind cannot be added without answering, and which
//! `blocking`'s own tests check the filters against.
//!
//! Run through `task docs:listing-coverage`; `task docs:listing-coverage:check`
//! fails the build if the committed page has drifted.

use batlehub_core::entities::{ListingSupport, RegistryKind};

fn main() {
    println!("| Registry | Listing document | Blocked versions hidden |");
    println!("| --- | --- | --- |");

    for kind in RegistryKind::ALL {
        let docs = kind.listing_filter();
        if docs.is_empty() {
            println!("| {} | — | no listing document |", kind.as_str());
            continue;
        }
        for doc in docs {
            let verdict = match doc.support {
                ListingSupport::Filtered => "yes".to_owned(),
                ListingSupport::Qualified(note) => format!("yes — {note}"),
                ListingSupport::Unsupported(reason) => format!("no — {reason}"),
            };
            println!("| {} | {} | {} |", kind.as_str(), doc.label, verdict);
        }
    }
}
