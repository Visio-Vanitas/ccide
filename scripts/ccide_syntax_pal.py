#!/usr/bin/env python3
"""Palette-Unified Syntax Highlighting Helper (`cc-ide`)."""
import json
import os
import re

SGR_CSI_RE = re.compile(r"\x1b\[([0-9;]*)m")

DEFAULT_PALETTE: dict = {
    "digest": "default_builtin",
    "preset": "default",
    "hex": {
        "text": "#E8ECF8", "dim": "#787E96", "border": "#444A5E",
        "cyan": "#56D6FF", "blue": "#62A4FF", "lavender": "#C08EFF",
        "amber": "#FFC25C", "green": "#5CE49A", "coral": "#EE7C64", "mint": "#38DAAC",
        "keyword": "#62A4FF", "function": "#56D6FF", "type_name": "#38DAAC",
        "string": "#5CE49A", "number": "#C08EFF", "comment": "#787E96",
        "operator": "#56D6FF", "variable": "#E8ECF8", "code_bg": "#242838",
        "diff_add_fg": "#5CE49A", "diff_add_bg": "#0C2C18", "diff_add_emph_bg": "#184E2C",
        "diff_del_fg": "#EE7C64", "diff_del_bg": "#340E12", "diff_del_emph_bg": "#5A1A22",
    },
    "sgr": {
        "31": "38;2;232;236;248", "32": "38;2;92;228;154", "33": "38;2;56;218;172",
        "34": "38;2;86;214;255", "35": "38;2;98;164;255", "36": "38;2;86;214;255",
        "37": "38;2;232;236;248", "38;5;8": "38;2;120;126;150", "38;5;9": "38;2;192;142;255",
        "38;5;16": "38;2;192;142;255", "38;5;17": "38;2;86;214;255", "code_bg": "48;2;36;40;56",
    },
}

_KEYWORDS = frozenset({
    "fn", "pub", "let", "mut", "const", "struct", "enum", "impl", "trait", "use", "mod",
    "match", "if", "else", "for", "while", "return", "async", "await", "def", "class",
    "import", "from", "function", "export", "interface", "type",
})

_TOKEN_RE = re.compile(
    r'(?P<comment>//[^\n]*|(?:^|(?<=\s))#[^\n]*)'
    r'|(?P<str>"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\')'
    r'|(?P<num>\b0[xXbB][0-9a-fA-F_]+\b|\b\d+(?:\.\d+)?\b)'
    r'|(?P<call>\b[A-Za-z_][A-Za-z0-9_]*(?=\s*\())'
    r'|(?P<ident>\b[A-Za-z_][A-Za-z0-9_]*\b)'
)


def _syntax_candidates() -> list[str]:
    cands: list[str] = []
    pal_file = os.environ.get("CCIDE_PALETTE_FILE", "").strip()
    if pal_file:
        cands.append(os.path.join(os.path.dirname(pal_file), "syntax_resolved.json"))
    xdg = os.environ.get("XDG_CONFIG_HOME", "").strip()
    if xdg:
        cands.append(os.path.join(xdg, "cc-ide", "syntax_resolved.json"))
    cands.append(os.path.expanduser("~/.config/cc-ide/syntax_resolved.json"))
    return cands


def load_syntax_palette() -> dict:
    hex_map = dict(DEFAULT_PALETTE["hex"])
    sgr_map = dict(DEFAULT_PALETTE["sgr"])
    out = {
        "digest": DEFAULT_PALETTE["digest"],
        "preset": DEFAULT_PALETTE["preset"],
        "bat_theme": "base16-256",
        "hex": hex_map,
        "sgr": sgr_map,
    }
    for path in _syntax_candidates():
        if not os.path.isfile(path):
            continue
        try:
            with open(path, "r", encoding="utf-8") as f:
                data = json.load(f)
            if isinstance(data, dict):
                if isinstance(data.get("digest"), str) and data["digest"]:
                    out["digest"] = data["digest"]
                if isinstance(data.get("preset"), str) and data["preset"]:
                    out["preset"] = data["preset"]
                if isinstance(data.get("bat_theme"), str) and data["bat_theme"]:
                    out["bat_theme"] = data["bat_theme"]
                for sec, dst in (("hex", hex_map), ("sgr", sgr_map)):
                    if isinstance(data.get(sec), dict):
                        for k, v in data[sec].items():
                            if isinstance(v, str) and v:
                                dst[k] = v
                return out
        except Exception:
            pass
    return out


def _parse_hex(hex_str: str) -> tuple[int, int, int] | None:
    s = (hex_str or "").strip().lstrip("#")
    if len(s) == 3:
        s = "".join(ch * 2 for ch in s)
    if len(s) != 6:
        return None
    try:
        return int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16)
    except ValueError:
        return None


def hex_to_fg(hex_str: str) -> str:
    rgb = _parse_hex(hex_str)
    return f"\x1b[38;2;{rgb[0]};{rgb[1]};{rgb[2]}m" if rgb else ""


def hex_to_bg(hex_str: str) -> str:
    rgb = _parse_hex(hex_str)
    return f"\x1b[48;2;{rgb[0]};{rgb[1]};{rgb[2]}m" if rgb else ""


def remap_base16_sgr(text: str, pal: dict | None = None) -> str:
    if "\x1b[" not in text:
        return text
    sgr = (pal or load_syntax_palette()).get("sgr", {})
    if not sgr:
        return text

    def _repl(m: re.Match[str]) -> str:
        params = m.group(1)
        if not params:
            return "\x1b[m"
        toks = params.split(";")
        out: list[str] = []
        i, n = 0, len(toks)
        while i < n:
            tok = toks[i]
            if tok in ("38", "48") and i + 1 < n:
                mode = toks[i + 1]
                if mode == "2" and i + 4 < n:
                    out.extend(toks[i : i + 5])
                    i += 5
                    continue
                if mode == "5" and i + 2 < n:
                    key = f"38;5;{toks[i + 2]}"
                    out.append(sgr[key] if (tok == "38" and key in sgr) else ";".join(toks[i : i + 3]))
                    i += 3
                    continue
            out.append(sgr[tok] if (tok in ("31", "32", "33", "34", "35", "36", "37") and tok in sgr) else tok)
            i += 1
        return f"\x1b[{';'.join(out)}m"

    return SGR_CSI_RE.sub(_repl, text)


_XTERM_LEVELS = (0, 95, 135, 175, 215, 255)


def _rgb_to_xterm256(r: int, g: int, b: int) -> int:
    best_idx, best_d = 16, 1 << 30
    for ri, rv in enumerate(_XTERM_LEVELS):
        for gi, gv in enumerate(_XTERM_LEVELS):
            for bi, bv in enumerate(_XTERM_LEVELS):
                d = (r - rv) ** 2 + (g - gv) ** 2 + (b - bv) ** 2
                if d < best_d:
                    best_d, best_idx = d, 16 + 36 * ri + 6 * gi + bi
    for gi in range(24):
        v = 8 + 10 * gi
        d = (r - v) ** 2 + (g - v) ** 2 + (b - v) ** 2
        if d < best_d:
            best_d, best_idx = d, 232 + gi
    return best_idx


def upgrade_chroma_256_sgr(text: str, pal: dict | None = None) -> str:
    """Upgrades Glamour/Chroma's `terminal256` `38;5;N` tokens back to exact 24-bit RGB."""
    if "\x1b[38;5;" not in text:
        return text
    hx = (pal or load_syntax_palette()).get("hex", {})
    lut: dict[str, str] = {}
    for slot in ("text", "comment", "operator", "variable", "number", "type_name", "function", "string", "keyword", "diff_add_fg", "diff_del_fg"):
        rgb = _parse_hex(hx.get(slot, ""))
        if rgb:
            lut[str(_rgb_to_xterm256(*rgb))] = f"38;2;{rgb[0]};{rgb[1]};{rgb[2]}"
    if not lut:
        return text
    return re.sub(r"\x1b\[38;5;(\d+)m", lambda m: f"\x1b[{lut[m.group(1)]}m" if m.group(1) in lut else m.group(0), text)


def highlight_code_fallback(line: str, pal: dict | None = None) -> str:
    if not line:
        return ""
    hx = (pal or load_syntax_palette()).get("hex", DEFAULT_PALETTE["hex"])
    c_kw, c_fn = hex_to_fg(hx.get("keyword", "#62A4FF")), hex_to_fg(hx.get("function", "#56D6FF"))
    c_ty, c_str = hex_to_fg(hx.get("type_name", "#38DAAC")), hex_to_fg(hx.get("string", "#5CE49A"))
    c_num, c_cmt = hex_to_fg(hx.get("number", "#C08EFF")), hex_to_fg(hx.get("comment", "#787E96"))
    c_var = hex_to_fg(hx.get("variable", "#E8ECF8"))

    out: list[str] = []
    pos = 0
    for m in _TOKEN_RE.finditer(line):
        if m.start() > pos:
            out.append(f"{c_var}{line[pos:m.start()]}")
        kind, tok = m.lastgroup, m.group(0)
        if kind == "comment":
            out.append(f"{c_cmt}{tok}\x1b[0m")
        elif kind == "str":
            out.append(f"{c_str}{tok}\x1b[0m")
        elif kind == "num":
            out.append(f"{c_num}{tok}\x1b[0m")
        elif kind == "call":
            out.append(f"{c_kw if tok in _KEYWORDS else c_fn}{tok}\x1b[0m")
        elif kind == "ident":
            col = c_kw if tok in _KEYWORDS else (c_ty if tok[0].isupper() else c_var)
            out.append(f"{col}{tok}\x1b[0m")
        pos = m.end()
    if pos < len(line):
        out.append(f"{c_var}{line[pos:]}\x1b[0m")
    elif out and not out[-1].endswith("\x1b[0m"):
        out.append("\x1b[0m")
    return "".join(out)
