#!/usr/bin/env python3
"""End-to-end tests for v0.9.0 Palette-Unified Syntax Highlighting & Ghostty Workspace Backend."""

import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
import xml.etree.ElementTree as ET

_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
_SCRIPTS_DIR = os.path.join(_REPO_ROOT, "scripts")
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)


def _engine_bin() -> str:
    exe = "ccide-core.exe" if os.name == "nt" else "ccide-core"
    cands = (
        os.path.join(_REPO_ROOT, "target", "debug", exe),
        os.path.join(_REPO_ROOT, "bin", exe),
        os.path.join(_REPO_ROOT, "target", "release", exe),
    )
    for cand in cands:
        if os.path.isfile(cand):
            return cand
    cargo = shutil.which("cargo")
    if cargo:
        subprocess.run([cargo, "build", "--locked"], cwd=_REPO_ROOT, check=True)
        if os.path.isfile(cands[0]):
            return cands[0]
    raise FileNotFoundError("ccide-core binary not built yet")


class PaletteSyntaxAndGhosttyTests(unittest.TestCase):
    def test_palette_presets_and_syntax_overrides_sync_all_artifacts(self):
        bin_path = _engine_bin()
        with tempfile.TemporaryDirectory() as td:
            pal_file = os.path.join(td, "palette.json")
            with open(pal_file, "w", encoding="utf-8") as f:
                f.write(
                    """{
                      // Nord base preset with custom syntax overrides
                      "preset": "nord",
                      "syntax": {
                        "syn_keyword": "#12AB34",
                        "syn_string": "#DE5612"
                      }
                    }"""
                )
            env = {**os.environ, "HOME": td, "CCIDE_PALETTE_FILE": pal_file, "CCIDE_IPC_DIR": os.path.join(td, "ipc")}
            res = subprocess.run([bin_path, "workspace", "theme", "sync"], env=env, capture_output=True, text=True)
            self.assertEqual(res.returncode, 0, msg=res.stderr)

            syn_json = os.path.join(td, "syntax_resolved.json")
            tm_file = os.path.join(td, "ccide.tmTheme")
            glow_file = os.path.join(td, "glow-style.json")
            ghostty_file = os.path.join(td, "ghostty-theme.conf")
            yazi_theme = os.path.join(td, "yazi-theme.toml")
            lg_yml = os.path.join(td, "lazygit.yml")
            for p in (syn_json, tm_file, glow_file, ghostty_file, yazi_theme, lg_yml):
                self.assertTrue(os.path.isfile(p), msg=f"Missing generated artifact: {p}")

            with open(yazi_theme, encoding="utf-8") as f:
                yt_txt = f.read()
            self.assertIn("syntect_theme", yt_txt)
            self.assertIn("[filetype]", yt_txt)
            with open(lg_yml, encoding="utf-8") as f:
                lg_txt = f.read()
            self.assertIn("activeBorderColor:", lg_txt)
            self.assertIn("selectedLineBgColor:", lg_txt)

            with open(syn_json, encoding="utf-8") as f:
                syn = json.load(f)
            self.assertEqual(syn["preset"], "nord")
            self.assertEqual(syn["hex"]["keyword"].upper(), "#12AB34")
            self.assertEqual(syn["hex"]["string"].upper(), "#DE5612")
            self.assertEqual(syn["sgr"]["35"], "38;2;18;171;52")
            self.assertEqual(syn["sgr"]["32"], "38;2;222;86;18")

            # Valid XML plist .tmTheme
            root = ET.parse(tm_file).getroot()
            self.assertEqual(root.tag, "plist")
            with open(tm_file, encoding="utf-8") as f:
                tm_txt = f.read()
            self.assertIn("#12AB34", tm_txt.upper())
            self.assertIn("#DE5612", tm_txt.upper())

            # Valid Glamour glow-style.json
            with open(glow_file, encoding="utf-8") as f:
                glow = json.load(f)
            self.assertEqual(glow["code_block"]["chroma"]["keyword"]["color"].upper(), "#12AB34")
            self.assertEqual(glow["code_block"]["chroma"]["literal_string"]["color"].upper(), "#DE5612")
            self.assertEqual(glow["h1"]["prefix"], "▌ ")

            # Valid Ghostty snippet
            with open(ghostty_file, encoding="utf-8") as f:
                gt_txt = f.read()
            self.assertIn("foreground =", gt_txt)
            self.assertIn("palette = 0=", gt_txt)

            # Idempotent second sync (zero writes when digest matches)
            m1 = os.stat(tm_file).st_mtime_ns
            time.sleep(0.02)
            subprocess.run([bin_path, "workspace", "theme", "sync"], env=env, check=True)
            m2 = os.stat(tm_file).st_mtime_ns
            self.assertEqual(m1, m2)

    def test_lazygit_pager_and_md_render_follow_palette_syntax(self):
        bin_path = _engine_bin()
        pager = os.path.join(_SCRIPTS_DIR, "lazygit-pager.sh")
        md_render = os.path.join(_SCRIPTS_DIR, "md-render.py")
        diff_sample = (
            "diff --git a/demo.rs b/demo.rs\n"
            "--- a/demo.rs\n"
            "+++ b/demo.rs\n"
            "@@ -1 +1 @@\n"
            "-pub fn old_fn() {}\n"
            '+pub fn new_fn() -> &str { "hello" }\n'
        )
        md_sample = "# Heading\n\n```rust\npub fn demo() -> usize {\n    let msg = \"hello\";\n    42\n}\n```\n"

        with tempfile.TemporaryDirectory() as td:
            pal_file = os.path.join(td, "palette.json")
            with open(pal_file, "w", encoding="utf-8") as f:
                json.dump(
                    {
                        "preset": "dracula",
                        "syntax": {
                            "syn_keyword": "#12AB34",
                            "syn_string": "#DE5612",
                            "syn_diff_add_fg": "#22CC88",
                        },
                    },
                    f,
                )
            base_env = {
                **os.environ,
                "HOME": td,
                "CCIDE_PALETTE_FILE": pal_file,
                "CCIDE_IPC_DIR": os.path.join(td, "ipc"),
            }
            subprocess.run([bin_path, "workspace", "theme", "sync"], env=base_env, check=True)

            # 1. Diff pager (with delta if available)
            r_diff = subprocess.run(
                [sys.executable, "-I", pager, "--width=100"],
                input=diff_sample,
                text=True,
                capture_output=True,
                env=base_env,
            )
            self.assertEqual(r_diff.returncode, 0, msg=r_diff.stderr)
            self.assertIn("18;171;52", r_diff.stdout)

            # 2. Pure-Python fallback diff pager (masking delta)
            fb_env = {**base_env, "PATH": ""}
            r_diff_fb = subprocess.run(
                [sys.executable, "-I", pager, "--width=100"],
                input=diff_sample,
                text=True,
                capture_output=True,
                env=fb_env,
            )
            self.assertEqual(r_diff_fb.returncode, 0)
            self.assertIn("34;204;136", r_diff_fb.stdout)

            # 3. Markdown renderer (glow + pure-Python fallback)
            for run_env in (base_env, fb_env):
                r_md = subprocess.run(
                    [sys.executable, "-I", md_render, "-w", "90", "-"],
                    input=md_sample,
                    text=True,
                    capture_output=True,
                    env=run_env,
                )
                self.assertEqual(r_md.returncode, 0, msg=r_md.stderr)
                self.assertIn("18;171;52", r_md.stdout)

    @unittest.skipIf(os.name == "nt", "POSIX mock executable test")
    def test_ghostty_native_open_focus_close_and_fallback(self):
        bin_path = _engine_bin()
        with tempfile.TemporaryDirectory() as td:
            log_file = os.path.join(td, "osascript.log")
            ver_file = os.path.join(td, "ver.txt")
            mock_osa = os.path.join(td, "mock_osascript")
            with open(ver_file, "w", encoding="utf-8") as f:
                f.write("1.3.1\n")
            with open(mock_osa, "w", encoding="utf-8") as f:
                f.write(
                    f"""#!/bin/sh
printf '%s\\n---\\n' "$*" >> "{log_file}"
case "$*" in
  *"get version"*)
    cat "{ver_file}"
    ;;
  *"split"*)
    echo "ghostty:term-center-1:term-yazi-2,term-git-3,term-sb-4"
    ;;
esac
exit 0
"""
                )
            os.chmod(mock_osa, stat.S_IRWXU)

            env = {
                k: v
                for k, v in os.environ.items()
                if k not in ("TMUX", "SSH_CONNECTION", "SSH_TTY", "ITERM_SESSION_ID", "WEZTERM_PANE")
            }
            env.update(
                {
                    "HOME": td,
                    "TERM_PROGRAM": "ghostty",
                    "CCIDE_OSASCRIPT_BIN": mock_osa,
                    "CCIDE_GHOSTTY_WIDTH_PX": "2400",
                    "CCIDE_IPC_DIR": os.path.join(td, "ipc"),
                }
            )

            # Native probe succeeds on Ghostty 1.3.1
            p_ok = subprocess.run([bin_path, "workspace", "probe-ghostty"], env=env)
            self.assertEqual(p_ok.returncode, 0)

            # Native open computes exact 30% | 54% | 16% pixel deltas (480px left, 456px right on 2400px)
            o_res = subprocess.run(
                [bin_path, "workspace", "open", td, "9901", "1", "0", "30", td, "1", "ghostty"],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(o_res.returncode, 0, msg=o_res.stderr)
            self.assertEqual(
                o_res.stdout.strip(),
                "ghostty:term-center-1:term-yazi-2,term-git-3,term-sb-4",
            )

            # Focus & Close lifecycle
            subprocess.run(
                [bin_path, "workspace", "switch-subagent", "9901", "sess1", "main", "0"],
                env=env,
                check=True,
            )
            time.sleep(0.08)
            subprocess.run([bin_path, "workspace", "close", "", "9901"], env=env, check=True)

            with open(log_file, encoding="utf-8") as f:
                logged = f.read()
            self.assertIn("resize_split:left,480", logged)
            self.assertIn("resize_split:right,456", logged)
            self.assertIn("term-center-1", logged)
            self.assertIn("term-yazi-2", logged)
            self.assertIn("term-sb-4", logged)

            # Fallback probe returns non-zero on Ghostty < 1.3 (e.g. 1.2.0)
            with open(ver_file, "w", encoding="utf-8") as f:
                f.write("1.2.0\n")
            p_old = subprocess.run([bin_path, "workspace", "probe-ghostty"], env=env)
            self.assertNotEqual(p_old.returncode, 0)

    def test_builtin_slash_commands_module_registered_in_hooks_json(self):
        with open(os.path.join(_REPO_ROOT, "hooks", "hooks.json"), encoding="utf-8") as f:
            hj = json.load(f)
        self.assertEqual(hj.get("modules"), ["./register.ts"])
        reg_path = os.path.join(_REPO_ROOT, "hooks", "register.ts")
        self.assertTrue(os.path.isfile(reg_path))
        with open(reg_path, encoding="utf-8") as f:
            src = f.read()
        for cmd in ("palette", "rebalance-panes", "clear-agents", "orch", "ccide-doctor"):
            self.assertIn(f"name: '{cmd}'", src)
            self.assertIn(f"{{ command: '{cmd}' }}", src)
        self.assertIn("immediate: true", src)
        self.assertIn("on('turn.step'", src)
        self.assertIn("on('classic.PostModelSwitch'", src)
        self.assertIn("pf_sid_", src)


if __name__ == "__main__":
    unittest.main()
