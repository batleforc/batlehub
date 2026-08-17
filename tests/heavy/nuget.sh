#!/usr/bin/env bash
# Heavy NuGet integration test — the dotnet CLI, against a local registry.
#
# RFC 0009 §12.4 found two shipped defects here, and neither is a wrong path:
#
#   1. **The search endpoint was unreachable.** With the resource types BatleHub
#      advertised — bare `SearchQueryService` plus `SearchQueryService/3.5.0` —
#      `dotnet package search` answers *"The source does not have a Search
#      service!"* and never issues a query. It selects
#      `SearchQueryService/3.0.0-beta`. So phase 6 un-stubbed an endpoint the
#      client still could not find, and phase 7 added autocomplete with the same
#      omission. A conformance fixture asserts paths; this is a resource *type*,
#      and only the client's resolver reads it.
#   2. **`skip` was ignored.** The client paginates `skip=0&take=20`, then
#      `skip=20`. The query parser read `q` and `take` only, so every page
#      returned the same first results — which reads as "this registry has
#      twenty packages".
#
# So the assertions are: the client issues a query at all, it finds the package
# that was published a moment earlier, a second page is *different from the
# first*, and `dotnet add package` installs from here.
#
# Run via `task test:nuget-heavy` or directly.
#
# Environment knobs: DATABASE_URL (required), HEAVY_PORT (8086),
# HEAVY_TAP_PORT (8096), COVERAGE, DOTNET_VERSION (10) for the mise fallback.

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

heavy_init nuget 8086 8096
heavy_need python3 "python3"

DOTNET_VERSION="${DOTNET_VERSION:-10}"
heavy_runner_for dotnet "dotnet@$DOTNET_VERSION"
DOTNET=("${HEAVY_RUNNER[@]}" dotnet)

REGISTRY="nuget-local-$HEAVY_RUN"
# NuGet ids are case-insensitive and the registration path lowercases them, so
# the probe carries a capital to keep that in the measurement.
PKG_A="HeavyProbe.A$HEAVY_RUN"
PKG_B="HeavyProbe.B$HEAVY_RUN"

export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1
export DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
# Keep NuGet's global caches inside the run: a package restored from
# ~/.nuget/packages is a package this server was not asked for.
export NUGET_PACKAGES="$HEAVY_WORK/nuget-packages"
export NUGET_HTTP_CACHE_PATH="$HEAVY_WORK/nuget-http-cache"

heavy_start_server tests/heavy/config.nuget.toml
heavy_start_tap

INDEX="$HEAVY_TAP_BASE/proxy/$REGISTRY/nuget/v3/index.json"

# NuGet walks up from the working directory for `nuget.config`, so one file at
# the root of the work tree covers every `dotnet` call below — pack, push,
# search and restore all run inside it.
#
# `allowInsecureConnections` is not optional: NuGet refuses a plain-HTTP source
# outright, on push as well as on restore (RFC 0009 §12.4). A real deployment is
# HTTPS; a local one needs this line, and an operator who does not know that
# reads the refusal as "the registry is broken".
cat > "$HEAVY_WORK/nuget.config" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="batlehub" value="$INDEX" allowInsecureConnections="true" />
  </packageSources>
</configuration>
EOF

DOTNET_SDK="$("${DOTNET[@]}" --version)"
# Target the SDK's own framework. `netstandard2.0` would be the conventional
# choice for a library, but it restores `NETStandard.Library` from nuget.org —
# and this config `<clear/>`s every source but BatleHub on purpose, so the probe
# has to be buildable with what the SDK already carries. A restore that reaches
# nuget.org is a restore this test did not observe.
TFM="net${DOTNET_SDK%%.*}.0"
heavy_log "dotnet $DOTNET_SDK, targeting $TFM"

# ── 1. Two packages, so pagination has something to paginate ─────────────────

pack() {  # id -> echoes the .nupkg path
  # Two statements: `local a=1 b="$a"` expands every word *before* the builtin
  # assigns any of them, so `b` would read the outer (unset) `a` — an error
  # under `set -u`, and a silently empty path without it.
  local id="$1"
  local dir="$HEAVY_WORK/src/$id"
  mkdir -p "$dir"
  cat > "$dir/$id.csproj" <<EOF
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>$TFM</TargetFramework>
    <PackageId>$id</PackageId>
    <Version>1.0.0</Version>
    <Authors>batlehub heavy tests</Authors>
    <Description>RFC 0009 heavy test probe</Description>
    <IncludeBuildOutput>true</IncludeBuildOutput>
  </PropertyGroup>
</Project>
EOF
  echo "namespace Probe { public static class Marker { public const string Id = \"$id\"; } }" \
    > "$dir/Marker.cs"
  (cd "$dir" && "${DOTNET[@]}" pack -c Release -o "$dir/out") >>"$HEAVY_WORK/pack.log" 2>&1 \
    || { tail -30 "$HEAVY_WORK/pack.log" >&2; heavy_fail "dotnet pack failed for $id"; }
  echo "$dir/out/$id.1.0.0.nupkg"
}

heavy_log "Packing $PKG_A and $PKG_B"
NUPKG_A="$(pack "$PKG_A")"
NUPKG_B="$(pack "$PKG_B")"

push() {
  (cd "$HEAVY_WORK" && "${DOTNET[@]}" nuget push "$1" --source "$INDEX" --api-key "$ADMIN_TOKEN") \
    >>"$HEAVY_WORK/push.log" 2>&1 \
    || { tail -30 "$HEAVY_WORK/push.log" >&2; heavy_fail "dotnet nuget push failed for $1"; }
}

heavy_mark "push"
heavy_log "dotnet nuget push -> $REGISTRY"
push "$NUPKG_A"
push "$NUPKG_B"
# The trailing slash is pinned deliberately: it is the client's, not ours. The
# service index advertises `…/api/v2/package`, `dotnet nuget push` appends the
# slash, and the route that answered only the unslashed spelling 404'd every
# real push while every test in the repository passed (RFC 0009 §12.16).
heavy_wire_after "push" "PUT /proxy/$REGISTRY/nuget/api/v2/package/ -> 201" \
  "dotnet nuget push did not reach the publish endpoint"
# The service index is what told it where to push. Its absence from the
# transcript would mean the client used a path of its own invention.
heavy_wire "GET /proxy/$REGISTRY/nuget/v3/index.json -> 200" \
  "the client never read the service index"

# ── 2. Search — does the client issue a query, and does it find it ───────────

heavy_mark "search"
heavy_log "dotnet package search $PKG_A"
set +e
(cd "$HEAVY_WORK" && "${DOTNET[@]}" package search "$PKG_A" --source "$INDEX" --format json) \
  >"$HEAVY_WORK/search.json" 2>"$HEAVY_WORK/search.err"
SEARCH_RC=$?
set -e
if grep -qi "does not have a Search service" "$HEAVY_WORK/search.err" "$HEAVY_WORK/search.json"; then
  cat "$HEAVY_WORK/search.err" >&2
  heavy_fail "the client refused to search: the service index does not advertise a resource @type its resolver accepts (RFC 0009 §12.4)"
fi
[[ $SEARCH_RC -eq 0 ]] || { cat "$HEAVY_WORK/search.err" >&2; heavy_fail "dotnet package search failed"; }
heavy_wire_after "search" "GET /proxy/$REGISTRY/nuget/v3/query" \
  "no query was issued — the client could not select the search resource"

python3 - "$HEAVY_WORK/search.json" "$PKG_A" <<'PY' || heavy_fail "dotnet package search did not find the package pushed one command earlier"
import json, sys
doc = json.load(open(sys.argv[1]))
wanted = sys.argv[2].lower()
found = any(
    p.get("id", "").lower() == wanted
    for source in doc.get("searchResult", [])
    for p in source.get("packages", [])
)
sys.exit(0 if found else 1)
PY
heavy_log "NUGET-SEARCH-OK (the client queried, and found what this registry holds)"

# ── 3. Pagination — the second page must not be the first ────────────────────
#
# Both probes share a prefix, so a `take=1` search matches two packages and the
# offset is the only thing that can distinguish the pages. With `skip` ignored
# these two calls return the same id, which is the §12.4 defect exactly.

page() {  # skip -> echoes the single id on that page
  (cd "$HEAVY_WORK" && "${DOTNET[@]}" package search "HeavyProbe" --source "$INDEX" \
    --format json --take 1 --skip "$1" 2>/dev/null) \
    | python3 -c '
import json, sys
doc = json.load(sys.stdin)
ids = [p.get("id", "") for s in doc.get("searchResult", []) for p in s.get("packages", [])]
print(ids[0] if ids else "")'
}

heavy_mark "pagination"
PAGE0="$(page 0)"
PAGE1="$(page 1)"
heavy_log "page(skip=0) = '$PAGE0', page(skip=1) = '$PAGE1'"
[[ -n "$PAGE0" && -n "$PAGE1" ]] \
  || heavy_fail "a take=1 search returned nothing on one of the two pages"
[[ "$PAGE0" != "$PAGE1" ]] \
  || heavy_fail "both pages returned '$PAGE0' — skip is being ignored, so every page is the first one (RFC 0009 §12.4)"
heavy_log "NUGET-PAGINATION-OK (skip advances the result window)"

# ── 4. Restore it back into a project ────────────────────────────────────────

heavy_mark "restore"
CONSUMER="$HEAVY_WORK/consumer"
mkdir -p "$CONSUMER"
cat > "$CONSUMER/consumer.csproj" <<EOF
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>$TFM</TargetFramework>
  </PropertyGroup>
</Project>
EOF

heavy_log "dotnet add package $PKG_A"
(cd "$CONSUMER" && "${DOTNET[@]}" add package "$PKG_A" --version 1.0.0 --source "$INDEX") \
  >"$HEAVY_WORK/add.log" 2>&1 \
  || { tail -30 "$HEAVY_WORK/add.log" >&2; heavy_fail "dotnet add package failed"; }
(cd "$CONSUMER" && "${DOTNET[@]}" restore) >"$HEAVY_WORK/restore.log" 2>&1 \
  || { tail -30 "$HEAVY_WORK/restore.log" >&2; heavy_fail "dotnet restore failed"; }

# The nupkg has to have come from here: NUGET_PACKAGES is inside the run, so
# there is no global cache to have satisfied it.
find "$NUGET_PACKAGES" -iname "$PKG_A*" -maxdepth 1 | grep -q . \
  || heavy_fail "$PKG_A is not in the run's package cache — restore resolved it from somewhere else"
heavy_wire_after "restore" "GET /proxy/$REGISTRY/nuget/v3/flat/" \
  "the flat container was not read — restore did not resolve through this registry"
heavy_log "NUGET-RESTORE-OK ($PKG_A restored from this instance)"

heavy_done NUGET-HEAVY-OK
