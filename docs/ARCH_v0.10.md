# cc-ide v0.10.0 — Frozen Architecture Contract

Status: **FROZEN for Wave 4**. Signatures, wire formats and CLI contracts below are binding; any
change needs an `@arch-opus` amendment on the board. Baseline: `dev` @ `2572c1a` (v0.9.4).
Global rules: MSRV `1.70.0`, zero crates, every `.rs` ≤ 300 lines after `rustfmt` (target ≤ 285),
no `crate::commands` in `src/{ffi,telemetry,session,tui,orchestrator,json,budget,platform}` or in
leaf files `src/{model_id,proto}.rs`, all IPC paths via `platform::ipc_path`, all writes of shared
state via `platform::atomic_write`, every signal via `ffi::wake_token`.

## 1. `src/commands/workspace/spec.rs` (NEW, pure + 3 I/O fns)
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend { Iterm, Osc, Tmux, WezTerm, Wt, Ghostty }
impl Backend { pub fn as_str(self) -> &'static str; pub fn from_name(s: &str) -> Option<Self>; } // names below; aliases iterm2, ssh-osc
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneSpec {
    pub backend: Backend,
    pub yid: String,              // tab key (yazi client id); always set (legacy: from caller arg)
    pub center: String,           // tmux %N / wezterm id / ghostty terminal id / iTerm caller guid; "" for wt, osc
    pub yazi: Option<String>, pub lazygit: Option<String>, pub mon: Option<String>, pub sidebar: Option<String>,
    pub left_pct: u8,             // as requested at open (clamped by LayoutPlan, not here)
    pub show_git: bool, pub show_mon: bool, pub show_right: bool,
    pub legacy_sides: Vec<String>,// v1 only: unclassified side ids in creation order (yazi moved to `yazi`)
    pub legacy: bool,             // true iff parsed from v1
}
impl PaneSpec {
    pub fn parse(raw: &str, yid_hint: &str) -> Option<Self>;   // v2 or v1, see grammar
    pub fn encode_v2(&self) -> Option<String>;  // None if any value violates the value charset
    pub fn encode(&self) -> String;             // encode_v2() or else v1 fallback (never fails)
    pub fn all_companion_ids(&self) -> Vec<&str>; // yazi,lazygit,mon,sidebar,legacy_sides; dedup; never center
}
pub struct OpenRequest<'a> {
    pub home: &'a str, pub target_dir: &'a str, pub yazi_id: &'a str, pub git_root: &'a str,
    pub show_git: bool, pub show_mon: bool, pub show_right: bool, pub left_pct: u8, pub shell_pid: u32,
}
pub fn read_pane_spec(yid: &str) -> Option<PaneSpec>;                   // ipc_path("panes_<yid>.txt")
pub fn write_pane_spec(spec: &PaneSpec) -> std::io::Result<()>;        // atomic_write(encode())
pub fn remove_tab_state(yid: &str);  // panes_<yid>.txt, ghostty_w_<yid>.txt, geom_<yid>_*.txt
pub fn prune_dead_specs(home: &str) -> usize; // remove_tab_state for yids whose tab shell is dead
```
`as_str` names: `iterm`, `osc`, `tmux`, `wezterm`, `wt`, `ghostty`. **v2 grammar** (single line, ASCII, no whitespace; `;`-separated `key=value`, fixed key order on encode,
any order on parse, unknown keys ignored):
`v2;b=<backend>;k=<yid>;c=<center>;y=<id>;l=<id>;m=<id>;s=<id>;lp=<u8>;f=<flags>`
- Omit `y/l/m/s` when `None`; omit `c` when empty. `f` = subset of `g`,`m`,`r` (show_git/mon/right), e.g. `f=gr`.
- Value charset: any byte except `;` `=` `,` whitespace and control chars (`:` and `%` ARE allowed, e.g.
  tmux `%12`, iTerm `w0t0p1:GUID`). `encode_v2()` returns `None` on violation → `encode()` writes v1.
- Example: `v2;b=tmux;k=1234;c=%1;y=%2;l=%3;s=%4;lp=30;f=gr`. iTerm (the daemon replies with a comma id list): assign
  roles in the order yazi, lazygit(g), mon(m), sidebar(r) only if the count matches the flags, else use `legacy_sides`.
  `close` sends `all_companion_ids().join(",")` to the daemon.
**v1 parse** (legacy `<backend>:<rest>`): `tmux|wezterm|ghostty` → `<center>:<s1,s2,…>` (first side →
`yazi`, rest → `legacy_sides`); `wt|osc` → `<rest>` is the yid; `iterm:<ids>` or a bare prefix-less
id list → `Iterm` with all ids (first → `yazi`). Flags default `show_git=false, show_mon=false,
show_right = legacy_sides non-empty`, `left_pct=30`, `legacy=true`. `yid` = `k`, else the v1 yid for
wt/osc, else `yid_hint`.

**Tab liveness** (`prune_dead_specs`): read `<cache_dir>/tab_shell_<yid>.txt` → `ProcToken::parse`;
`start_ms>0` → `is_alive_match()`; bare pid (legacy) → `is_pid_alive(pid)`; missing file → keep.

**Ownership rule:** only `open.rs` calls `write_pane_spec`, and it prints `spec.encode()` as the
sole stdout line of `workspace open`. Backends return `Option<PaneSpec>` and never write `panes_*`.
Launchers round-trip that line to `workspace close "<spec>" <yid>`; `close` accepts v1 or v2.
Compat note (CHANGELOG): already-running pre-0.10 sidebars cannot parse v2 specs, so center-focus
on click degrades until that tab is reopened; close/rebalance use the new binary and are unaffected.

## 2. `src/commands/workspace/layout.rs` (NEW, pure)
```rust
pub const LEFT_MIN: usize = 22; pub const CENTER_MIN: usize = 48; pub const RIGHT_MIN: usize = 28; pub const RIGHT_PCT: usize = 16;
pub const COLLAPSE_RIGHT_BELOW: usize = 100; pub const COLLAPSE_LEFT_BELOW: usize = 76; pub const LEFT_PCT_MIN: u8 = 12; pub const LEFT_PCT_MAX: u8 = 45;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutPlan { pub total_cols: usize, pub left_cols: usize, pub center_cols: usize,
                        pub right_cols: usize, pub collapse_left: bool, pub collapse_right: bool }
impl LayoutPlan {
    pub fn compute(total_cols: usize, left_pct: u8, show_right: bool) -> Self;
    pub fn tmux_lengths(&self) -> (usize, usize);            // (-l left, -l right) cells
    pub fn wezterm_percents(&self) -> (u8, u8);              // (left % of total, right % of center+right)
    pub fn wt_fractions(&self) -> (f32, f32);                // ((center+right)/total, right/(center+right))
    pub fn scale_px(&self, total_px: usize) -> (usize, usize, usize); // cols * total_px / total_cols
    pub fn ghostty_open_deltas(&self, total_px: usize) -> (isize, isize); // vs 50/50 splits
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebalanceOutcome { Applied, Reopened, Unsupported { backend: &'static str, reason: &'static str } }
```

**Algorithm** (`pct(t,p) = (t*p + 50) / 100`; must be reproduced exactly):
1. `lp = left_pct.clamp(12, 45)`; `collapse_right = !show_right || total < 100`; `collapse_left = total < 76`.
2. `right = if collapse_right {0} else {max(pct(total,16), 28)}`; `left = if collapse_left {0} else {max(pct(total,lp), 22)}`.
3. If `total - left - right < 48`: `deficit = 48 - (total-left-right)`; cut `right` toward 28 first,
   then `left` toward 22 (never below floors; never cut a collapsed column).
4. `center = total - left - right`. Invariant: `left + center + right == total` (always).
Separator cells are charged to `center` (realized center = `center_cols - separators`).
`ghostty_open_deltas`: `(total_px/2 - left_px, (total_px-left_px)/2 - right_px)`; positive = shrink.

**Golden table** (`lp=30`, `show_right=true` unless noted) — `layout.rs` tests MUST assert these:

`(total → left/center/right)`: 70→0/70/0 · 80→24/56/0 · 100→24/48/28 (deficit cut from left) · 120→36/56/28 ·
160→48/84/28 · 200→60/108/32 · 240→72/130/38 · 320→96/173/51 · 120,`show_right=false`→36/84/0 · 100,`lp=45`→24/48/28.
`ghostty_open_deltas` golden: `compute(240,30,true).ghostty_open_deltas(2400) == (480, 460)`.
The iTerm Python daemon keeps its own 56/80/38 floors in v0.10 (documented divergence; v1.0 passes cols).

**Rebalance CLI contract:** `ccide-core workspace rebalance [yid]`. Target: `yid` arg → `$CC_YAZI_ID`
→ `session::resolve_yazi_id_from_ppid` → every spec left after `prune_dead_specs`. Stdout line 1 is a
summary (`rebalanced 2/3 tabs`), then one line per tab: `applied <backend> <yid>` |
`reopened <backend> <yid>` | `unsupported <backend> <yid>: <reason>`. **Exit codes:** `0` if at least one
tab is Applied/Reopened; `3` if every tab is Unsupported or there are no specs (`unsupported none -: no
cc-ide workspace found`); `1` for internal errors. `register.ts` decides by exit code ONLY (no regex):
`0` → toast line 1; `3` → toast the first `unsupported` reason; other → "rebalance failed".
v0.10 per-backend: tmux Applied (`resize-pane -x`, using the center pane's own `#{window_width}` via
`-t`); wezterm Applied (`cli list --format json` + `adjust-pane-size`); ghostty-macOS Applied;
iterm Applied (daemon/py fallback succeeded); wt `Unsupported{"wt","wt.exe has no pane resize CLI"}`
(Reopen = v1.0); osc `Unsupported{"osc","no ack channel over SSH OSC 1337"}`.

## 3. Backend module signatures (dispatch in `mod.rs` / `rebalance.rs` via `match spec.backend`)
```rust
// tmux.rs / wezterm.rs / wt.rs / ghostty.rs — open
pub fn run_tmux_open(req: &OpenRequest) -> Option<PaneSpec>;   pub fn run_wezterm_open(req: &OpenRequest) -> Option<PaneSpec>;
pub fn run_wt_open(req: &OpenRequest) -> Option<PaneSpec>;     // close_wt_panes(yazi_id) unchanged
pub fn run_ghostty_open(req: &OpenRequest, cap: GhosttyCap) -> Option<PaneSpec>;
// close / focus
pub fn close_tmux(spec: &PaneSpec);  pub fn close_wezterm_panes(home: &str, spec: &PaneSpec);  pub fn close_ghostty_panes(spec: &PaneSpec);
pub fn focus_tmux_center(spec: &PaneSpec);  pub fn activate_wezterm_center(home: &str, spec: &PaneSpec);
pub fn activate_ghostty_center(spec: &PaneSpec);  // wt/osc/iterm focus: unchanged
// rebalance
pub fn rebalance_tmux(spec: &PaneSpec) -> RebalanceOutcome;  pub fn rebalance_wezterm(home: &str, spec: &PaneSpec) -> RebalanceOutcome;
pub fn rebalance_wt(spec: &PaneSpec) -> RebalanceOutcome /* Unsupported in v0.10 */;  pub fn rebalance_iterm(home: &str) -> RebalanceOutcome;
pub fn rebalance_ghostty(home: &str, spec: &PaneSpec) -> RebalanceOutcome;
// rebalance.rs (NEW)
pub fn run_rebalance(home: &str, yid_arg: Option<&str>) -> i32;        // prints contract, returns exit code
```

## 4. Ghostty (`ghostty.rs`, `ghostty_script.rs`, `ghostty_linux.rs` NEW)
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhosttyCap { None, MacOsAppleScript, LinuxCli, LinuxDbus }
impl GhosttyCap { pub fn as_str(self) -> &'static str; } // "none","macos-applescript","linux-cli","linux-dbus"
pub struct GhosttyProbeEnv { pub osascript_bin: String, pub ghostty_bin: String, pub gdbus_bin: String, pub linux_opt_in: bool,
                             pub in_ssh: bool, pub in_tmux: bool, pub is_ghostty: bool }  // + fn from_env() -> Self
pub fn probe_ghostty(home: &str) -> GhosttyCap;                        // = probe_ghostty_with(home, &from_env())
pub fn probe_ghostty_with(home: &str, env: &GhosttyProbeEnv) -> GhosttyCap;
pub fn probe_ghostty_native_capable(home: &str) -> bool;               // compat: != None (probe-ghostty exit code)
// ghostty_linux.rs
pub fn probe_linux(env: &GhosttyProbeEnv) -> GhosttyCap;
pub fn run_bounded(bin: &str, args: &[&str], timeout_ms: u64) -> Option<String>; // try_wait poll; kill+wait on timeout
pub fn run_ghostty_linux_open(req: &OpenRequest, cap: GhosttyCap) -> Option<PaneSpec>; // v0.10: always None
```
- `None` if `in_ssh || in_tmux || !is_ghostty`. macOS (or mocked `CCIDE_OSASCRIPT_BIN`) → the existing
  version ≥ 1.3 check → `MacOsAppleScript`.
- Linux: `None` unless `CCIDE_GHOSTTY_LINUX == "1"`. Then, with a 150 ms bound each:
  (a) `$CCIDE_GHOSTTY_BIN` (default `ghostty`) `+list-actions`; stdout containing `new_split` AND
  `resize_split` AND `goto_split` → `LinuxCli`;
  (b) otherwise `$CCIDE_GDBUS_BIN` (default `gdbus`) `introspect --session --dest com.mitchellh.ghostty
  --object-path /com/mitchellh/ghostty` succeeding AND output containing `org.gtk.Actions` AND
  `new-split` → `LinuxDbus`;
  (c) otherwise `None`.
- v0.10 ships the probe, mocks and doctor reporting only. `run_ghostty_linux_open` returns `None` and
  prints one stderr note, so `open` falls through to tmux. A real transport lands only once upstream
  can invoke split actions on a target surface from outside.
- Cap cache `ipc_path("ghostty_cap.cache")` = `"<cap>|<stamp>"`, 1 h TTL. `stamp` = Info.plist mtime
  on macOS, otherwise the mtime of the resolved `ghostty` binary. The `"1"` sentinel stamp is removed (F3).
- **F1 caller targeting (macOS):** before running the AppleScript, write `\x1b]2;ccide-<yid>\x07` to
  `/dev/tty` and sleep 30 ms. The script resolves `t_center` from `$GHOSTTY_SURFACE_ID` (if set), else
  `first terminal whose name is "ccide-<yid>"`. The `focused terminal of selected tab of front window`
  form is used ONLY when `CCIDE_GHOSTTY_FRONT_FALLBACK=1`. If no terminal matches, return `None` (falls
  through to tmux). Verify the `name` property in Ghostty.sdef 1.3; add a mocked-osascript test.
- **F2 live width:** `rebalance_ghostty` computes `total_px` as the sum of the live `geom_<yid>_yazi` and
  `geom_<yid>_sidebar` xpx, plus the caller's TIOCGWINSZ xpx when the caller tab == `spec.yid`,
  falling back to `ghostty_w_<yid>.txt`. It uses `spec.left_pct` (not a hard-coded 30) and
  `spec.show_right` (not the side-count heuristic).
- **Dead specs:** `rebalance_ghostty` and `reload_companion_panes` call `prune_dead_specs` first.

## 5. CLI, bootstrap & companion contracts

- **`src/proto.rs`** (NEW leaf): `pub const IPC_PROTO: u32 = 1; pub const PATCH_PROTO: u32 = 1;
  pub const PATCH_RULES_VERSION: &str = "2"; pub const SPEC_VERSION: u8 = 2; pub fn run(args: &[String]);`
- **`ccide-core proto [--json]`** (exit 0). Plain: `ccide-core <ver> ipc_proto=1 patch_proto=1 patch_rules=2 spec=2`.
  JSON: `{"ccide_version":"<ver>","ipc_proto":1,"patch_proto":1,"patch_rules_version":"2","spec_version":2,"ipc_dir":"<path|null>"}`.
- **`ccide-core --version` | `-V` | `version`** prints exactly `ccide-core <CARGO_PKG_VERSION>` (new in main.rs).
- **`ccide-core workspace detect [--json]`** (exit 0) prints ONE token, chosen with exactly `open.rs`'s `auto`
  precedence: `iterm`|`osc`|`wezterm`|`wt`|`ghostty`|`tmux`|`none` (the same names `workspace open` accepts).
  `tmux` = already inside `$TMUX`; `none` = no native backend and not in tmux.
  JSON: `{"backend":"…","ghostty_cap":"…","in_tmux":b,"in_ssh":b,"reasons":["…"]}`.
  Launchers: native ⇔ the token is not `tmux`/`none`. Bootstrap `tmux -L ccide` on `none` or `--tmux`.
  If detect fails or prints nothing (pre-0.10 binary), treat it as `none`. Delete all inline env sniffing
  from `scripts/ccide` and `scripts/ccide.ps1`.
- **`ccide-core doctor [--json]`** stays additive under `schema_version` 1. `DoctorReport` gains
  `pub ipc_dir: String, pub ghostty_cap: &'static str, pub legacy_patch: bool`. New JSON keys `"ipc_dir"`,
  `"ipc_proto": 1`, `"patch_proto": 1`, `"ghostty_cap"`, `"legacy_patch"`. The test keeps its 3 assertions and
  adds the new keys to its literal. New check item `IPC Patch Proto`: warn when `legacy_patch` (a live claude
  pid has `cur_pid_<pid>.txt` only in `legacy_ipc_dir()`) with the hint "run `ccide` or `./install.sh --patch-only`".
- **`scripts/ensure-ccide-core.{mjs,sh}`**
  - Tag = `$CCIDE_RELEASE_TAG`, else `v<major>.<minor>.0` from `${pluginRoot}/.claude-plugin/plugin.json`
    `"version"` (patches never publish binaries, AGENT §7.2). URL `…/releases/download/<tag>/<archive>`;
    `releases/latest` is removed. `User-Agent: ccide-bootstrap/<major>.<minor>`.
  - Skew: run `<bin> --version` once per binary stat, cached in `~/.claude/bin/.ccide-core.version` as
    `<size>:<mtime_ms>:<ver>`. Empty or unparsable output (pre-0.10), or binary `major.minor` < plugin
    → re-download atomically (existing lock + staging). Binary newer than plugin (dev build) → keep.
  - Offline / download failure → run the existing binary (fail-open), else exit 0.
- **Lazygit theme reload, all OS** (`reload.rs` + `pane_runner.rs`): `reload_companion_panes` atomically
  writes `ipc_path("reload_lg_<yid>")` for every live spec yid. On each `supervise()` poll, a lazygit-role
  runner checks it: if the sentinel's mtime > child spawn time, kill and respawn now (no crash backoff).
  Either way, remove the sentinel. The Unix `ps`+SIGTERM path stays for `pane-runner.sh` (tmux) panes;
  the mtime rule prevents a double restart.
- **Pane geometry** (`pane_runner.rs`): on start and on size change (checked ≤ 1 s), atomically write
  `ipc_path("geom_<yid>_<role>.txt")` = `"<cols> <rows> <xpx> <ypx>\n"`, role ∈ `yazi|lazygit|mon|sidebar`.
  Windows uses `GetConsoleScreenBufferInfo` (xpx=ypx=0).
- **`legacy_ipc_dir()`:** RETAINED read-only in v0.10.0 (proof for `claude_signalable`, plus `req_pid`/`cur_pid`
  for legacy-patched sessions). This amends #35's "remove in v0.10", because marketplace-only installs
  have no automatic repatch path. Doctor warns (above). Removal is scheduled for v0.11.0.

## 6. Wave 4 file ownership (exclusive; touching another worker's file = ask on the board)

| Worker | Files |
|---|---|
| `@rust-core` | `workspace/{spec,layout}.rs` (+ golden tests), `src/proto.rs`, `src/main.rs`, `src/commands/pane_runner.rs`, `workspace/reload.rs` |
| `@backends` | `workspace/{mod,open,detect,rebalance,iterm,actions}.rs`, `setup/{doctor,doctor_probes,doctor_render}.rs` |
| `@session-mod` | `workspace/{tmux,wezterm,wt}.rs`, `src/session/*` (switch `read_tab_pane_title` to `read_pane_spec`) |
| `@tests-qa` | `workspace/{ghostty,ghostty_script,ghostty_linux}.rs`, `tests/test_palette_syntax_and_ghostty.py` |
| `@plugin-scripts` | `scripts/{ccide,ccide.ps1,ensure-ccide-core.mjs,ensure-ccide-core.sh}`, `hooks/register.ts`, `tests/test_windows_support.py` |
| `@git-ci` | `AGENT.md` (§2/§4/§5/§8), `CHANGELOG.md` `[0.10.0]`, version bump (`Cargo.toml`, `.claude-plugin/*.json`, `install.sh`, `install.ps1`), `.github/workflows/*`, `tests/test_agent_infra.py` |

Order: `@rust-core` lands `spec.rs` + `layout.rs` first; everyone else codes against this document immediately.
