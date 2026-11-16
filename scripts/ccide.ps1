#!/usr/bin/env pwsh
# Adaptive Windows (`WezTerm` & `Windows Terminal wt.exe`) Launcher for `cc-ide` (`pwsh` / `cmd.exe`).

$ShowGit = "1"
$ShowMon = "0"
$ShowRight = "1"
$LeftPct = "30"
$KeepPanes = $false
$Backend = if ($env:CCIDE_BACKEND) { $env:CCIDE_BACKEND } else { "auto" }
$TargetDir = (Get-Location).Path
$ClaudeArgs = New-Object System.Collections.Generic.List[string]

$HomeDir = if ($env:USERPROFILE) { $env:USERPROFILE } else { $HOME }
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RepoRoot = Split-Path -Parent $ScriptDir
$CoreBin = Join-Path $HomeDir ".claude\bin\ccide-core.exe"
if (-not (Test-Path $CoreBin)) {
    $AltBin = Join-Path $RepoRoot "bin\ccide-core.exe"
    if (Test-Path $AltBin) { $CoreBin = $AltBin }
}
if (Test-Path $CoreBin) { $env:CCIDE_CORE_BIN = $CoreBin }
if (-not $env:CCIDE_IPC_DIR) {
    $ResolvedIpc = ""
    if (Test-Path $CoreBin) {
        $CandIpc = (& $CoreBin ipc-dir 2>$null | Select-Object -First 1)
        if ($LASTEXITCODE -eq 0 -and $CandIpc) { $ResolvedIpc = $CandIpc.Trim() }
    }
    if (-not $ResolvedIpc) {
        $WinTmp = if ($env:TEMP) { $env:TEMP } elseif ($env:TMP) { $env:TMP } else { [System.IO.Path]::GetTempPath() }
        $ResolvedIpc = Join-Path $WinTmp.TrimEnd('\', '/') ".cc-ide"
    }
    $env:CCIDE_IPC_DIR = $ResolvedIpc
}

function Resolve-PyExe {
    foreach ($n in @("python", "python3", "py")) {
        $c = Get-Command $n -ErrorAction SilentlyContinue
        if ($c -and $c.Source -notlike "*WindowsApps*") { return $c.Source }
    }
    return "python"
}

function Assert-CoreBin {
    if (-not (Test-Path $CoreBin)) {
        Write-Error "ccide-core binary not found ($CoreBin); run install.ps1 first."
        exit 1
    }
}

$i = 0
while ($i -lt $args.Count) {
    $arg = [string]$args[$i]
    switch -Regex ($arg) {
        "^(--no-git|--tree-only)$" { $ShowGit = "0"; $i++; continue }
        "^--mon$"                  { $ShowMon = "1"; $i++; continue }
        "^(--no-mon|--2|--3)$"     { $ShowMon = "0"; $i++; continue }
        "^--no-right$"             { $ShowRight = "0"; $i++; continue }
        "^--wezterm$"              { $Backend = "wezterm"; $i++; continue }
        "^--wt$"                   { $Backend = "wt"; $i++; continue }
        "^(--backend)$"            {
            if ($i + 1 -lt $args.Count) { $Backend = [string]$args[$i + 1]; $i += 2 } else { $i++ }
            continue
        }
        "^(--width)$"              {
            if ($i + 1 -lt $args.Count) { $LeftPct = [string]$args[$i + 1]; $i += 2 } else { $i++ }
            continue
        }
        "^--keep-panes$"           { $KeepPanes = $true; $i++; continue }
        "^(--clear-subagents|--clear-done)$" {
            Assert-CoreBin
            $sid = if ($i + 1 -lt $args.Count) { [string]$args[$i + 1] } else { "" }
            & $CoreBin workspace clear-subagents $sid
            exit $LASTEXITCODE
        }
        "^(--theme|--palette)$" {
            Assert-CoreBin
            $th = if ($i + 1 -lt $args.Count) { [string]$args[$i + 1] } else { "list" }
            & $CoreBin workspace theme $th
            exit $LASTEXITCODE
        }
        "^(doctor|--doctor)$" {
            Assert-CoreBin
            $rest = if ($i + 1 -lt $args.Count) { $args[($i + 1)..($args.Count - 1)] } else { @() }
            & $CoreBin doctor $RepoRoot @rest
            exit $LASTEXITCODE
        }
        "^(setup|--setup)$" {
            Assert-CoreBin
            & $CoreBin setup $RepoRoot
            exit $LASTEXITCODE
        }
        "^(--sync-pricing|sync-pricing)$" {
            $py = Resolve-PyExe
            $sp = Join-Path $HomeDir ".claude\scripts\sync-pricing.py"
            if (-not (Test-Path $sp)) { $sp = Join-Path $ScriptDir "sync-pricing.py" }
            $rest = if ($i + 1 -lt $args.Count) { $args[($i + 1)..($args.Count - 1)] } else { @() }
            & $py -I $sp @rest
            exit $LASTEXITCODE
        }
        "^(--check-cc|check-cc)$" {
            $py = Resolve-PyExe
            $cm = Join-Path $HomeDir ".claude\scripts\cc-maint.py"
            if (-not (Test-Path $cm)) { $cm = Join-Path $ScriptDir "cc-maint.py" }
            $rest = if ($i + 1 -lt $args.Count) { $args[($i + 1)..($args.Count - 1)] } else { @("--local") }
            if ($rest.Count -eq 0) { $rest = @("--local") }
            & $py -I $cm @rest
            exit $LASTEXITCODE
        }
        "^workflow$" {
            $py = Resolve-PyExe
            if ($i + 1 -lt $args.Count -and [string]$args[$i + 1] -eq "serve") {
                $srv = Join-Path $HomeDir ".claude\scripts\ccide_workflow_server.py"
                if (-not (Test-Path $srv)) { $srv = Join-Path $ScriptDir "ccide_workflow_server.py" }
                $rest = if ($i + 2 -lt $args.Count) { $args[($i + 2)..($args.Count - 1)] } else { @() }
                & $py -I $srv @rest
            } else {
                $run = Join-Path $HomeDir ".claude\scripts\ccide_workflow_runner.py"
                if (-not (Test-Path $run)) { $run = Join-Path $ScriptDir "ccide_workflow_runner.py" }
                $rest = if ($i + 1 -lt $args.Count) { $args[($i + 1)..($args.Count - 1)] } else { @() }
                & $py -I $run @rest
            }
            exit $LASTEXITCODE
        }
        "^orchestrator$" {
            Assert-CoreBin
            $rest = if ($i + 1 -lt $args.Count) { $args[($i + 1)..($args.Count - 1)] } else { @() }
            & $CoreBin orchestrator @rest
            exit $LASTEXITCODE
        }
        "^(-h|--help)$" {
            @"
Usage: ccide [options] [claude-args...]   (PowerShell & cmd.exe)

Windows Workspace Backends:
  (default)          Auto-detects WezTerm (`WEZTERM_PANE`) or Windows Terminal (`WT_SESSION` / `wt.exe`)
  --wezterm          Force WezTerm native CLI 3-column split (`30% | 54% | 16%`)
  --wt               Force Windows Terminal (`wt.exe -w 0`) 3-column split

Commands & Maintenance:
  --no-git           Hide bottom-left Lazygit pane
  --width <pct>      Left column width percentage (default: 30)
  --keep-panes       Keep companion panes open after Claude exits
  --clear-subagents  Dismiss completed subagents across open sidebars
  --theme [preset]   List or switch color palette preset
  doctor [--json]    Run environment & binary patch health check
  setup              Launch interactive TUI Setup & Live Theme Studio
  --sync-pricing     Sync pricing.json from LiteLLM & genai-prices
  --check-cc [...]   Run Cross-OS Claude Code symbol & schema maintenance workbench
  workflow [...]     Run or serve native workflows (`list`, `run <name>`, `serve`)
  orchestrator [...] Manage subagent/workflow model & effort presets (`status`, `preset`, `sync`)
"@ | Write-Host
            exit 0
        }
        default {
            $ClaudeArgs.Add($arg)
            $i++
        }
    }
}

# If launched from plain `cmd.exe` / console outside WT or WezTerm, bootstrap inside `wt.exe`
# automatically — unless the user explicitly requested the `wezterm` backend.
if ($Backend -ne "wezterm" -and -not $env:WEZTERM_PANE -and -not $env:WT_SESSION -and -not $env:CLAUDECODE) {
    $WtCmd = Get-Command wt.exe -ErrorAction SilentlyContinue
    if ($WtCmd) {
        $SelfPs1 = $MyInvocation.MyCommand.Definition
        $SelfShell = (Get-Process -Id $PID -ErrorAction SilentlyContinue).Path
        if (-not $SelfShell) {
            $PwshCmd = Get-Command pwsh -ErrorAction SilentlyContinue
            $SelfShell = if ($PwshCmd) { $PwshCmd.Source } else { "powershell.exe" }
        }
        # `wt.exe` parses `;` as a subcommand separator, so escape it inside paths.
        $WtDir = $TargetDir.Replace(";", "\;")
        $WtSelf = $SelfPs1.Replace(";", "\;")
        & $WtCmd.Source -d $WtDir $SelfShell -ExecutionPolicy Bypass -File $WtSelf @args
        $WtExit = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
        exit $WtExit
    }
}

$YaziId = [string]((($PID * 65537) + (Get-Random -Maximum 65535) * 257) -band 0x7fffffff)
$GitRoot = ""
$Cur = $TargetDir
while ($Cur -and (Split-Path $Cur -Parent) -ne $Cur) {
    if (Test-Path (Join-Path $Cur ".git")) { $GitRoot = $Cur; break }
    $Cur = Split-Path $Cur -Parent
}
if (-not $GitRoot) { $ShowGit = "0" }

$env:CCIDE_SHELL_PID = "$PID"
$env:CC_YAZI_ID = $YaziId

$PaneSpec = ""
if (Test-Path $CoreBin) {
    $GitRootArg = if ($GitRoot) { $GitRoot } else { "-" }
    $PaneSpec = (& $CoreBin workspace open $TargetDir $YaziId $ShowGit $ShowMon $LeftPct $GitRootArg $ShowRight $Backend 2>$null | Select-Object -First 1)
}

if ($env:CLAUDECODE) {
    if ($env:CLAUDE_CODE_SESSION_ID) {
        $CacheDir = Join-Path $HomeDir ".cache\cc-ide"
        $IpcDir = if ($env:CCIDE_IPC_DIR) { $env:CCIDE_IPC_DIR } else { Join-Path ([System.IO.Path]::GetTempPath()) ".cc-ide" }
        New-Item -ItemType Directory -Force -Path $CacheDir, $IpcDir | Out-Null
        $SessFile = "session_$($env:CLAUDE_CODE_SESSION_ID).yazi"
        Set-Content -LiteralPath (Join-Path $CacheDir $SessFile) -Value $YaziId -NoNewline -Encoding ASCII
        Set-Content -LiteralPath (Join-Path $IpcDir $SessFile) -Value $YaziId -NoNewline -Encoding ASCII
    }
    Write-Host "Workspace panes ready ($PaneSpec, Yazi ID: $YaziId)."
    exit 0
}

function Resolve-ClaudeExe {
    # Prefer a real `claude.exe` (patchable Bun standalone) over a `claude` shell shim.
    $Direct = Get-Command claude.exe -CommandType Application -ErrorAction SilentlyContinue
    if ($Direct) { return $Direct.Source }
    $ClaudeCmd = Get-Command claude -CommandType Application -ErrorAction SilentlyContinue
    if (-not $ClaudeCmd) { return $null }
    $Sibling = Join-Path (Split-Path -Parent $ClaudeCmd.Source) "bin\claude.exe"
    if (Test-Path $Sibling) { return $Sibling }
    return $ClaudeCmd.Source
}

function Invoke-AutoEnsureClaudePatched {
    $TargetExe = Resolve-ClaudeExe
    if (-not $TargetExe) { return }
    $TargetExe = [string]$TargetExe
    if (-not (Test-Path $TargetExe)) { return }
    # Only the Bun standalone executable is patchable; `claude` shell/cmd shims are skipped.
    if (-not $TargetExe.EndsWith(".exe", [System.StringComparison]::OrdinalIgnoreCase)) { return }

    $CacheDir = Join-Path $HomeDir ".cache\cc-ide"
    New-Item -ItemType Directory -Force -Path $CacheDir | Out-Null
    $StampFile = Join-Path $CacheDir "claude_bin.stamp"
    $Fi = Get-Item -LiteralPath $TargetExe -ErrorAction SilentlyContinue
    if (-not $Fi) { return }
    $CurStamp = "r2:$($Fi.Length):$($Fi.LastWriteTimeUtc.Ticks)"
    if ((Test-Path $StampFile) -and ((Get-Content -LiteralPath $StampFile -Raw -ErrorAction SilentlyContinue).Trim() -eq $CurStamp)) {
        return
    }

    $Patcher = Join-Path $HomeDir ".claude\scripts\patch_claude_binary.py"
    if (-not (Test-Path $Patcher)) { $Patcher = Join-Path $ScriptDir "patch_claude_binary.py" }
    $PyExe = Resolve-PyExe
    if ((Test-Path $Patcher) -and (Get-Command $PyExe -ErrorAction SilentlyContinue)) {
        & $PyExe -I $Patcher --binary $TargetExe *> $null
        if ($LASTEXITCODE -eq 0) {
            $FiAfter = Get-Item -LiteralPath $TargetExe -ErrorAction SilentlyContinue
            if ($FiAfter) {
                Set-Content -LiteralPath $StampFile -Value "r2:$($FiAfter.Length):$($FiAfter.LastWriteTimeUtc.Ticks)" -NoNewline -Encoding ASCII
            }
        }
    }
}
Invoke-AutoEnsureClaudePatched

$ClaudeExit = 0
try {
    $ClaudeExe = Resolve-ClaudeExe
    $ClaudeInvoke = if ($ClaudeExe) { $ClaudeExe } else { "claude" }
    & $ClaudeInvoke --dangerously-skip-permissions @ClaudeArgs
    $ClaudeExit = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
} finally {
    if (-not $KeepPanes -and $PaneSpec -and (Test-Path $CoreBin)) {
        & $CoreBin workspace close $PaneSpec $YaziId 2>$null | Out-Null
    }
}
exit $ClaudeExit
