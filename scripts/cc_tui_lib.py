#!/usr/bin/env python3
"""
cc_tui_lib.py — Standalone Reusable Multi-Cell Panel TUI Rendering Library
for the iTerm2 + Claude Code Tiling Workspace (`ccide`).

Provides unified, cross-window consistent primitives for:
1. `TIOCGWINSZ` pixel & character-cell geometry (`get_terminal_geometry`)
2. CJK / East-Asian display width measurement & truncation (`display_width`, `trunc_to_width`)
3. Closed 4-sided rounded box & multi-panel exact-height grid layout (`BoxPanel`, `render_multi_panel_grid`)
4. Resolution-adaptive sprite / avatar / 2x2 Quadrant Sub-Pixel vector logo rendering
5. Subagent trajectory parsing (extracting real project file operations identically to main chat)
"""
import ctypes
import dataclasses
import fcntl
import glob
import json
import os
import re
import struct
import subprocess
import sys
import termios
import time
import unicodedata
from typing import Optional

CACHE_DIR = os.path.expanduser("~/.cache/cc-ide")
LOGO_DIR = os.path.expanduser("~/.cache/cc-ide/logos")
QUADRANT_LOGOS_JSON = os.path.expanduser("~/.cache/cc-ide/logos/quadrant_logos.json")
CUSTOM_AVATAR_DIR = os.path.expanduser("~/.claude/avatars")

RESET = "\033[0m\033[49m"
BOLD = "\033[1m"
DIM = "\033[2m"
HILITE_BG = "\033[48;5;236m"
ACTIVE_VIEW_BG = "\033[48;5;24m"
GRAY = "\033[38;5;242m"
WHITE = "\033[38;5;254m"
CYAN = "\033[38;5;117m"
BLUE = "\033[38;5;75m"
GREEN = "\033[38;5;114m"
AMBER = "\033[38;5;221m"
CORAL = "\033[38;2;217;119;87m"
LAVENDER = "\033[38;5;183m"
MINT = "\033[38;2;16;163;127m"

ANSI_RE = re.compile(r"\x1b\[[0-9;]*[a-zA-Z]")
QUADRANTS = [
    " ", "▘", "▝", "▀",
    "▖", "▌", "▞", "▛",
    "▗", "▚", "▐", "▜",
    "▄", "▙", "▟", "█",
]

LOGO_PNG_MAP = {
    "claude": "bg_claude-color.svg.png",
    "deepseek": "bg_deepseek-color.svg.png",
    "gemini": "bg_gemini-color.svg.png",
    "grok": "bg_grok.svg.png",
    "openai": "bg_openai.svg.png",
}

_QUADRANT_DYNAMIC_CACHE: dict[tuple[str, int, int], list[str]] = {}
_QUADRANT_JSON_CACHE: Optional[dict] = None


@dataclasses.dataclass
class TerminalGeometry:
    cols: int
    rows: int
    xpixel: int
    ypixel: int
    cell_w_px: float
    cell_h_px: float

    @property
    def inner_cols(self) -> int:
        return max(10, self.cols - 2)


@dataclasses.dataclass
class BoxRow:
    text: str
    bg: str = ""
    tag: Optional[tuple[str, int]] = None  # e.g. ("sub", 0) or ("bg", 2) or ("file", idx)


@dataclasses.dataclass
class PanelSpec:
    title: str
    badge: str = ""
    accent: str = CYAN
    rows: list[BoxRow] = dataclasses.field(default_factory=list)
    fixed_height: Optional[int] = None
    min_height: int = 6
    weight: float = 1.0
    raw_interior: bool = False  # True when OSC 1337 inline image occupies interior rows


def get_terminal_geometry(default_cols: int = 43, default_rows: int = 65) -> TerminalGeometry:
    """
    Queries `/dev/tty` or `stdout` via `TIOCGWINSZ` ioctl for exact character grid
    and pixel dimensions, ensuring cross-window layout and aspect-ratio consistency.
    """
    for fd_cand in (sys.stdout.fileno(), sys.stdin.fileno()):
        try:
            r, c, xp, yp = struct.unpack(
                "HHHH", fcntl.ioctl(fd_cand, termios.TIOCGWINSZ, b"\x00" * 8)
            )
            if c > 0 and r > 0:
                cw = (xp / float(c)) if xp > 0 else 14.0
                ch = (yp / float(r)) if yp > 0 else 32.0
                return TerminalGeometry(c, r, xp, yp, cw, ch)
        except Exception:
            pass
    try:
        with open("/dev/tty", "rb") as tty_f:
            r, c, xp, yp = struct.unpack(
                "HHHH", fcntl.ioctl(tty_f.fileno(), termios.TIOCGWINSZ, b"\x00" * 8)
            )
            if c > 0 and r > 0:
                cw = (xp / float(c)) if xp > 0 else 14.0
                ch = (yp / float(r)) if yp > 0 else 32.0
                return TerminalGeometry(c, r, xp, yp, cw, ch)
    except Exception:
        pass
    try:
        sz = os.get_terminal_size()
        return TerminalGeometry(sz.columns, sz.lines, sz.columns * 14, sz.lines * 32, 14.0, 32.0)
    except OSError:
        return TerminalGeometry(default_cols, default_rows, default_cols * 14, default_rows * 32, 14.0, 32.0)


def char_display_width(ch: str) -> int:
    if unicodedata.combining(ch):
        return 0
    eaw = unicodedata.east_asian_width(ch)
    return 2 if eaw in ("W", "F") else 1


def display_width(s: str) -> int:
    clean = ANSI_RE.sub("", s)
    return sum(char_display_width(c) for c in clean)


def trunc_to_width(s: str, max_cols: int) -> str:
    """
    Truncates plain text (supporting CJK double-width characters) so its visual column width
    never exceeds `max_cols`.
    """
    if max_cols <= 0:
        return ""
    if display_width(s) <= max_cols:
        return s
    out = []
    cur = 0
    limit = max(1, max_cols - 1)
    for ch in s:
        cw = char_display_width(ch)
        if cur + cw > limit:
            break
        out.append(ch)
        cur += cw
    return "".join(out) + "…"


def render_box_header(title: str, right_badge: str, width: int, accent: str) -> str:
    t_w = display_width(title)
    b_w = display_width(right_badge)
    if right_badge:
        dashes = max(1, width - t_w - b_w - 8)
        return f"{GRAY}╭─ {BOLD}{accent}{title}{RESET}{GRAY} {'─' * dashes} {DIM}{right_badge}{RESET}{GRAY} ─╮{RESET}"
    else:
        dashes = max(1, width - t_w - 5)
        return f"{GRAY}╭─ {BOLD}{accent}{title}{RESET}{GRAY} {'─' * dashes}╮{RESET}"


def render_box_footer(width: int) -> str:
    return f"{GRAY}╰{'─' * max(1, width - 2)}╯{RESET}"


def format_boxed_row(inner_content: str, width: int, row_bg: str = "") -> str:
    inner_target = max(1, width - 2)
    vis = display_width(inner_content)
    pad = max(0, inner_target - vis)
    return f"{GRAY}│{RESET}{row_bg}{inner_content}{' ' * pad}{RESET}{GRAY}│{RESET}"


def allocate_panel_heights(total_rows: int, panels: list[PanelSpec]) -> list[int]:
    """
    Allocates exact integer row heights for a vertical stack of `PanelSpec`s
    so that `sum(heights) == total_rows` across every window and pane size.
    """
    n = len(panels)
    if n == 0:
        return []
    heights = [0] * n
    rem = total_rows
    flex_indices = []
    total_weight = 0.0

    for i, p in enumerate(panels):
        if p.fixed_height is not None:
            h = max(p.min_height, min(p.fixed_height, rem - (n - 1 - i) * 3))
            heights[i] = h
            rem -= h
        else:
            flex_indices.append(i)
            total_weight += max(0.1, p.weight)

    if not flex_indices:
        heights[-1] += rem
        return heights

    allocated_flex = 0
    for idx_pos, i in enumerate(flex_indices):
        p = panels[i]
        if idx_pos == len(flex_indices) - 1:
            h = max(2, rem - allocated_flex)
        else:
            share = int(round(rem * (max(0.1, p.weight) / total_weight)))
            h = max(p.min_height, share)
        heights[i] = h
        allocated_flex += h

    # Adjust final panel if rounding caused any 1-row discrepancy
    diff = total_rows - sum(heights)
    if diff != 0:
        heights[flex_indices[-1]] = max(3, heights[flex_indices[-1]] + diff)

    return heights


def render_multi_panel_grid(
    geom: TerminalGeometry,
    panels: list[PanelSpec],
) -> tuple[str, dict[int, tuple[str, int]]]:
    """
    Renders a list of `PanelSpec`s into a single flicker-free ANSI frame string
    spanning exact `geom.cols` x `geom.rows`, and returns `(frame_ansi, row_tag_map)`
    mapping 1-based terminal row numbers to `BoxRow.tag` for SGR mouse click handling.
    """
    w = max(20, geom.cols)
    total_rows = max(10, geom.rows)
    heights = allocate_panel_heights(total_rows, panels)

    buf: list[str] = []
    row_tag_map: dict[int, tuple[str, int]] = {}
    cur_screen_row = 1

    for p, h in zip(panels, heights):
        if cur_screen_row > total_rows:
            break
        # Header row
        hdr = render_box_header(p.title, p.badge, w, p.accent)
        buf.append(f"\033[{cur_screen_row};1H{hdr}")
        cur_screen_row += 1

        inner_h = max(0, h - 2)
        if p.raw_interior:
            # Draw only left and right borders so inline OSC 1337 image inside is preserved
            for _ in range(inner_h):
                if cur_screen_row <= total_rows:
                    buf.append(
                        f"\033[{cur_screen_row};1H{GRAY}│{RESET}\033[{cur_screen_row};{w}H{GRAY}│{RESET}"
                    )
                    cur_screen_row += 1
        else:
            for r_idx in range(inner_h):
                if cur_screen_row > total_rows:
                    break
                if r_idx < len(p.rows):
                    br = p.rows[r_idx]
                    line_s = format_boxed_row(br.text, w, br.bg)
                    if br.tag is not None:
                        row_tag_map[cur_screen_row] = br.tag
                else:
                    line_s = format_boxed_row("", w)
                buf.append(f"\033[{cur_screen_row};1H{line_s}")
                cur_screen_row += 1

        # Footer row
        if cur_screen_row <= total_rows:
            ftr = render_box_footer(w)
            buf.append(f"\033[{cur_screen_row};1H{ftr}")
            cur_screen_row += 1

    return "".join(buf), row_tag_map


def get_image_dimensions(path: str) -> tuple[int, int]:
    try:
        from PIL import Image  # type: ignore
        with Image.open(path) as im:
            return im.size
    except Exception:
        pass

    try:
        with open(path, "rb") as f:
            head = f.read(64)
            if len(head) >= 24 and head.startswith(b"\x89PNG\r\n\x1a\n"):
                w, h = struct.unpack(">II", head[16:24])
                return int(w), int(h)
            if head[:2] == b"\xff\xd8":
                f.seek(2)
                while True:
                    marker = f.read(2)
                    if len(marker) < 2:
                        break
                    while marker[1] == 0xFF:
                        marker = b"\xff" + f.read(1)
                    m = marker[1]
                    if m in (0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF):
                        f.read(3)
                        h, w = struct.unpack(">HH", f.read(4))
                        return int(w), int(h)
                    else:
                        seg_len = struct.unpack(">H", f.read(2))[0]
                        f.seek(seg_len - 2, 1)
    except Exception:
        pass
    return (512, 512)


def compute_adaptive_image_box(
    geom: TerminalGeometry,
    img_w: int,
    img_h: int,
    max_height_ratio: float = 0.48,
) -> tuple[int, int, int, int]:
    """
    Given terminal pixel geometry and an input image's `(img_w, img_h)`,
    computes `(render_cols, render_rows, col_start, panel_fixed_height)`
    so the sprite panel adapts to the image's exact aspect ratio.
    """
    inner_cols = geom.inner_cols
    max_sprite_rows = max(8, int(geom.rows * max_height_ratio))
    natural_rows = int(
        round((inner_cols * geom.cell_w_px) * (img_h / float(max(1, img_w))) / max(1.0, geom.cell_h_px))
    )
    render_rows = max(6, min(max_sprite_rows, natural_rows))
    render_cols = min(
        inner_cols,
        max(
            6,
            int(
                round(
                    (render_rows * geom.cell_h_px)
                    * (img_w / float(max(1, img_h)))
                    / max(1.0, geom.cell_w_px)
                )
            ),
        ),
    )
    render_rows = max(
        6,
        min(
            max_sprite_rows,
            int(
                round(
                    (render_cols * geom.cell_w_px)
                    * (img_h / float(max(1, img_w)))
                    / max(1.0, geom.cell_h_px)
                )
            ),
        ),
    )
    col_start = 2 + max(0, (inner_cols - render_cols) // 2)
    return render_cols, render_rows, col_start, render_rows + 2


def load_quadrant_logos() -> dict:
    global _QUADRANT_JSON_CACHE
    if _QUADRANT_JSON_CACHE is not None:
        return _QUADRANT_JSON_CACHE
    try:
        if os.path.exists(QUADRANT_LOGOS_JSON):
            with open(QUADRANT_LOGOS_JSON, "r", encoding="utf-8") as f:
                data = json.load(f)
                if isinstance(data, dict):
                    _QUADRANT_JSON_CACHE = data
                    return data
    except Exception:
        pass
    _QUADRANT_JSON_CACHE = {}
    return _QUADRANT_JSON_CACHE


def build_adaptive_logo_rows(
    family: str,
    accent: str,
    geom: TerminalGeometry,
    max_rows: int = 18,
) -> list[BoxRow]:
    inner_cols = geom.inner_cols
    target_w = max(16, min(inner_cols - 6, 32))
    key = (family, target_w, max_rows)
    if key in _QUADRANT_DYNAMIC_CACHE:
        art_lines = _QUADRANT_DYNAMIC_CACHE[key]
    else:
        art_lines = None
        png_file = os.path.join(LOGO_DIR, LOGO_PNG_MAP.get(family, "bg_claude-color.svg.png"))
        if os.path.exists(png_file):
            try:
                from PIL import Image  # type: ignore
                im = Image.open(png_file).convert("RGB")
                bbox = im.getbbox()
                if bbox:
                    im = im.crop(bbox)
                img_aspect = im.height / float(max(1, im.width))
                cell_aspect = geom.cell_w_px / float(max(1.0, geom.cell_h_px))
                ideal_h = int(round(target_w * cell_aspect * img_aspect))
                target_h = max(8, min(max_rows, ideal_h))
                adj_w = min(target_w, max(12, int(round(target_h / max(0.05, cell_aspect * img_aspect)))))
                sub_w, sub_h = adj_w * 2, target_h * 2
                canvas = im.resize((sub_w, sub_h), Image.Resampling.LANCZOS)

                lines_ansi = []
                for cy in range(target_h):
                    parts = []
                    last_col = None
                    for cx in range(adj_w):
                        pts = [
                            canvas.getpixel((cx * 2, cy * 2)),
                            canvas.getpixel((cx * 2 + 1, cy * 2)),
                            canvas.getpixel((cx * 2, cy * 2 + 1)),
                            canvas.getpixel((cx * 2 + 1, cy * 2 + 1)),
                        ]
                        mask = 0
                        r_s = g_s = b_s = cnt = 0
                        for bit, (r_p, g_p, b_p) in enumerate(pts):
                            if max(r_p, g_p, b_p) > 74:
                                mask |= 1 << bit
                                r_s += r_p
                                g_s += g_p
                                b_s += b_p
                                cnt += 1
                        if mask == 0:
                            parts.append(" ")
                        else:
                            ch = QUADRANTS[mask]
                            boost = 1.0 + 0.12 * (0.5 - (cy / float(max(1, target_h - 1))))
                            rc = max(0, min(255, int((r_s // cnt) * boost)))
                            gc = max(0, min(255, int((g_s // cnt) * boost)))
                            bc = max(0, min(255, int((b_s // cnt) * boost)))
                            col = (rc, gc, bc)
                            if col != last_col:
                                parts.append(f"\033[38;2;{rc};{gc};{bc}m{ch}")
                                last_col = col
                            else:
                                parts.append(ch)
                    parts.append("\033[0m")
                    lines_ansi.append("".join(parts))
                art_lines = lines_ansi
                _QUADRANT_DYNAMIC_CACHE[key] = art_lines
            except Exception:
                art_lines = None

        if not art_lines:
            logos = load_quadrant_logos()
            art_lines = logos.get(family) or [f"{accent}  {family.upper()}  {RESET}"]

    vis_w = display_width(art_lines[0]) if art_lines else target_w
    pad_left = max(0, (inner_cols - vis_w) // 2)
    prefix = " " * pad_left
    rows_out = [BoxRow("")]
    for line in art_lines:
        rows_out.append(BoxRow(f"{prefix}{line}{RESET}"))
    rows_out.append(BoxRow(""))
    return rows_out


def extract_subagent_file_ops(jsonl_path: str, target_cwd: str) -> tuple[str, str, int]:
    """
    Parses a subagent's JSONL trajectory identically to `yazi-reveal-hook.sh`
    for the main conversation: extracts real workspace `file_path` / `notebook_path` / `path`
    from `Read`, `Edit`, `Write`, `MultiEdit`, `NotebookEdit`, `Grep`, `Glob` tool calls.
    Never selects `.jsonl` trajectory files or `/tmp/claude-*` files!
    """
    if not jsonl_path or not os.path.exists(jsonl_path):
        return "", "", 0

    last_tool = ""
    last_file = ""
    file_op_seq = 0
    claude_projects_dir = os.path.expanduser("~/.claude/projects/")

    try:
        with open(jsonl_path, "r", encoding="utf-8", errors="ignore") as f:
            for raw in f:
                if '"tool_use"' not in raw:
                    continue
                try:
                    d = json.loads(raw)
                except Exception:
                    continue
                if d.get("type") != "assistant":
                    continue
                msg = d.get("message") or {}
                content = msg.get("content")
                if not isinstance(content, list):
                    continue
                for block in content:
                    if not isinstance(block, dict) or block.get("type") != "tool_use":
                        continue
                    tname = block.get("name") or ""
                    if tname:
                        last_tool = tname
                    if tname in ("Read", "Edit", "Write", "MultiEdit", "NotebookEdit", "Grep", "Glob"):
                        inp = block.get("input") or {}
                        cand = inp.get("file_path") or inp.get("notebook_path") or inp.get("path") or ""
                        if not cand or not isinstance(cand, str):
                            continue
                        if not os.path.isabs(cand):
                            cand = os.path.normpath(os.path.join(target_cwd, cand))
                        if (
                            cand.startswith(claude_projects_dir)
                            or "/tmp/claude-" in cand
                            or "/.cache/cc-ide/status" in cand
                            or "/.cache/cc-ide/live_" in cand
                        ):
                            continue
                        if not os.path.exists(cand) and cand.startswith("/tmp/") and target_cwd:
                            parts = cand.split("/", 4)
                            if len(parts) >= 5:
                                alt = os.path.join(target_cwd, parts[4])
                                if os.path.exists(alt):
                                    cand = alt
                        if os.path.exists(cand):
                            last_file = cand
                            last_tool = tname
                            file_op_seq += 1
    except Exception:
        pass

    return last_tool, last_file, file_op_seq


def get_main_session_file_ops(session_id: str, tab_yazi_id: str, target_cwd: str) -> tuple[str, str]:
    """
    Returns `(last_tool, last_file)` for the `main` conversation of this tab:
    checks `main_last_file_<yazi_id>.json` first, falling back to the tail of the main `.jsonl` transcript.
    """
    if tab_yazi_id:
        cpath = os.path.join(CACHE_DIR, f"main_last_file_{tab_yazi_id}.json")
        if os.path.exists(cpath):
            try:
                with open(cpath, "r", encoding="utf-8") as f:
                    d = json.load(f)
                fp = d.get("file") or ""
                tl = d.get("tool") or "active"
                if fp and os.path.exists(fp):
                    return tl, fp
            except Exception:
                pass

    if session_id:
        matches = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{session_id}.jsonl"))
        if matches and os.path.exists(matches[0]):
            try:
                jpath = matches[0]
                claude_projects_dir = os.path.expanduser("~/.claude/projects/")
                with open(jpath, "rb") as jf:
                    jf.seek(0, os.SEEK_END)
                    sz = jf.tell()
                    jf.seek(max(0, sz - 524288))
                    tail = jf.read().decode("utf-8", errors="ignore")
                last_tool = ""
                last_file = ""
                for raw in tail.splitlines():
                    if '"tool_use"' not in raw:
                        continue
                    try:
                        d = json.loads(raw)
                    except Exception:
                        continue
                    if d.get("type") != "assistant":
                        continue
                    for block in (d.get("message") or {}).get("content") or []:
                        if not isinstance(block, dict) or block.get("type") != "tool_use":
                            continue
                        tname = block.get("name") or ""
                        if tname and not last_file:
                            last_tool = tname
                        if tname in ("Read", "Edit", "Write", "MultiEdit", "NotebookEdit", "Grep", "Glob"):
                            inp = block.get("input") or {}
                            cand = inp.get("file_path") or inp.get("notebook_path") or inp.get("path") or ""
                            if not cand or not isinstance(cand, str):
                                continue
                            if not os.path.isabs(cand):
                                cand = os.path.normpath(os.path.join(target_cwd, cand))
                            if (
                                cand.startswith(claude_projects_dir)
                                or "/tmp/claude-" in cand
                                or "/.cache/cc-ide/status" in cand
                                or "/.cache/cc-ide/live_" in cand
                            ):
                                continue
                            if os.path.exists(cand):
                                last_file = cand
                                last_tool = tname
                return last_tool, last_file
            except Exception:
                pass

    return "", ""


def get_session_subagents(session_id: str, target_cwd: str) -> list[dict]:
    if not session_id:
        return []
    pattern = os.path.expanduser(f"~/.claude/projects/*/{session_id}/subagents/agent-*.meta.json")
    metas = sorted(glob.glob(pattern), key=os.path.getmtime, reverse=True)
    results = []
    now = time.time()

    for mpath in metas[:12]:
        try:
            with open(mpath, "r", encoding="utf-8") as f:
                meta = json.load(f)
            agent_id = os.path.basename(mpath).replace("agent-", "").replace(".meta.json", "")
            jpath = mpath.replace(".meta.json", ".jsonl")
            ctime = os.path.getctime(mpath)
            status = "done" if meta.get("finished") else "running"
            age_s = int(now - os.path.getmtime(mpath))

            if os.path.exists(jpath):
                mtime = os.path.getmtime(jpath)
                age_s = int(now - mtime)
                if status != "done":
                    with open(jpath, "rb") as jf:
                        jf.seek(0, os.SEEK_END)
                        sz = jf.tell()
                        jf.seek(max(0, sz - 4000))
                        tail = jf.read().decode("utf-8", errors="ignore")
                    lines = [l.strip() for l in tail.splitlines() if l.strip()]
                    if lines:
                        last = lines[-1]
                        if '"stop_reason":"end_turn"' in last or (age_s > 25 and '"type":"assistant"' in last and '"tool_use"' not in last):
                            status = "done"

            last_tool, last_file, file_op_seq = extract_subagent_file_ops(jpath, target_cwd)
            raw_type = meta.get("agentType", "general-purpose")

            results.append({
                "id": agent_id[:7],
                "full_id": agent_id,
                "raw_type": raw_type,
                "type": raw_type.split(":")[-1][:12],
                "desc": meta.get("description", "Subagent task"),
                "status": status,
                "tool": last_tool,
                "file": last_file,
                "file_op_seq": file_op_seq,
                "jsonl": jpath,
                "meta_path": mpath,
                "ctime": ctime,
                "age_s": age_s,
            })
        except Exception:
            pass

    # Rank all subagents in chronological creation order (matching Claude Code's Um(tasks) order)
    all_sorted = sorted(results, key=lambda x: x["ctime"])
    rank_map = {x["full_id"]: idx for idx, x in enumerate(all_sorted, start=1)}
    for item in results:
        item["footer_rank"] = rank_map.get(item["full_id"], 1)

    return results


# ─────────────────────────────────────────────────────────────────────────────
# 6. Live Hardware Telemetry Sampler & Multi-Row Oscilloscope Curve Renderer
# ─────────────────────────────────────────────────────────────────────────────
SPARK_BLOCKS = " ▁▂▃▄▅▆▇█"
GRID_DOT = "\033[38;2;45;50;68m·\033[0m"


class _HostCpuLoadInfo(ctypes.Structure):
    _fields_ = [("cpu_ticks", ctypes.c_uint32 * 4)]


class SystemTelemetrySampler:
    """
    Zero-overhead Apple Silicon (M4 Pro) hardware telemetry collector with rolling
    time-series history for multi-row oscilloscope curve rendering:
    - CPU Total & System/User % via Mach `host_statistics(HOST_CPU_LOAD_INFO)`
    - GPU Device/Renderer Utilization % & VRAM via `IOAccelerator` `PerformanceStatistics`
    - Unified Memory Used GB / % & Compressor GB via Mach `host_statistics64(HOST_VM_INFO64)`
    - Network RX/TX throughput (bytes/s) via `netstat -ibn`
    """

    def __init__(self, history_len: int = 60):
        self.history_len = history_len
        self.libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
        self.host = self.libc.mach_host_self()
        page_sz = ctypes.c_uint64(16384)
        try:
            self.libc.host_page_size(self.host, ctypes.byref(page_sz))
        except Exception:
            pass
        self.page_size = int(page_sz.value) or 16384

        try:
            brand = subprocess.check_output(["/usr/sbin/sysctl", "-n", "machdep.cpu.brand_string"], timeout=0.2).decode().strip()
            self.cpu_brand = brand.replace("Apple ", "")
        except Exception:
            self.cpu_brand = "M4 Pro"

        self.cpu_cores = os.cpu_count() or 12
        try:
            mem_b = int(subprocess.check_output(["/usr/sbin/sysctl", "-n", "hw.memsize"], timeout=0.2).decode().strip())
            self.total_mem_gb = mem_b / (1024**3)
        except Exception:
            self.total_mem_gb = 24.0

        self._prev_cpu_ticks: Optional[list[int]] = None
        self._prev_net: Optional[tuple[float, int, int]] = None

        # Pre-seed history with subtle baseline so curves render across the full width immediately
        self.cpu_hist: list[float] = [8.0] * history_len
        self.gpu_hist: list[float] = [6.0] * history_len
        self.mem_hist: list[float] = [50.0] * history_len
        self.net_hist: list[float] = [2.0] * history_len

        self.cpu_pct = 0.0
        self.load_1m = 0.0
        self.gpu_pct = 0.0
        self.gpu_vram_gb = 0.0
        self.mem_pct = 0.0
        self.mem_used_gb = 0.0
        self.mem_comp_gb = 0.0
        self.net_rx_bps = 0.0
        self.net_tx_bps = 0.0
        self.net_peak_bps = 1024.0 * 512.0

        # Take two quick initial samples to populate accurate starting values and seed history
        self.sample()
        time.sleep(0.04)
        self.sample()
        self.cpu_hist = [max(3.0, self.cpu_pct)] * history_len
        self.gpu_hist = [max(3.0, self.gpu_pct)] * history_len
        self.mem_hist = [max(10.0, self.mem_pct)] * history_len
        self.net_hist = [3.0] * history_len

    def sample(self):
        # 1. CPU Load via Mach host_statistics
        try:
            info = _HostCpuLoadInfo()
            cnt = ctypes.c_uint32(4)
            if self.libc.host_statistics(self.host, 3, ctypes.byref(info), ctypes.byref(cnt)) == 0:
                ticks = list(info.cpu_ticks)  # USER, SYSTEM, IDLE, NICE
                if self._prev_cpu_ticks is not None:
                    du = max(0, ticks[0] - self._prev_cpu_ticks[0])
                    ds = max(0, ticks[1] - self._prev_cpu_ticks[1])
                    di = max(0, ticks[2] - self._prev_cpu_ticks[2])
                    dn = max(0, ticks[3] - self._prev_cpu_ticks[3])
                    tot = du + ds + di + dn
                    if tot > 0:
                        self.cpu_pct = max(0.0, min(100.0, 100.0 * (du + ds + dn) / float(tot)))
                self._prev_cpu_ticks = ticks
        except Exception:
            pass

        # 2. Load average
        try:
            lavg = (ctypes.c_double * 3)()
            self.libc.getloadavg(lavg, 3)
            self.load_1m = float(lavg[0])
        except Exception:
            pass

        # 3. Unified Memory via Mach host_statistics64
        try:
            u32 = (ctypes.c_uint32 * 40)()
            vcnt = ctypes.c_uint32(38)
            if self.libc.host_statistics64(self.host, 4, ctypes.byref(u32), ctypes.byref(vcnt)) == 0:
                wired = u32[3]
                purgeable = u32[22]
                comp_occupied = u32[32]
                internal = u32[35]
                used_pages = max(0, internal - purgeable) + wired + comp_occupied
                self.mem_used_gb = (used_pages * self.page_size) / (1024**3)
                self.mem_comp_gb = (comp_occupied * self.page_size) / (1024**3)
                self.mem_pct = max(0.0, min(100.0, 100.0 * self.mem_used_gb / max(1.0, self.total_mem_gb)))
        except Exception:
            pass

        # 4. Apple Silicon AGX GPU via ioreg IOAccelerator
        try:
            out = subprocess.check_output(
                ["/usr/sbin/ioreg", "-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"],
                timeout=0.18,
                stderr=subprocess.DEVNULL,
            ).decode("utf-8", errors="ignore")
            m_dev = re.search(r'"Device Utilization %"=(\d+)', out)
            m_ren = re.search(r'"Renderer Utilization %"=(\d+)', out)
            dev_u = float(m_dev.group(1)) if m_dev else 0.0
            ren_u = float(m_ren.group(1)) if m_ren else 0.0
            self.gpu_pct = max(dev_u, ren_u)
            m_vram = re.search(r'"In use system memory"=(\d+)', out) or re.search(r'"Alloc system memory"=(\d+)', out)
            if m_vram:
                self.gpu_vram_gb = int(m_vram.group(1)) / (1024**3)
        except Exception:
            pass

        # 5. Network I/O via netstat -ibn
        try:
            now = time.monotonic()
            net_out = subprocess.check_output(
                ["/usr/sbin/netstat", "-ibn"],
                timeout=0.18,
                stderr=subprocess.DEVNULL,
            ).decode("utf-8", errors="ignore")
            rx_b = tx_b = 0
            for line in net_out.splitlines():
                parts = line.split()
                if len(parts) >= 10 and parts[0].startswith("en") and "<Link#" in parts[2]:
                    try:
                        rx_b += int(parts[6])
                        tx_b += int(parts[9])
                    except ValueError:
                        pass
            if self._prev_net is not None:
                dt = max(0.05, now - self._prev_net[0])
                self.net_rx_bps = max(0.0, (rx_b - self._prev_net[1]) / dt)
                self.net_tx_bps = max(0.0, (tx_b - self._prev_net[2]) / dt)
            self._prev_net = (now, rx_b, tx_b)
        except Exception:
            pass

        tot_net = self.net_rx_bps + self.net_tx_bps
        self.net_peak_bps = max(256.0 * 1024.0, self.net_peak_bps * 0.98, tot_net)
        net_pct = max(2.0 if tot_net > 0 else 0.0, min(100.0, 100.0 * tot_net / max(1.0, self.net_peak_bps)))

        self.cpu_hist = (self.cpu_hist + [self.cpu_pct])[-self.history_len :]
        self.gpu_hist = (self.gpu_hist + [self.gpu_pct])[-self.history_len :]
        self.mem_hist = (self.mem_hist + [self.mem_pct])[-self.history_len :]
        self.net_hist = (self.net_hist + [net_pct])[-self.history_len :]


def _fmt_rate(bps: float) -> str:
    if bps >= 1024 * 1024:
        return f"{bps / (1024 * 1024):.1f}M"
    if bps >= 1024:
        return f"{bps / 1024:.0f}K"
    return f"{int(bps)}B"


def render_multirow_curve(
    samples: list[float],
    width: int,
    height: int,
    col_low: str,
    col_mid: str,
    col_high: str,
) -> list[str]:
    """
    Renders a `height`-row × `width`-col sub-pixel oscilloscope curve (`8 * height` vertical levels)
    with a subtle dot-matrix coordinate reticle behind empty cells.
    """
    height = max(1, height)
    width = max(8, width)
    sliced = samples[-width:]
    if len(sliced) < width:
        sliced = ([sliced[0]] * (width - len(sliced)) if sliced else [0.0] * width) + sliced

    total_eighths = height * 8
    rows_str: list[str] = []

    for r in range(height):
        row_from_bottom = height - 1 - r
        row_color = col_high if r == 0 and height >= 3 else (col_mid if r < height - 1 else col_low)
        parts: list[str] = []
        active_color = ""

        for x, val in enumerate(sliced):
            v = max(0.0, min(100.0, val))
            fill_8 = int(round((v / 100.0) * total_eighths))
            if v > 0.2 and fill_8 == 0:
                fill_8 = 1
            rem = fill_8 - row_from_bottom * 8

            if rem <= 0:
                if active_color:
                    parts.append(RESET)
                    active_color = ""
                if x % 6 == 0:
                    parts.append(GRID_DOT)
                else:
                    parts.append(" ")
            else:
                lvl = min(8, rem)
                ch = SPARK_BLOCKS[lvl]
                c_use = col_high if v >= 80.0 else (col_mid if v >= 45.0 else row_color)
                if c_use != active_color:
                    parts.append(c_use)
                    active_color = c_use
                parts.append(ch)

        if active_color:
            parts.append(RESET)
        rows_str.append("".join(parts))

    return rows_str


def build_telemetry_panel_rows(
    sampler: SystemTelemetrySampler,
    inner_w: int,
    panel_height: int,
) -> list[BoxRow]:
    """
    Constructs exact-height `BoxRow`s for the bottom Device Telemetry Panel (`CPU`, `GPU`, `MEM`, `NET`)
    so `len(rows) == panel_height - 2` across any terminal resolution.
    """
    avail = max(8, panel_height - 2)
    curve_w = max(10, inner_w - 2)

    use_dividers = avail >= 15
    overhead = 7 if use_dividers else 4
    curve_budget = max(4, avail - overhead)

    h_cpu = max(1, curve_budget // 4 + (1 if curve_budget % 4 >= 1 else 0))
    h_gpu = max(1, curve_budget // 4 + (1 if curve_budget % 4 >= 2 else 0))
    h_mem = max(1, curve_budget // 4 + (1 if curve_budget % 4 >= 3 else 0))
    h_net = max(1, curve_budget - h_cpu - h_gpu - h_mem)

    div_row = BoxRow(f" \033[38;2;48;52;70m{'┄' * curve_w}{RESET}")
    rows: list[BoxRow] = []

    # 1. CPU Curve
    cpu_c = CORAL if sampler.cpu_pct >= 80 else (AMBER if sampler.cpu_pct >= 45 else MINT)
    rows.append(
        BoxRow(
            f" {cpu_c}●{RESET} {BOLD}{WHITE}CPU{RESET} {DIM}{sampler.cpu_cores}C{RESET}  "
            f"{BOLD}{cpu_c}{sampler.cpu_pct:4.1f}%{RESET}  {DIM}load {sampler.load_1m:.2f}{RESET}"
        )
    )
    for cline in render_multirow_curve(sampler.cpu_hist, curve_w, h_cpu, MINT, CYAN, CORAL):
        rows.append(BoxRow(f" {cline}"))
    if use_dividers:
        rows.append(div_row)

    # 2. GPU Curve
    gpu_c = CORAL if sampler.gpu_pct >= 80 else (AMBER if sampler.gpu_pct >= 45 else BLUE)
    rows.append(
        BoxRow(
            f" {gpu_c}●{RESET} {BOLD}{WHITE}GPU{RESET} {DIM}AGX{RESET}  "
            f"{BOLD}{gpu_c}{sampler.gpu_pct:4.1f}%{RESET}  {DIM}vram {sampler.gpu_vram_gb:.1f}G{RESET}"
        )
    )
    for cline in render_multirow_curve(sampler.gpu_hist, curve_w, h_gpu, BLUE, CYAN, AMBER):
        rows.append(BoxRow(f" {cline}"))
    if use_dividers:
        rows.append(div_row)

    # 3. Unified Memory Curve
    mem_c = CORAL if sampler.mem_pct >= 85 else (AMBER if sampler.mem_pct >= 70 else LAVENDER)
    rows.append(
        BoxRow(
            f" {mem_c}●{RESET} {BOLD}{WHITE}MEM{RESET} {DIM}{int(sampler.total_mem_gb)}G{RESET}  "
            f"{BOLD}{mem_c}{sampler.mem_pct:4.1f}%{RESET}  {DIM}{sampler.mem_used_gb:.1f}/{int(sampler.total_mem_gb)}G{RESET}"
        )
    )
    for cline in render_multirow_curve(sampler.mem_hist, curve_w, h_mem, LAVENDER, AMBER, CORAL):
        rows.append(BoxRow(f" {cline}"))
    if use_dividers:
        rows.append(div_row)

    # 4. Network I/O Curve
    rx_s = _fmt_rate(sampler.net_rx_bps)
    tx_s = _fmt_rate(sampler.net_tx_bps)
    rows.append(
        BoxRow(
            f" {CYAN}●{RESET} {BOLD}{WHITE}NET{RESET} {GREEN}▼{rx_s:>4}{RESET} {AMBER}▲{tx_s:>4}{RESET} "
            f"{DIM}pk {_fmt_rate(sampler.net_peak_bps)}/s{RESET}"
        )
    )
    for cline in render_multirow_curve(sampler.net_hist, curve_w, h_net, CYAN, MINT, GREEN):
        rows.append(BoxRow(f" {cline}"))

    return rows[:avail]
