#!/usr/bin/env python3
"""
Compact Single-Column Line-Number Diff/Patch Pager for Lazygit (`cc-ide`).

1. Default Unwrapped Full-Line Mode (`~/.config/cc-ide/viewer.json` `"wrap": false`):
   - Collapses dual diff line numbers into a single compact column (`109│`) and renders code,
     commit headers, and Markdown commit messages on full unwrapped lines so users pan horizontally
     via Magic Mouse or keyboard (`H`/`L`, `Shift/Alt + Left/Right`) with content-bounds clamping.
2. Optional `vi`-Style Full-Width Wrap Mode (`"wrap": true` or `CCIDE_WRAP=1`):
   - Wraps right up to `gocui.View.InnerWidth()` (`nolinebreak`), placing dim `↪` inside the
     line-number column (`  ↪│`) on wrapped code rows and `↪` / `↩` on prose rows.
"""
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import unicodedata

if hasattr(signal, "SIGPIPE"):
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)

ANSI_CSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
ANSI_OSC_RE = re.compile(r"\x1b\][^\x07]*(?:\x07|\x1b\\)")
TRAILING_ANSI_SPACES_RE = re.compile(r"(?:(?:\x1b\[[0-9;]*m)+[ \t]+)+(?:\x1b\[[0-9;]*m)*$")
SENTINEL_RE = re.compile(r"^(.*?)\x01(.*?)\x02(.*?)\x03(.*)$")
HUNK_HDR_RE = re.compile(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$")
FENCE_RE = re.compile(r"^\s{0,3}(`{3,}|~{3,})")
EMAIL_BRACKET_RE = re.compile(r"<([^>\s]+@[^>\s]+)>")

GUTTER_BAR = "\x1b[34m│\x1b[0m"
SHOWBREAK_GLYPH = "\x1b[38;5;242m↪\x1b[0m"
CJK_PAD_GLYPH = "\x1b[38;5;240m↩\x1b[0m"
COL_NUM_CTX = "\x1b[38;5;242m"
COL_NUM_DEL = "\x1b[38;5;167m"
COL_NUM_ADD = "\x1b[38;5;114m"
BG_DEL_LINE = "\x1b[48;2;52;14;18m"
BG_ADD_LINE = "\x1b[48;2;12;44;24m"


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


def get_ipc_dir() -> str:
    return os.environ.get("CCIDE_IPC_DIR") or (
        os.path.join(tempfile.gettempdir(), ".cc-ide") if os.name == "nt" else "/tmp/.cc-ide"
    )


def get_sid_str() -> str:
    return os.environ.get("CCIDE_LG_SID", "").strip() or str(
        os.getsid(0) if hasattr(os, "getsid") else os.getppid()
    )


def write_full_lines_cache(sid_str: str, panel_w: int, full_lines: list[str]) -> None:
    try:
        ipc_dir = get_ipc_dir()
        os.makedirs(ipc_dir, exist_ok=True)
        dst = os.path.join(ipc_dir, f"lg_full_{sid_str}.json")
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
    out.append("\x1b[0m")
    return "".join(out)


def finalize_output(rendered: str, panel_w: int, wrap_enabled: bool, cli_width_explicit: bool = False) -> str:
    """
    When `wrap_enabled` is `False` (default), caches the full unwrapped ANSI lines in
    `/tmp/.cc-ide/lg_full_<sid>.json` for `pty-hscroll-bridge.py`'s `< 0.15ms` 120Hz in-memory
    horizontal blitter, and outputs `pan_x = 0` sliced lines (`<= panel_w`) so Lazygit 0.54's
    `Views.Main.Wrap = true` never wraps a single line.
    """
    sid_str = get_sid_str()
    if wrap_enabled or cli_width_explicit:
        write_full_lines_cache(sid_str, panel_w, [])
        return rendered
    lines = rendered.splitlines()
    write_full_lines_cache(sid_str, panel_w, lines)
    sliced = [slice_ansi_window(ln, 0, panel_w) for ln in lines]
    return "\n".join(sliced) + ("\n" if sliced else "")


def load_wrap_enabled(argv: list[str] | None = None) -> bool:
    """Reads `wrap` boolean from CLI flags, `CCIDE_WRAP`, or `~/.config/cc-ide/viewer.json` (default: `False`)."""
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


def read_live_geom() -> tuple[int, int]:
    """Returns `(pane_cols, exact_main_inner_w)` from `pty-hscroll-bridge.py` if available."""
    try:
        sid_str = get_sid_str()
        geom_path = os.path.join(get_ipc_dir(), f"lg_geom_{sid_str}.txt")
        if os.path.isfile(geom_path):
            with open(geom_path, "r", encoding="utf-8") as f:
                parts = f.read().split()
            pane_cols = int(parts[1]) if len(parts) >= 2 and parts[1].isdigit() else 0
            main_w = int(parts[2]) if len(parts) >= 3 and parts[2].isdigit() else 0
            return pane_cols, main_w
    except Exception:
        pass
    return 0, 0


def resolve_panel_width(argv: list[str]) -> int:
    pane_cols, exact_main_w = read_live_geom()
    flag_col_w: int | None = None
    flag_full_w: int | None = None

    for idx, arg in enumerate(argv):
        if arg.startswith("--col-width="):
            val = arg.split("=", 1)[1].strip()
            if val.lstrip("-").isdigit():
                flag_col_w = int(val)
        elif arg == "--col-width" and idx + 1 < len(argv) and argv[idx + 1].strip().lstrip("-").isdigit():
            flag_col_w = int(argv[idx + 1].strip())
        elif arg.startswith("--width="):
            val = arg.split("=", 1)[1].strip()
            if val.isdigit():
                flag_full_w = int(val)
        elif arg == "--width" and idx + 1 < len(argv) and argv[idx + 1].strip().isdigit():
            flag_full_w = int(argv[idx + 1].strip())

    if flag_full_w is not None:
        return max(24, flag_full_w)

    if flag_col_w is not None:
        # Lazygit 0.54 computes `columnWidth = values.Width // 2 - 6`, losing `values.Width % 2` on odd widths.
        # Recover exact odd `InnerWidth` (`49` vs `48`) when `exact_main_w` matches within rounding!
        derived = (flag_col_w + 6) * 2
        if exact_main_w > 0 and abs(exact_main_w - derived) <= 2:
            return max(24, exact_main_w)
        return max(24, derived)

    env_w = os.environ.get("LAZYGIT_COLUMNS", "").strip()
    if env_w.isdigit():
        return max(24, int(env_w))

    if exact_main_w > 0:
        return max(24, exact_main_w)

    if pane_cols <= 0:
        pane_cols = 81
        try:
            fd = os.open("/dev/tty", os.O_RDONLY)
            try:
                sz = os.get_terminal_size(fd)
                if sz.columns > 0:
                    pane_cols = sz.columns
            finally:
                os.close(fd)
        except Exception:
            pass
    side_w = int(pane_cols * 0.32)
    return max(28, pane_cols - side_w - 3)


def find_bin(name: str) -> str | None:
    found = shutil.which(name)
    if found:
        return found
    for prefix in ("/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"):
        cand = os.path.join(prefix, name)
        if os.path.isfile(cand) and os.access(cand, os.X_OK):
            return cand
    return None


def wrap_vi_ansi(
    s: str,
    max_cols: int,
    cont_prefix: str = SHOWBREAK_GLYPH,
    cont_prefix_cols: int = 1,
    line_bg: str = "",
) -> list[str]:
    """
    `vi`-style (`:set wrap nolinebreak showbreak=↪`) exact-column ANSI wrapper:
    - Fills every single cell up to `max_cols` without early word-break whitespace gaps.
    - Carries active SGR escape sequences seamlessly across wrapped rows.
    - Prepends `cont_prefix` (default `↪`, width `cont_prefix_cols`) on wrapped continuation rows.
    - Fills a 1-cell trailing remainder before a 2-cell CJK glyph with dim `↩` so right edges stay flush.
    """
    first_budget = max(6, max_cols)
    cont_budget = max(4, max_cols - cont_prefix_cols)
    s = re.sub(r"\x1b\[0?K", "", s).expandtabs(4)

    tokens: list[tuple[bool, str, int]] = []
    i = 0
    n = len(s)
    while i < n:
        if s[i] == "\x1b":
            if i + 1 < n and s[i + 1] == "]":
                j = i + 2
                while j < n and s[j] != "\x07":
                    if s[j] == "\x1b" and j + 1 < n and s[j + 1] == "\\":
                        j += 1
                        break
                    j += 1
                i = min(n, j + 1)
                continue
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
        ch = s[i]
        tokens.append((False, ch, char_cols(ch)))
        i += 1

    segments: list[str] = []
    cur_buf: list[str] = [line_bg] if line_bg else []
    cur_w = 0
    active_sgr = line_bg
    budget = first_budget

    for is_ansi, txt, cw in tokens:
        if is_ansi:
            if txt in ("\x1b[0m", "\x1b[m"):
                active_sgr = line_bg
                cur_buf.append(txt)
                if line_bg:
                    cur_buf.append(line_bg)
            else:
                active_sgr += txt
                cur_buf.append(txt)
            continue

        if cur_w + cw > budget and cur_w > 0:
            # If 1 cell remains on this row because `txt` is a 2-cell CJK character, mark right edge with `↩`
            if cur_w == budget - 1 and cw == 2:
                cur_buf.append(f"\x1b[0m{CJK_PAD_GLYPH}")
            else:
                cur_buf.append("\x1b[0m")
            segments.append("".join(cur_buf))
            cur_buf = []
            if cont_prefix:
                cur_buf.append(cont_prefix)
            if active_sgr:
                cur_buf.append(active_sgr)
            cur_w = 0
            budget = cont_budget

        cur_buf.append(txt)
        cur_w += cw

    cur_buf.append("\x1b[0m")
    segments.append("".join(cur_buf))
    return segments if segments else ["\x1b[0m"]


def clean_glow_line(line: str) -> str:
    no_osc = ANSI_OSC_RE.sub("", line)
    cleaned = TRAILING_ANSI_SPACES_RE.sub("\x1b[0m", no_osc.rstrip(" "))
    if cleaned.startswith("  "):
        cleaned = cleaned[2:]
    return cleaned


def resolve_glow_style() -> str:
    xdg = os.environ.get("XDG_CONFIG_HOME", "").strip()
    candidates = []
    if xdg:
        candidates.append(os.path.join(xdg, "cc-ide", "glow-style.json"))
    candidates.append(os.path.expanduser("~/.config/cc-ide/glow-style.json"))
    script_dir = os.path.dirname(os.path.realpath(__file__))
    candidates.append(os.path.join(os.path.dirname(script_dir), "assets", "glow-style.json"))
    for p in candidates:
        if os.path.isfile(p):
            return p
    return "dark"


def preprocess_markdown_for_glow(md_text: str) -> str:
    """Normalizes spaceless `###Heading` and escapes `<email@host>` outside code fences."""
    out: list[str] = []
    in_fence = False
    fence_marker = ""
    for line in md_text.splitlines():
        fm = FENCE_RE.match(line)
        if fm:
            m = fm.group(1)
            if not in_fence:
                in_fence = True
                fence_marker = m[0] * len(m)
            elif line.lstrip().startswith(fence_marker):
                in_fence = False
                fence_marker = ""
            out.append(line)
            continue
        if not in_fence:
            line = re.sub(r"^(\s{0,3}#{1,6})([^\s#])", r"\1 \2", line)
            line = EMAIL_BRACKET_RE.sub(r"\\<\1\\>", line)
        out.append(line)
    return "\n".join(out)


def format_unwrapped_code_line(code: str, line_bg: str = "") -> str:
    cleaned = re.sub(r"\x1b\[0?K", "", code).expandtabs(4)
    if not line_bg:
        return f"{cleaned}\x1b[0m"
    # Re-assert `line_bg` after internal SGR resets so added/deleted tint spans the full unwrapped line
    reasserted = re.sub(r"\x1b\[0?m", f"\x1b[0m{line_bg}", cleaned)
    return f"{line_bg}{reasserted}\x1b[0m"


def run_glow(md_text: str, width: int, wrap_enabled: bool = False) -> str:
    md_norm = preprocess_markdown_for_glow(md_text)
    glow_bin = find_bin("glow")
    if glow_bin:
        env = os.environ.copy()
        env["CLICOLOR_FORCE"] = "1"
        try:
            res = subprocess.run(
                [glow_bin, "-s", resolve_glow_style(), "-w", "2000", "-"],
                input=md_norm,
                text=True,
                capture_output=True,
                env=env,
            )
            if res.returncode == 0 and res.stdout.strip():
                cleaned = [clean_glow_line(ln) for ln in res.stdout.splitlines()]
                while cleaned and not strip_ansi(cleaned[0]).strip():
                    cleaned.pop(0)
                while cleaned and not strip_ansi(cleaned[-1]).strip():
                    cleaned.pop()
                if not wrap_enabled:
                    return "\n".join(cleaned) + "\n"
                wrapped_out: list[str] = []
                for ln in cleaned:
                    if not strip_ansi(ln).strip():
                        wrapped_out.append("")
                    else:
                        wrapped_out.extend(wrap_vi_ansi(ln, width))
                return "\n".join(wrapped_out) + "\n"
        except Exception:
            pass

    if not wrap_enabled:
        return md_norm + ("\n" if not md_norm.endswith("\n") else "")
    wrapped_fb: list[str] = []
    for ln in md_norm.splitlines():
        wrapped_fb.extend(wrap_vi_ansi(ln, width) if ln else [""])
    return "\n".join(wrapped_fb) + "\n"


def format_delta_compact_single_column(delta_out: str, width: int, wrap_enabled: bool = False) -> str:
    raw_lines = delta_out.splitlines()
    parsed_rows: list[tuple[str, str, str] | str] = []
    max_num = 1

    for ln in raw_lines:
        m = SENTINEL_RE.match(ln)
        if not m:
            parsed_rows.append(ln)
            continue
        nm = strip_ansi(m.group(2)).strip()
        np = strip_ansi(m.group(3)).strip()
        code = m.group(4)
        if nm.isdigit():
            max_num = max(max_num, int(nm))
        if np.isdigit():
            max_num = max(max_num, int(np))
        parsed_rows.append((nm, np, code))

    num_w = max(3, len(str(max_num)))
    gutter_w = num_w + 1  # `<num>│`
    code_w = max(12, width - gutter_w)
    cont_gutter = (" " * (num_w - 1)) + SHOWBREAK_GLYPH + GUTTER_BAR
    rule_w = max(16, width if wrap_enabled else (width - 3))

    out_lines: list[str] = []
    for item in parsed_rows:
        if isinstance(item, str):
            clean_hdr = strip_ansi(item)
            if clean_hdr and all(ch == "─" for ch in clean_hdr):
                out_lines.append(f"\x1b[34m{'─' * rule_w}\x1b[0m")
            elif wrap_enabled and visible_width(item) > width:
                out_lines.extend(wrap_vi_ansi(item, width))
            else:
                out_lines.append(re.sub(r"\x1b\[0?K", "", item))
            continue

        nm, np, code = item
        if np and not nm:
            num_str = np
            num_col = COL_NUM_ADD
            line_bg = BG_ADD_LINE
        elif nm and not np:
            num_str = nm
            num_col = COL_NUM_DEL
            line_bg = BG_DEL_LINE
        else:
            num_str = np or nm
            num_col = COL_NUM_CTX
            line_bg = ""

        first_gutter = f"{num_col}{num_str:>{num_w}}\x1b[0m{GUTTER_BAR}"
        if not wrap_enabled:
            # `delta --dark` already emits cohesive added/deleted line & word-diff backgrounds
            out_lines.append(first_gutter + format_unwrapped_code_line(code, ""))
        else:
            wrapped = wrap_vi_ansi(code, code_w, cont_prefix="", cont_prefix_cols=0, line_bg=line_bg)
            for idx, seg in enumerate(wrapped):
                out_lines.append((first_gutter if idx == 0 else cont_gutter) + seg)

    return "\n".join(out_lines) + ("\n" if out_lines else "")


def render_fallback_unified_diff(diff_text: str, width: int, wrap_enabled: bool = False) -> str:
    """Zero-dependency single-column diff renderer when `delta` is unavailable."""
    lines = diff_text.splitlines()
    old_ln = 0
    new_ln = 0
    max_num = 1
    rule_w = max(16, width if wrap_enabled else (width - 3))
    structured: list[tuple[str, str, str] | str] = []

    for ln in lines:
        plain = strip_ansi(ln)
        mh = HUNK_HDR_RE.match(plain)
        if mh:
            old_ln = int(mh.group(1))
            new_ln = int(mh.group(2))
            max_num = max(max_num, old_ln, new_ln)
            structured.append(f"\x1b[34m── hunk -{old_ln} +{new_ln}{mh.group(3)} ──\x1b[0m")
            continue
        if plain.startswith(("diff --git ", "index ", "--- ", "+++ ")):
            if plain.startswith("+++ b/"):
                structured.append(f"\x1b[1;34mΔ {plain[6:]}\x1b[0m")
                structured.append(f"\x1b[34m{'─' * rule_w}\x1b[0m")
            continue
        if old_ln == 0 and new_ln == 0:
            structured.append(ln)
            continue

        if plain.startswith("+"):
            structured.append(("", str(new_ln), f"\x1b[38;5;114m{plain[1:]}\x1b[0m"))
            max_num = max(max_num, new_ln)
            new_ln += 1
        elif plain.startswith("-"):
            structured.append((str(old_ln), "", f"\x1b[38;5;167m{plain[1:]}\x1b[0m"))
            max_num = max(max_num, old_ln)
            old_ln += 1
        elif plain.startswith(" "):
            structured.append((str(old_ln), str(new_ln), plain[1:]))
            max_num = max(max_num, new_ln)
            old_ln += 1
            new_ln += 1
        else:
            structured.append(ln)

    num_w = max(3, len(str(max_num)))
    code_w = max(12, width - (num_w + 1))
    cont_gutter = (" " * (num_w - 1)) + SHOWBREAK_GLYPH + GUTTER_BAR
    out: list[str] = []

    for item in structured:
        if isinstance(item, str):
            if wrap_enabled:
                out.extend(wrap_vi_ansi(item, width))
            else:
                out.append(item.expandtabs(4))
            continue
        nm, np, code = item
        if np and not nm:
            gutter = f"{COL_NUM_ADD}{np:>{num_w}}\x1b[0m{GUTTER_BAR}"
            bg = BG_ADD_LINE
        elif nm and not np:
            gutter = f"{COL_NUM_DEL}{nm:>{num_w}}\x1b[0m{GUTTER_BAR}"
            bg = ""
            bg = BG_DEL_LINE
        else:
            gutter = f"{COL_NUM_CTX}{(np or nm):>{num_w}}\x1b[0m{GUTTER_BAR}"
            bg = ""
        if not wrap_enabled:
            out.append(gutter + format_unwrapped_code_line(code, bg))
        else:
            for idx, seg in enumerate(wrap_vi_ansi(code, code_w, cont_prefix="", cont_prefix_cols=0, line_bg=bg)):
                out.append((gutter if idx == 0 else cont_gutter) + seg)

    return "\n".join(out) + ("\n" if out else "")


def render_diff(diff_text: str, width: int, wrap_enabled: bool = False) -> str:
    delta_bin = find_bin("delta")
    if delta_bin:
        try:
            res = subprocess.run(
                [
                    delta_bin,
                    "--dark",
                    "--paging=never",
                    "--line-numbers",
                    "--line-numbers-left-format=\x01{nm}\x02",
                    "--line-numbers-right-format={np}\x03",
                    "--width=variable",
                    "--wrap-max-lines=0",
                    "--max-line-length=0",
                ],
                input=diff_text,
                text=True,
                capture_output=True,
            )
            if res.returncode == 0 and res.stdout:
                return format_delta_compact_single_column(res.stdout, width, wrap_enabled)
        except Exception:
            pass
    return render_fallback_unified_diff(diff_text, width, wrap_enabled)


def main(argv: list[str] | None = None) -> int:
    raw = sys.stdin.read()
    if not raw:
        return 0

    args = sys.argv[1:] if argv is None else argv
    panel_w = resolve_panel_width(args)
    wrap_enabled = load_wrap_enabled(args)
    cli_width_explicit = any(a == "--width" or a.startswith("--width=") for a in args)
    rule_w = max(16, panel_w if wrap_enabled else (panel_w - 2))
    first_line_clean = strip_ansi(raw.splitlines()[0]) if raw.splitlines() else ""

    # Case 1: Commit / Reflog entry (`commit <sha>...`)
    if first_line_clean.startswith("commit "):
        lines = raw.splitlines(keepends=True)
        header_lines: list[str] = []
        msg_lines: list[str] = []
        rest_lines: list[str] = []
        state = "header"

        for line in lines:
            clean = strip_ansi(line)
            if state == "header":
                header_lines.append(line)
                if clean.strip() == "":
                    state = "msg"
            elif state == "msg":
                if clean.startswith("---") or clean.startswith("diff --git "):
                    state = "rest"
                    rest_lines.append(line)
                else:
                    if clean.startswith("    "):
                        idx = line.find("    ")
                        if idx != -1:
                            line = line[:idx] + line[idx + 4 :]
                    msg_lines.append(line)
            else:
                rest_lines.append(line)

        buf_parts: list[str] = []
        for hl in header_lines:
            stripped_hl = hl.rstrip("\r\n")
            if not strip_ansi(stripped_hl).strip():
                buf_parts.append("\n")
            elif wrap_enabled:
                for seg in wrap_vi_ansi(stripped_hl, panel_w):
                    buf_parts.append(seg + "\n")
            else:
                buf_parts.append(stripped_hl + "\n")
        msg_raw = strip_ansi("".join(msg_lines)).strip("\n")
        if msg_raw:
            buf_parts.append(run_glow(msg_raw, panel_w, wrap_enabled) + "\n")
        if rest_lines:
            if strip_ansi(rest_lines[0]).strip() == "---":
                buf_parts.append(f"\x1b[38;5;238m{'─' * rule_w}\x1b[0m\n")
                rest_lines = rest_lines[1:]
            if rest_lines:
                buf_parts.append(render_diff("".join(rest_lines), panel_w, wrap_enabled))
        sys.stdout.write(finalize_output("".join(buf_parts), panel_w, wrap_enabled, cli_width_explicit))
        sys.stdout.flush()
        return 0

    # Case 2: Pre-formatted non-diff output (e.g. `lazygit-log.py` commit graph — already sliced)
    plain_all = strip_ansi(raw)
    if not re.search(r"^(?:diff --git |--- |\+\+\+ |@@ -)", plain_all, re.MULTILINE):
        sys.stdout.write(raw)
        if not raw.endswith("\n"):
            sys.stdout.write("\n")
        sys.stdout.flush()
        return 0

    # Case 3: Standard File Diff
    buf_parts = [render_diff(raw, panel_w, wrap_enabled)]

    # Append live glow preview when diffing a Markdown file
    m = re.search(r"^\+\+\+ b/(.+\.(?:md|MD|markdown))$", raw, re.MULTILINE)
    if m:
        md_path = m.group(1).strip()
        if os.path.isfile(md_path):
            buf_parts.append(f"\n\x1b[38;5;242m── \x1b[1m\x1b[38;5;117mMD PREVIEW: {md_path}\x1b[0m\n")
            try:
                with open(md_path, "r", encoding="utf-8", errors="replace") as f:
                    buf_parts.append(run_glow(f.read(), panel_w, wrap_enabled))
            except Exception:
                pass
    sys.stdout.write(finalize_output("".join(buf_parts), panel_w, wrap_enabled, cli_width_explicit))
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
