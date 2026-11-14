#!/usr/bin/env python3
"""
Smooth Unicode Box-Drawing DAG Commit Graph Renderer (`scripts/git_smooth_graph.py`).

Synthesizes connected rounded box-drawing trees (`○`, `⏣`, `│`, `─`, `╭`, `╮`, `╰`, `╯`,
`├`, `┤`, `┬`, `┴`, `┼`) directly from `(sha, parent_shas)` commit topology — matching
Lazygit `[4]-Commits` (`pkg/gui/presentation/graph/graph.go`) inside `[0]-Log` without
`git log --graph`'s disjointed ASCII `*`, `/`, `\\` slashes or extra connector rows.
"""

RESET = "\x1b[0m"

LANE_PALETTE = [
    "\x1b[38;5;114m",  # mint green
    "\x1b[38;5;117m",  # sky cyan
    "\x1b[38;5;183m",  # soft lavender
    "\x1b[38;5;215m",  # warm amber
    "\x1b[38;5;210m",  # coral rose
    "\x1b[38;5;151m",  # sage
]

UP = 1
DOWN = 2
LEFT = 4
RIGHT = 8

MASK_GLYPH = {
    0: " ",
    UP: "╵",
    DOWN: "╷",
    UP | DOWN: "│",
    LEFT: "╴",
    RIGHT: "╶",
    LEFT | RIGHT: "─",
    DOWN | RIGHT: "╭",
    DOWN | LEFT: "╮",
    UP | RIGHT: "╰",
    UP | LEFT: "╯",
    UP | DOWN | RIGHT: "├",
    UP | DOWN | LEFT: "┤",
    DOWN | LEFT | RIGHT: "┬",
    UP | LEFT | RIGHT: "┴",
    UP | DOWN | LEFT | RIGHT: "┼",
}


def _alloc_lane(lanes: list[tuple[str, str] | None], item: tuple[str, str]) -> int:
    for idx, slot in enumerate(lanes):
        if slot is None:
            lanes[idx] = item
            return idx
    lanes.append(item)
    return len(lanes) - 1


def render_smooth_dag_rows(
    commits: list[tuple[str, list[str]]],
) -> list[tuple[str, str, int]]:
    """
    Takes `[(sha, [parent_sha, ...]), ...]` in topological log order and returns
    `[(row_graph_ansi, wrap_continuation_graph_ansi, graph_visible_cols), ...]`.
    """
    lanes: list[tuple[str, str] | None] = []
    color_seq = 0
    results: list[tuple[str, str, int]] = []

    def next_color() -> str:
        nonlocal color_seq
        col = LANE_PALETTE[color_seq % len(LANE_PALETTE)]
        color_seq += 1
        return col

    for sha, parents in commits:
        prev_snapshot = list(lanes)
        matching = [i for i, slot in enumerate(lanes) if slot is not None and slot[0] == sha]
        if matching:
            col = matching[0]
            commit_color = lanes[col][1]  # type: ignore[index]
        else:
            commit_color = next_color()
            col = _alloc_lane(lanes, (sha, commit_color))
            matching = [col]

        next_lanes = list(lanes)
        for extra_idx in matching[1:]:
            next_lanes[extra_idx] = None

        horiz_spans: list[tuple[int, int, str]] = []
        for extra_idx in matching[1:]:
            prev_extra = prev_snapshot[extra_idx] if extra_idx < len(prev_snapshot) else None
            extra_col = prev_extra[1] if prev_extra is not None else commit_color
            horiz_spans.append((min(col, extra_idx), max(col, extra_idx), extra_col))

        if not parents:
            next_lanes[col] = None
        else:
            next_lanes[col] = (parents[0], commit_color)
            for p_sha in parents[1:]:
                existing = next(
                    (i for i, slot in enumerate(next_lanes) if slot is not None and slot[0] == p_sha),
                    None,
                )
                if existing is not None:
                    p_color = next_lanes[existing][1]  # type: ignore[index]
                    target_lane = existing
                else:
                    p_color = next_color()
                    target_lane = _alloc_lane(next_lanes, (p_sha, p_color))
                if target_lane != col:
                    horiz_spans.append((min(col, target_lane), max(col, target_lane), p_color))

        max_lanes = max(len(prev_snapshot), len(next_lanes), col + 1)
        width_cells = max_lanes * 2
        masks = [0] * width_cells
        colors = [""] * width_cells

        for i in range(max_lanes):
            prev_slot = prev_snapshot[i] if i < len(prev_snapshot) else None
            next_slot = next_lanes[i] if i < len(next_lanes) else None
            cx = i * 2
            if prev_slot is not None:
                masks[cx] |= UP
                colors[cx] = prev_slot[1]
            if next_slot is not None:
                masks[cx] |= DOWN
                if not colors[cx]:
                    colors[cx] = next_slot[1]

        for a, b, scol in horiz_spans:
            ax, bx = a * 2, b * 2
            masks[ax] |= RIGHT
            colors[ax] = colors[ax] or scol
            for mx in range(ax + 1, bx):
                masks[mx] |= LEFT | RIGHT
                colors[mx] = scol
            masks[bx] |= LEFT
            colors[bx] = scol

        # Fix tip detection: track seen parent targets across previous rows
        node_glyph = "⏣" if len(parents) >= 2 else "○"
        row_chars: list[str] = []
        cont_chars: list[str] = []
        for x in range(width_cells):
            if x == col * 2:
                row_chars.append(f"{commit_color}{node_glyph}{RESET}")
            else:
                if masks[x] == (UP | DOWN | LEFT | RIGHT):
                    ch = "─"
                else:
                    ch = MASK_GLYPH.get(masks[x], "│")
                c_ansi = colors[x]
                row_chars.append(f"{c_ansi}{ch}{RESET}" if (ch != " " and c_ansi) else ch)

            lane_idx = x // 2
            if x % 2 == 0 and lane_idx < len(next_lanes) and next_lanes[lane_idx] is not None:
                lc = next_lanes[lane_idx][1]  # type: ignore[index]
                cont_chars.append(f"{lc}│{RESET}")
            else:
                cont_chars.append(" ")

        while next_lanes and next_lanes[-1] is None:
            next_lanes.pop()
        lanes = next_lanes
        results.append(("".join(row_chars), "".join(cont_chars), width_cells))

    return results
