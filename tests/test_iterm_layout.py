#!/usr/bin/env python3
"""Unit tests for iTerm2 3-column layout math in ccide_daemon.py and iterm2_workspace.py."""

import importlib.machinery
import importlib.util
import os
import sys
import tempfile
import types
import unittest

_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


def _make_iterm2_stub() -> dict[str, types.ModuleType]:
    iterm2_mod = types.ModuleType("iterm2")
    iterm2_mod.App = object
    iterm2_mod.Session = object
    iterm2_mod.Tab = object
    iterm2_mod.LocalWriteOnlyProfile = object
    iterm2_mod.Selection = lambda items: items
    iterm2_mod.VariableScopes = types.SimpleNamespace(SESSION="session")
    iterm2_mod.InitialWorkingDirectory = types.SimpleNamespace(
        INITIAL_WORKING_DIRECTORY_CUSTOM="custom"
    )
    iterm2_mod.VariableMonitor = object
    iterm2_mod.NewSessionMonitor = object
    iterm2_mod.SessionTerminationMonitor = object
    iterm2_mod.FocusMonitor = object
    iterm2_mod.async_get_app = lambda *a, **kw: None
    iterm2_mod.run_forever = lambda *a, **kw: None
    iterm2_mod.run_until_complete = lambda *a, **kw: None

    notif_mod = types.ModuleType("iterm2.notifications")
    notif_mod.async_subscribe_to_custom_escape_sequence_notification = lambda *a, **kw: None
    iterm2_mod.notifications = notif_mod

    util_mod = types.ModuleType("iterm2.util")
    util_mod.Size = lambda w, h: (w, h)
    iterm2_mod.util = util_mod

    return {
        "iterm2": iterm2_mod,
        "iterm2.notifications": notif_mod,
        "iterm2.util": util_mod,
    }


def _load_with_iterm2_stub(mod_name: str, rel_path: str):
    full_path = os.path.join(_REPO_ROOT, rel_path)
    stubs = _make_iterm2_stub()
    prev_mods = {k: sys.modules.get(k) for k in stubs}
    prev_ipc = os.environ.get("CCIDE_IPC_DIR")
    try:
        for k, v in stubs.items():
            sys.modules[k] = v
        if not prev_ipc:
            os.environ["CCIDE_IPC_DIR"] = os.path.join(tempfile.gettempdir(), "ccide-test-ipc")
        loader = importlib.machinery.SourceFileLoader(mod_name, full_path)
        spec = importlib.util.spec_from_loader(mod_name, loader)
        assert spec is not None
        mod = importlib.util.module_from_spec(spec)
        loader.exec_module(mod)
        return mod
    finally:
        for k, old in prev_mods.items():
            if old is None:
                sys.modules.pop(k, None)
            else:
                sys.modules[k] = old
        if prev_ipc is None:
            os.environ.pop("CCIDE_IPC_DIR", None)
        else:
            os.environ["CCIDE_IPC_DIR"] = prev_ipc


class ItermLayoutTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls._had_iterm2 = "iterm2" in sys.modules
        cls.daemon = _load_with_iterm2_stub("ccide_daemon_layout_test", "scripts/ccide_daemon.py")
        cls.workspace = _load_with_iterm2_stub(
            "iterm2_workspace_layout_test", "scripts/iterm2_workspace.py"
        )

    def test_iterm2_stub_does_not_leak_into_sys_modules(self) -> None:
        if not self._had_iterm2:
            self.assertNotIn("iterm2", sys.modules)
            self.assertNotIn("iterm2.notifications", sys.modules)
            self.assertNotIn("iterm2.util", sys.modules)

    def test_daemon_and_workspace_compute_identical_column_widths(self) -> None:
        widths = [40, 60, 70, 80, 90, 97, 98, 100, 120, 150, 160, 174, 200, 220, 240, 268, 320]
        for total_w in widths:
            for left_pct in (5, 12, 20, 25, 30, 40, 45, 60):
                for has_right in (True, False):
                    got_d = self.daemon.compute_iterm_column_widths(total_w, left_pct, has_right)
                    got_w = self.workspace.compute_iterm_column_widths(total_w, left_pct, has_right)
                    self.assertEqual(
                        got_d,
                        got_w,
                        f"mismatch for total_w={total_w}, left_pct={left_pct}, has_right={has_right}",
                    )

    def test_conservation_invariant_never_overflows(self) -> None:
        widths = [40, 60, 80, 90, 100, 120, 150, 160, 174, 200, 220, 240, 268, 320]
        for mod in (self.daemon, self.workspace):
            for total_w in widths:
                for left_pct in (12, 25, 30, 45):
                    for has_right in (True, False):
                        left_w, mid_w, right_w = mod.compute_iterm_column_widths(
                            total_w, left_pct, has_right
                        )
                        self.assertEqual(
                            left_w + mid_w + right_w,
                            total_w,
                            f"sum overflow for {mod.__name__} total_w={total_w}, lp={left_pct}, hr={has_right}",
                        )
                        self.assertGreaterEqual(left_w, 0)
                        self.assertGreaterEqual(mid_w, 1)
                        self.assertGreaterEqual(right_w, 0)
                        if not has_right:
                            self.assertEqual(right_w, 0)

    def test_golden_values_matching_layout_plan(self) -> None:
        cases = [
            ((268, 30, True), (80, 145, 43)),
            ((174, 30, True), (52, 94, 28)),
            ((160, 30, True), (48, 84, 28)),
            ((120, 30, True), (36, 56, 28)),
            ((100, 30, True), (24, 48, 28)),
            ((200, 30, True), (60, 108, 32)),
            ((240, 30, True), (72, 130, 38)),
            ((320, 30, True), (96, 173, 51)),
            ((120, 30, False), (36, 84, 0)),
        ]
        for mod in (self.daemon, self.workspace):
            for args, expected in cases:
                got = mod.compute_iterm_column_widths(*args)
                self.assertEqual(got, expected, f"{mod.__name__} args={args} got={got}")
            # Explicit regression assertions for the old 268-col 86/123/59 overflow bug
            _, mid_268, right_268 = mod.compute_iterm_column_widths(268, 30, True)
            self.assertGreaterEqual(mid_268, 140)
            self.assertLessEqual(right_268, 45)

    def test_left_pct_clamping_to_12_and_45(self) -> None:
        for mod in (self.daemon, self.workspace):
            for total_w in (100, 160, 240, 268):
                for has_right in (True, False):
                    lo = mod.compute_iterm_column_widths(total_w, 12, has_right)
                    for below in (-5, 0, 5, 11):
                        self.assertEqual(
                            mod.compute_iterm_column_widths(total_w, below, has_right), lo
                        )
                    hi = mod.compute_iterm_column_widths(total_w, 45, has_right)
                    for above in (46, 60, 90):
                        self.assertEqual(
                            mod.compute_iterm_column_widths(total_w, above, has_right), hi
                        )


if __name__ == "__main__":
    unittest.main()
