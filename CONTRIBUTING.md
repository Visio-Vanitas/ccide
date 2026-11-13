# Contributing & AI Agent Playbook (`cc-ide`)

`cc-ide` (`ccide-core`) is structured so both human contributors and autonomous coding agents (`Claude Code`, subagents, and CI bots) can diagnose, implement, verify, and ship changes deterministically.

---

## 1. Machine-Readable Automation Interfaces

| Interface | Command | Output / Purpose |
|---|---|---|
| **Environment & Anchor Diagnostics** | `ccide doctor --json` (`./install.sh --doctor --json` / `.\install.ps1 -Doctor -Json`) | Schema-v1 JSON with OS/arch, active workspace backend, companion versions, and `claude.exe` structural anchor status |
| **Cross-OS Symbol & Schema Workbench** | `ccide --check-cc [--local \| --fetch-npm [ver]] [--json]` | Probes `darwin-arm64`, `linux-x64`, and `win32-x64` Bun binaries simultaneously, extracts missed-anchor byte snippets, and diffs `assets/cc-schema.json` |
| **Claude Binary Patcher Probe** | `python3 -I scripts/patch_claude_binary.py --verify [--binary <path>]` | Dry-run JSON report (`required_ok`, per-patch byte offsets, resolved runtime symbols, V8 ESM syntax validation) |
| **Issue -> Agent Task Brief** | `python3 -I scripts/agent-task-context.py --issue <N>` | Structured JSON brief mapping an Issue Form / Markdown issue to `areas`, `relevant_paths`, `suggested_branch`, `acceptance_criteria`, and `Closes #<N>` |
| **Modular File Size Gate** | `python3 -I scripts/agent-task-context.py --check-line-budget --changed-only` | JSON report checking touched `.rs` / `.py` modules against the `< 300`-line single-responsibility target |
| **Release Promotion Notes** | `python3 -I scripts/agent-task-context.py --promotion-notes origin/main..origin/dev` | Markdown summary + aggregated `Closes #<N>` directives for `dev -> preview -> main` promotion PRs |
| **Pricing Catalog Sync** | `ccide --sync-pricing` | Refreshes multi-vendor token & cache pricing from `BerriAI/litellm` and `pydantic/genai-prices` |

---

## 2. Core Engineering Invariants

1. **Rust Edition 2021 & Strict MSRV `1.70.0` Floor**:
   - Zero external Cargo crates (`Cargo.lock` contains only `ccide-core` itself) and zero post-1.70 stdlib APIs.
   - Always verify with `cargo +1.70.0 check --locked --all-targets` and `cargo +1.70.0 test --locked`.
2. **Modular `< 300`-Line Single-Responsibility Files**:
   - Keep individual source files concise (`< 300` lines wherever practical) by splitting TUI panels, parsers, IPC bridges, and OS probes into focused submodules.
3. **Strict Per-Tab Process Isolation**:
   - Never introduce global cross-session state files (like `live_latest.json`); scope state by `sessionId`, `yazi_id`, or `claude_pid` ancestry.
4. **Idempotent & Reversible User System Changes**:
   - Any modification to user configs (`~/.config/yazi/*`, `~/.ssh/config`, `com.googlecode.iterm2.plist`, `claude.exe`) must back up original state first, skip redundant writes on repeat runs, and restore cleanly via `--uninstall` / `--restore-*`.
5. **Isolated Python Execution (`python3 -I`)**:
   - Run repository Python scripts and unit tests with `-I` so untrusted working directories or downloaded archives cannot hijack module imports.

---

## 3. Branch & Issue-Closure Flow (`topic -> dev -> preview -> main`)

`.github/workflows/ci.yml` (`branch-flow-guard`) and `.github/workflows/issue-pr-lifecycle.yml` enforce a three-stage promotion pipeline:

1. **Topic / Fix PRs (`fix/issue-<N>-...` or `feat/issue-<N>-...` -> `dev`)**:
   - Branch off `dev`, include `Closes #<N>` in both the commit message and the PR body (`.github/PULL_REQUEST_TEMPLATE.md`), and target `dev`.
   - `issue-pr-lifecycle.yml` automatically labels the issue (`has-pr`, `area:*`, `agent-ready`) and transitions it to `stage:in-dev` when merged into `dev`.
2. **RC Preview Promotion (`dev` -> `preview`)**:
   - Runs live upstream `@anthropic-ai/claude-code@latest` verification, transitions linked issues to `stage:in-preview`, and publishes `v<ver>-rc.<run>` pre-releases.
3. **Immutable Stable Promotion (`preview` -> `main`)**:
   - Publishes immutable `v<ver>` binaries + `SHA256SUMS.txt` across all 6 targets (`aarch64`/`x86_64` × `macOS`/`Linux`/`Windows MSVC`) and automatically closes all promoted `Closes #<N>` issues on `main`.

---

## 4. Pre-PR Verification Checklist

Run these four commands locally before opening any PR:

```bash
cargo +1.70.0 check --locked --all-targets && cargo +1.70.0 test --locked
cargo +stable fmt --all -- --check && cargo +stable clippy --all-targets -- -D warnings
python3 -I -m unittest discover -s tests -v
sh -n scripts/ensure-ccide-core.sh && zsh -n install.sh scripts/ccide scripts/pane-runner.sh
```
