#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

# ── Colors ────────────────────────────────────────────────────────────────────
if [[ -t 1 ]] && [[ -z "${NO_COLOR:-}" ]]; then
    GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[1;33m'
    BOLD='\033[1m'; DIM='\033[2m'; RESET='\033[0m'
else
    GREEN=''; RED=''; YELLOW=''; BOLD=''; DIM=''; RESET=''
fi

# ── Usage ─────────────────────────────────────────────────────────────────────
usage() {
    cat <<EOF
Usage: $(basename "$0") [options]

Validate that each configured registry in a running batlehub instance
actually works end-to-end using real package manager tooling.

Options:
  --url <url>        Base URL of the running proxy (default: http://localhost:8080)
  --token <tok>      Bearer token for authenticated endpoints (optional)
  --npm <name>       Test npm registry named <name>
  --cargo <name>     Test cargo registry named <name>
  --go <name>        Test go registry named <name>
  --github <name>    Test github registry named <name>
  --openvsx <name>              Test openvsx registry named <name>
  --vscode-marketplace <name>   Test vscode-marketplace registry named <name>
  --maven <name>     Test maven registry named <name>
  --terraform <name> Test terraform registry named <name>
  --rubygems <name>  Test rubygems registry named <name>
  -h, --help         Show this help

Examples:
  # Test all registries using names from config.example.toml
  $(basename "$0") --npm npm --cargo cargo --go go --github github --openvsx openvsx \
    --maven maven --terraform terraform --rubygems gems

  # Test only npm and cargo against a remote instance with auth
  $(basename "$0") --url https://registry.example.com --token mytoken --npm my-npm --cargo my-cargo
EOF
}

# ── Defaults ──────────────────────────────────────────────────────────────────
BASE_URL="http://localhost:8080"
AUTH_TOKEN=""
NPM_NAME=""
CARGO_NAME=""
GO_NAME=""
GITHUB_NAME=""
OPENVSX_NAME=""
VSCODE_MARKETPLACE_NAME=""
MAVEN_NAME=""
TERRAFORM_NAME=""
RUBYGEMS_NAME=""

# ── Argument parsing ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --url)       BASE_URL="$2";       shift 2 ;;
        --token)     AUTH_TOKEN="$2";     shift 2 ;;
        --npm)       NPM_NAME="$2";       shift 2 ;;
        --cargo)     CARGO_NAME="$2";     shift 2 ;;
        --go)        GO_NAME="$2";        shift 2 ;;
        --github)    GITHUB_NAME="$2";    shift 2 ;;
        --openvsx)             OPENVSX_NAME="$2";             shift 2 ;;
        --vscode-marketplace)  VSCODE_MARKETPLACE_NAME="$2";  shift 2 ;;
        --maven)     MAVEN_NAME="$2";     shift 2 ;;
        --terraform) TERRAFORM_NAME="$2"; shift 2 ;;
        --rubygems)  RUBYGEMS_NAME="$2";  shift 2 ;;
        -h|--help)   usage; exit 0 ;;
        *) printf 'Unknown option: %s\n' "$1" >&2; usage >&2; exit 1 ;;
    esac
done

# ── Validate at least one registry was requested ──────────────────────────────
if [[ -z "$NPM_NAME" && -z "$CARGO_NAME" && -z "$GO_NAME" && -z "$GITHUB_NAME" && -z "$OPENVSX_NAME" && -z "$VSCODE_MARKETPLACE_NAME" && -z "$MAVEN_NAME" && -z "$TERRAFORM_NAME" && -z "$RUBYGEMS_NAME" ]]; then
    printf '%bNo registries specified. Use --npm, --cargo, --go, --github, --openvsx, --maven, --terraform, and/or --rubygems.%b\n' "$RED" "$RESET" >&2
    usage >&2
    exit 1
fi

# ── Global curl auth args ─────────────────────────────────────────────────────
CURL_AUTH=()
[[ -n "$AUTH_TOKEN" ]] && CURL_AUTH=(-H "Authorization: Bearer $AUTH_TOKEN")

# ── Temp dir + trap cleanup ───────────────────────────────────────────────────
TMPDIR_ROOT=$(mktemp -d)
cleanup() {
    local rc=$?
    sudo rm -rf "$TMPDIR_ROOT"
    exit $rc
}
trap cleanup EXIT
trap 'trap - EXIT; cleanup' INT TERM

# ── Result tracking ───────────────────────────────────────────────────────────
declare -A RESULTS=()
PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0

# ── Output helpers ────────────────────────────────────────────────────────────
print_pass()  { printf "  ${GREEN}PASS${RESET}  %s\n" "$*"; }
print_fail()  { printf "  ${RED}FAIL${RESET}  %s\n" "$*"; }
print_skip()  { printf "  ${YELLOW}SKIP${RESET}  %s\n" "$*"; }
print_warn()  { printf "  ${YELLOW}WARN${RESET}  %s\n" "$*"; }
section()     { printf "\n${BOLD}==> %s${RESET}\n" "$*"; }

record() {
    local reg="$1" label="$2" status="$3"
    RESULTS["${reg}:${label}"]="$status"
    case "$status" in
        PASS) PASS_COUNT=$(( PASS_COUNT + 1 )) ;;
        FAIL) FAIL_COUNT=$(( FAIL_COUNT + 1 )) ;;
        SKIP) SKIP_COUNT=$(( SKIP_COUNT + 1 )) ;;
    esac
}

tool_present() { command -v "$1" &>/dev/null; }

# ── HTTP helpers ──────────────────────────────────────────────────────────────

# decompress_if_gzip <file>
# The proxy streams cached artifacts as gzip without setting Content-Encoding,
# so curl cannot auto-decompress. Detect the gzip magic bytes and decompress
# in-place so jq always receives plain text.
decompress_if_gzip() {
    local file="$1"
    local first2
    first2=$(od -A n -t x1 -N 2 "$file" 2>/dev/null | tr -d ' \n')
    if [[ "$first2" == "1f8b" ]] && tool_present gunzip; then
        local tmp="${file}.dec"
        if gunzip -c "$file" > "$tmp" 2>/dev/null; then
            mv "$tmp" "$file"
        else
            rm -f "$tmp"
        fi
    fi
}

# http_check <label> <url> [<jq_expr> <expected>]
# Validates HTTP 200; optionally checks a jq expression equals expected.
# Body is written to a temp file (not captured via $()) to preserve binary
# content, then decompressed in-place if the proxy returned bare gzip.
http_check() {
    local label="$1" url="$2" jq_expr="${3:-}" expected="${4:-}"
    local body_file="${TMPDIR_ROOT}/http_body_${RANDOM}"
    local http_code rc=0

    http_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -o "$body_file" \
        -w '%{http_code}' "$url" 2>/dev/null) || rc=$?

    if (( rc != 0 )); then
        print_fail "$label: curl failed"
        rm -f "$body_file"
        return 1
    fi

    if [[ "$http_code" != "200" ]]; then
        print_fail "$label: HTTP $http_code (expected 200)"
        rm -f "$body_file"
        return 1
    fi

    decompress_if_gzip "$body_file"

    if [[ -n "$jq_expr" ]]; then
        local actual
        if tool_present jq; then
            actual=$(jq -r "$jq_expr" "$body_file" 2>/dev/null) || {
                local preview
                preview=$(tr -cd '[:print:]' < "$body_file" 2>/dev/null | head -c 120)
                print_fail "$label: failed to parse JSON (got: ${preview})"
                rm -f "$body_file"
                return 1
            }
        else
            # Fallback: grep for the expected value anywhere in the body
            actual=$(grep -o "\"${expected}\"" "$body_file" 2>/dev/null | head -1 | tr -d '"') || true
        fi

        if [[ -n "$expected" && "$actual" != "$expected" ]]; then
            print_fail "$label: expected $jq_expr == \"$expected\", got \"$actual\""
            rm -f "$body_file"
            return 1
        fi
    fi

    rm -f "$body_file"
    print_pass "$label"
    return 0
}

# http_check_not_5xx <label> <url>
# Accepts any non-5xx HTTP response (2xx, 3xx, 4xx all count as pass).
http_check_not_5xx() {
    local label="$1" url="$2"
    local http_code
    http_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -o /dev/null -w '%{http_code}' "$url" 2>/dev/null) || {
        print_fail "$label: curl failed"
        return 1
    }
    if [[ "$http_code" == 5* ]]; then
        print_fail "$label: HTTP $http_code (server error)"
        return 1
    fi
    print_pass "$label (HTTP $http_code)"
    return 0
}

# ── npm ───────────────────────────────────────────────────────────────────────
# The proxy streams the package tarball (.tgz) for every npm endpoint —
# /proxy/{name}/{pkg}, /proxy/{name}/{pkg}/{ver}, /proxy/{name}/{pkg}/{ver}/tarball.
# It is a binary download cache, not a packument-serving npm registry.
# Tests validate the gzip magic bytes and tar structure of the downloaded file.
test_npm() {
    local name="$1"
    section "npm registry: $name"

    # HTTP check — download the latest ms tarball and verify gzip magic bytes
    local npm_body="${TMPDIR_ROOT}/npm_http_${RANDOM}"
    local http_code rc=0
    http_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -o "$npm_body" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/ms" 2>/dev/null) || rc=$?

    local http_ok=true
    if (( rc != 0 )); then
        print_fail "npm:http — curl failed"; http_ok=false
    elif [[ "$http_code" != "200" ]]; then
        print_fail "npm:http — HTTP $http_code (expected 200)"; http_ok=false
    else
        local magic
        magic=$(od -A n -t x1 -N 2 "$npm_body" 2>/dev/null | tr -d ' \n')
        if [[ "$magic" == "1f8b" ]]; then
            print_pass "npm:http — ms tarball (gzip magic bytes confirmed)"
        else
            print_fail "npm:http — response is not a gzip tarball (magic: $magic)"
            http_ok=false
        fi
    fi
    rm -f "$npm_body"
    if $http_ok; then record npm http PASS; else record npm http FAIL; fi

    # Tool check — download a versioned tarball and verify its tar structure
    local tgz_file="${TMPDIR_ROOT}/npm_tool_${RANDOM}.tgz"
    rc=0
    http_code=$(curl -sS --max-time 60 "${CURL_AUTH[@]}" \
        -o "$tgz_file" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/ms/2.1.3/tarball" 2>/dev/null) || rc=$?

    if (( rc != 0 )); then
        print_fail "npm:tool — curl failed"
        rm -f "$tgz_file"
        record npm tool FAIL
    elif [[ "$http_code" != "200" ]]; then
        print_fail "npm:tool — ms@2.1.3/tarball: HTTP $http_code (expected 200)"
        rm -f "$tgz_file"
        record npm tool FAIL
    elif tool_present tar; then
        local tar_out tar_rc=0
        tar_out=$(tar tzf "$tgz_file" 2>&1) || tar_rc=$?
        rm -f "$tgz_file"
        if (( tar_rc == 0 )); then
            print_pass "npm:tool — ms@2.1.3 tarball is a valid .tgz"
            record npm tool PASS
        else
            print_fail "npm:tool — ms@2.1.3 tarball failed tar validation"
            printf '%bOutput:%b\n%s\n' "$DIM" "$RESET" "$(printf '%s' "$tar_out" | tail -5)"
            record npm tool FAIL
        fi
    else
        # tar not available — fall back to gzip magic bytes check
        local magic2
        magic2=$(od -A n -t x1 -N 2 "$tgz_file" 2>/dev/null | tr -d ' \n')
        rm -f "$tgz_file"
        if [[ "$magic2" == "1f8b" ]]; then
            print_pass "npm:tool — ms@2.1.3 tarball downloaded (tar unavailable for full check)"
            record npm tool PASS
        else
            print_fail "npm:tool — downloaded file is not a valid gzip (magic: $magic2)"
            record npm tool FAIL
        fi
    fi

    # Audit HTTP check — POST a minimal package tree to /-/npm/v1/audit/quick,
    # expect any non-5xx response (proxy layer is healthy if no server error).
    local audit_payload='{"name":"batlehub-check","version":"1.0.0","requires":{},"dependencies":{}}'
    local audit_body="${TMPDIR_ROOT}/npm_audit_${RANDOM}"
    local audit_code audit_rc=0
    audit_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -X POST -H "Content-Type: application/json" \
        -d "$audit_payload" \
        -o "$audit_body" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/-/npm/v1/audit/quick" 2>/dev/null) || audit_rc=$?
    rm -f "$audit_body"
    if (( audit_rc != 0 )); then
        print_fail "npm:audit-http — curl failed"
        record npm audit-http FAIL
    elif [[ "$audit_code" == 5* ]]; then
        print_fail "npm:audit-http — HTTP $audit_code (server error)"
        record npm audit-http FAIL
    else
        print_pass "npm:audit-http (HTTP $audit_code)"
        record npm audit-http PASS
    fi

    # Audit tool check — write a minimal lockfile and run npm audit through the proxy.
    # The proxy serves cached artifacts (gzip tarballs), not JSON packuments, so
    # `npm install` through it would fail. npm audit only needs a lockfile — it reads
    # the resolved dependency tree and POSTs it to the registry audit endpoint without
    # downloading any packages.
    if ! tool_present npm; then
        print_skip "npm:audit-tool — npm not installed"
        record npm audit-tool SKIP
    else
        local audit_dir="${TMPDIR_ROOT}/npm-audit-${name}"
        mkdir -p "$audit_dir"

        cat > "$audit_dir/package.json" <<'PKGJSON'
{"name":"batlehub-audit-check","version":"1.0.0","private":true,"dependencies":{"ms":"^2.1.3"}}
PKGJSON
        # Lockfile v3 (npm ≥ 7): npm audit reads this without fetching packages.
        cat > "$audit_dir/package-lock.json" <<'LOCKJSON'
{"name":"batlehub-audit-check","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"batlehub-audit-check","version":"1.0.0","dependencies":{"ms":"^2.1.3"}},"node_modules/ms":{"version":"2.1.3","resolved":"https://registry.npmjs.org/ms/-/ms-2.1.3.tgz","integrity":"sha512-6FlzubTLZG3J2a/NVCAleEhjzq5oxgHyaCU9yYXvcLsvoVaHJq/s5xXI6/XXP6tz7R9xAOtHnSO/tXtF3WRTg=="}}}
LOCKJSON

        local audit_out audit_exit=0
        audit_out=$(npm audit --prefix "$audit_dir" \
            --registry "${BASE_URL}/proxy/${name}" \
            --json 2>&1) || audit_exit=$?
        # exit 0 = no vulnerabilities, exit 1 = vulnerabilities found — both are valid
        # responses from the audit endpoint; only higher exit codes signal a failure.
        if (( audit_exit <= 1 )); then
            print_pass "npm:audit-tool — npm audit completed (exit $audit_exit)"
            record npm audit-tool PASS
        else
            print_fail "npm:audit-tool — npm audit failed (exit $audit_exit)"
            printf '%bOutput:%b\n%s\n' "$DIM" "$RESET" "$(printf '%s' "$audit_out" | tail -5)"
            record npm audit-tool FAIL
        fi
    fi
}

# ── Cargo ─────────────────────────────────────────────────────────────────────
test_cargo() {
    local name="$1"
    section "cargo registry: $name"

    # HTTP check
    local http_ok=true
    http_check "cargo:http — registry/config.json" \
        "${BASE_URL}/proxy/${name}/registry/config.json" \
        '.dl' '' || http_ok=false
    if $http_ok; then record cargo http PASS; else record cargo http FAIL; fi

    # Tool check
    if ! tool_present cargo; then
        print_skip "cargo:tool — cargo not installed"
        record cargo tool SKIP
        return
    fi

    # Require cargo >= 1.62 for stabilized `cargo add`
    local cargo_minor
    cargo_minor=$(cargo --version | grep -oP '1\.\K[0-9]+' | head -1) || cargo_minor=0
    if (( cargo_minor < 62 )); then
        print_skip "cargo:tool — cargo add requires Cargo >= 1.62 (found 1.${cargo_minor})"
        record cargo tool SKIP
        return
    fi

    local cargo_dir="${TMPDIR_ROOT}/cargo-${name}"
    local out rc=0

    out=$(cargo new --quiet --name batlehub-check "$cargo_dir" 2>&1) || {
        print_fail "cargo:tool — cargo new failed: $out"
        record cargo tool FAIL
        return
    }

    mkdir -p "${cargo_dir}/.cargo"
    cat > "${cargo_dir}/.cargo/config.toml" <<CARGOCONF
[registries.${name}]
index = "sparse+${BASE_URL}/proxy/${name}/registry/"

[source.crates-io]
replace-with = "${name}"

[source.${name}]
registry = "sparse+${BASE_URL}/proxy/${name}/registry/"
CARGOCONF

    # Auth: CARGO_REGISTRIES_<UPPER>_TOKEN env var
    local -a cargo_env=()
    if [[ -n "$AUTH_TOKEN" ]]; then
        local upper_name
        upper_name=$(printf '%s' "$name" | tr '[:lower:]-.' '[:upper:]__')
        cargo_env=("CARGO_REGISTRIES_${upper_name}_TOKEN=${AUTH_TOKEN}")
    fi

    # Must cd into the project dir so cargo finds .cargo/config.toml —
    # cargo reads config relative to the working directory, not --manifest-path.
    rc=0
    out=$(cd "$cargo_dir" && env "${cargo_env[@]}" cargo add serde \
        --registry "$name" 2>&1) || rc=$?

    if (( rc != 0 )); then
        print_fail "cargo:tool — cargo add serde (exit $rc)"
        printf '%bOutput:%b\n%s\n' "$DIM" "$RESET" "$(printf '%s' "$out" | tail -10)"
        record cargo tool FAIL
        return
    fi
    print_pass "cargo:tool — cargo add serde"

    # cargo add only resolves the index — it does not download the .crate file.
    # Run cargo fetch so the actual artifact is downloaded through the proxy,
    # which exercises the download endpoint, stores the artifact, and records
    # an audit-log event.
    rc=0
    out=$(cd "$cargo_dir" && env "${cargo_env[@]}" \
        CARGO_NET_OFFLINE=false \
        cargo fetch 2>&1) || rc=$?

    if (( rc == 0 )); then
        print_pass "cargo:tool — cargo fetch (crate downloaded via proxy)"
        record cargo tool PASS
    else
        print_fail "cargo:tool — cargo fetch (exit $rc)"
        printf '%bOutput:%b\n%s\n' "$DIM" "$RESET" "$(printf '%s' "$out" | tail -10)"
        record cargo tool FAIL
    fi
}

# ── Go ────────────────────────────────────────────────────────────────────────
test_go() {
    local name="$1"
    section "go registry: $name"

    # HTTP check — .Version field should be a non-empty string like "v0.x.y"
    local http_ok=true
    local version_url="${BASE_URL}/proxy/${name}/golang.org/x/text/@latest"
    local go_body_file="${TMPDIR_ROOT}/go_http_body_${RANDOM}"
    local http_code rc=0
    local go_version=""   # shared with tool check below

    http_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -o "$go_body_file" \
        -w '%{http_code}' "$version_url" 2>/dev/null) || { rc=$?; http_ok=false; }

    if $http_ok; then
        decompress_if_gzip "$go_body_file"
        if [[ "$http_code" != "200" ]]; then
            print_fail "go:http — golang.org/x/text/@latest: HTTP $http_code"
            http_ok=false
        else
            if tool_present jq; then
                go_version=$(jq -r '.Version // empty' "$go_body_file" 2>/dev/null) || true
            else
                go_version=$(grep -oP '"Version"\s*:\s*"\K[^"]+' "$go_body_file" 2>/dev/null || true)
            fi
            if [[ -z "$go_version" ]]; then
                print_fail "go:http — golang.org/x/text/@latest: .Version missing in response"
                http_ok=false
            else
                print_pass "go:http — golang.org/x/text/@latest ($go_version)"
            fi
        fi
    else
        print_fail "go:http — curl failed (exit $rc)"
    fi
    rm -f "$go_body_file"
    if $http_ok; then record go http PASS; else record go http FAIL; fi

    # Tool check
    if ! tool_present go; then
        print_skip "go:tool — go not installed"
        record go tool SKIP
        return
    fi

    # Use the exact version from the @latest response rather than @latest itself.
    # go get @latest internally also fetches /@v/list which may not be implemented;
    # a pinned version only needs /@v/{ver}.info and /@v/{ver}.mod.
    local go_target="golang.org/x/text"
    if [[ -n "$go_version" ]]; then
        go_target="golang.org/x/text@${go_version}"
    else
        go_target="golang.org/x/text@v0.14.0"   # fallback if http check skipped
    fi

    local go_dir="${TMPDIR_ROOT}/go-${name}"
    mkdir -p "$go_dir"

    local out rc=0
    out=$(cd "$go_dir" && go mod init batlehub-check 2>&1) || {
        print_fail "go:tool — go mod init failed: $out"
        record go tool FAIL
        return
    }

    local -a go_env=(
        "GOPROXY=${BASE_URL}/proxy/${name},off"
        "GONOSUMCHECK=*"
        "GONOSUMDB=*"
        "GOFLAGS="
        "HOME=${TMPDIR_ROOT}"
    )

    # Auth via .netrc (Go 1.21+ honours the NETRC env var)
    if [[ -n "$AUTH_TOKEN" ]]; then
        local netrc_file="${go_dir}/.netrc"
        local host_only="${BASE_URL##*://}"
        host_only="${host_only%%/*}"
        host_only="${host_only%%:*}"   # strip port — netrc machine matches on hostname
        printf 'machine %s login token password %s\n' "$host_only" "$AUTH_TOKEN" \
            > "$netrc_file"
        chmod 600 "$netrc_file"
        go_env+=("NETRC=${netrc_file}")
    fi

    rc=0
    out=$(cd "$go_dir" && env "${go_env[@]}" go get "$go_target" 2>&1) || rc=$?

    if (( rc == 0 )); then
        print_pass "go:tool — go get $go_target"
        record go tool PASS
    else
        print_fail "go:tool — go get $go_target (exit $rc)"
        printf '%bOutput:%b\n%s\n' "$DIM" "$RESET" "$(printf '%s' "$out" | tail -10)"
        record go tool FAIL
    fi
}

# ── GitHub ────────────────────────────────────────────────────────────────────
test_github() {
    local name="$1"
    section "github registry: $name"

    local asset_url="${BASE_URL}/proxy/${name}/cli/cli/releases/download/v2.48.0/gh_2.48.0_linux_amd64.tar.gz"

    # HTTP check — verify a well-known release asset is accessible.
    # Uses the asset download endpoint rather than the JSON tag metadata endpoint
    # because the metadata path hits the GitHub REST API (rate-limited at 60 req/hr
    # for unauthenticated callers), while the asset download path is cached.
    local http_code rc=0
    http_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -o /dev/null -w '%{http_code}' "$asset_url" 2>/dev/null) || rc=$?

    local http_ok=true
    if (( rc != 0 )); then
        print_fail "github:http — curl failed"; http_ok=false
    elif [[ "$http_code" == "200" ]]; then
        print_pass "github:http — asset download reachable (HTTP 200)"
    else
        print_fail "github:http — asset HTTP $http_code (expected 200)"; http_ok=false
    fi
    if $http_ok; then record github http PASS; else record github http FAIL; fi

    # Tool check — download the asset and verify it is a valid gzip tarball
    local asset_file="${TMPDIR_ROOT}/github_asset_${RANDOM}.tar.gz"
    rc=0
    http_code=$(curl -sS --max-time 60 "${CURL_AUTH[@]}" \
        -o "$asset_file" -w '%{http_code}' "$asset_url" 2>/dev/null) || rc=$?

    if (( rc != 0 )); then
        print_fail "github:tool — curl failed"
        rm -f "$asset_file"
        record github tool FAIL
    elif [[ "$http_code" != "200" ]]; then
        print_fail "github:tool — asset HTTP $http_code (expected 200)"
        rm -f "$asset_file"
        record github tool FAIL
    else
        local magic
        magic=$(od -A n -t x1 -N 2 "$asset_file" 2>/dev/null | tr -d ' \n')
        rm -f "$asset_file"
        if [[ "$magic" == "1f8b" ]]; then
            print_pass "github:tool — asset is a valid gzip tarball"
            record github tool PASS
        else
            print_fail "github:tool — downloaded file is not a valid gzip (magic: $magic)"
            record github tool FAIL
        fi
    fi
}

# ── OpenVSX ───────────────────────────────────────────────────────────────────
test_openvsx() {
    local name="$1"
    section "openvsx registry: $name"

    # HTTP check — any non-5xx is acceptable (upstream 404 is fine)
    local http_ok=true
    http_check_not_5xx "openvsx:http — redhat.java/1.26.0/vsix" \
        "${BASE_URL}/proxy/${name}/redhat.java/1.26.0/vsix" || http_ok=false
    if $http_ok; then record openvsx http PASS; else record openvsx http FAIL; fi

    # Tool check — download VSIX and verify ZIP magic bytes (VSIX files are ZIPs)
    local vsix_file="${TMPDIR_ROOT}/openvsx-${name}.vsix"
    local http_code rc=0
    http_code=$(curl -sS --max-time 60 "${CURL_AUTH[@]}" \
        -L -o "$vsix_file" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/redhat.java/1.26.0/vsix" 2>/dev/null) || rc=$?

    if (( rc != 0 )); then
        print_fail "openvsx:tool — download failed (curl error)"
        record openvsx tool FAIL
    elif [[ "$http_code" == "404" ]]; then
        print_skip "openvsx:tool — extension not found upstream (HTTP 404)"
        record openvsx tool SKIP
    elif [[ "$http_code" != "200" ]]; then
        print_fail "openvsx:tool — HTTP $http_code"
        record openvsx tool FAIL
    else
        # Check ZIP magic bytes: 50 4b 03 04
        local magic=""
        if tool_present xxd; then
            magic=$(xxd -l 4 "$vsix_file" 2>/dev/null | awk '{print $2$3}' | head -1) || true
            if [[ "$magic" == "504b0304" ]]; then
                print_pass "openvsx:tool — valid VSIX (ZIP magic bytes confirmed)"
                record openvsx tool PASS
            else
                print_fail "openvsx:tool — downloaded file is not a valid ZIP (magic: $magic)"
                record openvsx tool FAIL
            fi
        elif tool_present od; then
            magic=$(od -A x -t x1 -N 4 "$vsix_file" 2>/dev/null | awk 'NR==1{print $2$3$4$5}') || true
            if [[ "$magic" == "504b0304" ]]; then
                print_pass "openvsx:tool — valid VSIX (ZIP magic bytes confirmed)"
                record openvsx tool PASS
            else
                print_fail "openvsx:tool — downloaded file is not a valid ZIP (magic: $magic)"
                record openvsx tool FAIL
            fi
        else
            # Fall back to file size > 0
            local size
            size=$(wc -c < "$vsix_file")
            if (( size > 100 )); then
                print_pass "openvsx:tool — downloaded ${size} bytes (xxd/od unavailable for magic check)"
                record openvsx tool PASS
            else
                print_fail "openvsx:tool — downloaded file is suspiciously small (${size} bytes)"
                record openvsx tool FAIL
            fi
        fi
    fi
}

# ── VS Code Marketplace ───────────────────────────────────────────────────────
test_vscode_marketplace() {
    local name="$1"
    section "vscode-marketplace registry: $name"

    # HTTP check — any non-5xx is acceptable (upstream 404 is fine)
    local http_ok=true
    http_check_not_5xx "vscode-marketplace:http — ms-python.python/2024.2.1/vsix" \
        "${BASE_URL}/proxy/${name}/ms-python.python/2024.2.1/vsix" || http_ok=false
    if $http_ok; then record vscode-marketplace http PASS; else record vscode-marketplace http FAIL; fi

    # Tool check — download VSIX and verify ZIP magic bytes (VSIX files are ZIPs)
    local vsix_file="${TMPDIR_ROOT}/vscode-marketplace-${name}.vsix"
    local http_code rc=0
    http_code=$(curl -sS --max-time 60 "${CURL_AUTH[@]}" \
        -L -o "$vsix_file" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/ms-python.python/2024.2.1/vsix" 2>/dev/null) || rc=$?

    if (( rc != 0 )); then
        print_fail "vscode-marketplace:tool — download failed (curl error)"
        record vscode-marketplace tool FAIL
    elif [[ "$http_code" == "404" ]]; then
        print_skip "vscode-marketplace:tool — extension not found upstream (HTTP 404)"
        record vscode-marketplace tool SKIP
    elif [[ "$http_code" != "200" ]]; then
        print_fail "vscode-marketplace:tool — HTTP $http_code"
        record vscode-marketplace tool FAIL
    else
        # Check ZIP magic bytes: 50 4b 03 04
        local magic=""
        if tool_present xxd; then
            magic=$(xxd -l 4 "$vsix_file" 2>/dev/null | awk '{print $2$3}' | head -1) || true
            if [[ "$magic" == "504b0304" ]]; then
                print_pass "vscode-marketplace:tool — valid VSIX (ZIP magic bytes confirmed)"
                record vscode-marketplace tool PASS
            else
                print_fail "vscode-marketplace:tool — downloaded file is not a valid ZIP (magic: $magic)"
                record vscode-marketplace tool FAIL
            fi
        elif tool_present od; then
            magic=$(od -A x -t x1 -N 4 "$vsix_file" 2>/dev/null | awk 'NR==1{print $2$3$4$5}') || true
            if [[ "$magic" == "504b0304" ]]; then
                print_pass "vscode-marketplace:tool — valid VSIX (ZIP magic bytes confirmed)"
                record vscode-marketplace tool PASS
            else
                print_fail "vscode-marketplace:tool — downloaded file is not a valid ZIP (magic: $magic)"
                record vscode-marketplace tool FAIL
            fi
        else
            local size
            size=$(wc -c < "$vsix_file")
            if (( size > 100 )); then
                print_pass "vscode-marketplace:tool — downloaded ${size} bytes (xxd/od unavailable for magic check)"
                record vscode-marketplace tool PASS
            else
                print_fail "vscode-marketplace:tool — downloaded file is suspiciously small (${size} bytes)"
                record vscode-marketplace tool FAIL
            fi
        fi
    fi
    rm -f "$vsix_file"
}

# ── RubyGems ─────────────────────────────────────────────────────────────────
# RubyGems check strategy:
#   HTTP check  — GET /api/v1/gems/rake.json; verify .name == "rake"
#   Tool check  — download rake-13.2.1.gem; verify it is a valid tar archive
# No gem CLI required; both checks use curl only.
test_rubygems() {
    local name="$1"
    section "rubygems registry: $name"

    # HTTP check — gem info JSON for 'rake' (always present on rubygems.org)
    local rg_body="${TMPDIR_ROOT}/rg_http_${RANDOM}"
    local http_code rc=0
    http_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -o "$rg_body" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/api/v1/gems/rake.json" 2>/dev/null) || rc=$?

    local http_ok=true
    if (( rc != 0 )); then
        print_fail "rubygems:http — curl failed"; http_ok=false
    elif [[ "$http_code" != "200" ]]; then
        print_fail "rubygems:http — HTTP $http_code (expected 200)"; http_ok=false
    else
        decompress_if_gzip "$rg_body"
        local gem_name=""
        if tool_present jq; then
            gem_name=$(jq -r '.name // empty' "$rg_body" 2>/dev/null) || true
        else
            grep -q '"name":"rake"' "$rg_body" 2>/dev/null && gem_name="rake"
        fi
        if [[ "$gem_name" == "rake" ]]; then
            print_pass "rubygems:http — rake gem info (.name == \"rake\")"
        else
            print_fail "rubygems:http — .name != \"rake\" (got: \"${gem_name}\")"
            http_ok=false
        fi
    fi
    rm -f "$rg_body"
    if $http_ok; then record rubygems http PASS; else record rubygems http FAIL; fi

    # Tool check — download rake-13.2.1.gem and verify it is a tar archive
    # (.gem files are plain tar archives containing metadata.gz and data.tar.gz)
    local gem_file="${TMPDIR_ROOT}/rg_gem_${RANDOM}.gem"
    rc=0
    http_code=$(curl -sS --max-time 60 "${CURL_AUTH[@]}" \
        -o "$gem_file" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/gems/rake-13.2.1.gem" 2>/dev/null) || rc=$?

    if (( rc != 0 )); then
        print_fail "rubygems:tool — curl failed"
        rm -f "$gem_file"; record rubygems tool FAIL
    elif [[ "$http_code" != "200" ]]; then
        print_fail "rubygems:tool — rake-13.2.1.gem: HTTP $http_code (expected 200)"
        rm -f "$gem_file"; record rubygems tool FAIL
    elif tool_present tar; then
        local tar_rc=0
        tar tf "$gem_file" &>/dev/null || tar_rc=$?
        rm -f "$gem_file"
        if (( tar_rc == 0 )); then
            print_pass "rubygems:tool — rake-13.2.1.gem is a valid tar archive"
            record rubygems tool PASS
        else
            print_fail "rubygems:tool — downloaded file is not a valid tar archive"
            record rubygems tool FAIL
        fi
    else
        # tar not available — check the file is non-trivially sized
        local size
        size=$(wc -c < "$gem_file")
        rm -f "$gem_file"
        if (( size > 10240 )); then
            print_pass "rubygems:tool — rake-13.2.1.gem downloaded (${size} bytes, tar unavailable for full check)"
            record rubygems tool PASS
        else
            print_fail "rubygems:tool — downloaded file is suspiciously small (${size} bytes)"
            record rubygems tool FAIL
        fi
    fi
}

# ── Maven ─────────────────────────────────────────────────────────────────────
# Maven check strategy:
#   HTTP check  — fetch maven-metadata.xml for junit:junit (well-known, always in Central)
#   Tool check  — download junit-4.13.2.pom and verify it contains <project
# No Maven CLI required; both checks use curl only.
test_maven() {
    local name="$1"
    section "maven registry: $name"

    # HTTP check — maven-metadata.xml lists all published versions as XML
    local meta_file="${TMPDIR_ROOT}/maven_meta_${RANDOM}"
    local http_code rc=0
    http_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -o "$meta_file" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/maven2/junit/junit/maven-metadata.xml" 2>/dev/null) || rc=$?

    local http_ok=true
    if (( rc != 0 )); then
        print_fail "maven:http — curl failed"; http_ok=false
    elif [[ "$http_code" != "200" ]]; then
        print_fail "maven:http — HTTP $http_code (expected 200)"; http_ok=false
    elif ! grep -q '<metadata' "$meta_file" 2>/dev/null; then
        print_fail "maven:http — response is not valid maven-metadata.xml (no <metadata> element)"; http_ok=false
    else
        print_pass "maven:http — junit maven-metadata.xml"
    fi
    rm -f "$meta_file"
    if $http_ok; then record maven http PASS; else record maven http FAIL; fi

    # Tool check — download the junit-4.13.2 POM and verify it is valid XML
    local pom_file="${TMPDIR_ROOT}/maven_pom_${RANDOM}.pom"
    rc=0
    http_code=$(curl -sS --max-time 60 "${CURL_AUTH[@]}" \
        -o "$pom_file" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/maven2/junit/junit/4.13.2/junit-4.13.2.pom" 2>/dev/null) || rc=$?

    if (( rc != 0 )); then
        print_fail "maven:tool — curl failed"
        rm -f "$pom_file"; record maven tool FAIL
    elif [[ "$http_code" != "200" ]]; then
        print_fail "maven:tool — junit-4.13.2.pom: HTTP $http_code (expected 200)"
        rm -f "$pom_file"; record maven tool FAIL
    elif grep -q '<project' "$pom_file" 2>/dev/null; then
        print_pass "maven:tool — junit-4.13.2.pom is valid XML"
        rm -f "$pom_file"; record maven tool PASS
    else
        print_fail "maven:tool — downloaded file does not look like a valid POM"
        rm -f "$pom_file"; record maven tool FAIL
    fi
}

# ── Terraform ─────────────────────────────────────────────────────────────────
# Terraform check strategy:
#   HTTP check  — fetch provider version list for hashicorp/random (stable, small)
#   Tool check  — fetch download info JSON for a specific version; verify download_url present
# No Terraform CLI required; both checks use curl only.
test_terraform() {
    local name="$1"
    section "terraform registry: $name"

    # HTTP check — provider versions endpoint returns JSON with a .versions array
    local tf_body="${TMPDIR_ROOT}/tf_http_${RANDOM}"
    local http_code rc=0
    http_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -o "$tf_body" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/v1/providers/hashicorp/random/versions" 2>/dev/null) || rc=$?

    local http_ok=true
    if (( rc != 0 )); then
        print_fail "terraform:http — curl failed"; http_ok=false
    elif [[ "$http_code" != "200" ]]; then
        print_fail "terraform:http — HTTP $http_code (expected 200)"; http_ok=false
    else
        decompress_if_gzip "$tf_body"
        local has_versions=false
        if tool_present jq; then
            local count
            count=$(jq '.versions | length' "$tf_body" 2>/dev/null) || count=0
            (( count > 0 )) && has_versions=true
        else
            grep -q '"version"' "$tf_body" 2>/dev/null && has_versions=true
        fi
        if $has_versions; then
            print_pass "terraform:http — hashicorp/random versions"
        else
            print_fail "terraform:http — response contains no versions"
            http_ok=false
        fi
    fi
    rm -f "$tf_body"
    if $http_ok; then record terraform http PASS; else record terraform http FAIL; fi

    # Tool check — provider download info JSON for a specific version
    local tf_dl="${TMPDIR_ROOT}/tf_dl_${RANDOM}"
    rc=0
    http_code=$(curl -sS --max-time 30 "${CURL_AUTH[@]}" \
        -o "$tf_dl" -w '%{http_code}' \
        "${BASE_URL}/proxy/${name}/v1/providers/hashicorp/random/3.6.0/download/linux/amd64" 2>/dev/null) || rc=$?

    if (( rc != 0 )); then
        print_fail "terraform:tool — download info curl failed"
        rm -f "$tf_dl"; record terraform tool FAIL
    elif [[ "$http_code" == "404" ]]; then
        print_skip "terraform:tool — hashicorp/random 3.6.0 not found upstream (HTTP 404)"
        rm -f "$tf_dl"; record terraform tool SKIP
    elif [[ "$http_code" != "200" ]]; then
        print_fail "terraform:tool — download info HTTP $http_code (expected 200)"
        rm -f "$tf_dl"; record terraform tool FAIL
    else
        decompress_if_gzip "$tf_dl"
        local has_url=false
        if tool_present jq; then
            local dl_url
            dl_url=$(jq -r '.download_url // empty' "$tf_dl" 2>/dev/null) || true
            [[ -n "$dl_url" ]] && has_url=true
        else
            grep -q '"download_url"' "$tf_dl" 2>/dev/null && has_url=true
        fi
        rm -f "$tf_dl"
        if $has_url; then
            print_pass "terraform:tool — hashicorp/random 3.6.0 download info (linux/amd64)"
            record terraform tool PASS
        else
            print_fail "terraform:tool — download info JSON missing download_url"
            record terraform tool FAIL
        fi
    fi
}

# ── Summary table ─────────────────────────────────────────────────────────────
print_summary() {
    printf "\n${BOLD}%-20s  %-8s  %-8s${RESET}\n" "Registry" "HTTP" "Tool"
    printf '%-20s  %-8s  %-8s\n' "--------------------" "--------" "--------"

    local -a order=()
    [[ -n "$NPM_NAME"                ]] && order+=("npm")
    [[ -n "$CARGO_NAME"              ]] && order+=("cargo")
    [[ -n "$GO_NAME"                 ]] && order+=("go")
    [[ -n "$GITHUB_NAME"             ]] && order+=("github")
    [[ -n "$OPENVSX_NAME"            ]] && order+=("openvsx")
    [[ -n "$VSCODE_MARKETPLACE_NAME" ]] && order+=("vscode-marketplace")
    [[ -n "$MAVEN_NAME"              ]] && order+=("maven")
    [[ -n "$TERRAFORM_NAME"          ]] && order+=("terraform")
    [[ -n "$RUBYGEMS_NAME"           ]] && order+=("rubygems")

    for reg in "${order[@]}"; do
        local http_status="${RESULTS["${reg}:http"]:-SKIP}"
        local tool_status="${RESULTS["${reg}:tool"]:-SKIP}"

        local http_colored tool_colored
        case "$http_status" in
            PASS) http_colored="${GREEN}PASS${RESET}" ;;
            FAIL) http_colored="${RED}FAIL${RESET}" ;;
            SKIP) http_colored="${YELLOW}SKIP${RESET}" ;;
            *)    http_colored="$http_status" ;;
        esac
        case "$tool_status" in
            PASS) tool_colored="${GREEN}PASS${RESET}" ;;
            FAIL) tool_colored="${RED}FAIL${RESET}" ;;
            SKIP) tool_colored="${YELLOW}SKIP${RESET}" ;;
            *)    tool_colored="$tool_status" ;;
        esac

        printf "%-20s  %b%-8s%b  %b%-8s%b\n" \
            "$reg" \
            "" "$http_status" "" \
            "" "$tool_status" ""
        # Coloured version (printf doesn't count escape codes in width, so print separately)
        # Overwrite with coloured line using tput cursor-up if available
    done
}

print_summary_colored() {
    printf "\n${BOLD}%-20s  %-6s  %-6s${RESET}\n" "Registry" "HTTP" "Tool"
    printf "${DIM}%-20s  %-6s  %-6s${RESET}\n" "--------------------" "------" "------"

    local -a order=()
    [[ -n "$NPM_NAME"                ]] && order+=("npm")
    [[ -n "$CARGO_NAME"              ]] && order+=("cargo")
    [[ -n "$GO_NAME"                 ]] && order+=("go")
    [[ -n "$GITHUB_NAME"             ]] && order+=("github")
    [[ -n "$OPENVSX_NAME"            ]] && order+=("openvsx")
    [[ -n "$VSCODE_MARKETPLACE_NAME" ]] && order+=("vscode-marketplace")
    [[ -n "$MAVEN_NAME"              ]] && order+=("maven")
    [[ -n "$TERRAFORM_NAME"          ]] && order+=("terraform")
    [[ -n "$RUBYGEMS_NAME"           ]] && order+=("rubygems")

    for reg in "${order[@]}"; do
        local http_status="${RESULTS["${reg}:http"]:-SKIP}"
        local tool_status="${RESULTS["${reg}:tool"]:-SKIP}"

        local http_col tool_col
        case "$http_status" in
            PASS) http_col="$GREEN" ;; FAIL) http_col="$RED" ;; *) http_col="$YELLOW" ;;
        esac
        case "$tool_status" in
            PASS) tool_col="$GREEN" ;; FAIL) tool_col="$RED" ;; *) tool_col="$YELLOW" ;;
        esac

        printf "%-20s  ${http_col}%-6s${RESET}  ${tool_col}%-6s${RESET}\n" \
            "$reg" "$http_status" "$tool_status"
    done
}

# ── Main ──────────────────────────────────────────────────────────────────────
printf "${BOLD}batlehub registry check${RESET}  ${DIM}%s${RESET}\n" "$BASE_URL"
[[ -n "$AUTH_TOKEN" ]] && printf "${DIM}(using bearer token auth)${RESET}\n"

[[ -n "$NPM_NAME"                ]] && test_npm                "$NPM_NAME"
[[ -n "$CARGO_NAME"              ]] && test_cargo              "$CARGO_NAME"
[[ -n "$GO_NAME"                 ]] && test_go                 "$GO_NAME"
[[ -n "$GITHUB_NAME"             ]] && test_github             "$GITHUB_NAME"
[[ -n "$OPENVSX_NAME"            ]] && test_openvsx            "$OPENVSX_NAME"
[[ -n "$VSCODE_MARKETPLACE_NAME" ]] && test_vscode_marketplace "$VSCODE_MARKETPLACE_NAME"
[[ -n "$MAVEN_NAME"              ]] && test_maven              "$MAVEN_NAME"
[[ -n "$TERRAFORM_NAME"          ]] && test_terraform          "$TERRAFORM_NAME"
[[ -n "$RUBYGEMS_NAME"           ]] && test_rubygems           "$RUBYGEMS_NAME"

printf "\n${BOLD}Summary${RESET}\n"
print_summary_colored

printf "\n"
if (( FAIL_COUNT > 0 )); then
    printf "${RED}${BOLD}%d check(s) failed${RESET}  (passed: %d, skipped: %d)\n" \
        "$FAIL_COUNT" "$PASS_COUNT" "$SKIP_COUNT"
    exit 1
else
    printf "${GREEN}${BOLD}All checks passed${RESET}  (passed: %d, skipped: %d)\n" \
        "$PASS_COUNT" "$SKIP_COUNT"
    exit 0
fi
