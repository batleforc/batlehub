const BYTE_UNITS = ["B", "KiB", "MiB", "GiB", "TiB"] as const;

/** Formats a byte count using binary (1024) units, e.g. `1.5 MiB`. */
export function formatBytes(
  bytes: number | null | undefined,
  opts?: { fallback?: string },
): string {
  const fallback = opts?.fallback ?? "—";
  if (bytes === null || bytes === undefined || Number.isNaN(bytes)) return fallback;
  if (bytes < 1024) return `${bytes} B`;
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < BYTE_UNITS.length - 1) {
    value /= 1024;
    unitIndex++;
  }
  return `${value.toFixed(1)} ${BYTE_UNITS[unitIndex]}`;
}

/** Formats an ISO date string using the browser locale, e.g. `1/2/2026, 3:04:05 PM`. */
export function formatDate(iso: string | null | undefined, opts?: { fallback?: string }): string {
  const fallback = opts?.fallback ?? "—";
  if (!iso) return fallback;
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return fallback;
  return date.toLocaleString();
}

/** Formats an ISO date string as a short relative time, e.g. `5m ago`. */
export function formatRelative(
  iso: string | null | undefined,
  opts?: { fallback?: string },
): string {
  const fallback = opts?.fallback ?? "Never";
  if (!iso) return fallback;
  const time = new Date(iso).getTime();
  if (Number.isNaN(time)) return fallback;
  const diff = Date.now() - time;
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/** Formats a count using the browser locale's thousands separators, e.g. `1,234`. */
export function formatCount(n: number | null | undefined, opts?: { fallback?: string }): string {
  const fallback = opts?.fallback ?? "—";
  if (n === null || n === undefined || Number.isNaN(n)) return fallback;
  return n.toLocaleString();
}
