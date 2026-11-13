#!/usr/bin/env python3
"""
Full-Stream Hash-Free Git Graph Log for Lazygit `[0]-Log` (`cc-ide`).

1. Hides noisy SHA hashes by default — starts directly from `Author + Commit Graph + [Ref] + Subject`.
2. Default Unwrapped Mode (`~/.config/cc-ide/viewer.json` `"wrap": false`):
   - Renders each commit on a single full unwrapped line so users pan horizontally via Magic Mouse
     or keyboard (`H`/`L`, `Shift/Alt + Left/Right`) with content-bounds clamping.
3. Optional `vi`-Style Wrap Mode (`"wrap": true` or `CCIDE_WRAP=1`):
   - Wraps long subjects right up to `gocui.View.InnerWidth()` with dim `↪` continuation markers
     and continuous vertical `--graph` branch pipes.
"""
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import unicodedata

if hasattr(signal, "SIGPIPE"):
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)

ANSI_CSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
ANSI_OSC_RE = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")
RESET = "\x1b[0m"
BOLD = "\x1b[1m"
SHOWBREAK_GLYPH = "\x1b[38;5;242m↪\x1b[0m"
CJK_PAD_GLYPH = "\x1b[38;5;240m↩\x1b[0m"
AUTHOR_PALETTE = [
    "\x1b[38;5;117m",
    "\x1b[38;5;114m",
    "\x1b[38;5;215m",
    "\x1b[38;5;183m",
    "\x1b[38;5;210m",
    "\x1b[38;5;151m",
]


def strip_ansi(s: str) -> str:
    return ANSI_CSI_RE.sub("", ANSI_OSC_RE.sub("", s))


def char_cols(ch: str) -> int:
    o = ord(ch)
    if (
        o < 0x20
        or 0x7F <= o < 0xA0
        or o in (0x200B, 0x200C, 0x200D, 0xFE0E, 0xFE0F)
        or unicodedata.combining(ch)
    ):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def visible_width(s: str) -> int:
    return sum(char_cols(c) for c in strip_ansi(s))


def get_sid_str() -> str:
    return os.environ.get("CCIDE_LG_SID", "").strip() or str(os.getsid(0))


def write_full_lines_cache(sid_str: str, panel_w: int, full_lines: list[str]) -> None:
    try:
        os.makedirs("/tmp/.cc-ide", exist_ok=True)
        dst = f"/tmp/.cc-ide/lg_full_{sid_str}.json"
        h = hashlib.blake2s(f"{panel_w}\0".encode("utf-8"), digest_size=12)
        for ln in full_lines:
            h.update(ln.encode("utf-8", errors="surrogatepass"))
            h.update(b"\n")
        digest = h.hexdigest()
        if os.path.isfile(dst):
            try:
                with open(dst, "r", encoding="utf-8") as rf:
                    head = rf.read(96)
                if f'"digest": "{digest}"' in head or f'"digest":"{digest}"' in head:
                    return
            except OSError:
                pass
        tmp = f"{dst}.tmp.{os.getpid()}"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump({"digest": digest, "panel_w": panel_w, "lines": full_lines}, f, ensure_ascii=False)
        os.replace(tmp, dst)
    except Exception:
        pass


def slice_ansi_window(s: str, offset_cols: int, width_cols: int) -> str:
    """Slices ANSI text to exact display columns `[offset_cols : offset_cols + width_cols]` without wrapping."""
    width_cols = max(8, width_cols)
    s = (
        re.sub(r"\x1b\[0?K|\x1b[\(\)\*\+][A-Za-z0-9]", "", ANSI_OSC_RE.sub("", s))
        .replace("\r", "")
        .expandtabs(4)
    )
    if offset_cols <= 0 and visible_width(s) <= width_cols:
        return s

    tokens: list[tuple[bool, str, int]] = []
    i, n = 0, len(s)
    while i < n:
        if s[i] == "\x1b":
            if i + 1 < n and s[i + 1] == "[":
                j = i + 2
                while j < n and not ("@" <= s[j] <= "~"):
                    j += 1
                if j < n:
                    seq = s[i : j + 1]
                    if seq.endswith("m"):
                        tokens.append((True, seq, 0))
                    i = j + 1
                    continue
            i += 2 if (i + 1 < n) else 1
            continue
        ch = s[i]
        o = ord(ch)
        if o < 0x20 or (0x7F <= o < 0xA0):
            i += 1
            continue
        tokens.append((False, ch, char_cols(ch)))
        i += 1

    active_sgr = ""
    pos_col = 0
    out: list[str] = []
    out_w = 0
    started = False
    budget = width_cols

    for is_ansi, txt, cw in tokens:
        if is_ansi:
            if txt in ("\x1b[0m", "\x1b[m"):
                active_sgr = ""
            else:
                active_sgr += txt
            if started:
                out.append(txt)
            continue

        if pos_col + cw <= offset_cols:
            pos_col += cw
            continue

        if not started:
            started = True
            if offset_cols > 0:
                out.append("\x1b[38;5;242m‹\x1b[0m")
                budget = max(4, width_cols - 1)
            if active_sgr:
                out.append(active_sgr)
            if pos_col < offset_cols and cw == 2:
                # 2-cell CJK character straddled the left cut -> 1-cell pad
                if out_w + 1 <= budget:
                    out.append(" ")
                    out_w += 1
                pos_col += cw
                continue

        if out_w + cw > budget:
            break
        out.append(txt)
        out_w += cw
        pos_col += cw

    if not started:
        return ""
    out.append(RESET)
    return "".join(out)


def load_wrap_enabled(argv: list[str] | None = None) -> bool:
    if argv:
        for arg in argv:
            if arg == "--wrap":
                return True
            if arg == "--no-wrap":
                return False
    env_v = os.environ.get("CCIDE_WRAP", "").strip().lower()
    if env_v in ("1", "true", "yes", "on"):
        return True
    if env_v in ("0", "false", "no", "off"):
        return False

    xdg = os.environ.get("XDG_CONFIG_HOME", "").strip()
    candidates = []
    if xdg:
        candidates.append(os.path.join(xdg, "cc-ide", "viewer.json"))
    candidates.append(os.path.expanduser("~/.config/cc-ide/viewer.json"))
    script_dir = os.path.dirname(os.path.realpath(__file__))
    candidates.append(os.path.join(os.path.dirname(script_dir), "assets", "viewer.json"))

    for p in candidates:
        if os.path.isfile(p):
            try:
                with open(p, "r", encoding="utf-8") as f:
                    raw = f.read()
                no_block = re.sub(r"/\*.*?\*/", "", raw, flags=re.DOTALL)
                no_line = re.sub(r"(?m)^\s*//.*$|(?<=\s)//.*$", "", no_block)
                no_trail = re.sub(r",\s*([}\]])", r"\1", no_line)
                data = json.loads(no_trail)
                if isinstance(data, dict) and isinstance(data.get("wrap"), bool):
                    return data["wrap"]
            except Exception:
                pass
    return False


def get_panel_geometry() -> tuple[int, int]:
    rows, cols, exact_main_w = 0, 0, 0
    try:
        sid_str = os.environ.get("CCIDE_LG_SID", "").strip() or str(os.getsid(0))
        geom_path = f"/tmp/.cc-ide/lg_geom_{sid_str}.txt"
        if os.path.isfile(geom_path):
            with open(geom_path, "r", encoding="utf-8") as f:
                parts = f.read().split()
            if len(parts) >= 2 and parts[0].isdigit() and parts[1].isdigit():
                rows, cols = int(parts[0]), int(parts[1])
            if len(parts) >= 3 and parts[2].isdigit():
                exact_main_w = int(parts[2])
    except Exception:
        pass

    if rows <= 0 or cols <= 0:
        try:
            fd = os.open("/dev/tty", os.O_RDONLY)
            try:
                sz = os.get_terminal_size(fd)
                if sz.lines > 0 and sz.columns > 0:
                    rows, cols = sz.lines, sz.columns
            finally:
                os.close(fd)
        except Exception:
            pass
    if rows <= 0 or cols <= 0:
        rows = int(os.environ.get("CCIDE_LG_ROWS", "32") or 32)
        cols = int(os.environ.get("CCIDE_LG_COLS", "81") or 81)

    if exact_main_w > 0:
        panel_w = max(28, exact_main_w)
    else:
        side_w = int(cols * 0.32)
        panel_w = max(28, cols - side_w - 3)
    panel_h = max(12, rows - 3)
    return panel_h, panel_w


def compact_author(raw_author: str) -> str:
    name = raw_author.strip()
    if not name:
        return "??"
    if any(char_cols(c) == 2 for c in name):
        out = []
        w = 0
        for c in name:
            cw = char_cols(c)
            if w + cw > 4:
                break
            out.append(c)
            w += cw
        return "".join(out)
    parts = [p for p in re.split(r"[\s._-]+", name) if p]
    if len(parts) >= 2:
        return (parts[0][0] + parts[-1][0]).upper()
    return name[:3]


def author_color(name: str) -> str:
    h = sum(ord(c) for c in name)
    return AUTHOR_PALETTE[h % len(AUTHOR_PALETTE)]


def wrap_vi_ansi(
    s: str,
    first_budget: int,
    cont_prefix: str,
    cont_budget: int,
) -> list[str]:
    """Wraps ANSI text `vi`-style right up to `first_budget` / `cont_budget` with `cont_prefix`."""
    first_budget = max(8, first_budget)
    cont_budget = max(6, cont_budget)
    s = ANSI_OSC_RE.sub("", s)

    tokens: list[tuple[bool, str, int]] = []
    i, n = 0, len(s)
    while i < n:
        if s[i] == "\x1b" and i + 1 < n and s[i + 1] == "[":
            j = i + 2
            while j < n and not ("@" <= s[j] <= "~"):
                j += 1
            if j < n:
                seq = s[i : j + 1]
                if seq.endswith("m"):
                    tokens.append((True, seq, 0))
                i = j + 1
                continue
        ch = s[i]
        tokens.append((False, ch, char_cols(ch)))
        i += 1

    segments: list[str] = []
    cur_buf: list[str] = []
    cur_w = 0
    active_sgr = ""
    budget = first_budget

    for is_ansi, txt, cw in tokens:
        if is_ansi:
            if txt in ("\x1b[0m", "\x1b[m"):
                active_sgr = ""
            else:
                active_sgr += txt
            cur_buf.append(txt)
            continue

        if cur_w + cw > budget and cur_w > 0:
            if cur_w == budget - 1 and cw == 2:
                cur_buf.append(f"{RESET}{CJK_PAD_GLYPH}")
            else:
                cur_buf.append(RESET)
            segments.append("".join(cur_buf))
            cur_buf = [cont_prefix]
            if active_sgr:
                cur_buf.append(active_sgr)
            cur_w = 0
            budget = cont_budget

        cur_buf.append(txt)
        cur_w += cw

    cur_buf.append(RESET)
    segments.append("".join(cur_buf))
    return segments if segments else [RESET]


def format_full_width_log(
    branch_arg: str | None,
    show_all: bool,
    max_commits: int,
    panel_w: int,
    wrap_enabled: bool = False,
) -> list[str]:
    sep = "\x1f"
    pretty = f"format:{sep}%an{sep}%d{sep}%s"
    cmd = [
        "git",
        "--no-optional-locks",
        "log",
        "--graph",
        "--color=always",
        f"-n{max_commits}",
        f"--pretty={pretty}",
    ]
    if show_all:
        cmd.append("--all")
    elif branch_arg:
        cmd.extend([branch_arg, "--"])

    try:
        res = subprocess.run(cmd, capture_output=True, text=True, check=False)
    except Exception:
        return []

    raw_rows = res.stdout.splitlines()
    out_lines: list[str] = []
    total_rows = len(raw_rows)

    for idx, raw in enumerate(raw_rows):
        raw_clean_osc = ANSI_OSC_RE.sub("", raw)
        if sep not in raw_clean_osc:
            conn_line = "    " + raw_clean_osc + RESET
            if wrap_enabled and visible_width(conn_line) > panel_w:
                out_lines.extend(wrap_vi_ansi(conn_line, panel_w, SHOWBREAK_GLYPH, panel_w - 1))
            else:
                out_lines.append(conn_line)
            continue

        parts = raw_clean_osc.split(sep)
        if len(parts) < 4:
            out_lines.append("    " + raw_clean_osc + RESET)
            continue

        graph_pfx = parts[0]
        author_raw = strip_ansi(parts[1])
        deco_raw = strip_ansi(parts[2]).strip()
        subject_raw = strip_ansi(parts[3]).strip()

        au = compact_author(author_raw)
        au_w = visible_width(au)
        pad_w = max(1, 4 - au_w)
        au_prefix = f"{author_color(author_raw)}{BOLD}{au}{RESET}{' ' * pad_w}"
        au_total_w = au_w + pad_w

        deco_badge = ""
        if deco_raw.startswith("(") and deco_raw.endswith(")"):
            inner = deco_raw[1:-1]
            refs = [r.strip().replace("HEAD -> ", "→").replace("tag: ", "") for r in inner.split(",") if r.strip()]
            if refs:
                badge_txt = ", ".join(refs[:2])
                if len(badge_txt) > 18:
                    badge_txt = badge_txt[:17] + "…"
                deco_badge = f"\x1b[38;5;221m[{badge_txt}]{RESET} "

        lead = f"{au_prefix}{graph_pfx}{RESET}"
        body = f"{deco_badge}\x1b[38;5;252m{subject_raw}{RESET}"

        if not wrap_enabled:
            out_lines.append(lead + body)
            continue

        lead_w = au_total_w + visible_width(graph_pfx)
        has_next = idx + 1 < total_rows
        pipe_repl = "│" if has_next else " "
        graph_cont = graph_pfx.replace("*", pipe_repl)
        cont_pfx = f"{' ' * au_total_w}{graph_cont}{RESET}{SHOWBREAK_GLYPH}"
        cont_w = lead_w + 1

        first_budget = max(10, panel_w - lead_w)
        cont_budget = max(8, panel_w - cont_w)

        wrapped_body = wrap_vi_ansi(body, first_budget, cont_pfx, cont_budget)
        for seg_idx, seg in enumerate(wrapped_body):
            if seg_idx == 0:
                out_lines.append(lead + seg)
            else:
                out_lines.append(seg)

    return out_lines


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    branch_name: str | None = None
    show_all = False
    override_w = 0
    wrap_enabled = load_wrap_enabled(args)

    for idx, arg in enumerate(args):
        if arg in ("--all", "--wrap", "--no-wrap"):
            if arg == "--all":
                show_all = True
        elif arg.startswith("--width="):
            val = arg.split("=", 1)[1].strip()
            if val.isdigit():
                override_w = int(val)
        elif arg.startswith("--branch="):
            branch_name = arg.split("=", 1)[1].strip()
        elif arg == "--branch" and idx + 1 < len(args):
            branch_name = args[idx + 1].strip()
        elif not arg.startswith("-") and branch_name is None:
            branch_name = arg

    _, panel_w = get_panel_geometry()
    if override_w > 0:
        panel_w = override_w

    lines = format_full_width_log(branch_name, show_all, 120, panel_w, wrap_enabled)
    sid_str = get_sid_str()
    if not wrap_enabled:
        write_full_lines_cache(sid_str, panel_w, lines)
        lines = [slice_ansi_window(ln, 0, panel_w) for ln in lines]
    else:
        write_full_lines_cache(sid_str, panel_w, [])

    if lines:
        sys.stdout.write("\n".join(lines) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
