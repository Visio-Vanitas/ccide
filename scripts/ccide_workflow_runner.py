#!/usr/bin/env python3
"""Headless Native Claude Code Workflow CLI & macro-expanding runner (stdlib-only)."""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

_DEFAULT_REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def strip_jsonc(text: str) -> str:
    out, i, n, in_str, esc = [], 0, len(text), False, False
    while i < n:
        ch = text[i]
        if in_str:
            out.append(ch)
            esc = (not esc and ch == "\\")
            if not esc and ch == '"':
                in_str = False
            i += 1
        elif ch == '"':
            in_str = True
            out.append(ch)
            i += 1
        elif text[i : i + 2] == "//":
            i = text.find("\n", i + 2)
            if i == -1:
                break
        elif text[i : i + 2] == "/*":
            end = text.find("*/", i + 2)
            i = n if end == -1 else end + 2
        else:
            out.append(ch)
            i += 1
    return re.sub(r",(\s*[}\]])", r"\1", "".join(out))


def load_orchestrator_config(repo_root: str = _DEFAULT_REPO_ROOT) -> dict:
    cfg_home = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    for path in (os.path.join(cfg_home, "cc-ide", "orchestrator.json"), os.path.join(repo_root, "assets", "orchestrator.json")):
        if os.path.isfile(path):
            with open(path, encoding="utf-8") as f:
                return json.loads(strip_jsonc(f.read()))
    return {"active_preset": "balanced", "model_pool": [], "presets": {}}


def resolve_roles(cfg: dict, preset: str | None = None, role_overrides=None):
    preset_name = preset or cfg.get("active_preset") or "balanced"
    preset_map = (cfg.get("presets") or {}).get(preset_name) or {}
    roles = {
        r: {"models": list((preset_map.get(r) or {}).get("models") or ["inherit"]), "effort": (preset_map.get(r) or {}).get("effort") or "medium"}
        for r in ("scout", "coder", "reviewer", "plan", "workflow")
    }
    items = [f"{k}={v}" for k, v in role_overrides.items()] if isinstance(role_overrides, dict) else list(role_overrides or [])
    for spec in items:
        if "=" not in spec:
            continue
        r_name, val = (x.strip() for x in spec.split("=", 1))
        if ":" in val:
            m_val, e_val = val.rsplit(":", 1)
            roles[r_name] = {"models": [m_val.strip()], "effort": e_val.strip()}
        else:
            roles[r_name] = {"models": [val], "effort": (roles.get(r_name) or {}).get("effort", "medium")}
    return preset_name, roles, list(cfg.get("model_pool") or [])


def expand_workflow_macros(script_text: str, roles: dict) -> str:
    out = script_text
    for role, spec in roles.items():
        primary = (spec.get("models") or ["inherit"])[0]
        effort = spec.get("effort") or "medium"
        out = re.sub(rf"(effort\s*:\s*)(['\"])@{re.escape(role)}\2", lambda m, e=effort: f"{m.group(1)}{json.dumps(e)}", out)
        out = re.sub(rf"(['\"])@{re.escape(role)}\1", lambda _, p=primary: json.dumps(p), out)
    return out


def extract_meta(script_text: str, fallback_name: str) -> dict:
    m = re.search(r"export\s+const\s+meta\s*=\s*\{", script_text)
    if not m:
        return {"name": fallback_name, "description": "", "phases": []}
    start, depth, i, n, in_str, quote, esc, block = m.end() - 1, 0, m.end() - 1, len(script_text), False, "", False, ""
    while i < n:
        ch = script_text[i]
        if in_str:
            esc = (not esc and ch == "\\")
            if not esc and ch == quote:
                in_str = False
        elif ch in ("'", '"', "`"):
            in_str, quote = True, ch
        elif ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                block = script_text[start : i + 1]
                break
        i += 1
    if not block:
        return {"name": fallback_name, "description": "", "phases": []}
    name_m = re.search(r"\bname\s*:\s*['\"]([^'\"]+)['\"]", block)
    desc_m = re.search(r"\bdescription\s*:\s*['\"]([^'\"]+)['\"]", block)
    phases = [{"title": p} for p in re.findall(r"title\s*:\s*['\"]([^'\"]+)['\"]", block)]
    return {"name": name_m.group(1) if name_m else fallback_name, "description": desc_m.group(1) if desc_m else "", "phases": phases}


def workflow_search_dirs(cwd: str | None = None, repo_root: str = _DEFAULT_REPO_ROOT) -> list:
    cwd_dir = cwd or os.getcwd()
    return [os.path.join(cwd_dir, ".claude", "workflows"), os.path.join(os.path.expanduser("~"), ".claude", "workflows"), os.path.join(repo_root, "assets", "workflows")]


def discover_workflows(cwd: str | None = None, repo_root: str = _DEFAULT_REPO_ROOT) -> list:
    seen, found = set(), []
    for d in workflow_search_dirs(cwd, repo_root):
        if not os.path.isdir(d):
            continue
        for fn in sorted(os.listdir(d)):
            if not fn.endswith(".js") or fn[:-3] in seen:
                continue
            name, path = fn[:-3], os.path.join(d, fn)
            seen.add(name)
            with open(path, encoding="utf-8") as f:
                meta = extract_meta(f.read(), name)
            found.append({"name": meta["name"], "id": name, "path": path, **meta})
    return found


def build_catalog(cwd: str | None = None, repo_root: str = _DEFAULT_REPO_ROOT) -> dict:
    cfg = load_orchestrator_config(repo_root)
    return {"workflows": discover_workflows(cwd, repo_root), "active_preset": cfg.get("active_preset", "balanced"), "presets": cfg.get("presets", {}), "model_pool": cfg.get("model_pool", [])}


def resolve_workflow_source(target: str | None = None, cwd: str | None = None, file_path: str | None = None, inline_script: str | None = None, repo_root: str = _DEFAULT_REPO_ROOT):
    if inline_script is not None:
        return "<inline>", inline_script
    cand = file_path or target
    if cand and os.path.isfile(cand):
        with open(cand, encoding="utf-8") as f:
            return os.path.abspath(cand), f.read()
    name = (target or "").removesuffix(".js")
    for d in workflow_search_dirs(cwd, repo_root):
        p = os.path.join(d, f"{name}.js")
        if os.path.isfile(p):
            with open(p, encoding="utf-8") as f:
                return p, f.read()
    raise FileNotFoundError(f"Workflow not found: {cand}")


def _extract_stream_events(obj: dict):
    ev_type = obj.get("event") or obj.get("type")
    if ev_type in ("workflow_phase", "workflow_agent", "workflow_log", "tool_result"):
        return [{"event": ev_type, "data": obj.get("data", obj)}]
    events = []
    if isinstance(obj.get("tool_use_result"), dict):
        events.append({"event": "tool_result", "data": obj["tool_use_result"]})
    msg = obj.get("message") if isinstance(obj.get("message"), dict) else None
    for blk in (msg.get("content") if msg and isinstance(msg.get("content"), list) else []):
        if isinstance(blk, dict) and blk.get("type") == "tool_result":
            content = blk.get("content")
            if isinstance(content, str):
                try:
                    content = json.loads(content)
                except Exception:
                    content = None
            if isinstance(content, dict) and ("runId" in content or "transcriptDir" in content):
                events.append({"event": "tool_result", "data": content})
    return events


def execute_workflow_stream(workflow=None, script=None, file_path=None, args=None, preset=None, roles=None, resume_from_run_id=None, cwd=None, dispatcher_model=None, repo_root=_DEFAULT_REPO_ROOT):
    cfg = load_orchestrator_config(repo_root)
    preset_name, resolved_roles, _ = resolve_roles(cfg, preset=preset, role_overrides=roles)
    _, raw_js = resolve_workflow_source(workflow, cwd=cwd, file_path=file_path, inline_script=script, repo_root=repo_root)
    merged_args = dict(args) if isinstance(args, dict) else ({} if args is None else {"value": args})
    merged_args["__ccide"] = {"preset": preset_name, "roles": {r: {"model": s["models"][0], "pool": s["models"], "effort": s["effort"]} for r, s in resolved_roles.items()}}
    dispatch_payload = {"script": expand_workflow_macros(raw_js, resolved_roles), "args": merged_args}
    if resume_from_run_id:
        dispatch_payload["resumeFromRunId"] = resume_from_run_id

    fd, dispatch_path = tempfile.mkstemp(prefix="ccide_wf_", suffix=".json")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(dispatch_payload, f)
        env = os.environ.copy()
        env.pop("CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL", None)
        env.pop("CLAUDE_CODE_SUBAGENT_MODEL_FORCE", None)
        env["CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS"] = "0"
        env["CCIDE_WORKFLOW_DISPATCH_JSON"] = dispatch_path

        disp_model = dispatcher_model or (resolved_roles.get("workflow") or {}).get("models", ["claude-sonnet-4-6"])[0]
        if disp_model == "inherit":
            disp_model = "claude-sonnet-4-6"
        claude_bin = os.environ.get("CCIDE_CLAUDE_BIN") or shutil.which("claude") or "claude"
        cmd = [claude_bin, "-p", "ultracode: Invoke the Workflow tool once now and return its output.", "--model", disp_model, "--effort", "low", "--output-format", "stream-json", "--verbose", "--permission-mode", "bypassPermissions"]
        with subprocess.Popen(cmd, cwd=cwd or os.getcwd(), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1) as proc:
            run_id, transcript_dir, final_result = None, None, None
            assert proc.stdout is not None
            for line in proc.stdout:
                s = line.strip()
                if not s:
                    continue
                try:
                    obj = json.loads(s)
                except Exception:
                    yield {"event": "workflow_log", "data": {"message": s}}
                    continue
                if obj.get("type") == "result" and "result" in obj:
                    final_result = obj.get("result")
                for ev in _extract_stream_events(obj):
                    if ev["event"] == "tool_result" and isinstance(ev["data"], dict):
                        run_id = ev["data"].get("runId") or run_id
                        transcript_dir = ev["data"].get("transcriptDir") or transcript_dir
                        if "result" in ev["data"]:
                            final_result = ev["data"]["result"]
                    yield ev
            rc = proc.wait()
        journal = []
        if transcript_dir and os.path.isfile(os.path.join(transcript_dir, "journal.jsonl")):
            with open(os.path.join(transcript_dir, "journal.jsonl"), encoding="utf-8") as jf:
                journal = [json.loads(jl) for jl in jf if jl.strip()]
        yield {"event": "workflow_result", "data": {"ok": rc == 0, "run_id": run_id, "transcript_dir": transcript_dir, "journal": journal, "result": final_result}}
    finally:
        if os.path.exists(dispatch_path):
            os.remove(dispatch_path)


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="ccide workflow", description="Headless Native Claude Code Workflow Runner")
    sub = p.add_subparsers(dest="cmd", required=True)
    p_list = sub.add_parser("list", help="List available workflows & orchestrator presets")
    p_list.add_argument("--json", action="store_true")
    p_list.add_argument("--cwd", default=None)

    p_run = sub.add_parser("run", help="Run a native workflow")
    p_run.add_argument("workflow", nargs="?", default=None)
    p_run.add_argument("--file", dest="file_path", default=None)
    p_run.add_argument("--script", default=None)
    p_run.add_argument("--args", default="{}")
    p_run.add_argument("--preset", default=None)
    p_run.add_argument("--role", action="append", default=[])
    p_run.add_argument("--resume", dest="resume_id", default=None)
    p_run.add_argument("--cwd", default=None)
    p_run.add_argument("--dispatcher-model", default=None)
    p_run.add_argument("--stream", action="store_true")
    p_run.add_argument("--json", action="store_true")

    ns = p.parse_args(argv)
    if ns.cmd == "list":
        cat = build_catalog(cwd=ns.cwd)
        if ns.json:
            print(json.dumps(cat, indent=2))
        else:
            print(f"Active preset: {cat['active_preset']}")
            for wf in cat["workflows"]:
                print(f"  {wf['name']:18s} {wf.get('description', '')}")
        return 0

    last_res = None
    for ev in execute_workflow_stream(
        workflow=ns.workflow, script=ns.script, file_path=ns.file_path, args=json.loads(ns.args) if ns.args else {},
        preset=ns.preset, roles=ns.role, resume_from_run_id=ns.resume_id, cwd=ns.cwd, dispatcher_model=ns.dispatcher_model,
    ):
        if ns.stream:
            print(json.dumps(ev), flush=True)
        if ev["event"] == "workflow_result":
            last_res = ev["data"]
    if not ns.stream:
        print(json.dumps(last_res, indent=2) if ns.json else json.dumps(last_res))
    return 0 if (last_res and last_res.get("ok")) else 1


if __name__ == "__main__":
    sys.exit(main())
