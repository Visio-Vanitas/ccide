#!/usr/bin/env python3
"""Tests for bootstrap resolver pinning/skew/fail-open, detect launchers, register.ts & proto CLI."""

import hashlib, io, json, os, platform, shutil, stat, subprocess, tarfile, tempfile, unittest

_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
_MJS = os.path.join(_REPO_ROOT, "scripts", "ensure-ccide-core.mjs")
_SH = os.path.join(_REPO_ROOT, "scripts", "ensure-ccide-core.sh")


def _write_exec(path: str, content: str) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(content)
    os.chmod(path, 0o755)


def _host_triple() -> str | None:
    sys_name, mach = platform.system(), platform.machine().lower()
    if sys_name == "Darwin":
        return "aarch64-apple-darwin" if mach in ("arm64", "aarch64") else "x86_64-apple-darwin"
    if sys_name == "Linux":
        return "aarch64-unknown-linux-gnu" if mach in ("arm64", "aarch64") else "x86_64-unknown-linux-gnu"
    return None


class BootstrapAndProtoTests(unittest.TestCase):
    def test_milestone_tag_pinning_detect_launchers_and_register_exit_code(self):
        for rel in ("scripts/ensure-ccide-core.mjs", "scripts/ensure-ccide-core.sh"):
            with open(os.path.join(_REPO_ROOT, rel), encoding="utf-8") as f:
                txt = f.read()
            self.assertNotIn("releases/latest", txt, msg=rel)
            self.assertIn("/releases/download/", txt, msg=rel)
            self.assertIn("ccide-bootstrap/", txt, msg=rel)
            self.assertIn(".ccide-core.version", txt, msg=rel)
            self.assertIn("CCIDE_RELEASE_TAG", txt, msg=rel)
            self.assertIn("--version", txt, msg=rel)

        for rel in ("scripts/ccide", "scripts/ccide.ps1"):
            with open(os.path.join(_REPO_ROOT, rel), encoding="utf-8") as f:
                txt = f.read()
            self.assertIn("workspace detect", txt, msg=rel)
            self.assertIn("--version", txt, msg=rel)
            for sniff in ("$env:WEZTERM_PANE", "$env:WT_SESSION", "${TERM_PROGRAM", "${ITERM_SESSION_ID"):
                self.assertNotIn(sniff, txt, msg=f"{rel} still sniffs {sniff}")
            if rel.endswith(".ps1"):
                self.assertIn("--wt-child", txt)

        with open(os.path.join(_REPO_ROOT, "hooks", "register.ts"), encoding="utf-8") as f:
            reg = f.read()
        self.assertIn("code === 0", reg)
        self.assertIn("code === 3", reg)
        self.assertIn("ccide-core unavailable", reg)
        self.assertNotIn("/unsupported/", reg)

    def test_mjs_version_cache_matching_dev_skew_and_offline_fail_open(self):
        node = shutil.which("node")
        if not node:
            self.skipTest("node not installed")
        with tempfile.TemporaryDirectory() as td:
            home = os.path.join(td, "home")
            plug = os.path.join(td, "plugin")
            os.makedirs(os.path.join(plug, ".claude-plugin"), exist_ok=True)
            with open(os.path.join(plug, ".claude-plugin", "plugin.json"), "w", encoding="utf-8") as f:
                json.dump({"name": "cc-ide", "version": "0.10.0"}, f)

            bin_p = os.path.join(home, ".claude", "bin", "ccide-core")
            cnt_log = os.path.join(td, "ver_calls.txt")
            _write_exec(
                bin_p,
                "#!/bin/sh\n"
                f'if [ "$1" = "--version" ]; then echo x >> "{cnt_log}"; echo "ccide-core 0.10.0"; exit 0; fi\n'
                'echo "RAN:$*"; exit 42\n',
            )
            env = {**os.environ, "HOME": home, "USERPROFILE": home, "CLAUDE_PLUGIN_ROOT": plug}
            r1 = subprocess.run([node, _MJS, "hello"], env=env, capture_output=True, text=True)
            r2 = subprocess.run([node, _MJS, "world"], env=env, capture_output=True, text=True)
            self.assertEqual(r1.returncode, 42)
            self.assertEqual(r2.returncode, 42)
            self.assertIn("RAN:world", r2.stdout)
            with open(cnt_log, encoding="utf-8") as f:
                self.assertEqual(len(f.read().splitlines()), 1, msg="second run should use .ccide-core.version cache")
            with open(os.path.join(home, ".claude", "bin", ".ccide-core.version"), encoding="utf-8") as f:
                self.assertTrue(f.read().strip().endswith("000:0.10.0"))

            # Dev build newer than plugin (0.11.0 > 0.10.2) is kept.
            _write_exec(bin_p, '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "ccide-core 0.11.0"; exit 0; fi\nexit 43\n')
            self.assertEqual(subprocess.run([node, _MJS, "x"], env=env, capture_output=True).returncode, 43)

            # Skewed patch binary (0.10.0 < 0.10.2) with missing tag -> fail-open runs existing binary.
            _write_exec(bin_p, '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "ccide-core 0.10.0"; exit 0; fi\nexit 42\n')
            with open(os.path.join(plug, ".claude-plugin", "plugin.json"), "w", encoding="utf-8") as f:
                json.dump({"name": "cc-ide", "version": "0.10.2"}, f)
            off_env = {**env, "CCIDE_RELEASE_TAG": "v0.0.0-ccide-test-missing"}
            self.assertEqual(subprocess.run([node, _MJS, "x"], env=off_env, capture_output=True).returncode, 42)

            # Skewed minor binary (0.9.4 < 0.10.2) with missing tag -> fail-open runs existing binary without live GitHub.
            _write_exec(bin_p, '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "ccide-core 0.9.4"; exit 0; fi\nexit 44\n')
            self.assertEqual(subprocess.run([node, _MJS, "x"], env=off_env, capture_output=True).returncode, 44)

            # Missing binary with missing tag -> fail-open exits 0.
            os.unlink(bin_p)
            self.assertEqual(subprocess.run([node, _MJS, "x"], env=off_env, capture_output=True).returncode, 0)

    def test_sh_pinned_download_sha256_skew_upgrade_and_fail_open(self):
        triple = _host_triple()
        if not triple:
            self.skipTest("unsupported POSIX host triple")
        with tempfile.TemporaryDirectory() as td:
            home = os.path.join(td, "home")
            plug = os.path.join(td, "plugin")
            mock_bin = os.path.join(td, "mockbin")
            srv_dir = os.path.join(td, "srv")
            os.makedirs(os.path.join(plug, ".claude-plugin"), exist_ok=True)
            os.makedirs(mock_bin, exist_ok=True)
            os.makedirs(srv_dir, exist_ok=True)
            with open(os.path.join(plug, ".claude-plugin", "plugin.json"), "w", encoding="utf-8") as f:
                json.dump({"name": "cc-ide", "version": "0.10.3"}, f)

            archive = f"ccide-core-{triple}.tar.gz"
            new_bin_bytes = b'#!/bin/sh\nif [ "$1" = "--version" ]; then echo "ccide-core 0.10.0"; exit 0; fi\necho "UPGRADED:$*"\nexit 7\n'
            tar_buf = io.BytesIO()
            with tarfile.open(fileobj=tar_buf, mode="w:gz") as tf:
                ti = tarfile.TarInfo("ccide-core")
                ti.size = len(new_bin_bytes)
                ti.mode = 0o755
                tf.addfile(ti, io.BytesIO(new_bin_bytes))
            tar_bytes = tar_buf.getvalue()
            with open(os.path.join(srv_dir, archive), "wb") as f:
                f.write(tar_bytes)
            digest = hashlib.sha256(tar_bytes).hexdigest()
            with open(os.path.join(srv_dir, "SHA256SUMS.txt"), "w", encoding="utf-8") as f:
                f.write(f"{digest}  {archive}\n")

            curl_log = os.path.join(td, "curl_log.txt")
            _write_exec(
                os.path.join(mock_bin, "curl"),
                "#!/bin/sh\n"
                f'echo "$*" >> "{curl_log}"\n'
                'url=""; out=""\n'
                'while [ $# -gt 0 ]; do\n'
                '  case "$1" in -o) out="$2"; shift 2;; -A|--connect-timeout|--max-time) shift 2;; -*) shift;; *) url="$1"; shift;; esac\n'
                "done\n"
                f'if [ "${{MOCK_FAIL:-0}}" = "1" ]; then exit 1; fi\n'
                f'cp "{srv_dir}/$(basename "$url")" "$out"\n',
            )
            old_bin = os.path.join(home, ".claude", "bin", "ccide-core")
            _write_exec(old_bin, '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "ccide-core 0.9.4"; exit 0; fi\nexit 55\n')
            env = {**os.environ, "HOME": home, "CLAUDE_PLUGIN_ROOT": plug, "PATH": f"{mock_bin}:/usr/bin:/bin"}

            # Offline fail-open on skewed binary keeps exit 55.
            r_off = subprocess.run(["sh", _SH, "ok"], env={**env, "MOCK_FAIL": "1"}, capture_output=True, text=True)
            self.assertEqual(r_off.returncode, 55)

            # Skewed binary triggers pinned v0.10.0 download + UA + SHA256 verify + upgrade.
            r_up = subprocess.run(["sh", _SH, "arg1"], env=env, capture_output=True, text=True)
            self.assertEqual(r_up.returncode, 7, msg=r_up.stderr)
            self.assertIn("UPGRADED:arg1", r_up.stdout)
            with open(curl_log, encoding="utf-8") as f:
                log = f.read()
            self.assertIn(f"https://github.com/Visio-Vanitas/ccide/releases/download/v0.10.0/{archive}", log)
            self.assertIn("-A ccide-bootstrap/0.10", log)

            # CCIDE_RELEASE_TAG override works when missing (clear both target and plugin bin copies).
            for p in (old_bin, os.path.join(plug, "bin", "ccide-core")):
                if os.path.exists(p):
                    os.unlink(p)
            r_tag = subprocess.run(
                ["sh", _SH, "arg2"],
                env={**env, "CCIDE_RELEASE_TAG": "v0.10.99"},
                capture_output=True,
                text=True,
            )
            self.assertEqual(r_tag.returncode, 7)
            with open(curl_log, encoding="utf-8") as f:
                self.assertIn("/releases/download/v0.10.99/", f.read())

            # Patch-tag fallback: when v0.11.0 404s (exit 1), ensure-ccide-core.sh falls back to exact v0.11.2.
            for p in (old_bin, os.path.join(plug, "bin", "ccide-core")):
                if os.path.exists(p):
                    os.unlink(p)
            with open(os.path.join(plug, ".claude-plugin", "plugin.json"), "w", encoding="utf-8") as f:
                json.dump({"name": "cc-ide", "version": "0.11.2"}, f)
            _write_exec(old_bin, '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "ccide-core 0.11.0"; exit 0; fi\nexit 55\n')
            _write_exec(
                os.path.join(mock_bin, "curl"),
                "#!/bin/sh\n"
                f'echo "$*" >> "{curl_log}"\n'
                'url=""; out=""\n'
                'while [ $# -gt 0 ]; do\n'
                '  case "$1" in -o) out="$2"; shift 2;; -A|--connect-timeout|--max-time) shift 2;; -*) shift;; *) url="$1"; shift;; esac\n'
                "done\n"
                'case "$url" in */v0.11.0/*) exit 1;; esac\n'
                f'cp "{srv_dir}/$(basename "$url")" "$out"\n',
            )
            r_fb = subprocess.run(["sh", _SH, "fb1"], env=env, capture_output=True, text=True)
            self.assertEqual(r_fb.returncode, 7, msg=r_fb.stderr)
            self.assertIn("UPGRADED:fb1", r_fb.stdout)
            with open(curl_log, encoding="utf-8") as f:
                fb_log = f.read()
            self.assertIn(f"/releases/download/v0.11.0/{archive}", fb_log)
            self.assertIn(f"/releases/download/v0.11.2/{archive}", fb_log)

    def test_core_binary_version_proto_and_detect_when_built(self):
        exe = "ccide-core.exe" if os.name == "nt" else "ccide-core"
        cands = [
            p
            for p in (
                os.path.join(_REPO_ROOT, "target", "debug", exe),
                os.path.join(_REPO_ROOT, "target", "release", exe),
                os.path.join(_REPO_ROOT, "bin", exe),
            )
            if os.path.isfile(p)
        ]
        bin_path = max(cands, key=os.path.getmtime) if cands else None
        if not bin_path:
            self.skipTest("target/{debug,release}/ccide-core not built")
        ver = subprocess.run([bin_path, "--version"], capture_output=True, text=True)
        if ver.returncode != 0 or not ver.stdout.strip().startswith("ccide-core "):
            self.skipTest("built ccide-core binary is pre-0.10")
        self.assertRegex(ver.stdout.strip(), r"^ccide-core \d+\.\d+\.\d+$")
        proto = json.loads(subprocess.run([bin_path, "proto", "--json"], capture_output=True, text=True, check=True).stdout)
        self.assertEqual(proto["ipc_proto"], 1)
        self.assertEqual(proto["patch_proto"], 1)
        self.assertEqual(proto["patch_rules_version"], "2")
        self.assertEqual(proto["spec_version"], 2)
        det = json.loads(subprocess.run([bin_path, "workspace", "detect", "--json"], capture_output=True, text=True, check=True).stdout)
        self.assertIn(det["backend"], ("iterm", "osc", "wezterm", "wt", "ghostty", "tmux", "none"))


if __name__ == "__main__":
    unittest.main()
