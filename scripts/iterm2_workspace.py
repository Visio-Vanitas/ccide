#!/usr/bin/env python3
"""
iTerm2 Python API 3-Column Tiling Workspace Controller (Hyprland / r/unixporn style):
- Left Column   (~30% width, ~80 cols): Top = Yazi File Tree, Bottom = Lazygit Diff & M4 Pro Status
- Center Column (~54% width, ~145 cols): Claude Code Main Workspace
- Right Column  (~16% width, ~43 cols): 3-Tier AI Sidebar (Top = Logo/Avatar, Mid = SUBAGENTS, Bot = BG AGENTS)
Uses pure iTerm2 Python API `Tab.async_update_layout()` over refreshed `tab.root` tree for deterministic,
consistent column widths across all tabs/windows, locks TUI side-pane scrollback to 0 lines, and drives
Claude Code's native `viewing-agent` state machine (`YU` / `EL`) without ever splitting or burying panes.
"""
import asyncio
import json
import os
import socket
import subprocess
import sys

try:
    import iterm2  # type: ignore
except ModuleNotFoundError:
    for cand in (
        os.path.expanduser("~/miniconda3/bin/python3"),
        os.path.expanduser("~/anaconda3/bin/python3"),
        os.path.expanduser("~/.pyenv/shims/python3"),
    ):
        if os.path.exists(cand) and os.path.realpath(cand) != os.path.realpath(sys.executable):
            os.execv(cand, [cand] + sys.argv)
    raise

DAEMON_SOCK = "/tmp/.cc-ide/iterm_daemon.sock"


def try_daemon_fastpath(action: str) -> bool:
    if not os.path.exists(DAEMON_SOCK):
        return False
    req: dict = {"op": action, "caller_guid": get_caller_guid()}
    if action == "open":
        req["target_dir"] = sys.argv[2] if len(sys.argv) > 2 else "."
        req["yazi_id"] = sys.argv[3] if len(sys.argv) > 3 else "1000"
        req["show_git"] = (sys.argv[4] == "1") if len(sys.argv) > 4 else True
        req["show_mon"] = (sys.argv[5] == "1") if len(sys.argv) > 5 else False
        req["left_pct"] = int(sys.argv[6]) if len(sys.argv) > 6 else 30
        req["git_root"] = sys.argv[7] if len(sys.argv) > 7 and sys.argv[7] else req["target_dir"]
        req["show_right"] = (sys.argv[8] == "1") if len(sys.argv) > 8 else True
    elif action == "close":
        req["pane_ids"] = sys.argv[2] if len(sys.argv) > 2 else ""
    elif action == "focus":
        req["query"] = sys.argv[2] if len(sys.argv) > 2 else ""
    elif action == "switch-subagent":
        req["yazi_id"] = sys.argv[2] if len(sys.argv) > 2 else ""
        req["session_id"] = sys.argv[3] if len(sys.argv) > 3 else ""
        req["view_id"] = sys.argv[4] if len(sys.argv) > 4 else "main"
        req["footer_rank"] = int(sys.argv[5]) if len(sys.argv) > 5 else 0

    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(1.5)
            s.connect(DAEMON_SOCK)
            s.sendall((json.dumps(req) + "\n").encode("utf-8"))
            resp = s.recv(4096).decode("utf-8", errors="ignore").strip()
            if resp and not resp.startswith("ERR:"):
                if action == "open":
                    print(resp)
                return True
    except Exception:
        pass
    return False


def ensure_daemon_spawned():
    daemon_py = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ccide_daemon.py")
    if os.path.exists(daemon_py):
        try:
            subprocess.Popen(
                [sys.executable, daemon_py],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
        except Exception:
            pass


def get_caller_guid() -> str:
    raw = os.environ.get("ITERM_SESSION_ID", "")
    if ":" in raw:
        return raw.split(":", 1)[1]
    return raw


async def find_target_session(app: iterm2.App, caller_guid: str):
    if caller_guid:
        sess = app.get_session_by_id(caller_guid)
        if sess is not None:
            for w in app.terminal_windows:
                for t in w.tabs:
                    if sess in t.sessions:
                        return w, t, sess
    w = app.current_terminal_window
    if w is None and app.terminal_windows:
        w = app.terminal_windows[0]
    if w is None:
        return None, None, None
    t = w.current_tab
    s = t.current_session if t else None
    return w, t, s


def make_command_profile(cmd: str, cwd: str) -> iterm2.LocalWriteOnlyProfile:
    """
    Creates a write-only profile for TUI side panes (Yazi, Lazygit, Right Sidebar)
    with terminal scrollback completely disabled (`scrollback_lines = 0`,
    `scrollback_in_alternate_screen = False`) and mouse wheel reporting enabled
    so the TUI application handles 100% of scrolling without ever showing past buffer remnants.
    """
    prof = iterm2.LocalWriteOnlyProfile()
    prof.set_use_custom_command("Yes")
    prof.set_command(cmd)
    prof.set_unlimited_scrollback(False)
    prof.set_scrollback_lines(0)
    prof.set_scrollback_in_alternate_screen(False)
    prof.set_scrollback_with_status_bar(False)
    prof.set_disable_smcup_rmcup(False)
    prof.set_mouse_reporting(True)
    prof.set_mouse_reporting_allow_mouse_wheel(True)
    if cwd and os.path.isdir(cwd):
        prof.set_initial_directory_mode(
            iterm2.InitialWorkingDirectory.INITIAL_WORKING_DIRECTORY_CUSTOM
        )
        prof.set_custom_directory(cwd)
    return prof


async def lock_tui_pane_no_scrollback(sess: iterm2.Session):
    """
    Enforces zero terminal scrollback and full TUI mouse-wheel takeover on a live session.
    """
    try:
        prof = await sess.async_get_profile()
        await prof.async_set_unlimited_scrollback(False)
        await prof.async_set_scrollback_lines(0)
        await prof.async_set_scrollback_in_alternate_screen(False)
        await prof.async_set_scrollback_with_status_bar(False)
        await prof.async_set_disable_smcup_rmcup(False)
        await prof.async_set_mouse_reporting(True)
        await prof.async_set_mouse_reporting_allow_mouse_wheel(True)
    except Exception:
        pass


def collect_leaf_sessions(node) -> list[iterm2.Session]:
    if isinstance(node, iterm2.Session):
        return [node]
    out = []
    if hasattr(node, "children"):
        for ch in node.children:
            out.extend(collect_leaf_sessions(ch))
    return out


async def apply_consistent_tab_widths(
    app: iterm2.App,
    tab: iterm2.Tab,
    left_pct: int = 30,
    right_pct: int = 16,
):
    """
    Deterministically computes the true total grid width & height of `tab` from `tab.root`,
    then assigns exact preferred_size (`30% | 54% | 16%`) to every Session node inside `tab.root`
    and calls `await tab.async_update_layout()` so all windows/tabs get identical column widths
    regardless of whether the left column has 1 pane (Yazi only) or 2 panes (Yazi + Lazygit).
    """
    await app.async_refresh()
    root = tab.root
    if not hasattr(root, "children") or len(root.children) < 2:
        return

    col_widths = []
    col_heights = []
    for col_node in root.children:
        leaves = collect_leaf_sessions(col_node)
        if not leaves:
            continue
        col_widths.append(max(s.grid_size.width for s in leaves))
        col_heights.append(sum(s.grid_size.height for s in leaves))

    if not col_widths:
        return

    total_w = sum(col_widths)
    total_h = max(col_heights) if col_heights else 65

    has_right = len(root.children) >= 3
    left_w = max(56, int(round(total_w * left_pct / 100.0)))
    right_w = max(38, int(round(total_w * right_pct / 100.0))) if has_right else 0
    mid_w = max(80, total_w - left_w - right_w)

    # Column 0: Left (Yazi, optional Lazygit, optional Macmon)
    left_leaves = collect_leaf_sessions(root.children[0])
    n_left = max(1, len(left_leaves))
    for i, s in enumerate(left_leaves):
        h_share = (total_h // n_left) if i < n_left - 1 else (total_h - (total_h // n_left) * (n_left - 1))
        s.preferred_size = iterm2.util.Size(left_w, max(10, h_share))

    # Column 1: Center (Claude Code Main Workspace)
    mid_leaves = collect_leaf_sessions(root.children[1])
    for s in mid_leaves:
        s.preferred_size = iterm2.util.Size(mid_w, total_h)

    # Column 2: Right (3-Tier AI Sidebar)
    if has_right:
        right_leaves = collect_leaf_sessions(root.children[2])
        for s in right_leaves:
            s.preferred_size = iterm2.util.Size(right_w, total_h)

    try:
        await tab.async_update_layout()
    except Exception:
        pass


async def cmd_open(connection):
    target_dir = sys.argv[2]
    yazi_id = sys.argv[3]
    show_git = sys.argv[4] == "1"
    show_mon = sys.argv[5] == "1"
    left_pct = int(sys.argv[6]) if len(sys.argv) > 6 else 30
    git_root = sys.argv[7] if len(sys.argv) > 7 and sys.argv[7] else target_dir
    show_right = (sys.argv[8] == "1") if len(sys.argv) > 8 else True
    right_pct = 16 if show_right else 0

    app = await iterm2.async_get_app(connection)
    caller_guid = get_caller_guid()
    _, tab, claude_sess = await find_target_session(app, caller_guid)
    if claude_sess is None or tab is None:
        sys.exit(1)

    existing_side_ids_str = await claude_sess.async_get_variable("user.cc_side_panes")
    existing_side_ids = set((existing_side_ids_str or "").split(",")) if existing_side_ids_str else set()
    closed_any = False
    for s in list(tab.sessions):
        if s.session_id == claude_sess.session_id:
            continue
        name = s.name or ""
        if (
            s.session_id in existing_side_ids
            or "[TREE] yazi" in name
            or "[GIT] lazygit" in name
            or "[SYS] macmon" in name
            or "[AI] Companion" in name
            or "[PET] clawd" in name
        ):
            try:
                await s.async_close()
                closed_any = True
            except Exception:
                pass
    if closed_any:
        await asyncio.sleep(0.03)

    shell_pid = await claude_sess.async_get_variable("pid") or 0
    cache_dir = os.path.expanduser("~/.cache/cc-ide")
    os.makedirs(cache_dir, exist_ok=True)
    if yazi_id and shell_pid:
        try:
            with open(os.path.join(cache_dir, f"tab_shell_{yazi_id}.txt"), "w", encoding="utf-8") as f:
                f.write(str(shell_pid))
        except Exception:
            pass

    runner = os.path.expanduser("~/.claude/scripts/pane-runner.sh")
    yazi_cmd = f"'{runner}' yazi '{target_dir}' {yazi_id}"
    git_cmd = f"'{runner}' lazygit '{git_root}'"
    mon_cmd = f"'{runner}' macmon '{target_dir}'"
    sidebar_cmd = f"'{runner}' sidebar '{target_dir}' {yazi_id} {shell_pid}"

    created_ids = []

    yazi_prof = make_command_profile(yazi_cmd, target_dir)
    yazi_sess = await claude_sess.async_split_pane(
        vertical=True,
        before=True,
        profile_customizations=yazi_prof,
    )
    if yazi_sess:
        created_ids.append(yazi_sess.session_id)
        left_bottom = yazi_sess

        if show_git:
            git_prof = make_command_profile(git_cmd, git_root)
            git_sess = await left_bottom.async_split_pane(
                vertical=False,
                before=False,
                profile_customizations=git_prof,
            )
            if git_sess:
                created_ids.append(git_sess.session_id)
                left_bottom = git_sess

        if show_mon:
            mon_prof = make_command_profile(mon_cmd, target_dir)
            mon_sess = await left_bottom.async_split_pane(
                vertical=False,
                before=False,
                profile_customizations=mon_prof,
            )
            if mon_sess:
                created_ids.append(mon_sess.session_id)

    right_sess = None
    if show_right:
        sidebar_prof = make_command_profile(sidebar_cmd, target_dir)
        right_sess = await claude_sess.async_split_pane(
            vertical=True,
            before=False,
            profile_customizations=sidebar_prof,
        )
        if right_sess:
            created_ids.append(right_sess.session_id)

    await apply_consistent_tab_widths(app, tab, left_pct=left_pct, right_pct=right_pct)

    await claude_sess.async_set_variable("user.cc_yazi_id", str(yazi_id))
    await claude_sess.async_set_variable("user.cc_side_panes", ",".join(created_ids))
    await claude_sess.async_activate()
    print(",".join(created_ids))


async def cmd_rebalance(connection):
    app = await iterm2.async_get_app(connection)
    await app.async_refresh()
    cache_dir = os.path.expanduser("~/.cache/cc-ide")
    os.makedirs(cache_dir, exist_ok=True)
    for w in app.terminal_windows:
        for t in w.tabs:
            for s in t.sessions:
                yid = await s.async_get_variable("user.cc_yazi_id")
                spid = await s.async_get_variable("pid")
                if yid and spid:
                    try:
                        with open(os.path.join(cache_dir, f"tab_shell_{yid}.txt"), "w", encoding="utf-8") as f:
                            f.write(str(spid))
                    except Exception:
                        pass
                name = s.name or ""
                if any(tag in name for tag in ("[TREE] yazi", "[GIT] lazygit", "[AI] Companion")):
                    await lock_tui_pane_no_scrollback(s)
            if hasattr(t.root, "children") and len(t.root.children) >= 3:
                await apply_consistent_tab_widths(app, t, left_pct=30, right_pct=16)


async def cmd_close(connection):
    pane_ids_arg = sys.argv[2] if len(sys.argv) > 2 else ""
    app = await iterm2.async_get_app(connection)

    if pane_ids_arg:
        target_ids = {x.strip() for x in pane_ids_arg.split(",") if x.strip()}
        for pid in target_ids:
            s = app.get_session_by_id(pid)
            if s is not None:
                try:
                    await s.async_close()
                except Exception:
                    pass
        return

    caller_guid = get_caller_guid()
    _, tab, claude_sess = await find_target_session(app, caller_guid)
    if tab is None:
        return

    side_ids_str = await claude_sess.async_get_variable("user.cc_side_panes") if claude_sess else ""
    side_ids = set((side_ids_str or "").split(",")) if side_ids_str else set()

    for s in list(tab.sessions):
        if claude_sess and s.session_id == claude_sess.session_id:
            continue
        name = s.name or ""
        if (
            s.session_id in side_ids
            or "[TREE] yazi" in name
            or "[GIT] lazygit" in name
            or "[SYS] macmon" in name
            or "[AI] Companion" in name
            or "[PET] clawd" in name
        ):
            try:
                await s.async_close()
            except Exception:
                pass


async def cmd_focus(connection):
    query = sys.argv[2] if len(sys.argv) > 2 else ""
    if not query:
        return
    app = await iterm2.async_get_app(connection)
    q_low = query.lower()

    for w in app.terminal_windows:
        for t in w.tabs:
            for s in t.sessions:
                sname = (s.name or "").lower()
                if q_low in sname or s.session_id == query:
                    await w.async_activate()
                    await t.async_select()
                    await s.async_activate()
                    return


async def cmd_switch_subagent(connection):
    """
    Pure native Claude Code `viewing-agent` switching:
    - Never creates or buries iTerm2 panes.
    - Focuses the center `claude_sess` pane.
    - If `/tmp/.cc-ide/req_<session_id>.json` was consumed by `jn()` inside `claude.exe` (≤350ms),
      Claude Code has already switched natively via `YU(id, Qt)` / `EL(Qt)`.
    - If `/tmp/.cc-ide/req_<session_id>.json` is still present after 350ms (meaning the running
      `claude` process was launched before the binary patch), falls back to driving Claude Code's
      native footer selector (`Esc` / arrow keys).
    """
    tab_yazi_id = sys.argv[2] if len(sys.argv) > 2 else ""
    session_id = sys.argv[3] if len(sys.argv) > 3 else ""
    view_id = sys.argv[4] if len(sys.argv) > 4 else "main"
    footer_rank = int(sys.argv[5]) if len(sys.argv) > 5 else 0

    app = await iterm2.async_get_app(connection)
    await app.async_refresh()

    target_claude: iterm2.Session | None = None
    for w in app.terminal_windows:
        for t in w.tabs:
            for s in t.sessions:
                yid = await s.async_get_variable("user.cc_yazi_id")
                if tab_yazi_id and str(yid) == str(tab_yazi_id):
                    target_claude = s
                    break
            if target_claude:
                break

    if target_claude is None:
        return

    await target_claude.async_activate()

    req_file = f"/tmp/.cc-ide/req_{session_id}.json" if session_id else ""
    if req_file:
        for _ in range(7):
            await asyncio.sleep(0.05)
            if not os.path.exists(req_file):
                # Consumed natively by `jn()` inside `claude.exe`!
                return
        try:
            os.remove(req_file)
        except OSError:
            pass

    # Fallback for a pre-patch running process: native footer navigation
    if view_id == "main" or footer_rank <= 0:
        await target_claude.async_send_text("\x1b")
    else:
        await target_claude.async_send_text("\x1b")
        await asyncio.sleep(0.04)
        await target_claude.async_send_text("\x1b[B")
        await asyncio.sleep(0.04)
        await target_claude.async_send_text("\x1b[A" * 10)
        await asyncio.sleep(0.04)
        await target_claude.async_send_text("\x1b[B" * footer_rank)
        await asyncio.sleep(0.04)
        await target_claude.async_send_text("\r")


if __name__ == "__main__":
    action = sys.argv[1] if len(sys.argv) > 1 else "open"
    if try_daemon_fastpath(action):
        sys.exit(0)
    ensure_daemon_spawned()
    if action == "open":
        iterm2.run_until_complete(cmd_open)
    elif action == "close":
        iterm2.run_until_complete(cmd_close)
    elif action == "focus":
        iterm2.run_until_complete(cmd_focus)
    elif action == "rebalance":
        iterm2.run_until_complete(cmd_rebalance)
    elif action == "switch-subagent":
        iterm2.run_until_complete(cmd_switch_subagent)
