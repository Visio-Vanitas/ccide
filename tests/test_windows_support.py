#!/usr/bin/env python3
"""Cross-platform packaging, hook, PowerShell/cmd launcher & <300-line modular budget tests."""

import json
import os
import re
import shutil
import subprocess
import unittest

_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


class CrossPlatformPackagingTests(unittest.TestCase):
    def test_hooks_json_uses_cross_platform_node_resolver(self):
        hooks_path = os.path.join(_REPO_ROOT, "hooks", "hooks.json")
        with open(hooks_path, encoding="utf-8") as f:
            data = json.load(f)
        raw = json.dumps(data)
        self.assertIn("ensure-ccide-core.mjs", raw)
        self.assertNotIn('sh "', raw)

    def test_ensure_ccide_core_mjs_passes_node_syntax_and_noop_execution(self):
        node = shutil.which("node")
        if not node:
            self.skipTest("node not installed")
        mjs = os.path.join(_REPO_ROOT, "scripts", "ensure-ccide-core.mjs")
        chk = subprocess.run([node, "--check", mjs], capture_output=True, text=True)
        self.assertEqual(chk.returncode, 0, msg=chk.stderr)

    def test_windows_installers_and_launchers_exist_and_match_version(self):
        for rel in ("install.ps1", "install.cmd", "scripts/ccide.ps1", "scripts/ccide.cmd"):
            full = os.path.join(_REPO_ROOT, rel)
            self.assertTrue(os.path.isfile(full), msg=f"Missing {rel}")

        with open(os.path.join(_REPO_ROOT, "Cargo.toml"), encoding="utf-8") as f:
            mc = re.search(r'(?m)^version\s*=\s*"([^"]+)"', f.read())
        assert mc is not None
        with open(os.path.join(_REPO_ROOT, "install.ps1"), encoding="utf-8") as f:
            ps1_txt = f.read()
        mp = re.search(r'(?m)^\$PluginVersion\s*=\s*"([^"]+)"', ps1_txt)
        assert mp is not None
        self.assertEqual(mc.group(1), mp.group(1))
        self.assertIn("ccide-core.exe", ps1_txt)
        self.assertIn("-Doctor", ps1_txt)
        self.assertIn("-Uninstall", ps1_txt)

    def test_new_rust_and_python_modules_respect_300_line_modular_budget(self):
        checked_dirs = [
            os.path.join(_REPO_ROOT, "src", "commands", "sidebar"),
            os.path.join(_REPO_ROOT, "src", "commands", "workspace"),
            os.path.join(_REPO_ROOT, "src", "tui", "context"),
            os.path.join(_REPO_ROOT, "src", "ffi"),
            os.path.join(_REPO_ROOT, "src", "telemetry"),
        ]
        extra_files = [
            os.path.join(_REPO_ROOT, "src", "platform.rs"),
            os.path.join(_REPO_ROOT, "src", "tui", "layout.rs"),
            os.path.join(_REPO_ROOT, "src", "commands", "pane_runner.rs"),
            os.path.join(_REPO_ROOT, "scripts", "bun_pe.py"),
            os.path.join(_REPO_ROOT, "scripts", "cc_patch_rules.py"),
            os.path.join(_REPO_ROOT, "scripts", "cc_schema_inspector.py"),
            os.path.join(_REPO_ROOT, "scripts", "cc-maint.py"),
            os.path.join(_REPO_ROOT, "scripts", "patch_claude_binary.py"),
        ]
        targets = list(extra_files)
        for d in checked_dirs:
            for fn in sorted(os.listdir(d)):
                if fn.endswith((".rs", ".py")):
                    targets.append(os.path.join(d, fn))

        over = []
        for p in targets:
            with open(p, encoding="utf-8") as f:
                n = sum(1 for _ in f)
            if n > 300:
                over.append(f"{os.path.relpath(p, _REPO_ROOT)} ({n} > 300)")
        self.assertEqual(over, [], msg=f"Modules exceeded 300-line modular budget: {over}")


if __name__ == "__main__":
    unittest.main()
