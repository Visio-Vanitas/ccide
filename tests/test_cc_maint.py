#!/usr/bin/env python3
"""Unit tests for Cross-OS Bun PE parsing (`bun_pe.py`) and Maintenance Workbench (`cc-maint.py`)."""

import importlib.util
import json
import os
import struct
import sys
import unittest

_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
_SCRIPTS_DIR = os.path.join(_REPO_ROOT, "scripts")
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

import bun_pe  # type: ignore[import-not-found]  # noqa: E402
import cc_schema_inspector as inspector  # type: ignore[import-not-found]  # noqa: E402
import patch_claude_binary as patcher  # type: ignore[import-not-found]  # noqa: E402

_spec = importlib.util.spec_from_file_location("cc_maint", os.path.join(_SCRIPTS_DIR, "cc-maint.py"))
assert _spec and _spec.loader
cc_maint = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cc_maint)

BUN_MARKER = b"\n---- Bun! ----\n"


class WindowsBunPeAndMaintWorkbenchTests(unittest.TestCase):
    def test_parses_windows_pe_bun_drive_prefix_with_authenticode_trailer(self):
        js_code = b'function fnEnter(a,b){return"tengu_transcript_view_enter"}\n'
        mod_name = b"B:/~BUN/root/claude.exe"
        payload = bytearray(js_code + mod_name)
        table_off = len(payload)
        entry = [0] * 13
        entry[0] = len(js_code)
        entry[1] = len(mod_name)
        entry[2] = 0
        entry[3] = len(js_code)
        entry[6] = 0x4000
        entry[7] = 0x200
        entry[12] = 0x101
        payload += struct.pack("<13I", *entry)
        byte_count = len(payload)
        payload += struct.pack("<QII", byte_count, table_off, 52) + b"\x00" * 16 + BUN_MARKER
        # Append a synthetic 2048-byte Windows PE Authenticode signature block AFTER the Bun marker!
        authenticode_cert = b"\x30\x82\x07\xfc" + b"\xab" * 2044
        pe_buf = bytearray(b"MZ" + b"\x00" * 4094 + payload + authenticode_cert)

        layout = bun_pe.parse_bun_layout(pe_buf)
        self.assertIsNotNone(layout)
        assert layout is not None
        self.assertEqual(len(layout.js_modules), 1)
        self.assertEqual(layout.js_modules[0].name, "B:/~BUN/root/claude.exe")

        cleared = bun_pe.clear_bun_bytecode_for_offsets(pe_buf, layout, [4100])
        self.assertEqual(cleared, [(0, "B:/~BUN/root/claude.exe")])

    def test_anchor_snippet_extraction_and_schema_diff(self):
        sample_buf = (
            b'prefix_pad;function driftFn(){let k="quietlyParked";return k}suffix_pad;'
            b"var h={exceeds_200k_tokens:!1,session_id:a,cwd:b,model:c,workspace:d,cost:e,brand_new_field:1};"
            b"let u={cache_creation_input_tokens:4,input_tokens:1,output_tokens:2,iterations:[],future_usage_metric:9};"
        )
        snip = inspector.extract_anchor_snippet(sample_buf, "quietlyParked", radius=30)
        self.assertIn("quietlyParked", snip)

        with open(os.path.join(_REPO_ROOT, "assets", "cc-schema.json"), encoding="utf-8") as f:
            baseline = json.load(f)
        diff = inspector.extract_live_schema(sample_buf, baseline)
        self.assertIn("brand_new_field", diff["statusline_keys"]["added"])
        self.assertIn("future_usage_metric", diff["usage_keys"]["added"])

    def test_injected_windows_and_unix_hook_has_no_unconditional_sigusr2(self):
        from test_patch_claude_binary import build_bun_standalone, build_js_bundle

        buf, _, _ = build_bun_standalone(build_js_bundle())
        rep = patcher.patch_buffer_in_memory(buf)
        self.assertTrue(rep["required_ok"])
        self.assertEqual(rep["syntax_errors"], [])
        raw = bytes(buf)
        self.assertIn(b'process.platform==="win32"', raw)
        self.assertIn(b"fs.watch(", raw)
        self.assertIn(b'"wake-pid"', raw)


if __name__ == "__main__":
    unittest.main()
