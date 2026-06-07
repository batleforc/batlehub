#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

# ── Colors ────────────────────────────────────────────────────────────────────
if [[ -t 1 ]] && [[ -z "${NO_COLOR:-}" ]]; then
    GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[1;33m'
    BOLD='\033[1m'; RESET='\033[0m'
else
    GREEN=''; RED=''; YELLOW=''; BOLD=''; RESET=''
fi

REPO_URL="https://git.batleforc.fr/batleforc/batlehub"
SEMVER_RE='^[0-9]+\.[0-9]+\.[0-9]+$'
RELEASE_FILES=(Cargo.toml Cargo.lock ui/package.json helm/batlehub/Chart.yaml CHANGELOG.md)

# ── Usage ─────────────────────────────────────────────────────────────────────
usage() {
    cat <<EOF
Usage: $(basename "$0") <version> [options]
       $(basename "$0") --major|--minor|--patch [options]

Prepare a BatleHub release: bump every version-bearing file (Cargo workspace,
Cargo.lock, UI package.json, Helm chart), cut the [Unreleased] changelog
section into a dated release section, then create a local commit + annotated
tag. Never pushes — publishing the tag triggers the public release pipeline.

Arguments:
  <version>     Explicit target version, e.g. 0.2.0 (a leading "v" is accepted and stripped)

Bump modes (computed from the current [workspace.package] version in Cargo.toml):
  --major       1.2.3 -> 2.0.0
  --minor       1.2.3 -> 1.3.0
  --patch       1.2.3 -> 1.2.4

Options:
  --dry-run      Perform every edit but stop before commit/tag, leaving the
                 working tree modified so you can review it (discard with
                 'git checkout -- <files>')
  --skip-checks  Skip the cargo fmt / clippy / test quality gate
  -h, --help     Show this help

Examples:
  $(basename "$0") 0.2.0
  $(basename "$0") --minor
  $(basename "$0") --dry-run --skip-checks --patch
EOF
}

# ── Output helpers ────────────────────────────────────────────────────────────
step() { printf "\n${BOLD}==> %s${RESET}\n" "$*"; }
info() { printf "  %s\n" "$*"; }
ok()   { printf "  ${GREEN}done${RESET}  %s\n" "$*"; }
warn() { printf "  ${YELLOW}note${RESET}  %s\n" "$*"; }
die()  { printf "${RED}error:${RESET} %s\n" "$*" >&2; exit 1; }

# ── Argument parsing ──────────────────────────────────────────────────────────
EXPLICIT_VERSION=""
BUMP_MODE=""
DRY_RUN=0
SKIP_CHECKS=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --major|--minor|--patch)
            [[ -n "$BUMP_MODE" ]] && die "Only one of --major/--minor/--patch may be given"
            BUMP_MODE="${1#--}"
            shift
            ;;
        --dry-run)     DRY_RUN=1;     shift ;;
        --skip-checks) SKIP_CHECKS=1; shift ;;
        -h|--help)     usage; exit 0 ;;
        --*) die "Unknown option: $1" ;;
        *)
            [[ -n "$EXPLICIT_VERSION" ]] && die "Only one version argument may be given"
            EXPLICIT_VERSION="$1"
            shift
            ;;
    esac
done

if [[ -n "$EXPLICIT_VERSION" && -n "$BUMP_MODE" ]]; then
    die "Pass either an explicit version or a bump flag (--major/--minor/--patch), not both"
fi
if [[ -z "$EXPLICIT_VERSION" && -z "$BUMP_MODE" ]]; then
    usage >&2
    die "Missing version: pass an explicit version or one of --major/--minor/--patch"
fi

# ── Repo root check ───────────────────────────────────────────────────────────
[[ -f Cargo.toml ]] && grep -q '^\[workspace\]' Cargo.toml \
    || die "Run this from the repository root (Cargo.toml with [workspace] not found here)"

# ── Resolve current and target version ───────────────────────────────────────
CURRENT_VERSION=$(awk '
    /^\[workspace\.package\]/ { in_block=1; next }
    /^\[/                     { in_block=0 }
    in_block && match($0, /^version[[:space:]]*=[[:space:]]*"([^"]+)"/, m) { print m[1]; exit }
' Cargo.toml)
[[ "$CURRENT_VERSION" =~ $SEMVER_RE ]] \
    || die "Could not parse [workspace.package] version from Cargo.toml (got '${CURRENT_VERSION}')"

if [[ -n "$EXPLICIT_VERSION" ]]; then
    NEW_VERSION="${EXPLICIT_VERSION#v}"
    [[ "$NEW_VERSION" =~ $SEMVER_RE ]] || die "Version must be in X.Y.Z form (got '${EXPLICIT_VERSION}')"
else
    IFS='.' read -r cur_major cur_minor cur_patch <<< "$CURRENT_VERSION"
    case "$BUMP_MODE" in
        major) NEW_VERSION="$((cur_major + 1)).0.0" ;;
        minor) NEW_VERSION="${cur_major}.$((cur_minor + 1)).0" ;;
        patch) NEW_VERSION="${cur_major}.${cur_minor}.$((cur_patch + 1))" ;;
        *) die "Invalid bump mode: ${BUMP_MODE}" ;;
    esac
fi
TAG="v${NEW_VERSION}"

version_gt() {
    local -a a b
    local v1="$1" v2="$2"
    IFS='.' read -r -a a <<< "$v1"
    IFS='.' read -r -a b <<< "$v2"
    for i in 0 1 2; do
        (( a[i] > b[i] )) && return 0
        (( a[i] < b[i] )) && return 1
    done
    return 1
}
version_gt "$NEW_VERSION" "$CURRENT_VERSION" \
    || die "New version (${NEW_VERSION}) must be greater than the current version (${CURRENT_VERSION})"

step "Release plan"
info "Current version : ${CURRENT_VERSION}"
info "New version     : ${NEW_VERSION}"
info "Tag             : ${TAG}"

# ── Pre-flight checks ─────────────────────────────────────────────────────────
step "Pre-flight checks"

[[ -z "$(git status --porcelain)" ]] || die "Working tree is not clean — commit or stash changes first"
ok "Working tree is clean"

CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
[[ "$CURRENT_BRANCH" == "main" ]] || die "Must be on 'main' to prepare a release (currently on '${CURRENT_BRANCH}')"
ok "On branch 'main'"

git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null && die "Tag ${TAG} already exists"
ok "Tag ${TAG} does not exist yet"

# ── Quality gate ──────────────────────────────────────────────────────────────
if (( SKIP_CHECKS )); then
    warn "Skipping quality gate (--skip-checks)"
else
    step "Quality gate"
    info "cargo fmt --all --check"
    cargo fmt --all --check
    info "cargo clippy --workspace -- -D warnings"
    cargo clippy --workspace -- -D warnings
    info "cargo test --workspace"
    cargo test --workspace
    info "ui: npm run lint"
    (cd ui && npm run lint)
    info "ui: npm run format:check"
    (cd ui && npm run format:check)
    ok "fmt, clippy, tests, and UI lint/format checks passed"
fi

# ── Bump version strings ──────────────────────────────────────────────────────
step "Bumping version strings (${CURRENT_VERSION} -> ${NEW_VERSION})"

sed -i "/^\[workspace\.package\]/,/^\[/{s/^version = \"[^\"]*\"/version = \"${NEW_VERSION}\"/}" Cargo.toml
ok "Cargo.toml"

sed -i "0,/\"version\":/{s/\"version\": \"[^\"]*\"/\"version\": \"${NEW_VERSION}\"/}" ui/package.json
ok "ui/package.json"

sed -i \
    -e "s/^version: .*/version: ${NEW_VERSION}/" \
    -e "s/^appVersion: .*/appVersion: \"${NEW_VERSION}\"/" \
    helm/batlehub/Chart.yaml
ok "helm/batlehub/Chart.yaml"

info "Regenerating Cargo.lock (cargo check --workspace)"
cargo check --workspace --quiet
ok "Cargo.lock"

# ── Cut the changelog ─────────────────────────────────────────────────────────
step "Cutting CHANGELOG.md"

grep -qx '## \[Unreleased\]' CHANGELOG.md \
    || die "CHANGELOG.md: '## [Unreleased]' heading not found"
grep -q '^\[Unreleased\]: ' CHANGELOG.md \
    || die "CHANGELOG.md: '[Unreleased]' link reference not found"

RELEASE_DATE=$(date +%F)
PREV_TAG=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)
if [[ -n "$PREV_TAG" ]]; then
    VERSION_LINK="${REPO_URL}/compare/${PREV_TAG}...${TAG}"
else
    VERSION_LINK="${REPO_URL}/releases/tag/${TAG}"
fi

CHANGELOG_TMP=$(mktemp)
awk -v version="$NEW_VERSION" -v date="$RELEASE_DATE" \
    -v repo="$REPO_URL" -v tag="$TAG" -v version_link="$VERSION_LINK" '
    $0 == "## [Unreleased]" {
        print "## [Unreleased]"
        print ""
        print "## [" version "] - " date
        next
    }
    /^\[Unreleased\]: / {
        print "[Unreleased]: " repo "/compare/" tag "...HEAD"
        print "[" version "]: " version_link
        next
    }
    { print }
' CHANGELOG.md > "$CHANGELOG_TMP"
mv "$CHANGELOG_TMP" CHANGELOG.md
ok "CHANGELOG.md (## [${NEW_VERSION}] - ${RELEASE_DATE}, fresh ## [Unreleased])"

# ── Review ────────────────────────────────────────────────────────────────────
step "Changes"
git --no-pager diff --stat -- "${RELEASE_FILES[@]}"

if (( DRY_RUN )); then
    files_list=$(printf '%s ' "${RELEASE_FILES[@]}")
    warn "Dry run — working tree left modified for review."
    info "Discard with: git checkout -- ${files_list% }"
    info "Re-run without --dry-run to commit and tag ${TAG}."
    exit 0
fi

# ── Commit and tag ────────────────────────────────────────────────────────────
step "Committing and tagging"
git add "${RELEASE_FILES[@]}"
git commit --quiet -m "chore(release): ${TAG}"
git tag -a "${TAG}" -m "${TAG}"
ok "Created commit and annotated tag ${TAG}"

step "Next steps"
cat <<EOF
  Release ${TAG} is prepared locally on branch '${CURRENT_BRANCH}'.
  Review it (git show HEAD, git show ${TAG}), then publish with:

    git push origin ${CURRENT_BRANCH}
    git push origin ${TAG}

  Pushing the tag triggers the release pipeline (.github/workflows/build.yaml):
  server binary, CLI archives, container image, and Helm chart are built and
  published to GHCR — this step is not reversible.
EOF
