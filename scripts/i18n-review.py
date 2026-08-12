#!/usr/bin/env python3
"""Regenerate docs/i18n-review-fr.md from the message catalogues.

The French catalogue needs human review (RFC 0003 open question 3 rules out
machine translation for domain terms), and reviewing raw JSON side by side is
how that review does not happen. This produces a table with the judgement calls
flagged, so a reviewer can spend their attention on the rows that need it.
"""
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
EN = json.loads((ROOT / "ui/src/locales/en.json").read_text())
FR = json.loads((ROOT / "ui/src/locales/fr.json").read_text())

# Judgement calls worth a reviewer's attention. Keys absent here are mechanical.
NOTES = {
    "nav.packages": "**Decided: `Paquets`.** Standard FR dev usage; reads as French rather than as a half-translated interface, despite the URL staying `/packages`.",
    "account.tokens": "**Kept verbatim.** `Token` is what the API, the CLI flag (`--token`) and the docs call it. `Jeton` would be correct French and unsearchable.",
    "account.namespace": "**Kept verbatim.** Matches the config key and the admin surface (`Team Namespaces`).",
    "account.cli": "**Kept verbatim.** It is the binary's name.",
    "home.myNamespace": "**Kept verbatim** for the same reason as `account.namespace`.",
    "catalog.upstream": "**Decided: kept as `upstream`.** Widely used untranslated by FR infra teams, and it matches the config key `upstreams` that operators edit. Applied consistently in `catalog.emptyFilteredBody` too.",
    "tools.accessCheck": "Translated, because it names a *page*, not an API concept.",
    "tools.urlMapper": "**Decided: `Mappage d'URL`.** Closer to the English and 7 characters shorter, which matters in a tab strip beside `Vérification d'accès`.",
    "home.freshBodyAdmin": "**`[[registries]]` and `config.toml` kept verbatim** — they are literal things to type. Enforced by a test.",
    "dashboard.freshBody": "Same verbatim rule as above.",
    "config.readOnlyBody": "**`ConfigMap`, `Helm` kept verbatim** — product names. Enforced by a test.",
    "destructive.cannotUndo": "**Reviewed and kept.** Safety-critical: this sentence stands between an operator and a permanent purge, and `irréversible` was judged to carry the same weight as the English.",
    "destructive.typeToConfirm": "`{name}` is injected verbatim — the placeholder must survive, and a test enforces that.",
    "common.copied": "French convention puts a space before `!` — `Copié !`. Intentional, not a typo.",
    "setup.noToolMatch": "Uses French quotation marks « » rather than “ ”. Intentional.",
    "a11y.skipToContent": "Screen-reader only. Never seen visually, but read aloud.",
    "locale.en": "Language names are conventionally written in their own language, so both stay as-is in both catalogues.",
    "locale.fr": "Same.",
}

VERBATIM_HINT = "`config.toml`, `[[registries]]`, `ConfigMap`, `Helm`, `TOML`, `CLI`"


def flatten(tree, prefix=""):
    out = {}
    for key, value in tree.items():
        path = f"{prefix}.{key}" if prefix else key
        if isinstance(value, dict):
            out.update(flatten(value, path))
        else:
            out[path] = value
    return out


def untranslated_count() -> str:
    """Ask the audit script, so this document cannot drift from the gate."""
    try:
        result = subprocess.run(
            ["node", "build/i18n-audit.mjs"],
            cwd=ROOT / "ui", capture_output=True, text=True, timeout=120,
        )
        return result.stdout.strip().splitlines()[-1]
    except Exception:
        return "unknown (run `task ui:i18n` to measure)"


def main() -> int:
    en, fr = flatten(EN), flatten(FR)
    rows = []
    for key in sorted(en):
        source, target = en[key], fr.get(key, "—")
        note = NOTES.get(key, "")
        if len(source) >= 20 and len(target) > len(source) * 1.25:
            grew = round((len(target) / len(source) - 1) * 100)
            note = f"{note} " if note else ""
            note += f"**{grew}% longer than English** — check it does not overflow."
        rows.append((key, source, target, note))

    escape = lambda text: text.replace("|", "\\|").replace("\n", " ")
    lines = [
        "# French catalogue — review sheet",
        "",
        "Generated from `ui/src/locales/{en,fr}.json`. Regenerate with `task ui:i18n:review`.",
        "",
        "## How to review this",
        "",
        "You do not need to check every row. The ones that matter, in order:",
        "",
        "1. **Rows with a note.** Judgement calls I made, or strings that grew noticeably",
        "   longer in French and may overflow a layout sized for English.",
        "2. **`destructive.*`.** These are the words between an operator and a permanent",
        "   deletion. If any reads as softer in French than in English, that is a safety",
        "   bug, not a style preference.",
        "3. **Anything that names a thing you type.** " + VERBATIM_HINT + " must appear",
        "   verbatim in both columns — a test enforces this, but the test only knows the",
        "   terms it was told about.",
        "",
        "Mark a row by replacing the French with yours; nothing else needs changing.",
        "",
        "## The rule applied",
        "",
        "Translate the sentence, never the domain term. A French UI that renames `yank`,",
        "`latest`, a registry mode, or a config key leaves the reader unable to search for",
        "it, type it, or match it against the docs — which is worse than English.",
        "",
        f"## Strings ({len(rows)})",
        "",
        "| Key | English | French | Note |",
        "| --- | --- | --- | --- |",
    ]
    lines += [
        f"| `{key}` | {escape(source)} | {escape(target)} | {note} |"
        for key, source, target, note in rows
    ]
    lines += [
        "",
        "## Not yet translated",
        "",
        f"`task ui:i18n` reports: **{untranslated_count()}** — the surfaces not yet",
        "rebuilt. That number is a gate (`task ui:i18n:check`): it may fall, never rise.",
        "Phase 8 closes when it reaches 0.",
        "",
    ]
    (ROOT / "docs/i18n-review-fr.md").write_text("\n".join(lines))
    print(f"docs/i18n-review-fr.md — {len(rows)} strings")
    return 0


if __name__ == "__main__":
    sys.exit(main())
