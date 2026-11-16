#!/usr/bin/env python3
"""Unit tests for Issue/PR templates and `scripts/agent-task-context.py`."""

import importlib.util
import os
import tempfile
import unittest

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SCRIPT_PATH = os.path.join(REPO_ROOT, "scripts", "agent-task-context.py")

spec = importlib.util.spec_from_file_location("agent_task_context", SCRIPT_PATH)
assert spec and spec.loader
atc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(atc)


class IssueTemplateAndAgentInfraTests(unittest.TestCase):
    def test_issue_and_pr_templates_exist_with_required_hooks(self):
        expected_templates = [
            ".github/ISSUE_TEMPLATE/config.yml",
            ".github/ISSUE_TEMPLATE/bug_report.yml",
            ".github/ISSUE_TEMPLATE/feature_request.yml",
            ".github/ISSUE_TEMPLATE/upstream_compat.yml",
            ".github/PULL_REQUEST_TEMPLATE.md",
            ".github/workflows/issue-pr-lifecycle.yml",
        ]
        for rel in expected_templates:
            full = os.path.join(REPO_ROOT, rel)
            self.assertTrue(os.path.isfile(full), msg=f"Missing {rel}")

        with open(os.path.join(REPO_ROOT, ".github/ISSUE_TEMPLATE/bug_report.yml"), encoding="utf-8") as f:
            bug_yml = f.read()
        for field_id in ("id: component", "id: summary", "id: repro_steps", "id: doctor_json", "id: acceptance_criteria"):
            self.assertIn(field_id, bug_yml)

        with open(os.path.join(REPO_ROOT, ".github/ISSUE_TEMPLATE/feature_request.yml"), encoding="utf-8") as f:
            feat_yml = f.read()
        for field_id in ("id: component", "id: motivation", "id: proposal", "id: constraints", "id: acceptance_criteria"):
            self.assertIn(field_id, feat_yml)

        with open(os.path.join(REPO_ROOT, ".github/PULL_REQUEST_TEMPLATE.md"), encoding="utf-8") as f:
            pr_md = f.read()
        self.assertIn("Closes #", pr_md)
        self.assertIn("ccide-automation:", pr_md)

    def test_parse_chinese_and_english_issues_into_agent_brief(self):
        issue_cn = {
            "number": 7,
            "title": "fix(sidebar/patcher): 右侧栏模型精灵图偶发闪回 CLAUDE 并显示 false",
            "url": "https://github.com/Visio-Vanitas/ccide/issues/7",
            "labels": [],
            "body": (
                "## 现象\n右侧栏偶发切回 CLAUDE 并显示 false。\n\n"
                "## 根因分析\n`scripts/patch_claude_binary.py` 与 `src/commands/sidebar` 时间戳竞争。\n\n"
                "## 验收标准\n"
                "- [ ] `cur_pid_<pid>.txt` 不再出现 `false`\n"
                "- `cargo test` 与 `patch_claude_binary.py --verify` 全部通过\n"
            ),
        }
        brief = atc.parse_issue_to_brief(issue_cn)
        self.assertEqual(brief["issue_number"], 7)
        self.assertEqual(brief["kind"], "fix")
        self.assertEqual(brief["target_base"], "dev")
        self.assertEqual(brief["closing_directive"], "Closes #7")
        self.assertIn("area:patcher", brief["areas"])
        self.assertIn("area:sidebar", brief["areas"])
        self.assertTrue(brief["agent_ready"])
        self.assertEqual(len(brief["acceptance_criteria"]), 2)

        issue_form = {
            "number": 42,
            "title": "feat: native Ghostty split backend",
            "labels": [{"name": "enhancement"}],
            "body": (
                "### Affected Component\nworkspace (Tier 1 iTerm2 Daemon / Tier 3 tmux)\n\n"
                "### Acceptance Criteria\n- [ ] Splits 30% | 54% | 16% in Ghostty\n"
            ),
        }
        brief2 = atc.parse_issue_to_brief(issue_form)
        self.assertEqual(brief2["kind"], "feat")
        self.assertIn("area:workspace", brief2["areas"])
        self.assertIn("src/commands/workspace/", brief2["relevant_paths"])
        self.assertEqual(brief2["acceptance_criteria"], ["Splits 30% | 54% | 16% in Ghostty"])

        issue_ws_dir = {
            "number": 55,
            "title": "fix(workspace): harden src/commands/workspace/ ghostty probe",
            "labels": [],
            "body": "### Acceptance Criteria\n- [ ] Passes Ghostty tests\n",
        }
        brief3 = atc.parse_issue_to_brief(issue_ws_dir)
        self.assertIn("area:workspace", brief3["areas"])
        self.assertIn("src/commands/workspace/", brief3["relevant_paths"])

    def test_extract_closing_numbers_and_line_budget(self):
        refs = atc.extract_closing_numbers("Summary\nCloses #6, fixes #7 and RESOLVED #12.")
        self.assertEqual(refs, [6, 7, 12])

        with tempfile.TemporaryDirectory() as td:
            os.makedirs(os.path.join(td, "src"))
            os.makedirs(os.path.join(td, "scripts"))
            with open(os.path.join(td, "src", "small.rs"), "w", encoding="utf-8") as f:
                f.write("fn a() {}\n" * 40)
            with open(os.path.join(td, "scripts", "big.py"), "w", encoding="utf-8") as f:
                f.write("x = 1\n" * 305)
            rep = atc.check_line_budget(td, changed_only=False, max_lines=300)
            self.assertEqual(rep["checked_count"], 2)
            self.assertEqual(rep["over_budget_count"], 1)
            self.assertEqual(rep["over_budget"][0]["path"], "scripts/big.py")

    def test_rust_layering_and_line_budget_invariants(self):
        src_root = os.path.join(REPO_ROOT, "src")
        non_command_modules = (
            "ffi",
            "telemetry",
            "session",
            "tui",
            "orchestrator",
            "json",
            "budget",
            "model_id",
            "platform",
            "proto",
        )

        def iter_rs_files(base_path: str):
            if os.path.isfile(base_path) and base_path.endswith(".rs"):
                yield base_path
            elif os.path.isdir(base_path):
                for root, _, files in os.walk(base_path):
                    for fn in sorted(files):
                        if fn.endswith(".rs"):
                            yield os.path.join(root, fn)

        def read_non_comment_code(rs_file: str) -> str:
            with open(rs_file, encoding="utf-8") as f:
                return "\n".join(
                    line for line in f if not line.lstrip().startswith("//")
                )

        for mod_name in non_command_modules:
            for candidate in (
                os.path.join(src_root, mod_name),
                os.path.join(src_root, f"{mod_name}.rs"),
            ):
                for rs_file in iter_rs_files(candidate):
                    rel = os.path.relpath(rs_file, REPO_ROOT)
                    txt = read_non_comment_code(rs_file)
                    self.assertNotIn(
                        "crate::commands",
                        txt,
                        msg=f"Layering violation: {rel} must not depend on crate::commands",
                    )

        for rs_file in iter_rs_files(os.path.join(src_root, "session")):
            rel = os.path.relpath(rs_file, REPO_ROOT)
            txt = read_non_comment_code(rs_file)
            self.assertNotIn(
                "crate::orchestrator",
                txt,
                msg=f"Cycle violation: {rel} must not depend on crate::orchestrator",
            )

        for candidate in (
            os.path.join(src_root, "orchestrator"),
            os.path.join(src_root, "orchestrator.rs"),
        ):
            for rs_file in iter_rs_files(candidate):
                rel = os.path.relpath(rs_file, REPO_ROOT)
                txt = read_non_comment_code(rs_file)
                self.assertNotIn(
                    "crate::session",
                    txt,
                    msg=f"Cycle violation: {rel} must not depend on crate::session",
                )

        over_budget = []
        for rs_file in iter_rs_files(src_root):
            rel = os.path.relpath(rs_file, REPO_ROOT)
            with open(rs_file, encoding="utf-8") as f:
                line_count = sum(1 for _ in f)
            if line_count > 300:
                over_budget.append(f"{rel} ({line_count} lines)")
        self.assertEqual(
            over_budget,
            [],
            msg=f"Rust files exceeding 300-line budget: {', '.join(over_budget)}",
        )

    def test_release_version_parity_and_arch_contract_exist(self):
        import json
        import re

        self.assertTrue(
            os.path.isfile(os.path.join(REPO_ROOT, "docs", "ARCH_v0.10.md"))
        )
        with open(os.path.join(REPO_ROOT, "Cargo.toml"), encoding="utf-8") as f:
            cargo_ver = re.search(r'(?m)^version\s*=\s*"([^"]+)"', f.read()).group(1)
        with open(os.path.join(REPO_ROOT, "Cargo.lock"), encoding="utf-8") as f:
            lock_ver = re.search(
                r'(?ms)name\s*=\s*"ccide-core"\s*\nversion\s*=\s*"([^"]+)"', f.read()
            ).group(1)
        with open(
            os.path.join(REPO_ROOT, ".claude-plugin", "plugin.json"), encoding="utf-8"
        ) as f:
            plugin_ver = json.load(f)["version"]
        with open(
            os.path.join(REPO_ROOT, ".claude-plugin", "marketplace.json"),
            encoding="utf-8",
        ) as f:
            mkt = json.load(f)
            mkt_ver = mkt["version"]
            mkt_p0_ver = mkt["plugins"][0]["version"]
        with open(os.path.join(REPO_ROOT, "install.sh"), encoding="utf-8") as f:
            inst_ver = re.search(r'(?m)^PLUGIN_VERSION="([^"]+)"', f.read()).group(1)
        with open(os.path.join(REPO_ROOT, "install.ps1"), encoding="utf-8") as f:
            ps1_ver = re.search(r'(?m)^\$PluginVersion\s*=\s*"([^"]+)"', f.read()).group(1)
        self.assertEqual(
            {cargo_ver, lock_ver, plugin_ver, mkt_ver, mkt_p0_ver, inst_ver, ps1_ver},
            {cargo_ver},
        )


if __name__ == "__main__":
    unittest.main()
