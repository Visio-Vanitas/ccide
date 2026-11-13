#!/usr/bin/env python3
"""
agent-task-context.py — Deterministic Issue/PR Context & Modularity Gate for AI Agents & CI.

Usage:
  python3 -I scripts/agent-task-context.py --issue 7
  python3 -I scripts/agent-task-context.py --check-line-budget --changed-only
  python3 -I scripts/agent-task-context.py --promotion-notes origin/main..HEAD
  python3 -I scripts/agent-task-context.py --sync-labels --repo Visio-Vanitas/ccide
"""
import argparse
import json
import os
import re
import subprocess
import sys

AREA_MAP = {
    "area:patcher": {
        "keywords": ("patch_claude_binary", "patcher", "sigusr2", "__bun", "bun!"),
        "paths": ["scripts/patch_claude_binary.py", "tests/test_patch_claude_binary.py"],
        "color": "d93f0b",
    },
    "area:sidebar": {
        "keywords": ("sidebar", "sprite", "subagents", "context", "cost", "tokscale", "cur_pid"),
        "paths": ["src/commands/sidebar/", "src/tui/context/", "src/tui/logo/", "src/session/"],
        "color": "5319e7",
    },
    "area:installer": {
        "keywords": ("install.sh", "iterm-preferences", "iterm-pyenv", "pyenv", "doctor", "setup"),
        "paths": ["install.sh", "src/commands/setup/", "scripts/ccide"],
        "color": "0e8a16",
    },
    "area:workspace": {
        "keywords": ("workspace.rs", "ccide_daemon", "iterm2_workspace", "osc 1337", "tmux"),
        "paths": ["src/commands/workspace.rs", "scripts/ccide_daemon.py", "scripts/iterm2_workspace.py"],
        "color": "1d76db",
    },
    "area:hscroll": {
        "keywords": ("pty-hscroll", "yazi-hscroll", "lazygit-pager", "lazygit-log", "md-render", "yazi"),
        "paths": ["scripts/pty-hscroll-bridge.py", "assets/yazi-hscroll.lua", "src/commands/yazi_hook.rs"],
        "color": "006b75",
    },
    "area:statusline": {
        "keywords": ("statusline", "autocompact", "auto-compact", "powerline"),
        "paths": ["src/commands/statusline/", "src/json.rs"],
        "color": "fbca04",
    },
    "area:ci": {
        "keywords": ("ci.yml", "release.yml", "cc-compat-watch", "workflow", "issue-pr-lifecycle"),
        "paths": [".github/workflows/", ".github/ISSUE_TEMPLATE/"],
        "color": "bfd4f2",
    },
}

EXTRA_LABELS = {
    "compat": "b60205",
    "agent-ready": "0e8a16",
    "has-pr": "1d76db",
    "stage:in-dev": "5319e7",
    "stage:in-preview": "fbca04",
    "triage": "ededed",
}


def slugify(text: str, max_len: int = 36) -> str:
    ascii_part = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    if not ascii_part:
        return "task"
    return ascii_part[:max_len].rstrip("-")


def extract_closing_numbers(text: str) -> list[int]:
    return sorted(
        {
            int(m.group(1))
            for m in re.finditer(
                r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)\b",
                text or "",
            )
        }
    )


def extract_sections(markdown_body: str) -> dict[str, str]:
    sections: dict[str, list[str]] = {}
    cur = "preamble"
    for line in (markdown_body or "").splitlines():
        m = re.match(r"^#{2,3}\s+(.+?)\s*$", line)
        if m:
            cur = m.group(1).strip()
            sections.setdefault(cur, [])
        else:
            sections.setdefault(cur, []).append(line)
    return {k: "\n".join(v).strip() for k, v in sections.items() if "\n".join(v).strip()}


def parse_issue_to_brief(issue_obj: dict) -> dict:
    num = int(issue_obj.get("number", 0))
    title = str(issue_obj.get("title", "")).strip()
    body = str(issue_obj.get("body", "") or "")
    combined = f"{title}\n{body}".lower()

    areas: list[str] = []
    relevant_paths: list[str] = []
    for area, meta in AREA_MAP.items():
        if any(kw in combined for kw in meta["keywords"]):
            areas.append(area)
            for p in meta["paths"]:
                if p not in relevant_paths:
                    relevant_paths.append(p)

    is_fix = (
        title.lower().startswith("fix")
        or "[compat alert]" in title.lower()
        or "bug" in [lb.get("name", "") if isinstance(lb, dict) else str(lb) for lb in issue_obj.get("labels", [])]
        or "## 现象" in body
        or "### minimal reproduction" in combined
    )
    prefix = "fix" if is_fix else "feat"
    sections = extract_sections(body)

    criteria: list[str] = []
    for sec_title, sec_body in sections.items():
        if "acceptance" in sec_title.lower() or "验收标准" in sec_title:
            for ln in sec_body.splitlines():
                s = ln.strip()
                if s.startswith(("- ", "* ")):
                    cleaned = re.sub(r"^[-*]\s*(?:\[[ xX]\]\s*)?", "", s).strip()
                    if cleaned:
                        criteria.append(cleaned)

    return {
        "schema_version": 1,
        "issue_number": num,
        "title": title,
        "url": issue_obj.get("url", ""),
        "kind": prefix,
        "suggested_branch": f"{prefix}/issue-{num}-{slugify(title)}",
        "target_base": "dev",
        "closing_directive": f"Closes #{num}",
        "areas": areas,
        "agent_ready": bool(criteria),
        "relevant_paths": relevant_paths,
        "acceptance_criteria": criteria,
        "sections": sections,
        "verification_commands": [
            "cargo +1.70.0 check --locked --all-targets && cargo +1.70.0 test --locked",
            "cargo +stable fmt --all -- --check && cargo +stable clippy --all-targets -- -D warnings",
            "python3 -I -m unittest discover -s tests -v",
            "python3 -I scripts/agent-task-context.py --check-line-budget --changed-only",
        ],
    }


def check_line_budget(repo_root: str, changed_only: bool, max_lines: int) -> dict:
    candidates: list[str] = []
    if changed_only:
        out = subprocess.run(
            ["git", "-C", repo_root, "status", "--porcelain"],
            capture_output=True,
            text=True,
            check=False,
        ).stdout
        for line in out.splitlines():
            if len(line) < 4:
                continue
            status, rel = line[:2], line[3:].strip()
            if "D" in status:
                continue
            if " -> " in rel:
                rel = rel.split(" -> ", 1)[1]
            full = os.path.join(repo_root, rel)
            if os.path.isdir(full):
                for root, _, files in os.walk(full):
                    for fn in files:
                        candidates.append(os.path.relpath(os.path.join(root, fn), repo_root))
            else:
                candidates.append(rel)
    else:
        for sub in ("src", "scripts"):
            base = os.path.join(repo_root, sub)
            for root, _, files in os.walk(base):
                for fn in files:
                    candidates.append(os.path.relpath(os.path.join(root, fn), repo_root))

    checked: list[dict] = []
    over: list[dict] = []
    for rel in sorted(set(candidates)):
        if not rel.endswith((".rs", ".py")):
            continue
        fp = os.path.join(repo_root, rel)
        if not os.path.isfile(fp):
            continue
        with open(fp, encoding="utf-8", errors="replace") as f:
            n_lines = sum(1 for _ in f)
        rec = {"path": rel, "lines": n_lines, "within_budget": n_lines <= max_lines}
        checked.append(rec)
        if n_lines > max_lines:
            over.append(rec)

    return {
        "max_lines": max_lines,
        "changed_only": changed_only,
        "checked_count": len(checked),
        "over_budget_count": len(over),
        "over_budget": over,
        "files": checked,
    }


def build_promotion_notes(repo_root: str, git_range: str) -> str:
    log_out = subprocess.run(
        ["git", "-C", repo_root, "log", "--format=%h %s%n%b", git_range],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    issues = extract_closing_numbers(log_out)
    subjects = subprocess.run(
        ["git", "-C", repo_root, "log", "--no-merges", "--format=- `%h` %s", git_range],
        capture_output=True,
        text=True,
        check=False,
    ).stdout.strip()
    lines = ["## Promotion Summary", "", subjects or "- No non-merge commits in range.", ""]
    if issues:
        lines.append("## Linked Issues")
        for i in issues:
            lines.append(f"- Closes #{i}")
        lines.append("")
    return "\n".join(lines)


def sync_github_labels(repo: str) -> None:
    all_labels = {k: v["color"] for k, v in AREA_MAP.items()}
    all_labels.update(EXTRA_LABELS)
    for name, color in sorted(all_labels.items()):
        subprocess.run(
            ["gh", "label", "create", name, "--repo", repo, "--color", color, "--force"],
            check=False,
            capture_output=True,
        )
    print(json.dumps({"synced_labels": sorted(all_labels.keys()), "repo": repo}, indent=2))


def main() -> None:
    ap = argparse.ArgumentParser(description="cc-ide Agent & CI Automation Context Helper")
    ap.add_argument("--issue", type=int, default=0, help="Fetch & parse GitHub issue #<N> into JSON brief")
    ap.add_argument("--issue-json", type=str, default="", help="Parse an offline issue JSON file")
    ap.add_argument("--check-line-budget", action="store_true", help="Check <300-line modular file budget")
    ap.add_argument("--changed-only", action="store_true", help="Only inspect git-modified/untracked files")
    ap.add_argument("--max-lines", type=int, default=300, help="Target line budget per module (default: 300)")
    ap.add_argument("--strict", action="store_true", help="Exit 1 if any inspected file exceeds --max-lines")
    ap.add_argument("--promotion-notes", type=str, default="", help="Generate PR notes + Closes #N for range")
    ap.add_argument("--sync-labels", action="store_true", help="Create/update standard repo labels via gh")
    ap.add_argument("--repo", type=str, default="", help="GitHub owner/repo override")
    args = ap.parse_args()

    repo_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

    if args.issue_json:
        with open(args.issue_json, encoding="utf-8") as f:
            print(json.dumps(parse_issue_to_brief(json.load(f)), indent=2, ensure_ascii=False))
        return

    if args.issue > 0:
        cmd = ["gh", "issue", "view", str(args.issue), "--json", "number,title,body,labels,url"]
        if args.repo:
            cmd.extend(["--repo", args.repo])
        proc = subprocess.run(cmd, capture_output=True, text=True, check=True)
        print(json.dumps(parse_issue_to_brief(json.loads(proc.stdout)), indent=2, ensure_ascii=False))
        return

    if args.check_line_budget:
        rep = check_line_budget(repo_root, args.changed_only, args.max_lines)
        print(json.dumps(rep, indent=2))
        if args.strict and rep["over_budget_count"] > 0:
            sys.exit(1)
        return

    if args.promotion_notes:
        print(build_promotion_notes(repo_root, args.promotion_notes))
        return

    if args.sync_labels:
        target_repo = args.repo or "Visio-Vanitas/ccide"
        sync_github_labels(target_repo)
        return

    ap.print_help()


if __name__ == "__main__":
    main()
