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

function Resolve-PyExe {
    foreach ($n in @("python", "python3", "py")) {
        $c = Get-Command $n -ErrorAction SilentlyContinue
        if ($c -and $c.Source -notlike "*WindowsApps*") { return $c.Source }
    }
    return "python"
}

$i = 0
while ($i -lt $args.Count) {
    $arg = [string]$args[$i]
    switch -Regex ($arg) {
        "^(--no-git|--tree-only)$" { $ShowGit = "0"; $i++; continue }
        "^--no-right$"             { $ShowRight = "0"; $i++; continue }
        "^--wezterm$"              { $Backend = "wezterm"; $i++; continue }
        "^--wt$"                   { $Backend = "wt"; $i++; continue }
        "^--backend$"              { $Backend = [string]$args[$i + 1]; $i += 2; continue }
        "^--width$"                { $LeftPct = [string]$args[$i + 1]; $i += 2; continue }
        "^--keep-panes$"           { $KeepPanes = $true; $i++; continue }
        "^(--clear-subagents|--clear-done)$" {
            $sid = if ($i + 1 -lt $args.Count) { [string]$args[$i + 1] } else { "" }
            & $CoreBin workspace clear-subagents $sid
            exit $LASTEXITCODE
        }
        "^(--theme|--palette)$" {
            $th = if ($i + 1 -lt $args.Count) { [string]$args[$i + 1] } else { "list" }
            & $CoreBin workspace theme $th
            exit $LASTEXITCODE
        }
        "^(doctor|--doctor)$" {
            $rest = if ($i + 1 -lt $args.Count) { $args[($i + 1)..($args.Count - 1)] } else { @() }
            & $CoreBin doctor $RepoRoot @rest
            exit $LASTEXITCODE
        }
        "^(setup|--setup)$" {
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
"@ | Write-Host
            exit 0
        }
        default {
            $ClaudeArgs.Add($arg)
            $i++
        }
    }
}

# If launched from plain `cmd.exe` / console outside WT or WezTerm, bootstrap inside `wt.exe` automatically!
if (-not $env:WEZTERM_PANE -and -not $env:WT_SESSION -and -not $env:CLAUDECODE) {
    $WtCmd = Get-Command wt.exe -ErrorAction SilentlyContinue
    if ($WtCmd) {
        $SelfPs1 = $MyInvocation.MyCommand.Definition
        & $WtCmd.Source -d $TargetDir pwsh -NoExit -ExecutionPolicy Bypass -File $SelfPs1 @args
        exit 0
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
    $PaneSpec = (& $CoreBin workspace open $TargetDir $YaziId $ShowGit $ShowMon $LeftPct $GitRoot $ShowRight $Backend 2>$null | Select-Object -First 1)
}

try {
    & claude --dangerously-skip-permissions @ClaudeArgs
} finally {
    if (-not $KeepPanes -and $PaneSpec -and (Test-Path $CoreBin)) {
        & $CoreBin workspace close $PaneSpec $YaziId 2>$null | Out-Null
    }
}
