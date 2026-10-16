# setup.ps1 - builds the PC port of Ratchet & Clank (PAL / Europe or USA, v1.00) from YOUR OWN disc image. Windows PowerShell 5.1.
#
#   Double-click Setup.cmd: a small window opens (pick the .iso, Install, Play); no console window is shown.
#   For troubleshooting, the old console mode: Setup.cmd -Console, or
#   powershell -ExecutionPolicy Bypass -File .\setup.ps1 -Console -Iso "<your folder>\Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"
#   The window runs the same steps below as a hidden child (setup.ps1 -Console ...) and shows its progress.
#
# What it does, in order:
#   (a) downloads a private, pinned copy of Rust (GNU host, linker included), Python (embeddable) and the GNU assembler into .\toolchain
#       (once; nothing is installed on the PC, no admin rights, no PATH or registry change)
#   (b) checks that the disc image is Ratchet & Clank PAL / Europe v1.00 (SCES-50916) or USA v1.00 (SCUS-97199) (size and SHA-1)
#   (c) generates the game code from the disc image with tools\recgen_iso.py (boot code + 19 levels) into .\oracle\rec and .\oracle\game
#   (c2) generates the VU microcode translations (graphics code) from the disc image with tools\vugen_iso.py into
#        .\oracle\src\vu0_prog0.rs and .\rac-rs\src\gsfull\vu1_prog*.rs + vu1aot_gen.rs
#   (d) builds the game with cargo (release) into .\build
#   (e) writes generated\play.cfg (the disc image path) and Play.cmd, which starts the game from your disc image (cold boot: logos, title screen, New Game, no savestate),
#       and a desktop shortcut to the game exe (the exe, started with no arguments, reads generated\play.cfg)
# Everything it makes stays inside this folder (toolchain\, oracle\rec, oracle\game, the generated .rs files named above, generated\
# for logs and stamps, build\, Play.cmd, and player\ once you play), plus the desktop shortcut. Downloads (internet needed once, about
# 220 MB): Rust, Python and the GNU assembler into toolchain\ (pinned versions, SHA-256 checked) and the Rust crates that cargo fetches for the build
# (into toolchain\cargo). Delete the folder to remove everything. The disc image is only read.
#
# Switches: -Console (no window: run the steps here in the console),
#           -SkipHash (size check only), -CheckOnly (do (a) and (b) only), -Regenerate (redo (c)), -Jobs N (parallel generators, default 4),
#           -NoBuild (stop after (c2)), -NoShortcut (no desktop shortcut), -NoSharedSlow (generate the memory-access slow paths inline instead of as shared helpers).
param(
    [string]$Iso = '',
    [switch]$SkipHash,
    [switch]$CheckOnly,
    [switch]$Regenerate,
    [switch]$NoBuild,
    [switch]$NoSharedSlow,
    [switch]$NoShortcut,
    [int]$Jobs = 4,
    [switch]$Console,
    [string]$ShortcutDir = '',      # where the shortcut goes (default: the desktop)
    [switch]$TestNoShow,            # test only: build the launcher without showing it and save PNGs of its views
    [string]$TestShot = '',
    [string]$TestIso = '',
    [switch]$TestInstall,
    [switch]$Launch,                # started by the game exe: looks for an update; silent unless there is one
    [switch]$Update,                # the update window: waits for the game to close, then updates and rebuilds
    [switch]$ApplyUpdate,           # worker of the update window (with -Console): download + apply the update, then the normal steps
    [switch]$FinishOnly,            # with -Update: the files are already updated, only the rebuild is left
    [string]$NewVersion = '',
    [int]$ExePid = 0,               # the game exe that started -Launch (the update waits for it to close)
    [switch]$SkipUnblock,
    [string]$WindowFlag = '',       # -Launch: a file it creates when it shows a window (the exe then waits for the answer)
    [string]$TestUpdateBase = '',   # test only: folder / file:// / http URL that serves VERSION, CHANGELOG.md and v<version>.zip instead of GitHub
    [string]$TestChoice = ''        # test only (-Launch -TestNoShow): 'update' or 'notnow' instead of a click; 'firstview' (launcher test): only the first view
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location -LiteralPath $root

# The supported discs (v1.00 of each): Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00), serial SCES-50916, and
# Ratchet & Clank (USA) (En,Fr,De,Es,It), serial SCUS-97199. They have different sizes; the SHA-1 then proves the copy.
$Discs = @(
    @{ Size = [long]4213997568; Sha1 = 'B0DFBD8EAC0D84F173B6FC25D8A02A5FF22C652F'; Name = 'PAL / Europe v1.00 (SCES-50916)' },
    @{ Size = [long]4214095872; Sha1 = 'AEE9275384F739C9B24CD5DE6F7E200292DB7B01'; Name = 'USA v1.00 (SCUS-97199)' }
)
# The PAL values until a disc is picked; (b) sets them from the disc found.
$ExpectedSize = $Discs[0].Size
$ExpectedSha1 = $Discs[0].Sha1
$DiscName = $Discs[0].Name
$GenFlags = @('--no-comments', '--compact', '--shared-slow', '--outline-mem')
if ($NoSharedSlow) { $GenFlags = @('--no-comments', '--compact', '--outline-mem') }   # inline slow paths: bigger source and slower build, speed measured in SETUP_NOTES 7b
$Levels = 0..18

function Step($text) { Write-Host ''; Write-Host "== $text" -ForegroundColor Cyan }
function Fail($text) { Write-Host "ERROR: $text" -ForegroundColor Red; exit 1 }
$timings = [ordered]@{}

# ================================================================ starting the game, desktop shortcut (shared by the launcher and -Console)
function Test-Installed {
    return ((Test-Path -LiteralPath (Join-Path $root 'build\release\rac-rs.exe')) -and (Test-Path -LiteralPath (Join-Path $root 'Play.cmd')) -and (Test-Path -LiteralPath (Join-Path $root 'generated\play.cfg')))
}

function Write-PlayCfg([string]$iso) {
    # generated\play.cfg: key=value lines (UTF-8, no BOM). The game exe, started with no arguments, reads it to find the disc image.
    $dir = Join-Path $root 'generated'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $dir 'play.cfg'), ("iso=$iso`n"), (New-Object System.Text.UTF8Encoding($false)))
}

function Get-InstalledIso {
    # the disc image path that setup wrote into generated\play.cfg (Play.cmd as a fallback)
    $cfg = Join-Path $root 'generated\play.cfg'
    if (Test-Path -LiteralPath $cfg) {
        foreach ($l in ([System.IO.File]::ReadAllLines($cfg, (New-Object System.Text.UTF8Encoding($false))))) { if ($l -match '^iso=(.+)$') { return $Matches[1].Trim() } }
    }
    $pc = Join-Path $root 'Play.cmd'
    if (Test-Path -LiteralPath $pc) {
        foreach ($l in (Get-Content -LiteralPath $pc)) { if ($l -match '^set "ORACLE_ISO=(.*)"\s*$') { return $Matches[1] } }
    }
    return ''
}

function Get-IconFile {
    # the game logo as an .ico (shipped in the folder), or '' when it is missing
    $ico = Join-Path $root 'rac-rs\assets\icon\rac-rs.ico'
    if (Test-Path -LiteralPath $ico -PathType Leaf) { return $ico }
    return ''
}

function Get-GamePsi {
    # The game exe itself, started with no arguments (it reads generated\play.cfg), no console window, working folder = this folder.
    $exe = Join-Path $root 'build\release\rac-rs.exe'
    if (-not (Test-Path -LiteralPath $exe)) { throw 'The game is not built yet. Run Setup.cmd first.' }
    $iso = Get-InstalledIso
    if (-not $iso) { throw 'The setup information was not found. Open Setup.cmd and use Repair / rebuild.' }
    if (-not (Test-Path -LiteralPath $iso -PathType Leaf)) { throw "Your disc image was not found at:`r`n$iso`r`n`r`nIf you moved it, open Setup.cmd and use Repair / rebuild to pick it again." }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = ''
    $psi.WorkingDirectory = $root
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    return $psi
}

function Start-Game([bool]$noUpdateCheck = $false) {
    $psi = Get-GamePsi
    if ($noUpdateCheck) { $psi.EnvironmentVariables['RAC_NO_UPDATE_CHECK'] = '1' }
    $pl = Join-Path $root 'player'
    if (-not (Test-Path -LiteralPath $pl)) { New-Item -ItemType Directory -Force -Path $pl | Out-Null }
    # safety copies of the save card before each start (the newest two starts), as Play.cmd does
    try {
        $c1 = Join-Path $pl 'card.start1.bin'; $c2 = Join-Path $pl 'card.start2.bin'; $c0 = Join-Path $pl 'card.bin'
        if (Test-Path -LiteralPath $c1) { Copy-Item -LiteralPath $c1 -Destination $c2 -Force }
        if (Test-Path -LiteralPath $c0) { Copy-Item -LiteralPath $c0 -Destination $c1 -Force }
    } catch { }
    $null = [System.Diagnostics.Process]::Start($psi)
}

function New-GameShortcut([string]$dir) {
    # Desktop shortcut: the game exe itself (no arguments, working folder = this folder).
    $exe = Join-Path $root 'build\release\rac-rs.exe'
    $lnk = Join-Path $dir 'Ratchet & Clank (rac-rs).lnk'
    $sc = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
    $sc.TargetPath = $exe
    $sc.Arguments = ''
    $sc.WorkingDirectory = $root
    $ico = Get-IconFile
    $sc.IconLocation = $(if ($ico) { $ico } else { "$exe,0" })
    $sc.Description = 'Ratchet & Clank (rac-rs)'
    $sc.Save()
    return $lnk
}

# ================================================================ updates (shared by -Launch, the update window, the launcher and the -Console worker)
# Version = the one line in VERSION (missing = 0.1.0, the first public release). generated\built_version.txt = the version the game exe was built from.
$RepoRaw = 'https://raw.githubusercontent.com/AsobiCodes/rac-rs/main'
$RepoArchive = 'https://github.com/AsobiCodes/rac-rs/archive'

function Get-LocalVersion {
    $f = Join-Path $root 'VERSION'
    if (Test-Path -LiteralPath $f -PathType Leaf) { try { $v = ([System.IO.File]::ReadAllText($f)).Trim(); if ($v -match '^\d+\.\d+\.\d+$') { return $v } } catch { } }
    return '0.1.0'
}

function Get-BuiltVersion {
    $f = Join-Path $root 'generated\built_version.txt'
    if (Test-Path -LiteralPath $f -PathType Leaf) { try { $v = ([System.IO.File]::ReadAllText($f)).Trim(); if ($v -match '^\d+\.\d+\.\d+$') { return $v } } catch { } }
    if (Test-Installed) { return '0.1.0' }      # built before versions existed
    return ''
}

function Compare-Version([string]$a, [string]$b) { return ([version]$a).CompareTo([version]$b) }

function Read-Remote([string]$name, [int]$timeoutMs = 4000) {
    # Text of VERSION / CHANGELOG.md from GitHub (or from -TestUpdateBase: a folder, a file:// or an http URL). $null on any failure; never throws.
    try {
        $base = $(if ($TestUpdateBase) { $TestUpdateBase } else { $RepoRaw })
        if ($base -notmatch '^(https?|file)://') {
            $f = Join-Path $base $name
            if (-not (Test-Path -LiteralPath $f -PathType Leaf)) { return $null }
            return [System.IO.File]::ReadAllText($f)
        }
        try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }
        $wc = New-Object System.Net.WebClient
        $wc.Headers['User-Agent'] = 'rac-rs-updater'
        # Started without the window's synchronization context: WebClient hands its result to the context it was started on, and the
        # window's thread is blocked in Wait below, so with a window open (the change list, "Check for updates") it always timed out.
        $ctx = [System.Threading.SynchronizationContext]::Current
        [System.Threading.SynchronizationContext]::SetSynchronizationContext($null)
        try { $task = $wc.DownloadStringTaskAsync(($base.TrimEnd('/') + '/' + $name)) } finally { [System.Threading.SynchronizationContext]::SetSynchronizationContext($ctx) }
        if (-not $task.Wait($timeoutMs)) { try { $wc.CancelAsync() } catch { }; return $null }
        return [string]$task.Result
    } catch { return $null }
}

function Get-ChangelogFor([string]$text, [string]$from, [string]$to) {
    # CHANGELOG.md has one section per version, newest first, starting with "## v<version>". Returns the versions newer than $from up to $to.
    $out = New-Object System.Collections.ArrayList
    $take = $false
    foreach ($l in ($text -split "`r?`n")) {
        if ($l -match '^##\s+v?(\d+\.\d+\.\d+)') {
            $v = $Matches[1]
            $take = ((Compare-Version $v $from) -gt 0) -and ((Compare-Version $v $to) -le 0)
            if ($take) { if ($out.Count -gt 0) { [void]$out.Add('') }; [void]$out.Add('Version ' + $v) }
            continue
        }
        if ($take -and $l.Trim().Length -gt 0) { [void]$out.Add(($l -replace '^\s*[-*]\s+', ([string][char]0x2022 + ' '))) }
    }
    return ($out -join "`r`n")
}

function Get-FilesHash([string[]]$rels) {
    # SHA-256 over the files (path, NUL, content without CR) so that line-ending changes by git or an unzipper do not matter.
    $latin = [System.Text.Encoding]::GetEncoding(28591)
    $ms = New-Object System.IO.MemoryStream
    foreach ($r in $rels) {
        $nb = $latin.GetBytes($r + [string][char]0); $ms.Write($nb, 0, $nb.Length)
        $b = $latin.GetBytes($latin.GetString([System.IO.File]::ReadAllBytes((Join-Path $root $r))).Replace("`r", '')); $ms.Write($b, 0, $b.Length)
    }
    $ms.Position = 0
    $sha = [System.Security.Cryptography.SHA256]::Create()
    return ([System.BitConverter]::ToString($sha.ComputeHash($ms))).Replace('-', '').ToLowerInvariant()
}

function Get-VuGenFiles {
    $names = [string[]]@([System.IO.Directory]::GetFiles((Join-Path $root 'tools\vu')) | ForEach-Object { [System.IO.Path]::GetFileName($_) })
    [Array]::Sort($names, [System.StringComparer]::Ordinal)
    return @('tools/vugen_iso.py') + @($names | ForEach-Object { 'tools/vu/' + $_ })
}

function Test-ProtectedPath([string]$rel) {
    # What an update never touches: the toolchain, the build, the generated code, the player's saves, the disc image.
    $r = $rel.Replace('\', '/').TrimStart('/').ToLowerInvariant()
    foreach ($p in @('toolchain/', 'build/', 'generated/', 'player/', 'oracle/rec/', 'oracle/game/', 'test_shots/')) { if ($r.StartsWith($p)) { return $true } }
    if ($r -in @('play.cmd', 'oracle/src/vu0_prog0.rs', 'rac-rs/src/gsfull/vu1aot_gen.rs', 'rac-rs/src/particles/orb_tables.rs')) { return $true }
    if ($r -match '^rac-rs/src/gsfull/vu1_prog\d+\.rs$') { return $true }
    return $r.EndsWith('.iso')
}

function Get-Sha256Hex([string]$path) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $fs = [System.IO.File]::OpenRead($path)
    try { return [System.BitConverter]::ToString($sha.ComputeHash($fs)) } finally { $fs.Dispose() }
}

function Save-Url([string]$url, [string]$file) {
    try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }
    if ($url -like 'file://*') { (New-Object System.Net.WebClient).DownloadFile($url, $file); return }
    $old = $ProgressPreference; $ProgressPreference = 'SilentlyContinue'
    try { Invoke-WebRequest -Uri $url -OutFile $file -UseBasicParsing -TimeoutSec 600 } finally { $ProgressPreference = $old }
}

function Get-UpdateZip([string]$ver, [string]$zip) {
    # v<ver>.zip from the GitHub tag (the main branch only if the tag does not exist); with -TestUpdateBase from there. Throws on failure.
    if ($TestUpdateBase) {
        if ($TestUpdateBase -match '^(https?|file)://') { Save-Url ($TestUpdateBase.TrimEnd('/') + "/v$ver.zip") $zip; return }
        $f = Join-Path $TestUpdateBase "v$ver.zip"
        if (-not (Test-Path -LiteralPath $f -PathType Leaf)) { throw "not found: $f" }
        Copy-Item -LiteralPath $f -Destination $zip -Force
        return
    }
    $notFound = $false
    for ($try = 1; $try -le 3; $try++) {
        try { Save-Url "$RepoArchive/refs/tags/v$ver.zip" $zip; return }
        catch {
            $code = 0; try { $code = [int]$_.Exception.Response.StatusCode } catch { }
            if ($code -eq 404) { $notFound = $true; break }
            Write-Host "  download failed ($($_.Exception.Message)), retrying" -ForegroundColor Yellow
            if ($try -eq 3) { throw }
            Start-Sleep -Seconds (3 * $try)
        }
    }
    if ($notFound) { Write-Host '  no tag for this version yet: using the main branch'; Save-Url "$RepoArchive/refs/heads/main.zip" $zip }
}

function Expand-UpdateZip([string]$zip, [string]$outDir) {
    # Unpacks the zip without its single top folder (GitHub: rac-rs-<version>/). Refuses paths that leave $outDir.
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    $outFull = [System.IO.Path]::GetFullPath($outDir).TrimEnd('\') + '\'
    $za = [System.IO.Compression.ZipFile]::OpenRead($zip)
    $top = $null
    try {
        foreach ($e in $za.Entries) {
            $n = $e.FullName.Replace('\', '/')
            if ($n.StartsWith('/') -or $n -match '(^|/)\.\.(/|$)') { throw "unsafe path in the update file: $n" }
            $i = $n.IndexOf('/')
            if ($i -lt 1) { continue }
            $t = $n.Substring(0, $i)
            if ($null -eq $top) { $top = $t } elseif ($top -ne $t) { throw 'the update file has an unexpected layout' }
            $rel = $n.Substring($i + 1)
            if ($rel.Length -eq 0 -or $rel.EndsWith('/')) { continue }
            $dst = [System.IO.Path]::GetFullPath((Join-Path $outFull ($rel.Replace('/', '\'))))
            if (-not $dst.StartsWith($outFull, [System.StringComparison]::OrdinalIgnoreCase)) { throw "unsafe path in the update file: $n" }
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dst) | Out-Null
            [System.IO.Compression.ZipFileExtensions]::ExtractToFile($e, $dst, $true)
        }
    } finally { $za.Dispose() }
    foreach ($need in @('VERSION', 'setup.ps1', 'rac-rs\Cargo.toml')) { if (-not (Test-Path -LiteralPath (Join-Path $outFull $need))) { throw "the update file is not a rac-rs release ($need missing)" } }
}

function Sync-Update([string]$src, [string]$dst) {
    # Copies the new version over this install. Only files that are new or differ (SHA-256) are copied, and they get the CURRENT time
    # (cargo rebuilds by file time); unchanged files are not touched, so nothing that did not change is rebuilt. Files the old
    # release_files.txt lists and the new one does not are deleted (no old list = no deletions). Protected paths are never touched.
    # VERSION and release_files.txt come last: an interrupted copy leaves the old VERSION, so the update is offered again.
    $src = [System.IO.Path]::GetFullPath($src).TrimEnd('\') + '\'
    $dst = [System.IO.Path]::GetFullPath($dst).TrimEnd('\') + '\'
    $oldList = @(); $newList = @()
    if (Test-Path -LiteralPath (Join-Path $dst 'release_files.txt')) { $oldList = @(Get-Content -LiteralPath (Join-Path $dst 'release_files.txt') | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }
    if (Test-Path -LiteralPath (Join-Path $src 'release_files.txt')) { $newList = @(Get-Content -LiteralPath (Join-Path $src 'release_files.txt') | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }
    $last = @('VERSION', 'release_files.txt')
    $items = New-Object System.Collections.ArrayList
    foreach ($f in (Get-ChildItem -LiteralPath $src -Recurse -File)) {
        $rel = $f.FullName.Substring($src.Length).Replace('\', '/')
        if ((Test-ProtectedPath $rel) -or ($last -contains $rel)) { continue }
        [void]$items.Add([pscustomobject]@{ Rel = $rel; Full = $f.FullName; Len = $f.Length })
    }
    foreach ($rel in $last) { $p = Join-Path $src $rel; if (Test-Path -LiteralPath $p) { [void]$items.Add([pscustomobject]@{ Rel = $rel; Full = $p; Len = (Get-Item -LiteralPath $p).Length }) } }
    $changed = New-Object System.Collections.ArrayList
    foreach ($it in $items) {
        $d = Join-Path $dst ($it.Rel.Replace('/', '\'))
        $copy = $true
        if (Test-Path -LiteralPath $d -PathType Leaf) {
            if ((Get-Item -LiteralPath $d).Length -eq $it.Len -and (Get-Sha256Hex $d) -eq (Get-Sha256Hex $it.Full)) { $copy = $false }
        }
        if ($copy) {
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $d) | Out-Null
            [System.IO.File]::Copy($it.Full, $d, $true)
            [System.IO.File]::SetLastWriteTimeUtc($d, [DateTime]::UtcNow)
            [void]$changed.Add($it.Rel)
            Write-Host "  updated $($it.Rel)"
        }
    }
    $removed = New-Object System.Collections.ArrayList
    if ($oldList.Count -gt 0 -and $newList.Count -gt 0) {
        $keep = @{}; foreach ($n in $newList) { $keep[$n] = $true }
        foreach ($o in $oldList) {
            if ($keep.ContainsKey($o) -or ($last -contains $o) -or (Test-ProtectedPath $o) -or $o -match '(^|/)\.\.(/|$)' -or $o.StartsWith('/')) { continue }
            $d = Join-Path $dst ($o.Replace('/', '\'))
            if (Test-Path -LiteralPath $d -PathType Leaf) {
                Remove-Item -LiteralPath $d -Force
                [void]$removed.Add($o)
                Write-Host "  removed $o"
                $dir = Split-Path -Parent $d
                while ($dir.Length -gt $dst.Length -and @(Get-ChildItem -LiteralPath $dir -Force).Count -eq 0) { Remove-Item -LiteralPath $dir -Force; $dir = Split-Path -Parent $dir }
            }
        }
    }
    # Marker for the -ApplyUpdate worker: this version's files were put in place HERE (changed files carry the current time). The setup build removes it when it succeeds.
    $gdir = Join-Path $dst 'generated'
    New-Item -ItemType Directory -Force -Path $gdir | Out-Null
    $ver = $(try { ([System.IO.File]::ReadAllText((Join-Path $dst 'VERSION'))).Trim() } catch { '' })
    if ($ver -match '^\d+\.\d+\.\d+$') { [System.IO.File]::WriteAllText((Join-Path $gdir ("synced_$ver")), "$ver`n", (New-Object System.Text.UTF8Encoding($false))) }
    return [pscustomobject]@{ Changed = $changed; Removed = $removed }
}

function Set-ReleaseFileTimes {
    # Sets the time of every release file (release_files.txt) to now; returns how many. Used when the files of a new version were NOT copied by
    # Sync-Update but unzipped by hand: a ZIP keeps the times of its files (the commit time), and if the last build is newer than those, cargo would
    # think nothing changed and silently keep the old game. With the times set to now, the next build compiles everything once.
    $rels = @()
    $list = Join-Path $root 'release_files.txt'
    if (Test-Path -LiteralPath $list -PathType Leaf) { $rels = @(Get-Content -LiteralPath $list | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }
    else {
        foreach ($d in @('oracle', 'rac-rs', 'tools')) {
            $dd = Join-Path $root $d
            if (Test-Path -LiteralPath $dd) { foreach ($f in (Get-ChildItem -LiteralPath $dd -Recurse -File)) { $rels += $f.FullName.Substring($root.Length).TrimStart('\').Replace('\', '/') } }
        }
    }
    $now = [DateTime]::UtcNow; $n = 0
    foreach ($r in $rels) {
        if ((Test-ProtectedPath $r) -or $r.StartsWith('/') -or $r -match '(^|/)\.\.(/|$)') { continue }
        $f = Join-Path $root ($r.Replace('/', '\'))
        if (Test-Path -LiteralPath $f -PathType Leaf) { [System.IO.File]::SetLastWriteTimeUtc($f, $now); $n++ }
    }
    return $n
}

function Get-GameProcs {
    $exe = Join-Path $root 'build\release\rac-rs.exe'
    $r = @()
    foreach ($p in @(Get-Process -Name 'rac-rs' -ErrorAction SilentlyContinue)) { try { if ($p.Path -eq $exe) { $r += $p } } catch { } }
    return $r
}

function Wait-GameClosed {
    # The build cannot replace rac-rs.exe while it runs: wait for the exe that started the update (and any other of this install).
    if ($ExePid -gt 0) { try { $p = [System.Diagnostics.Process]::GetProcessById($ExePid); [void]$p.WaitForExit(60000) } catch { } }
    $end = (Get-Date).AddSeconds(10)
    while (((Get-GameProcs).Count -gt 0) -and ((Get-Date) -lt $end)) { Start-Sleep -Milliseconds 300 }
}

function Start-UpdateProcess([string]$ver, [bool]$finish) {
    # A separate hidden setup.ps1 -Update: it outlives this process (which tells the game exe to exit) and shows the update window.
    $a = '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "' + (Join-Path $root 'setup.ps1') + '" -Update -NewVersion ' + $ver + ' -ExePid ' + $ExePid
    if ($finish) { $a += ' -FinishOnly' }
    if ($TestUpdateBase) { $a += ' -TestUpdateBase "' + $TestUpdateBase.TrimEnd('\') + '"' }
    if ($TestNoShow) { $a += ' -TestNoShow -TestShot "' + $(if ($TestShot) { $TestShot.TrimEnd('\') } else { Join-Path $root 'test_shots' }) + '"' }
    $null = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -ArgumentList $a -WorkingDirectory $root -WindowStyle Hidden
}

# -Launch (started by the game exe): is there something to update? Offline, an error or "not newer" = exit 0 at once, no window.
$launchMode = ''; $launchNew = ''
if ($Launch) {
    try {
        $lv = Get-LocalVersion; $bv = Get-BuiltVersion
        if ($bv -and ($bv -ne $lv)) { $launchMode = 'finish'; $launchNew = $lv }
        # Also when an update is unfinished: a newer version online replaces it (the unfinished one may be what fails to build, e.g. a bad release).
        $rv = Read-Remote 'VERSION'
        if ($rv) { $rv = $rv.Trim(); if (($rv -match '^\d+\.\d+\.\d+$') -and ((Compare-Version $rv $lv) -gt 0)) { $launchMode = 'offer'; $launchNew = $rv } }
    } catch { $launchMode = '' }
    if (-not $launchMode) { exit 0 }
    if ($WindowFlag) { try { [System.IO.File]::WriteAllText($WindowFlag, 'window') } catch { } }     # the exe now waits for the answer without a time limit
}

# ================================================================ launcher window (default; -Console skips it and runs the steps below in the console)
if (-not $Console) {
    $S = @{
        view = ''; discOk = $false; discPath = ''; check = $null; repair = $false
        proc = $null; outPath = ''; errPath = ''; outPos = [long]0; errPos = [long]0; outRest = ''; errRest = ''
        log = (New-Object System.Text.StringBuilder); step = 0; levels = 0; compiled = 0; progress = 0.0
        t0 = $null; lastError = ''; status = ''; shortcut = $false; ranOk = $false; finished = $false
        update = $false; newV = ''; finish = $false; finishPending = $false; workerExtra = ''; workerCode = 0; buildT0 = $null; expect = 0; exitCode = 0
        test = [bool]$TestNoShow; toolsSub = 0.0; vu = $false; suppress = $false; shortcutDir = ''; shortcutPath = ''
    }
    $StepNames = @('Download tools', 'Check disc', 'Prepare game code', 'Build game', 'Finish')
    $StepNamesInstall = $StepNames
    $StepNamesUpdate = @('Download update', 'Check tools', 'Prepare game code', 'Build game', 'Finish')
    $ExpectedCompiles = 528      # "Compiling" lines of a complete first build (measured); the bar never goes above 98% for the build
    $AppTitle = 'Ratchet & Clank (rac-rs)'
    $psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $UI = @{}

    try {
        Add-Type -AssemblyName System.Windows.Forms
        Add-Type -AssemblyName System.Drawing
        # sharp text on high-DPI screens (no compiler needed: a dynamic P/Invoke)
        try {
            $ab = [AppDomain]::CurrentDomain.DefineDynamicAssembly((New-Object Reflection.AssemblyName('RacDpi')), [Reflection.Emit.AssemblyBuilderAccess]::Run)
            $tb = $ab.DefineDynamicModule('m').DefineType('N', 'Public,Class')
            $null = $tb.DefinePInvokeMethod('SetProcessDPIAware', 'user32.dll', 'Public,Static,PinvokeImpl', 'Standard', [bool], [Type[]]@(), 'Winapi', 'Auto')
            $null = $tb.CreateType().GetMethod('SetProcessDPIAware').Invoke($null, $null)
        } catch { }
        [System.Windows.Forms.Application]::EnableVisualStyles()
        [System.Windows.Forms.Application]::SetCompatibleTextRenderingDefault($false)
    } catch {
        Write-Host "The launcher window could not start ($($_.Exception.Message)). Use Setup.cmd -Console instead." -ForegroundColor Red
        exit 1
    }
    $Mutex = $null
    if (-not $TestNoShow) {
        $created = $false
        $mname = $(if ($Launch) { 'Local\RacRsUpdateOffer' } elseif ($Update) { 'Local\RacRsUpdate' } else { 'Local\RacRsLauncher' })
        $Mutex = New-Object System.Threading.Mutex($true, $mname, [ref]$created)
        if (-not $created) {
            if (-not $Launch -and -not $Update) { [void][System.Windows.Forms.MessageBox]::Show('The Ratchet & Clank launcher is already open.', $AppTitle, 'OK', 'Information') }
            exit 0
        }
    }

    # ---------------------------------------------------------------- helpers
    $fN = New-Object System.Drawing.Font('Segoe UI', 9.5)
    $fSmall = New-Object System.Drawing.Font('Segoe UI', 8.75)
    $fTitle = New-Object System.Drawing.Font('Segoe UI Semibold', 16)
    $fBig = New-Object System.Drawing.Font('Segoe UI Semibold', 20)
    $fGlyph = New-Object System.Drawing.Font('Segoe UI Symbol', 11)
    $fMono = New-Object System.Drawing.Font('Consolas', 8.5)
    $cText = [System.Drawing.Color]::FromArgb(32, 32, 32)
    $cGrey = [System.Drawing.Color]::FromArgb(110, 110, 110)
    $cGreen = [System.Drawing.Color]::FromArgb(16, 124, 16)
    $cRed = [System.Drawing.Color]::FromArgb(196, 43, 28)
    $cBlue = [System.Drawing.Color]::FromArgb(0, 103, 192)
    $cAccent = [System.Drawing.Color]::FromArgb(0, 120, 212)

    function New-Label([string]$text, [int]$x, [int]$y, [int]$w, [int]$h, $font, $color) {
        $l = New-Object System.Windows.Forms.Label
        $l.AutoSize = $false; $l.UseMnemonic = $false
        $l.Location = New-Object System.Drawing.Point($x, $y); $l.Size = New-Object System.Drawing.Size($w, $h)
        $l.Text = $text; $l.Font = $font; $l.ForeColor = $color
        return $l
    }
    function New-Button([string]$text, [int]$x, [int]$y, [int]$w, [int]$h, [bool]$primary) {
        $b = New-Object System.Windows.Forms.Button
        $b.Location = New-Object System.Drawing.Point($x, $y); $b.Size = New-Object System.Drawing.Size($w, $h)
        $b.Text = $text; $b.Font = $fN; $b.UseVisualStyleBackColor = $true
        if ($primary) { $b.FlatStyle = 'Flat'; $b.FlatAppearance.BorderSize = 0; $b.BackColor = $cAccent; $b.ForeColor = [System.Drawing.Color]::White; $b.UseVisualStyleBackColor = $false }
        return $b
    }
    function New-Link([string]$text, [int]$x, [int]$y, [int]$w, [int]$h) {
        $l = New-Object System.Windows.Forms.LinkLabel
        $l.AutoSize = $false; $l.Location = New-Object System.Drawing.Point($x, $y); $l.Size = New-Object System.Drawing.Size($w, $h)
        $l.Text = $text; $l.Font = $fN; $l.LinkColor = $cBlue; $l.ActiveLinkColor = $cAccent
        return $l
    }
    function New-Panel() {
        $p = New-Object System.Windows.Forms.Panel
        $p.Dock = 'Fill'; $p.BackColor = [System.Drawing.Color]::White; $p.Visible = $false
        return $p
    }
    function New-Bar([int]$x, [int]$y, [int]$w, [int]$h) {
        # a plain bar (track + fill) that draws the same everywhere
        $t = New-Object System.Windows.Forms.Panel
        $t.Location = New-Object System.Drawing.Point($x, $y); $t.Size = New-Object System.Drawing.Size($w, $h)
        $t.BackColor = [System.Drawing.Color]::FromArgb(228, 228, 228); $t.Tag = 0
        $f = New-Object System.Windows.Forms.Panel
        $f.Dock = 'Left'; $f.Width = 0; $f.BackColor = $cAccent
        $t.Controls.Add($f)
        return $t
    }
    function Set-Bar($bar, [int]$pct) {
        $bar.Tag = $pct
        $bar.Controls[0].Width = [int][math]::Floor($bar.ClientSize.Width * $pct / 100.0)
    }
    function Format-Elapsed($ts) { return ('{0:00}:{1:00}' -f [int][math]::Floor($ts.TotalMinutes), $ts.Seconds) }

    # ---------------------------------------------------------------- the window
    $form = New-Object System.Windows.Forms.Form
    $UI.form = $form
    $form.Text = $AppTitle
    $form.AutoScaleDimensions = New-Object System.Drawing.SizeF(96, 96)
    $form.AutoScaleMode = 'Dpi'
    $form.Font = $fN
    $form.ClientSize = New-Object System.Drawing.Size(580, 440)
    $form.FormBorderStyle = 'FixedDialog'
    $form.MaximizeBox = $false
    $form.StartPosition = 'CenterScreen'
    $form.BackColor = [System.Drawing.Color]::White
    $exePath = Join-Path $root 'build\release\rac-rs.exe'
    $icoFile = Get-IconFile
    if ($icoFile) { try { $form.Icon = New-Object System.Drawing.Icon($icoFile) } catch { $icoFile = '' } }
    if (-not $icoFile -and (Test-Path -LiteralPath $exePath)) { try { $form.Icon = [System.Drawing.Icon]::ExtractAssociatedIcon($exePath) } catch { } }

    # --- view: pick the disc image
    $pPick = New-Panel; $UI.pPick = $pPick
    $pPick.Controls.Add((New-Label 'Set up Ratchet & Clank' 28 22 520 36 $fTitle $cText))
    $UI.lblPickText = New-Label 'Pick the .iso image of your own Ratchet & Clank disc (PAL / Europe or USA, v1.00)' 28 68 520 44 $fN $cText
    $pPick.Controls.Add($UI.lblPickText)
    $UI.txtIso = New-Object System.Windows.Forms.TextBox
    $UI.txtIso.Location = New-Object System.Drawing.Point(28, 124); $UI.txtIso.Size = New-Object System.Drawing.Size(404, 26); $UI.txtIso.Font = $fN
    $pPick.Controls.Add($UI.txtIso)
    $UI.btnBrowse = New-Button 'Browse...' 442 122 110 30 $false
    $pPick.Controls.Add($UI.btnBrowse)
    $UI.pbCheck = New-Bar 28 164 524 8
    $UI.pbCheck.Visible = $false
    $pPick.Controls.Add($UI.pbCheck)
    $UI.lblDisc = New-Label '' 28 180 524 62 $fN $cGrey
    $pPick.Controls.Add($UI.lblDisc)
    $UI.lblPickNote = New-Label 'Your disc image is only read. Nothing is uploaded.' 28 252 524 22 $fSmall $cGrey
    $pPick.Controls.Add($UI.lblPickNote)
    $UI.chkShortcut = New-Object System.Windows.Forms.CheckBox
    $UI.chkShortcut.Text = 'Create desktop shortcut'; $UI.chkShortcut.Checked = $true; $UI.chkShortcut.Font = $fN
    $UI.chkShortcut.Location = New-Object System.Drawing.Point(28, 300); $UI.chkShortcut.Size = New-Object System.Drawing.Size(300, 26)
    $pPick.Controls.Add($UI.chkShortcut)
    $pPick.Controls.Add((New-Label 'Needs about 5 GB of free disk space and an internet connection (once).' 28 372 400 40 $fSmall $cGrey))
    $UI.btnInstall = New-Button 'Install' 442 364 110 42 $true
    $pPick.Controls.Add($UI.btnInstall)

    # --- view: installing
    $pInst = New-Panel; $UI.pInst = $pInst
    $UI.lblInstTitle = New-Label 'Installing' 28 22 520 36 $fTitle $cText
    $pInst.Controls.Add($UI.lblInstTitle)
    $UI.stepGlyph = @(); $UI.stepText = @()
    for ($i = 0; $i -lt $StepNames.Count; $i++) {
        $g = New-Label '' 30 (70 + 30 * $i) 26 26 $fGlyph $cGrey
        $t = New-Label $StepNames[$i] 62 (72 + 30 * $i) 300 24 $fN $cGrey
        $pInst.Controls.Add($g); $pInst.Controls.Add($t)
        $UI.stepGlyph += $g; $UI.stepText += $t
    }
    $UI.pb = New-Bar 28 234 524 14
    $pInst.Controls.Add($UI.pb)
    $UI.lblStatus = New-Label '' 28 256 524 20 $fSmall $cGrey
    $pInst.Controls.Add($UI.lblStatus)
    $UI.lblElapsed = New-Label 'Elapsed: 00:00' 28 284 300 22 $fN $cText
    $pInst.Controls.Add($UI.lblElapsed)
    $UI.lblPct = New-Label '0%' 452 284 100 22 $fN $cText
    $UI.lblPct.TextAlign = 'TopRight'
    $pInst.Controls.Add($UI.lblPct)
    $UI.lblInstNote = New-Label 'This takes about 15 minutes. You can keep using your PC.' 28 312 524 22 $fN $cText
    $pInst.Controls.Add($UI.lblInstNote)
    $UI.lnkDetails = New-Link 'Show details' 28 346 200 22
    $pInst.Controls.Add($UI.lnkDetails)
    $UI.txtLog = New-Object System.Windows.Forms.TextBox
    $UI.txtLog.Multiline = $true; $UI.txtLog.ReadOnly = $true; $UI.txtLog.ScrollBars = 'Both'; $UI.txtLog.WordWrap = $false; $UI.txtLog.Font = $fMono
    $UI.txtLog.Location = New-Object System.Drawing.Point(28, 376); $UI.txtLog.Size = New-Object System.Drawing.Size(524, 200); $UI.txtLog.Visible = $false
    $UI.txtLog.Anchor = 'Top,Left,Right,Bottom'
    $pInst.Controls.Add($UI.txtLog)

    # --- view: done / already installed
    $pDone = New-Panel; $UI.pDone = $pDone
    $UI.lblDoneTitle = New-Label 'Ready to play' 28 22 524 40 $fTitle $cText
    $pDone.Controls.Add($UI.lblDoneTitle)
    $UI.lblDoneText = New-Label '' 28 66 524 44 $fN $cText
    $pDone.Controls.Add($UI.lblDoneText)
    $UI.btnPlay = New-Button 'Play' 140 130 300 84 $true
    $UI.btnPlay.Font = $fBig
    $pDone.Controls.Add($UI.btnPlay)
    $UI.btnFolder = New-Button 'Open game folder' 200 236 180 34 $false
    $pDone.Controls.Add($UI.btnFolder)
    $UI.lnkRepair = New-Link 'Repair / rebuild' 200 288 180 22
    $UI.lnkRepair.TextAlign = 'MiddleCenter'
    $pDone.Controls.Add($UI.lnkRepair)
    $UI.lblDoneNote = New-Label '' 28 340 524 60 $fSmall $cGrey
    $pDone.Controls.Add($UI.lblDoneNote)
    $UI.lblVersion = New-Label '' 28 408 200 20 $fSmall $cGrey
    $pDone.Controls.Add($UI.lblVersion)
    $UI.lnkCheck = New-Link 'Check for updates' 392 406 160 22
    $UI.lnkCheck.Font = $fSmall; $UI.lnkCheck.TextAlign = 'MiddleRight'
    $pDone.Controls.Add($UI.lnkCheck)

    # --- view: error
    $pErr = New-Panel; $UI.pErr = $pErr
    $pErr.Controls.Add((New-Label 'Something went wrong' 28 22 524 36 $fTitle $cRed))
    $UI.lblErrMsg = New-Label '' 28 64 524 84 $fN $cText
    $pErr.Controls.Add($UI.lblErrMsg)
    $UI.txtErrLog = New-Object System.Windows.Forms.TextBox
    $UI.txtErrLog.Multiline = $true; $UI.txtErrLog.ReadOnly = $true; $UI.txtErrLog.ScrollBars = 'Both'; $UI.txtErrLog.WordWrap = $false; $UI.txtErrLog.Font = $fMono
    $UI.txtErrLog.Location = New-Object System.Drawing.Point(28, 154); $UI.txtErrLog.Size = New-Object System.Drawing.Size(524, 200)
    $pErr.Controls.Add($UI.txtErrLog)
    $UI.btnCopy = New-Button 'Copy details' 28 372 130 34 $false
    $pErr.Controls.Add($UI.btnCopy)
    $UI.btnRetry = New-Button 'Try again' 322 372 110 34 $true
    $pErr.Controls.Add($UI.btnRetry)
    $UI.btnErrClose = New-Button 'Close' 442 372 110 34 $false
    $pErr.Controls.Add($UI.btnErrClose)

    # --- view: an update is available (shown by -Launch)
    $pOffer = New-Panel; $UI.pOffer = $pOffer
    $UI.lblOfferTitle = New-Label 'Update available' 28 22 524 36 $fTitle $cText
    $pOffer.Controls.Add($UI.lblOfferTitle)
    $pOffer.Controls.Add((New-Label 'What is new' 28 66 300 22 $fN $cGrey))
    $UI.txtChanges = New-Object System.Windows.Forms.TextBox
    $UI.txtChanges.Multiline = $true; $UI.txtChanges.ReadOnly = $true; $UI.txtChanges.ScrollBars = 'Vertical'; $UI.txtChanges.Font = $fN
    $UI.txtChanges.Location = New-Object System.Drawing.Point(28, 90); $UI.txtChanges.Size = New-Object System.Drawing.Size(524, 130)
    $pOffer.Controls.Add($UI.txtChanges)
    $UI.lblOfferNote = New-Label '' 28 232 524 44 $fN $cText
    $pOffer.Controls.Add($UI.lblOfferNote)
    $UI.btnUpdNow = New-Button 'Update now' 312 292 140 40 $true
    $pOffer.Controls.Add($UI.btnUpdNow)
    $UI.btnNotNow = New-Button 'Not now' 462 292 90 40 $false
    $pOffer.Controls.Add($UI.btnNotNow)

    foreach ($p in @($pPick, $pInst, $pDone, $pErr, $pOffer)) { $form.Controls.Add($p) }

    # ---------------------------------------------------------------- views
    function Get-Scale { return ($form.CurrentAutoScaleDimensions.Width / 96.0) }
    function Set-FormHeight([int]$h96) { $form.ClientSize = New-Object System.Drawing.Size($form.ClientSize.Width, [int][math]::Round($h96 * (Get-Scale))) }

    function Show-View([string]$name) {
        $S.view = $name
        foreach ($p in @($pPick, $pInst, $pDone, $pErr, $pOffer)) { $p.Visible = $false }
        Set-FormHeight $(if ($name -eq 'offer') { 350 } else { 440 })
        switch ($name) {
            'offer' { $pOffer.Visible = $true; $form.AcceptButton = $UI.btnUpdNow }
            'pick' {
                $pPick.Visible = $true; $form.AcceptButton = $UI.btnInstall
                if ($S.repair) {
                    $UI.lblPickText.Text = 'Repair / rebuild: this makes the game code again from your disc and rebuilds the game (about 15 minutes). Pick the .iso image of your Ratchet & Clank disc (PAL / Europe or USA, v1.00).'
                    $UI.btnInstall.Text = 'Repair'
                } else {
                    $UI.lblPickText.Text = 'Pick the .iso image of your own Ratchet & Clank disc (PAL / Europe or USA, v1.00)'
                    $UI.btnInstall.Text = 'Install'
                }
            }
            'install' { $pInst.Visible = $true; $form.AcceptButton = $null; $UI.lnkDetails.Text = 'Show details'; $UI.txtLog.Visible = $false }
            'done'    { $pDone.Visible = $true; $form.AcceptButton = $UI.btnPlay }
            'installed' { $pDone.Visible = $true; $form.AcceptButton = $UI.btnPlay }
            'error'   { $pErr.Visible = $true; $form.AcceptButton = $UI.btnRetry }
        }
        if ($name -eq 'done' -or $name -eq 'installed') {
            $isDone = ($name -eq 'done')
            $UI.lblDoneTitle.Text = $(if ($isDone) { 'Ready to play' } else { 'Ratchet & Clank is installed' })
            $UI.lblDoneText.Text = $(if ($isDone) { 'Everything is set up. Click Play to start the game.' } else { 'Click Play to start the game.' })
            $UI.lnkRepair.Visible = (-not $isDone)
            $note = ''
            if ($isDone -and $S.shortcut) {
                $desk = [Environment]::GetFolderPath('Desktop')
                if ($S.shortcutPath -and ((Split-Path -Parent $S.shortcutPath) -ne $desk)) { $note = "A shortcut was created here: $($S.shortcutPath)" }
                else { $note = 'The shortcut "Ratchet & Clank (rac-rs)" is on your desktop. It starts this copy of the game (an older shortcut with the same name was replaced).' }
            }
            $iso = Get-InstalledIso
            if ($iso -and -not (Test-Path -LiteralPath $iso -PathType Leaf)) { $note = 'Your disc image was not found where it was during setup. Use Repair / rebuild to pick it again.' }
            $UI.lblDoneNote.Text = $note
            $UI.lblDoneNote.ForeColor = $(if ($note -like 'Your disc*') { $cRed } else { $cGrey })
            $S.finishPending = $false; $UI.btnPlay.Text = 'Play'
            if ($isDone -and $S.update) {
                $UI.lblDoneTitle.Text = 'Updated to v' + $S.newV; $UI.lblDoneText.Text = 'Click Play to start the game.'
                $UI.lblDoneNote.Text = 'Your saves and settings were kept.'; $UI.lblDoneNote.ForeColor = $cGrey
            }
            if (-not $isDone) {
                $bv = Get-BuiltVersion; $lv = Get-LocalVersion
                if ($bv -and ($bv -ne $lv)) {
                    $UI.lblDoneTitle.Text = 'Finish the update'
                    $UI.lblDoneText.Text = "Version $lv is here, but the game is still version $bv. Click Update to finish (about 2 to 10 minutes; your disc image is used as before)."
                    $UI.btnPlay.Text = 'Update'; $S.finishPending = $true; $S.newV = $lv
                }
            }
            $UI.lblVersion.Text = 'Version ' + (Get-LocalVersion)
        }
        $UI.lblVersion.Visible = ($name -eq 'done' -or $name -eq 'installed'); $UI.lnkCheck.Visible = ($name -eq 'installed')
        $form.PerformLayout()
    }

    # ---------------------------------------------------------------- disc check (background runspace: size, then SHA-1 with progress)
    $DiscScript = {
        param($path, $discs, $sync)
        try {
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { $sync.msg = 'That file could not be found.'; $sync.state = 'bad'; return }
            $len = (Get-Item -LiteralPath $path).Length
            $disc = $discs | Where-Object { $_.Size -eq $len } | Select-Object -First 1
            if (-not $disc) {
                $sz = $(if ($len -ge 1GB) { ('{0:N1} GB' -f ($len / 1GB)) } elseif ($len -ge 1MB) { ('{0:N0} MB' -f ($len / 1MB)) } else { ('{0:N0} KB' -f [math]::Ceiling($len / 1KB)) })
                $sync.msg = "This is not the right disc image. It must be Ratchet & Clank PAL / Europe v1.00 or USA v1.00, one .iso file of about 3.9 GB. This file is $sz. Other regions, other versions and Platinum / Greatest Hits re-releases do not work."
                $sync.state = 'bad'; return
            }
            $sync.total = $len
            $fs = [System.IO.File]::Open($path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
            try {
                $h = [System.Security.Cryptography.SHA1]::Create()
                $buf = New-Object byte[] (4MB)
                while ($true) {
                    if ($sync.cancel) { $sync.state = 'cancel'; return }
                    $n = $fs.Read($buf, 0, $buf.Length)
                    if ($n -le 0) { break }
                    $null = $h.TransformBlock($buf, 0, $n, $null, 0)
                    $sync.pos = $sync.pos + $n
                }
                $null = $h.TransformFinalBlock($buf, 0, 0)
                $hex = ([System.BitConverter]::ToString($h.Hash)).Replace('-', '')
            } finally { $fs.Dispose() }
            if ($hex -ne $disc.Sha1) {
                $sync.msg = "This file has the right size but it is not the $($disc.Name) disc (its checksum is different). Try making the .iso again from your disc, or use another copy."
                $sync.state = 'bad'; return
            }
            $sync.name = $disc.Name
            $sync.state = 'ok'
        } catch {
            $sync.msg = 'The file could not be read: ' + $_.Exception.Message
            $sync.state = 'bad'
        }
    }

    function Set-InstallEnabled([bool]$on) {
        $UI.btnInstall.Enabled = $on
        $UI.btnInstall.BackColor = $(if ($on) { $cAccent } else { [System.Drawing.Color]::FromArgb(200, 200, 200) })
        $UI.btnInstall.ForeColor = $(if ($on) { [System.Drawing.Color]::White } else { [System.Drawing.Color]::FromArgb(245, 245, 245) })
    }
    function Set-DiscStatus([string]$kind, [string]$msg) {
        switch ($kind) {
            'none' { $UI.lblDisc.Text = ''; $UI.lblDisc.ForeColor = $cGrey }
            'run'  { $UI.lblDisc.Text = 'Checking the disc image ...'; $UI.lblDisc.ForeColor = $cGrey }
            'ok'   { $UI.lblDisc.Text = "Disc OK: Ratchet & Clank, $(if ($msg) { $msg } else { 'v1.00' })."; $UI.lblDisc.ForeColor = $cGreen }
            'bad'  { $UI.lblDisc.Text = $msg; $UI.lblDisc.ForeColor = $cRed }
        }
        $UI.pbCheck.Visible = ($kind -eq 'run')
    }

    function Stop-DiscCheck {
        if ($S.check) {
            $S.check.Sync.cancel = $true
            try { $null = $S.check.Ps.BeginStop($null, $null) } catch { }
            $S.check = $null
        }
    }

    function Start-DiscCheck([string]$path) {
        Stop-DiscCheck
        $path = $path.Trim().Trim('"')
        $S.discOk = $false; $S.discPath = ''
        Set-InstallEnabled $false
        if (-not $path) { Set-DiscStatus 'none' ''; return }
        $sync = [hashtable]::Synchronized(@{ pos = [long]0; total = [long]0; state = 'run'; msg = ''; name = ''; cancel = $false })
        $ps = [powershell]::Create()
        $null = $ps.AddScript($DiscScript).AddArgument($path).AddArgument($Discs).AddArgument($sync)
        $S.check = [pscustomobject]@{ Ps = $ps; Sync = $sync; Handle = $ps.BeginInvoke(); Path = $path }
        Set-Bar $UI.pbCheck 0
        Set-DiscStatus 'run' ''
    }

    function Tick-Disc {
        $c = $S.check
        if (-not $c) { return }
        $st = $c.Sync.state
        if ($st -eq 'run') {
            if ($c.Sync.total -gt 0) {
                $pct = [int][math]::Min(100, [math]::Floor(100.0 * $c.Sync.pos / $c.Sync.total))
                Set-Bar $UI.pbCheck $pct
                $UI.lblDisc.Text = "Checking the disc image ... $pct%"
            }
            return
        }
        if (-not $c.Handle.IsCompleted) { return }
        $S.check = $null
        try { $c.Ps.Dispose() } catch { }
        if ($st -eq 'ok') {
            $S.discOk = $true; $S.discPath = $c.Path
            Set-DiscStatus 'ok' $c.Sync.name
            Set-InstallEnabled $true
        } elseif ($st -eq 'bad') {
            Set-DiscStatus 'bad' $c.Sync.msg
        }
    }

    function Set-IsoPath([string]$path) {
        $S.suppress = $true
        $UI.txtIso.Text = $path
        $S.suppress = $false
        $UI.debounce.Stop()
        Start-DiscCheck $path
    }

    # ---------------------------------------------------------------- install (the console steps run in a hidden child, output read from files)
    function Reset-InstallState {
        $S.log = New-Object System.Text.StringBuilder
        $S.step = 0; $S.levels = 0; $S.compiled = 0; $S.progress = 0.0; $S.lastError = ''; $S.status = ''
        $S.shortcut = $false; $S.shortcutPath = ''; $S.ranOk = $false; $S.finished = $false
        $S.outPos = [long]0; $S.errPos = [long]0; $S.outRest = ''; $S.errRest = ''; $S.expect = 0; $S.buildT0 = $null
        $UI.txtLog.Clear(); $UI.txtErrLog.Clear()
        Set-Bar $UI.pb 0; $UI.lblPct.Text = '0%'; $UI.lblStatus.Text = ''; $UI.lblElapsed.Text = 'Elapsed: 00:00'
        Set-Step 0
    }

    function Set-Step([int]$idx, [bool]$failed = $false) {
        $S.step = $idx
        for ($i = 0; $i -lt $StepNames.Count; $i++) {
            $g = $UI.stepGlyph[$i]; $t = $UI.stepText[$i]
            if ($i -lt $idx) { $g.Text = [string][char]0x2713; $g.ForeColor = $cGreen; $t.ForeColor = $cText }
            elseif ($i -eq $idx) {
                if ($failed) { $g.Text = [string][char]0x2717; $g.ForeColor = $cRed; $t.ForeColor = $cRed }
                elseif ($S.finished) { $g.Text = [string][char]0x2713; $g.ForeColor = $cGreen; $t.ForeColor = $cText }
                else { $g.Text = [string][char]0x25B6; $g.ForeColor = $cBlue; $t.ForeColor = $cText }
            }
            else { $g.Text = [string][char]0x25CB; $g.ForeColor = $cGrey; $t.ForeColor = $cGrey }
        }
    }

    function Update-Progress {
        $v = 0.0
        switch ($S.step) {
            0 { $v = 1.0 + 7.0 * $S.toolsSub }
            1 { $v = 9.0 }
            2 { $v = 10.0 + 18.0 * [math]::Min(1.0, $S.levels / 19.0); if ($S.vu) { $v = 29.0 } }
            3 {
                $exp = $(if ($S.expect -gt 0) { $S.expect } else { $ExpectedCompiles })
                $done = $(if ($S.update) { [math]::Max(0, $S.compiled - 1) } else { $S.compiled })      # cargo prints "Compiling" when a crate STARTS
                $v = 30.0 + 68.0 * [math]::Min(1.0, $done / [double]$exp)
                if ($S.update -and $S.buildT0) { $v = [math]::Max($v, 30.0 + 60.0 * (1.0 - [math]::Exp(-((Get-Date) - $S.buildT0).TotalSeconds / 150.0))) }
            }
            4 { $v = 99.0 }
        }
        if ($S.finished) { $v = 100.0 }
        if ($v -gt $S.progress) { $S.progress = $v }
        $p = [int][math]::Min(100, [math]::Floor($S.progress))
        Set-Bar $UI.pb $p
        $UI.lblPct.Text = "$p%"
    }

    function Feed-Line([string]$line) {
        [void]$S.log.AppendLine($line)
        $UI.txtLog.AppendText($line + "`r`n")
        $t = $line.Trim()
        if ($t.Length -gt 0 -and $t -notlike '==*') { $S.status = $(if ($t.Length -gt 100) { $t.Substring(0, 100) } else { $t }) }
        if ($line -match '^== \((u|a|b|c|c2|d|e)\)') {
            $map = @{ 'u' = 0; 'a' = $(if ($S.update) { 1 } else { 0 }); 'b' = 1; 'c' = 2; 'c2' = 2; 'd' = 3; 'e' = 4 }
            $k = $Matches[1]
            if ($k -eq 'c2') { $S.vu = $true }
            $S.status = @{ 'u' = 'Downloading and applying the update ...'; 'a' = 'Getting the tools ready ...'; 'b' = 'Checking the disc image ...'; 'c' = 'Making the game code from your disc ...'; 'c2' = 'Making the graphics code from your disc ...'; 'd' = 'Building the game (the long part) ...'; 'e' = 'Finishing ...' }[$k]
            if ($map[$k] -ne $S.step) { Set-Step $map[$k] }
            if ($k -eq 'd' -and -not $S.buildT0) { $S.buildT0 = Get-Date }
        }
        elseif ($line -match '^expect-compiles: (\d+)') { $S.expect = [int]$Matches[1] }
        elseif ($line -match '^== done') { $S.finished = $true; Set-Step 4 }
        elseif ($line -match '^\s*level \d+ done') { $S.levels++ }
        elseif ($line -match '^\s+Compiling ') { $S.compiled++ }
        elseif ($line -match 'downloading Rust') { $S.toolsSub = 0.15 }
        elseif ($line -match 'downloading Python') { $S.toolsSub = 0.6 }
        elseif ($line -match 'downloading the GNU assembler') { $S.toolsSub = 0.8 }
        elseif ($line -match 'desktop shortcut created: (.+)$') { $S.shortcut = $true; $S.shortcutPath = $Matches[1].Trim() }
        if ($line -match '^ERROR: (.*)$') { $S.lastError = $Matches[1] }
        elseif ($line -match '^error(\[E\d+\])?: (.*)$' -and -not $S.lastError) { $S.lastError = $Matches[2] }
    }

    function Read-NewText([string]$which) {
        $path = $S["${which}Path"]
        if (-not $path -or -not (Test-Path -LiteralPath $path)) { return }
        $txt = ''
        $fs = $null
        try {
            $fs = New-Object System.IO.FileStream($path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
            $pos = $S["${which}Pos"]
            if ($fs.Length -le $pos) { return }
            [void]$fs.Seek($pos, [System.IO.SeekOrigin]::Begin)
            $buf = New-Object byte[] ([int]($fs.Length - $pos))
            $r = $fs.Read($buf, 0, $buf.Length)
            $S["${which}Pos"] = $pos + $r
            $txt = [System.Text.Encoding]::UTF8.GetString($buf, 0, $r)
        } finally { if ($fs) { $fs.Dispose() } }
        $parts = ($S["${which}Rest"] + $txt) -split '[\r\n]+'
        $S["${which}Rest"] = $parts[$parts.Count - 1]
        for ($i = 0; $i -lt $parts.Count - 1; $i++) { if ($parts[$i].Length -gt 0) { Feed-Line $parts[$i] } }
    }

    function Set-Mode([bool]$upd) {
        $S.update = $upd
        if (-not $upd) { $S.workerExtra = '' }
        $script:StepNames = $(if ($upd) { $StepNamesUpdate } else { $StepNamesInstall })
        for ($i = 0; $i -lt $StepNames.Count; $i++) { $UI.stepText[$i].Text = $StepNames[$i] }
        $UI.lblInstTitle.Text = $(if ($upd) { 'Updating to v' + $S.newV } else { 'Installing' })
        $UI.lblInstNote.Text = $(if ($upd) { 'This takes about 2 to 10 minutes. You can keep using your PC.' } else { 'This takes about 15 minutes. You can keep using your PC.' })
        $UI.btnErrClose.Text = $(if ($upd) { 'Play old version' } else { 'Close' })
    }

    function Start-UpdateRun {
        # the update runs as a hidden worker (setup.ps1 -Console -ApplyUpdate ...); this window shows its steps
        Set-Mode $true
        $iso = Get-InstalledIso
        $msg = ''
        if (@(Get-GameProcs).Count -gt 0) { $msg = "The game is still running.`r`n`r`nClose it, then click Try again." }
        elseif (-not $iso -or -not (Test-Path -LiteralPath $iso -PathType Leaf)) { $msg = "Your disc image was not found at:`r`n$iso`r`n`r`nPut it back where it was, or open Setup.cmd and use Repair / rebuild to pick it again." }
        if ($msg) {
            Reset-InstallState
            $S.workerCode = 1
            $UI.lblErrMsg.Text = $msg; $UI.txtErrLog.Text = ''
            Show-View 'error'
            return
        }
        $S.workerExtra = '-ApplyUpdate -NewVersion ' + $S.newV + $(if ($S.finish) { ' -FinishOnly' } else { '' }) + ' -SkipUnblock' + $(if ($TestUpdateBase) { ' -TestUpdateBase "' + $TestUpdateBase.TrimEnd('\') + '"' } else { '' })
        $deskDir = $(if ($S.shortcutDir) { $S.shortcutDir } else { [Environment]::GetFolderPath('Desktop') })
        $hasShortcut = Test-Path -LiteralPath (Join-Path $deskDir 'Ratchet & Clank (rac-rs).lnk')
        Start-Install $iso $hasShortcut $false
    }

    function Set-OfferContent {
        $lv = Get-LocalVersion; $nv = $S.newV; $from = $lv
        $local = Join-Path $root 'CHANGELOG.md'
        if ($S.finish) {
            $from = Get-BuiltVersion
            $UI.lblOfferTitle.Text = "Finish the update to v$nv"
            $UI.lblOfferNote.Text = 'An earlier update was not finished. It takes about 2 to 10 minutes. Your saves and settings are kept.'
            $ct = $(if (Test-Path -LiteralPath $local) { [System.IO.File]::ReadAllText($local) } else { Read-Remote 'CHANGELOG.md' })
        } else {
            $bvx = Get-BuiltVersion; if ($bvx -and ((Compare-Version $bvx $lv) -lt 0)) { $from = $bvx }      # an unfinished update: the game you have is the built one
            $UI.lblOfferTitle.Text = "Update available: v$from " + [string][char]0x2192 + " v$nv"
            $UI.lblOfferNote.Text = 'This takes about 2 to 10 minutes. Your saves and settings are kept.'
            $ct = Read-Remote 'CHANGELOG.md'
        }
        $txt = $(if ($ct) { Get-ChangelogFor $ct $from $nv } else { '' })
        if (-not $txt) { $txt = '(The list of changes could not be loaded.)' }
        $UI.txtChanges.Text = $txt
    }

    function Start-Install([string]$iso, [bool]$shortcut, [bool]$regen) {
        Reset-InstallState
        $S.toolsSub = 0.0; $S.vu = $false
        $tmp = Join-Path ([System.IO.Path]::GetTempPath()) 'rac-rs-setup'
        New-Item -ItemType Directory -Force -Path $tmp | Out-Null
        $S.outPath = Join-Path $tmp 'out.log'; $S.errPath = Join-Path $tmp 'err.log'
        Remove-Item -LiteralPath $S.outPath, $S.errPath -Force -ErrorAction SilentlyContinue
        $a = '-NoProfile -ExecutionPolicy Bypass -File "' + (Join-Path $root 'setup.ps1') + '" -Console -Iso "' + $iso + '" -SkipHash'
        if ($S.workerExtra) { $a += ' ' + $S.workerExtra }
        if (-not $shortcut) { $a += ' -NoShortcut' }
        if ($S.shortcutDir) { $a += ' -ShortcutDir "' + $S.shortcutDir + '"' }
        if ($regen) { $a += ' -Regenerate' }
        $S.t0 = Get-Date
        Show-View 'install'
        Set-Step 0
        $S.proc = Start-Process -FilePath $psExe -ArgumentList $a -WorkingDirectory $root -WindowStyle Hidden -PassThru -RedirectStandardOutput $S.outPath -RedirectStandardError $S.errPath
        $null = $S.proc.Handle
    }

    function Stop-Install {
        $p = $S.proc
        if ($p -and -not $p.HasExited) {
            # only the child we started and its descendants
            try { Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\taskkill.exe') -ArgumentList "/PID $($p.Id) /T /F" -WindowStyle Hidden -Wait } catch { }
        }
    }

    function Get-ErrorText {
        if ($S.update -and $S.workerCode -eq 3) { return "The update could not be downloaded.`r`n`r`nPlease check your internet connection and click Try again. Nothing on your PC was changed." }
        $all = $S.log.ToString()
        $name = $StepNames[[math]::Min($S.step, $StepNames.Count - 1)]
        $m = "The step '$name' did not finish."
        $noCompile = (($all -split "`r?`n") | Where-Object { $_ -notmatch '^\s+Compiling ' }) -join "`n"
        $net = ($S.step -eq 0) -or ($noCompile -match '(?i)could not download|failed to download|download of .* failed|couldn.t resolve|could not resolve|resolve host|timed out|unable to connect|spurious network|network failure|failed to get ')
        $disk = ($all -match '(?i)no space left|not enough space|os error 112|there is not enough space')
        if ($S.step -eq 1) { $m = "The disc image was not accepted.`r`n`r`nIt must be Ratchet & Clank PAL / Europe v1.00 or USA v1.00 as one .iso file. Click Try again and pick the right file." }
        elseif ($disk) { $m += "`r`n`r`nThere may not be enough free disk space (about 5 GB are needed). Free some space and click Try again." }
        elseif ($net) { $m += "`r`n`r`nPlease check your internet connection, then click Try again. Setup continues where it stopped." }
        else {
            if ($S.lastError) { $m += "`r`n`r`n" + $S.lastError }
            $m += "`r`n`r`nClick Try again. If it keeps failing, click Copy details and share them when you ask for help."
        }
        if ($S.update) { $m += "`r`n`r`nYour old version is still installed. Click Play old version to keep playing it." }
        return $m
    }

    function Get-DetailsText {
        $lines = $S.log.ToString() -split "`r?`n"
        $tail = $lines | Select-Object -Last 200
        $name = $StepNames[[math]::Min($S.step, $StepNames.Count - 1)]
        return ("Ratchet & Clank (rac-rs) setup - problem report`r`nStep: $name`r`nError: $($S.lastError)`r`nWindows: $([Environment]::OSVersion.VersionString)`r`n`r`nLog (last 200 lines):`r`n" + ($tail -join "`r`n"))
    }

    function Show-Error {
        $UI.lblErrMsg.Text = Get-ErrorText
        $UI.btnCopy.Visible = (-not ($S.update -and $S.workerCode -eq 3)); $UI.btnCopy.Text = 'Copy details'
        $UI.txtErrLog.Text = $S.log.ToString()
        Show-View 'error'
        $UI.txtErrLog.SelectionStart = $UI.txtErrLog.TextLength
        $UI.txtErrLog.ScrollToCaret()
    }

    function Tick-Install {
        $p = $S.proc
        if (-not $p) { return }
        if ($S.t0) { $UI.lblElapsed.Text = 'Elapsed: ' + (Format-Elapsed ((Get-Date) - $S.t0)) }
        Read-NewText 'out'; Read-NewText 'err'
        if ($S.status) { $UI.lblStatus.Text = $S.status }
        Update-Progress
        if ($p.HasExited) {
            Read-NewText 'out'; Read-NewText 'err'
            foreach ($w in @('out', 'err')) { if ($S["${w}Rest"].Length -gt 0) { Feed-Line $S["${w}Rest"]; $S["${w}Rest"] = '' } }
            $S.proc = $null
            $code = $p.ExitCode
            $S.workerCode = $code
            if ($code -eq 0) {
                $S.finished = $true; $S.ranOk = $true
                Set-Step 4; Update-Progress
                Show-View 'done'
            } else {
                Set-Step $S.step $true
                Show-Error
            }
        }
    }

    function Tick-All { Tick-Disc; Tick-Install }

    function Switch-Details {
        if ($UI.txtLog.Visible) {
            $UI.txtLog.Visible = $false; $UI.lnkDetails.Text = 'Show details'; Set-FormHeight 440
        } else {
            Set-FormHeight 660; $UI.txtLog.Size = New-Object System.Drawing.Size($UI.txtLog.Width, [int]($form.ClientSize.Height - $UI.txtLog.Top - 18 * (Get-Scale)))
            $UI.txtLog.Visible = $true; $UI.lnkDetails.Text = 'Hide details'
            $UI.txtLog.SelectionStart = $UI.txtLog.TextLength; $UI.txtLog.ScrollToCaret()
        }
    }

    # ---------------------------------------------------------------- events
    $UI.debounce = New-Object System.Windows.Forms.Timer
    $UI.debounce.Interval = 700
    $UI.debounce.Add_Tick({ $UI.debounce.Stop(); Start-DiscCheck $UI.txtIso.Text })
    $UI.txtIso.Add_TextChanged({
        if ($S.suppress) { return }
        Stop-DiscCheck
        $S.discOk = $false; Set-InstallEnabled $false; Set-DiscStatus 'none' ''
        $UI.debounce.Stop(); $UI.debounce.Start()
    })
    $UI.btnBrowse.Add_Click({
        $dlg = New-Object System.Windows.Forms.OpenFileDialog
        $dlg.Title = 'Pick your Ratchet & Clank (Europe or USA, v1.00) disc image'
        $dlg.Filter = 'Disc image (*.iso)|*.iso|All files (*.*)|*.*'
        if ($UI.txtIso.Text -and (Test-Path -LiteralPath (Split-Path -Parent $UI.txtIso.Text) -ErrorAction SilentlyContinue)) { $dlg.InitialDirectory = Split-Path -Parent $UI.txtIso.Text }
        if ($dlg.ShowDialog($form) -eq [System.Windows.Forms.DialogResult]::OK) { Set-IsoPath $dlg.FileName }
    })
    $UI.btnInstall.Add_Click({
        if (-not $S.discOk) { return }
        Set-Mode $false
        Start-Install $S.discPath $UI.chkShortcut.Checked $S.repair
    })
    $UI.lnkDetails.Add_LinkClicked({ Switch-Details })
    $UI.btnPlay.Add_Click({
        if ($S.finishPending) { $S.finish = $true; Start-UpdateRun; return }
        try { Start-Game $S.update; $form.Close() }
        catch { [void][System.Windows.Forms.MessageBox]::Show($form, "The game could not be started.`r`n`r`n" + $_.Exception.Message, $AppTitle, 'OK', 'Warning') }
    })
    $UI.btnFolder.Add_Click({ try { Start-Process -FilePath 'explorer.exe' -ArgumentList ('"' + $root + '"') } catch { } })
    $UI.lnkRepair.Add_LinkClicked({
        $S.repair = $true
        Show-View 'pick'
        $iso = Get-InstalledIso
        if ($iso -and (Test-Path -LiteralPath $iso -PathType Leaf)) { Set-IsoPath $iso } else { Set-IsoPath '' }
    })
    $UI.btnCopy.Add_Click({
        try { [System.Windows.Forms.Clipboard]::SetText((Get-DetailsText)); $UI.btnCopy.Text = 'Copied' } catch { $UI.btnCopy.Text = 'Copy failed' }
    })
    $UI.btnRetry.Add_Click({
        if ($S.update) { Start-UpdateRun; return }
        Show-View 'pick'
        if ($S.discOk) { Set-InstallEnabled $true }
    })
    $UI.btnErrClose.Add_Click({
        if ($S.update) { try { Start-Game $true } catch { } }
        $form.Close()
    })
    $UI.btnUpdNow.Add_Click({ try { Start-UpdateProcess $S.newV $S.finish; $S.exitCode = 10 } catch { $S.exitCode = 0 }; $form.Close() })      # could not start the update window: play instead
    $UI.btnNotNow.Add_Click({ $S.exitCode = 0; $form.Close() })
    $UI.lnkCheck.Add_LinkClicked({
        $UI.lnkCheck.Text = 'Checking ...'; $form.Refresh()
        $lv = Get-LocalVersion; $rv = Read-Remote 'VERSION'
        $UI.lnkCheck.Text = 'Check for updates'
        if (-not $rv -or ($rv.Trim() -notmatch '^\d+\.\d+\.\d+$')) { [void][System.Windows.Forms.MessageBox]::Show($form, 'Could not check for updates. Are you online?', $AppTitle, 'OK', 'Information'); return }
        $rv = $rv.Trim()
        if ((Compare-Version $rv $lv) -le 0) { [void][System.Windows.Forms.MessageBox]::Show($form, "You have the latest version (v$lv).", $AppTitle, 'OK', 'Information'); return }
        $r = [System.Windows.Forms.MessageBox]::Show($form, "Version $rv is available (you have $lv).`r`n`r`nUpdate now? It takes about 2 to 10 minutes. Your saves and settings are kept.", $AppTitle, 'YesNo', 'Question')
        if ($r -eq [System.Windows.Forms.DialogResult]::Yes) { $S.newV = $rv; $S.finish = $false; Start-UpdateRun }
    })
    $form.Add_FormClosing({
        param($sender, $e)
        if ($S.proc -and -not $S.proc.HasExited) {
            if ($S.test) { Stop-Install }
            else {
                $r = [System.Windows.Forms.MessageBox]::Show($form, "Setup is still running.`r`n`r`nStop it and close this window? You can start it again later and it will continue.", $AppTitle, 'YesNo', 'Question')
                if ($r -ne [System.Windows.Forms.DialogResult]::Yes) { $e.Cancel = $true; return }
                Stop-Install
            }
        }
        Stop-DiscCheck
    })
    $UI.timer = New-Object System.Windows.Forms.Timer
    $UI.timer.Interval = 400
    $UI.timer.Add_Tick({ try { Tick-All } catch { } })
    $UI.timer.Start()

    Set-InstallEnabled $false
    # first view: -Launch = the update offer, -Update = the update window, otherwise installed -> Play, or pick the disc
    if ($Launch) { $S.newV = $launchNew; $S.finish = ($launchMode -eq 'finish'); Set-OfferContent; Show-View 'offer' }
    elseif ($Update) {
        $S.newV = $NewVersion; $S.finish = [bool]$FinishOnly
        if (-not $TestNoShow) { Wait-GameClosed; Start-UpdateRun }
    }
    elseif (Test-Installed) { Show-View 'installed' } else { Show-View 'pick' }

    # ---------------------------------------------------------------- hidden test mode: nothing is shown, the views are saved as PNG
    function Wait-Until([scriptblock]$cond, [int]$sec) {
        $end = (Get-Date).AddSeconds($sec)
        while (-not (& $cond)) {
            Tick-All
            if ((Get-Date) -gt $end) { return $false }
            Start-Sleep -Milliseconds 150
        }
        Tick-All
        return $true
    }
    function Save-Shot([string]$name) {
        $form.PerformLayout()
        $bmp = New-Object System.Drawing.Bitmap($form.Width, $form.Height)
        try {
            $form.DrawToBitmap($bmp, (New-Object System.Drawing.Rectangle(0, 0, $form.Width, $form.Height)))
            $f = Join-Path $TestShot "$name.png"
            $bmp.Save($f, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $bmp.Dispose() }
        Write-Host "  shot $name.png"
    }
    function Test-Check([string]$what, [bool]$ok) {
        $script:TestFails += $(if ($ok) { 0 } else { 1 })
        Write-Host ('  {0}: {1}' -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $what)
    }

    if ($TestNoShow) {
        $script:TestFails = 0
        if (-not $TestShot) { $TestShot = Join-Path $root 'test_shots' }
        New-Item -ItemType Directory -Force -Path $TestShot | Out-Null
        $null = $form.Handle
        # a window that is never shown has no child windows yet: create them (without showing anything) so they can be drawn
        try { [void]$form.GetType().GetMethod('CreateControl', [Reflection.BindingFlags]'NonPublic,Instance', $null, [Type[]]@([bool]), $null).Invoke($form, @($true)) } catch { }
        $S.shortcutDir = Join-Path $TestShot 'desk'
        New-Item -ItemType Directory -Force -Path $S.shortcutDir | Out-Null
        if ($Launch) {
            # -Launch test: the offer window is built but never shown; the choice comes from -TestChoice
            Write-Host ('  title: ' + $UI.lblOfferTitle.Text); Write-Host ('  changes: ' + ($UI.txtChanges.Text -replace "`r`n", ' | '))
            Save-Shot $(if ($S.finish) { 'launch_finish' } else { 'launch_offer' })
            $code = $(if ($TestChoice -eq 'update') { 10 } else { 0 })
            if ($code -eq 10) { Start-UpdateProcess $S.newV $S.finish }
            Write-Host "  choice '$TestChoice' -> exit $code"
            exit $code
        }
        if ($Update) {
            # -Update test: the whole update runs hidden; the window is built but never shown
            Wait-GameClosed
            Save-Shot 'upd_00_start'
            Start-UpdateRun
            [void](Wait-Until { $S.step -ge 3 -or -not $S.proc } 1800)
            if ($S.proc) { Save-Shot 'upd_01_building' }
            [void](Wait-Until { -not $S.proc } 3600)
            Save-Shot $(if ($S.view -eq 'done') { 'upd_02_done' } else { 'upd_02_error' })
            Write-Host ("  update view: {0}, worker exit code {1}" -f $S.view, $S.workerCode)
            if ($S.view -ne 'done') { Write-Host ('  error text: ' + ($UI.lblErrMsg.Text -replace "`r`n", ' | ')) }
            Set-Content -LiteralPath (Join-Path $TestShot 'upd_log.txt') -Value $S.log.ToString() -Encoding UTF8
            Stop-Install
            exit $(if ($S.view -eq 'done') { 0 } else { 1 })
        }
        if ($TestChoice -eq 'firstview') {
            # test only: just the first view of the launcher (the installed view says "Finish the update" after a ZIP was unzipped over an older folder)
            Save-Shot 'first_view'
            Write-Host ('  view: ' + $S.view + '; title: ' + $UI.lblDoneTitle.Text + '; text: ' + $UI.lblDoneText.Text + '; button: ' + $UI.btnPlay.Text)
            exit 0
        }
        Write-Host "launcher test (nothing is shown); shots in $TestShot"

        Test-Check 'first view matches the installed check' (($S.view -eq 'installed') -eq (Test-Installed))
        Save-Shot '01_first_view'
        # play.cfg and the game start (the game is NOT launched)
        if ($TestIso) {
            Write-PlayCfg $TestIso
            $b = [System.IO.File]::ReadAllBytes((Join-Path $root 'generated\play.cfg'))
            Test-Check 'play.cfg is UTF-8 without BOM, iso= line' (($b[0] -ne 0xEF) -and ([System.Text.Encoding]::UTF8.GetString($b) -eq ("iso=$TestIso`n")))
            Test-Check 'installed check passes once play.cfg exists' (Test-Installed)
            $psi = Get-GamePsi
            Write-Host ('  game: {0} [args: "{1}"] cwd={2}' -f $psi.FileName, $psi.Arguments, $psi.WorkingDirectory)
            Test-Check 'game start: exe, no arguments, no console, working dir = root' (($psi.FileName -like '*\build\release\rac-rs.exe') -and ($psi.Arguments -eq '') -and $psi.CreateNoWindow -and -not $psi.UseShellExecute -and ($psi.WorkingDirectory -eq $root))
            Test-Check 'iso read back from play.cfg' ((Get-InstalledIso) -eq $TestIso)
        }
        # a stale shortcut with our name that points to another exe (like an old dev shortcut) must be replaced
        $lnk = Join-Path $S.shortcutDir 'Ratchet & Clank (rac-rs).lnk'
        $old = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
        $old.TargetPath = Join-Path $env:SystemRoot 'System32\notepad.exe'; $old.Arguments = '--old-dev-arg'; $old.WorkingDirectory = $env:SystemRoot; $old.Save()
        Test-Check 'stale shortcut prepared (points elsewhere, has arguments)' (((New-Object -ComObject WScript.Shell).CreateShortcut($lnk)).Arguments -eq '--old-dev-arg')
        $lnk = New-GameShortcut $S.shortcutDir
        $sc = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
        Write-Host ('  shortcut: {0} [args: "{1}"] cwd={2} icon={3}' -f $sc.TargetPath, $sc.Arguments, $sc.WorkingDirectory, $sc.IconLocation)
        $icoShipped = (Get-IconFile).Length -gt 0
        $icoUsed = $sc.IconLocation -like '*rac-rs.ico*'
        Write-Host ("  icon shipped={0} used={1}; target ok={2} args empty={3}" -f $icoShipped, $icoUsed, ($sc.TargetPath -like '*\build\release\rac-rs.exe'), ($sc.Arguments -eq ''))
        Test-Check 'shortcut: the exe, no arguments, icon is the .ico when shipped' (($sc.TargetPath -like '*\build\release\rac-rs.exe') -and ($sc.Arguments -eq '') -and ($icoShipped -eq $icoUsed))
        Test-Check 'shortcut written to the test folder (not the desktop)' (Test-Path -LiteralPath $lnk)

        # picker: empty, then a dummy small file, then the real disc
        $S.repair = $false
        Show-View 'pick'; Set-IsoPath ''
        Save-Shot '02_pick_empty'
        $dummy = Join-Path $TestShot 'dummy.iso'
        [System.IO.File]::WriteAllBytes($dummy, (New-Object byte[] 4096))
        Set-IsoPath $dummy
        [void](Wait-Until { -not $S.check } 60)
        Test-Check 'dummy file: red error, Install stays disabled' ((-not $UI.btnInstall.Enabled) -and ($UI.lblDisc.ForeColor -eq $cRed))
        Write-Host ('  message: ' + $UI.lblDisc.Text)
        Save-Shot '03_pick_bad'
        Set-IsoPath (Join-Path $TestShot 'does_not_exist.iso')
        [void](Wait-Until { -not $S.check } 60)
        Test-Check 'missing file: red error' ((-not $UI.btnInstall.Enabled) -and ($UI.lblDisc.ForeColor -eq $cRed))
        if ($TestIso) {
            Set-IsoPath $TestIso
            [void](Wait-Until { $S.check -and $S.check.Sync.pos -gt 300MB } 120)
            Save-Shot '04_pick_checking'
            [void](Wait-Until { -not $S.check } 300)
            Test-Check 'real disc: Disc OK and Install enabled' ($UI.btnInstall.Enabled -and ($UI.lblDisc.ForeColor -eq $cGreen))
            Save-Shot '05_pick_ok'
        }

        # installing view with a simulated log (the bar, the ticks and the details box)
        Reset-InstallState; $S.toolsSub = 0.0; $S.vu = $false; $S.t0 = (Get-Date).AddMinutes(-7).AddSeconds(-12)
        Show-View 'install'
        foreach ($l in @('== (a) tools', '== (b) disc image', '== (c) generating game code', '== (c2) VU', '== (d) building the game')) { Feed-Line $l }
        foreach ($i in 1..237) { Feed-Line ('   Compiling crate-{0} v0.1.0' -f $i) }
        Feed-Line '   Compiling rec-l4-03 v0.1.0 (...)'
        $UI.lblElapsed.Text = 'Elapsed: ' + (Format-Elapsed ((Get-Date) - $S.t0)); $UI.lblStatus.Text = $S.status; Update-Progress
        Test-Check 'simulated build: step 4 active, bar between 30 and 98' (($S.step -eq 3) -and ($UI.pb.Tag -gt 30) -and ($UI.pb.Tag -lt 98))
        Save-Shot '06_installing'
        Switch-Details
        Save-Shot '07_installing_details'
        foreach ($i in 238..2000) { Feed-Line ('   Compiling crate-{0} v0.1.0' -f $i) }
        Update-Progress
        Test-Check 'never above 98 percent before the end' ($UI.pb.Tag -le 98)

        # a real run in this (already built) folder: a dummy file as disc -> the error view; the real disc -> the done view
        Reset-InstallState
        Start-Install $dummy $false $false
        [void](Wait-Until { -not $S.proc } 600)
        Test-Check 'wrong disc through the installer: error view' ($S.view -eq 'error')
        Write-Host ('  error text: ' + ($UI.lblErrMsg.Text -replace "`r`n", ' | '))
        Save-Shot '08_error'
        $d = Get-DetailsText
        Set-Content -LiteralPath (Join-Path $TestShot 'copy_details.txt') -Value $d -Encoding UTF8
        Test-Check 'copy details text built' ($d.Length -gt 50)
        if ($TestIso -and $TestInstall) {
            Start-Install $TestIso $false $false
            [void](Wait-Until { $S.step -ge 3 } 600)
            Save-Shot '09_installing_real'
            [void](Wait-Until { -not $S.proc } 1500)
            Test-Check 'real run (everything already built): done view' ($S.view -eq 'done')
            Save-Shot '10_done'
        }
        # done view as after a successful run (simulated log lines, as the worker prints them)
        Reset-InstallState
        foreach ($l in @('== (e) play.cfg and Play.cmd', '== done', ('  desktop shortcut created: ' + $lnk))) { Feed-Line $l }
        $S.finished = $true; Set-Step 4; Update-Progress
        Show-View 'done'
        Test-Check 'done view names the shortcut' ($UI.lblDoneNote.Text -like '*shortcut*')
        Write-Host ('  done note: ' + $UI.lblDoneNote.Text)
        Save-Shot '10_done'
        Show-View 'installed'
        Save-Shot '11_installed_again'
        Stop-DiscCheck
        Stop-Install
        Write-Host ("launcher test finished, failures: {0}" -f $script:TestFails)
        if ($Mutex) { $Mutex.ReleaseMutex() }
        exit $script:TestFails
    }

    try {
        [System.Windows.Forms.Application]::Run($form)
    } catch {
        try { [void][System.Windows.Forms.MessageBox]::Show("The launcher stopped unexpectedly:`r`n`r`n" + $_.Exception.Message + "`r`n`r`nYou can use Setup.cmd -Console instead.", $AppTitle, 'OK', 'Error') } catch { }
        Stop-Install
        exit 1
    }
    if ($Mutex) { try { $Mutex.ReleaseMutex() } catch { } }
    exit $S.exitCode
}


# Files from a downloaded ZIP carry the 'from the internet' mark; clear it so the scripts and build run without prompts.
if ($ApplyUpdate) {
    # Worker of the update window. Downloads the new version, copies it over this install (Sync-Update), then runs the normal steps below with
    # the NEW setup.ps1 as a fresh process (the tools, stamps and build of the new version decide what is redone).
    Step "(u) updating to v$NewVersion"
    if (-not $FinishOnly) {
        $utmp = Join-Path ([System.IO.Path]::GetTempPath()) "rac-rs-update\$NewVersion"
        try {
            if (Test-Path -LiteralPath $utmp) { Remove-Item -LiteralPath $utmp -Recurse -Force }
            New-Item -ItemType Directory -Force -Path $utmp | Out-Null
            Write-Host "  downloading v$NewVersion ..."
            Get-UpdateZip $NewVersion (Join-Path $utmp 'update.zip')
            Write-Host '  unpacking ...'
            Expand-UpdateZip (Join-Path $utmp 'update.zip') (Join-Path $utmp 'x')
        } catch {
            Write-Host "ERROR: could not download the update: $($_.Exception.Message)" -ForegroundColor Red
            Remove-Item -LiteralPath $utmp -Recurse -Force -ErrorAction SilentlyContinue
            exit 3
        }
        try {
            $got = ([System.IO.File]::ReadAllText((Join-Path $utmp 'x\VERSION'))).Trim()
            if ($got -notmatch '^\d+\.\d+\.\d+$' -or (Compare-Version $got (Get-LocalVersion)) -le 0) { throw "the downloaded version ($got) is not newer than this one ($(Get-LocalVersion))" }
            Write-Host "  applying v$got ..."
            $res = Sync-Update (Join-Path $utmp 'x') $root
        } catch {
            Write-Host "ERROR: could not apply the update: $($_.Exception.Message)" -ForegroundColor Red
            Remove-Item -LiteralPath $utmp -Recurse -Force -ErrorAction SilentlyContinue
            exit 1
        }
        Remove-Item -LiteralPath $utmp -Recurse -Force -ErrorAction SilentlyContinue
        $ch = @($res.Changed)
        $exp = 1
        if (@($ch | Where-Object { $_ -like 'oracle/src/*' -or $_ -eq 'oracle/Cargo.toml' }).Count -gt 0) { $exp = 528 }
        elseif (@($ch | Where-Object { $_ -like 'oracle/hand/*' }).Count -gt 0) { $exp = 3 }
        Write-Host "  $($ch.Count) files updated, $(@($res.Removed).Count) removed"
        Write-Host "expect-compiles: $exp"
    } else {
        Write-Host '  the files are already updated; only the rebuild is left'
        Write-Host 'expect-compiles: 528'
    }
    # Files that were not copied by Sync-Update (no generated\synced_<version>: a ZIP unzipped over the folder) carry the ZIP's times: stamp them, or cargo may keep the old game.
    $syncMark = Join-Path $root ('generated\synced_' + (Get-LocalVersion))
    if (-not (Test-Path -LiteralPath $syncMark -PathType Leaf)) {
        $nst = Set-ReleaseFileTimes
        Write-Host "  the files were put in place by hand: set the time of $nst files to now, so that everything is rebuilt once"
        New-Item -ItemType Directory -Force -Path (Join-Path $root 'generated') | Out-Null
        [System.IO.File]::WriteAllText($syncMark, ((Get-LocalVersion) + "`n"), (New-Object System.Text.UTF8Encoding($false)))
    }
    $a2 = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $root 'setup.ps1'), '-Console', '-Iso', $Iso, '-SkipHash', '-SkipUnblock')
    if ($NoShortcut) { $a2 += '-NoShortcut' }
    if ($ShortcutDir) { $a2 += @('-ShortcutDir', $ShortcutDir) }
    & (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') @a2
    exit $LASTEXITCODE
}

if (-not $SkipUnblock) { try { Get-ChildItem -LiteralPath $root -Recurse -File -ErrorAction SilentlyContinue | Unblock-File -ErrorAction SilentlyContinue } catch { } }

# ---------------------------------------------------------------- (a) tools
# A private, pinned copy of Rust, Python and the GNU assembler is downloaded INTO this folder (.\toolchain, about 1.2 GB on disk): nothing is installed
# on the PC, no admin rights, no PATH or registry change. Rust uses the GNU host (x86_64-pc-windows-gnu): its linker ships with
# Rust itself, so the Microsoft C++ build tools are not needed. Python is the official embeddable package from python.org.
# Rust or Python already installed on the PC are not used and not touched. Running setup again reuses .\toolchain (no new download).
Step '(a) tools'
$RustVersion = '1.99.0'
$RustHost = 'x86_64-pc-windows-gnu'
$RustupVersion = '1.29.1'
$RustupUrl = "https://static.rust-lang.org/rustup/archive/$RustupVersion/$RustHost/rustup-init.exe"
$RustupSha256 = '6D5B5709ADDC0122C916D8C810DA8D8A7B086A5D64FA805EF404D506392AADC8'
$PyVersion = '3.14.8'
$PyUrl = "https://www.python.org/ftp/python/$PyVersion/python-$PyVersion-embed-amd64.zip"
$PySha256 = 'A93ABE456AB01BD96D7A085B3CDB6566B3063F4241360D114142FBDB07F0A310'
$tc = Join-Path $root 'toolchain'
$tcDl = Join-Path $tc 'downloads'
$t = Get-Date

# Only this folder's toolchain is used: its own rustup/cargo homes, the pinned Rust version, and no build settings from the PC
# (they could change the build or its speed).
$env:RUSTUP_HOME = Join-Path $tc 'rustup'
$env:CARGO_HOME = Join-Path $tc 'cargo'
$env:RUSTUP_TOOLCHAIN = "$RustVersion-$RustHost"
$env:Path = (Join-Path $tc 'cargo\bin') + ';' + $env:Path
foreach ($v in @('RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET', 'CARGO_BUILD_RUSTFLAGS',
                  'CARGO_BUILD_RUSTC_WRAPPER', 'CARGO_TARGET_DIR', 'CARGO_PROFILE_RELEASE_OPT_LEVEL', 'CARGO_PROFILE_RELEASE_LTO', 'RUSTUP_DIST_SERVER')) {
    Remove-Item -LiteralPath "Env:$v" -ErrorAction SilentlyContinue
}

function Get-Pinned($url, $file, $sha) {
    # Downloads $url to $file and checks its SHA-256; up to 3 tries.
    if ((Test-Path -LiteralPath $file) -and (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -eq $sha) { return }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $file) | Out-Null
    try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }
    $part = "$file.part"
    for ($try = 1; $try -le 3; $try++) {
        try {
            $old = $ProgressPreference; $ProgressPreference = 'SilentlyContinue'     # the progress bar makes Invoke-WebRequest very slow
            try { Invoke-WebRequest -Uri $url -OutFile $part -UseBasicParsing } finally { $ProgressPreference = $old }
            $h = (Get-FileHash -LiteralPath $part -Algorithm SHA256).Hash
            if ($h -eq $sha) { Move-Item -LiteralPath $part -Destination $file -Force; return }
            Write-Host "  download of $url has the wrong SHA-256 ($h), retrying" -ForegroundColor Yellow
        } catch {
            Write-Host "  download of $url failed ($($_.Exception.Message)), retrying" -ForegroundColor Yellow
        }
        Remove-Item -LiteralPath $part -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds (5 * $try)
    }
    Fail "could not download $url (check the internet connection, then run Setup.cmd again)"
}

# Rust
$cargo = Join-Path $tc 'cargo\bin\cargo.exe'
$rv = ''
if (Test-Path -LiteralPath $cargo) { try { $rv = (& (Join-Path $tc 'cargo\bin\rustc.exe') --version 2>$null) } catch { $rv = '' } }
if ($rv -notmatch "^rustc $([regex]::Escape($RustVersion)) ") {
    Write-Host "  downloading Rust $RustVersion ($RustHost, about 150 MB) into toolchain\ ..."
    $ri = Join-Path $tcDl "rustup-init-$RustupVersion.exe"
    Get-Pinned $RustupUrl $ri $RustupSha256
    # rustup checks every Rust package it downloads against the signed release manifest of $RustVersion.
    & $ri -y --no-modify-path --profile minimal --default-host $RustHost --default-toolchain $RustVersion
    if ($LASTEXITCODE -ne 0) { Fail 'installing Rust into toolchain\ failed (see the messages above); run Setup.cmd again' }
    $rv = ''
    try { $rv = (& (Join-Path $tc 'cargo\bin\rustc.exe') --version 2>$null) } catch { $rv = '' }
    if ($rv -notmatch "^rustc $([regex]::Escape($RustVersion)) ") { Fail "Rust in toolchain\ does not run (rustc --version: '$rv')" }
}
Write-Host "  $rv ($RustHost, toolchain\)"
# The GNU host's own linker tools (ld, dlltool for the Windows API import libraries) come with Rust; rustc finds dlltool on PATH
# (the GNU assembler that dlltool runs is added to the same folder below, after Python).
$rsc = Join-Path $tc "rustup\toolchains\$RustVersion-$RustHost\lib\rustlib\$RustHost\bin\self-contained"
if (-not (Test-Path -LiteralPath (Join-Path $rsc 'dlltool.exe'))) { Fail "Rust in toolchain\ is incomplete ($rsc\dlltool.exe missing); delete the toolchain folder and run Setup.cmd again" }
$env:Path = "$rsc;$env:Path"

# Python (embeddable package: python.exe + the standard library, nothing else; the generators need nothing more)
$python = Join-Path $tc 'python\python.exe'
$pv = ''
if (Test-Path -LiteralPath $python) { try { $pv = (& $python -c "import sys; print('%d.%d.%d' % sys.version_info[:3])" 2>$null) } catch { $pv = '' } }
if ($pv -ne $PyVersion) {
    Write-Host "  downloading Python $PyVersion (embeddable, about 12 MB) into toolchain\ ..."
    $pz = Join-Path $tcDl "python-$PyVersion-embed-amd64.zip"
    Get-Pinned $PyUrl $pz $PySha256
    $pd = Join-Path $tc 'python'
    if (Test-Path -LiteralPath $pd) { Remove-Item -LiteralPath $pd -Recurse -Force }
    Expand-Archive -LiteralPath $pz -DestinationPath $pd -Force
    $pv = ''
    try { $pv = (& $python -c "import sys; print('%d.%d.%d' % sys.version_info[:3])" 2>$null) } catch { $pv = '' }
    if ($pv -ne $PyVersion) { Fail "Python in toolchain\ does not run (version: '$pv')" }
}
Write-Host "  python $pv (toolchain\python)"

# GNU assembler. rustc makes the Windows API import libraries of some crates (windows-link and its users) with dlltool.exe,
# which comes with Rust, and dlltool runs the GNU assembler (as.exe), which does not. as.exe and the four DLLs it needs are
# taken from pinned MSYS2 packages (about 9 MB, SHA-256 checked) by tools\mingw_as.py into Rust's linker folder next to dlltool.
$MsysRepo = 'https://repo.msys2.org/mingw/mingw64'
$AsPkgs = [ordered]@{
    'mingw-w64-x86_64-binutils-2.47-3-any.pkg.tar.zst'        = '827363748CE3320683319D860EE4FCFCDBB36BAF66AAC6DAB0EC3822BE2D4FF7'
    'mingw-w64-x86_64-gettext-runtime-1.0-1-any.pkg.tar.zst'  = 'BE68D7F260633284B910C588C6D82EE304A81C8817A686D2CD9DF83F872C27AF'
    'mingw-w64-x86_64-libiconv-1.19-1-any.pkg.tar.zst'        = '21E334D0911F25DE75D3E18E0697648BCECFA9658256D600CAD0827D719C2F35'
    'mingw-w64-x86_64-zlib-1.3.2-2-any.pkg.tar.zst'           = '9E75842A070BA648E986E12424E1C92C9D7D77200E85F6A34EEB600819F2E694'
    'mingw-w64-x86_64-zstd-1.5.7-2-any.pkg.tar.zst'           = '1ADD6705B344664F6ACA108C85F79AB5BDD9E1162662BB06A4CF40A34F6E0907'
}
$asFiles = @('as.exe', 'libintl-8.dll', 'libiconv-2.dll', 'zlib1.dll', 'libzstd.dll')
if (@($asFiles | Where-Object { -not (Test-Path -LiteralPath (Join-Path $rsc $_)) }).Count -gt 0) {
    Write-Host '  downloading the GNU assembler (MSYS2 binutils 2.47, about 9 MB) into toolchain\ ...'
    $pk = @()
    foreach ($e in $AsPkgs.GetEnumerator()) {
        $f = Join-Path $tcDl $e.Key
        Get-Pinned "$MsysRepo/$($e.Key)" $f $e.Value
        $pk += $f
    }
    & $python -I (Join-Path $root 'tools\mingw_as.py') $rsc @pk
    if ($LASTEXITCODE -ne 0) { Fail 'unpacking the GNU assembler into toolchain\ failed (see the messages above); run Setup.cmd again' }
}
Write-Host '  GNU assembler (binutils 2.47, toolchain\)'
if (Test-Path -LiteralPath $tcDl) { Remove-Item -LiteralPath $tcDl -Recurse -Force -ErrorAction SilentlyContinue }
$timings['tools'] = [math]::Round(((Get-Date) - $t).TotalSeconds, 1)

# ---------------------------------------------------------------- (b) disc image
Step '(b) disc image'
if (-not $Iso) {
    # Setup.cmd (double-click) passes no -Iso: ask with a file picker, typed path as fallback.
    try {
        Add-Type -AssemblyName System.Windows.Forms
        $dlg = New-Object System.Windows.Forms.OpenFileDialog
        $dlg.Title = 'Pick your Ratchet & Clank (Europe or USA, v1.00) disc image'
        $dlg.Filter = 'Disc image (*.iso)|*.iso|All files (*.*)|*.*'
        if ($dlg.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { $Iso = $dlg.FileName }
    } catch { }
}
if (-not $Iso) { $Iso = Read-Host 'Path to your Ratchet & Clank (Europe or USA, v1.00) .iso file' }
$Iso = $Iso.Trim().Trim('"')
if (-not (Test-Path -LiteralPath $Iso -PathType Leaf)) { Fail "disc image not found: $Iso" }
$Iso = (Resolve-Path -LiteralPath $Iso).ProviderPath
$len = (Get-Item -LiteralPath $Iso).Length
Write-Host "  $Iso ($len bytes)"
$disc = $Discs | Where-Object { $_.Size -eq $len } | Select-Object -First 1
if (-not $disc) {
    Fail "wrong size: expected $($Discs[0].Size) bytes (Ratchet & Clank PAL/EU v1.00, serial SCES-50916) or $($Discs[1].Size) bytes (USA v1.00, serial SCUS-97199), both En/Fr/De/Es/It. Other regions and versions are not supported."
}
$ExpectedSize = $disc.Size
$ExpectedSha1 = $disc.Sha1
$DiscName = $disc.Name
Write-Host "  disc: Ratchet & Clank $DiscName (by size)"
if (-not $SkipHash) {
    Write-Host '  computing SHA-1 (about half a minute) ...'
    $t = Get-Date
    $h = (Get-FileHash -LiteralPath $Iso -Algorithm SHA1).Hash.ToUpper()
    $timings['sha1'] = [math]::Round(((Get-Date) - $t).TotalSeconds, 1)
    if ($h -ne $ExpectedSha1) { Fail "SHA-1 mismatch: $h (expected $ExpectedSha1). This is not the $DiscName disc image (re-dump, or another version)." }
    Write-Host "  SHA-1 ok: Ratchet & Clank $DiscName"
} else {
    Write-Host '  SHA-1 skipped (-SkipHash); size matches'
}
if ($CheckOnly) { Write-Host 'Checks passed.' -ForegroundColor Green; exit 0 }

# ---------------------------------------------------------------- (c) generate
Step '(c) generating game code from the disc image'
$gen = Join-Path $root 'generated'          # logs and stamps
$orc = Join-Path $root 'oracle'             # the generated crates go to oracle\rec and oracle\game (the layout of the dev tree)
$stamp = Join-Path $gen 'done.txt'
$wantBase = "$ExpectedSha1 $ExpectedSize flags=$($GenFlags -join ' ')"
$genFiles = @('tools/recgen_iso.py', 'tools/fn_bounds.tsv', 'oracle/stubs.txt')
if ($ExpectedSha1 -eq 'AEE9275384F739C9B24CD5DE6F7E200292DB7B01') { $genFiles += 'tools/fn_bounds_usa.tsv' }
$genHash = Get-FilesHash $genFiles
# Earlier generator revisions (the first is v0.1.0, whose stamp has no gen= part) that make byte-identical output: an install made by one of them
# is still current, so an update that only changed HOW the files are written does not regenerate. Add the old hash here when a generator
# change does not change its output.
$GenCompatible = @('9a84d825c333842cd58ed6d12d1ed54c7141ffab30276e6add765ef00c859c51', '1c5c1935a7b98a397719acf76287677cdf061816bbd749c6e07e42ce50afa050')
$want = "$wantBase gen=$genHash"
$have = ''
if (Test-Path -LiteralPath $stamp) { $have = (Get-Content -LiteralPath $stamp -Raw).Trim() }
if (-not (Test-Path -LiteralPath (Join-Path $orc 'game\src\lib.rs'))) { $have = '' }
if ($have -and $have -ne $want) {
    $oldGen = ''
    if ($have -eq $wantBase) { $oldGen = $GenCompatible[0] } elseif ($have.StartsWith($wantBase + ' gen=')) { $oldGen = $have.Substring($wantBase.Length + 5) }
    if ($oldGen -and ($GenCompatible -contains $oldGen)) {
        Set-Content -LiteralPath $stamp -Value $want -Encoding ASCII
        $have = $want
        Write-Host '  the generator changed but makes the same files: stamp updated, nothing to redo'
    }
}
if ($Regenerate -or $have -ne $want) {
    # The generators write in place and only touch files whose content differs (cargo rebuilds by file time): nothing is deleted first.
    Remove-Item -LiteralPath $stamp -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath (Join-Path $gen 'logs')) { Remove-Item -LiteralPath (Join-Path $gen 'logs') -Recurse -Force }
    New-Item -ItemType Directory -Force -Path (Join-Path $gen 'logs') | Out-Null
    $t = Get-Date
    $queue = New-Object System.Collections.Queue
    foreach ($l in $Levels) { $queue.Enqueue($l) }
    $running = @()
    $failed = @()
    while ($queue.Count -gt 0 -or $running.Count -gt 0) {
        while ($queue.Count -gt 0 -and $running.Count -lt [math]::Max(1, $Jobs)) {
            $lv = $queue.Dequeue()
            $a = @(('"' + (Join-Path $root 'tools\recgen_iso.py') + '"'), '--iso', ('"' + $Iso + '"'), '--level', "$lv", '--out', ('"' + $orc + '"'), '--oracle', ('"' + $orc + '"')) + $GenFlags
            if ($lv -gt 0) { $a += '--keep-boot' }
            $log = Join-Path $gen "logs\level_$lv.log"
            $p = Start-Process -FilePath $python -ArgumentList $a -WorkingDirectory $root -NoNewWindow -PassThru -RedirectStandardOutput $log -RedirectStandardError "$log.err"
            $null = $p.Handle
            $running += [pscustomobject]@{ Level = $lv; Proc = $p; Log = $log }
            Write-Host "  level $lv started"
        }
        Start-Sleep -Milliseconds 700
        $still = @()
        foreach ($r in $running) {
            if ($r.Proc.HasExited) {
                $r.Proc.WaitForExit()
                if ($r.Proc.ExitCode -ne 0) { $failed += $r.Level; Write-Host "  level $($r.Level) FAILED (see $($r.Log).err)" -ForegroundColor Red }
                else { Write-Host ("  level {0} done: {1}" -f $r.Level, (Get-Content -LiteralPath $r.Log -Tail 1)) }
            } else { $still += $r }
        }
        $running = $still
    }
    if ($failed.Count -gt 0) { Fail "generation failed for level(s) $($failed -join ', '); logs are in generated\logs" }
    # the parallel generators race on game\src\lib.rs: rewrite it once from what is on disk
    & $python (Join-Path $root 'tools\recgen_iso.py') --facade-only --out $orc --oracle $orc
    if ($LASTEXITCODE -ne 0) { Fail 'facade rewrite failed' }
    # every crate must be in the facade (concurrent generators rewrite it)
    $nCrates = (Get-ChildItem -LiteralPath (Join-Path $orc 'rec') -Directory).Count
    $nReg = (Select-String -LiteralPath (Join-Path $orc 'game\src\lib.rs') -Pattern '::FNS\)' ).Count
    if ($nCrates -ne $nReg) { Fail "facade lists $nReg of $nCrates crates; run setup.ps1 -Regenerate -Jobs 1" }
    Set-Content -LiteralPath $stamp -Value $want -Encoding ASCII
    $timings['generate'] = [math]::Round(((Get-Date) - $t).TotalSeconds, 1)
    Write-Host "  generated $nCrates crates in $($timings['generate']) s"
} else {
    Write-Host '  already generated for this disc image (use -Regenerate to redo)'
}

# ---------------------------------------------------------------- (c2) VU microcode translations
# The VU0/VU1 microprograms (graphics/maths code of the game) are cut out of YOUR disc image and translated to Rust here;
# none of that is shipped. Output (written in place, the files the sources already include): oracle\src\vu0_prog0.rs,
# rac-rs\src\gsfull\vu1_prog*.rs and vu1aot_gen.rs.
Step '(c2) generating the VU microcode translations from the disc image'
New-Item -ItemType Directory -Force -Path $gen | Out-Null
$vuStamp = Join-Path $gen 'vu_done.txt'
$vuBase = "$ExpectedSha1 vu1"
$VuCompatible = @('5b49455da24a986799cd727bb9a4563a70609ab5759df4c060f5df0b00bec669', '066ba6b16b9293793872b68e6a66c106bb2e4f4c8a2e184b1a83ce29eedf26d0')      # as $GenCompatible, for tools\vugen_iso.py and tools\vu\ (the first is v0.1.0)
$vuWant = "$vuBase gen=$(Get-FilesHash (Get-VuGenFiles))"
$vuHave = ''
if (Test-Path -LiteralPath $vuStamp) { $vuHave = (Get-Content -LiteralPath $vuStamp -Raw).Trim() }
if (-not (Test-Path -LiteralPath (Join-Path $orc 'src\vu0_prog0.rs')) -or -not (Test-Path -LiteralPath (Join-Path $root 'rac-rs\src\gsfull\vu1aot_gen.rs')) -or -not (Test-Path -LiteralPath (Join-Path $root 'rac-rs\src\particles\orb_tables.rs'))) { $vuHave = '' }
if ($vuHave -and $vuHave -ne $vuWant) {
    $oldGen = ''
    if ($vuHave -eq $vuBase) { $oldGen = $VuCompatible[0] } elseif ($vuHave.StartsWith($vuBase + ' gen=')) { $oldGen = $vuHave.Substring($vuBase.Length + 5) }
    if ($oldGen -and ($VuCompatible -contains $oldGen)) {
        Set-Content -LiteralPath $vuStamp -Value $vuWant -Encoding ASCII
        $vuHave = $vuWant
        Write-Host '  the VU generator changed but makes the same files: stamp updated, nothing to redo'
    }
}
if ($Regenerate -or $vuHave -ne $vuWant) {
    $t = Get-Date
    & $python (Join-Path $root 'tools\vugen_iso.py') --iso $Iso --tree $root
    if ($LASTEXITCODE -ne 0) { Fail 'VU microcode generation failed (tools\vugen_iso.py)' }
    Set-Content -LiteralPath $vuStamp -Value $vuWant -Encoding ASCII
    $timings['vu'] = [math]::Round(((Get-Date) - $t).TotalSeconds, 1)
    Write-Host "  generated in $($timings['vu']) s"
} else {
    Write-Host '  already generated (use -Regenerate to redo)'
}
if ($NoBuild) { Write-Host 'Generation done (-NoBuild).' -ForegroundColor Green; exit 0 }

# ---------------------------------------------------------------- (d) build
Step '(d) building the game (release; this takes a long time and a lot of memory on the first run)'
# The generated crates (oracle\rec\*, oracle\game) are 97% of the compile work: they build at opt-level 1 in one codegen unit
# (about 2x less work than the default opt-level 3; same behaviour, gameplay speed measured equal). Everything else (the
# emulator core, hands, renderer, wgpu) keeps the default release profile. Passed to cargo as a config file so that
# rac-rs\Cargo.toml stays unchanged.
$prof = Join-Path $root 'generated\build_profile.toml'
$pl = @('# written by setup.ps1 (step d): build profile of the generated crates', '[profile.release.package.ee-game]', 'opt-level = 1', 'codegen-units = 1')
foreach ($d in (Get-ChildItem -LiteralPath (Join-Path $orc 'rec') -Directory | Sort-Object Name)) {
    $pl += @('', "[profile.release.package.rec-$($d.Name.Replace('_', '-'))]", 'opt-level = 1', 'codegen-units = 1')
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $prof) | Out-Null
Set-Content -LiteralPath $prof -Value $pl -Encoding ASCII
$t = Get-Date
& $cargo build --release --manifest-path (Join-Path $root 'rac-rs\Cargo.toml') --target-dir (Join-Path $root 'build') --bin rac-rs --features rac-rs/windows_gui --config $prof
if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
$timings['build'] = [math]::Round(((Get-Date) - $t).TotalSeconds, 1)
$exe = Join-Path $root 'build\release\rac-rs.exe'
if (-not (Test-Path -LiteralPath $exe)) { Fail "build finished but $exe is missing" }
Write-Host "  built $exe ($([math]::Round((Get-Item -LiteralPath $exe).Length / 1MB)) MB) in $($timings['build']) s"
[System.IO.File]::WriteAllText((Join-Path $root 'generated\built_version.txt'), ((Get-LocalVersion) + "`n"), (New-Object System.Text.UTF8Encoding($false)))      # the version this build was made from
Get-ChildItem -LiteralPath (Join-Path $root 'generated') -Filter 'synced_*' -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue      # (see Sync-Update / -ApplyUpdate)

# ---------------------------------------------------------------- (e) Play.cmd
Step '(e) play.cfg and Play.cmd'
if ($Iso -cmatch '[^\x00-\x7F]') { Write-Host '  note: the disc image path has non-ASCII characters; if Play.cmd cannot find it, move the .iso to a plain path and run setup again' -ForegroundColor Yellow }
$cmd = @"
@echo off
rem Written by setup.ps1. Starts Ratchet & Clank from your disc image: the game itself starts from the disc (logos, title screen, New Game),
rem there is no savestate. Press Cross (X) during the logo movies to skip them. Saves go to the player folder next to this file.
set "ORACLE_ISO=$Iso"
set "ORACLE_STUBS=%~dp0oracle\stubs.txt"
set /p ORACLE_HAND_SKIP=<"%~dp0oracle\hand\hand_skip.txt"
set "RAC_PLAYER_DIR=%~dp0player"
if not exist "%RAC_PLAYER_DIR%" mkdir "%RAC_PLAYER_DIR%"
rem Language: en fr de es it  (set it before the next line to change the game language, e.g.  set COLD_LANG=de)
if not defined COLD_LANG set COLD_LANG=en
rem Safety copies of your save card before each start (the newest two starts)
if exist "%RAC_PLAYER_DIR%\card.start1.bin" copy /y "%RAC_PLAYER_DIR%\card.start1.bin" "%RAC_PLAYER_DIR%\card.start2.bin" >nul
if exist "%RAC_PLAYER_DIR%\card.bin" copy /y "%RAC_PLAYER_DIR%\card.bin" "%RAC_PLAYER_DIR%\card.start1.bin" >nul
"%~dp0build\release\rac-rs.exe" view "%ORACLE_ISO%" 0 --retail-cold --modern-cam %*
if errorlevel 1 pause
"@
Set-Content -LiteralPath (Join-Path $root 'Play.cmd') -Value $cmd -Encoding ASCII
Write-Host '  wrote Play.cmd'
Write-PlayCfg $Iso
Write-Host '  wrote generated\play.cfg'

Step 'done'
$timings.GetEnumerator() | ForEach-Object { Write-Host ("  {0,-9} {1} s" -f $_.Key, $_.Value) }
try {
    $dir = $ShortcutDir
    if (-not $dir) { $dir = [Environment]::GetFolderPath('Desktop') }
    if (-not $NoShortcut) {
        # always (re)written, so it points to THIS install even when an older shortcut of the same name exists
        $lnkPath = New-GameShortcut $dir
        Write-Host "  desktop shortcut created: $lnkPath"
    }
} catch { Write-Host "  could not create the desktop shortcut: $($_.Exception.Message)" -ForegroundColor Yellow }
Write-Host ''
Write-Host 'All done. To play: double-click "Ratchet & Clank (rac-rs)" on your desktop, or open Setup.cmd and click Play (or Play.cmd in this folder).' -ForegroundColor Green
