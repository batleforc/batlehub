#!/usr/bin/env bash
# Wraps `k6 run` and appends a resource summary (min/max/median RAM and CPU)
# for the batlehub server process.
#
# Usage: bash perf/scripts/run_with_metrics.sh [k6-flags...] <scenario.js>
#   e.g. bash perf/scripts/run_with_metrics.sh --no-thresholds perf/k6/scenarios/02_warm_read.js
#
# RSS is read from /proc/{PID}/status (accurate, Linux-only).
# CPU% is computed from /proc/{PID}/stat deltas so it is instantaneous
# (same method as `top`), not the lifetime average that `ps pcpu` reports.
set -euo pipefail

# ── Locate the server process ─────────────────────────────────────────────────
# Accept an explicit PID via env var to bypass auto-detection.
if [[ -n "${BATLEHUB_PID:-}" ]]; then
  PID="$BATLEHUB_PID"
else
  # 1. Exact binary name match (works for both `cargo run` and direct invocation).
  # 2. Fall back to full-path match in case multiple binaries share the name.
  # Using two separate pgrep calls avoids ERE alternation portability issues.
  PID=$(pgrep -x "batlehub" 2>/dev/null | head -1 || \
        pgrep -f "target/release/batlehub" 2>/dev/null | head -1 || true)
fi

if [[ -z "$PID" ]]; then
  echo "  [resource-monitor] batlehub process not found — run 'task perf:server' first"
  echo "  [resource-monitor] tip: set BATLEHUB_PID=<pid> to pin the process manually"
  echo "  [resource-monitor] skipping resource metrics, running k6 only"
  exec k6 run "$@"
fi

CLK_TCK=$(getconf CLK_TCK 2>/dev/null || echo 100)
RSS_FILE=$(mktemp /tmp/batlehub-rss.XXXXXX)
CPU_FILE=$(mktemp /tmp/batlehub-cpu.XXXXXX)
trap 'rm -f "$RSS_FILE" "$CPU_FILE"' EXIT

echo "  [resource-monitor] tracking PID $PID  CLK_TCK=$CLK_TCK"

# ── Background sampler ────────────────────────────────────────────────────────
(
  prev_ticks=0
  prev_ms=0

  while kill -0 "$PID" 2>/dev/null; do
    # RSS in kB (from /proc — always instantaneous)
    rss=$(awk '/VmRSS/{print $2; exit}' "/proc/$PID/status" 2>/dev/null || echo "")

    # Instantaneous CPU% from tick delta between samples
    curr_ticks=$(awk '{print $14+$15}' "/proc/$PID/stat" 2>/dev/null || echo 0)
    curr_ms=$(date +%s%3N)

    if [[ "$prev_ms" -gt 0 ]] && [[ -n "$rss" ]]; then
      tick_delta=$(( curr_ticks - prev_ticks ))
      ms_delta=$(( curr_ms - prev_ms ))
      cpu=$(awk -v td="$tick_delta" -v ms="$ms_delta" -v clk="$CLK_TCK" \
        'BEGIN { if(ms>0) printf "%.2f", (td/clk)/(ms/1000)*100; else print "0" }')
      echo "$rss" >> "$RSS_FILE"
      echo "$cpu"  >> "$CPU_FILE"
    fi

    prev_ticks=$curr_ticks
    prev_ms=$curr_ms
    sleep 1
  done
) &
SAMPLER_PID=$!

# ── Run k6 ────────────────────────────────────────────────────────────────────
set +e
k6 run "$@"
K6_EXIT=$?
set -e

kill "$SAMPLER_PID" 2>/dev/null
wait "$SAMPLER_PID" 2>/dev/null || true

# ── Print resource summary ────────────────────────────────────────────────────
N=$(wc -l < "$RSS_FILE" 2>/dev/null | tr -d ' ')
N=${N:-0}

echo ""
echo "┌─ Resource usage — PID $PID — $N samples @ 1 s ─────────────────────────────"

awk_stats() {
  local file="$1" label="$2" divisor="$3" unit="$4"
  if [[ ! -s "$file" ]]; then
    printf "│  %-14s no data\n" "$label"
    return
  fi
  awk -v lbl="$label" -v div="$divisor" -v unit="$unit" '
    {
      v = $1 / div
      a[NR] = v
      sum += v
      if (NR == 1 || v < mn) mn = v
      if (NR == 1 || v > mx) mx = v
    }
    END {
      n = NR
      # insertion sort for median
      for (i = 2; i <= n; i++) {
        k = a[i]; j = i - 1
        while (j >= 1 && a[j] > k) { a[j+1] = a[j]; j-- }
        a[j+1] = k
      }
      med = (n % 2 == 1) ? a[int(n/2)+1] : (a[n/2] + a[n/2+1]) / 2
      printf "│  %-14s  min=%8.1f   median=%8.1f   max=%8.1f   %s\n", \
             lbl, mn, med, mx, unit
    }
  ' "$file"
}

awk_stats "$RSS_FILE" "RAM (RSS)"  1024  "MiB"
awk_stats "$CPU_FILE" "CPU"        1     "%"

echo "└────────────────────────────────────────────────────────────────────────────"

exit $K6_EXIT
