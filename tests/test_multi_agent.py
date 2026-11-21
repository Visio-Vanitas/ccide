#!/usr/bin/env python3
"""Unit & integration tests for v0.11.0 Multi-Agent (Claude Code, Antigravity agy, Codex)."""

import importlib.machinery
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
_SCRIPTS_DIR = os.path.join(_REPO_ROOT, "scripts")


def _load_merger():
    path = os.path.join(_SCRIPTS_DIR, "merge_agent_hooks.py")
    loader = importlib.machinery.SourceFileLoader("merge_agent_hooks", path)
    spec = importlib.util.spec_from_loader("merge_agent_hooks", loader)
    assert spec is not None
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


def _v11_engine_bin() -> str | None:
    exe = "ccide-core.exe" if os.name == "nt" else "ccide-core"
    cands = (
        os.path.join(_REPO_ROOT, "target", "debug", exe),
        os.path.join(_REPO_ROOT, "bin", exe),
        os.path.join(_REPO_ROOT, "target", "release", exe),
    )
    with tempfile.TemporaryDirectory() as td:
        env = {**os.environ, "HOME": td, "CCIDE_IPC_DIR": os.path.join(td, "ipc")}
        os.makedirs(env["CCIDE_IPC_DIR"], exist_ok=True)
        for cand in cands:
            if not os.path.isfile(cand):
                continue
            try:
                res = subprocess.run(
                    [cand, "yazi-hook", "--agent", "agy"],
                    input='{"hookEventName":"SessionStart","conversationId":"v11-probe","cwd":"/tmp"}',
                    text=True,
                    capture_output=True,
                    env=env,
                    timeout=3,
                )
                if res.returncode == 0 and res.stdout.strip() == "{}":
                    return cand
            except Exception:
                continue
    return None


class MultiAgentTests(unittest.TestCase):
    def test_antigravity_and_codex_manifests_valid(self) -> None:
        agy_dir = os.path.join(_REPO_ROOT, "integrations", "antigravity")
        with open(os.path.join(agy_dir, "plugin.json"), encoding="utf-8") as f:
            agy_plug = json.load(f)
        self.assertEqual(agy_plug["name"], "cc-ide")
        self.assertTrue(agy_plug["author"]["name"])
        self.assertIn("github.com", agy_plug["homepage"])

        for fname, exp_cmd in (
            ("hooks.json", "./hook.sh yazi-hook --agent agy"),
            ("hooks.windows.json", "hook.cmd yazi-hook --agent agy"),
        ):
            with open(os.path.join(agy_dir, fname), encoding="utf-8") as f:
                data = json.load(f)
            cc = data["cc-ide"]
            self.assertNotIn("PreToolUse", cc)
            self.assertNotIn("Stop", cc)
            self.assertEqual(cc["SessionStart"][0]["command"], exp_cmd)
            self.assertEqual(cc["SessionStart"][0]["timeout"], 5)
            post_grp = cc["PostToolUse"][0]
            matcher_tokens = set(post_grp["matcher"].split("|"))
            for expected_tool in (
                "replace_file_content",
                "swarm",
                "post_message",
                "invoke_subagent",
                "swarm_wait",
            ):
                self.assertIn(expected_tool, matcher_tokens)
            self.assertEqual(post_grp["hooks"][0]["command"], exp_cmd)
            self.assertEqual(post_grp["hooks"][0]["timeout"], 5)

        cdx_dir = os.path.join(_REPO_ROOT, "integrations", "codex")
        with open(os.path.join(cdx_dir, ".codex-plugin", "plugin.json"), encoding="utf-8") as f:
            cdx_plug = json.load(f)
        for key in (
            "name",
            "version",
            "description",
            "author",
            "homepage",
            "repository",
            "license",
            "keywords",
            "hooks",
        ):
            self.assertIn(key, cdx_plug)
        self.assertEqual(cdx_plug["hooks"], "./hooks.json")

        with open(os.path.join(cdx_dir, "hooks.json"), encoding="utf-8") as f:
            cdx_hooks = json.load(f)
        hk = cdx_hooks["hooks"]
        self.assertNotIn("PreToolUse", hk)
        self.assertEqual(hk["PostToolUse"][0]["matcher"], "apply_patch")
        for ev in ("SessionStart", "PostToolUse"):
            handler = hk[ev][0]["hooks"][0]
            self.assertEqual(handler["type"], "command")
            self.assertEqual(handler["command"], "./hook.sh yazi-hook --agent codex")
            self.assertEqual(handler["commandWindows"], "hook.cmd yazi-hook --agent codex")
            self.assertEqual(handler["timeoutSec"], 5)
            self.assertNotIn("timeout", handler)
            self.assertNotIn("async", handler)

    @unittest.skipIf(os.name == "nt", "POSIX shell wrapper test")
    def test_hook_sh_missing_binary_fallbacks(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            env = {**os.environ, "HOME": td, "CCIDE_CORE_BIN": os.path.join(td, "missing-core")}
            agy_sh = os.path.join(_REPO_ROOT, "integrations", "antigravity", "hook.sh")
            cdx_sh = os.path.join(_REPO_ROOT, "integrations", "codex", "hook.sh")
            self.assertEqual(subprocess.run(["sh", "-n", agy_sh]).returncode, 0)
            self.assertEqual(subprocess.run(["sh", "-n", cdx_sh]).returncode, 0)

            r_agy = subprocess.run(
                ["sh", agy_sh, "yazi-hook", "--agent", "agy"],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(r_agy.returncode, 0)
            self.assertEqual(r_agy.stdout.strip(), "{}")

            r_cdx = subprocess.run(
                ["sh", cdx_sh, "yazi-hook", "--agent", "codex"],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(r_cdx.returncode, 0)
            self.assertEqual(r_cdx.stdout, "")

    def test_merge_agent_hooks_insert_idempotence_foreign_preserve_and_remove(self) -> None:
        merger = _load_merger()
        foreign_pre = {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo foreign-pre"}]}
        foreign_post = {"matcher": "Write", "hooks": [{"type": "command", "command": "echo foreign-post"}]}
        initial = {"hooks": {"PreToolUse": [foreign_pre], "PostToolUse": [foreign_post]}}

        with tempfile.TemporaryDirectory() as td:
            target = os.path.join(td, "hooks.json")
            with open(target, "w", encoding="utf-8") as f:
                json.dump(initial, f, indent=2)

            hook_bin = os.path.join(td, "cc-ide", "hook.sh")
            changed1 = merger.merge_hooks(target, hook_bin=hook_bin)
            self.assertTrue(changed1)
            bak = target + ".ccide.bak"
            self.assertTrue(os.path.isfile(bak))
            with open(bak, encoding="utf-8") as f:
                self.assertEqual(json.load(f), initial)

            with open(target, encoding="utf-8") as f:
                merged = json.load(f)
            self.assertEqual(merged["hooks"]["PreToolUse"], [foreign_pre])
            self.assertIn(foreign_post, merged["hooks"]["PostToolUse"])
            self.assertTrue(any(merger.is_ccide_entry(g) for g in merged["hooks"]["PostToolUse"]))
            self.assertTrue(any(merger.is_ccide_entry(g) for g in merged["hooks"]["SessionStart"]))
            ss_cmd = merged["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            self.assertIn(hook_bin, ss_cmd)

            # Second merge is strictly idempotent
            changed2 = merger.merge_hooks(target, hook_bin=hook_bin)
            self.assertFalse(changed2)

            # Remove strips only cc-ide entries and preserves foreign entries
            removed = merger.merge_hooks(target, remove=True)
            self.assertTrue(removed)
            with open(target, encoding="utf-8") as f:
                after_rm = json.load(f)
            self.assertEqual(after_rm["hooks"]["PreToolUse"], [foreign_pre])
            self.assertEqual(after_rm["hooks"]["PostToolUse"], [foreign_post])
            self.assertNotIn("SessionStart", after_rm["hooks"])
            self.assertFalse(merger.merge_hooks(target, remove=True))

            # CLI subprocess path: quoted commandWindows for paths with spaces + --remove cleanup
            space_dir = os.path.join(td, "cc-ide with spaces")
            os.makedirs(space_dir, exist_ok=True)
            hook_cmd = os.path.join(space_dir, "hook.cmd")
            merger_script = os.path.join(_REPO_ROOT, "scripts", "merge_agent_hooks.py")
            cli_target = os.path.join(td, "cli_hooks.json")
            r_add = subprocess.run(
                [sys.executable, "-I", merger_script, "--target", cli_target, "--hook-bin", hook_cmd, "--json"],
                capture_output=True,
                text=True,
                check=True,
            )
            self.assertTrue(json.loads(r_add.stdout)["changed"])
            with open(cli_target, encoding="utf-8") as fh:
                cli_data = json.load(fh)
            self.assertEqual(
                cli_data["hooks"]["SessionStart"][0]["hooks"][0]["commandWindows"],
                f'"{hook_cmd}" yazi-hook --agent codex',
            )
            r_rm = subprocess.run(
                [sys.executable, "-I", merger_script, "--target", cli_target, "--remove", "--json"],
                capture_output=True,
                text=True,
                check=True,
            )
            self.assertTrue(json.loads(r_rm.stdout)["changed"])
            with open(cli_target, encoding="utf-8") as fh:
                cli_after = json.load(fh)
            self.assertNotIn("hooks", cli_after)

    @unittest.skipIf(os.name == "nt", "POSIX zsh launcher test")
    def test_ccide_launcher_syntax_and_agent_validation(self) -> None:
        ccide = os.path.join(_SCRIPTS_DIR, "ccide")
        if shutil.which("zsh"):
            self.assertEqual(subprocess.run(["zsh", "-n", ccide]).returncode, 0)
            r_unk = subprocess.run(
                ["zsh", ccide, "--agent", "unknown"],
                capture_output=True,
                text=True,
            )
            self.assertEqual(r_unk.returncode, 2)
            self.assertIn("unknown agent", r_unk.stderr.lower())

            if not os.path.exists("/opt/homebrew/bin/agy") and not os.path.exists("/usr/local/bin/agy"):
                with tempfile.TemporaryDirectory() as td:
                    r_miss = subprocess.run(
                        ["zsh", ccide, "--agy"],
                        env={"HOME": td, "PATH": "/usr/bin:/bin"},
                        capture_output=True,
                        text=True,
                    )
                    self.assertEqual(r_miss.returncode, 127)
                    self.assertIn("agy", r_miss.stderr)

    @unittest.skipIf(os.name == "nt", "macOS /usr/bin/python3 3.9 compatibility test")
    def test_merge_agent_hooks_python39_compat(self) -> None:
        sys_py = "/usr/bin/python3"
        if not os.path.isfile(sys_py):
            self.skipTest("/usr/bin/python3 not present")
        merger_py = os.path.join(_SCRIPTS_DIR, "merge_agent_hooks.py")
        r_help = subprocess.run(
            [sys_py, "-I", merger_py, "--help"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(r_help.returncode, 0, msg=r_help.stderr)
        r_imp = subprocess.run(
            [sys_py, "-I", "-c", f"import importlib.util; s=importlib.util.spec_from_file_location('m', {merger_py!r}); m=importlib.util.module_from_spec(s); s.loader.exec_module(m); assert callable(m.merge_hooks)"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(r_imp.returncode, 0, msg=r_imp.stderr)

    @unittest.skipIf(os.name == "nt", "POSIX zsh launcher test")
    def test_ccide_launcher_conversation_validation(self) -> None:
        ccide = os.path.join(_SCRIPTS_DIR, "ccide")
        if not shutil.which("zsh"):
            self.skipTest("zsh not installed")
        for bad in ("../bad", "short", "bad/slash"):
            r_bad = subprocess.run(
                ["zsh", ccide, "--agy", "--conversation", bad],
                capture_output=True,
                text=True,
            )
            self.assertEqual(r_bad.returncode, 2, msg=f"bad={bad} stderr={r_bad.stderr}")
            self.assertIn("conversation", r_bad.stderr.lower())
        r_eq = subprocess.run(
            ["zsh", ccide, "--agy", "--conversation=../bad"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(r_eq.returncode, 2, msg=r_eq.stderr)

    @unittest.skipIf(os.name == "nt", "POSIX zsh installer test")
    def test_install_sh_with_agy_arg_forwarding(self) -> None:
        installer = os.path.join(_REPO_ROOT, "install.sh")
        if not shutil.which("zsh"):
            self.skipTest("zsh not installed")
        self.assertEqual(subprocess.run(["zsh", "-n", installer]).returncode, 0)
        r_help = subprocess.run(
            ["zsh", installer, "--with-agy", "--help"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(r_help.returncode, 0, msg=r_help.stderr)
        with open(installer, encoding="utf-8") as fh:
            src = fh.read()
        self.assertNotIn("shift || true", src)
        self.assertIn('scripts/iterm-preferences.py" --apply "${EXTRA_ARGS[@]}"', src)
        self.assertIn('scripts/iterm-preferences.py" --restore "${EXTRA_ARGS[@]}"', src)

    def test_ccide_ps1_agent_and_conversation_validation(self) -> None:
        pwsh = shutil.which("pwsh")
        if not pwsh:
            self.skipTest("pwsh not installed")
        ps1 = os.path.join(_SCRIPTS_DIR, "ccide.ps1")
        r_unk = subprocess.run(
            [pwsh, "-NoProfile", "-File", ps1, "--agent", "unknown"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(r_unk.returncode, 2, msg=r_unk.stderr)
        r_bad = subprocess.run(
            [pwsh, "-NoProfile", "-File", ps1, "--agy", "--conversation", "../bad"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(r_bad.returncode, 2, msg=r_bad.stderr)

    def test_v11_engine_hooks_and_doctor_when_built(self) -> None:
        bin_path = _v11_engine_bin()
        if not bin_path:
            self.skipTest("v0.11.0 ccide-core binary not built yet")
        with tempfile.TemporaryDirectory() as td:
            ipc_dir = os.path.join(td, "ipc")
            os.makedirs(ipc_dir, exist_ok=True)
            env = {**os.environ, "HOME": td, "CCIDE_IPC_DIR": ipc_dir}

            # agy PostToolUse -> '{}' on stdout + live_sess written
            r_agy = subprocess.run(
                [bin_path, "yazi-hook", "--agent", "agy"],
                input=json.dumps({
                    "hookEventName": "PostToolUse",
                    "conversationId": "conv-agy-1",
                    "cwd": td,
                    "transcriptPath": os.path.join(_REPO_ROOT, "tests", "fixtures", "agy", "main.jsonl"),
                    "modelName": "Claude Opus 4.6 (Thinking)",
                    "toolCall": {"id": "1", "name": "read_file", "args": {"Path": "src/lib.rs"}},
                }),
                text=True,
                capture_output=True,
                env=env,
                check=True,
            )
            self.assertEqual(r_agy.stdout.strip(), "{}")
            self.assertTrue(os.path.isfile(os.path.join(td, ".cache", "cc-ide", "live_sess_conv-agy-1.json")))

            # agy swarm PostToolUse wake-only smoke test -> '{}' + live_sess written, no main_last_file
            cache_dir = os.path.join(td, ".cache", "cc-ide")
            edited_file = os.path.join(td, "edited.rs")
            with open(edited_file, "w", encoding="utf-8") as fh:
                fh.write("fn main() {}\n")
            env_yid = {**env, "CC_YAZI_ID": "42424"}
            r_swarm = subprocess.run(
                [bin_path, "yazi-hook", "--agent", "agy"],
                input=json.dumps({
                    "hookEventName": "PostToolUse",
                    "conversationId": "conv-agy-swarm",
                    "cwd": td,
                    "transcriptPath": os.path.join(
                        _REPO_ROOT, "tests", "fixtures", "agy_swarm", "brain", "coord-001", ".system_generated", "logs", "transcript.jsonl"
                    ),
                    "modelName": "gemini-flash-worker-agy",
                    "toolCall": {"id": "sw1", "name": "swarm", "args": {"action": "add", "name": "w1", "path": edited_file}},
                }),
                text=True,
                capture_output=True,
                env=env_yid,
                check=True,
            )
            self.assertEqual(r_swarm.stdout.strip(), "{}")
            self.assertTrue(os.path.isfile(os.path.join(cache_dir, "live_sess_conv-agy-swarm.json")))
            self.assertFalse(os.path.isfile(os.path.join(cache_dir, "main_last_file_42424.json")))

            # agy worker PostToolUse edit_file isolation: writes live_sess_<worker> but must NOT overwrite coordinator tab state
            coord_tp = os.path.join(
                _REPO_ROOT, "tests", "fixtures", "agy_swarm", "brain", "coord-001", ".system_generated", "logs", "transcript.jsonl"
            )
            with open(os.path.join(cache_dir, "tab_conv_42424.txt"), "w", encoding="utf-8") as fh:
                fh.write("coord-001")
            with open(os.path.join(cache_dir, "live_tab_42424.json"), "w", encoding="utf-8") as fh:
                json.dump({"agent": "agy", "session_id": "coord-001", "transcript_path": coord_tp}, fh)
            r_worker = subprocess.run(
                [bin_path, "yazi-hook", "--agent", "agy"],
                input=json.dumps({
                    "hookEventName": "PostToolUse",
                    "conversationId": "worker-argon-002",
                    "cwd": td,
                    "transcriptPath": os.path.join(
                        _REPO_ROOT, "tests", "fixtures", "agy_swarm", "brain", "worker-argon-002", ".system_generated", "logs", "transcript.jsonl"
                    ),
                    "modelName": "gemini-4-argon",
                    "toolCall": {"id": "ed1", "name": "edit_file", "args": {"path": edited_file}},
                }),
                text=True,
                capture_output=True,
                env=env_yid,
                check=True,
            )
            self.assertEqual(r_worker.stdout.strip(), "{}")
            self.assertTrue(os.path.isfile(os.path.join(cache_dir, "live_sess_worker-argon-002.json")))
            with open(os.path.join(cache_dir, "live_tab_42424.json"), encoding="utf-8") as fh:
                live_tab_after = json.load(fh)
            self.assertEqual(live_tab_after["session_id"], "coord-001")
            self.assertFalse(os.path.isfile(os.path.join(cache_dir, "main_last_file_42424.json")))
            self.assertFalse(os.path.isfile(os.path.join(cache_dir, "session_worker-argon-002.yazi")))

            # codex PostToolUse -> empty stdout + live_sess written
            r_cdx = subprocess.run(
                [bin_path, "yazi-hook", "--agent", "codex"],
                input=json.dumps({
                    "hook_event_name": "PostToolUse",
                    "session_id": "0194b8c2-1111-7000-8000-000000000144",
                    "cwd": td,
                    "transcript_path": os.path.join(_REPO_ROOT, "tests", "fixtures", "codex", "main_0144.jsonl"),
                    "tool_name": "apply_patch",
                    "tool_input": {"input": "*** Begin Patch\n*** Update File: src/lib.rs\n"},
                }),
                text=True,
                capture_output=True,
                env=env,
                check=True,
            )
            self.assertEqual(r_cdx.stdout, "")

            # pre-tool-hook contracts
            r_pre_agy = subprocess.run(
                [bin_path, "pre-tool-hook", "--agent", "agy"],
                input='{"hookEventName":"PreToolUse","conversationId":"c1","toolCall":{"name":"read_file","args":{}}}',
                text=True,
                capture_output=True,
                env=env,
                check=True,
            )
            self.assertEqual(json.loads(r_pre_agy.stdout), {"decision": "ask"})

            r_pre_cdx = subprocess.run(
                [bin_path, "pre-tool-hook", "--agent", "codex"],
                input='{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"apply_patch"}',
                text=True,
                capture_output=True,
                env=env,
                check=True,
            )
            self.assertEqual(r_pre_cdx.stdout, "")

            # doctor --json includes additive agents & agent_hooks keys
            r_doc = subprocess.run(
                [bin_path, "doctor", "--json"],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
            doc = json.loads(r_doc.stdout)
            self.assertIn("agents", doc)
            self.assertIn("agent_hooks", doc)


if __name__ == "__main__":
    unittest.main()
