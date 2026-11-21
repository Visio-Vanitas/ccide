#!/usr/bin/env python3
"""
ccide_daemon.py — Persistent iTerm2 AutoLaunch Daemon & OSC 1337 SSH Bridge for `cc-ide`.

Runs two zero-polling event services inside a single warm iTerm2 asyncio connection:
1. Local Unix Domain Socket Server (`/tmp/.cc-ide/iterm_daemon.sock`):
   - Executes `open`, `close`, `rebalance`, `focus`, and `switch-subagent` in `< 5ms`
     using the warm `iterm2.App` state (eliminating the ~350ms Python + WebSocket cold start).
2. Global `OSC 1337;Custom=id=ccide:<json>` Escape Sequence Listener:
   - Captures `\033]1337;Custom=id=ccide:<json>\007` emitted by remote SSH sessions (or local `--osc`).
   - Automatically inspects the Center pane's local `ssh` child process, reuses/establishes
     an SSH `ControlMaster` socket (`~/.ssh/ccide-cm-%C`), and splits the **local iTerm2 tab natively**
     into `30% | 54% | 16%` Metal panes running the remote `yazi`, `lazygit`, and `ccide-core sidebar`.
"""
import asyncio
import json
import os
import shlex
import shutil
import signal
import socket
import subprocess
import sys

try:
    import iterm2  # type: ignore
    from iterm2.notifications import async_subscribe_to_custom_escape_sequence_notification  # type: ignore
except ModuleNotFoundError:
    for cand in (
        os.path.expanduser("~/.local/share/cc-ide/pyenv/bin/python3"),
        os.path.expanduser("~/miniconda3/bin/python3"),
        os.path.expanduser("~/anaconda3/bin/python3"),
        os.path.expanduser("~/.pyenv/shims/python3"),
    ):
        if os.path.exists(cand) and os.path.realpath(cand) != os.path.realpath(sys.executable):
            os.execv(cand, [cand] + sys.argv)
    raise

IPC_DIR = os.environ.get("CCIDE_IPC_DIR") or f"/tmp/.cc-ide-{os.getuid()}"
SOCK_PATH = os.path.join(IPC_DIR, "iterm_daemon.sock")
PID_PATH = os.path.join(IPC_DIR, "iterm_daemon.pid")
YAZI_SESSION_CACHE: dict[str, str] = {}
LAST_TAB_TITLES: dict[str, str] = {}


def write_tab_shell_pid(cache_dir: str, yid: str, shell_pid) -> None:
    if not yid or not shell_pid:
        return
    sp_str = str(shell_pid)
    path = os.path.join(cache_dir, f"tab_shell_{yid}.txt")
    try:
        if os.path.isfile(path):
            with open(path, "r", encoding="utf-8") as f:
                if f.read().strip().startswith(f"{sp_str}:"):
                    return
        with open(path, "w", encoding="utf-8") as f:
            f.write(sp_str)
    except Exception:
        pass


def write_center_title_and_wake(yid: str, raw_name: str) -> None:
    if not yid:
        return
    val = (raw_name or "").strip()
    if LAST_TAB_TITLES.get(yid) == val:
        return
    LAST_TAB_TITLES[yid] = val
    title_path = os.path.join(IPC_DIR, f"tab_title_{yid}.txt")
    try:
        os.makedirs(IPC_DIR, mode=0o700, exist_ok=True)
        tmp_path = f"{title_path}.tmp.{os.getpid()}"
        with open(tmp_path, "w", encoding="utf-8") as f:
            f.write(val + "\n")
        os.replace(tmp_path, title_path)
    except OSError:
        pass
    sb_pf = os.path.join(IPC_DIR, f"sb_yazi_{yid}.pid")
    if not os.path.isfile(sb_pf):
        return
    try:
        with open(sb_pf, encoding="utf-8") as pf:
            raw = pf.read().strip()
            sb_pid = int(raw.split(":")[0] or "0")
    except (OSError, ValueError):
        return
    if sb_pid <= 1:
        return
    try:
        os.kill(sb_pid, signal.SIGUSR2)
    except ProcessLookupError:
        # Sidebar already exited: drop the stale PID file so later frames stay cheap.
        try:
            os.unlink(sb_pf)
        except OSError:
            pass
    except ValueError:
        try:
            os.unlink(sb_pf)
        except OSError:
            pass
    except PermissionError:
        pass


def is_existing_daemon_alive() -> bool:
    if not os.path.exists(SOCK_PATH):
        return False
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(0.15)
            s.connect(SOCK_PATH)
            s.sendall(b'{"op":"ping"}\n')
            data = s.recv(64)
            return data.strip() == b"pong"
    except Exception:
        return False


def normalize_guid(raw: str) -> str:
    if ":" in raw:
        return raw.split(":", 1)[1]
    return raw


async def find_target_session(app: iterm2.App, caller_guid: str):
    guid = normalize_guid(caller_guid)
    if guid:
        sess = app.get_session_by_id(guid)
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


def make_command_profile(cmd: str, cwd: str = "") -> iterm2.LocalWriteOnlyProfile:
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
    if hasattr(prof, "_simple_set"):
        prof._simple_set("Mouse Reporting allow clicks and drags", True)
    if cwd and os.path.isdir(cwd):
        prof.set_initial_directory_mode(
            iterm2.InitialWorkingDirectory.INITIAL_WORKING_DIRECTORY_CUSTOM
        )
        prof.set_custom_directory(cwd)
    return prof


async def lock_tui_pane_no_scrollback(sess: iterm2.Session, center_bg_img: str | None = None):
    try:
        prof = await sess.async_get_profile()
        await prof.async_set_unlimited_scrollback(False)
        await prof.async_set_scrollback_lines(0)
        await prof.async_set_scrollback_in_alternate_screen(False)
        await prof.async_set_scrollback_with_status_bar(False)
        await prof.async_set_disable_smcup_rmcup(False)
        await prof.async_set_mouse_reporting(True)
        await prof.async_set_mouse_reporting_allow_mouse_wheel(True)
        if hasattr(prof, "_async_simple_set"):
            await prof._async_simple_set("Mouse Reporting allow clicks and drags", True)
        if center_bg_img is not None and hasattr(prof, "async_set_background_image_location"):
            if (prof.background_image_location or "") != center_bg_img:
                await prof.async_set_background_image_location(center_bg_img)
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


LEFT_MIN = 22
CENTER_MIN = 48
RIGHT_MIN = 28
RIGHT_PCT = 16
LEFT_PCT_MIN = 12
LEFT_PCT_MAX = 45


def compute_iterm_column_widths(total_w: int, left_pct: int = 30, has_right: bool = True) -> tuple[int, int, int]:
    """Compute conserved `(left_w, mid_w, right_w)` column widths matching Rust `LayoutPlan`."""
    if total_w <= 0:
        return (0, 0, 0)
    lp = max(LEFT_PCT_MIN, min(LEFT_PCT_MAX, int(left_pct)))
    min_budget = LEFT_MIN + CENTER_MIN + (RIGHT_MIN if has_right else 0)
    if total_w < min_budget:
        left_w = max(1, (total_w * lp) // 100)
        right_w = max(1, (total_w * RIGHT_PCT) // 100) if has_right else 0
        while left_w + right_w >= total_w and (left_w > 0 or right_w > 0):
            if right_w > left_w and right_w > 0:
                right_w -= 1
            elif left_w > 0:
                left_w -= 1
            elif right_w > 0:
                right_w -= 1
        return (left_w, max(1, total_w - left_w - right_w), right_w)

    right_w = max(RIGHT_MIN, (total_w * RIGHT_PCT + 50) // 100) if has_right else 0
    left_w = max(LEFT_MIN, (total_w * lp + 50) // 100)
    if left_w + right_w + CENTER_MIN > total_w:
        deficit = left_w + right_w + CENTER_MIN - total_w
        if has_right and right_w > RIGHT_MIN:
            cut = min(deficit, right_w - RIGHT_MIN)
            right_w -= cut
            deficit -= cut
        if deficit > 0 and left_w > LEFT_MIN:
            cut = min(deficit, left_w - LEFT_MIN)
            left_w -= cut
    mid_w = max(1, total_w - left_w - right_w)
    return (left_w, mid_w, right_w)


async def apply_consistent_tab_widths(
    app: iterm2.App,
    tab: iterm2.Tab,
    left_pct: int = 30,
    right_pct: int = 16,
    min_total_w: int = 0,
):
    _ = right_pct
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

    total_w = max(sum(col_widths), int(min_total_w or 0))
    total_h = max(col_heights) if col_heights else 65

    has_right = len(root.children) >= 3
    left_w, mid_w, right_w = compute_iterm_column_widths(total_w, left_pct, has_right)

    left_leaves = collect_leaf_sessions(root.children[0])
    n_left = max(1, len(left_leaves))
    for i, s in enumerate(left_leaves):
        h_share = (total_h // n_left) if i < n_left - 1 else (total_h - (total_h // n_left) * (n_left - 1))
        s.preferred_size = iterm2.util.Size(left_w, max(10, h_share))

    mid_leaves = collect_leaf_sessions(root.children[1])
    for s in mid_leaves:
        s.preferred_size = iterm2.util.Size(mid_w, total_h)

    if has_right:
        right_leaves = collect_leaf_sessions(root.children[2])
        for s in right_leaves:
            s.preferred_size = iterm2.util.Size(right_w, total_h)

    try:
        await tab.async_update_layout()
    except Exception:
        pass


def detect_ssh_transport_for_pid(root_pid: int) -> list[str] | None:
    """
    Walks the local process tree under `root_pid` (the Center iTerm2 pane's shell PID)
    to find an active `ssh` client process and extract its destination & connection flags.
    Returns `None` if `root_pid` is running locally without `ssh`.
    """
    if not root_pid or root_pid <= 1:
        return None
    try:
        out = subprocess.check_output(
            ["ps", "-ax", "-o", "pid=,ppid=,comm=,args="],
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except Exception:
        return None

    children: dict[int, list[tuple[int, str, str]]] = {}
    for line in out.splitlines():
        parts = line.strip().split(None, 3)
        if len(parts) < 4:
            continue
        try:
            pid = int(parts[0])
            ppid = int(parts[1])
        except ValueError:
            continue
        comm = os.path.basename(parts[2])
        args_str = parts[3]
        children.setdefault(ppid, []).append((pid, comm, args_str))

    queue = [int(root_pid)]
    visited = set()
    while queue:
        cur = queue.pop(0)
        if cur in visited:
            continue
        visited.add(cur)
        for cpid, comm, args_str in children.get(cur, []):
            if comm == "ssh":
                try:
                    tokens = shlex.split(args_str)
                except ValueError:
                    continue
                # Extract SSH connection options (-p, -i, -J, -F, -l, -o) and destination host
                ssh_opts: list[str] = []
                target_host: str | None = None
                i = 1
                opt_with_arg = {"-p", "-i", "-J", "-F", "-l", "-o", "-b", "-c", "-D", "-E", "-I", "-L", "-m", "-O", "-Q", "-R", "-S", "-W", "-w"}
                while i < len(tokens):
                    tok = tokens[i]
                    if tok in opt_with_arg and i + 1 < len(tokens):
                        if tok in {"-p", "-i", "-J", "-F", "-l", "-o", "-S"}:
                            ssh_opts.extend([tok, tokens[i + 1]])
                        i += 2
                    elif tok.startswith("-"):
                        i += 1
                    else:
                        target_host = tok
                        break
                if target_host:
                    return ssh_opts + [target_host]
            queue.append(cpid)
    return None


async def close_existing_side_panes(tab: iterm2.Tab, claude_sess: iterm2.Session):
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
            or "[SYS] monitor" in name
            or "[AI] Companion" in name
            or "[PET] clawd" in name
        ):
            try:
                await s.async_close()
                closed_any = True
            except Exception:
                pass
    if closed_any:
        await asyncio.sleep(0.02)


async def handle_open(
    app: iterm2.App,
    caller_guid: str,
    target_dir: str,
    yazi_id: str,
    show_git: bool,
    show_mon: bool,
    left_pct: int,
    git_root: str,
    show_right: bool,
    remote_ssh_spec: dict | None = None,
) -> str:
    await app.async_refresh()
    _, tab, claude_sess = await find_target_session(app, caller_guid)
    if claude_sess is None or tab is None:
        return ""

    await close_existing_side_panes(tab, claude_sess)
    pre_total_w = getattr(getattr(claude_sess, "grid_size", None), "width", 0) or 0

    local_shell_pid = await claude_sess.async_get_variable("pid") or 0
    cache_dir = os.path.expanduser("~/.cache/cc-ide")
    os.makedirs(cache_dir, exist_ok=True)
    os.makedirs(IPC_DIR, mode=0o700, exist_ok=True)

    # Check if this is an SSH session requesting client-side native split panes
    ssh_transport = None
    if remote_ssh_spec is not None:
        ssh_transport = detect_ssh_transport_for_pid(int(local_shell_pid))
        if ssh_transport is None and remote_ssh_spec.get("ssh_conn"):
            # Fallback to user@server_ip:port from SSH_CONNECTION ("client_ip client_port server_ip server_port")
            conn_parts = str(remote_ssh_spec.get("ssh_conn", "")).split()
            r_user = remote_ssh_spec.get("user") or os.environ.get("USER", "")
            if len(conn_parts) >= 4:
                srv_ip, srv_port = conn_parts[2], conn_parts[3]
                host_spec = f"{r_user}@{srv_ip}" if r_user else srv_ip
                ssh_transport = ["-p", srv_port, host_spec]

    if ssh_transport is not None:
        r_shell_pid = remote_ssh_spec.get("shell_pid", 0) if remote_ssh_spec else 0
        q_dir = shlex.quote(target_dir)
        q_git = shlex.quote(git_root or target_dir)
        q_yid = shlex.quote(str(yazi_id))
        ssh_prefix = (
            "ssh -o ControlMaster=auto -o ControlPath=~/.ssh/ccide-cm-%C -o ControlPersist=10m "
            + " ".join(shlex.quote(x) for x in ssh_transport)
            + " -t "
        )
        yazi_cmd = ssh_prefix + "sh -lc " + shlex.quote(
            f'exec "$HOME/.claude/scripts/pane-runner.sh" yazi {q_dir} {q_yid}'
        )
        git_cmd = ssh_prefix + "sh -lc " + shlex.quote(
            f'exec "$HOME/.claude/scripts/pane-runner.sh" lazygit {q_git}'
        )
        mon_cmd = ssh_prefix + "sh -lc " + shlex.quote(
            f'exec "$HOME/.claude/scripts/pane-runner.sh" macmon {q_dir}'
        )
        sidebar_cmd = ssh_prefix + "sh -lc " + shlex.quote(
            f'exec "$HOME/.claude/scripts/pane-runner.sh" sidebar {q_dir} {q_yid} {int(r_shell_pid)}'
        )
        local_cwd = os.path.expanduser("~")
    else:
        shell_pid = (
            remote_ssh_spec.get("shell_pid", local_shell_pid)
            if remote_ssh_spec
            else local_shell_pid
        ) or local_shell_pid
        if yazi_id and shell_pid:
            write_tab_shell_pid(cache_dir, str(yazi_id), shell_pid)
        if remote_ssh_spec is not None:
            # Local `--osc` test mode: touch the ACK file immediately
            try:
                with open(f"{IPC_DIR}/osc_ack_{yazi_id}", "w", encoding="utf-8") as f:
                    f.write("ok")
            except Exception:
                pass
        runner = shlex.quote(os.path.expanduser("~/.claude/scripts/pane-runner.sh"))
        q_dir = shlex.quote(target_dir)
        q_git = shlex.quote(git_root or target_dir)
        q_yid = shlex.quote(str(yazi_id))
        yazi_cmd = f"{runner} yazi {q_dir} {q_yid}"
        git_cmd = f"{runner} lazygit {q_git}"
        mon_cmd = f"{runner} macmon {q_dir}"
        sidebar_cmd = f"{runner} sidebar {q_dir} {q_yid} {int(shell_pid)}"
        local_cwd = target_dir

    created_ids = []
    right_pct = 16 if show_right else 0

    yazi_prof = make_command_profile(yazi_cmd, local_cwd)
    yazi_sess = await claude_sess.async_split_pane(
        vertical=True,
        before=True,
        profile_customizations=yazi_prof,
    )
    if yazi_sess:
        created_ids.append(yazi_sess.session_id)
        left_bottom = yazi_sess

        if show_git:
            git_prof = make_command_profile(git_cmd, git_root if not ssh_transport else local_cwd)
            git_sess = await left_bottom.async_split_pane(
                vertical=False,
                before=False,
                profile_customizations=git_prof,
            )
            if git_sess:
                created_ids.append(git_sess.session_id)
                left_bottom = git_sess

        if show_mon:
            mon_prof = make_command_profile(mon_cmd, local_cwd)
            mon_sess = await left_bottom.async_split_pane(
                vertical=False,
                before=False,
                profile_customizations=mon_prof,
            )
            if mon_sess:
                created_ids.append(mon_sess.session_id)

    if show_right:
        sidebar_prof = make_command_profile(sidebar_cmd, local_cwd)
        right_sess = await claude_sess.async_split_pane(
            vertical=True,
            before=False,
            profile_customizations=sidebar_prof,
        )
        if right_sess:
            created_ids.append(right_sess.session_id)

    await apply_consistent_tab_widths(app, tab, left_pct=left_pct, right_pct=right_pct, min_total_w=pre_total_w)
    await claude_sess.async_set_variable("user.cc_yazi_id", str(yazi_id))
    await claude_sess.async_set_variable("user.cc_left_pct", str(int(left_pct)))
    await claude_sess.async_set_variable("user.cc_side_panes", ",".join(created_ids))
    if yazi_id:
        YAZI_SESSION_CACHE[str(yazi_id)] = claude_sess.session_id
        # Re-read the session so the title reflects a rename that happened since `async_refresh`.
        fresh = app.get_session_by_id(claude_sess.session_id) or claude_sess
        write_center_title_and_wake(str(yazi_id), fresh.name or "")
    await claude_sess.async_activate()
    return ",".join(created_ids)


def discard_yazi_state(yid: str) -> None:
    """Drops every per-tab cache entry and the published title file for `yid`."""
    if not yid:
        return
    yid = str(yid)
    YAZI_SESSION_CACHE.pop(yid, None)
    LAST_TAB_TITLES.pop(yid, None)
    try:
        os.unlink(os.path.join(IPC_DIR, f"tab_title_{yid}.txt"))
    except OSError:
        pass


async def handle_close(app: iterm2.App, pane_ids_arg: str = "", caller_guid: str = "", yid: str = ""):
    await app.async_refresh()
    if not yid and caller_guid:
        # Resolve the owning tab from the Center session so caches are always released.
        _, _, caller_sess = await find_target_session(app, caller_guid)
        if caller_sess is not None:
            yid = str(await caller_sess.async_get_variable("user.cc_yazi_id") or "")
    discard_yazi_state(yid)
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

    if yid:
        for w in app.terminal_windows:
            for t in w.tabs:
                for s in t.sessions:
                    s_yid = await s.async_get_variable("user.cc_yazi_id")
                    if str(s_yid) == str(yid):
                        await close_existing_side_panes(t, s)
                        return

    _, tab, claude_sess = await find_target_session(app, caller_guid)
    if tab is not None and claude_sess is not None:
        await close_existing_side_panes(tab, claude_sess)


async def handle_rebalance(app: iterm2.App):
    await app.async_refresh()
    cache_dir = os.path.expanduser("~/.cache/cc-ide")
    os.makedirs(cache_dir, exist_ok=True)
    for w in app.terminal_windows:
        for t in w.tabs:
            center_bg_img = None
            tab_left_pct = 30
            has_ccide_tab = False
            for s in t.sessions:
                yid = await s.async_get_variable("user.cc_yazi_id")
                spid = await s.async_get_variable("pid")
                if yid:
                    has_ccide_tab = True
                    try:
                        raw_lp = await s.async_get_variable("user.cc_left_pct")
                        if raw_lp is not None and str(raw_lp).strip():
                            tab_left_pct = int(str(raw_lp).strip())
                    except (TypeError, ValueError):
                        pass
                    try:
                        cprof = await s.async_get_profile()
                        center_bg_img = cprof.background_image_location or ""
                    except Exception:
                        pass
                if yid and spid:
                    YAZI_SESSION_CACHE[str(yid)] = s.session_id
                    write_center_title_and_wake(str(yid), s.name or "")
                    write_tab_shell_pid(cache_dir, str(yid), spid)
            for s in t.sessions:
                name = s.name or ""
                if any(tag in name for tag in ("[TREE] yazi", "[GIT] lazygit", "[AI] Companion")):
                    await lock_tui_pane_no_scrollback(s, center_bg_img)
            if hasattr(t.root, "children") and (len(t.root.children) >= 3 or (has_ccide_tab and len(t.root.children) >= 2)):
                await apply_consistent_tab_widths(app, t, left_pct=tab_left_pct, right_pct=16)


async def handle_focus_or_switch(
    app: iterm2.App,
    yazi_id: str,
    session_id: str = "",
):
    _ = session_id
    target_claude: iterm2.Session | None = None
    if yazi_id:
        cached_sid = YAZI_SESSION_CACHE.get(str(yazi_id), "")
        if cached_sid:
            cached_sess = app.get_session_by_id(cached_sid)
            if cached_sess is not None:
                try:
                    await cached_sess.async_activate()
                    return
                except Exception:
                    pass
            YAZI_SESSION_CACHE.pop(str(yazi_id), None)

    if yazi_id:
        await app.async_refresh()
        for w in app.terminal_windows:
            for t in w.tabs:
                for s in t.sessions:
                    yid = await s.async_get_variable("user.cc_yazi_id")
                    if str(yid) == str(yazi_id):
                        YAZI_SESSION_CACHE[str(yazi_id)] = s.session_id
                        target_claude = s
                        break
                if target_claude:
                    break
            if target_claude:
                break

    if target_claude is not None:
        await target_claude.async_activate()


async def main(connection):
    app = await iterm2.async_get_app(connection)
    os.makedirs(IPC_DIR, mode=0o700, exist_ok=True)

    # 1. Subscribe to `OSC 1337;Custom=id=ccide:<json>` across ALL sessions (local & SSH)
    async def on_custom_escape(_conn, notification):
        _ = _conn
        if getattr(notification, "sender_identity", "") != "ccide":
            return
        raw_payload = getattr(notification, "payload", "")
        sender_session_id = getattr(notification, "session", "")
        try:
            req = json.loads(raw_payload)
        except Exception:
            return

        op = req.get("op", "")
        if op in ("open", "open_ssh"):
            tdir = req.get("dir") or req.get("target_dir") or "."
            groot = req.get("git_root") or tdir
            yid = str(req.get("yid") or req.get("yazi_id") or "1000")
            show_git = bool(req.get("git", 1))
            show_mon = bool(req.get("mon", 0))
            left_pct = int(req.get("left_pct", 30))
            show_right = bool(req.get("right", 1))
            await handle_open(
                app,
                sender_session_id,
                tdir,
                yid,
                show_git,
                show_mon,
                left_pct,
                groot,
                show_right,
                remote_ssh_spec=req if op == "open_ssh" else None,
            )
        elif op in ("focus", "switch-subagent"):
            yid = str(req.get("yid") or req.get("yazi_id") or "")
            sid = str(req.get("sid") or req.get("session_id") or "")
            await handle_focus_or_switch(app, yid, sid)
        elif op == "close":
            yid = str(req.get("yid") or req.get("yazi_id") or "")
            await handle_close(app, caller_guid=sender_session_id, yid=yid)
        elif op == "rebalance":
            await handle_rebalance(app)

    await async_subscribe_to_custom_escape_sequence_notification(
        connection, on_custom_escape, None
    )

    # 1b. Real-Time Unfocused Selection & Read-Only Focus Guard:
    #     - `VariableMonitor(SESSION, "selection")` fires immediately even when focus is in the Center
    #       Claude Code pane and the user drags directly across `[AI] Companion` (Sprite Logo / Telemetry)
    #       or `[SYS] macmon`, clearing the selection in `< 0.5ms` via `async_set_selection(Selection([]))`.
    #     - `FocusMonitor` bounces focus back to Center Claude Code if `[SYS] macmon` is clicked.
    watched_selection_sessions: set[str] = set()
    # `sess_id -> (selection_task, name_task)` so a closing session can cancel both.
    watched_tasks: dict[str, tuple[asyncio.Task, asyncio.Task]] = {}

    async def watch_session_selection(sess_id: str):
        if not sess_id or sess_id in watched_selection_sessions:
            return
        watched_selection_sessions.add(sess_id)
        try:
            async with iterm2.VariableMonitor(
                connection, iterm2.VariableScopes.SESSION, "selection", sess_id
            ) as mon:
                while True:
                    val = await mon.async_get()
                    if not val:
                        continue
                    sess = app.get_session_by_id(sess_id)
                    if sess is None:
                        break
                    name = sess.name or ""
                    if any(tag in name for tag in ("[AI] Companion", "[SYS] macmon", "[SYS] monitor", "[PET] clawd")):
                        try:
                            await sess.async_set_selection(iterm2.Selection([]))
                        except Exception:
                            pass
        except asyncio.CancelledError:
            raise
        except Exception:
            pass
        finally:
            watched_selection_sessions.discard(sess_id)

    watched_name_sessions: set[str] = set()

    async def watch_session_name(sess_id: str):
        if not sess_id or sess_id in watched_name_sessions:
            return
        watched_name_sessions.add(sess_id)
        try:
            sess0 = app.get_session_by_id(sess_id)
            if sess0 is not None:
                yid0 = await sess0.async_get_variable("user.cc_yazi_id")
                if yid0:
                    write_center_title_and_wake(str(yid0), sess0.name or "")
            async with iterm2.VariableMonitor(
                connection, iterm2.VariableScopes.SESSION, "name", sess_id
            ) as mon:
                while True:
                    new_name = await mon.async_get()
                    sess = app.get_session_by_id(sess_id)
                    if sess is None:
                        break
                    yid = await sess.async_get_variable("user.cc_yazi_id")
                    if yid:
                        write_center_title_and_wake(str(yid), str(new_name or sess.name or ""))
        except asyncio.CancelledError:
            raise
        except Exception:
            pass
        finally:
            watched_name_sessions.discard(sess_id)

    def spawn_session_watchers(sess_id: str) -> None:
        if not sess_id or sess_id in watched_tasks:
            return
        watched_tasks[sess_id] = (
            asyncio.create_task(watch_session_selection(sess_id)),
            asyncio.create_task(watch_session_name(sess_id)),
        )

    def stop_session_watchers(sess_id: str) -> None:
        for task in watched_tasks.pop(sess_id, ()):
            task.cancel()
        watched_selection_sessions.discard(sess_id)
        watched_name_sessions.discard(sess_id)

    for w in app.terminal_windows:
        for t in w.tabs:
            for s in t.sessions:
                spawn_session_watchers(s.session_id)

    async def monitor_new_sessions():
        while True:
            try:
                async with iterm2.NewSessionMonitor(connection) as mon:
                    while True:
                        new_sid = await mon.async_get()
                        try:
                            if new_sid:
                                spawn_session_watchers(new_sid)
                        except Exception:
                            pass
            except Exception:
                await asyncio.sleep(0.5)

    asyncio.create_task(monitor_new_sessions())

    async def monitor_session_termination():
        # Releases per-session watchers the moment iTerm2 closes a pane, so closed tabs
        # never leave orphaned `VariableMonitor` tasks or stale cache entries behind.
        while True:
            try:
                async with iterm2.SessionTerminationMonitor(connection) as mon:
                    while True:
                        term_sid = await mon.async_get()
                        try:
                            stop_session_watchers(term_sid)
                        except Exception:
                            pass
            except Exception:
                await asyncio.sleep(0.5)

    asyncio.create_task(monitor_session_termination())

    ya_bin = (
        shutil.which("ya")
        or ("/opt/homebrew/bin/ya" if os.path.exists("/opt/homebrew/bin/ya") else "")
        or ("/usr/local/bin/ya" if os.path.exists("/usr/local/bin/ya") else "")
    )

    async def guard_readonly_pane_focus():
        yazi_focused_yids: set[str] = set()
        while True:
            try:
                async with iterm2.FocusMonitor(connection) as monitor:
                    while True:
                        update = await monitor.async_get_next_update()
                        try:
                            changed = update.active_session_changed
                            if not changed or not changed.session_id:
                                continue
                            sess = app.get_session_by_id(changed.session_id)
                            if sess is None:
                                continue
                            name = sess.name or ""
                            found_tab = False
                            for w in app.terminal_windows:
                                for t in w.tabs:
                                    if sess in t.sessions:
                                        found_tab = True
                                        tab_yid = ""
                                        target_center = None
                                        for cand in t.sessions:
                                            yid = await cand.async_get_variable("user.cc_yazi_id")
                                            if yid:
                                                tab_yid = str(yid)
                                                target_center = cand
                                                YAZI_SESSION_CACHE[tab_yid] = cand.session_id
                                                break
                                        if ("[SYS] macmon" in name or "[SYS] monitor" in name or "[PET] clawd" in name) and target_center:
                                            await target_center.async_activate()
                                        if tab_yid and ya_bin:
                                            if "[TREE] yazi" in name:
                                                yazi_focused_yids.add(tab_yid)
                                            elif tab_yid in yazi_focused_yids:
                                                yazi_focused_yids.discard(tab_yid)
                                                try:
                                                    proc = await asyncio.create_subprocess_exec(
                                                        ya_bin,
                                                        "emit-to",
                                                        tab_yid,
                                                        "plugin",
                                                        "hscroll",
                                                        "blur",
                                                        stdin=asyncio.subprocess.DEVNULL,
                                                        stdout=asyncio.subprocess.DEVNULL,
                                                        stderr=asyncio.subprocess.DEVNULL,
                                                    )
                                                    asyncio.create_task(proc.wait())
                                                except Exception:
                                                    pass
                                        break
                                if found_tab:
                                    break
                        except Exception:
                            pass
            except Exception:
                await asyncio.sleep(0.5)

    asyncio.create_task(guard_readonly_pane_focus())

    # 2. Start Local Unix Domain Socket Server (`/tmp/.cc-ide/iterm_daemon.sock`)
    if os.path.exists(SOCK_PATH):
        try:
            os.unlink(SOCK_PATH)
        except OSError:
            pass

    async def handle_unix_client(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        try:
            line = await asyncio.wait_for(reader.readline(), timeout=2.0)
            if not line:
                writer.close()
                return
            req = json.loads(line.decode("utf-8", errors="ignore").strip())
            op = req.get("op", "")
            resp = "ok"

            if op == "ping":
                resp = "pong"
            elif op == "open":
                resp = await handle_open(
                    app,
                    str(req.get("caller_guid", "")),
                    str(req.get("target_dir", ".")),
                    str(req.get("yazi_id", "1000")),
                    bool(req.get("show_git", True)),
                    bool(req.get("show_mon", False)),
                    int(req.get("left_pct", 30)),
                    str(req.get("git_root", "")),
                    bool(req.get("show_right", True)),
                )
            elif op == "close":
                await handle_close(
                    app,
                    pane_ids_arg=str(req.get("pane_ids", "")),
                    caller_guid=str(req.get("caller_guid", "")),
                    yid=str(req.get("yazi_id", "")),
                )
            elif op == "rebalance":
                await handle_rebalance(app)
            elif op == "switch-subagent":
                await handle_focus_or_switch(
                    app,
                    str(req.get("yazi_id", "")),
                    str(req.get("session_id", "")),
                )
            elif op == "focus":
                q = str(req.get("query", "")).lower()
                if q:
                    found = False
                    for w in app.terminal_windows:
                        if found:
                            break
                        for t in w.tabs:
                            if found:
                                break
                            for s in t.sessions:
                                if q in (s.name or "").lower() or s.session_id.lower() == q:
                                    await w.async_activate()
                                    await t.async_select()
                                    await s.async_activate()
                                    found = True
                                    break

            writer.write((resp + "\n").encode("utf-8"))
            await writer.drain()
        except Exception as e:
            try:
                writer.write(f"ERR:{e}\n".encode("utf-8"))
                await writer.drain()
            except Exception:
                pass
        finally:
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass

    server = await asyncio.start_unix_server(handle_unix_client, path=SOCK_PATH)
    os.chmod(SOCK_PATH, 0o600)

    with open(PID_PATH, "w", encoding="utf-8") as f:
        f.write(str(os.getpid()))

    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    os.makedirs(IPC_DIR, mode=0o700, exist_ok=True)
    if is_existing_daemon_alive():
        sys.exit(0)
    try:
        iterm2.run_forever(main)
    except KeyboardInterrupt:
        pass
