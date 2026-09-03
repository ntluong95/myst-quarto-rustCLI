#!/usr/bin/env python3
"""Regenerate the sentinel-bounded Markdown tables in
docs/dialect-comparison.md from the machine-readable contract in
mappings.toml.

mappings.toml is the source of truth (Phase 2 of
plans/260903-1749-rust-port-dialect-fidelity/). This script renders six
tables — §2 non-structural block constructs, §5 inline constructs, §8.2
config field mapping, §8.3 export/format mapping, §10 legacy read-only
surface, and §3.3 label-prefix rules — and splices each one, byte for byte,
between its `<!-- generated: do not edit -->` / `<!-- end generated -->`
sentinel pair in the doc.

crates/mystquarto-core/tests/doc-sync.rs reimplements this same rendering
logic independently in Rust and asserts it matches the committed doc, so the
reference and the data cannot silently drift apart.

Usage: uv run python scripts/render-mapping-tables.py
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MAPPINGS_PATH = REPO_ROOT / "mappings.toml"
DOC_PATH = REPO_ROOT / "docs" / "dialect-comparison.md"

FIDELITY_SYMBOL = {"exact": "✅", "lossy": "⚠️", "unmappable": "❌"}

# Sentinel markers, keyed by the id used in docs/dialect-comparison.md, e.g.:
#   <!-- generated: do not edit (directive) -->
#   ...table...
#   <!-- end generated (directive) -->
SENTINELS = [
    "directive",
    "inline",
    "config_field",
    "export_format",
    "legacy_role",
    "label_prefix",
]


def esc(value: str | None) -> str:
    """Escape a cell value for Markdown table syntax, or render '—' for None."""
    if value is None:
        return "—"
    return value.replace("|", "\\|")


def note_cell(value: str | None) -> str:
    """Notes render as an empty cell (not '—') when absent."""
    if value is None:
        return ""
    return value.replace("|", "\\|")


def fidelity_cell(value: str) -> str:
    return FIDELITY_SYMBOL[value]


def render_table(header: list[str], rows: list[list[str]]) -> str:
    lines = ["| " + " | ".join(header) + " |"]
    lines.append("|" + "|".join(["---"] * len(header)) + "|")
    for row in rows:
        lines.append("| " + " | ".join(row) + " |")
    return "\n".join(lines)


def render_directive(mappings: dict) -> str:
    rows = [
        [
            esc(d["myst"]),
            esc(d.get("quarto")),
            fidelity_cell(d["fidelity"]),
            note_cell(d.get("note")),
        ]
        for d in mappings.get("directive", [])
    ]
    return render_table(["MyST", "Quarto", "Fidelity", "Note"], rows)


def render_inline(mappings: dict) -> str:
    rows = []
    for r in mappings.get("role", []):
        rows.append(
            [
                esc(r.get("myst")),
                esc(r.get("quarto")),
                fidelity_cell(r["fidelity"]),
                note_cell(r.get("note")),
            ]
        )
    for i in mappings.get("inline", []):
        rows.append(
            [
                esc(i["myst"]),
                esc(i["quarto"]),
                fidelity_cell(i["fidelity"]),
                note_cell(i.get("note")),
            ]
        )
    return render_table(["MyST", "Quarto", "Fidelity", "Note"], rows)


def render_config_field(mappings: dict) -> str:
    rows = [
        [
            esc(c.get("myst")),
            esc(c.get("quarto")),
            fidelity_cell(c["fidelity"]),
            note_cell(c.get("note")),
        ]
        for c in mappings.get("config_field", [])
    ]
    return render_table(
        ["`myst.yml` field", "`_quarto.yml` field", "Fidelity", "Note"], rows
    )


def render_export_format(mappings: dict) -> str:
    rows = [
        [
            esc(e["myst"]),
            esc(e.get("quarto")),
            fidelity_cell(e["fidelity"]),
            note_cell(e.get("note")),
        ]
        for e in mappings.get("export_format", [])
    ]
    return render_table(["MyST export", "Quarto format", "Fidelity", "Note"], rows)


def render_legacy_role(mappings: dict) -> str:
    rows = [
        [
            esc(le["myst"]),
            esc(le["modern_myst"]),
            esc(le["quarto"]),
            fidelity_cell(le["fidelity"]),
            note_cell(le.get("note")),
        ]
        for le in mappings.get("legacy_role", [])
    ]
    return render_table(
        ["Legacy construct", "Modern MyST", "Quarto", "Fidelity", "Note"], rows
    )


def render_label_prefix(mappings: dict) -> str:
    rows = [
        [
            esc(lp["myst"]),
            esc(lp["quarto"]),
            fidelity_cell(lp["fidelity"]),
            note_cell(lp.get("note")),
        ]
        for lp in mappings.get("label_prefix", [])
    ]
    return render_table(["MyST prefix", "Quarto prefix", "Fidelity", "Note"], rows)


RENDERERS = {
    "directive": render_directive,
    "inline": render_inline,
    "config_field": render_config_field,
    "export_format": render_export_format,
    "legacy_role": render_legacy_role,
    "label_prefix": render_label_prefix,
}


def splice(doc_text: str, section_id: str, table_markdown: str) -> str:
    start = f"<!-- generated: do not edit ({section_id}) -->"
    end = f"<!-- end generated ({section_id}) -->"
    pattern = re.compile(re.escape(start) + r"\n.*?\n" + re.escape(end), re.DOTALL)
    if not pattern.search(doc_text):
        raise SystemExit(
            f"error: sentinel pair for {section_id!r} not found in {DOC_PATH}"
        )
    replacement = f"{start}\n{table_markdown}\n{end}"
    # A callable repl (rather than a string) makes re.sub use the return
    # value verbatim — no backslash/group escaping applies, which matters
    # here since our cell content legitimately contains literal backslashes
    # (e.g. escaped pipes, "\\ at end of line").
    return pattern.sub(lambda _: replacement, doc_text, count=1)


def main() -> int:
    with MAPPINGS_PATH.open("rb") as f:
        mappings = tomllib.load(f)

    doc_text = DOC_PATH.read_text()
    for section_id in SENTINELS:
        table_markdown = RENDERERS[section_id](mappings)
        doc_text = splice(doc_text, section_id, table_markdown)

    DOC_PATH.write_text(doc_text)
    print(f"Regenerated {len(SENTINELS)} table(s) in {DOC_PATH.relative_to(REPO_ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
