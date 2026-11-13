#!/usr/bin/env python3
"""
iterm-preferences.py — Reversible iTerm2 Bundle Sniffer & Recommended Preference Manager (`cc-ide`).

CLI:
  python3 -I scripts/iterm-preferences.py --status [--json]
  python3 -I scripts/iterm-preferences.py --apply
  python3 -I scripts/iterm-preferences.py --restore [--yes] [--purge-venv]
"""
import argparse
import datetime
import json
import os
import platform
import plistlib
import shutil
import subprocess
import sys

RECOMMENDED_PREFS: dict[str, bool] = {
    "ShowPaneTitles": False,
    "ShowPaneTitlesEvenIfOnlyOnePane": False,
    "PerPaneBackgroundImage": False,
    "DimInactiveSplitPanes": False,
    "EnableAPIServer": True,
}


def discover_iterm_bundle(home_dir: str) -> str | None:
    override = os.environ.get("CCIDE_ITERM_APP", "").strip()
    if override:
        return override if os.path.isdir(override) else None
    if platform.system() != "Darwin":
        return None
    for cand in ("/Applications/iTerm.app", os.path.join(home_dir, "Applications", "iTerm.app")):
        if os.path.isdir(cand):
            return cand
    if shutil.which("mdfind"):
        try:
            proc = subprocess.run(
                ["mdfind", "kMDItemCFBundleIdentifier == 'com.googlecode.iterm2'"],
                capture_output=True,
                text=True,
                timeout=3.0,
                check=False,
            )
            for line in proc.stdout.splitlines():
                p = line.strip()
                if p.endswith(".app") and os.path.isdir(p):
                    return p
        except Exception:
            pass
    return None


def resolve_paths(home_dir: str) -> tuple[str, str, bool]:
    custom_plist = os.environ.get("CCIDE_ITERM_PLIST", "").strip()
    plist_path = custom_plist or os.path.join(
        home_dir, "Library", "Preferences", "com.googlecode.iterm2.plist"
    )
    backup_dir = os.environ.get("CCIDE_ITERM_BACKUP_DIR", "").strip() or os.path.join(
        home_dir, ".config", "cc-ide", "backups", "iterm2"
    )
    return plist_path, backup_dir, bool(custom_plist)


def load_plist_dict(plist_path: str) -> dict:
    if not os.path.isfile(plist_path):
        return {}
    try:
        with open(plist_path, "rb") as f:
            data = plistlib.load(f)
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def write_pref_key(plist_path: str, use_file_mode: bool, key: str, val: bool | None) -> None:
    if use_file_mode or platform.system() != "Darwin" or not shutil.which("defaults"):
        cur = load_plist_dict(plist_path)
        os.makedirs(os.path.dirname(plist_path), exist_ok=True)
        if val is None:
            cur.pop(key, None)
        else:
            cur[key] = val
        tmp = plist_path + ".tmp"
        with open(tmp, "wb") as f:
            plistlib.dump(cur, f)
        os.replace(tmp, plist_path)
        return

    if val is None:
        subprocess.run(
            ["defaults", "delete", "com.googlecode.iterm2", key],
            capture_output=True,
            check=False,
        )
    else:
        subprocess.run(
            ["defaults", "write", "com.googlecode.iterm2", key, "-bool", "true" if val else "false"],
            check=True,
        )


def collect_status(home_dir: str) -> dict:
    plist_path, backup_dir, use_file_mode = resolve_paths(home_dir)
    bundle = discover_iterm_bundle(home_dir)
    installed = bool(bundle) or use_file_mode
    cur = load_plist_dict(plist_path)
    prefs_state = {}
    pending_changes = 0
    for k, rec in RECOMMENDED_PREFS.items():
        present = k in cur
        actual = bool(cur[k]) if present else None
        matches = present and (actual is rec)
        if not matches:
            pending_changes += 1
        prefs_state[k] = {"current": actual, "recommended": rec, "matches": matches}
    return {
        "platform": sys.platform,
        "iterm_installed": installed,
        "bundle_path": bundle,
        "plist_path": plist_path,
        "backup_dir": backup_dir,
        "api_enabled": bool(cur.get("EnableAPIServer", False)),
        "pending_changes": pending_changes,
        "preferences": prefs_state,
    }


def apply_preferences(home_dir: str) -> int:
    plist_path, backup_dir, use_file_mode = resolve_paths(home_dir)
    bundle = discover_iterm_bundle(home_dir)
    if not bundle and not use_file_mode:
        print("  [SKIP] Skipping iTerm2 configuration (iTerm2 not installed); Tier 3 tmux backend ready.")
        return 0

    cur = load_plist_dict(plist_path)
    if not bool(cur.get("EnableAPIServer", False)):
        print(
            "  [INFO] iTerm2 Python API (`EnableAPIServer`) was disabled.\n"
            "         Enabling via defaults (if prompted inside iTerm2, confirm under:\n"
            "         iTerm2 -> Settings -> General -> Magic -> Enable Python API,\n"
            "         or run: `defaults write com.googlecode.iterm2 EnableAPIServer -bool true`)."
        )

    diffs: list[tuple[str, object, bool]] = []
    for key, rec_val in RECOMMENDED_PREFS.items():
        if key not in cur or bool(cur[key]) is not rec_val:
            old_repr = cur[key] if key in cur else "<unset>"
            diffs.append((key, old_repr, rec_val))

    if not diffs:
        print("  [OK] iTerm2 preferences already match ccide recommendations (idempotent, 0 writes).")
        return 0

    os.makedirs(backup_dir, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    if os.path.isfile(plist_path):
        plist_bak = os.path.join(backup_dir, f"com.googlecode.iterm2.{stamp}.plist")
        shutil.copy2(plist_path, plist_bak)
        print(f"  [OK] Backed up iTerm2 plist -> {plist_bak}")

    snap_entries = {
        k: ({"absent": False, "value": bool(cur[k])} if k in cur else {"absent": True})
        for k in RECOMMENDED_PREFS
    }
    snap_payload = {"created_at": stamp, "plist_path": plist_path, "keys": snap_entries}
    baseline_path = os.path.join(backup_dir, "baseline_snapshot.json")
    latest_path = os.path.join(backup_dir, "latest_snapshot.json")
    if not os.path.isfile(baseline_path):
        with open(baseline_path, "w", encoding="utf-8") as f:
            json.dump(snap_payload, f, indent=2)
    with open(latest_path, "w", encoding="utf-8") as f:
        json.dump(snap_payload, f, indent=2)

    for key, old_repr, rec_val in diffs:
        write_pref_key(plist_path, use_file_mode, key, rec_val)
        print(f"  [PREF] {key}: {old_repr} -> {rec_val}")

    return 0


def restore_preferences(home_dir: str, assume_yes: bool, purge_venv: bool) -> int:
    plist_path, backup_dir, use_file_mode = resolve_paths(home_dir)
    snap_file = os.path.join(backup_dir, "baseline_snapshot.json")
    if not os.path.isfile(snap_file):
        snap_file = os.path.join(backup_dir, "latest_snapshot.json")

    if os.path.isfile(snap_file):
        with open(snap_file, encoding="utf-8") as f:
            snap = json.load(f)
        cur = load_plist_dict(plist_path)
        restored = 0
        for key, meta in snap.get("keys", {}).items():
            cur_repr = cur[key] if key in cur else "<unset>"
            if meta.get("absent"):
                if key in cur:
                    write_pref_key(plist_path, use_file_mode, key, None)
                    print(f"  [RESTORE] {key}: {cur_repr} -> <unset>")
                    restored += 1
            else:
                orig_val = bool(meta.get("value"))
                if key not in cur or bool(cur[key]) is not orig_val:
                    write_pref_key(plist_path, use_file_mode, key, orig_val)
                    print(f"  [RESTORE] {key}: {cur_repr} -> {orig_val}")
                    restored += 1
        if restored == 0:
            print("  [OK] iTerm2 preferences already match backed-up original values.")
    else:
        print("  [SKIP] No ccide iTerm2 preference backup snapshot found; nothing to restore.")

    venv_dir = os.environ.get("CCIDE_PYENV_DIR", "").strip() or os.path.join(
        home_dir, ".local", "share", "cc-ide", "pyenv"
    )
    py_cache = os.path.join(home_dir, ".cache", "cc-ide", "python_bin")
    if os.path.isdir(venv_dir):
        do_remove = False
        if purge_venv and assume_yes:
            do_remove = True
        elif sys.stdin.isatty() and (purge_venv or not assume_yes):
            ans = input(f"  Remove dedicated iTerm2 venv at {venv_dir}? [y/N]: ").strip().lower()
            do_remove = ans in ("y", "yes")
        if do_remove:
            shutil.rmtree(venv_dir, ignore_errors=True)
            if os.path.isfile(py_cache):
                os.remove(py_cache)
            print(f"  [OK] Removed dedicated iTerm2 venv -> {venv_dir}")
    return 0


def main() -> None:
    ap = argparse.ArgumentParser(description="cc-ide Reversible iTerm2 Preference Controller")
    ap.add_argument("--status", action="store_true", help="Inspect iTerm2 bundle & preference state")
    ap.add_argument("--json", action="store_true", help="Output machine-readable JSON status")
    ap.add_argument("--apply", action="store_true", help="Back up plist and apply recommended keys")
    ap.add_argument("--restore", action="store_true", help="Restore backed-up iTerm2 preference keys")
    ap.add_argument("--purge-venv", action="store_true", help="Also remove ~/.local/share/cc-ide/pyenv")
    ap.add_argument("--yes", "-y", action="store_true", help="Confirm prompts non-interactively")
    args = ap.parse_args()

    home_dir = os.environ.get("HOME", os.path.expanduser("~"))
    if args.apply:
        sys.exit(apply_preferences(home_dir))
    if args.restore:
        sys.exit(restore_preferences(home_dir, args.yes, args.purge_venv))

    st = collect_status(home_dir)
    if args.json:
        print(json.dumps(st, indent=2))
    else:
        print(f"iTerm2 bundle: {st['bundle_path'] or 'not found'}")
        for k, info in st["preferences"].items():
            print(f"  {k}: current={info['current']} recommended={info['recommended']}")


if __name__ == "__main__":
    main()
