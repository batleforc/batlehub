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
    # ── Added with the §14 extraction (the 384 strings the gate had been
    # missing). These are the rows where a rule had to be chosen rather than
    # applied; everything else in `common.*` and `adminNav.*` is mechanical.
    "myProfile.tokenAnonymous": "**Merged with a `/`.** The two auth states — a custom/static token, which carries no identity-provider group propagation, and no token at all — read as one alternative in French exactly as in English, so the slash is kept rather than expanded into a sentence. The only row where a slash carries the same meaning in both languages.",
    "common.outcome": "**Decided: `Résultat`**, the same word as `common.result`. `Issue` was tried first for being shorter, but it collides with the ticket sense and reads wrong in an audit table. Two English words legitimately map to one French word here.",
    "common.hits": "**Kept verbatim.** Cache vocabulary, used untranslated by FR infra teams, and it sits in a column beside `Misses`.",
    "common.misses": "**Kept verbatim**, same reason as `common.hits`.",
    "common.upstream": "**Kept verbatim**, consistent with `catalog.upstream` and the `upstreams` config key.",
    "common.proxied": "**Decided: `Via proxy`.** A participle would have to be `proxifié`, which is not established French; the prepositional form says the same thing without inventing a verb.",
    "common.firewall": "**Kept verbatim.** `Pare-feu` exists but `firewall` is what the surrounding infra vocabulary uses.",
    "common.release": "**Kept verbatim** — it names the release channel, not the act of releasing.",
    "common.admin": "**Kept as `Admin`.** Used identically in French, and it labels a section whose URL is `/admin`.",
    "common.webhook": "**Kept verbatim.** It is the config key and the API concept.",
    "adminNav.warming": "**Decided: `Préchauffage`.** The most debatable row on this sheet: `warming` is cache jargon and stays English in a lot of FR infra writing. Translated because it names a *page*, and because the operator reading it may not be the one who configured it. Applied consistently across `adminWarming.*`.",
    "adminAccessCheck.allow": "**Translated to `✓ AUTORISÉ`.** These are policy verdicts shown to a human, not config values — unlike the registry modes, nobody types them.",
    "adminAccessCheck.deny": "Same as `adminAccessCheck.allow`.",
    "cliDownload.yankVersion": "**`Yank` kept verbatim inside a French sentence** — it is the CLI subcommand (`version yank`). Translating the verb would leave the reader unable to find the command.",
    "adminExploreCache.upstream_unavailableTrue": "**Kept verbatim.** It is a literal API response field, reproduced so an operator can match it against a log line.",
    "pagination.pageOf": "**`sur`, not `de`** — French pagination convention is `Page 3 sur 10`. Two whole messages rather than a sentence assembled around a value, because the count is not always known.",
    "visibility.public": "The em-dash gloss structure is kept in all three visibility rows so the select reads as one list. `visibilityShort.*` carries the one-word form used in the table, rather than the long form cut at the dash — French does not put the dash where English does.",
    "a11y.breadcrumb": "**Decided: `Fil d'Ariane`**, the established French term. Screen-reader only.",
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
        f"`task ui:i18n` reports: **{untranslated_count()}**, and `task ui:i18n:check`",
        "is pinned there — a new hardcoded string fails the build.",
        "",
        "That number has read zero before while most of the console was still English, so",
        "it is worth saying what it now counts. The audit reads four places, because text",
        "hides in all four (RFC 0003 §14):",
        "",
        "| Where | Example |",
        "| --- | --- |",
        "| Text nodes | `<th>Registry</th>` |",
        "| Human-facing attributes *and component props* | `title=\"Dashboard\"`, `label=\"Registries\"` |",
        "| Literals inside template expressions | `{{ busy ? 'Loading…' : 'Refresh' }}` |",
        "| Literals in `<script>` | `{ label: \"All Packages\" }` |",
        "",
        "It had been reading only the first, and a case-insensitive identifier rule inside",
        "that one excluded every bare capitalised word — `Registries`, `You`, `Version`.",
        "384 user-visible strings sat behind a green gate; the catalogue went from 418",
        "keys to 646.",
        "",
        "Deliberately **not** counted: `:class`/`:style` literals (Tailwind lists, not",
        "prose), and `registryTypes.ts` / `registryPathFields.ts` — the setup-snippet data",
        "RFC 0003 §6.7 keeps as data, whose labels are tool names and config keys that",
        "§4.6 keeps verbatim anyway.",
        "",
    ]
    (ROOT / "docs/i18n-review-fr.md").write_text("\n".join(lines))
    print(f"docs/i18n-review-fr.md — {len(rows)} strings")
    return 0


if __name__ == "__main__":
    sys.exit(main())
