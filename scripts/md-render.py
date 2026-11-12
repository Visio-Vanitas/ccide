#!/usr/bin/env python3
"""
Markdown Preview Renderer for Yazi & Lazygit (`cc-ide`).

Fixes two common Markdown preview defects in terminal file-tree panes:
1. Glamour's built-in `dark` stylesheet hardcodes literal `"## "`, `"### "`, `"#### "`
   prefixes on H2..H6 headings and emits per-cell trailing ANSI space padding. We pass
   `assets/glow-style.json` (`~/.config/cc-ide/glow-style.json`) so all heading levels
   (`#` through `######`) render with clean hierarchical glyphs (`▌`, `▍`, `▸`, `•`, `›`, `·`)
   and zero raw `#` markers, then strip trailing ANSI space padding.
2. Pre-normalizes ATX headings written without a space after `#` (e.g. `###标题` -> `### 标题`)
   outside fenced code blocks so CommonMark/Goldmark always parses them as headings, and
   provides a zero-dependency pure-Python ANSI renderer when `glow` is not installed.
"""
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import unicodedata

if hasattr(signal, "SIGPIPE"):
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)

ANSI_CSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
ANSI_OSC_RE = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")
TRAILING_ANSI_SPACES_RE = re.compile(r"(?:(?:\x1b\[[0-9;]*m)+[ \t]+)+(?:\x1b\[[0-9;]*m)*$")
ATX_NOSPACE_RE = re.compile(r"^(\s{0,3}#{1,6})([^\s#])")
ATX_HEADING_RE = re.compile(r"^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$")
FENCE_RE = re.compile(r"^\s{0,3}(`{3,}|~{3,})")


def strip_ansi(s: str) -> str:
    return ANSI_CSI_RE.sub("", s)


def find_glow() -> str | None:
    found = shutil.which("glow")
    if found:
        return found
    for prefix in ("/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"):
        cand = os.path.join(prefix, "glow")
        if os.path.isfile(cand) and os.access(cand, os.X_OK):
            return cand
    return None


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


def normalize_markdown_headings(md_text: str) -> str:
    """Normalizes `###Heading` -> `### Heading` outside fenced code blocks."""
    md_text = md_text.lstrip("﻿").replace("\r\n", "\n")
    out: list[str] = []
    in_fence = False
    fence_marker = ""

    for line in md_text.split("\n"):
        fm = FENCE_RE.match(line)
        if fm:
            marker = fm.group(1)
            if not in_fence:
                in_fence = True
                fence_marker = marker[0] * len(marker)
            elif line.lstrip().startswith(fence_marker):
                in_fence = False
                fence_marker = ""
            out.append(line)
            continue

        if not in_fence:
            line = ATX_NOSPACE_RE.sub(r"\1 \2", line)
        out.append(line)
    return "\n".join(out)


def clean_glow_line(line: str) -> str:
    # Strip OSC-8 hyperlinks so Yazi's CSI-only `ui.Text.parse()` never leaks escape fragments,
    # then strip Glamour's per-cell trailing ANSI space padding.
    no_osc = ANSI_OSC_RE.sub("", line)
    return TRAILING_ANSI_SPACES_RE.sub("\x1b[0m", no_osc.rstrip(" "))


def char_cols(ch: str) -> int:
    o = ord(ch)
    if o < 0x20 or 0x7F <= o < 0xA0 or unicodedata.combining(ch):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def wrap_plain(text: str, width: int) -> list[str]:
    budget = max(12, width)
    cur: list[str] = []
    cur_w = 0
    rows: list[str] = []
    for ch in text:
        cw = char_cols(ch)
        if cur_w + cw > budget and cur_w > 0:
            rows.append("".join(cur).rstrip())
            cur = [] if ch == " " else [ch]
            cur_w = 0 if ch == " " else cw
        else:
            cur.append(ch)
            cur_w += cw
    if cur or not rows:
        rows.append("".join(cur).rstrip())
    return rows


def style_inline(text: str) -> str:
    text = re.sub(r"`([^`]+)`", "\x1b[38;5;115;48;5;236m\\1\x1b[0m", text)
    text = re.sub(r"\*\*([^*]+)\*\*", "\x1b[1;38;5;255m\\1\x1b[0m", text)
    return text


def render_fallback_markdown(md_text: str, width: int, wrap_enabled: bool = False) -> str:
    """Pure-Python ANSI Markdown renderer used when `glow` is unavailable."""
    heading_styles = {
        1: ("▌ ", "\x1b[1;38;5;231;48;5;62m", " "),
        2: ("▍ ", "\x1b[1;38;5;75m", ""),
        3: ("▸ ", "\x1b[1;38;5;80m", ""),
        4: ("• ", "\x1b[1;38;5;183m", ""),
        5: ("› ", "\x1b[1;38;5;150m", ""),
        6: ("· ", "\x1b[1;38;5;246m", ""),
    }
    in_fence = False
    out: list[str] = []
    for line in md_text.split("\n"):
        if FENCE_RE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            full_code = f"\x1b[38;5;240m│\x1b[0m \x1b[38;5;115m{line.expandtabs(4)}\x1b[0m"
            out.extend(wrap_vi_ansi(full_code, width) if wrap_enabled else [full_code])
            continue
        mh = ATX_HEADING_RE.match(line)
        if mh:
            lvl = len(mh.group(1))
            pfx, col, sfx = heading_styles.get(lvl, ("• ", "\x1b[1;36m", ""))
            title = re.sub(r"[`*_]", "", mh.group(2))
            full_hdr = f"{col}{pfx}{title}{sfx}\x1b[0m"
            out.extend(wrap_vi_ansi(full_hdr, width) if wrap_enabled else [full_hdr])
            continue
        stripped = line.strip()
        if stripped in ("---", "***", "___"):
            out.append(f"\x1b[38;5;240m{'─' * min(max(16, width - 2), 40)}\x1b[0m")
            continue
        if stripped.startswith(("- ", "* ")):
            full_li = "\x1b[38;5;214m•\x1b[0m " + style_inline(stripped[2:])
            out.extend(wrap_vi_ansi(full_li, width) if wrap_enabled else [full_li])
            continue
        if not stripped:
            out.append("")
            continue
        styled_ln = style_inline(line)
        out.extend(wrap_vi_ansi(styled_ln, width) if wrap_enabled else [styled_ln])
    return "\n".join(out) + "\n"


def wrap_vi_ansi(s: str, max_cols: int) -> list[str]:
    """`vi`-style (`:set wrap nolinebreak showbreak=↪`) exact-column ANSI wrapper."""
    showbreak = "\x1b[38;5;242m↪\x1b[0m"
    cjk_pad = "\x1b[38;5;240m↩\x1b[0m"
    first_budget = max(8, max_cols)
    cont_budget = max(6, max_cols - 1)
    s = re.sub(r"\x1b\[0?K", "", s).expandtabs(4)

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
            cur_buf.append(f"\x1b[0m{cjk_pad}" if (cur_w == budget - 1 and cw == 2) else "\x1b[0m")
            segments.append("".join(cur_buf))
            cur_buf = [showbreak] + ([active_sgr] if active_sgr else [])
            cur_w = 0
            budget = cont_budget
        cur_buf.append(txt)
        cur_w += cw

    cur_buf.append("\x1b[0m")
    segments.append("".join(cur_buf))
    return segments if segments else ["\x1b[0m"]


def render_markdown(md_text: str, width: int, wrap_enabled: bool = False) -> str:
    norm = normalize_markdown_headings(md_text)
    glow_bin = find_glow()
    if glow_bin:
        style_arg = resolve_glow_style()
        env = os.environ.copy()
        env["CLICOLOR_FORCE"] = "1"
        try:
            res = subprocess.run(
                [glow_bin, "-s", style_arg, "-w", "2000", "-"],
                input=norm,
                text=True,
                capture_output=True,
                env=env,
            )
            if res.returncode == 0 and res.stdout.strip():
                cleaned = []
                for ln in res.stdout.splitlines():
                    cl = clean_glow_line(ln)
                    if cl.startswith("  "):
                        cl = cl[2:]
                    cleaned.append(cl)
                while cleaned and not strip_ansi(cleaned[0]).strip():
                    cleaned.pop(0)
                while cleaned and not strip_ansi(cleaned[-1]).strip():
                    cleaned.pop()
                if not wrap_enabled:
                    return "\n".join(cleaned) + "\n"
                wrapped: list[str] = []
                for ln in cleaned:
                    if not strip_ansi(ln).strip():
                        wrapped.append("")
                    else:
                        wrapped.extend(wrap_vi_ansi(ln, width))
                return "\n".join(wrapped) + "\n"
        except Exception:
            pass
    return render_fallback_markdown(norm, width, wrap_enabled)


def parse_args(argv: list[str]) -> tuple[int, str | None, bool]:
    width = int(os.environ.get("w", "0") or "0")
    file_path: str | None = None
    wrap_enabled = load_wrap_enabled(argv)
    i = 0
    while i < len(argv):
        arg = argv[i]
        if arg in ("--wrap", "--no-wrap"):
            pass
        elif arg.startswith("-w=") or arg.startswith("--width="):
            val = arg.split("=", 1)[1].strip()
            if val.isdigit():
                width = int(val)
        elif arg in ("-w", "--width") and i + 1 < len(argv):
            val = argv[i + 1].strip()
            if val.isdigit():
                width = int(val)
            i += 1
        elif arg == "-":
            file_path = None
        elif not arg.startswith("-"):
            file_path = arg
        i += 1
    if width <= 0:
        width = 72
    return max(20, width), file_path, wrap_enabled


def main(argv: list[str] | None = None) -> int:
    width, file_path, wrap_enabled = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if file_path:
            with open(file_path, "r", encoding="utf-8", errors="replace") as f:
                raw = f.read()
        else:
            raw = sys.stdin.read()
    except Exception as exc:
        sys.stdout.write(f"Failed to read Markdown: {exc}\n")
        return 1

    if not raw:
        return 0
    sys.stdout.write(render_markdown(raw, width, wrap_enabled))
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
