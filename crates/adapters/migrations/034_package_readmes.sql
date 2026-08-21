-- A package version's own account of itself (RFC 0007).
--
-- Keyed by the coordinate, because a README describes the code that shipped
-- with it: a package whose 2.x README documents an API the 1.x README does not
-- is the normal case. A package-level column would show 2.x's API to a 1.x
-- reader, and the version key cannot be retrofitted without a migration that
-- cannot backfill.
--
-- The **source** is stored, never the rendered HTML. A fix to the sanitiser then
-- applies to everything already here — the render cache is keyed by `digest`
-- plus a renderer version, so bumping that version invalidates every rendering
-- in one commit with no backfill — and an operator can still read what the
-- package actually said rather than a transformation of it.
--
-- **No foreign key**, deliberately. A README outlives the bytes: the catalogue
-- already describes versions it holds none of (`ResolutionState::Pending`), and
-- a panel that emptied itself when LRU eviction ran would be inexplicable. A
-- cascade from anything evictable would do exactly that. Deletion is explicit,
-- from `delete_for_version` / `delete_for_package`.
--
-- Nothing here holds a row for a version this instance only knows about from an
-- upstream document. That answer is derived from the metadata cache on each
-- read, because a row written because somebody browsed a page has nothing that
-- would ever delete it (RFC 0007 §5.6).
CREATE TABLE IF NOT EXISTS package_readmes (
    registry     TEXT        NOT NULL,
    package_name TEXT        NOT NULL,
    version      TEXT        NOT NULL,
    -- The source exactly as it arrived, truncated at the registry's `max_bytes`
    -- and no more.
    content      TEXT        NOT NULL,
    -- 'markdown' | 'html' | 'rst' | 'plain' — what the source *is*, not how it
    -- should be displayed.
    format       TEXT        NOT NULL,
    -- 'upstream-metadata' | 'archive' | 'local-publish'.
    source       TEXT        NOT NULL,
    -- Hex SHA-256 of `content`: the render-cache key, and the change detector
    -- that decides whether a re-resolve actually replaced anything.
    digest       TEXT        NOT NULL,
    -- The source hit `max_bytes` and `content` is a prefix. Surfaced to the
    -- reader, never silent.
    truncated    BOOLEAN     NOT NULL DEFAULT FALSE,
    -- The text was the *package's*, not this version's: npm's packument carries
    -- a `readme` at the document root as well as per version. It is attributed
    -- to the version `dist-tags.latest` names — never invented for another one —
    -- and the panel says so, because presenting a package-level document as a
    -- per-version fact is a guess dressed as an answer (RFC 0007, decision 6).
    package_level BOOLEAN    NOT NULL DEFAULT FALSE,
    -- When *this instance* read the text, not when upstream published it.
    extracted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (registry, package_name, version)
);

-- The two per-package reads: the state of every version for the detail page's
-- version table, and the newest version that has one for the fallback rule.
-- Both ask by package, never by version alone, so the primary key's leading
-- columns would serve — but the fallback orders by `extracted_at`, and this
-- keeps that ordering off a sort of the whole package.
CREATE INDEX IF NOT EXISTS idx_package_readmes_package
    ON package_readmes (registry, package_name, extracted_at DESC);
