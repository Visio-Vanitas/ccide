#!/usr/bin/env python3
"""
120Hz Silky-Smooth Hover-Aware & Content-Bounds-Clamped PTY Bridge for Lazygit (`cc-ide`).

Grounded in Lazygit (`jesseduffield/lazygit`) + `gocui` (`jesseduffield/gocui`) internals:
1. Sub-0.1ms 120Hz Direct ANSI Viewport Blitter for `[0]` (`[0]-Log`, `[0]-Patch-`, `[0]-Commit`):
   - Lazygit 0.54 hardcodes `Views.Main.Wrap = true` (`ox = 0`), so `lazygit-log.py` and
     `lazygit-pager.sh` emit `pan_x = 0` sliced lines (`<= InnerWidth`) to prevent wrapping and
     cache the full unwrapped ANSI lines in `/tmp/.cc-ide/lg_full_<sid>.json`.
   - Pre-tokenizes cached ANSI lines once on `mtime` change and blits `[pan_x : pan_x + InnerWidth]`
     (`±2` cols per 120Hz Magic Mouse event, `±4` cols per key press) directly to `stdout_fd` in
     `< 0.10ms` — zero subprocesses, zero `R` refreshes, zero focus changes, and zero flicker even
     during diagonal vertical + horizontal Magic Mouse scrolling.
2. Exact Visible-Viewport Content Pre-Fill Bounds:
   - Matches `[0]`'s visible rows in `VtScreen` against the cached lines and computes `vis_max_over`
     across ONLY the currently visible rows — if visible lines fit inside `[0]`, horizontal scroll is
     locked at `0`; otherwise clamped strictly to `[0, vis_max_over]`.
3. Commits Tree Lock (`[4]-Commits`):
   - Redirects header clicks on `- Reflog` to `Commits` (tab 0), blocks `[`/`]` tab cycling on `[4]`,
     and auto-heals back to `Commits` if `[0]-Reflog Entry` ever appears.
"""
import codecs
import errno
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import threading
import time
import unicodedata

SGR_MOUSE_RE = re.compile(rb"\x1b\[<(\d+);(\d+);(\d+)([Mm])")
COMPLETE_CSI_PREFIX_RE = re.compile(rb"^\x1b(?:\[[0-9;<=>?]*[@-~]|[\(\)\*\+][A-Za-z0-9]|[78=>M])")
FOCUS_EVENT_RE = re.compile(rb"\x1b\[[IO]")
KEY_HLEFT_RE = re.compile(rb"\x1b\[1;[23]D")
KEY_HRIGHT_RE = re.compile(rb"\x1b\[1;[23]C")
PANEL_HDR_RE = re.compile(r"[╭┌]─\[([0-5])\]")
ANSI_CSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
OSC_STRIP_RE = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")


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


def write_all(fd: int, payload: bytes) -> None:
    """Writes every byte of `payload` to `fd`, retrying across POSIX/Darwin PTY short writes & EAGAIN."""
    view = memoryview(payload)
    total = len(view)
    sent = 0
    while sent < total:
        try:
            n = os.write(fd, view[sent:])
            if n <= 0:
                break
            sent += n
        except InterruptedError:
            continue
        except BlockingIOError:
            try:
                select.select([], [fd], [], 0.05)
            except Exception:
                pass
        except OSError as exc:
            if exc.errno == errno.EINTR:
                continue
            if exc.errno in (errno.EAGAIN, errno.EWOULDBLOCK):
                try:
                    select.select([], [fd], [], 0.05)
                except Exception:
                    pass
                continue
            break


def apply_sgr_seq(state: list, seq: str) -> None:
    """
    Updates canonical SGR state `[fg, bg, bold, dim, italic, underline, reverse]` from `\x1b[...m`.
    Handles compound sequences like `\x1b[48;2;0;40;0;38;2;255;255;255m`, `\x1b[39m`, `\x1b[49m`, `\x1b[0m`.
    """
    body = seq[2:-1]
    if not body:
        state[:] = ["", "", False, False, False, False, False]
        return
    parts = body.split(";")
    i = 0
    n = len(parts)
    while i < n:
        p = parts[i]
        if ":" in p:
            sub = p.split(":")
            head = int(sub[0]) if sub[0].isdigit() else -1
            if head in (38, 48) and len(sub) >= 3:
                slot = 0 if head == 38 else 1
                if sub[1] == "5" and sub[2].isdigit():
                    state[slot] = f"{head};5;{sub[2]}"
                elif (
                    sub[1] == "2"
                    and len(sub) >= 5
                    and sub[-3].isdigit()
                    and sub[-2].isdigit()
                    and sub[-1].isdigit()
                ):
                    state[slot] = f"{head};2;{sub[-3]};{sub[-2]};{sub[-1]}"
            elif head == 4:
                state[5] = len(sub) < 2 or sub[1] != "0"
            i += 1
            continue
        code = int(p) if p.isdigit() else (0 if p == "" else -1)
        if code == 0:
            state[:] = ["", "", False, False, False, False, False]
        elif code == 1:
            state[2] = True
        elif code == 2:
            state[3] = True
        elif code == 3:
            state[4] = True
        elif code == 4:
            state[5] = True
        elif code == 7:
            state[6] = True
        elif code == 22:
            state[2] = False
            state[3] = False
        elif code == 23:
            state[4] = False
        elif code == 24:
            state[5] = False
        elif code == 27:
            state[6] = False
        elif (30 <= code <= 37) or (90 <= code <= 97):
            state[0] = str(code)
        elif code == 39:
            state[0] = ""
        elif (40 <= code <= 47) or (100 <= code <= 107):
            state[1] = str(code)
        elif code == 49:
            state[1] = ""
        elif code in (38, 48) and i + 1 < n:
            slot = 0 if code == 38 else 1
            mode = parts[i + 1]
            if mode == "5" and i + 2 < n and parts[i + 2].isdigit():
                state[slot] = f"{code};5;{parts[i + 2]}"
                i += 2
            elif (
                mode == "2"
                and i + 4 < n
                and parts[i + 2].isdigit()
                and parts[i + 3].isdigit()
                and parts[i + 4].isdigit()
            ):
                state[slot] = f"{code};2;{parts[i + 2]};{parts[i + 3]};{parts[i + 4]}"
                i += 4
        i += 1


def serialize_sgr(state: list) -> str:
    codes: list[str] = []
    if state[2]:
        codes.append("1")
    if state[3]:
        codes.append("2")
    if state[4]:
        codes.append("3")
    if state[5]:
        codes.append("4")
    if state[6]:
        codes.append("7")
    if state[0]:
        codes.append(state[0])
    if state[1]:
        codes.append(state[1])
    return f"\x1b[{';'.join(codes)}m" if codes else ""


def tokenize_ansi_line(s: str) -> tuple[list[tuple[bool, str, int]], str, int]:
    """Pre-tokenizes an ANSI string once into `(tokens, plain_str, total_cols)` for `< 0.08ms` 120Hz slicing."""
    s = (
        re.sub(r"\x1b\[0?K|\x1b[\(\)\*\+][A-Za-z0-9]", "", OSC_STRIP_RE.sub("", s))
        .replace("\r", "")
        .expandtabs(4)
    )
    tokens: list[tuple[bool, str, int]] = []
    plain_chars: list[str] = []
    total_w = 0
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
        cw = char_cols(ch)
        tokens.append((False, ch, cw))
        if cw > 0:
            plain_chars.append(ch)
            total_w += cw
        i += 1
    return tokens, "".join(plain_chars), total_w


def clip_plain_cols(plain: str, max_cols: int) -> str:
    out: list[str] = []
    w = 0
    for ch in plain:
        cw = char_cols(ch)
        if w + cw > max_cols:
            break
        out.append(ch)
        w += cw
    return "".join(out).rstrip()


def slice_tokens_padded(tokens: list[tuple[bool, str, int]], offset_cols: int, width_cols: int) -> str:
    """Slices pre-tokenized ANSI line to `[offset_cols : offset_cols + width_cols]`, padded to exact `width_cols`."""
    if width_cols <= 0:
        return ""
    sgr_state: list = ["", "", False, False, False, False, False]
    pos_col = 0
    out: list[str] = []
    out_w = 0
    started = False
    # When panned right (`offset_cols > 0`), cell 0 displays `‹` in place of `offset_cols` so columns align cleanly
    start_col = (offset_cols + 1) if offset_cols > 0 else 0
    indicator_bg = ""

    for is_ansi, txt, cw in tokens:
        if is_ansi:
            apply_sgr_seq(sgr_state, txt)
            if started:
                out.append(txt)
            continue

        if offset_cols > 0 and not indicator_bg and pos_col + cw > offset_cols:
            indicator_bg = sgr_state[1]

        if pos_col + cw <= start_col:
            pos_col += cw
            continue

        if not started:
            started = True
            if offset_cols > 0:
                bg_part = f";{indicator_bg or sgr_state[1]}" if (indicator_bg or sgr_state[1]) else ""
                out.append(f"\x1b[0;38;5;242{bg_part}m‹\x1b[0m")
                out_w = 1
                if out_w >= width_cols:
                    break
            active_seq = serialize_sgr(sgr_state)
            if active_seq:
                out.append(active_seq)
            if pos_col < start_col and cw == 2:
                if out_w + 1 <= width_cols:
                    out.append(" ")
                    out_w += 1
                pos_col += cw
                continue

        if out_w + cw > width_cols:
            break
        out.append(txt)
        out_w += cw
        pos_col += cw

    out.append("\x1b[0m")
    if out_w < width_cols:
        out.append(" " * (width_cols - out_w))
    return "".join(out)


class VtFrameGate:
    """
    Byte-level ECMA-48 + UTF-8 frame boundary gate across 1024-byte Darwin/POSIX PTY reads.

    Never emits incomplete UTF-8 multi-byte codepoints or unfinished ESC/CSI/OSC/DCS/charset
    sequences (`\\x1b]8;;\\x1b\\\\`, `\\x1b[38;2;...m`, `\\x1b(B`, `\\x1b[row;colH`), preventing
    120Hz `[0]` overlays from ever tearing Lazygit/`tcell` escape sequences mid-flight.
    """

    def __init__(self) -> None:
        self.tail = b""

    def has_pending(self) -> bool:
        return bool(self.tail)

    def push(self, chunk: bytes) -> bytes:
        if not chunk:
            return b""
        buf = (self.tail + chunk) if self.tail else chunk
        n = len(buf)
        # Immediately after ST (`\x1b\\`), ECMA-48 is unconditionally in GROUND state with 0 UTF-8 bytes pending.
        scan_from = 0
        if n > 256:
            st_pos = buf.rfind(b"\x1b\\", 0, n - 128)
            if st_pos != -1:
                scan_from = st_pos + 2

        # States: 0=ground, 1=utf8, 2=esc, 3=esc_inter, 4=csi, 5=osc_dcs, 6=osc_esc
        state = 0
        utf8_rem = 0
        safe_end = scan_from
        i = scan_from
        while i < n:
            b = buf[i]
            if state == 0:
                if b == 0x1B:
                    state = 2
                elif b < 0x80:
                    safe_end = i + 1
                elif 0xC2 <= b <= 0xDF:
                    state = 1
                    utf8_rem = 1
                elif 0xE0 <= b <= 0xEF:
                    state = 1
                    utf8_rem = 2
                elif 0xF0 <= b <= 0xF4:
                    state = 1
                    utf8_rem = 3
                else:
                    safe_end = i + 1
            elif state == 1:
                if 0x80 <= b <= 0xBF:
                    utf8_rem -= 1
                    if utf8_rem == 0:
                        state = 0
                        safe_end = i + 1
                else:
                    state = 0
                    continue
            elif state == 2:
                if b == 0x5B:  # '[' -> CSI
                    state = 4
                elif b in (0x5D, 0x50, 0x5F, 0x5E, 0x58):  # ']', 'P', '_', '^', 'X' -> OSC/DCS
                    state = 5
                elif 0x20 <= b <= 0x2F:  # '(', ')', '*', '+', '#', ' ' -> intermediate + final
                    state = 3
                elif 0x30 <= b <= 0x7E:  # 2-byte ESC ('7', '8', '=', '>', 'M', 'c')
                    state = 0
                    safe_end = i + 1
                else:
                    state = 0
                    safe_end = i + 1
            elif state == 3:
                if 0x20 <= b <= 0x2F:
                    pass
                else:
                    state = 0
                    safe_end = i + 1
            elif state == 4:
                if 0x40 <= b <= 0x7E:
                    state = 0
                    safe_end = i + 1
                elif b == 0x1B:
                    state = 2
            elif state == 5:
                if b == 0x07:
                    state = 0
                    safe_end = i + 1
                elif b == 0x1B:
                    state = 6
            elif state == 6:
                if b == 0x5C:  # '\\' -> ST
                    state = 0
                    safe_end = i + 1
                elif b == 0x5B:
                    state = 4
                elif b in (0x5D, 0x50, 0x5F, 0x5E, 0x58):
                    state = 5
                else:
                    state = 0
                    safe_end = i + 1
            i += 1

        if n - safe_end > 4096:
            self.tail = b""
            return buf
        self.tail = buf[safe_end:]
        return buf[:safe_end]


class VtScreen:
    """Lightweight VT cell tracker + 120Hz in-memory `[0]` viewport blitter."""

    def __init__(self, rows: int, cols: int) -> None:
        self.rows = max(4, rows)
        self.cols = max(20, cols)
        self.grid: list[list[str]] = [[" "] * self.cols for _ in range(self.rows)]
        self.cell_touched = bytearray(self.rows * self.cols)
        self.r = 0
        self.c = 0
        self.saved_r = 0
        self.saved_c = 0
        self.cursor_sgr: list = ["", "", False, False, False, False, False]
        self._utf8_dec = codecs.getincrementaldecoder("utf-8")(errors="replace")
        self._vt_state = "ground"
        self._csi_buf = ""
        self.frame_gate = VtFrameGate()
        self.stdin_tail = b""
        self.stdin_tail_ts = 0.0
        self.focused_panel = "1"
        self.pan_x: dict[str, int] = {str(i): 0 for i in range(6)}
        self.main_dirty_on_term = False
        self._saved_pan0: tuple[tuple[int, int, int, int], int, list[list[tuple[bool, str, int]]]] | None = None
        self.pending_pan_flush = False
        self.main_inner_w = 0
        self.sid = 0
        self._last_reflog_heal = 0.0

        # Cached pre-tokenized `[0]` lines from `lg_full_<sid>.json`
        self.cache_mtime = -1.0
        self.cache_digest = ""
        self.cached_panel_w = 0
        self.tok_lines: list[list[tuple[bool, str, int]]] = []
        self.plain_lines: list[str] = []
        self.line_widths: list[int] = []
        self.plain_clips_mw = 0
        self.plain_clips: list[str] = []
        self.matched_top_idx: int | None = None

    def set_full_lines(self, lines: list[str], panel_w: int = 0) -> None:
        toks: list[list[tuple[bool, str, int]]] = []
        plains: list[str] = []
        widths: list[int] = []
        for ln in lines:
            tk, pl, tw = tokenize_ansi_line(ln)
            toks.append(tk)
            plains.append(pl)
            widths.append(tw)
        self.cached_panel_w = panel_w
        self.tok_lines = toks
        self.plain_lines = plains
        self.line_widths = widths
        self.plain_clips_mw = 0
        self.plain_clips = []
        self.matched_top_idx = None

    def load_cache_if_updated(self) -> bool:
        if self.sid <= 0:
            return False
        p = f"/tmp/.cc-ide/lg_full_{self.sid}.json"
        try:
            st = os.stat(p)
        except OSError:
            return False
        if st.st_mtime == self.cache_mtime:
            return False
        try:
            with open(p, "r", encoding="utf-8") as f:
                payload = json.load(f)
            if not isinstance(payload, dict):
                return False
            digest = str(payload.get("digest") or "")
            if digest and digest == self.cache_digest:
                self.cache_mtime = st.st_mtime
                return False
            lines = payload.get("lines")
            pw = int(payload.get("panel_w") or 0)
            if isinstance(lines, list):
                prev_hdr = self.plain_lines[:2]
                prev_pan = self.pan_x.get("0", 0)
                self.cache_mtime = st.st_mtime
                self.cache_digest = digest
                self.set_full_lines([str(x) for x in lines], pw)
                if prev_hdr and self.plain_lines[:2] == prev_hdr:
                    self.pan_x["0"] = prev_pan
                else:
                    self.pan_x["0"] = 0
                return True
        except Exception:
            pass
        return False

    def resize(self, rows: int, cols: int) -> None:
        rows = max(4, rows)
        cols = max(20, cols)
        if rows == self.rows and cols == self.cols:
            return
        self.rows = rows
        self.cols = cols
        self.grid = [[" "] * self.cols for _ in range(self.rows)]
        self.cell_touched = bytearray(self.rows * self.cols)
        self.r = min(self.r, self.rows - 1)
        self.c = min(self.c, self.cols - 1)
        self.matched_top_idx = None

    def clear_touched(self) -> None:
        n = len(self.cell_touched)
        self.cell_touched[:] = b"\x00" * n

    def feed(self, raw: bytes) -> None:
        """Incremental UTF-8 + VT100/xterm/OSC-8 state machine safe across arbitrary PTY read boundaries."""
        s = self._utf8_dec.decode(raw, final=False)
        rows = self.rows
        cols = self.cols
        grid = self.grid
        touched = self.cell_touched
        r = self.r
        c = self.c
        state = self._vt_state
        csi_buf = self._csi_buf

        for ch in s:
            if state == "ground":
                if ch == "\x1b":
                    state = "esc"
                elif ch == "\r":
                    c = 0
                elif ch == "\n":
                    r = min(rows - 1, r + 1)
                elif ch == "\x08":
                    c = max(0, c - 1)
                elif ord(ch) >= 32:
                    cw = char_cols(ch)
                    if cw > 0 and 0 <= r < rows and 0 <= c < cols:
                        row_ref = grid[r]
                        row_base = r * cols
                        if c > 0 and row_ref[c] == "":
                            row_ref[c - 1] = " "
                            touched[row_base + c - 1] = 1
                        row_ref[c] = ch
                        touched[row_base + c] = 1
                        if cw == 2 and c + 1 < cols:
                            row_ref[c + 1] = ""
                            touched[row_base + c + 1] = 1
                        elif cw == 1 and c + 1 < cols and row_ref[c + 1] == "":
                            row_ref[c + 1] = " "
                            touched[row_base + c + 1] = 1
                        c += cw
            elif state == "esc":
                if ch == "[":
                    state = "csi"
                    csi_buf = ""
                elif ch == "]":
                    state = "osc"
                elif ch in ("(", ")", "*", "+"):
                    state = "charset"
                elif ch in ("P", "_", "^", "X"):
                    state = "dcs"
                elif ch == "7":
                    self.saved_r = r
                    self.saved_c = c
                    state = "ground"
                elif ch == "8":
                    r = min(rows - 1, max(0, self.saved_r))
                    c = min(cols - 1, max(0, self.saved_c))
                    state = "ground"
                else:
                    state = "ground"
            elif state == "csi":
                if "@" <= ch <= "~":
                    seq = csi_buf
                    cmd = ch
                    state = "ground"
                    csi_buf = ""
                    if cmd == "m":
                        apply_sgr_seq(self.cursor_sgr, f"\x1b[{seq}m")
                    elif cmd in ("H", "f"):
                        parts = seq.split(";")
                        r = max(0, min(rows - 1, (int(parts[0]) if parts[0].isdigit() else 1) - 1))
                        c = max(
                            0,
                            min(
                                cols - 1,
                                (int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 1) - 1,
                            ),
                        )
                    elif cmd in ("G", "`"):
                        c = max(0, min(cols - 1, (int(seq) if seq.isdigit() else 1) - 1))
                    elif cmd == "d":
                        r = max(0, min(rows - 1, (int(seq) if seq.isdigit() else 1) - 1))
                    elif cmd == "J":
                        if seq in ("", "0"):
                            if 0 <= r < rows:
                                rb = r * cols
                                for cx in range(max(0, c), cols):
                                    grid[r][cx] = " "
                                    touched[rb + cx] = 1
                            for ry in range(r + 1, rows):
                                grid[ry] = [" "] * cols
                                touched[ry * cols : (ry + 1) * cols] = b"\x01" * cols
                        elif seq == "1":
                            for ry in range(0, r):
                                grid[ry] = [" "] * cols
                                touched[ry * cols : (ry + 1) * cols] = b"\x01" * cols
                            if 0 <= r < rows:
                                rb = r * cols
                                for cx in range(0, min(cols, c + 1)):
                                    grid[r][cx] = " "
                                    touched[rb + cx] = 1
                        elif seq in ("2", "3"):
                            for ry in range(rows):
                                grid[ry] = [" "] * cols
                            touched[:] = b"\x01" * (rows * cols)
                    elif cmd == "K":
                        if 0 <= r < rows:
                            row_ref = grid[r]
                            rb = r * cols
                            if seq in ("", "0"):
                                for cx in range(max(0, c), cols):
                                    row_ref[cx] = " "
                                    touched[rb + cx] = 1
                            elif seq == "1":
                                for cx in range(0, min(cols, c + 1)):
                                    row_ref[cx] = " "
                                    touched[rb + cx] = 1
                            elif seq == "2":
                                for cx in range(cols):
                                    row_ref[cx] = " "
                                    touched[rb + cx] = 1
                    elif cmd == "X":
                        if 0 <= r < rows:
                            step = int(seq) if seq.isdigit() else 1
                            row_ref = grid[r]
                            rb = r * cols
                            for cx in range(max(0, c), min(cols, c + step)):
                                row_ref[cx] = " "
                                touched[rb + cx] = 1
                    elif cmd in ("C", "a"):
                        step = int(seq) if seq.isdigit() else 1
                        c = min(cols - 1, c + step)
                    elif cmd == "D":
                        step = int(seq) if seq.isdigit() else 1
                        c = max(0, c - step)
                    elif cmd == "A":
                        step = int(seq) if seq.isdigit() else 1
                        r = max(0, r - step)
                    elif cmd in ("B", "e"):
                        step = int(seq) if seq.isdigit() else 1
                        r = min(rows - 1, r + step)
                    elif cmd == "E":
                        step = int(seq) if seq.isdigit() else 1
                        r = min(rows - 1, r + step)
                        c = 0
                    elif cmd == "F":
                        step = int(seq) if seq.isdigit() else 1
                        r = max(0, r - step)
                        c = 0
                else:
                    if len(csi_buf) < 64:
                        csi_buf += ch
                    else:
                        state = "ground"
                        csi_buf = ""
            elif state == "osc":
                if ch == "\x07":
                    state = "ground"
                elif ch == "\x1b":
                    state = "osc_esc"
            elif state == "osc_esc":
                if ch == "\\":
                    state = "ground"
                elif ch == "[":
                    state = "csi"
                    csi_buf = ""
                elif ch == "]":
                    state = "osc"
                else:
                    state = "ground"
            elif state == "charset":
                state = "ground"
            elif state == "dcs":
                if ch == "\x07":
                    state = "ground"
                elif ch == "\x1b":
                    state = "dcs_esc"
            elif state == "dcs_esc":
                state = "ground" if ch == "\\" else "dcs"

        self.r = r
        self.c = c
        self._vt_state = state
        self._csi_buf = csi_buf

    def find_boxes(self) -> dict[str, tuple[int, int, int, int]]:
        """Returns `{panel_id: (x0, y0, x1, y1)}` for all visible Lazygit boxes."""
        boxes: dict[str, tuple[int, int, int, int]] = {}
        for y0 in range(self.rows - 2):
            line_str = "".join(ch if ch != "" else " " for ch in self.grid[y0])
            for m in PANEL_HDR_RE.finditer(line_str):
                pid = m.group(1)
                x0 = m.start()
                x1 = -1
                for cx in range(x0 + 4, self.cols):
                    if self.grid[y0][cx] in ("╮", "┐"):
                        x1 = cx
                        break
                if x1 == -1:
                    continue
                y1 = -1
                for ry in range(y0 + 2, self.rows):
                    if self.grid[ry][x0] in ("╰", "└"):
                        y1 = ry
                        break
                if y1 == -1:
                    y1 = self.rows - 1
                boxes[pid] = (x0, y0, x1, y1)
        return boxes

    def panel_at(self, col_1based: int, row_1based: int) -> tuple[str | None, tuple[int, int, int, int] | None]:
        mx = col_1based - 1
        my = row_1based - 1
        boxes = self.find_boxes()
        for pid, rect in boxes.items():
            x0, y0, x1, y1 = rect
            if x0 <= mx <= x1 and y0 <= my <= y1:
                return pid, rect
        return None, None

    def render_unfocused_border_dim(self) -> bytes:
        """Repaints the focused Lazygit panel's active green border & blue selection bar in neutral inactive style on `FocusOut` (`\\x1b[O`) while preserving `[0]`'s syntax-highlighted diff."""
        boxes = self.find_boxes()
        rect = boxes.get(self.focused_panel)
        if rect is None:
            return b""
        x0, y0, x1, y1 = rect
        end_y = min(self.rows - 1, y1)
        end_x = min(self.cols - 1, x1)
        if x0 < 0 or y0 < 0 or end_x <= x0 or end_y <= y0:
            return b""
        self._dimmed_unfocused = True
        out: list[str] = ["\x1b7\x1b[0m"]

        def _slice_row(row: list[str]) -> str:
            parts: list[str] = []
            for x in range(x0, end_x + 1):
                ch = row[x]
                if x == x0 and ch == "":
                    parts.append(" ")
                elif x == end_x and ch != "" and char_cols(ch) == 2:
                    parts.append(" ")
                else:
                    parts.append(ch)
            return "".join(parts)

        top_s = _slice_row(self.grid[y0])
        out.append(f"\x1b[{y0 + 1};{x0 + 1}H{top_s}")
        bot_s = _slice_row(self.grid[end_y])
        out.append(f"\x1b[{end_y + 1};{x0 + 1}H{bot_s}")
        repaint_interior = self.focused_panel != "0"
        for ry in range(y0 + 1, end_y):
            if repaint_interior:
                full_s = _slice_row(self.grid[ry])
                out.append(f"\x1b[{ry + 1};{x0 + 1}H{full_s}")
            else:
                lc = self.grid[ry][x0] or "│"
                rc = self.grid[ry][end_x] or "│"
                out.append(f"\x1b[{ry + 1};{x0 + 1}H{lc}\x1b[{ry + 1};{end_x + 1}H{rc}")
        out.append("\x1b8")
        return "".join(out).encode("utf-8", "replace")

    def has_popup_over_main(self, b0: tuple[int, int, int, int]) -> bool:
        """Detects Lazygit modal popups (`╭─...─╮` + `│` vertical borders) without false-positiving on `delta` `───┐`."""
        x0, y0, x1, y1 = b0
        end_y = min(self.rows - 1, y1)
        missing_left_border = 0
        for ry in range(y0 + 1, end_y):
            if self.grid[ry][x0] not in ("│", "├", "┤"):
                missing_left_border += 1
                if missing_left_border >= 3:
                    return True
            row_str = "".join(ch if ch != "" else " " for ch in self.grid[ry][x0 + 1 : x1])
            p0 = row_str.find("╭─")
            if p0 != -1:
                p1 = row_str.find("╮", p0 + 3)
                if p1 != -1 and self.grid[ry + 1][x0 + 1 + p0] == "│":
                    return True
        return False

    def match_visible_main(self, b0: tuple[int, int, int, int]) -> int | None:
        """Finds the exact top line index `top_idx` in `self.tok_lines` currently displayed in `[0]`."""
        self.load_cache_if_updated()
        if not self.tok_lines or self.has_popup_over_main(b0):
            self.matched_top_idx = None
            return None

        x0, y0, x1, y1 = b0
        mw = max(0, x1 - x0 - 1)
        vis_end_y = min(self.rows, y1)
        vis_h = max(0, vis_end_y - (y0 + 1))
        if mw < 8 or vis_h <= 0:
            self.matched_top_idx = None
            return None

        eff_w = min(mw, self.cached_panel_w) if self.cached_panel_w > 0 else mw
        cmp_w = max(8, eff_w - 2)
        if self.plain_clips_mw != cmp_w:
            self.plain_clips = [clip_plain_cols(pl, cmp_w) for pl in self.plain_lines]
            self.plain_clips_mw = cmp_w

        grid_rows = [
            clip_plain_cols("".join(ch if ch != "" else " " for ch in self.grid[ry][x0 + 1 : x1]).rstrip(), cmp_w)
            for ry in range(y0 + 1, vis_end_y)
        ]
        if not any(grid_rows):
            self.matched_top_idx = None
            return None

        n_lines = len(self.plain_clips)
        clips = self.plain_clips

        def matches_at(idx: int) -> bool:
            if idx < 0 or idx >= n_lines:
                return False
            mismatches = 0
            checked = 0
            for k in range(vis_h):
                exp = clips[idx + k] if (idx + k) < n_lines else ""
                got = grid_rows[k]
                if not exp and not got:
                    continue
                checked += 1
                if got != exp:
                    mismatches += 1
                    if mismatches > max(1, checked // 4):
                        return False
            return checked > 0 and mismatches <= max(1, checked // 5)

        if self.matched_top_idx is not None:
            base = self.matched_top_idx
            for delta in (0, 2, -2, 1, -1, 3, -3, 4, -4):
                cand = base + delta
                if matches_at(cand):
                    self.matched_top_idx = cand
                    return cand

        # Find first non-empty visible row to anchor the search
        anchor_k = 0
        for k, gr in enumerate(grid_rows):
            if gr:
                anchor_k = k
                break
        if n_lines <= anchor_k:
            self.matched_top_idx = None
            return None
        anchor_txt = grid_rows[anchor_k]
        for idx in range(0, n_lines - anchor_k):
            if clips[idx + anchor_k] == anchor_txt and matches_at(idx):
                self.matched_top_idx = idx
                return idx

        self.matched_top_idx = None
        return None

    def visible_main_max_overflow(self, b0: tuple[int, int, int, int], top_idx: int) -> int:
        x0, y0, x1, y1 = b0
        mw = max(0, x1 - x0 - 1)
        vis_h = max(0, min(self.rows, y1) - (y0 + 1))
        end_idx = min(len(self.line_widths), top_idx + vis_h)
        max_w = max(self.line_widths[top_idx:end_idx], default=0)
        return max(0, max_w - mw)

    def render_main_pan_ansi(self, b0: tuple[int, int, int, int], pan_x: int) -> bytes:
        top_idx = self.matched_top_idx
        if top_idx is None:
            return b""
        x0, y0, x1, y1 = b0
        mw = max(0, x1 - x0 - 1)
        vis_end_y = min(self.rows, y1)
        if mw <= 0 or vis_end_y <= y0 + 1:
            return b""

        n_lines = len(self.tok_lines)
        col_1b = x0 + 2
        restore_sgr = serialize_sgr(self.cursor_sgr)
        # DEC Mode 2026 synchronized output + clean G0/OSC-8/TrueColor save & restore
        chunks: list[str] = ["\x1b[?2026h\x1b7\x1b(B\x1b[0m\x1b]8;;\x1b\\"]
        for k, ry in enumerate(range(y0 + 1, vis_end_y)):
            src_idx = top_idx + k
            row_1b = ry + 1
            if src_idx < n_lines:
                row_txt = slice_tokens_padded(self.tok_lines[src_idx], pan_x, mw)
            else:
                row_txt = " " * mw
            chunks.append(f"\x1b[{row_1b};{col_1b}H{row_txt}")
        chunks.append(f"\x1b[0m\x1b8{restore_sgr}\x1b[?2026l")
        if pan_x > 0:
            self.main_dirty_on_term = True
            self._saved_pan0 = (b0, top_idx, self.tok_lines)
        else:
            self.main_dirty_on_term = False
            self._saved_pan0 = None
        return "".join(chunks).encode("utf-8")

    def _render_saved_pan0_restore(
        self,
        saved: tuple[tuple[int, int, int, int], int, list[list[tuple[bool, str, int]]]],
        pre_sgr: list,
    ) -> bytes:
        """
        Reconstructs `[0]`'s exact `pan_x = 0` cells prior to applying a `tcell.Show()` differential
        stream so `State_A(pan_x=0) + Delta(State_A -> State_B) == State_B` holds for every cell.
        """
        b0, top_idx, tok_lines = saved
        x0, y0, x1, y1 = b0
        mw = max(0, x1 - x0 - 1)
        vis_end_y = min(self.rows, y1)
        if mw <= 0 or vis_end_y <= y0 + 1:
            return b""
        n_lines = len(tok_lines)
        col_1b = x0 + 2
        restore_sgr = serialize_sgr(pre_sgr)
        chunks: list[str] = ["\x1b[?2026h\x1b7\x1b(B\x1b[0m\x1b]8;;\x1b\\"]
        for k, ry in enumerate(range(y0 + 1, vis_end_y)):
            src_idx = top_idx + k
            row_1b = ry + 1
            if src_idx < n_lines:
                row_txt = slice_tokens_padded(tok_lines[src_idx], 0, mw)
            else:
                row_txt = " " * mw
            chunks.append(f"\x1b[{row_1b};{col_1b}H{row_txt}")
        chunks.append(f"\x1b[0m\x1b8{restore_sgr}\x1b[?2026l")
        return "".join(chunks).encode("utf-8")

    def render_untouched_blank_scrub(self, b0: tuple[int, int, int, int]) -> bytes:
        """
        Scrubs ghost `[0]` cells (`pan_x > 0` characters & diff backgrounds) that `tcell.Show()`
        skipped writing because `tcell`'s internal front-buffer already held `' '` at `pan_x = 0`.
        """
        x0, y0, x1, y1 = b0
        end_x = min(self.cols, x1)
        vis_end_y = min(self.rows, y1)
        if end_x <= x0 + 1 or vis_end_y <= y0 + 1:
            self.main_dirty_on_term = False
            return b""

        cols = self.cols
        touched = self.cell_touched
        ops: list[str] = []
        for ry in range(y0 + 1, vis_end_y):
            row_ref = self.grid[ry]
            rb = ry * cols
            cx = x0 + 1
            while cx < end_x:
                if touched[rb + cx] == 0 and row_ref[cx] == " ":
                    run_start = cx
                    cx += 1
                    while cx < end_x and touched[rb + cx] == 0 and row_ref[cx] == " ":
                        cx += 1
                    ops.append(f"\x1b[{ry + 1};{run_start + 1}H{' ' * (cx - run_start)}")
                else:
                    cx += 1

        self.main_dirty_on_term = False
        if not ops:
            return b""
        restore_sgr = serialize_sgr(self.cursor_sgr)
        return (
            "\x1b[?2026h\x1b7\x1b(B\x1b[0m\x1b]8;;\x1b\\"
            + "".join(ops)
            + f"\x1b[0m\x1b8{restore_sgr}\x1b[?2026l"
        ).encode("utf-8")

    def process_pty_chunk(
        self,
        raw: bytes,
        applied_rows: int = 0,
        applied_cols: int = 0,
        master_write_cb=None,
    ) -> bytes:
        """
        Feeds raw PTY bytes through `VtFrameGate`, updates `VtScreen`, and appends `[0]` horizontal
        pan overlays or ghost-cell scrubs ONLY at clean complete-frame boundaries.
        """
        complete = self.frame_gate.push(raw)
        if not complete:
            return b""

        was_dirty = self.main_dirty_on_term
        saved_pan0 = self._saved_pan0
        pre_sgr = list(self.cursor_sgr)

        self.clear_touched()
        self.feed(complete)

        # If `tcell` is still mid-escape-sequence across a 1024-byte PTY boundary, defer overlay
        # until the remaining bytes of the sequence arrive so we never interleave mid-frame.
        if self.frame_gate.has_pending():
            self.pending_pan_flush = True
            if was_dirty and saved_pan0 is not None:
                pre_restore = self._render_saved_pan0_restore(saved_pan0, pre_sgr)
                self.main_dirty_on_term = False
                self._saved_pan0 = None
                return pre_restore + complete
            return complete

        self.pending_pan_flush = False
        boxes_now = self.find_boxes()
        b0 = boxes_now.get("0")
        pre_restore = b""
        overlay = b""
        if b0 is not None:
            mw = max(0, b0[2] - b0[0] - 1)
            if mw > 0 and mw != self.main_inner_w:
                self.main_inner_w = mw
                if self.sid > 0 and applied_rows > 0 and applied_cols > 0:
                    write_geom_file(self.sid, applied_rows, applied_cols, mw)
            hdr0 = "".join(
                ch if ch != "" else " " for ch in self.grid[b0[1]][b0[0] : min(self.cols, b0[2])]
            )
            if "Reflog Entry" in hdr0 and "4" in boxes_now and master_write_cb is not None:
                now_t = time.monotonic()
                if now_t - self._last_reflog_heal > 0.4:
                    self._last_reflog_heal = now_t
                    b4 = boxes_now["4"]
                    c4 = b4[0] + 6
                    r4 = b4[1] + 1
                    master_write_cb(f"\x1b[<0;{c4};{r4}M\x1b[<0;{c4};{r4}m".encode("ascii"))

            top_idx = self.match_visible_main(b0)
            cur_pan = self.pan_x.get("0", 0)
            if top_idx is not None and cur_pan > 0:
                max_over = self.visible_main_max_overflow(b0, top_idx)
                next_pan = min(cur_pan, max_over)
                self.pan_x["0"] = next_pan
                if was_dirty and saved_pan0 is not None and (next_pan == 0 or saved_pan0[0] != b0):
                    pre_restore = self._render_saved_pan0_restore(saved_pan0, pre_sgr)
                overlay = self.render_main_pan_ansi(b0, next_pan)
            elif was_dirty:
                if saved_pan0 is not None:
                    pre_restore = self._render_saved_pan0_restore(saved_pan0, pre_sgr)
                if top_idx is not None:
                    overlay = self.render_main_pan_ansi(b0, 0)
                else:
                    overlay = self.render_untouched_blank_scrub(b0)
                self.main_dirty_on_term = False
                self._saved_pan0 = None
        elif was_dirty and saved_pan0 is not None:
            pre_restore = self._render_saved_pan0_restore(saved_pan0, pre_sgr)
            self.main_dirty_on_term = False
            self._saved_pan0 = None

        return pre_restore + complete + overlay

    def can_scroll_right(self, pid: str, rect: tuple[int, int, int, int]) -> bool:
        if pid == "1":
            return False
        x0, y0, x1, y1 = rect
        edge_x = x1 - 1
        if edge_x <= x0 + 1:
            return False
        check_second = self.pan_x.get(pid, 0) > 0 and (edge_x - 1) > (x0 + 2)
        for ry in range(y0 + 1, min(self.rows, y1)):
            if self.grid[ry][edge_x] not in (" ", ""):
                return True
            if check_second and self.grid[ry][edge_x - 1] not in (" ", ""):
                return True
        return False


def get_winsize_packed(fd: int) -> bytes:
    try:
        packed = fcntl.ioctl(fd, termios.TIOCGWINSZ, b"\x00" * 8)
        rows, cols, _, _ = struct.unpack("HHHH", packed)
        if rows > 0 and cols > 0:
            return packed
    except OSError:
        pass
    return struct.pack("HHHH", 32, 81, 0, 0)


def write_geom_file(sid: int, rows: int, cols: int, main_inner_w: int = 0) -> None:
    try:
        os.makedirs("/tmp/.cc-ide", exist_ok=True)
        tmp = f"/tmp/.cc-ide/lg_geom_{sid}.txt.tmp"
        dst = f"/tmp/.cc-ide/lg_geom_{sid}.txt"
        with open(tmp, "w", encoding="utf-8") as f:
            if main_inner_w > 0:
                f.write(f"{rows} {cols} {main_inner_w}\n")
            else:
                f.write(f"{rows} {cols}\n")
        os.replace(tmp, dst)
    except OSError:
        pass


def restore_main_pan_zero(screen: VtScreen, direct_out: list[bytes]) -> None:
    """Restores `[0]` terminal cells back to `pan_x = 0` before Lazygit draws popups or changes selection."""
    if screen.pan_x.get("0", 0) > 0:
        screen.pan_x["0"] = 0
        if screen.frame_gate.has_pending():
            screen.pending_pan_flush = True
            return
        b0 = screen.find_boxes().get("0")
        if b0 is not None and screen.matched_top_idx is not None:
            ansi = screen.render_main_pan_ansi(b0, 0)
            if ansi:
                direct_out.append(ansi)


def emit_hscroll_for_panel(
    screen: VtScreen,
    pid: str,
    rect: tuple[int, int, int, int],
    dir_x: int,
    step_cols: int,
    direct_out: list[bytes],
) -> bytes:
    if pid == "1":
        return b""

    if pid == "0":
        top_idx = screen.match_visible_main(rect)
        if top_idx is not None:
            max_over = screen.visible_main_max_overflow(rect, top_idx)
            cur_pan = screen.pan_x.get("0", 0)
            if dir_x > 0 and max_over <= 0:
                return b""
            delta = step_cols if dir_x > 0 else -step_cols
            next_pan = max(0, min(max_over, cur_pan + delta))
            if next_pan == cur_pan:
                return b""
            screen.pan_x["0"] = next_pan
            if screen.frame_gate.has_pending():
                screen.pending_pan_flush = True
                return b""
            ansi = screen.render_main_pan_ansi(rect, next_pan)
            if ansi:
                direct_out.append(ansi)
            return b""
        # Never send `"0L"` to hijack focus when hovering `[0]`; only pan interactive Staging if already focused on `0`
        if screen.focused_panel != "0":
            return b""

    # Side panels `[2]..[5]` (or interactive Staging view when focused on `0`):
    # Never switch Lazygit's focused panel on unfocused hover-swipes (which would clobber `[0]`'s diff!).
    if screen.focused_panel != pid:
        return b""
    if dir_x > 0:
        if not screen.can_scroll_right(pid, rect):
            return b""
        screen.pan_x[pid] = screen.pan_x.get(pid, 0) + 1
        return b"L"

    cur = screen.pan_x.get(pid, 0)
    if cur <= 0:
        return b""
    screen.pan_x[pid] = max(0, cur - 1)
    return b"H"


def translate_key_chunk(buf: bytes, screen: VtScreen, direct_out: list[bytes]) -> bytes:
    """Translates non-mouse keyboard input (`Shift/Option + Left/Right`, `H`/`L`, panel digits)."""
    if not buf:
        return b""

    def clamp_key_dir(dir_x: int) -> bytes:
        boxes = screen.find_boxes()
        target_pid = "0" if screen.focused_panel == "1" else screen.focused_panel
        rect = boxes.get(target_pid)
        if rect is None:
            return b""
        return emit_hscroll_for_panel(screen, target_pid, rect, dir_x, 4, direct_out)

    if b"\x1b" in buf:
        if KEY_HLEFT_RE.search(buf):
            return KEY_HLEFT_RE.sub(lambda _: clamp_key_dir(-1), buf)
        if KEY_HRIGHT_RE.search(buf):
            return KEY_HRIGHT_RE.sub(lambda _: clamp_key_dir(1), buf)
        if b"\x1b[O" in buf:
            dim_ansi = screen.render_unfocused_border_dim()
            if dim_ansi:
                direct_out.append(dim_ansi)
        if FOCUS_EVENT_RE.sub(b"", buf):
            restore_main_pan_zero(screen, direct_out)
        return buf

    if len(buf) == 1:
        # Lock panel `[4]` to the visual commit graph tree (`Commits`), blocking `[` / `]` switch to `Reflog`
        if screen.focused_panel == "4" and buf in (b"[", b"]"):
            return b""
        if buf == b"L":
            return clamp_key_dir(1)
        if buf == b"H":
            return clamp_key_dir(-1)
        if buf in (b"0", b"1", b"2", b"3", b"4", b"5"):
            new_p = buf.decode("ascii")
            if new_p != "0":
                restore_main_pan_zero(screen, direct_out)
            screen.focused_panel = new_p
            return buf
        restore_main_pan_zero(screen, direct_out)

    return buf


def translate_input(
    buf: bytes,
    screen: VtScreen,
    last_emit: list[float],
    direct_out: list[bytes] | None = None,
) -> bytes:
    if direct_out is None:
        direct_out = []
    if screen.stdin_tail:
        buf = screen.stdin_tail + buf
        screen.stdin_tail = b""
        screen.stdin_tail_ts = 0.0
    if not buf:
        return b""

    # Hold any incomplete trailing ESC/CSI/SGR-mouse sequence across `os.read(stdin_fd)` boundaries
    last_esc = buf.rfind(b"\x1b")
    if last_esc != -1 and len(buf) - last_esc < 32:
        tail = buf[last_esc:]
        if len(tail) == 1 or (tail.startswith(b"\x1b[") and not COMPLETE_CSI_PREFIX_RE.match(tail)):
            screen.stdin_tail = tail
            screen.stdin_tail_ts = time.monotonic()
            buf = buf[:last_esc]
            if not buf:
                return b""

    now = time.monotonic()

    if b"\x1b[<" in buf:
        out_parts: list[bytes] = []
        pos = 0
        for m in SGR_MOUSE_RE.finditer(buf):
            if m.start() > pos:
                out_parts.append(translate_key_chunk(buf[pos : m.start()], screen, direct_out))
            pos = m.end()
            btn = int(m.group(1))
            base_btn = btn & ~0x1C
            col = int(m.group(2))
            row = int(m.group(3))
            kind = m.group(4)

            if base_btn in (66, 67):
                if kind == b"m":
                    continue
                pid, rect = screen.panel_at(col, row)
                if pid is None or rect is None:
                    continue
                # `[0]` runs 100% in-memory (`< 0.1ms`) at full 120Hz (`0` throttle, 2 cols/tick);
                # `[2]..[5]` forward `H`/`L` to Lazygit (`8ms` = 120Hz cap).
                if pid != "0" and (now - last_emit[0]) < 0.008:
                    continue
                dir_x = 1 if base_btn == 66 else -1
                seq = emit_hscroll_for_panel(screen, pid, rect, dir_x, 2, direct_out)
                last_emit[0] = now
                if seq:
                    out_parts.append(seq)
                continue

            if base_btn in (64, 65):
                pid, _ = screen.panel_at(col, row)
                if pid is not None and pid != "0":
                    restore_main_pan_zero(screen, direct_out)
                out_parts.append(m.group(0))
                continue

            if btn in (0, 32):
                pid, rect = screen.panel_at(col, row)
                if pid is not None and rect is not None:
                    if btn == 0 and kind == b"M":
                        screen.focused_panel = pid
                        if pid != "0":
                            restore_main_pan_zero(screen, direct_out)
                    if pid == "4" and (row - 1) == rect[1] and (col - 1) >= rect[0] + 11:
                        safe_col = rect[0] + 6
                        out_parts.append(f"\x1b[<{btn};{safe_col};{row}{kind.decode('ascii')}".encode("ascii"))
                        continue

            out_parts.append(m.group(0))

        if pos < len(buf):
            out_parts.append(translate_key_chunk(buf[pos:], screen, direct_out))
        return b"".join(out_parts)

    return translate_key_chunk(buf, screen, direct_out)


def read_master_burst(master_fd: int) -> bytes:
    """Coalesces back-to-back 1024-byte Darwin/POSIX PTY chunks of a single `tcell.Show()` frame."""
    chunks: list[bytes] = []
    total = 0
    while total < 65536:
        try:
            d = os.read(master_fd, 16384)
        except BlockingIOError:
            break
        except OSError as exc:
            if exc.errno in (errno.EAGAIN, errno.EWOULDBLOCK, errno.EINTR):
                break
            if not chunks:
                raise
            break
        if not d:
            break
        chunks.append(d)
        total += len(d)
        wait_s = 0.0004 if (len(d) >= 1024 or d[-1:] == b"\x1b") else 0.0
        try:
            r_more, _, _ = select.select([master_fd], [], [], wait_s)
        except Exception:
            break
        if not r_more:
            break
    return b"".join(chunks)


def main() -> int:
    cmd_args = sys.argv[2:] if len(sys.argv) > 1 and sys.argv[1] == "--" else sys.argv[1:]
    if not cmd_args:
        return 1

    stdin_fd = sys.stdin.fileno()
    stdout_fd = sys.stdout.fileno()
    init_packed = get_winsize_packed(stdout_fd)
    init_rows, init_cols, _, _ = struct.unpack("HHHH", init_packed)

    orig_termios = None
    try:
        orig_termios = termios.tcgetattr(stdin_fd)
    except termios.error:
        orig_termios = None

    child_pid, master_fd = pty.fork()
    if child_pid == 0:
        try:
            fcntl.ioctl(0, termios.TIOCSWINSZ, init_packed)
        except OSError:
            pass
        os.environ["CCIDE_LG_ROWS"] = str(init_rows)
        os.environ["CCIDE_LG_COLS"] = str(init_cols)
        os.environ["CCIDE_LG_SID"] = str(os.getpid())
        write_geom_file(os.getpid(), init_rows, init_cols)
        os.execvp(cmd_args[0], cmd_args)

    try:
        fcntl.ioctl(master_fd, termios.TIOCSWINSZ, init_packed)
    except OSError:
        pass
    write_geom_file(child_pid, init_rows, init_cols)
    screen = VtScreen(init_rows, init_cols)
    screen.sid = child_pid
    applied_size = [init_rows, init_cols]

    # Single-threaded self-pipe so SIGWINCH & post-launch resize never race with `VtScreen.feed()`
    wake_r, wake_w = os.pipe()
    for wfd in (wake_r, wake_w):
        flags = fcntl.fcntl(wfd, fcntl.F_GETFL)
        fcntl.fcntl(wfd, fcntl.F_SETFL, flags | os.O_NONBLOCK)
    winch_req = [False, False]  # [pending, force_nudge]

    def request_winch(force_nudge: bool = False) -> None:
        winch_req[0] = True
        if force_nudge:
            winch_req[1] = True
        try:
            os.write(wake_w, b"\x01")
        except OSError:
            pass

    def sync_winch_on_loop(force_nudge: bool = False) -> None:
        packed = get_winsize_packed(stdout_fd)
        rows, cols, xpx, ypx = struct.unpack("HHHH", packed)
        changed = (rows != applied_size[0]) or (cols != applied_size[1])
        applied_size[0], applied_size[1] = rows, cols
        write_geom_file(child_pid, rows, cols, screen.main_inner_w)
        screen.resize(rows, cols)
        try:
            if force_nudge and cols > 4:
                fcntl.ioctl(
                    master_fd,
                    termios.TIOCSWINSZ,
                    struct.pack("HHHH", rows, cols - 1, xpx, ypx),
                )
                os.killpg(child_pid, signal.SIGWINCH)
                time.sleep(0.015)
            fcntl.ioctl(master_fd, termios.TIOCSWINSZ, packed)
            if changed or force_nudge:
                os.killpg(child_pid, signal.SIGWINCH)
        except OSError:
            pass

    signal.signal(signal.SIGWINCH, lambda *_: request_winch(False))

    def post_launch_sync() -> None:
        started_narrow = init_cols < 60
        for idx, delay in enumerate((0.10, 0.25, 0.55)):
            time.sleep(delay)
            request_winch(force_nudge=(started_narrow and idx == 1))

    threading.Thread(target=post_launch_sync, daemon=True).start()

    if orig_termios is not None:
        raw = termios.tcgetattr(stdin_fd)
        raw[0] &= ~(
            termios.BRKINT | termios.ICRNL | termios.INPCK | termios.ISTRIP | termios.IXON
        )
        raw[1] &= ~termios.OPOST
        raw[2] |= termios.CS8
        raw[3] &= ~(termios.ECHO | termios.ICANON | termios.IEXTEN | termios.ISIG)
        termios.tcsetattr(stdin_fd, termios.TCSADRAIN, raw)

    write_all(stdout_fd, b"\x1b[?1004h")
    last_emit = [0.0]
    exit_code = 0
    try:
        while True:
            sel_timeout = (
                max(0.0, 0.035 - (time.monotonic() - screen.stdin_tail_ts))
                if screen.stdin_tail
                else None
            )
            try:
                rlist, _, _ = select.select([stdin_fd, master_fd, wake_r], [], [], sel_timeout)
            except InterruptedError:
                continue
            except select.error as exc:
                if exc.args and exc.args[0] == errno.EINTR:
                    continue
                break

            if wake_r in rlist:
                try:
                    while os.read(wake_r, 64):
                        pass
                except OSError:
                    pass
                if winch_req[0] and not screen.frame_gate.has_pending():
                    nudge = winch_req[1]
                    winch_req[0] = False
                    winch_req[1] = False
                    sync_winch_on_loop(force_nudge=nudge)

            if master_fd in rlist:
                try:
                    data = read_master_burst(master_fd)
                except OSError:
                    break
                if not data:
                    break
                out_bytes = screen.process_pty_chunk(
                    data,
                    applied_rows=applied_size[0],
                    applied_cols=applied_size[1],
                    master_write_cb=lambda b: write_all(master_fd, b),
                )
                if out_bytes:
                    write_all(stdout_fd, out_bytes)
                if winch_req[0] and not screen.frame_gate.has_pending():
                    nudge = winch_req[1]
                    winch_req[0] = False
                    winch_req[1] = False
                    sync_winch_on_loop(force_nudge=nudge)

            if stdin_fd in rlist:
                try:
                    in_data = os.read(stdin_fd, 4096)
                except OSError:
                    break
                if not in_data:
                    break
                had_focus_in = b"\x1b[I" in in_data
                direct_out: list[bytes] = []
                translated = translate_input(in_data, screen, last_emit, direct_out)
                if direct_out and not screen.frame_gate.has_pending():
                    write_all(stdout_fd, b"".join(direct_out))
                if translated:
                    write_all(master_fd, translated)
                if had_focus_in and getattr(screen, "_dimmed_unfocused", False):
                    screen._dimmed_unfocused = False
                    if not FOCUS_EVENT_RE.sub(b"", translated):
                        request_winch(force_nudge=True)

            if screen.stdin_tail and (
                not rlist or (time.monotonic() - screen.stdin_tail_ts) >= 0.035
            ):
                pending = screen.stdin_tail
                screen.stdin_tail = b""
                screen.stdin_tail_ts = 0.0
                direct_out = []
                translated = translate_key_chunk(pending, screen, direct_out)
                if direct_out and not screen.frame_gate.has_pending():
                    write_all(stdout_fd, b"".join(direct_out))
                if translated:
                    write_all(master_fd, translated)
    finally:
        try:
            write_all(stdout_fd, b"\x1b[?1004l")
        except OSError:
            pass
        if screen.frame_gate.tail:
            write_all(stdout_fd, screen.frame_gate.tail)
            screen.frame_gate.tail = b""
        if orig_termios is not None:
            try:
                termios.tcsetattr(stdin_fd, termios.TCSADRAIN, orig_termios)
            except termios.error:
                pass
        for wfd in (wake_r, wake_w):
            try:
                os.close(wfd)
            except OSError:
                pass
        for stale_p in (
            f"/tmp/.cc-ide/lg_geom_{child_pid}.txt",
            f"/tmp/.cc-ide/lg_full_{child_pid}.json",
        ):
            try:
                os.unlink(stale_p)
            except OSError:
                pass
        try:
            os.close(master_fd)
        except OSError:
            pass
        try:
            _, status = os.waitpid(child_pid, 0)
            if os.WIFEXITED(status):
                exit_code = os.WEXITSTATUS(status)
        except OSError:
            pass

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
