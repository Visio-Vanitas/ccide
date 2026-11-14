# Claude Code Extension & Orchestration Interfaces

> **Verified against:** `@anthropic-ai/claude-code` `2.1.304` (`sdk-tools.d.ts`, bundled Bun JS Zod schemas, and `assets/cc-schema.json` baseline `2.1.323`).

---

## 1. Plugin & Marketplace Manifests

### 1.1 Directory Layout & Discovery
- **Plugin root:** Contains `.claude-plugin/plugin.json` (or `plugin.json` at root for local/eval discovery) plus auto-loaded directories: `commands/`, `skills/`, `agents/`, `hooks/hooks.json`, `themes/`, `output-styles/`, `monitors/`, `workflows/`, `.mcp.json`, `.lsp.json`.
- **Cache & Registry:**
  - Installed plugins cache: `~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/`
  - Installed index: `~/.claude/plugins/installed_plugins.json` (`version: 2`, keyed by `<plugin>@<marketplace>`)
  - Marketplace index: `~/.claude/plugins/known_marketplaces.json`
  - CLI override: `--plugin-dir <path>` / `--plugin-dir-no-mcp <path>`
- **Path & Config Variables:**
  - `${CLAUDE_PLUGIN_ROOT}` — absolute path to the resolved plugin directory (expanded in `hooks`, `mcpServers`, `lspServers`, and command strings).
  - `${CLAUDE_PLUGIN_DATA}` — persistent per-plugin data directory.
  - `${CLAUDE_PROJECT_DIR}` — active project root.
  - `${user_config.<KEY>}` — substituted from `userConfig` / `channels[].userConfig` (exported to hooks as `CLAUDE_PLUGIN_OPTION_<KEY>`).

### 1.2 `.claude-plugin/plugin.json`
| Field | Type | Notes |
|---|---|---|
| `name` | `string` (required) | Plugin identifier (`^[A-Za-z0-9][-A-Za-z0-9._]*$`). |
| `version` | `string` | Semver; **wins over** marketplace entry `version` at install time. |
| `description` | `string` | Brief user-facing summary. |
| `displayName` | `string` | Display title override. |
| `author` | `object` (`{name, email?, url?}`) | Creator/maintainer metadata. |
| `homepage` / `repository` | `string` (URL) | Docs and source repo URLs. |
| `license` | `string` | SPDX identifier (e.g. `MIT`, `Apache-2.0`). |
| `keywords` | `string[]` | Discovery tags. |
| `defaultEnabled` | `boolean` | Initial enable state (default `true`; `enabledPlugins` and dependents override). |
| `dependencies` | `(string \| {name, marketplace?})[]` | Required plugins (`plugin` or `plugin@marketplace`). Bare names resolve against the same marketplace. |
| `commands` | `string \| string[] \| Record<string, CommandSpec>` | Overrides auto-loading of `commands/` when set. `CommandSpec`: `{source?, content?, description?, argumentHint?, model?, allowedTools?}`. |
| `agents` | `string \| string[]` | Agent `.md` paths; overrides auto-loading of `agents/`. |
| `skills` | `string \| string[]` | Skill directory paths (`"."` / `"./"` = plugin root); loaded **in addition** to `skills/`. |
| `hooks` | `string \| HooksMap \| Array` | Additional hooks file path(s) or inline `HooksMap` (added to `hooks/hooks.json`). |
| `mcpServers` | `string \| Record<string, McpConfig> \| Array` | Additional MCP server configs/files/MCPB paths (added to `.mcp.json`). |
| `lspServers` | `string \| Record<string, LspConfig>` | LSP configs: `{command, args?, extensionToLanguage, transport?: "stdio"\|"socket", env?, initializationOptions?, settings?, workspaceFolder?, startupTimeout?, shutdownTimeout?, requestTimeout?, restartOnCrash?, maxRestarts?, diagnostics?}`. |
| `outputStyles` | `string \| string[]` | Overrides auto-loading of `output-styles/`. |
| `themes` | `string \| string[]` | Overrides auto-loading of `themes/`. |
| `workflows` | `string \| string[]` | Workflow `.js` file/dir paths; overrides auto-loading of `workflows/`. |
| `monitors` | `string \| string[]` | Monitor definitions. |
| `userConfig` | `Record<string, UserConfigField>` | `{type: "string"\|"number"\|"boolean"\|"directory"\|"file", title, description, required?, default?, multiple?, sensitive?, min?, max?, options?}`. |
| `channels` | `Array<{server, displayName?, userConfig?}>` | Binds an `mcpServers` entry as an assistant-mode message channel. |
| `syntaxHighlighting` | `{hljsLanguages: Array<{id, remote?, integrity?}>}` | Max 16 custom highlight.js languages (`npm:<pkg>[@ver]` or `github:<owner>/<repo>@<ref>#<path>.js`, optional SRI `sha256-\|sha384-\|sha512-...`). |
| `binaries` | `Record<string, {url, sha256, ...}>` | SHA256-pinned binaries fetched into `bin/` at install time. |
| `settings` | `Record<string, unknown>` | Allowlisted settings merged while plugin is enabled. |
| `metadata` | `Record<string, unknown>` | Free-form author metadata. |

### 1.3 `.claude-plugin/marketplace.json`
- **Top-level fields:** `$schema?`, `name` (required), `owner` (required), `version?`, `description?`, `plugins` (required array), `forceRemoveDeletedPlugins?`, `metadata?: {pluginRoot?, version?, description?}`, `allowCrossMarketplaceDependenciesOn?: string[]`, `renames?: Record<string, string | null>`.
- **Plugin entry fields:** `name`, `source` (relative `"./..."` path, bare name if `metadata.pluginRoot` is set, or object `{source: "npm"|"url"|"github"|"git-subdir"|"archive"|"command", ...}`), `description?`, `version?`, `category?`, `tags?`, `strict?: boolean` (default `true`; `false` allows inline manifest fields when the plugin folder lacks `plugin.json`), `headers?`, `headersHelper?` (requires `strict: false` in catalog entries), `relevance?`.
- **Marketplace source types (in `known_marketplaces.json` / settings):** `url`, `github` (`owner/repo[#ref]`), `git` (`path`, `sparsePaths`), `npm`, `file`, `directory`, `skills-dir`, `settings` (inline).

---

## 2. Hook Events (`hooks/hooks.json` & `settings.json`)

### 2.1 Hook Entry Types (`hooks.<EventName>[].hooks[]`)
All hook types support optional `if` (condition), `timeout` (seconds), `statusMessage` (spinner text), `once?: boolean` (run once then remove).
- **`type: "command"`**: `{command, args?, shell?, timeout?, onFailure?, async?: boolean, asyncTimeout?, asyncRewake?: boolean, rewakeMessage?, rewakeSummary?, cloud?: "device"|"skip"}`
  - `async: true` runs in background without blocking. `asyncRewake: true` implies `async` and wakes the model on exit code `2`.
- **`type: "prompt"`**: `{prompt, model?, continueOnBlock?}` (`$ARGUMENTS` = hook input JSON; defaults to small fast model).
- **`type: "agent"`**: `{prompt, model?, timeout?}` (default timeout `60s`, default model Haiku).
- **`type: "http"`**: `{url, headers?, allowedEnvVars?, onFailure?, cloud?}` (only `allowedEnvVars` are interpolated in `headers` via `$VAR` / `${VAR}`).
- **`type: "mcp_tool"`**: `{server, tool, input?}` (string values in `input` support `${path}` interpolation from hook input JSON).

Also in `hooks/hooks.json`: `modules?: string[]` (max 1 ES module path exporting `register(on)`).

### 2.2 Common Stdin JSON Envelope
Every hook receives JSON on `stdin` with these base fields:
```ts
{
  session_id: string;
  transcript_path: string;        // "" if persistence disabled or served remote call
  cwd: string;
  prompt_id?: string;             // UUID correlating user prompt -> OTel prompt.id
  permission_mode?: string;
  hook_event_name: HookEventName;
  agent_id?: string;              // Present ONLY inside a subagent (absent on main thread, even with --agent)
  agent_type?: string;            // Present inside subagent or on main thread started with --agent
  effort?: { level: string };     // Present in tool-use context on effort-capable models (also CLAUDE_EFFORT env)
  scratchpad_dir?: string;
}
```

### 2.3 All 33 Hook Events, Matchers, Exit Codes & Payloads
- **Exit code `0`**: Success; parse stdout JSON (if valid JSON) or treat stdout as plain context where supported.
- **Exit code `2`**: Blocking / feedback error (exact effect per event below).
- **Other exit codes**: Non-blocking; stderr shown to user only (or debug-logged).

| Event | `fieldToMatch` | Exit `2` Behavior | Event-Specific Stdin Fields | `hookSpecificOutput` Fields |
|---|---|---|---|---|
| `PreToolUse` | `tool_name` | Block tool call; stderr -> model | `tool_name`, `tool_input`, `tool_use_id`, `mcp_server?` | `permissionDecision?: "allow"\|"deny"\|"ask"\|"defer"`, `permissionDecisionReason?`, `updatedInput?: Record<string, unknown>`, `additionalContext?` |
| `PostToolUse` | `tool_name` | Stderr -> model immediately | `tool_name`, `tool_input`, `tool_response`, `tool_use_id`, `duration_ms?`, `mcp_server?` | `additionalContext?`, `classifierContext?` (<=2000 UTF-16), `updatedToolOutput?`, `updatedMCPToolOutput?` |
| `PostToolUseFailure` | `tool_name` | Stderr -> model immediately | `tool_name`, `tool_input`, `tool_use_id`, `error`, `is_interrupt?`, `duration_ms?`, `mcp_server?` | `additionalContext?` |
| `PostToolBatch` | *(none)* | Stop agentic loop (stderr -> user) | `tool_calls: [{tool_name, tool_input, tool_use_id, tool_response?}]` | `additionalContext?` |
| `PermissionRequest` | `tool_name` | Stderr -> user | `tool_name`, `tool_input`, `permission_suggestions?`, `mcp_server?` | `decision: {behavior: "allow", updatedInput?, updatedPermissions?} \| {behavior: "deny", message?, interrupt?}` |
| `PermissionDenied` | `tool_name` | Stderr -> user | `tool_name`, `tool_input`, `tool_use_id`, `reason`, `mcp_server?` | `retry?: boolean` |
| `UserPromptSubmit` | *(none)* | Block & erase prompt; stderr -> user | `prompt`, `source?: "user"\|"sdk"\|"system"\|"loop_wakeup"\|"schedule_wakeup"\|"poll_event"`, `session_title?` | `additionalContext?`, `sessionTitle?`, `suppressOriginalPrompt?: boolean` |
| `UserPromptExpansion` | `command_name` | Block expansion; stderr -> user | `expansion_type: "slash_command"\|"mcp_prompt"`, `command_name`, `command_args`, `command_source?`, `prompt` | `additionalContext?`, `suppressOriginalPrompt?: boolean` |
| `SessionStart` | `source` (`startup\|resume\|clear\|compact\|fork`) | Stderr -> user | `source`, `agent_type?`, `model?`, `session_title?`, `seconds_since_last_response?`, `context_tokens?`, `prompt_cache_likely_expired?`, `estimated_cache_write_usd?` | `additionalContext?`, `initialUserMessage?`, `sessionTitle?`, `watchPaths?: string[]`, `reloadSkills?: boolean` |
| `SessionEnd` | `reason` (`clear\|resume\|logout\|prompt_input_exit\|other`) | Stderr -> user | `reason` | *(none)* |
| `Setup` | `trigger` (`init\|maintenance`) | Stderr -> user | `trigger` | `additionalContext?` |
| `Stop` | *(none)* | Stderr -> model & continue turn | `stop_hook_active`, `last_assistant_message?`, `background_tasks?`, `session_crons?` | `additionalContext?` |
| `StopFailure` | `error` | Ignored (fire-and-forget) | `error`, `error_details?`, `last_assistant_message?` | *(none)* |
| `SubagentStart` | `agent_type` | Stderr -> user | `agent_id`, `agent_type` | `additionalContext?` (shown to subagent) |
| `SubagentStop` | `agent_type` | Stderr -> subagent & continue | `stop_hook_active`, `agent_id`, `agent_transcript_path`, `agent_type`, `last_assistant_message?`, `background_tasks?`, `session_crons?` | `additionalContext?` |
| `PreCompact` | `trigger` (`manual\|auto`) | Block compaction | `trigger`, `custom_instructions: string \| null` | *(none; exit 0 stdout -> custom compact instructions)* |
| `PostCompact` | `trigger` (`manual\|auto`) | Stderr -> user | `trigger`, `compact_summary` | *(none; exit 0 stdout -> user)* |
| `PreModelSwitch` | `to_model` | Block switch; stderr -> user | `from_model`, `to_model`, `requested_model`, `source` (`command\|picker\|sdk`), `context_tokens`, `prompt_cache_warm`, `cache_ttl`, `estimated_cache_write_usd`, `pricing` | `permissionDecision?: "allow"\|"deny"\|"ask"`, `permissionDecisionReason?` |
| `PostModelSwitch` | `to_model` | Stderr -> user | Same as `PreModelSwitch` (`source` also allows `auto\|resume`) | `additionalContext?` (shown to Claude on next request) |
| `Notification` | `notification_type` | Stderr -> user | `message`, `title?`, `notification_type` | `additionalContext?` |
| `TeammateIdle` | *(none)* | Stderr -> teammate & prevent idle | `teammate_name`, `team_name` *(deprecated)* | *(none)* |
| `TaskCreated` | *(none)* | Stderr -> model & block creation | `task_id`, `task_subject`, `task_description?`, `teammate_name?`, `team_name?` | *(none)* |
| `TaskCompleted` | *(none)* | Stderr -> model & block completion | `task_id`, `task_subject`, `task_description?`, `teammate_name?`, `team_name?` | *(none)* |
| `Elicitation` | `mcp_server_name` | Deny elicitation | `mcp_server_name`, `message`, `mode?: "form"\|"url"`, `url?`, `elicitation_id?`, `requested_schema?` | `action?: "accept"\|"decline"\|"cancel"`, `content?: Record<string, unknown>` |
| `ElicitationResult` | `mcp_server_name` | Block response (becomes `decline`) | `mcp_server_name`, `elicitation_id?`, `mode?`, `action`, `content?` | `action?: "accept"\|"decline"\|"cancel"`, `content?: Record<string, unknown>` |
| `ConfigChange` | `source` | Block config change | `source` (`user_settings\|project_settings\|local_settings\|policy_settings\|skills`), `file_path?` | *(none)* |
| `InstructionsLoaded` | `load_reason` | Observability-only (no block) | `file_path`, `memory_type` (`User\|Project\|Local\|Managed`), `load_reason` (`session_start\|nested_traversal\|path_glob_match\|include\|compact`), `globs?`, `trigger_file_path?`, `parent_file_path?` | *(none)* |
| `WorktreeCreate` | *(none)* | Creation failed | `name` (suggested slug) | `worktreePath: string` (or stdout = abs path) |
| `WorktreeRemove` | *(none)* | Stderr -> user | `worktree_path` | *(none)* |
| `CwdChanged` | *(none)* | Stderr -> user | `old_cwd`, `new_cwd` (`CLAUDE_ENV_FILE` set) | `watchPaths?: string[]` |
| `FileChanged` | filename (e.g. `.envrc\|.env`) | Stderr -> user | `file_path`, `event: "change"\|"add"\|"unlink"` (`CLAUDE_ENV_FILE` set) | `watchPaths?: string[]` |
| `DirectoryAdded` | `source` (`slash_command\|register_repo_root`) | Debug-logged / summarized | `directory`, `source` | *(none)* |
| `MessageDisplay` | *(none)* | Display original delta | `turn_id`, `message_id`, `index`, `final`, `delta` | `displayContent?: string` (display-only) |

**Top-level stdout JSON fields (optional on any hook):** `continue?: boolean`, `suppressOutput?: boolean`, `stopReason?: string`, `decision?: "approve"|"block"`, `reason?: string`, `systemMessage?: string`, `terminalSequence?: string` (OSC 0/1/2/9/99/777 & BEL only).

---

## 3. Subagents & Custom Agents

### 3.1 Definition Locations & Schema
- **Files:** `.claude/agents/*.md`, `~/.claude/agents/*.md`, plugin `agents/*.md` (YAML frontmatter + Markdown system prompt body).
- **CLI inline:** `--agents '<json>'` (object keyed by agent name).

| Field | Type | Notes |
|---|---|---|
| `name` | `string` (required) | Agent identifier for `Agent` tool (`subagent_type`) and `--agent`. |
| `description` | `string` (required) | Shown in `Agent` tool listing. |
| `model` | `string` | **Arbitrary string allowed** in frontmatter / `--agents` (e.g. `"sonnet"`, `"claude-sonnet-4-6"`, gateway ID, or `"inherit"`). Contrast with `Agent` tool input schema, which restricts `model` to `"sonnet" \| "opus" \| "haiku" \| "fable"`. |
| `effort` | `"low" \| "medium" \| "high" \| "xhigh" \| "max" \| "lower" \| "higher" \| number` | Reasoning effort (`lower`/`higher` step relative to inherited effort; `Agent` tool accepts `"low"\|"medium"\|"high"\|"xhigh"\|"max"`). |
| `tools` | `string \| string[]` | Replaces default tool set (`"*"` for all). |
| `disallowedTools` | `string \| string[]` | Removed from default set (ignored if `tools` is set). |
| `permissionMode` | `"acceptEdits" \| "auto" \| "bypassPermissions" \| "default" \| "dontAsk" \| "plan"` | Permission mode override. |
| `isolation` | `"worktree" \| "remote"` | `"worktree"` runs in a temp git worktree (auto-cleaned if unchanged). |
| `mcpServers` | `object \| array` | MCP servers connected while agent runs. |
| `hooks` | `HooksMap` | Scoped hooks active while agent runs. |
| `maxTurns` | `number` | Max API round-trips before stopping. |
| `autoCompactWindow` | `number` | Token threshold for subagent self-compaction (only lowers inherited window; no effect on main session). |
| `skills` | `string[]` | Skill names preloaded into agent context. |
| `initialPrompt` | `string` | Auto-submitted first user turn when running as main thread (`--agent`); ignored for subagents. |
| `memory` | `"user" \| "project" \| "local"` | Persistent agent memory scope. |
| `background` | `boolean` | Run in background by default. |
| `omitClaudeMd` | `boolean` | Omit user/project/local `CLAUDE.md` when spawned as subagent (managed policy kept). |
| `color` | `string` | Display color in agents UI. |
| `observer` / `observerMessage` / `observeSubagents` | `string` / `string` / `boolean` | Auto-spawned background observer agent config. |
| `experimental.cacheTtl` | `"5m" \| "1h"` | Prompt cache TTL override. |

### 3.2 Exact Model-Resolution Precedence
Evaluated in `Ej()` (`Lbs()`):
1. **`CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL`** (when inside a `Workflow` run and not `"inherit"`): overrides call-site `model` and agent frontmatter `model`.
2. **`CLAUDE_CODE_SUBAGENT_MODEL_FORCE`**: when truthy, strips call-site `model` (unless `"inherit"`) and frontmatter `model`.
3. **Call-site `model`** (`Agent({ model })`): overrides agent frontmatter and `CLAUDE_CODE_SUBAGENT_MODEL` (ignored for `subagent_type: "fork"`).
4. **Agent frontmatter `model`** (or `--agents` JSON `model`; `"inherit"` resolves to parent `mainLoopModel`).
5. **`CLAUDE_CODE_SUBAGENT_MODEL`** (from env / `settings.json` `env`, if set and not `"inherit"`).
6. **Parent `mainLoopModel`** (fallback).

*(If the resolved model is outside `availableModels`, Claude Code steps down to the newest allowed model in the same family or falls back to parent/env.)*

---

## 4. Native `Workflow` Orchestration Engine

### 4.1 Locations & Invocation
- **Locations:** `.claude/workflows/*.js`, `~/.claude/workflows/*.js`, plugin `workflows/*.js`.
- **Tool (`WorkflowInput`):** `{ script?: string, name?: string, scriptPath?: string, args?: Record<string, unknown>, resumeFromRunId?: string }`. `scriptPath` takes precedence over `script` and `name`.
- **Headless:** `claude -p --output-format stream-json ...` (streams workflow progress and agent events).

### 4.2 Script Contract (`export const meta` + VM Globals)
- Scripts are **plain JavaScript** (not TypeScript) running in an async VM context.
- **Forbidden non-deterministic globals:** `Date.now()`, `Math.random()`, and argless `new Date()` throw (to preserve replay determinism); no direct `fs`/Node APIs.
- **Required header:** `export const meta = { name, description, whenToUse?, phases?: [{ title, detail?, model? }] }` — must be a **pure object literal** (no variables, calls, or spreads).

**Runtime Globals:**
- `agent(prompt, opts?) => Promise<string | object | null>`
  - `opts`: `{ label?, phase?, schema?, model?: string, effort?, isolation?: "worktree", agentType?, disallowedTools?: string[], bashCommandClamp?: string[], stallMs?: number }`
  - `model` accepts **any string** (unlike `Agent` tool's 4-enum).
  - `schema` (JSON Schema `{type: "object", properties: {...}}`) forces `StructuredOutput` tool use and returns the validated object.
  - `bashCommandClamp`: array of `'Bash(<command or prefix>)'` rules; malformed entries refuse spawn.
  - `stallMs`: per-agent stall timeout override (ms).
  - Returns `null` if skipped or terminal API error occurs.
- `pipeline(items, ...stages) => Promise<any[]>` — per-item streaming stages without cross-item barriers; each stage receives `(prevResult, originalItem, index)`.
- `parallel(thunks) => Promise<any[]>` — barrier awaiting all `() => Promise<any>`; failed thunks resolve to `null`.
- `phase(title)` / `log(message)` — progress group & narrator line.
- `workflow(nameOrRef, args?) => Promise<any>` — inline child workflow (max depth 1; shares concurrency cap, abort signal, and `budget`).
- `args` — verbatim value of `WorkflowInput.args` (pass JSON arrays/objects directly, not stringified).
- `budget` — `{ total: number | null, spent(): number, remaining(): number }`; hard ceiling on output tokens across main loop + workflows.

### 4.3 Replay Journal & Resume
- Persisted to `<transcriptDir>/journal.jsonl` (or V5 sidecar storage).
- Passing `resumeFromRunId` (same session only, after stopping prior run via `TaskStop`) replays completed `agent()` calls whose `(prompt, sanitizedOpts)` hash matches, re-running only modified or new calls.

---

## 5. `statusLine` Protocol

Configured via `settings.json` (`statusLine: { type: "command", command: "...", padding?: number, refreshInterval?: number }`). Invoked with JSON on `stdin`; stdout (with ANSI colors & OSC 8 links) is rendered as the bottom status bar.

### Stdin JSON Structure (`StatusLineCommandInput`)
```ts
{
  session_id: string;
  session_name?: string;
  transcript_path: string;
  cwd: string;
  scratchpad_dir?: string;
  prompt_id?: string;
  permission_mode?: string;
  version: string;
  model: { id: string; display_name: string };
  output_style: { name: string };
  workspace: {
    current_dir: string;
    project_dir: string;
    added_dirs: string[];
    git_worktree?: string;
    repo?: { owner: string; name: string; ... };
  };
  cost: {
    total_cost_usd: number;
    total_duration_ms: number;
    total_api_duration_ms: number;
    total_lines_added: number;
    total_lines_removed: number;
  };
  context_window: {
    total_input_tokens: number;
    total_output_tokens: number;
    context_window_size: number;
    current_usage: UsageObject | null;
    used_percentage: number;
    remaining_percentage: number;
  };
  exceeds_200k_tokens: boolean;
  fast_mode: boolean;
  effort?: { level: string };
  thinking?: { enabled: boolean };
  prompt_cache?: {
    warm: boolean; caching_observed: boolean; ttl: "5m" | "1h";
    expires_at: number | null; requests: number; misses: number;
    expected_rebuilds: number; hit_ratio: number;
    cache_write_tokens: number; miss_recache_tokens: number;
    last_miss_at: number | null; last_miss_cause: { causes: string[] } | null;
  };
  rate_limits?: {
    five_hour?: { used_percentage: number; resets_at: number };
    seven_day?: { used_percentage: number; resets_at: number };
    spend_limit?: { used_percentage: number; resets_at: number; used_usd?: number; limit_usd?: number; period?: string };
  };
  vim?: { mode: string };
  agent?: { name: string };
  remote?: { session_id: string };
  pr?: { number: number; url: string; review_state?: string; kind?: string };
  worktree?: { name: string; path: string; branch: string; original_cwd: string; original_branch: string };
}
```

---

## 6. Skills & Slash Commands (`skills/<name>/SKILL.md`, `.claude/commands/*.md`)

| Frontmatter Key | Type | Notes |
|---|---|---|
| `name` | `string` | Skill/command name (defaults to directory/file name). |
| `description` | `string` | Shown in `/help` and `Skill` tool listing. |
| `when_to_use` | `string` | Guidance appended to `Skill` tool description. |
| `argument-hint` | `string` | Placeholder shown after `/command` (e.g. `"[file]"`). |
| `allowed-tools` | `string \| string[]` | Comma-separated or YAML list of tools allowed while active. |
| `disallowed-tools` / `disallowedTools` | `string \| string[]` | Tools removed while active (cleared on next user message). |
| `model` | `string` | `"haiku" \| "sonnet" \| "opus" \| "fable"`, full ID, or `"inherit"`. |
| `effort` | `string \| number` | `"low" \| "medium" \| "high" \| "max"` or integer. |
| `disable-model-invocation` | `boolean` | If `true`, only user can invoke via `/slash` (hidden from `Skill` tool). |
| `user-invocable` | `boolean` | If `false`, hides slash command from user (model-only via `Skill`). |
| `context` | `"inline" \| "fork"` | `"inline"` expands into current chat; `"fork"` spawns a subagent. |
| `agent` | `string` | Agent type used when `context: fork`. |
| `paths` | `string \| string[]` | Glob patterns; skill only loads when model touches matching files. |
| `hooks` | `HooksMap` | Scoped hooks active while skill runs. |
| `shell` | `"bash" \| "powershell"` | Shell for `!` command blocks (default `"bash"`). |

---

## 7. On-Disk Session, Transcript & Subagent State (`~/.claude/`)

- **`~/.claude/sessions/<pid>.json`**:
  `{ pid?, cwd, startedAt, procStart?, procStartFt?, kind?: "interactive"|"bg"|"daemon"|"daemon-worker", sessionId?, status?: "busy"|"shell"|"idle"|"waiting", entrypoint?, pidDomain? }`
- **`~/.claude/projects/<slug>/<sessionId>.jsonl`**:
  - `user` / `assistant`: `{ type, uuid, parentUuid?, timestamp, sessionId, cwd, version, gitBranch?, isSidechain?, agentId?, promptId?, requestId?, isMeta?, isCompactSummary?, message: { role, id?, model?, stop_reason?, content, usage?: { input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens, cache_creation?: { ephemeral_5m_input_tokens, ephemeral_1h_input_tokens }, server_tool_use?: { web_search_requests, web_fetch_requests }, service_tier?, speed?, inference_geo?, output_tokens_details?: { thinking_tokens? }, iterations?, fallback_credit? } } }`
  - `system` (`subtype: "compact_boundary"`): `{ type: "system", subtype: "compact_boundary", compact_metadata: { trigger: "manual"|"auto", pre_tokens, post_tokens?, cumulative_dropped_tokens? }, uuid, timestamp }`
  - `permission-mode`: `{ type: "permission-mode", permissionMode }` (last-wins).
  - `api-request` / `api-request-shape` / `api-request-blob`: Request replay records (`shapeHash`, `params`, `base`, `keep`, `tail`, `cacheControl`, `requestDigest`, `blobCandidates`).
- **`~/.claude/projects/<slug>/<sessionId>/subagents/`**:
  - `agent-<id>.jsonl`: Subagent transcript stream.
  - `agent-<id>.meta.json`: `{ agentType, customAgentType?, spawnedAgentType?, name?, description?, model?, effort?, cwd?, permissionMode?, taskKind?, teamName?, toolUseId?, color?, isBuiltIn?, isFork?, isObserver?, observer?, observerStopped?, observerTaskId?, parentAgentId?, spawnDepth?, spawnMode?, worktreePath?, worktreeBranch?, workflowPhase?, finished?, stoppedByUser?, revivedAt? }`
- **`~/.claude/tasks/<sessionId>/*.json`**:
  `{ id, subject, description?, activeForm?, status, owner?, blocks?: string[], blockedBy?: string[] }`

---

## 8. Key Environment Variables & CLI Flags

### 8.1 Environment Variables
| Variable | Purpose |
|---|---|
| `ANTHROPIC_DEFAULT_MODEL` | Default primary model override. |
| `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU,FABLE}_MODEL` | Alias-to-model-ID mapping for gateways/Bedrock/Vertex (`*_NAME`, `*_DESCRIPTION`, `*_SUPPORTED_CAPABILITIES` also supported). |
| `CLAUDE_CODE_SUBAGENT_MODEL` | Default subagent model when not overridden by call-site or frontmatter. |
| `CLAUDE_CODE_SUBAGENT_MODEL_FORCE` | Force-ignores call-site and frontmatter `model` overrides. |
| `CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL` | Forces all `Workflow` `agent()` calls onto one model. |
| `CLAUDE_CODE_WORKFLOW_MAX_CONCURRENT_AGENTS` | Concurrency cap for `Workflow` subagents. |
| `CLAUDE_CODE_WORKFLOW_PREFIX_STAGGER_MS` | Prefix-cache stagger delay (ms) between parallel workflow spawns. |
| `CLAUDE_CODE_AUTO_COMPACT_WINDOW` | Override effective auto-compact token window. |
| `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` | Override auto-compact threshold percentage. |
| `CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS` | Max wait (ms) for background tasks in `-p` mode (`0` = wait indefinitely). |
| `CLAUDE_CODE_OVERLOADED_RETRY_MAX_DELAY_MS` | Max backoff delay (ms) for HTTP 529 overloaded retries. |
| `CLAUDE_CODE_ENTRYPOINT` | Entrypoint tag (`cli`, `sdk-cli`, `claude-vscode`, etc.). |

### 8.2 Orchestration & Headless CLI Flags
- `-p` / `--print` — non-interactive headless mode.
- `--input-format stream-json` / `--output-format stream-json` (`--verbose`, `--include-partial-messages`) — NDJSON bidirectional SDK/pipe protocol.
- `--json-schema '<schema>'` — enforce structured JSON output on final result.
- `--agents '<json>'` — inline custom agent definitions map.
- `--agent <name>` — run main session as a named custom agent.
- `--plugin-dir <path>` / `--plugin-dir-no-mcp <path>` — load local plugin(s) for the session.
- `--fork-session` / `--resume-session-at <uuid>` — branch or resume at a specific message UUID.
