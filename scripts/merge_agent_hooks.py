#!/usr/bin/env python3
"""Non-destructive, idempotent, reversible hook merger for agent configs (Codex hooks.json)."""

from __future__ import annotations

import argparse, copy, json, os, shlex, shutil, sys


def is_ccide_entry(obj) -> bool:
    if isinstance(obj, str):
        return "ccide-core" in obj or "cc-ide" in obj
    if isinstance(obj, list):
        return any(is_ccide_entry(x) for x in obj)
    if isinstance(obj, dict):
        return any(is_ccide_entry(v) for v in obj.values())
    return False


def _rewrite_cmd(cmd: str, hook_bin: str, win_mode: bool) -> str:
    quoted = f'"{hook_bin}"' if (win_mode and " " in hook_bin) else (shlex.quote(hook_bin) if not win_mode else hook_bin)
    for prefix in ("./hook.sh", "hook.sh", "./hook.cmd", "hook.cmd"):
        if cmd == prefix or cmd.startswith(prefix + " "):
            return quoted + cmd[len(prefix) :]
    return cmd


def _rewrite_group(group: dict, hook_bin_unix: str, hook_bin_win: str) -> dict:
    out = copy.deepcopy(group)
    hooks = out.get("hooks")
    if isinstance(hooks, list):
        for h in hooks:
            if not isinstance(h, dict):
                continue
            if isinstance(h.get("command"), str):
                h["command"] = _rewrite_cmd(h["command"], hook_bin_unix, False)
            if isinstance(h.get("commandWindows"), str):
                h["commandWindows"] = _rewrite_cmd(h["commandWindows"], hook_bin_win, True)
    return out


def _clean_group(group) -> dict | None:
    if not isinstance(group, dict):
        return None if is_ccide_entry(group) else group
    hooks = group.get("hooks")
    if isinstance(hooks, list):
        kept_hooks = [h for h in hooks if not is_ccide_entry(h)]
        if not kept_hooks:
            return None
        if len(kept_hooks) == len(hooks):
            return group
        out = copy.deepcopy(group)
        out["hooks"] = kept_hooks
        return out
    return None if is_ccide_entry(group) else group


def _atomic_write_json(target: str, data: dict) -> None:
    parent = os.path.dirname(os.path.abspath(target))
    os.makedirs(parent, exist_ok=True)
    if os.path.isfile(target) and not os.path.exists(target + ".ccide.bak"):
        shutil.copy2(target, target + ".ccide.bak")
    tmp = f"{target}.tmp.{os.getpid()}"
    payload = json.dumps(data, indent=2, ensure_ascii=False) + "\n"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(payload)
    os.replace(tmp, target)


def merge_hooks(target: str, template_path: str | None = None, hook_bin: str | None = None, remove: bool = False) -> bool:
    if remove:
        if not os.path.isfile(target):
            return False
        with open(target, encoding="utf-8") as f:
            orig = json.load(f)
        if not isinstance(orig, dict) or not isinstance(orig.get("hooks"), dict):
            return False
        data = copy.deepcopy(orig)
        for ev in list(data["hooks"].keys()):
            groups = data["hooks"][ev]
            if not isinstance(groups, list):
                continue
            cleaned = [g for g in (_clean_group(x) for x in groups) if g is not None]
            if cleaned:
                data["hooks"][ev] = cleaned
            else:
                del data["hooks"][ev]
        if not data["hooks"]:
            del data["hooks"]
        if data == orig:
            return False
        _atomic_write_json(target, data)
        return True

    if not template_path:
        repo_root = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
        fname = "hooks.windows.json" if os.name == "nt" else "hooks.json"
        template_path = os.path.join(repo_root, "integrations", "codex", fname)
    with open(template_path, encoding="utf-8") as f:
        tpl = json.load(f)

    orig: dict = {}
    if os.path.isfile(target):
        with open(target, encoding="utf-8") as f:
            loaded = json.load(f)
        if not isinstance(loaded, dict):
            raise ValueError(f"Expected JSON object in {target}")
        orig = loaded

    win_mode = template_path.endswith(".windows.json") or os.name == "nt"
    base_dir = os.path.dirname(os.path.abspath(hook_bin)) if hook_bin else os.path.join(os.path.dirname(os.path.abspath(target)), "cc-ide")
    if hook_bin and hook_bin.lower().endswith(".cmd"):
        hook_bin_win = hook_bin
        hook_bin_unix = os.path.join(base_dir, "hook.sh")
    elif hook_bin:
        hook_bin_unix = hook_bin
        hook_bin_win = os.path.join(base_dir, "hook.cmd")
    else:
        hook_bin_unix = os.path.join(base_dir, "hook.sh")
        hook_bin_win = os.path.join(base_dir, "hook.cmd")
    _ = win_mode

    data = copy.deepcopy(orig)
    hooks_obj = data.setdefault("hooks", {})
    if not isinstance(hooks_obj, dict):
        raise ValueError(f"Expected object for 'hooks' in {target}")

    for ev, tpl_groups in tpl.get("hooks", {}).items():
        if not isinstance(tpl_groups, list):
            continue
        cur = hooks_obj.get(ev, [])
        if not isinstance(cur, list):
            raise ValueError(f"Expected array for hooks.{ev} in {target}")
        kept = [g for g in (_clean_group(x) for x in cur) if g is not None]
        added = [_rewrite_group(g, hook_bin_unix, hook_bin_win) for g in tpl_groups if isinstance(g, dict)]
        hooks_obj[ev] = kept + added

    if os.path.isfile(target) and data == orig:
        return False
    _atomic_write_json(target, data)
    return True


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description="Merge/remove cc-ide hooks into Codex hooks.json")
    default_codex = os.environ.get("CODEX_HOME") or os.path.expanduser("~/.codex")
    p.add_argument("--target", default=os.path.join(default_codex, "hooks.json"))
    p.add_argument("--template", default=None)
    p.add_argument("--hook-bin", default=None)
    p.add_argument("--remove", action="store_true")
    p.add_argument("--json", action="store_true")
    ns = p.parse_args(argv)
    try:
        changed = merge_hooks(ns.target, template_path=ns.template, hook_bin=ns.hook_bin, remove=ns.remove)
    except Exception as exc:
        print(f"merge_agent_hooks error: {exc}", file=sys.stderr)
        return 1
    if ns.json:
        print(json.dumps({"target": ns.target, "changed": changed, "removed": ns.remove}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
