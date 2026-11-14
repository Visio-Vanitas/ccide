---
name: orchestrate
description: Configure heterogeneous subagent/workflow models and effort levels (~/.config/cc-ide/orchestrator.json), run built-in native Workflow templates (ccide-review, ccide-implement, ccide-consensus), or start the local HTTP/SSE Workflow API server.
---

# `/cc-ide:orchestrate` — Heterogeneous Model Orchestration & Native Workflows

`cc-ide` routes `Agent` and `Workflow` calls to role-tuned models and reasoning efforts defined in `~/.config/cc-ide/orchestrator.json` (live hot-reloaded by the Right Sidebar `[ORCH]` panel and applied via `PreToolUse` `updatedInput` + `~/.claude/agents/ccide-{gp,ro}--*.md` shims).

## Orchestrator Presets & Role Overrides

```bash
# Inspect active preset, role bindings, and model_pool
ccide orchestrator status

# Switch preset (balanced | fast | deep-audit | hetero-rr)
ccide orchestrator preset hetero-rr

# Override a role's primary model & effort, then regenerate agent shims
ccide orchestrator set reviewer gemini-4-carbon[1M] high
ccide orchestrator sync
```

## Inline Prompt/Description Overrides

Prefix any `Agent` prompt or description with `[@<model>:<effort>]` (or `[@<model>]`) to override the active preset for that single call without editing config:

- `[@kimi-k3:high] Audit the auth middleware for race conditions`
- `[@deepseek-v4.1-flash[1M]:low] Summarize modified files`

## Workflow Macros (`@scout`, `@coder`, `@reviewer`, `@plan`, `@workflow`)

Built-in templates in `~/.claude/workflows/` (`ccide-review`, `ccide-implement`, `ccide-consensus`) use role macros (`model: "@scout"`, `effort: "@reviewer"`, etc.) that are expanded at dispatch time from `orchestrator.json`:

- **`ccide-review`**: Scout targets -> parallel dimension finders (`@scout`) -> adversarial multi-model judge panel (`@reviewer` pool).
- **`ccide-implement`**: Scout non-overlapping file batches (`@scout`) -> parallel `@coder` workers -> `@reviewer` verification.
- **`ccide-consensus`**: Concurrent candidate generation across models -> `@plan` synthesis.

## Headless Workflow CLI & Local HTTP/SSE API Server

```bash
# List discovered workflows, presets, and model_pool
ccide workflow list --json

# Run a workflow headlessly with streaming NDJSON events
ccide workflow run ccide-review --preset hetero-rr --args '{"targets":["src/"]}' --stream

# Resume a previous workflow run from its journal
ccide workflow run ccide-implement --resume <runId>

# Start the local HTTP/SSE Workflow API server (default 127.0.0.1:18099)
ccide workflow serve --port 18099
```

Server endpoints: `GET /health`, `GET /v1/workflows`, `POST /v1/workflows/run`, and `POST /v1/messages` / `POST /v1/chat/completions` with `model: "workflow/<name>"`.
