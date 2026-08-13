import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * The stub fixtures are checked against `openapi.json` (RFC 0003 §13).
 *
 * `build/stub-server.mjs` lets the rendered gates scan pages with rows in them
 * instead of empty states. That is only worth anything while the rows look like
 * what the server actually sends — a hand-written fixture that drifts keeps the
 * gate green over a UI that no longer matches production.
 *
 * `openapi.json` is already the contract and already re-synced by
 * `task dump-spec`, so validating against it means the existing spec gate is
 * what catches drift, rather than a second source of truth.
 */

const ROOT = resolve(process.cwd());
const spec = JSON.parse(readFileSync(resolve(ROOT, "openapi.json"), "utf8"));
const fixtures = JSON.parse(
  readFileSync(resolve(ROOT, "build/fixtures/captured.json"), "utf8"),
) as Record<string, unknown>;

type Node = Record<string, any>;

function deref(schema: Node): Node {
  let node = schema;
  while (node?.$ref) {
    node = node.$ref
      .replace(/^#\//, "")
      .split("/")
      .reduce((acc: Node, part: string) => acc?.[part], spec);
  }
  return node ?? {};
}

/** The spec path for a concrete URL, resolving `{param}` segments. */
function specPathFor(pathname: string): string | undefined {
  if (spec.paths[pathname]) return pathname;
  const segments = pathname.split("/");
  return Object.keys(spec.paths).find((candidate) => {
    const parts = candidate.split("/");
    return (
      parts.length === segments.length &&
      parts.every((part, i) => part === segments[i] || /^\{.+\}$/.test(part))
    );
  });
}

function responseSchema(pathname: string): Node | undefined {
  const specPath = specPathFor(pathname);
  const content = specPath && spec.paths[specPath]?.get?.responses?.["200"]?.content;
  if (!content) return undefined;
  const media = Object.values(content)[0] as Node;
  return media.schema ? deref(media.schema) : undefined;
}

const paths = Object.keys(fixtures);

describe("stub API fixtures", () => {
  it("covers every endpoint the authenticated routes call", () => {
    expect(paths.length).toBeGreaterThanOrEqual(21);
  });

  /**
   * The whole reason for stubbing rather than booting a real backend: a fresh
   * backend renders every list empty, and an empty page is what the CI gate was
   * already scanning when it reported six clean pages that never rendered.
   */
  /**
   * Nested collections too, not just top-level arrays.
   *
   * The original check only looked at `Array.isArray(fixtures[p])`, so a
   * paginated envelope like `{"items": [], "total": 0}` or `{"events": []}`
   * passed while rendering nothing — and the rendered gates were scoring the
   * empty state of `/admin/packages` (the largest page in the console) and of
   * the inbound-events view. Exactly the failure this file was written to stop,
   * one level of nesting down.
   */
  it("leaves no collection empty, at any depth", () => {
    const emptyCollections = (value: unknown, trail: string): string[] => {
      if (Array.isArray(value)) return value.length ? [] : [trail];
      if (value && typeof value === "object") {
        return Object.entries(value as Node).flatMap(([k, v]) =>
          emptyCollections(v, `${trail}.${k}`),
        );
      }
      return [];
    };
    const empty = paths.flatMap((p) => emptyCollections(fixtures[p], p));
    expect(empty, "empty fixtures put the gate back to measuring empty states").toEqual([]);
  });

  it("matches the required properties openapi.json declares", () => {
    const problems: string[] = [];
    for (const path of paths) {
      const schema = responseSchema(path);
      if (!schema) continue; // documented below as a spec gap, not a fixture fault
      const item = deref(schema.items ?? schema);
      const required: string[] = item.required ?? [];
      if (!required.length) continue;

      const rows = Array.isArray(fixtures[path]) ? (fixtures[path] as Node[]) : [fixtures[path]];
      for (const [i, row] of rows.entries()) {
        if (row === null || typeof row !== "object") continue;
        const missing = required.filter((key) => !(key in row));
        if (missing.length) problems.push(`${path}[${i}] missing ${missing.join(", ")}`);
      }
    }
    expect(problems).toEqual([]);
  });

  /**
   * Six endpoints the console called used to have no documented 200 body, which
   * is why `src/lib/registry-types.ts` hand-wrote their DTOs and the generated
   * client typed them as unknown. RFC 0004 §4.1 annotated every one of them and
   * deleted that file, so this is an equality, not a ceiling: a new untyped
   * endpoint is a regression, and the server-side gate
   * (`crates/web/tests/openapi_contract.rs`) is what stops one being written.
   */
  it("leaves no called endpoint untyped", () => {
    const undocumented = paths.filter((p) => !responseSchema(p));
    expect(undocumented, `undocumented: ${undocumented.join(", ")}`).toEqual([]);
  });
});
