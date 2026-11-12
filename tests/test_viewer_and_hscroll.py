import importlib.machinery
import importlib.util
import json
import os
import subprocess
import sys
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
        try:
            self.assertFalse(pager.load_wrap_enabled([]))
            self.assertTrue(pager.load_wrap_enabled(["--wrap"]))
            self.assertFalse(pager.load_wrap_enabled(["--no-wrap"]))
        finally:
            if env_backup is not None:
                os.environ["CCIDE_WRAP"] = env_backup

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
        cache_file = f"/tmp/.cc-ide/lg_full_{sid}.json"
        try:
            if os.path.exists(cache_file):
                os.unlink(cache_file)
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
        finally:
            if os.path.exists(cache_file):
                os.unlink(cache_file)

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
            "\x1b[48;2;0;40;0;38;2;114;220;120m0123456789abcdefghij\x1b[0m"
        )
        sliced = bridge.slice_tokens_padded(toks, 6, 10)
        self.assertIn("48;2;0;40;0", sliced)
        self.assertIn("38;2;114;220;120", sliced)
        self.assertIn("‹", sliced)
        self.assertIn("789abcdef", sliced)


if __name__ == "__main__":
    unittest.main()
