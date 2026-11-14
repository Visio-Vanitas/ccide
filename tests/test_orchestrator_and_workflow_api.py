#!/usr/bin/env python3
"""Tests for built-in Workflow JS templates, macro expansion, CLI runner & HTTP/SSE API server."""

import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import unittest
import urllib.request

_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
_SCRIPTS_DIR = os.path.join(_REPO_ROOT, "scripts")
sys.path.insert(0, _SCRIPTS_DIR)

import ccide_workflow_runner as wf_runner  # type: ignore[import-not-found]  # noqa: E402
import ccide_workflow_server as wf_server  # type: ignore[import-not-found]  # noqa: E402

_MOCK_CLAUDE_PY = r"""#!/usr/bin/env python3
import json, os, sys
assert "CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL" not in os.environ
assert "CLAUDE_CODE_SUBAGENT_MODEL_FORCE" not in os.environ
assert os.environ.get("CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS") == "0"
disp_path = os.environ["CCIDE_WORKFLOW_DISPATCH_JSON"]
with open(disp_path, encoding="utf-8") as f:
    disp = json.load(f)
tdir = os.environ["TEST_TRANSCRIPT_DIR"]
os.makedirs(tdir, exist_ok=True)
with open(os.path.join(tdir, "journal.jsonl"), "w", encoding="utf-8") as jf:
    jf.write(json.dumps({"step": 1, "model": disp["args"]["__ccide"]["roles"]["coder"]["model"]}) + "\n")
print(json.dumps({"type": "workflow_phase", "data": {"title": "Generate"}}), flush=True)
print(json.dumps({"type": "workflow_log", "data": {"message": "running consensus"}}), flush=True)
print(json.dumps({"type": "assistant", "message": {"content": [{"type": "tool_result", "content": {"runId": "run_mock_1", "transcriptDir": tdir, "scriptPath": "/tmp/mock.js"}}]}}), flush=True)
print(json.dumps({"type": "result", "result": {"synthesis": "mocked-ok", "prompt": disp["args"].get("prompt", "")}}), flush=True)
"""


class WorkflowTemplatesAndApiTests(unittest.TestCase):
    def test_builtin_workflow_templates_meta_and_determinism_rules(self):
        wf_dir = os.path.join(_REPO_ROOT, "assets", "workflows")
        for name in ("ccide-review", "ccide-implement", "ccide-consensus"):
            path = os.path.join(wf_dir, f"{name}.js")
            self.assertTrue(os.path.isfile(path), msg=f"Missing {path}")
            with open(path, encoding="utf-8") as f:
                src = f.read()
            self.assertIn("export const meta", src)
            for forbidden in ("Date.now()", "Math.random()", "new Date()"):
                self.assertNotIn(forbidden, src, msg=f"{name}.js contains forbidden {forbidden}")
            meta = wf_runner.extract_meta(src, name)
            self.assertEqual(meta["name"], name)
            self.assertTrue(meta["description"])
            self.assertGreaterEqual(len(meta["phases"]), 2)
            for ph in meta["phases"]:
                self.assertRegex(src, rf"phase\(\s*['\"]{re.escape(ph['title'])}['\"]\s*\)")

    def test_macro_expansion_resolves_model_and_effort(self):
        cfg = wf_runner.load_orchestrator_config(_REPO_ROOT)
        preset, roles, _ = wf_runner.resolve_roles(cfg, preset="hetero-rr", role_overrides=["reviewer=kimi-k3:xhigh"])
        self.assertEqual(preset, "hetero-rr")
        self.assertEqual(roles["reviewer"]["models"], ["kimi-k3"])
        self.assertEqual(roles["reviewer"]["effort"], "xhigh")

        sample = 'agent("hi", { model: "@scout", effort: "@scout" }); agent("r", { model: "@reviewer", effort: "@reviewer" });'
        expanded = wf_runner.expand_workflow_macros(sample, roles)
        self.assertIn('"deepseek-v4.1-flash[1M]"', expanded)
        self.assertIn('"low"', expanded)
        self.assertIn('"kimi-k3"', expanded)
        self.assertIn('"xhigh"', expanded)
        self.assertNotIn("@scout", expanded)
        self.assertNotIn("@reviewer", expanded)

    def test_cli_list_json(self):
        proc = subprocess.run(
            [sys.executable, "-I", os.path.join(_SCRIPTS_DIR, "ccide_workflow_runner.py"), "list", "--json", "--cwd", _REPO_ROOT],
            capture_output=True, text=True, check=True,
        )
        data = json.loads(proc.stdout)
        names = {w["name"] for w in data["workflows"]}
        self.assertTrue({"ccide-review", "ccide-implement", "ccide-consensus"}.issubset(names))
        self.assertIn("balanced", data["presets"])
        self.assertIn("hetero-rr", data["presets"])

    def test_workflow_http_server_and_anthropic_sse_with_mock_claude(self):
        with tempfile.TemporaryDirectory() as tmp:
            mock_bin = os.path.join(tmp, "claude")
            with open(mock_bin, "w", encoding="utf-8") as f:
                f.write(_MOCK_CLAUDE_PY)
            os.chmod(mock_bin, 0o755)
            tdir = os.path.join(tmp, "transcript")

            old_env = {k: os.environ.get(k) for k in ("CCIDE_CLAUDE_BIN", "TEST_TRANSCRIPT_DIR", "CCIDE_WORKFLOW_SERVER_QUIET", "CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL", "CLAUDE_CODE_SUBAGENT_MODEL_FORCE")}
            os.environ["CCIDE_CLAUDE_BIN"] = mock_bin
            os.environ["TEST_TRANSCRIPT_DIR"] = tdir
            os.environ["CCIDE_WORKFLOW_SERVER_QUIET"] = "1"
            os.environ["CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL"] = "should-be-stripped"
            os.environ["CLAUDE_CODE_SUBAGENT_MODEL_FORCE"] = "1"

            srv = wf_server.create_server("127.0.0.1", 0)
            port = srv.server_address[1]
            thr = threading.Thread(target=srv.serve_forever, daemon=True)
            thr.start()
            try:
                base = f"http://127.0.0.1:{port}"
                with urllib.request.urlopen(f"{base}/v1/workflows") as r:
                    cat = json.loads(r.read().decode("utf-8"))
                self.assertIn("ccide-consensus", {w["name"] for w in cat["workflows"]})

                req = urllib.request.Request(
                    f"{base}/v1/workflows/run",
                    data=json.dumps({"workflow": "ccide-consensus", "args": {"prompt": "hello"}, "cwd": _REPO_ROOT, "stream": False}).encode("utf-8"),
                    headers={"Content-Type": "application/json"},
                )
                with urllib.request.urlopen(req) as r:
                    res = json.loads(r.read().decode("utf-8"))
                self.assertTrue(res["ok"])
                self.assertEqual(res["run_id"], "run_mock_1")
                self.assertEqual(len(res["journal"]), 1)
                self.assertEqual(res["result"]["synthesis"], "mocked-ok")

                sse_req = urllib.request.Request(
                    f"{base}/v1/messages",
                    data=json.dumps({
                        "model": "workflow/ccide-consensus",
                        "stream": True,
                        "messages": [{"role": "user", "content": "compare designs"}],
                        "metadata": {"cwd": _REPO_ROOT},
                    }).encode("utf-8"),
                    headers={"Content-Type": "application/json"},
                )
                with urllib.request.urlopen(sse_req) as r:
                    sse_body = r.read().decode("utf-8")
                self.assertIn("event: message_start", sse_body)
                self.assertIn("event: content_block_delta", sse_body)
                self.assertIn("mocked-ok", sse_body)
                self.assertIn("event: message_stop", sse_body)
            finally:
                srv.shutdown()
                srv.server_close()
                for k, v in old_env.items():
                    if v is None:
                        os.environ.pop(k, None)
                    else:
                        os.environ[k] = v


if __name__ == "__main__":
    unittest.main()
