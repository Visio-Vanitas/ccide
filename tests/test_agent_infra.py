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
        self.assertEqual(brief2["acceptance_criteria"], ["Splits 30% | 54% | 16% in Ghostty"])

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


if __name__ == "__main__":
    unittest.main()
