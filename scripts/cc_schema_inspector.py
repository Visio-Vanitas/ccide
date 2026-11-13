#!/usr/bin/env python3
"""
cc_schema_inspector.py — Anchor Snippet Diagnostics & Protocol Field Extractor for Claude Code Bundles.

Used by `scripts/cc-maint.py` to:
1. Extract `+/-220` byte minified JS context around invariant string anchors whenever any patch
   misses on `darwin-arm64`, `linux-x64`, or `win32-x64`, enabling 10-second regex updates.
2. Scan JS bundles (`/$bunfs/` or `B:/~BUN/`) for `statusLine`, `message.usage`, `subagents/*.meta.json`,
   and Hook event (`MK` array) keys and diff them against `assets/cc-schema.json`.
"""
import re

from bun_pe import BunLayout, iter_js_regions  # type: ignore[import-not-found]


def collect_js_corpus(buf: bytearray, layout: BunLayout | None) -> bytes:
    chunks = [bytes(reg) for _, reg in iter_js_regions(buf, layout)]
    return b"\n".join(chunks)


def extract_anchor_snippet(corpus: bytes, anchor: str, radius: int = 220) -> str:
    needle = anchor.encode("utf-8")
    pos = corpus.find(needle)
    if pos == -1:
        return f"<anchor '{anchor}' not found in JS modules>"
    start = max(0, pos - radius)
    end = min(len(corpus), pos + len(needle) + radius)
    raw = corpus[start:end].decode("utf-8", errors="replace")
    return raw.replace("\n", "\\n")


def diagnose_missed_patches(
    patch_report: dict, corpus: bytes, patch_anchors: dict[str, str]
) -> list[dict]:
    diagnostics: list[dict] = []
    for p in patch_report.get("patches", []):
        if not p.get("applied"):
            pname = p["name"]
            anc = patch_anchors.get(pname, "")
            diagnostics.append(
                {
                    "patch": pname,
                    "required": bool(p.get("required")),
                    "message": p.get("message", ""),
                    "anchor": anc,
                    "context_snippet": extract_anchor_snippet(corpus, anc) if anc else "",
                }
            )
    return diagnostics


def extract_hook_events(corpus: bytes) -> list[str]:
    m = re.search(
        rb'\[(?:"PreToolUse"|\'PreToolUse\')(?:,[A-Za-z0-9_"\']+){15,50}\]',
        corpus,
    )
    if not m:
        return []
    return sorted(set(re.findall(r'"([A-Z][A-Za-z0-9]+)"', m.group(0).decode("ascii", "ignore"))))


def extract_keys_near_marker(
    corpus: bytes, marker: bytes, window_before: int, window_after: int, snake_only: bool = False
) -> list[str]:
    pos = corpus.find(marker)
    if pos == -1:
        return []
    span = corpus[max(0, pos - window_before) : min(len(corpus), pos + window_after)]
    txt = span.decode("utf-8", errors="ignore")
    pat = r'(?<=[{,])\s*"?([a-z][a-z0-9_]{2,34})"?\s*:' if snake_only else r'(?<=[{,])\s*"?([a-zA-Z][a-zA-Z0-9_]{2,34})"?\s*:'
    keys = set(re.findall(pat, txt))
    return sorted(keys)


def extract_live_schema(corpus: bytes, baseline: dict) -> dict:
    hook_events = extract_hook_events(corpus)

    sl_near = set(extract_keys_near_marker(corpus, b"exceeds_200k_tokens:", 900, 900, snake_only=True))
    for k in baseline.get("statusline_keys", []):
        if re.search(rb'\b' + re.escape(k.encode("utf-8")) + rb'\s*:', corpus):
            sl_near.add(k)

    usage_near = set(
        extract_keys_near_marker(corpus, b"cache_creation_input_tokens:", 320, 480, snake_only=True)
    )
    for k in baseline.get("usage_keys", []):
        if k.encode("utf-8") in corpus:
            usage_near.add(k)

    meta_near = set(extract_keys_near_marker(corpus, b"stoppedByUser:", 450, 550, snake_only=False))
    for k in baseline.get("subagent_meta_keys", []):
        if k.encode("utf-8") in corpus:
            meta_near.add(k)

    noise = {
        "return",
        "true",
        "false",
        "null",
        "void",
        "let",
        "const",
        "var",
        "type",
        "case",
        "default",
        "level",
        "kind",
        "name",
        "id",
        "url",
        "path",
        "status",
        "value",
        "error",
        "data",
    }

    def diff_set(found: set[str], base_list: list[str], filter_noise: bool = True) -> dict:
        base_set = set(base_list)
        clean_found = {x for x in found if (not filter_noise or x not in noise or x in base_set)}
        return {
            "added": sorted(clean_found - base_set),
            "removed": sorted(base_set - clean_found),
            "matched_count": len(clean_found & base_set),
        }

    return {
        "hook_events": diff_set(set(hook_events), baseline.get("hook_events", []), False)
        if hook_events
        else {"added": [], "removed": [], "matched_count": 0},
        "statusline_keys": diff_set(sl_near, baseline.get("statusline_keys", [])),
        "usage_keys": diff_set(usage_near, baseline.get("usage_keys", [])),
        "subagent_meta_keys": diff_set(meta_near, baseline.get("subagent_meta_keys", [])),
    }
