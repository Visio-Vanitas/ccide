#!/usr/bin/env python3
"""Hermetic unit tests for Issue #6 (`iterm-preferences.py` & `iterm-pyenv.py`)."""

import contextlib
import http.server
import importlib.util
import io
import os
import plistlib
import socketserver
import tempfile
import threading
import time
import unittest
from unittest import mock

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


def _load_script(name: str, filename: str):
    path = os.path.join(REPO_ROOT, "scripts", filename)
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


iterm_prefs = _load_script("iterm_preferences", "iterm-preferences.py")
iterm_pyenv = _load_script("iterm_pyenv", "iterm-pyenv.py")


class _DelayedMirrorHandler(http.server.BaseHTTPRequestHandler):
    delay_s = 0.0
    status_code = 200

    def do_GET(self):  # noqa: N802
        if self.delay_s > 0:
            time.sleep(self.delay_s)
        self.send_response(self.status_code)
        self.end_headers()
        self.wfile.write(b"<html><body>iterm2</body></html>")

    def log_message(self, format, *args):  # noqa: A002
        del format, args


@contextlib.contextmanager
def _local_mirror_server(delay_s: float = 0.0, status_code: int = 200):
    class Handler(_DelayedMirrorHandler):
        pass

    Handler.delay_s = delay_s
    Handler.status_code = status_code
    srv = socketserver.TCPServer(("127.0.0.1", 0), Handler)
    port = srv.server_address[1]
    th = threading.Thread(target=srv.serve_forever, daemon=True)
    th.start()
    try:
        yield f"http://127.0.0.1:{port}/simple"
    finally:
        srv.shutdown()
        srv.server_close()


class ITermPreferencesAndPyenvTests(unittest.TestCase):
    def test_preferences_apply_idempotency_and_exact_restore(self):
        with tempfile.TemporaryDirectory() as td:
            plist_path = os.path.join(td, "com.googlecode.iterm2.plist")
            bak_dir = os.path.join(td, "backups")
            orig_data = {
                "ShowPaneTitles": True,
                "DimInactiveSplitPanes": True,
                "SplitPaneDimmingAmount": 0.42,
                # Note: `PerPaneBackgroundImage`, `ShowPaneTitlesEvenIfOnlyOnePane`, `EnableAPIServer` absent!
            }
            with open(plist_path, "wb") as f:
                plistlib.dump(orig_data, f)

            env = {
                "CCIDE_ITERM_PLIST": plist_path,
                "CCIDE_ITERM_BACKUP_DIR": bak_dir,
                "CCIDE_PYENV_DIR": os.path.join(td, "pyenv"),
            }
            with mock.patch.dict(os.environ, env, clear=False):
                out1 = io.StringIO()
                with contextlib.redirect_stdout(out1):
                    rc = iterm_prefs.apply_preferences(td)
                self.assertEqual(rc, 0)
                txt1 = out1.getvalue()
                self.assertIn("[PREF] ShowPaneTitles: True -> False", txt1)
                self.assertIn("[PREF] PerPaneBackgroundImage: <unset> -> False", txt1)
                self.assertIn("[PREF] EnableAPIServer: <unset> -> True", txt1)

                applied = iterm_prefs.load_plist_dict(plist_path)
                self.assertIs(applied["ShowPaneTitles"], False)
                self.assertIs(applied["ShowPaneTitlesEvenIfOnlyOnePane"], False)
                self.assertIs(applied["PerPaneBackgroundImage"], False)
                self.assertIs(applied["DimInactiveSplitPanes"], False)
                self.assertIs(applied["EnableAPIServer"], True)
                self.assertAlmostEqual(applied["SplitPaneDimmingAmount"], 0.42)

                # Second apply must be 100% idempotent (0 writes)
                out2 = io.StringIO()
                with contextlib.redirect_stdout(out2):
                    iterm_prefs.apply_preferences(td)
                self.assertIn("0 writes", out2.getvalue())

                # Restore must reinstate original values AND delete keys that were originally absent!
                out3 = io.StringIO()
                with contextlib.redirect_stdout(out3):
                    iterm_prefs.restore_preferences(td, assume_yes=True, purge_venv=False)
                restored = iterm_prefs.load_plist_dict(plist_path)
                self.assertEqual(restored, orig_data)

    def test_skips_cleanly_when_iterm_not_installed(self):
        with tempfile.TemporaryDirectory() as td:
            env = {
                "CCIDE_ITERM_APP": os.path.join(td, "nonexistent.app"),
                "CCIDE_ITERM_PLIST": "",
            }
            with mock.patch.dict(os.environ, env, clear=False):
                with mock.patch.object(iterm_prefs, "discover_iterm_bundle", return_value=None):
                    buf = io.StringIO()
                    with contextlib.redirect_stdout(buf):
                        rc = iterm_prefs.apply_preferences(td)
                    self.assertEqual(rc, 0)
                    self.assertIn("Skipping iTerm2 configuration", buf.getvalue())

    def test_pypi_mirror_respects_user_index_and_selects_lowest_latency_with_cache(self):
        with tempfile.TemporaryDirectory() as td:
            cache_file = os.path.join(td, "pypi_mirror.json")

            # 1. Explicit PIP_INDEX_URL must never be overridden by mirror probing
            with mock.patch.dict(
                os.environ,
                {"PIP_INDEX_URL": "https://corp.internal/pypi/simple"},
                clear=False,
            ):
                chosen, src = iterm_pyenv.select_pypi_index("/usr/bin/python3", cache_file, mirrors=[])
                self.assertEqual(chosen, "https://corp.internal/pypi/simple")
                self.assertEqual(src, "user-configured")

            # 2. Concurrent probe selects fastest HTTP 200 mirror and writes 24h TTL cache
            with _local_mirror_server(delay_s=0.14, status_code=200) as slow_url, _local_mirror_server(
                delay_s=0.0, status_code=200
            ) as fast_url, _local_mirror_server(delay_s=0.0, status_code=503) as dead_url:
                with mock.patch.dict(os.environ, {"PIP_INDEX_URL": ""}, clear=False):
                    with mock.patch.object(iterm_pyenv, "detect_user_pip_index", return_value=None):
                        chosen2, src2 = iterm_pyenv.select_pypi_index(
                            "/usr/bin/python3",
                            cache_file,
                            mirrors=[("slow", slow_url), ("fast", fast_url), ("dead", dead_url)],
                        )
                        self.assertEqual(chosen2, fast_url)
                        self.assertIn("fast", src2)
                        self.assertTrue(os.path.isfile(cache_file))

                        # Subsequent call hits 24h cache without probing network
                        chosen3, src3 = iterm_pyenv.select_pypi_index(
                            "/usr/bin/python3", cache_file, mirrors=[]
                        )
                        self.assertEqual(chosen3, fast_url)
                        self.assertEqual(src3, "cached")

    def test_offline_all_mirrors_unreachable_falls_back_gracefully_to_tier3(self):
        with tempfile.TemporaryDirectory() as td:
            env = {
                "CCIDE_PYENV_DIR": os.path.join(td, "pyenv"),
                "CCIDE_CACHE_DIR": os.path.join(td, "cache"),
                "CCIDE_FORCE_VENV": "1",
                "PIP_INDEX_URL": "",
            }
            with _local_mirror_server(delay_s=0.0, status_code=503) as offline_url:
                with mock.patch.dict(os.environ, env, clear=False):
                    with mock.patch.object(iterm_pyenv, "detect_user_pip_index", return_value=None):
                        buf = io.StringIO()
                        with contextlib.redirect_stdout(buf):
                            rc = iterm_pyenv.ensure_iterm_pyenv(td, mirrors=[("off", offline_url)])
                        self.assertEqual(rc, 0)
                        out = buf.getvalue()
                        self.assertIn("Tier 3 (`tmux`)", out)
                        self.assertIn("-m venv", out)


if __name__ == "__main__":
    unittest.main()
