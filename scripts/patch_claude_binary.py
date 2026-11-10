#!/usr/bin/env python3
"""
patch_claude_binary.py — Patches Claude Code (`claude.exe` Mach-O arm64 Bun binary) for `ccide`:
1. Whole-Conversation Native Subagent Retention (`fR=9e9`, `yut=9e9`, `aIn=9e9`, `aD(h)=false`, `SEt` adopt finished).
2. Direct Native `viewing-agent` Main-View Switching (`YU(id, Qt)` / `EL(Qt)` + on-demand `svs` registration)
   driven by `/tmp/.cc-ide/req_<sessionId>.json` and publishing `/tmp/.cc-ide/cur_<sessionId>.txt` via `SIGUSR2`.
3. Hides the bottom-most `⏵⏵ bypass permissions on (shift+tab to cycle)` row (`FooterHintLine` / `zP` / `h3t`)
   while keeping interactive prompts (`Ctrl+R` search, `Ctrl+C` exit confirmation, pasting hints) intact.
"""
import glob
import os
import shutil
import struct
import subprocess
import sys


def discover_claude_bin() -> str:
    which_claude = shutil.which("claude")
    if which_claude:
        real_claude = os.path.realpath(which_claude)
        candidate_exe = os.path.join(os.path.dirname(real_claude), "bin", "claude.exe")
        if os.path.exists(candidate_exe):
            return candidate_exe
        if os.path.exists(real_claude):
            return real_claude
    matches = sorted(
        glob.glob(
            os.path.expanduser(
                "~/.nvm/versions/node/*/lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe"
            )
        )
    )
    if matches:
        return matches[-1]
    return os.path.expanduser("~/.local/bin/claude")


CLAUDE_BIN = discover_claude_bin()
CLAUDE_BAK = CLAUDE_BIN + ".bak"


def replace_exact_length(buf: bytearray, old: bytes, new: bytes, label: str) -> int:
    if len(new) > len(old):
        raise ValueError(f"[{label}] replacement is longer than original ({len(new)} > {len(old)})")
    padded = new + b" " * (len(old) - len(new))
    pos = buf.find(old)
    if pos == -1:
        raise RuntimeError(f"[{label}] target byte sequence not found")
    buf[pos : pos + len(old)] = padded
    print(f"  [OK] {label} @ offset {pos} ({len(old)} bytes)")
    return pos


def disable_bun_bytecode_for_offsets(buf: bytearray, patched_offsets: list[int]):
    """
    In the Mach-O __BUN section, locates the StandaloneModuleGraph table and clears
    (bytecode_off, bytecode_len) + bytecode flag for any module spanning a patched offset
    so Bun parses the updated UTF-8 JS source directly.
    """
    base = 92520456
    mod_off = 0x0A971DD5
    mod_len = 0x00022A54
    table_start = base + mod_off
    n_mods = mod_len // 52

    cleared = []
    if table_start + mod_len <= len(buf):
        for i in range(n_mods):
            entry_off = table_start + i * 52
            entry = list(struct.unpack_from("<13I", buf, entry_off))
            c_start = base + entry[2]
            c_end = c_start + entry[3]
            if any(c_start <= off < c_end for off in patched_offsets):
                name_start = base + entry[0]
                name_end = name_start + entry[1]
                if name_end <= len(buf):
                    name = bytes(buf[name_start:name_end]).decode("utf-8", errors="ignore")
                    if name.startswith("/$bunfs/"):
                        entry[6] = 0
                        entry[7] = 0
                        entry[12] &= ~0x100
                        struct.pack_into("<13I", buf, entry_off, *entry)
                        cleared.append((i, name))
    for idx, nm in cleared:
        print(f"  [OK] Cleared Bun bytecode cache for module #{idx}: {nm}")


def apply_patches(src_path: str, dst_path: str):
    with open(src_path, "rb") as f:
        buf = bytearray(f.read())

    offsets = []

    # 1. Module #324: Keep completed subagents for the whole session (9e9 ms)
    offsets.append(
        replace_exact_length(
            buf,
            b"var A0t=3000,fR=30000,yut=30000;",
            b"var A0t=3000,fR=9e9,yut=9e9;",
            "Module #324 subagent retention timer (fR, yut)",
        )
    )

    # 2. Module #453: Keep viewed subagents after exiting transcript view (aIn = 9e9 ms)
    offsets.append(
        replace_exact_length(
            buf,
            b"var iIn=3000;var aIn=30000;",
            b"var iIn=3000;var aIn=9e9;",
            "Module #453 transcript view retention timer (aIn)",
        )
    )

    # 3. Module #1678: Never hide completed background subagents in `aD(h)`
    offsets.append(
        replace_exact_length(
            buf,
            b'function aD(h){return h.type==="local_agent"&&h.status==="completed"&&mR(h)&&h.isBackgrounded&&!op(h)&&h.quietlyParked!==!0&&[...h.keepaliveReasons??[]].every((R)=>R===BU)}',
            b"function aD(h){return!1}",
            "Module #1678 aD(h) keep completed subagents visible",
        )
    )

    # 4. Module #1678: Event-driven SIGUSR2 native view-switch & state push (zero idle polling!)
    old_jn = (
        b'jn=()=>{let _n=Date.now();let lo=!1;let pn=[];let Rn=Rtn(te);let $n=[];let Zn=[];let Wn=K_(te,lt);'
        b'for(const nr of Object.values(te)){if(nr.type==="local_workflow"){if(nr.status!=="running"&&(nr.evictAfter??Infinity)<=_n)to.evictTerminal(nr.id);continue}'
        b'if(!um(nr)){continue}let zn=nr.type==="in_process_teammate"&&nr.evictAfterHeldBySibling===!0;'
        b'if(j0(nr.status)){if(nr.type==="local_agent"&&Wn.has(nr.id)&&nr.evictAfter!==void 0&&nr.evictAfter>0&&nr.evictAfter<=_n+fR-5000)Zn.push(nr.id);'
        b'else if(nr.evictAfter!==void 0&&nr.evictAfter<=_n)to.evictTerminal(nr.id)}'
        b'else if((Rn||zn)&&nr.isIdle&&nr.evictAfter!==void 0&&nr.evictAfter>0){if(Rn!==zn||nr.evictAfter-_n<fR/2)$n.push(nr.id)}'
        b'else if(nr.evictAfter!==void 0&&nr.evictAfter>0&&nr.evictAfter<=_n)pn.push(nr.id);'
        b'else if(!j0(nr.status)&&!nr.isIdle)lo=!0}'
        b'if(pn.length>0||$n.length>0||Zn.length>0)Qt((er)=>{let Bn;for(const Nn of pn){let Gn=er.tasks[Nn];if(THe(Gn,_n,to)){if(Bn===void 0)Bn={...er.tasks};Bn[Nn]={...Gn,evictAfter:0}}}'
        b'for(const lr of $n){let tr=er.tasks[lr];if(Sie(tr)){if(Bn===void 0)Bn={...er.tasks};Bn[lr]={...tr,evictAfter:_n+fR,evictAfterHeldBySibling:Rn?!0:void 0}}}'
        b'for(const Hr of Zn){let ur=er.tasks[Hr];if(um(ur)&&j0(ur.status)&&ur.evictAfter!==void 0&&ur.evictAfter>0){if(Bn===void 0)Bn={...er.tasks};Bn[Hr]={...ur,evictAfter:_n+fR}}}'
        b'return Bn?{...er,tasks:Bn}:er});if(lo&&!Ge)hn(_n)},R[18]=Ge,R[19]=Qt,R[20]=to,R[21]=te,R[22]=lt,R[23]=jn;else jn=R[23];ha(jn,ro||wo?1000:null);'
    )
    new_jn = (
        b'jn=()=>{let _n=Date.now(),lo=!1;'
        b'for(const nr of Object.values(te)){if(um(nr)&&!j0(nr.status)&&!nr.isIdle)lo=!0}'
        b'if(lo&&!Ge)hn(_n)},R[18]=Ge,R[19]=Qt,R[20]=to,R[21]=te,R[22]=lt,R[23]=jn;else jn=R[23];ha(jn,ro||wo?1000:null);'
        b'try{let fs=import.meta.require("fs"),sid=K(),g=globalThis,d="/tmp/.cc-ide";g.__cc={Qt,to,te,lt,sid,fs};'
        b'if(!g.__cc_h){fs.mkdirSync(d,{recursive:!0});g.__cc_h=()=>{try{let c=g.__cc;if(!c)return;let rf=`${d}/req_${c.sid}.json`;'
        b'if(c.fs.existsSync(rf)){let q=JSON.parse(c.fs.readFileSync(rf,"utf8"));c.fs.unlinkSync(rf);'
        b'if(!q.id||q.id==="main"){if(c.lt!==void 0)EL(c.Qt)}'
        b'else{if(!c.te[q.id])svs({agentId:q.id,description:q.desc||"subagent",agentType:q.type||"general-purpose"},c.to);YU(q.id,c.Qt)}}}catch{}};'
        b'process.on("SIGUSR2",g.__cc_h)}'
        b'let st=`${lt??"main"}\\n${_e??""}\\n${Object.keys(te).length}`;'
        b'if(g.__cc_s!==st){g.__cc_s=st;fs.writeFileSync(`${d}/cur_${sid}.txt`,st);'
        b'let sp=`${d}/sb_${sid}.pid`;if(fs.existsSync(sp))process.kill(+fs.readFileSync(sp,"utf8"),"SIGUSR2")}}catch{}'
    )
    offsets.append(
        replace_exact_length(
            buf,
            old_jn,
            new_jn,
            "Module #1678 event-driven SIGUSR2 view-switch & state push hook",
        )
    )

    # 5b. Module #1678: Hide the built-in bottom `⏺ main / ◯ general-purpose ...` footer task list in `TQ(h)` & `Uk()`
    offsets.append(
        replace_exact_length(
            buf,
            b"if(Ge||no.length===0&&!wo&&!Pr){return null}",
            b"if(!0||no.length===0&&!wo&&!Pr){return null}",
            "Module #1678 TQ(h) hide built-in bottom `⏺ main / ◯ subagent` list",
        )
    )
    offsets.append(
        replace_exact_length(
            buf,
            b"function Uk(){let[h]=y(bUe),R=V(h);return AQ()?0:R}",
            b"function Uk(){let[h]=y(bUe),R=V(h);return!0?0:R}",
            "Module #1678 Uk() report 0 visible footer task rows",
        )
    )

    # 6. Module #1678: Hide `⏵⏵ bypass permissions on (shift+tab to cycle)` row (`Dfe` / `h3t` / `zP`)
    offsets.append(
        replace_exact_length(
            buf,
            b'function h3t(h){return{key:"mode",width:t2e(h),node:e(dO,{mode:h},"mode")}}',
            b"function h3t(h){return null}",
            "Module #1678 h3t(h) hide statusline mode pill",
        )
    )
    old_dfe = (
        b"var Dfe=(h)=>{let R=w(13),L,H;if(R[0]!==h)({wrappedRowWidth:H,...L}=h),R[0]=h,R[1]=L,R[2]=H;else L=R[1],H=R[2];"
        b"const Z=!L.isInputEmpty,te=L.isSearching||L.exitMessage.show||L.isPasting===!0||L.showExpandPasteHint===!0||(L.row1Hint??null)!==null;"
        b"let _e;if(R[3]!==L)_e=e(Y5,{...L}),R[3]=L,R[4]=_e;else _e=R[4];"
        b"let Ee;if(R[5]!==L.isLoading||R[6]!==L.rowWidth||R[7]!==L.toolPermissionContext.mode||R[8]!==Z||R[9]!==te||R[10]!==_e||R[11]!==H)"
        b"Ee=e(uj.PromptHintPart,{isDraft:Z,isWorking:L.isLoading,isInteractive:te,mode:L.toolPermissionContext.mode,rowColumns:L.rowWidth,wrappedRowColumns:H,children:_e}),"
        b"R[5]=L.isLoading,R[6]=L.rowWidth,R[7]=L.toolPermissionContext.mode,R[8]=Z,R[9]=te,R[10]=_e,R[11]=H,R[12]=Ee;else Ee=R[12];return Ee};"
    )
    new_dfe = (
        b"var Dfe=(h)=>{let R=w(13),L,H;if(R[0]!==h)({wrappedRowWidth:H,...L}=h),R[0]=h,R[1]=L,R[2]=H;else L=R[1],H=R[2];"
        b"const Z=!L.isInputEmpty,te=L.isSearching||L.exitMessage.show||L.isPasting===!0||L.showExpandPasteHint===!0||(L.row1Hint??null)!==null;"
        b"let _e=te?e(Y5,{...L}):null;"
        b"let Ee=te?e(uj.PromptHintPart,{isDraft:Z,isWorking:L.isLoading,isInteractive:te,mode:L.toolPermissionContext.mode,rowColumns:L.rowWidth,wrappedRowColumns:H,children:_e}):null;"
        b"return Ee};"
    )
    offsets.append(
        replace_exact_length(
            buf,
            old_dfe,
            new_dfe,
            "Module #1678 Dfe(h) hide bottom bypass-permissions row",
        )
    )

    disable_bun_bytecode_for_offsets(buf, offsets)

    tmp_dst = dst_path + ".tmp"
    with open(tmp_dst, "wb") as f:
        f.write(buf)
    os.chmod(tmp_dst, 0o755)
    if sys.platform == "darwin" and shutil.which("codesign"):
        subprocess.run(["codesign", "-f", "-s", "-", tmp_dst], check=True)
    os.replace(tmp_dst, dst_path)
    print(f"  [OK] Installed -> {dst_path}")


if __name__ == "__main__":
    test_mode = "--test" in sys.argv
    if not os.path.exists(CLAUDE_BIN):
        print(f"  [SKIP] claude binary not found at {CLAUDE_BIN}; skipping binary patch.")
        sys.exit(0)

    # Refresh backup automatically if `CLAUDE_BIN` was upgraded (unpatched binary without `g.__cc_h`)
    with open(CLAUDE_BIN, "rb") as _f:
        cur_bytes = _f.read()
    is_already_patched = b"g.__cc_h" in cur_bytes
    if not os.path.exists(CLAUDE_BAK) or not is_already_patched:
        shutil.copy2(CLAUDE_BIN, CLAUDE_BAK)
        print(f"  [OK] Created/refreshed pristine backup -> {CLAUDE_BAK}")

    target = "/tmp/claude-test-bin" if test_mode else CLAUDE_BIN
    try:
        apply_patches(CLAUDE_BAK, target)
        res = subprocess.run([target, "--version"], capture_output=True, text=True, check=True)
        print(f"  [OK] Verified `{target} --version` -> {res.stdout.strip()}")
    except RuntimeError as e:
        print(f"  [WARN] Binary patch skipped (Claude Code version layout differs): {e}")
