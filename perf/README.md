# BatleHub — Performance Testing Guide

This directory contains everything needed to measure throughput, latency, and resource usage of the BatleHub API under load.

## Table of contents

1. [Prerequisites](#prerequisites)
2. [Architecture of the test environment](#architecture-of-the-test-environment)
3. [Quick start — filesystem + memory (default)](#quick-start--filesystem--memory-default)
4. [Quick start — S3 + Redis](#quick-start--s3--redis)
5. [Comparing backends head-to-head](#comparing-backends-head-to-head)
6. [Scenarios](#scenarios)
7. [Tuning the mock upstream](#tuning-the-mock-upstream)
8. [Reading the results](#reading-the-results)
9. [Known bottlenecks and what to watch](#known-bottlenecks-and-what-to-watch)
10. [Running against a remote server](#running-against-a-remote-server)

---

## Prerequisites

| Tool | Install |
|------|---------|
| k6 | `mise install k6` · [k6.io/docs/get-started/installation](https://k6.io/docs/get-started/installation/) |
| Podman + podman-compose | already used by existing `task compose:*` tasks |
| Rust toolchain | already present (used to build the mock upstream) |
| PostgreSQL | started via `task compose:db` |

---

## Architecture of the test environment

```
┌─────────────┐   HTTP   ┌────────────────────────┐   HTTP   ┌──────────────────────┐
│    k6       │ ───────► │  BatleHub server        │ ───────► │  mock-upstream       │
│  load gen   │          │  :8080                  │  :9999   │  (npm / cargo mock)  │
└─────────────┘          │  perf/config.perf.toml  │          └──────────────────────┘
                         └──────────┬──────────────┘
                                    │ SQL
                         ┌──────────▼──────────────┐
                         │  PostgreSQL :5432        │
                         └─────────────────────────┘

                    ┌──────────────────────────────────────┐
                    │  Prometheus :9090  ◄── scrapes /metrics every 5 s
                    │  Grafana    :3000  ◄── reads Prometheus
                    └──────────────────────────────────────┘
```

**Registries defined in `perf/config.perf.toml`:**

| Name | Mode | Purpose |
|------|------|---------|
| `perf-npm` | proxy → mock upstream | scenarios 02 (warm read) and 03 (cache miss) |
| `perf-local-npm` | local (no upstream) | scenarios 04 (upload) and 05 (mixed) |

---

## Quick start — filesystem + memory (default)

Run these commands in separate terminals:

```bash
# Terminal 1 — database
task compose:db

# Terminal 2 — mock upstream registry (npm + cargo responses)
task perf:upstream

# Terminal 3 — BatleHub server using the perf config
task perf:server

# Terminal 4 — Prometheus + Grafana
task perf:infra:up
# Open http://localhost:3000  (admin / admin)
# → BatleHub folder → "BatleHub Performance" dashboard

# Terminal 5 — warm cache + verify connectivity
task perf:seed

# Run scenarios (terminal 5, sequentially)
task perf:run:rest     # 60 s baseline
task perf:run:read     # warm-cache ramp test
task perf:run:miss     # cache-miss / proxy-through
task perf:run:upload   # publish / upload
task perf:run:mixed    # 10-minute realistic mix
```

To run the full suite in one shot (all scenarios run even when thresholds are crossed):

```bash
task perf:run:all
```

> **Exit code 99** — when you run a scenario directly (e.g. `task perf:run:read`), k6 exits with code 99 if any threshold is violated. This is intentional: it lets you use individual scenarios as CI latency gates. The `perf:run:all` task passes `--no-thresholds` to k6 so every scenario always runs to completion; threshold results are still printed in the summary, but a violation does not abort the suite.

---

## Quick start — S3 + Redis

Uses MinIO as the S3-compatible object store and Redis as the shared metadata cache. The k6 scenarios and the mock upstream are identical — only the server config changes.

```bash
# Terminal 1 — database (same as before; skip if already running)
task compose:db

# Terminal 2 — mock upstream (same as before; skip if already running)
task perf:upstream

# Terminal 3 — start MinIO (:9200) and Redis (:6380)
task perf:s3:infra:up
# MinIO console: http://localhost:9201  (minioadmin / minioadmin)

# Terminal 4 — BatleHub server with S3 + Redis config
task perf:s3:server

# Terminal 5 — warm cache and verify
task perf:seed

# Run all scenarios
task perf:s3:run:all
```

Individual scenarios follow the same naming convention as the default suite:

```bash
task perf:s3:run:rest
task perf:s3:run:read
task perf:s3:run:miss
task perf:s3:run:upload
task perf:s3:run:mixed
```

The MinIO bucket (`perf-artifacts`) is created automatically by the `perf-minio-init` container on first `perf:s3:infra:up`.

---

## Comparing backends head-to-head

Run both suites back-to-back without changing the k6 scripts or mock upstream. The server is the only variable — same DB, same registries, same load profile.

```bash
# 1. Run filesystem + memory suite
task perf:server        # terminal A
task perf:run:all       # terminal B — save terminal output to fs-results.txt

# 2. Stop the FS server, start the S3+Redis server
# (Ctrl-C in terminal A, then:)
task perf:s3:server     # terminal A
task perf:s3:run:all    # terminal B — save terminal output to s3-results.txt
```

**What to compare:**

| Metric | filesystem + memory | S3 + Redis | Expected winner |
|--------|--------------------|--------------------|-----------------|
| Warm-read P95 latency | — | — | filesystem (local disk < network S3) |
| Cache-miss P95 latency | — | — | similar (both bottlenecked by upstream) |
| Upload P95 latency | — | — | S3 (async multipart vs synchronous fsync) |
| RAM at peak load | — | — | S3+Redis (no in-process metadata map) |
| CPU at peak load | — | — | S3+Redis higher (TLS + ser/deser overhead) |

Fill in the blanks with your measured values. The Grafana dashboard (started with `task perf:infra:up`) stays up across both runs, so you can overlay the two time series.

---

## Scenarios

### 01 — At-rest baseline (`perf:run:rest`)

**Goal:** capture idle resource usage before any load.  
**Profile:** 1 VU, 60 s.  
**Endpoints hit:** `/healthz`, `/metrics`, `/api/v1/me`.

Check Grafana while this runs to record the resting RSS and CPU. This is your baseline for interpreting numbers in later scenarios.

---

### 02 — Warm cache reads (`perf:run:read`)

**Goal:** measure maximum throughput for already-cached artifacts.  
**Profile:** ramp 10 → 50 → 100 → 200 VU over ~4 min.

Every VU hits the same pre-warmed URL:

```
GET /proxy/perf-npm/perf-pkg/1.0.0/tarball
```

Because the artifact is in the filesystem cache after the first request, the server never contacts the mock upstream. This isolates the path: **auth middleware → rate-limit check → DB TTL query → filesystem read → stream to client**.

**Expected thresholds:** P95 < 200 ms, error rate < 1%.

**What degrades first:** the DB connection pool (default 10, raised to 50 in `config.perf.toml`). Watch for `pool_waiting` appearing in traces and latency climbing steeply around 100+ VU.

---

### 03 — Cache miss / proxy-through (`perf:run:miss`)

**Goal:** measure the full proxy pipeline including upstream fetch and cache write.  
**Profile:** 20 VU, 120 s. Each VU uses a unique version string (`0.<VU>.<ITER>`) so every request is a cache miss.

The path per request: **auth → DB check → upstream HTTP GET packument → upstream HTTP GET tarball → filesystem write → DB write → stream to client**.

**Expected thresholds:** P95 < 3 s (tunable by adjusting `--delay-ms` on mock upstream).

**What to tune:** restart `task perf:upstream` with `DELAY_MS=200` to simulate a slow upstream and see how latency distributes:

```bash
DELAY_MS=200 task perf:upstream
```

---

### 04 — Artifact upload (`perf:run:upload`)

**Goal:** measure publish throughput and memory pressure from buffering.  
**Profile:** 10 concurrent VUs, 60 s. Each publish is a uniquely-named version.

The upload path buffers the entire payload in memory before writing to disk. Default test artifact is 64 KB. Use `ARTIFACT_KB` to test larger sizes:

```bash
ARTIFACT_KB=1024 task perf:run:upload     # 1 MiB payloads
ARTIFACT_KB=51200 task perf:run:upload    # 50 MiB payloads — watch RSS carefully
```

**What to watch:** server RSS in Grafana. With 10 concurrent 50 MiB uploads, peak in-memory usage reaches ~500 MiB. This reveals the buffering bottleneck documented in §7.

---

### 05 — Realistic mixed workload (`perf:run:mixed`)

**Goal:** simulate a 10-minute production traffic mix to reveal how bottlenecks interact.  
**Profile (three named k6 scenarios running simultaneously):**

| Scenario | VUs | Type |
|----------|-----|------|
| `warm_read` | ramp 0→80 | cached GET |
| `cache_miss` | 10 constant | proxy-through |
| `upload` | 3 constant | PUT publish |

**Thresholds:** P95 < 200 ms for warm reads, P95 < 3 s for cache misses, error rate < 2%.

This is the most realistic run. The mixed write pressure on the DB (access_events inserts, quota updates, touch_artifact) combined with read load shows how much headroom the DB pool has.

---

## Tuning the mock upstream

`task perf:upstream` accepts two variables:

| Variable | Default | Effect |
|----------|---------|--------|
| `DELAY_MS` | `0` | Simulated upstream response time (ms) |
| `ARTIFACT_KB` | `512` | Size of served artifact bodies (KB) |

Examples:

```bash
# Simulate a 100 ms upstream (CDN-like latency)
DELAY_MS=100 task perf:upstream

# Simulate a slow upstream + large artifacts
DELAY_MS=500 ARTIFACT_KB=4096 task perf:upstream
```

---

## Reading the results

### k6 terminal output

After each scenario, k6 prints a summary:

```
✓ status 200
✓ body non-empty

checks.........................: 100.00% ✓ 48312  ✗ 0
data_received..................: 24 GB   40 MB/s
http_req_duration...............: avg=12ms   min=1ms   med=8ms    max=892ms  p(90)=28ms   p(95)=45ms
http_req_failed.................: 0.00%  ✓ 0      ✗ 24156
iterations.....................: 24156   402/s
```

Key columns: `p(95)` latency, `iterations/s` (≈ req/s for single-request VUs), `http_req_failed` rate.

### Grafana dashboard

Open **http://localhost:3000** → BatleHub folder → **BatleHub Performance**.

Panels:

| Panel | What to look for |
|-------|-----------------|
| **Request Rate** | req/s by registry and outcome — should track k6 iterations/s |
| **Latency P50/P95/P99** | Where P95 climbs steeply = bottleneck point |
| **Cache Hit Rate** | Should be ~100% during scenario 02; ~0% during 03 |
| **Upstream Errors** | Non-zero = mock upstream overloaded or mis-configured |
| **Artifact Cache Hits vs Misses** | Cross-check with k6 scenario |
| **Latency Heatmap** | Bimodal distribution = two code paths competing |

### System resource monitoring

While tests run, watch server resources in a separate terminal:

```bash
# CPU and memory of the batlehub process
watch -n1 "ps -o pid,pcpu,pmem,rss,vsz,comm -p \$(pgrep batlehub)"

# Or with pidstat (more detail)
pidstat -u -r -p \$(pgrep batlehub) 1
```

---

## Known bottlenecks and what to watch

These are the code paths identified as likely degradation points, in priority order:

### 1. DB connection pool

**Config:** `max_connections = 50` in `perf/config.perf.toml` (raised from the default 10).  
**Trigger:** scenario 02 at 100+ VU.  
**Signal:** P95 latency climbs non-linearly; sqlx pool queue grows.  
**Location:** `crates/adapters/src/cache/postgres.rs` — every cache hit writes `access_events` and potentially `touch_artifact`.

To observe the default-10 behaviour, edit `config.perf.toml` and set `max_connections = 10`, then rerun scenario 02.

---

### 2. Artifact buffering (upload memory pressure)

**Trigger:** scenario 04 with large `ARTIFACT_KB`.  
**Signal:** server RSS grows proportionally to VU × artifact size.  
**Location:** `crates/core/src/services/local_registry.rs` — the entire request body is collected into a `Bytes` before the storage write begins.

Run scenario 04 with 50 MiB payloads and watch RSS in `ps` or Grafana (node-exporter if added).

---

### 3. Filesystem `exists()` blocking call

**Trigger:** scenario 02 at high VU.  
**Signal:** tokio thread pool CPU spikes; latency tail widens.  
**Location:** `crates/adapters/src/storage/filesystem.rs` — `path.exists()` is a synchronous syscall not wrapped in `spawn_blocking`.

This manifests as higher P99 without a corresponding P95 increase.

---

### 4. `touch_artifact` DB write on every cache hit

**Trigger:** scenario 02 at high sustained RPS.  
**Signal:** DB write rate equals request rate even with 100% cache hits.  
**Location:** `crates/core/src/services/proxy.rs` — `touch_artifact()` is spawned async on every served hit.

Disable artifact TTL in `config.perf.toml` to measure the difference:

```toml
# comment out artifact_ttl_secs under [registries.cache] to skip the touch path
```

---

### 5. Rate-limit middleware lock

**Trigger:** scenario 05 (mixed) at sustained 1k+ req/s.  
**Signal:** CPU increases without proportional throughput gain; latency tail spikes.  
**Location:** `crates/adapters/src/rate_limit/in_memory.rs` — single Mutex/RwLock protecting the token-bucket map.

To disable rate limiting for a clean baseline, remove the `[registries.rate_limit]` blocks from `perf/config.perf.toml`.

---

## Running against a remote server

All k6 scenarios read `BATLEHUB_URL` and `BATLEHUB_TOKEN` from the environment:

```bash
export BATLEHUB_URL=https://batlehub.example.com
export BATLEHUB_TOKEN=your-token-here

task perf:run:read
```

The seed script also accepts a URL argument:

```bash
bash perf/scripts/seed.sh https://batlehub.example.com
```

When testing a remote server, skip `task perf:upstream` (the real upstream is used) and skip `task perf:infra:up` (point Prometheus at the remote `/metrics` endpoint instead by editing `perf/prometheus.yml`).
