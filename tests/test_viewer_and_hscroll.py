import importlib.machinery
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def load_script_module(name: str, rel_path: str):
    full_path = os.path.join(REPO_ROOT, rel_path)
    loader = importlib.machinery.SourceFileLoader(name, full_path)
    spec = importlib.util.spec_from_loader(name, loader)
    assert spec is not None
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


class TestViewerWrapAndHScroll(unittest.TestCase):
    def test_viewer_json_defaults_to_wrap_false(self) -> None:
        pager = load_script_module("lazygit_pager", "scripts/lazygit-pager.sh")
        env_backup = os.environ.pop("CCIDE_WRAP", None)
        home_backup = os.environ.get("HOME")
        xdg_backup = os.environ.pop("XDG_CONFIG_HOME", None)
        try:
            with tempfile.TemporaryDirectory() as tmp_home:
                os.environ["HOME"] = tmp_home
                os.environ["XDG_CONFIG_HOME"] = os.path.join(tmp_home, ".config")
                self.assertFalse(pager.load_wrap_enabled([]))
                self.assertTrue(pager.load_wrap_enabled(["--wrap"]))
                self.assertFalse(pager.load_wrap_enabled(["--no-wrap"]))

                cfg_dir = os.path.join(tmp_home, ".config", "cc-ide")
                os.makedirs(cfg_dir, exist_ok=True)
                with open(os.path.join(cfg_dir, "viewer.json"), "w", encoding="utf-8") as f:
                    f.write('{\n  // user override\n  "wrap": true,\n}\n')
                self.assertTrue(pager.load_wrap_enabled([]))
                self.assertFalse(pager.load_wrap_enabled(["--no-wrap"]))
        finally:
            if env_backup is not None:
                os.environ["CCIDE_WRAP"] = env_backup
            if home_backup is not None:
                os.environ["HOME"] = home_backup
            else:
                os.environ.pop("HOME", None)
            if xdg_backup is not None:
                os.environ["XDG_CONFIG_HOME"] = xdg_backup
            else:
                os.environ.pop("XDG_CONFIG_HOME", None)

    def test_pager_emits_unwrapped_clipped_lines_and_full_cache(self) -> None:
        pager = load_script_module("lazygit_pager_slice", "scripts/lazygit-pager.sh")
        sample_diff = (
            "diff --git a/demo.rs b/demo.rs\n"
            "--- a/demo.rs\n"
            "+++ b/demo.rs\n"
            "@@ -10,1 +10,1 @@\n"
            "+let very_long_identifier = \"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\";\n"
        )
        pager_path = os.path.join(REPO_ROOT, "scripts", "lazygit-pager.sh")
        sid = "999991"
        env = os.environ.copy()
        env.pop("CCIDE_WRAP", None)
        env["CCIDE_LG_SID"] = sid
        with tempfile.TemporaryDirectory() as tmp_ipc:
            env["CCIDE_IPC_DIR"] = tmp_ipc
            cache_file = os.path.join(tmp_ipc, f"lg_full_{sid}.json")
            res_p0 = subprocess.run(
                [sys.executable, pager_path, "--col-width=14", "--no-wrap"],
                input=sample_diff,
                text=True,
                capture_output=True,
                env=env,
                check=True,
            )
            self.assertNotIn("↪", res_p0.stdout)
            for ln in res_p0.stdout.splitlines():
                self.assertLessEqual(pager.visible_width(ln), 40)

            self.assertTrue(os.path.isfile(cache_file))
            with open(cache_file, "r", encoding="utf-8") as f:
                cached = json.load(f)
            self.assertTrue(any("ABCDEFGHIJKLMNOPQRSTUVWXYZ" in ln for ln in cached["lines"]))

            res_wrapped = subprocess.run(
                [sys.executable, pager_path, "--col-width=14", "--wrap"],
                input=sample_diff,
                text=True,
                capture_output=True,
                env=env,
                check=True,
            )
            self.assertIn("↪", res_wrapped.stdout)

    def test_md_render_headings_and_unwrapped_default(self) -> None:
        md_path = os.path.join(REPO_ROOT, "scripts", "md-render.py")
        md_input = "###紧凑标题\n\nA long single paragraph line " + ("word " * 25) + "\n"
        res_unwrapped = subprocess.run(
            [sys.executable, md_path, "--width=36", "--no-wrap", "-"],
            input=md_input,
            text=True,
            capture_output=True,
            check=True,
        )
        self.assertNotIn("###", res_unwrapped.stdout)
        self.assertNotIn("↪", res_unwrapped.stdout)

        res_wrapped = subprocess.run(
            [sys.executable, md_path, "--width=36", "--wrap", "-"],
            input=md_input,
            text=True,
            capture_output=True,
            check=True,
        )
        self.assertIn("↪", res_wrapped.stdout)

        # Explicitly exercise pure-Python fallback so macOS (with `glow`) & Ubuntu CI (without `glow`) test identically
        md_mod = load_script_module("md_render_fallback", "scripts/md-render.py")
        norm_md = md_mod.normalize_markdown_headings(md_input)
        fb_unwrapped = md_mod.render_fallback_markdown(norm_md, 36, wrap_enabled=False)
        self.assertNotIn("###", fb_unwrapped)
        self.assertNotIn("↪", fb_unwrapped)
        fb_wrapped = md_mod.render_fallback_markdown(norm_md, 36, wrap_enabled=True)
        self.assertIn("↪", fb_wrapped)

    def test_vtscreen_120hz_direct_blitter_and_commits_reflog_lock(self) -> None:
        bridge = load_script_module("pty_hscroll_bridge", "scripts/pty-hscroll-bridge.py")
        scr = bridge.VtScreen(10, 44)
        scr.sid = 999992
        box_frame = (
            "\x1b[1;1H╭─[4]─Commits - Reflog─╮\x1b[1;25H╭─[0]─Log──────────╮"
            "\x1b[2;1H│* feat: demo          │\x1b[2;25H│CA * Merge PR #2  │"
            "\x1b[3;1H╰──────────────────────╯\x1b[3;25H╰──────────────────╯"
        ).encode("utf-8")
        scr.feed(box_frame)

        # 1. Short visible line (width 16 <= inner_w 18): horizontal swipe produces zero output
        scr.set_full_lines(["\x1b[32mCA\x1b[0m * Merge PR #2"])
        direct_out: list[bytes] = []
        self.assertEqual(bridge.translate_input(b"\x1b[<66;30;2M", scr, [0.0], direct_out), b"")
        self.assertEqual(direct_out, [])

        # 2. Long visible line (width 24 > inner_w 18 -> max_overflow == 6):
        #    SGR 66 blits 2-col smooth ANSI directly into `direct_out` with ZERO master_fd bytes!
        scr.feed("\x1b[2;25H│CA * Merge PR #2 f│".encode("utf-8"))
        scr.set_full_lines(["\x1b[32mCA\x1b[0m * Merge PR #2 from XY"])
        direct_out = []
        pty_bytes = bridge.translate_input(b"\x1b[<66;30;2M", scr, [0.0], direct_out)
        self.assertEqual(pty_bytes, b"")
        self.assertEqual(scr.pan_x["0"], 2)
        self.assertEqual(len(direct_out), 1)
        self.assertIn("‹".encode("utf-8"), direct_out[0])

        # Clamp at max_overflow == 6 across rapid 120Hz swipes
        for _ in range(5):
            bridge.translate_input(b"\x1b[<66;30;2M", scr, [0.0], direct_out)
        self.assertEqual(scr.pan_x["0"], 6)

        # SGR 67 pans back left by 2 columns
        direct_out = []
        bridge.translate_input(b"\x1b[<67;30;2M", scr, [0.0], direct_out)
        self.assertEqual(scr.pan_x["0"], 4)
        self.assertEqual(len(direct_out), 1)

        # 3. Clicking `Reflog` on [4]'s top border redirects to `Commits` (col 6), and `]` is blocked on [4]
        click_reflog = bridge.translate_input(b"\x1b[<0;18;1M", scr, [0.0], [])
        self.assertEqual(click_reflog, b"\x1b[<0;6;1M")
        self.assertEqual(scr.focused_panel, "4")
        self.assertEqual(bridge.translate_input(b"]", scr, [0.0], []), b"")
        self.assertEqual(bridge.translate_input(b"[", scr, [0.0], []), b"")

        # 4. Clicking and dragging inside Lazygit panels ([0], [1], etc.) passes through so text/line selection works
        scr.feed("\x1b[5;1H╭─[1]─Status───────────╮\x1b[6;1H│✓ cc-ide -> dev       │\x1b[7;1H╰──────────────────────╯".encode("utf-8"))
        self.assertEqual(bridge.translate_input(b"\x1b[<0;5;6M", scr, [0.0], []), b"\x1b[<0;5;6M")
        self.assertEqual(bridge.translate_input(b"\x1b[<32;30;2M", scr, [0.0], []), b"\x1b[<32;30;2M")

        # 5. Real `delta` hunk header (`─────┐`) + odd/even `panel_w` mismatch (`17` vs `18`) still matches & scrolls!
        scr_delta = bridge.VtScreen(10, 44)
        scr_delta.sid = 999993
        scr_delta.feed(
            (
                "\x1b[1;25H╭─[0]─Patch────────╮"
                "\x1b[2;25H│─────────┐        │"
                "\x1b[3;25H│ 10│let long_code│"
                "\x1b[4;25H╰──────────────────╯"
            ).encode("utf-8")
        )
        scr_delta.set_full_lines(
            [
                "\x1b[34m─────────┐\x1b[0m",
                "\x1b[32m 10│let long_code_line = 1234567890;\x1b[0m",
            ],
            panel_w=17,
        )
        d_out: list[bytes] = []
        bridge.translate_input(b"\x1b[<66;30;3M", scr_delta, [0.0], d_out)
        self.assertEqual(scr_delta.pan_x["0"], 2)
        self.assertEqual(len(d_out), 1)

        # 6. `tcell` OSC-8 `\x1b]8;;\x1b\\` split mid-sequence across PTY read chunks + split stdin SGR mouse bursts
        scr_osc = bridge.VtScreen(10, 48)
        scr_osc.sid = 999994
        osc_stream = (
            b"\x1b[1;1H\x1b]8;;\x1b\\\xe2\x95\xad\xe2\x94\x80[2]\xe2\x94\x80Files\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x95\xae"
            b"\x1b[1;25H\x1b]8;;\x1b\\\xe2\x95\xad\xe2\x94\x80[0]\xe2\x94\x80Unstaged\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x95\xae"
            b"\x1b[2;1H\x1b]8;;\x1b\\\xe2\x94\x82M \xef\x81\xbb demo.json       \xe2\x94\x82"
            b"\x1b[2;25H\x1b]8;;"
        )
        # Split right before `\x1b\\` of OSC-8 (previously leaked `8;;` into `[0]`'s left `│` border!)
        scr_osc.feed(osc_stream)
        scr_osc.feed(
            b"\x1b\\\xe2\x94\x82   1\xe2\x94\x82\"displayName\": \xe2\x94\x82"
            b"\x1b[3;1H\xe2\x95\xb0\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x801 of 9\xe2\x94\x80\xe2\x95\xaf"
            b"\x1b[3;25H\xe2\x95\xb0\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x95\xaf"
        )
        b0_osc = scr_osc.find_boxes()["0"]
        self.assertFalse(scr_osc.has_popup_over_main(b0_osc))
        scr_osc.set_full_lines(['   1│"displayName": "CC-IDE 3-Column Tiling Workspace"'], panel_w=18)

        osc_out: list[bytes] = []
        # Split `?1003h` motion (`\x1b[<35;30;2M`) + focus event (`\x1b[I`) + `SGR 66` across 2 reads
        p1 = bridge.translate_input(b"\x1b[I\x1b[<35;30", scr_osc, [0.0], osc_out)
        p2 = bridge.translate_input(b";2M\x1b[<66;30;2M", scr_osc, [0.0], osc_out)
        self.assertEqual(p1 + p2, b"\x1b[I\x1b[<35;30;2M")
        self.assertEqual(scr_osc.pan_x["0"], 2)
        self.assertEqual(len(osc_out), 1)
        self.assertIn("‹".encode("utf-8"), osc_out[0])

        blur_out: list[bytes] = []
        scr_osc.focused_panel = "2"
        bridge.translate_input(b"\x1b[O", scr_osc, [0.0], blur_out)
        self.assertEqual(len(blur_out), 1)
        self.assertIn(b"\x1b7\x1b[0m", blur_out[0])

        dag_mod = load_script_module("git_smooth_graph", "scripts/git_smooth_graph.py")
        log_mod = load_script_module("lazygit_log_smooth", "scripts/lazygit-log.py")
        dag_rows = dag_mod.render_smooth_dag_rows(
            [("m1", ["b0", "f1"]), ("f1", ["b0"]), ("b0", [])]
        )
        plain_dag = [log_mod.strip_ansi(r[0]) for r in dag_rows]
        self.assertEqual(plain_dag, ["⏣─╮ ", "│ ○ ", "○─╯ "])

        # Verify `assets/yazi-hscroll.lua` contains zero invalid Lua 5.4 string escape sequences (e.g. `\?`)
        import re as _re

        with open(os.path.join(REPO_ROOT, "assets", "yazi-hscroll.lua"), encoding="utf-8") as lf:
            lua_src = lf.read()
        for line_no, raw_ln in enumerate(lua_src.splitlines(), 1):
            stripped = raw_ln.split("--", 1)[0]
            for m in _re.finditer(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'', stripped):
                lit = m.group(0)[1:-1]
                i = 0
                while i < len(lit):
                    if lit[i] == "\\" and i + 1 < len(lit):
                        nxt = lit[i + 1]
                        self.assertTrue(
                            nxt in 'abfnrtv\\"\'\n\rzxu' or nxt.isdigit(),
                            msg=f"Invalid Lua 5.4 escape \\{nxt} at yazi-hscroll.lua:{line_no}",
                        )
                        i += 2
                    else:
                        i += 1

    def test_write_all_handles_short_pty_writes_and_compound_delta_sgr(self) -> None:
        bridge = load_script_module("pty_hscroll_bridge_w", "scripts/pty-hscroll-bridge.py")
        r_fd, w_fd = os.pipe()
        received = bytearray()

        def reader() -> None:
            while True:
                chunk = os.read(r_fd, 512)
                if not chunk:
                    break
                received.extend(chunk)

        import threading
        from unittest import mock

        t = threading.Thread(target=reader)
        t.start()
        real_write = os.write

        # Simulate Darwin slave PTY short writes (capping each `os.write` syscall to 333 bytes mid-UTF8/CSI)
        def short_write(fd: int, data: bytes) -> int:
            return real_write(fd, bytes(data)[:333])

        payload = ("\x1b[12;1H╭─[4]─Commits─╮│\U000f062d CA ⏣─╮ Merge PR│" * 400).encode("utf-8")
        try:
            with mock.patch("os.write", side_effect=short_write):
                bridge.write_all(w_fd, payload)
        finally:
            os.close(w_fd)
            t.join()
            os.close(r_fd)
        self.assertEqual(bytes(received), payload)

        # Compound `delta` SGR (`48;2;0;40;0;38;2;255;255;255`) carries both fg & bg cleanly past off-screen cut
        toks, _, _ = bridge.tokenize_ansi_line(
            "\x1b[48;2;0;40;0;38;2;114;220;120m0123456789abcdefghij\r\x1b(B\x1b[0m"
        )
        sliced = bridge.slice_tokens_padded(toks, 6, 10)
        self.assertIn("48;2;0;40;0", sliced)
        self.assertIn("38;2;114;220;120", sliced)
        self.assertNotIn("\r", sliced)
        self.assertIn("‹", sliced)
        self.assertIn("789abcdef", sliced)

    def test_torn_1024b_pty_escape_interleaving_and_tcell_ghost_residue_scrub(self) -> None:
        bridge = load_script_module("pty_hscroll_bridge_gate", "scripts/pty-hscroll-bridge.py")
        scr = bridge.VtScreen(10, 48)
        scr.sid = 999995
        term = bridge.VtScreen(10, 48)

        # Initial frame: [4] Commits + [5] Stash on left, [0] Unstaged on right
        init_frame = (
            "\x1b[1;1H╭─[4]─Commits──────────╮\x1b[1;25H╭─[0]─Unstaged─────────╮"
            "\x1b[2;1H│\U000f062d CA ⏣─╮ v0.5.0 Merge │\x1b[2;25H│   1│fn extract_json()│"
            "\x1b[3;1H│\U000f0718 Vi │ ◯ fix(md-rende │\x1b[3;25H│   2│let long_line = 1│"
            "\x1b[4;1H╰──────────────────────╯\x1b[4;25H│   3│                 │"
            "\x1b[5;1H╭─[5]─Stash────────────╮\x1b[5;25H╰──────────────────────╯"
            "\x1b[6;1H│                      │"
            "\x1b[7;1H╰───────────────0 of 0─╯"
        ).encode("utf-8")
        term.feed(scr.process_pty_chunk(init_frame, 10, 48))
        scr.set_full_lines(
            [
                "   1│fn extract_json() -> Option<u64> {",
                "\x1b[48;2;0;40;0;38;2;114;220;120m   2│let long_line = 1234567890123456;\x1b[0m",
                "   3│",
            ],
            panel_w=22,
        )

        # Pan [0] right to pan_x = 6 via SGR 66
        for _ in range(3):
            d_out: list[bytes] = []
            bridge.translate_input(b"\x1b[<66;30;2M", scr, [0.0], d_out)
            for b in d_out:
                term.feed(b)
        self.assertEqual(scr.pan_x["0"], 6)
        self.assertTrue(scr.main_dirty_on_term)

        # Now simulate a `tcell.Show()` refresh torn across multiple mid-CSI, mid-OSC-8, and mid-UTF-8
        # splits (matching exact Darwin 1024-byte PTY boundaries!) while 120Hz SGR 66 arrives mid-frame:
        torn_frame = (
            b"\x1b[3;1H\x1b(B\x1b[m\x1b]8;;\x1b\\\xe2\x94\x82\xf3\xb0\x9c\x98 Vi \xe2\x94\x82 \x1b[38;5;21"
            b"4m\xe2\x8f\xa3\x1b[m fix(md-rende \xe2\x94\x82"
            b"\x1b[5;7H\x1b]8"
            b";;\x1b\\Stash\x1b[3;48H\x1b[38;2;106;112"
            b";134m\x1b]8;;\x1b\\\xe2\x94"
            b"\x82\x1b[?25l"
        )
        # Feed across exact mid-CSI (`\x1b[38;5;21` | `4m`), mid-OSC-8 (`\x1b]8` | `;;`), mid-SGR, and mid-UTF-8 cuts:
        p_csi = torn_frame.index(b"4m")
        p_osc = torn_frame.index(b";;\x1b\\Stash")
        p_sgr = torn_frame.index(b";134m")
        p_utf8 = torn_frame.rindex(b"\x82\x1b[?25l")
        cuts = [0, p_csi, p_osc, p_sgr, p_utf8, len(torn_frame)]
        for idx in range(len(cuts) - 1):
            piece = torn_frame[cuts[idx] : cuts[idx + 1]]
            out_b = scr.process_pty_chunk(piece, 10, 48)
            if out_b:
                term.feed(out_b)
            d_mid: list[bytes] = []
            bridge.translate_input(b"\x1b[<66;30;2M", scr, [0.0], d_mid)
            if d_mid and not scr.frame_gate.has_pending():
                for b in d_mid:
                    term.feed(b)

        row3_str = "".join(ch if ch != "" else " " for ch in term.grid[2])
        row5_str = "".join(ch if ch != "" else " " for ch in term.grid[4])
        # Zero leaked `4` before `⏣`, zero `[StashStash8`, and `[0]`'s right border `│` at col 48 is intact!
        self.assertNotIn("4⏣", row3_str)
        self.assertIn("⏣ fix(md-rende", row3_str)
        self.assertEqual(term.grid[2][47], "│")
        self.assertIn("╭─[5]─Stash────────────╮", row5_str)
        self.assertNotIn("Stash8", row5_str)

        # Finally simulate `tcell.Show()` switching `[0]` to an uncached empty view (`No unstaged changes`):
        # `tcell` diffs from `pan_x = 0` (`"   1│fn extract_json()"` -> `"No unstaged           "`),
        # while row 3 at `pan_x = 0` was `"   2│let long_line = 1"` -> `"                      "` (and row 4
        # had trailing spaces at `pan_x = 0` that `tcell` does NOT even rewrite!).
        scr.set_full_lines([], panel_w=22)
        differential_empty = (
            b"\x1b[2;26HNo unstaged           "
            b"\x1b[3;26H                      "
        )
        term.feed(scr.process_pty_chunk(differential_empty, 10, 48))
        for ry in range(1, 4):
            self.assertEqual(term.grid[ry], scr.grid[ry])

    def test_cjk_main_hscroll_side_edge_and_bold_border_focus_sync(self) -> None:
        bridge = load_script_module("pty_hscroll_bridge_cjk", "scripts/pty-hscroll-bridge.py")

        def disp_w(s: str) -> int:
            return sum(bridge.char_cols(ch) for ch in s)

        # 1. [0]-Log CJK row matching & SGR 66/67 smooth pan
        full_rows = [
            "BD  │ ○ fix(test): 日期格式下拉用固定示例日期，避免月==日时文本碰撞",
            "CA  ⏣─┤ Merge pull request #388 from org/fix-date-picker-collision",
        ]
        inner_w = 52
        vis0 = bridge.clip_plain_cols(full_rows[0], inner_w)
        vis0 += " " * (inner_w - disp_w(vis0))
        vis1 = bridge.clip_plain_cols(full_rows[1], inner_w)
        vis1 += " " * (inner_w - disp_w(vis1))

        scr_cjk = bridge.VtScreen(6, 56)
        scr_cjk.sid = 999996
        scr_cjk.feed(
            (
                f"\x1b[1;1H╭─[0]─Log{'─' * (inner_w - 7)}╮"
                f"\x1b[2;1H│{vis0}│"
                f"\x1b[3;1H│{vis1}│"
                f"\x1b[4;1H╰{'─' * inner_w}╯"
            ).encode("utf-8")
        )
        scr_cjk.set_full_lines(full_rows, panel_w=inner_w)
        b0 = scr_cjk.find_boxes()["0"]
        self.assertEqual(scr_cjk.match_visible_main(b0), 0)

        direct_out: list[bytes] = []
        self.assertEqual(bridge.translate_input(b"\x1b[<66;20;2M", scr_cjk, [0.0], direct_out), b"")
        self.assertEqual(scr_cjk.pan_x["0"], 2)
        self.assertEqual(bridge.translate_input(b"\x1b[<66;20;2M", scr_cjk, [0.0], direct_out), b"")
        self.assertEqual(scr_cjk.pan_x["0"], 4)
        self.assertEqual(len(direct_out), 2)
        self.assertIn("‹".encode("utf-8"), direct_out[-1])

        self.assertEqual(bridge.translate_input(b"\x1b[<67;20;2M", scr_cjk, [0.0], direct_out), b"")
        self.assertEqual(bridge.translate_input(b"\x1b[<67;20;2M", scr_cjk, [0.0], direct_out), b"")
        self.assertEqual(scr_cjk.pan_x["0"], 0)

        # 2. Side panel [4]-Commits CJK trailing half "" at edge_x = x1 - 1
        scr_side = bridge.VtScreen(5, 28)
        scr_side.focused_panel = "4"
        scr_side.feed(
            (
                "\x1b[1;1H╭─[4]─Commits──────────╮"
                "\x1b[2;1H│* feat: 123456789012期│"
                "\x1b[3;1H╰──────────────────────╯"
            ).encode("utf-8")
        )
        b4 = scr_side.find_boxes()["4"]
        edge_x = b4[2] - 1
        self.assertEqual(scr_side.grid[1][edge_x - 1], "期")
        self.assertEqual(scr_side.grid[1][edge_x], "")
        self.assertTrue(scr_side.can_scroll_right("4", b4))
        self.assertEqual(bridge.translate_input(b"\x1b[<66;8;2M", scr_side, [0.0], []), b"L")

        # 3. Active bold border auto-syncing focused_panel from "1" -> "2"
        scr_focus = bridge.VtScreen(10, 30)
        scr_focus.focused_panel = "1"
        focus_frame = (
            b"\x1b[?2026h"
            b"\x1b[0m\x1b[1;1H\xe2\x95\xad\xe2\x94\x80[1]\xe2\x94\x80Status\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x95\xae"
            b"\x1b[2;1H\xe2\x94\x82clean                 \xe2\x94\x82"
            b"\x1b[3;1H\xe2\x95\xb0\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x95\xaf"
            b"\x1b[4;1H\x1b[38;2;139;233;253m\x1b[1m\xe2\x95\xad\xe2\x94\x80[2]\xe2\x94\x80Files\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x95\xae"
            b"\x1b[0m\x1b[5;1H\xe2\x94\x82M long_filename_overflow\xe2\x94\x82"
            b"\x1b[6;1H\xe2\x95\xb0\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x95\xaf"
            b"\x1b[0m\x1b[7;1H\xe2\x95\xad\xe2\x94\x80[4]\xe2\x94\x80Commits\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x95\xae"
            b"\x1b[8;1H\xe2\x94\x82* c1                  \xe2\x94\x82"
            b"\x1b[9;1H\xe2\x95\xb0\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x95\xaf"
            b"\x1b[?2026l"
        )
        scr_focus.process_pty_chunk(focus_frame, 10, 30)
        self.assertEqual(scr_focus.focused_panel, "2")
        self.assertEqual(bridge.translate_input(b"\x1b[<66;6;5M", scr_focus, [0.0], []), b"L")

    def test_lazygit_log_compact_author_cjk_fixed_4col_alignment(self) -> None:
        from types import SimpleNamespace
        from unittest.mock import patch

        log_mod = load_script_module("lazygit_log_cjk", "scripts/lazygit-log.py")

        cases = [
            ("林栀夏", "林"),
            ("油松", "油"),
            ("林 栀夏", "林"),
            ("林_dev", "林"),
            ("林A", "林A"),
            ("A林", "A林"),
            ("AB林", "AB"),
            ("Visio Vanitas", "VV"),
            ("57U", "57U"),
            ("Neko", "Nek"),
            ("", "??"),
        ]
        for raw, expected in cases:
            au = log_mod.compact_author(raw)
            self.assertEqual(au, expected, msg=f"raw={raw!r}")
            self.assertLessEqual(log_mod.visible_width(au), 3, msg=f"raw={raw!r}, au={au!r}")

        dag_glyphs = {"○", "⏣", "│", "├", "╰"}

        def prefix_before_dag(plain_line: str) -> str:
            for idx, ch in enumerate(plain_line):
                if ch in dag_glyphs:
                    return plain_line[:idx]
            self.fail(f"Expected DAG glyph in line: {plain_line!r}")

        # Mixed ASCII & CJK commits forming merges + branches (every non-root row keeps a pipe)
        git_stdout = "\n".join(
            [
                "\x1f".join(["c5", "m1", "Chihaya Anon", " (HEAD -> dev)", "feat: align commit log columns"]),
                "\x1f".join(["m1", "b1 f1", "57U", "", "Merge pull request #388 from org/fix-cjk"]),
                "\x1f".join(["f1", "b0", "林栀夏", "", "fix(test): 日期格式下拉用固定示例日期，避免月==日时文本碰撞以及超长主题换行对齐"]),
                "\x1f".join(["b1", "b0", "油松", "", "chore: update lazygit sidebar layout"]),
                "\x1f".join(["b0", "", "林A", "", "init: bootstrap repository"]),
            ]
        ) + "\n"
        fake_res = SimpleNamespace(stdout=git_stdout, returncode=0)

        with patch.object(log_mod.subprocess, "run", return_value=fake_res):
            rows_unwrapped = log_mod.format_full_width_log(None, True, 20, 60, wrap_enabled=False)
        self.assertEqual(len(rows_unwrapped), 5)
        for row in rows_unwrapped:
            plain = log_mod.strip_ansi(row)
            pfx = prefix_before_dag(plain)
            self.assertEqual(log_mod.visible_width(pfx), 4, msg=f"unwrapped misalignment: {plain!r}")

        with patch.object(log_mod.subprocess, "run", return_value=fake_res):
            rows_wrapped = log_mod.format_full_width_log(None, True, 20, 40, wrap_enabled=True)
        plain_wrapped = [log_mod.strip_ansi(r) for r in rows_wrapped]
        cjk_idx = next(i for i, ln in enumerate(plain_wrapped) if "林" in ln and "日期格式" in ln)
        self.assertEqual(log_mod.visible_width(prefix_before_dag(plain_wrapped[cjk_idx])), 4)
        cont_lines = []
        for ln in plain_wrapped[cjk_idx + 1 :]:
            if "↪" in ln:
                cont_lines.append(ln)
            else:
                break
        self.assertGreaterEqual(len(cont_lines), 1, msg=f"Expected continuation line after {plain_wrapped[cjk_idx]!r}")
        for cln in cont_lines:
            self.assertEqual(
                log_mod.visible_width(prefix_before_dag(cln)),
                4,
                msg=f"continuation misalignment: {cln!r}",
            )


if __name__ == "__main__":
    unittest.main()
