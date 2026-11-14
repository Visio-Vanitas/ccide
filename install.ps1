#!/usr/bin/env pwsh
<#
.SYNOPSIS
    cc-ide Native Windows Installer, Doctor & Clean Uninstaller (PowerShell 7 / Windows PowerShell 5.1)
.DESCRIPTION
    Builds or downloads + SHA256-verifies `ccide-core.exe`, installs launchers (`ccide.cmd` + `ccide.ps1`)
    into `%USERPROFILE%\.local\bin` (with reversible User PATH update), syncs `%APPDATA%\yazi\config` and
    `%LOCALAPPDATA%\lazygit\config.yml` (`.ccide.bak` backups), registers the `cc-ide@cc-ide` Claude Code
    plugin + Powerline `statusLine`, and patches Windows `claude.exe` safely.
#>
param(
    [switch]$Yes,
    [switch]$Apply,
    [switch]$Tui,
    [switch]$Doctor,
    [switch]$Json,
    [switch]$PatchOnly,
    [switch]$Uninstall,
    [switch]$Help
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Definition
$PluginVersion = "0.7.0"
$HomeDir = if ($env:USERPROFILE) { $env:USERPROFILE } else { $HOME }
$ClaudeDir = Join-Path $HomeDir ".claude"
$CacheDir = Join-Path $HomeDir ".cache\cc-ide"
$LocalBinDir = Join-Path $HomeDir ".local\bin"
$ConfigDir = Join-Path $HomeDir ".config\cc-ide"
$PluginCacheDir = Join-Path $ClaudeDir "plugins\cache\cc-ide\cc-ide\$PluginVersion"
$IpcDir = if ($env:CCIDE_IPC_DIR) { $env:CCIDE_IPC_DIR } else { Join-Path ([System.IO.Path]::GetTempPath()) ".cc-ide" }

function Write-Step([string]$Msg) { Write-Host "==> $Msg" -ForegroundColor Cyan }
function Write-Ok([string]$Msg)   { Write-Host "  [OK] $Msg" -ForegroundColor Green }
function Write-Warn([string]$Msg) { Write-Host "  [WARN] $Msg" -ForegroundColor Yellow }

if ($Help) {
    @"
Usage: .\install.ps1 [options]  (or `install.cmd [options]` from cmd.exe)

Options:
  (no args)        Launch interactive TUI Setup & Doctor (falls back to -Yes if non-interactive)
  -Yes             Non-interactive full build/download, plugin install, claude.exe patch & PATH setup
  -Doctor [-Json]  Run environment, WezTerm/WT & patch-anchor health check (TUI or JSON)
  -PatchOnly       Re-apply structural patches to Windows claude.exe
  -Uninstall       Cleanly restore claude.exe.bak, remove managed configs & strip User PATH entry
"@ | Write-Host
    exit 0
}

function Resolve-PythonExe {
    foreach ($cand in @(
        (Join-Path $HomeDir ".local\share\cc-ide\pyenv\Scripts\python.exe"),
        (Join-Path $HomeDir "miniconda3\python.exe")
    )) {
        if (Test-Path $cand) { return $cand }
    }
    foreach ($name in @("python", "python3", "py")) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd -and $cmd.Source -notlike "*WindowsApps*") { return $cmd.Source }
    }
    return $null
}

function Backup-Once([string]$Path) {
    if ((Test-Path $Path) -and -not (Test-Path "$Path.ccide.bak")) {
        Copy-Item -LiteralPath $Path -Destination "$Path.ccide.bak" -Force
    }
}

function Ensure-EngineBinary {
    $BinOut = Join-Path $ProjectRoot "bin\ccide-core.exe"
    $ClaudeBin = Join-Path $ClaudeDir "bin\ccide-core.exe"
    New-Item -ItemType Directory -Force -Path (Split-Path $BinOut), (Split-Path $ClaudeBin), $IpcDir | Out-Null

    if ($env:CCIDE_SKIP_BUILD -eq "1" -and (Test-Path $BinOut)) {
        Copy-Item -LiteralPath $BinOut -Destination $ClaudeBin -Force
        return $BinOut
    }

    $CargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $CargoCmd) {
        $FallbackCargo = Join-Path $HomeDir ".cargo\bin\cargo.exe"
        if (Test-Path $FallbackCargo) { $CargoCmd = @{ Source = $FallbackCargo } }
    }

    if ($CargoCmd) {
        Write-Step "1/6 Building native Win32 Rust engine (ccide-core.exe)..."
        Push-Location $ProjectRoot
        try {
            & $CargoCmd.Source build --release --locked --quiet
            if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
        } finally {
            Pop-Location
        }
        Copy-Item -LiteralPath (Join-Path $ProjectRoot "target\release\ccide-core.exe") -Destination $BinOut -Force
        Copy-Item -LiteralPath $BinOut -Destination $ClaudeBin -Force
        Write-Ok "Compiled $BinOut"
        return $BinOut
    }

    Write-Step "1/6 cargo not found; downloading prebuilt SHA256-verified ccide-core.exe..."
    $NodeCmd = Get-Command node -ErrorAction SilentlyContinue
    if ($NodeCmd) {
        $env:CLAUDE_PLUGIN_ROOT = $ProjectRoot
        & $NodeCmd.Source (Join-Path $ProjectRoot "scripts\ensure-ccide-core.mjs") --help | Out-Null
    }
    if (Test-Path $ClaudeBin) {
        Copy-Item -LiteralPath $ClaudeBin -Destination $BinOut -Force
        Write-Ok "Downloaded prebuilt ccide-core.exe -> $ClaudeBin"
        return $BinOut
    }
    throw "Neither cargo nor prebuilt GitHub Release binary could be resolved."
}

$PyExe = Resolve-PythonExe

if ($Uninstall) {
    Write-Step "Uninstalling cc-ide Windows patches, launchers & managed config blocks..."
    Get-Process -Name "ccide-core" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    if ($PyExe) {
        $FoundBin = & $PyExe -I -c "import sys; sys.path.insert(0, r'$ProjectRoot\scripts'); import bun_pe; print(bun_pe.discover_claude_bin())" 2>$null
        if (-not $FoundBin) {
        $ClaudeCmd = Get-Command claude -ErrorAction SilentlyContinue
        if ($ClaudeCmd) {
            $CandExe = Join-Path (Split-Path -Parent $ClaudeCmd.Source) "bin\claude.exe"
            $FoundBin = if (Test-Path $CandExe) { $CandExe } else { $ClaudeCmd.Source }
        }
    }
    if ($FoundBin -and (Test-Path "$FoundBin.bak")) {
        Copy-Item -LiteralPath "$FoundBin.bak" -Destination $FoundBin -Force
        Write-Ok "Restored pristine $FoundBin from .bak"
    }
    $YaziCfgDir = if ($env:APPDATA) { Join-Path $env:APPDATA "yazi\config" } else { Join-Path $HomeDir ".config\yazi" }
    foreach ($fn in @("yazi.toml", "keymap.toml", "init.lua")) {
        $fp = Join-Path $YaziCfgDir $fn
        if (Test-Path "$fp.ccide.bak") {
            Copy-Item -LiteralPath "$fp.ccide.bak" -Destination $fp -Force
        }
    }
    $StFile = Join-Path $ClaudeDir "settings.json"
    if (Test-Path $StFile) {
        if ($PyExe) {
            & $PyExe -I -c "import json,os; p=r'$StFile'; st=json.load(open(p,encoding='utf-8')); (st.get('enabledPlugins') or {}).pop('cc-ide@cc-ide',None); (st.get('extraKnownMarketplaces') or {}).pop('cc-ide',None); sl=st.get('statusLine'); (st.pop('statusLine',None) if isinstance(sl,dict) and 'ccide-core' in str(sl.get('command','')) else None); json.dump(st,open(p,'w',encoding='utf-8'),indent=2)"
        } else {
            try {
                $st = Get-Content -LiteralPath $StFile -Raw -Encoding UTF8 | ConvertFrom-Json
                if ($st.enabledPlugins -and ($st.enabledPlugins.PSObject.Properties.Name -contains "cc-ide@cc-ide")) {
                    $st.enabledPlugins.PSObject.Properties.Remove("cc-ide@cc-ide")
                }
                if ($st.extraKnownMarketplaces -and ($st.extraKnownMarketplaces.PSObject.Properties.Name -contains "cc-ide")) {
                    $st.extraKnownMarketplaces.PSObject.Properties.Remove("cc-ide")
                }
                if ($st.statusLine -and $st.statusLine.command -and ($st.statusLine.command -like "*ccide-core*")) {
                    $st.PSObject.Properties.Remove("statusLine")
                }
                $st | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $StFile -Encoding UTF8
            } catch {}
        }
    }
    if (Test-Path $PluginCacheDir) {
        Remove-Item -LiteralPath $PluginCacheDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($UserPath -and $UserPath.Contains($LocalBinDir)) {
        $Cleaned = (($UserPath -split ";") | Where-Object { $_ -and ($_ -ne $LocalBinDir) }) -join ";"
        [Environment]::SetEnvironmentVariable("Path", $Cleaned, "User")
    }
    Remove-Item -LiteralPath (Join-Path $LocalBinDir "ccide.cmd"), (Join-Path $LocalBinDir "cc-ide.cmd"), (Join-Path $LocalBinDir "ccide.ps1") -Force -ErrorAction SilentlyContinue
    Write-Ok "Uninstalled cc-ide cleanly."
    exit 0
}

if ($PatchOnly) {
    if (-not $PyExe) { throw "Python 3 is required to run patch_claude_binary.py" }
    Write-Step "Applying cross-OS structural patches to Windows claude.exe..."
    & $PyExe -I (Join-Path $ProjectRoot "scripts\patch_claude_binary.py")
    exit $LASTEXITCODE
}

$EngineExe = Ensure-EngineBinary

if ($Doctor) {
    $DocArgs = @("doctor", $ProjectRoot)
    if ($Json) { $DocArgs += "--json" }
    & $EngineExe @DocArgs
    exit $LASTEXITCODE
}

$IsInteractive = [Environment]::UserInteractive -and -not [Console]::IsInputRedirected -and ($env:CCIDE_NONINTERACTIVE -ne "1")
if ($Tui -or (-not $Yes -and -not $Apply -and $IsInteractive)) {
    & $EngineExe setup $ProjectRoot
    exit $LASTEXITCODE
}

$OptPlugin    = if ($env:CCIDE_OPT_PLUGIN)    { $env:CCIDE_OPT_PLUGIN }    else { "1" }
$OptPatch     = if ($env:CCIDE_OPT_PATCH)     { $env:CCIDE_OPT_PATCH }     else { "1" }
$OptCompanion = if ($env:CCIDE_OPT_COMPANION) { $env:CCIDE_OPT_COMPANION } else { "1" }

Write-Step "2/6 Installing sprites, pricing catalog, PowerShell/cmd launchers & PATH..."
New-Item -ItemType Directory -Force -Path "$CacheDir\logos", "$CacheDir\sprites", "$CacheDir\status", "$ClaudeDir\scripts", $LocalBinDir, "$ConfigDir\sprites" | Out-Null
Copy-Item -LiteralPath (Join-Path $ProjectRoot "assets\adaptive_quadrant_logos.json") -Destination "$CacheDir\logos\adaptive_quadrant_logos.json" -Force
Copy-Item -Path (Join-Path $ProjectRoot "assets\sprites\*") -Destination "$CacheDir\sprites\" -Force
Get-ChildItem -Path (Join-Path $ProjectRoot "assets\sprites\*") | ForEach-Object {
    $dst = Join-Path "$ConfigDir\sprites" $_.Name
    if (-not (Test-Path $dst)) { Copy-Item -LiteralPath $_.FullName -Destination $dst }
}
Copy-Item -LiteralPath (Join-Path $ProjectRoot "assets\lazygit.yml") -Destination "$ConfigDir\lazygit.yml" -Force
Copy-Item -LiteralPath (Join-Path $ProjectRoot "assets\glow-style.json") -Destination "$ConfigDir\glow-style.json" -Force
foreach ($cfgName in @("palette.json", "viewer.json", "pricing.json", "orchestrator.json")) {
    $dstCfg = Join-Path $ConfigDir $cfgName
    if (-not (Test-Path $dstCfg)) {
        Copy-Item -LiteralPath (Join-Path $ProjectRoot "assets\$cfgName") -Destination $dstCfg
    }
}
$WorkflowSrcDir = Join-Path $ProjectRoot "assets\workflows"
if (Test-Path $WorkflowSrcDir) {
    $UserWfDir = Join-Path $ClaudeDir "workflows"
    New-Item -ItemType Directory -Force -Path $UserWfDir | Out-Null
    Copy-Item -Path (Join-Path $WorkflowSrcDir "*.js") -Destination $UserWfDir -Force -ErrorAction SilentlyContinue
}

Copy-Item -Path (Join-Path $ProjectRoot "scripts\*") -Destination "$ClaudeDir\scripts\" -Force
Copy-Item -LiteralPath (Join-Path $ProjectRoot "scripts\lazygit-log.py") -Destination "$ClaudeDir\scripts\lazygit-log.py" -Force
Copy-Item -LiteralPath (Join-Path $ProjectRoot "scripts\git_smooth_graph.py") -Destination "$ClaudeDir\scripts\git_smooth_graph.py" -Force
Copy-Item -LiteralPath (Join-Path $ProjectRoot "scripts\ccide.ps1") -Destination (Join-Path $LocalBinDir "ccide.ps1") -Force
Copy-Item -LiteralPath (Join-Path $ProjectRoot "scripts\ccide.cmd") -Destination (Join-Path $LocalBinDir "ccide.cmd") -Force
Copy-Item -LiteralPath (Join-Path $ProjectRoot "scripts\ccide.cmd") -Destination (Join-Path $LocalBinDir "cc-ide.cmd") -Force

$CurUserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not $CurUserPath) { $CurUserPath = "" }
if (($CurUserPath -split ";") -notcontains $LocalBinDir) {
    $NewUserPath = if ($CurUserPath) { "$LocalBinDir;$CurUserPath" } else { $LocalBinDir }
    [Environment]::SetEnvironmentVariable("Path", $NewUserPath, "User")
    $env:Path = "$LocalBinDir;$env:Path"
    Write-Ok "Added $LocalBinDir to User PATH (available in cmd.exe & pwsh)"
} else {
    Write-Ok "Launchers ccide.cmd / ccide.ps1 updated in $LocalBinDir"
}

Write-Step "3/6 Syncing Yazi (%APPDATA%\yazi\config) & Lazygit companion configs..."
if ($OptCompanion -eq "1") {
    $YaziCfgDir = if ($env:APPDATA) { Join-Path $env:APPDATA "yazi\config" } else { Join-Path $HomeDir ".config\yazi" }
    New-Item -ItemType Directory -Force -Path (Join-Path $YaziCfgDir "plugins\hscroll.yazi") | Out-Null
    Copy-Item -LiteralPath (Join-Path $ProjectRoot "assets\yazi-hscroll.lua") -Destination (Join-Path $YaziCfgDir "plugins\hscroll.yazi\main.lua") -Force
    $InitLua = Join-Path $YaziCfgDir "init.lua"
    Backup-Once $InitLua
    $InitTxt = if (Test-Path $InitLua) { Get-Content -LiteralPath $InitLua -Raw } else { "" }
    if ($InitTxt -notmatch 'require\(["'']hscroll["'']\)') {
        Set-Content -LiteralPath $InitLua -Value (($InitTxt.TrimEnd() + "`nrequire(`"hscroll`"):setup()`n").TrimStart("`n")) -Encoding UTF8
    }
    $LgCfgDir = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "lazygit" } else { Join-Path $HomeDir "AppData\Local\lazygit" }
    New-Item -ItemType Directory -Force -Path $LgCfgDir | Out-Null
    $LgCfg = Join-Path $LgCfgDir "config.yml"
    Backup-Once $LgCfg
    $PyCmd = if ($PyExe) { "`"$PyExe`"" } else { "python" }
    $PagerPath = Join-Path $ClaudeDir "scripts\lazygit-pager.sh"
    $LogPath = Join-Path $ClaudeDir "scripts\lazygit-log.py"
    $LgYml = Get-Content -LiteralPath (Join-Path $ProjectRoot "assets\lazygit.yml") -Raw -Encoding UTF8
    $LgYml = $LgYml -replace '(?m)^(\s*pager:\s*).*$', "`$1'$PyCmd -I `"$PagerPath`" --col-width={{columnWidth}}'"
    $LgYml = $LgYml -replace '(?m)^(\s*branchLogCmd:\s*).*$', "`$1'$PyCmd -I `"$LogPath`" --branch={{branchName}}'"
    $LgYml = $LgYml -replace '(?m)^(\s*-\s*)lazygit-log\.py --all\s*$', "`$1'$PyCmd -I `"$LogPath`" --all'"
    Set-Content -LiteralPath $LgCfg -Value $LgYml -Encoding UTF8
    Write-Ok "Installed hscroll.yazi -> $YaziCfgDir & Lazygit Windows config -> $LgCfg"
} else {
    Write-Ok "Skipped Yazi/Lazygit companion config edits per user selection"
}

Write-Step "4/6 Packaging & registering official Claude Code plugin (cc-ide@cc-ide v$PluginVersion)..."
if (Test-Path $PluginCacheDir) { Remove-Item -LiteralPath $PluginCacheDir -Recurse -Force }
foreach ($sub in @(".claude-plugin", "hooks", "skills", "scripts", "assets", "bin")) {
    New-Item -ItemType Directory -Force -Path (Join-Path $PluginCacheDir $sub) | Out-Null
}
Copy-Item -LiteralPath (Join-Path $ProjectRoot ".claude-plugin\plugin.json") -Destination "$PluginCacheDir\.claude-plugin\plugin.json" -Force
Copy-Item -LiteralPath (Join-Path $ProjectRoot ".claude-plugin\marketplace.json") -Destination "$PluginCacheDir\.claude-plugin\marketplace.json" -Force
Copy-Item -LiteralPath (Join-Path $ProjectRoot "hooks\hooks.json") -Destination "$PluginCacheDir\hooks\hooks.json" -Force
Copy-Item -Path (Join-Path $ProjectRoot "skills\*") -Destination "$PluginCacheDir\skills\" -Recurse -Force
Copy-Item -Path (Join-Path $ProjectRoot "scripts\*") -Destination "$PluginCacheDir\scripts\" -Recurse -Force
Copy-Item -Path (Join-Path $ProjectRoot "assets\*") -Destination "$PluginCacheDir\assets\" -Recurse -Force
if (Test-Path (Join-Path $ProjectRoot "assets\workflows")) {
    New-Item -ItemType Directory -Force -Path (Join-Path $PluginCacheDir "workflows") | Out-Null
    Copy-Item -Path (Join-Path $ProjectRoot "assets\workflows\*.js") -Destination "$PluginCacheDir\workflows\" -Force -ErrorAction SilentlyContinue
}
Copy-Item -LiteralPath $EngineExe -Destination "$PluginCacheDir\bin\ccide-core.exe" -Force
Copy-Item -LiteralPath (Join-Path $ProjectRoot "AGENT.md") -Destination "$PluginCacheDir\AGENT.md" -Force

if ($OptPlugin -eq "1" -and $PyExe) {
    Backup-Once (Join-Path $ClaudeDir "settings.json")
    $StatusCmd = "`"" + (Join-Path $ClaudeDir "bin\ccide-core.exe") + "`" statusline"
    $RegScript = @"
import datetime, json, os, sys
proj, pcache, ver, cdir, sl_cmd = sys.argv[1:6]
pdir = os.path.join(cdir, "plugins")
os.makedirs(pdir, exist_ok=True)
now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000Z")
km_p = os.path.join(pdir, "known_marketplaces.json")
km = json.load(open(km_p, encoding="utf-8")) if os.path.isfile(km_p) else {}
km["cc-ide"] = {"source": {"source": "directory", "path": proj}, "installLocation": proj, "lastUpdated": now}
json.dump(km, open(km_p, "w", encoding="utf-8"), indent=2)
ip_p = os.path.join(pdir, "installed_plugins.json")
ip = json.load(open(ip_p, encoding="utf-8")) if os.path.isfile(ip_p) else {"version": 2, "plugins": {}}
ip.setdefault("plugins", {})["cc-ide@cc-ide"] = [{"scope": "user", "installPath": pcache, "version": ver, "installedAt": now, "lastUpdated": now}]
json.dump(ip, open(ip_p, "w", encoding="utf-8"), indent=2)
st_p = os.path.join(cdir, "settings.json")
st = json.load(open(st_p, encoding="utf-8")) if os.path.isfile(st_p) else {}
st.setdefault("enabledPlugins", {})["cc-ide@cc-ide"] = True
st.setdefault("extraKnownMarketplaces", {})["cc-ide"] = {"source": {"source": "directory", "path": proj}}
st["statusLine"] = {"type": "command", "command": sl_cmd}
json.dump(st, open(st_p, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
"@
    $RegScript | & $PyExe -I - $ProjectRoot $PluginCacheDir $PluginVersion $ClaudeDir $StatusCmd
    Write-Ok "Registered cc-ide@cc-ide & quoted statusLine in $ClaudeDir\settings.json"
}

Write-Step "5/6 Applying structural patches to Windows claude.exe..."
if ($OptPatch -eq "1" -and $PyExe) {
    & $PyExe -I (Join-Path $ProjectRoot "scripts\patch_claude_binary.py")
} else {
    Write-Ok "Skipped claude.exe binary patch"
}

Write-Step "6/6 Installation complete! Launch `ccide` inside WezTerm or Windows Terminal (`wt.exe`)."
& $EngineExe orchestrator sync 2>$null | Out-Null
Write-Ok "Run `ccide doctor` at any time to verify your Windows workspace."
