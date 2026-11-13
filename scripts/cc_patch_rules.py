#!/usr/bin/env python3
"""
cc_patch_rules.py — Version-Independent Structural Regex Patch Rules for Claude Code (`macOS`/`Linux`/`Windows`).
"""
import re

from bun_pe import BunLayout, iter_js_regions  # type: ignore[import-not-found]


def discover_runtime_symbols(buf: bytearray, layout: BunLayout | None) -> dict[str, str]:
    re_enter = re.compile(
        rb'function\s+([A-Za-z0-9_$]+)\s*\([^,)]+,[^,)]+\)\s*\{[^}]{0,220}["\']tengu_transcript_view_enter["\']'
    )
    re_exit = re.compile(
        rb'function\s+([A-Za-z0-9_$]+)\s*\([^,)]+\)\s*\{[^}]{0,220}["\']tengu_transcript_view_exit["\']'
    )
    re_adopt = re.compile(
        rb'function\s+([A-Za-z0-9_$]+)\s*\([A-Za-z0-9_$]+,[A-Za-z0-9_$]+\)\s*\{[^}]{0,280}["\']local_agent["\'][^}]{0,220}["\']\(resumed agent\)["\']'
    )
    syms: dict[str, str] = {}
    for _, region in iter_js_regions(buf, layout):
        blob = bytes(region)
        for key, pat in (("enter_fn", re_enter), ("exit_fn", re_exit), ("adopt_fn", re_adopt)):
            if key not in syms:
                m = pat.search(blob)
                if m:
                    syms[key] = m.group(1).decode("ascii")
        if len(syms) == 3:
            break
    return syms


def apply_regex_patch_in_regions(
    buf: bytearray, layout: BunLayout | None, pattern: re.Pattern[bytes], builder, label: str
) -> tuple[int | None, str]:
    for base_off, region in iter_js_regions(buf, layout):
        blob = bytes(region)
        m = pattern.search(blob)
        if not m:
            continue
        old_bytes, new_bytes = m.group(0), builder(m)
        if len(new_bytes) > len(old_bytes):
            return None, f"[{label}] replacement exceeds original span ({len(new_bytes)} > {len(old_bytes)} bytes)"
        abs_pos = base_off + m.start()
        buf[abs_pos : abs_pos + len(old_bytes)] = new_bytes + b" " * (len(old_bytes) - len(new_bytes))
        return abs_pos, f"[OK] {label} @ offset {abs_pos} ({len(new_bytes)}/{len(old_bytes)} bytes)"
    return None, f"[MISS] {label}: pattern not found"


def apply_all_structural_patches(
    buf: bytearray, layout: BunLayout | None, syms: dict[str, str]
) -> tuple[list[dict], list[int]]:
    results: list[dict] = []
    patched_offsets: list[int] = []

    def record(name: str, required: bool, pos: int | None, msg: str) -> None:
        if pos is not None:
            patched_offsets.append(pos)
        results.append({"name": name, "required": required, "applied": pos is not None, "offset": pos, "message": msg})

    # 1. Subagent retention timers (`var X=3000,Y=30000,Z=30000;` before `in_process_teammate`)
    re_ret1 = re.compile(
        rb"var\s+([A-Za-z0-9_$]+)=\d+,([A-Za-z0-9_$]+)=30000,([A-Za-z0-9_$]+)=30000;"
        rb"(?=function\s+[A-Za-z0-9_$]+\([^)]*\)\{[^}]{0,220}in_process_teammate)"
    )
    record(
        "subagent_retention_timers", True,
        *apply_regex_patch_in_regions(
            buf, layout, re_ret1,
            lambda m: b"var " + m.group(1) + b"=3000," + m.group(2) + b"=9e9," + m.group(3) + b"=9e9;",
            "Subagent retention timers (9e9 ms)",
        ),
    )

    # 2. Transcript view retention timer (`var X=3000;var Y=30000;` before `local_agent`)
    re_ret2 = re.compile(
        rb"var\s+([A-Za-z0-9_$]+)=3000;\s*var\s+([A-Za-z0-9_$]+)=30000;"
        rb"(?=function\s+[A-Za-z0-9_$]+\([^)]*\)\{[^}]{0,180}local_agent)"
    )
    record(
        "transcript_view_retention", True,
        *apply_regex_patch_in_regions(
            buf, layout, re_ret2,
            lambda m: b"var " + m.group(1) + b"=3000;var " + m.group(2) + b"=9e9;",
            "Transcript view exit retention timer (9e9 ms)",
        ),
    )

    # 3. Never hide completed background subagents (`function aD(h){...quietlyParked...}`)
    re_vis = re.compile(
        rb"function\s+([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\)\{"
        rb'return\s+\2\.type==="local_agent"&&\2\.status==="completed"&&[^}]{15,260}quietlyParked[^}]+\}'
    )
    record(
        "keep_completed_visible", True,
        *apply_regex_patch_in_regions(
            buf, layout, re_vis,
            lambda m: b"function " + m.group(1) + b"(" + m.group(2) + b"){return!1}",
            "Keep completed background subagents visible",
        ),
    )

    # 4. Cross-OS Event-Driven View-Switch & State Push Hook (`SIGUSR2` on Unix + `fs.watch` on Win32)
    enter_fn, exit_fn, adopt_fn = syms.get("enter_fn", ""), syms.get("exit_fn", ""), syms.get("adopt_fn", "")
    if enter_fn and exit_fn and adopt_fn:
        re_jn = re.compile(
            rb"([A-Za-z0-9_$]+)=\(\)=>\{let\s+([A-Za-z0-9_$]+)=Date\.now\(\);let\s+([A-Za-z0-9_$]+)=!1;"
            rb"(?:let\s+[^;]{1,80};){1,10}"
            rb"let\s+[A-Za-z0-9_$]+=[A-Za-z0-9_$]+\(([A-Za-z0-9_$]+),([A-Za-z0-9_$]+)\);"
            rb'for\(const\s+[A-Za-z0-9_$]+\s+of\s+Object\.values\(\4\)\)\{if\([A-Za-z0-9_$]+\.type==="local_workflow"\)\{'
            rb'if\([A-Za-z0-9_$]+\.status!=="running"&&\([A-Za-z0-9_$]+\.evictAfter\?\?Infinity\)<=\2\)([A-Za-z0-9_$]+)\.evictTerminal\([A-Za-z0-9_$]+\.id\);continue\}'
            rb"if\(!([A-Za-z0-9_$]+)\([A-Za-z0-9_$]+\)\)\{continue\}.{120,1800}?"
            rb"if\([A-Za-z0-9_$]+\.length>0\|\|[A-Za-z0-9_$]+\.length>0\|\|[A-Za-z0-9_$]+\.length>0\)([A-Za-z0-9_$]+)\(\([A-Za-z0-9_$]+\)=>\{.{80,1000}?\}\);"
            rb"if\(\3&&!([A-Za-z0-9_$]+)\)([A-Za-z0-9_$]+)\(\2\)\}"
            rb"(,[^;]{10,180};else\s+\1=[^;]{3,40};[A-Za-z0-9_$]+\(\1,[^;]{3,50}\);)"
        )

        def build_jn(m: re.Match[bytes]) -> bytes:
            jn_v, now_v, lo_v = m.group(1), m.group(2), m.group(3)
            te_v, lt_v, to_v, um_v = m.group(4), m.group(5), m.group(6), m.group(7)
            qt_v, ge_v, hn_v, tail = m.group(8), m.group(9), m.group(10), m.group(11)
            return (
                jn_v + b"=()=>{let " + now_v + b"=Date.now()," + lo_v + b"=!1;for(const u of Object.values(" + te_v
                + b")){if(" + um_v + b'(u)&&u.status==="running"&&!u.isIdle)' + lo_v + b"=!0}if(" + lo_v + b"&&!" + ge_v
                + b")" + hn_v + b"(" + now_v + b")}" + tail
                + b'try{let r=import.meta.require,fs=r("fs"),p=process.pid,g=globalThis,w=process.platform==="win32",'
                + b'd=process.env.CCIDE_IPC_DIR||(w?r("os").tmpdir()+"/.cc-ide":"/tmp/.cc-ide");'
                + b"g.__cc={Qt:" + qt_v + b",to:" + to_v + b",te:" + te_v + b",lt:" + lt_v + b",fs};"
                + b"if(!g.__cc_h){fs.mkdirSync(d,{recursive:!0});"
                + b"g.__cc_h=()=>{try{let c=g.__cc,rf=`${d}/req_pid_${p}.json`;if(!c||!c.fs.existsSync(rf))return;"
                + b'let q=JSON.parse(c.fs.readFileSync(rf,"utf8"));c.fs.unlinkSync(rf);'
                + b'if(!q.id||q.id==="main"){if(c.lt!==void 0)' + exit_fn.encode("ascii")
                + b"(c.Qt)}else{if(!(c.to.get?c.to.get(q.id):c.te[q.id]))" + adopt_fn.encode("ascii")
                + b'({agentId:q.id,description:q.desc||"subagent",agentType:q.type||"general-purpose"},c.to);'
                + enter_fn.encode("ascii") + b'(q.id,c.Qt)}}catch{}};'
                + b'w?fs.watch(d,(_,f)=>f==`req_pid_${p}.json`&&g.__cc_h()).unref():process.on("SIGUSR2",g.__cc_h)}w&&g.__cc_h();'
                + b"let st=`${" + lt_v + b'??"main"}\\n\\n${Object.keys(' + te_v
                + b").length}`;if(g.__cc_s!==st){g.__cc_s=st;fs.writeFileSync(`${d}/cur_pid_${p}.txt`,st);"
                + b'let sp=`${d}/sb_pid_${p}.pid`;if(fs.existsSync(sp)){let k=+fs.readFileSync(sp,"utf8");'
                + b'k>1&&(w?r("child_process").spawn("ccide-core",["wake-pid",""+k],{stdio:"ignore",windowsHide:!0}).unref():process.kill(k,"SIGUSR2"))}}}catch{}'
            )

        record("sigusr2_view_switch_hook", True, *apply_regex_patch_in_regions(buf, layout, re_jn, build_jn, f"Event-driven view-switch hook ({enter_fn}/{exit_fn}/{adopt_fn})"))
    else:
        record("sigusr2_view_switch_hook", True, None, f"[MISS] Runtime symbols incomplete: {syms}")

    # 5a. Hide built-in bottom `⏺ main / ◯ subagent` footer task list (`TQ` early return)
    re_tq_hide = re.compile(rb"if\(([A-Za-z0-9_$]+)\|\|([A-Za-z0-9_$]+\.length===0&&![A-Za-z0-9_$]+&&![A-Za-z0-9_$]+)\)\{return null\}")
    record("hide_footer_subagent_list", False, *apply_regex_patch_in_regions(buf, layout, re_tq_hide, lambda m: b"if(!0||" + m.group(2) + b"){return null}", "Hide built-in bottom subagent footer list"))

    # 5b. Report 0 visible footer task rows (`Uk()`)
    re_uk = re.compile(
        rb"(function\s+[A-Za-z0-9_$]+\(\)\{let\[[A-Za-z0-9_$]+\]=[A-Za-z0-9_$]+\(([A-Za-z0-9_$]+)\),[A-Za-z0-9_$]+=[A-Za-z0-9_$]+\([A-Za-z0-9_$]+\);return\s+)"
        rb"[A-Za-z0-9_$]+\(\)(\?0:[A-Za-z0-9_$]+\})(?=function\s+\2\(\)\{[^}]{0,260}(?:idleTeammatesExpanded|taskDecorations))"
    )
    record("zero_footer_task_rows", False, *apply_regex_patch_in_regions(buf, layout, re_uk, lambda m: m.group(1) + b"!0" + m.group(3), "Report 0 visible footer task rows"))

    # 5c. Block `Down`/`Left`/`Right` arrow keys from entering invisible `footerSelection="tasks"`
    re_nav_tasks = re.compile(rb'(?<=[=\[])[A-Za-z0-9_$]+&&"tasks"(?=,(?:[A-Za-z0-9_$]+=)?[A-Za-z0-9_$]+&&"frame")')
    record("lock_hidden_footer_tasks_nav", False, *apply_regex_patch_in_regions(buf, layout, re_nav_tasks, lambda _: b"!1", "Lock hidden footer tasks out of prompt arrow-key navigation"))

    # 6a. Hide statusline mode pill (`key:"mode"`)
    re_mode_pill = re.compile(rb'function\s+([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\)\{return\{key:"mode",width:.{5,80}?,"mode"\)\}\}')
    record("hide_mode_pill", False, *apply_regex_patch_in_regions(buf, layout, re_mode_pill, lambda m: b"function " + m.group(1) + b"(" + m.group(2) + b"){return null}", "Hide statusline mode pill"))

    # 6b. Hide bottom `⏵⏵ bypass permissions on` hint row (`PromptHintPart`) unless interactive
    re_dfe = re.compile(
        rb"(var\s+[A-Za-z0-9_$]+=\([A-Za-z0-9_$]+\)=>\{let\s+[A-Za-z0-9_$]+=[A-Za-z0-9_$]+\(\d+\),([A-Za-z0-9_$]+),([A-Za-z0-9_$]+);[^;]+;else\s+\2=[^;]+;"
        rb"const\s+([A-Za-z0-9_$]+)=!\2\.isInputEmpty,([A-Za-z0-9_$]+)=\2\.isSearching\|\|\2\.exitMessage\.show\|\|[^;]+showExpandPasteHint[^;]+;)"
        rb"let\s+([A-Za-z0-9_$]+);if\([^)]+\)\6=([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+),(\{\.\.\.\2[^}]*\})\),[^;]+;else\s+\6=[^;]+;"
        rb"let\s+[A-Za-z0-9_$]+;if\([^)]+\)[A-Za-z0-9_$]+=\7\(([A-Za-z0-9_$]+\.PromptHintPart),\{([^}]+)\}\),[^;]+;else\s+[A-Za-z0-9_$]+=[^;]+;return\s+[A-Za-z0-9_$]+\};"
    )
    record(
        "hide_bypass_permissions_row", False,
        *apply_regex_patch_in_regions(
            buf, layout, re_dfe,
            lambda m: m.group(1) + b"let " + m.group(6) + b"=" + m.group(5) + b"?" + m.group(7) + b"(" + m.group(8) + b"," + m.group(9) + b"):null;return " + m.group(5) + b"?" + m.group(7) + b"(" + m.group(10) + b",{" + m.group(11) + b"}):null};",
            "Hide bottom bypass-permissions row while keeping interactive hints",
        ),
    )

    # 6c. Safe `Escape` exit from `viewing-agent` without calling `CP(...)` kill
    if exit_fn:
        exit_b = exit_fn.encode("ascii")
        re_esc_safe = re.compile(
            rb'(if\(([A-Za-z0-9_$]+)\.name==="escape"&&([A-Za-z0-9_$]+)==="viewing-agent"\)\{if\([A-Za-z0-9_$]+\)return;\2\.preventDefault\(\);)'
            rb"let\s+[A-Za-z0-9_$]+=[A-Za-z0-9_$]+;if\([A-Za-z0-9_$]+\)\{.{25,360}?\}" + re.escape(exit_b) + rb"\(([A-Za-z0-9_$]+)\)\}"
        )
        record("safe_escape_from_subagent_view", False, *apply_regex_patch_in_regions(buf, layout, re_esc_safe, lambda m: m.group(1) + exit_b + b"(" + m.group(4) + b")}", f"Safe Escape exit from viewing-agent ({exit_fn})"))

    # 6d. Prevent `DPe()` from yanking `viewing-agent` back to `main` on main conversation updates
    if enter_fn and adopt_fn:
        enter_b, adopt_b = enter_fn.encode("ascii"), adopt_fn.encode("ascii")
        re_dpe = re.compile(
            rb"(function\s+[A-Za-z0-9_$]+\(\)\{let\s+([A-Za-z0-9_$]+)=[A-Za-z0-9_$]+\(\),"
            rb"([A-Za-z0-9_$]+)=([A-Za-z0-9_$]+)\(\([A-Za-z0-9_$]+\)=>[A-Za-z0-9_$]+\.viewingAgentTaskId\),"
            rb"([A-Za-z0-9_$]+)=\4\(\([A-Za-z0-9_$]+\)=>[A-Za-z0-9_$]+\.viewingAgentTaskId\?[A-Za-z0-9_$]+\.tasks\[[A-Za-z0-9_$]+\.viewingAgentTaskId\]:void 0\))"
            rb"[^}]+parentAgentId.{1,400}?([A-Za-z0-9_$]+)\(\(\)=>\{if\(!\3\)return;.{10,260}?\},\[[^\]]+\]\)\}"
        )
        record(
            "keep_viewing_agent_on_turn_update", False,
            *apply_regex_patch_in_regions(
                buf, layout, re_dpe,
                lambda m: m.group(1) + b";" + m.group(6) + b"(()=>{let c=globalThis.__cc;if(" + m.group(3) + b"&&!" + m.group(5) + b"&&c?.to){" + adopt_b + b"({agentId:" + m.group(3) + b"},c.to);" + enter_b + b"(" + m.group(3) + b"," + m.group(2) + b")}},[" + m.group(3) + b"," + m.group(5) + b"," + m.group(2) + b"])}",
                f"Keep viewing-agent pinned across main conversation updates ({adopt_fn}/{enter_fn})",
            ),
        )

    # 7. Export exact monotonic stream `ttftMs` & `durationMs` per API turn (`pf_<pid>.jsonl`, standalone)
    re_perf = re.compile(
        rb'("tengu_api_success",\{model:[A-Za-z0-9_$]+\(([A-Za-z0-9_$]+)\),'
        rb".{40,560}?outputTokens:([A-Za-z0-9_$]+)\.output_tokens,)"
        rb"cachedInputTokens:\3\.cache_read_input_tokens\?\?0,"
        rb"uncachedInputTokens:\3\.cache_creation_input_tokens\?\?0,"
        rb"cache_creation_5m_input_tokens:\3\.cache_creation\?\.ephemeral_5m_input_tokens\?\?0,"
        rb"cache_creation_1h_input_tokens:\3\.cache_creation\?\.ephemeral_1h_input_tokens\?\?0,"
        rb"(.{0,140}?durationMs:([A-Za-z0-9_$]+),durationMsIncludingRetries:[A-Za-z0-9_$]+,attempt:[A-Za-z0-9_$]+,"
        rb"ttftMs:([A-Za-z0-9_$]+)\?\?void 0,firstContentMs:([A-Za-z0-9_$]+)\?\?void 0)"
    )
    record(
        "export_api_perf_metrics", False,
        *apply_regex_patch_in_regions(
            buf, layout, re_perf,
            lambda m: (
                m.group(1)
                + b'...(()=>{try{let f=import.meta.require("fs"),d=(process.env.TEMP||"/tmp")+"/.cc-ide";'
                + b"f.mkdirSync(d,{recursive:!0});f.appendFileSync(`${d}/pf_${process.pid}.jsonl`,JSON.stringify({m:"
                + m.group(2) + b",o:" + m.group(3) + b".output_tokens,d:" + m.group(5)
                + b",t:" + m.group(6) + b"??" + m.group(7) + b"})+`\\n`)}catch{}})(),"
                + m.group(4)
            ),
            "Export exact monotonic stream TPS & TTFT metrics",
        ),
    )

    return results, patched_offsets
