<#
.SYNOPSIS
    One-time seeder for ~/.claude/project-index.json.

.DESCRIPTION
    Scans existing Claude Code session transcripts under ~/.claude/projects/
    (and the live registry under ~/.claude/sessions/) to populate the
    project index with projects that already existed before the
    project-tracker skill / hooks were installed.

    Only ADDS projects that are not already present in the index (matched by
    real cwd, case-insensitive). It never overwrites an existing entry, so
    it's safe to re-run at any time and won't fight with track-session.ps1
    or manual edits (e.g. statusOverride).

    Superseded for cross-Profile/cross-tool backfilling by `aam session
    scan`/`aam session adopt` (docs/05-session-memory-bank-module.md §5.7),
    which generalize this same idea beyond the one fixed ~/.claude
    directory this script still only knows about. Kept around (and updated
    to fill deviceId/profileLabel, same as track-session.ps1) for users who
    haven't set up aam Profiles yet and just want their existing Claude
    history seeded.

.NOTES
    Run manually once: powershell -File backfill-index.ps1
#>

$ErrorActionPreference = 'Stop'

$ScriptDir     = Split-Path -Parent $MyInvocation.MyCommand.Path
$IndexPath     = Join-Path $HOME '.claude\project-index.json'
$ProjectsRoot  = Join-Path $HOME '.claude\projects'

# Keep in sync with track-session.ps1's $ExcludePrefixes.
$ExcludePrefixes = @(
    (Join-Path $HOME '.claude')
)

function Get-JsonlCwd([string]$file) {
    try {
        $m = Select-String -LiteralPath $file -Pattern '"cwd"\s*:\s*"((?:[^"\\]|\\.)*)"' | Select-Object -First 1
        if (-not $m) { return $null }
        $raw = $m.Matches[0].Groups[1].Value
        # Reuse JSON string parsing to correctly unescape \\, \uXXXX, etc.
        return ('"' + $raw + '"') | ConvertFrom-Json
    } catch {
        return $null
    }
}

function Get-LatestAiTitle([string]$file) {
    try {
        $matches = Select-String -LiteralPath $file -Pattern '"type"\s*:\s*"ai-title"'
        if (-not $matches) { return $null }
        $obj = $matches[-1].Line | ConvertFrom-Json
        if ($obj.aiTitle) { return [string]$obj.aiTitle }
    } catch { }
    return $null
}

function Get-AamWhoAmI {
    # See track-session.ps1's copy of this function for the rationale --
    # never lets a missing/failing `aam` break the backfill.
    try {
        $json = & aam whoami --tool claude 2>$null
        if (-not $json) { return $null }
        return $json | ConvertFrom-Json
    } catch {
        return $null
    }
}

if (-not (Test-Path -LiteralPath $ProjectsRoot)) {
    Write-Host "No projects directory found at $ProjectsRoot, nothing to backfill."
    exit 0
}

# 1. Group every top-level session transcript by its real (decoded) cwd.
#    (Skip subagent transcripts, which live under <sessionId>\subagents\ -
#    those aren't separate projects.)
$byCwd = @{}
Get-ChildItem -Path $ProjectsRoot -Filter '*.jsonl' -Recurse -File |
    Where-Object { $_.FullName -notmatch '\\subagents\\' } |
    ForEach-Object {
    $file = $_.FullName
    $cwd = Get-JsonlCwd $file
    if (-not $cwd) { return }
    foreach ($prefix in $ExcludePrefixes) {
        if ($cwd.ToLower().StartsWith($prefix.ToLower())) { return }
    }
    $key = $cwd.ToLower()
    if (-not $byCwd.ContainsKey($key)) {
        $byCwd[$key] = [pscustomobject]@{
            cwd       = $cwd
            latestFile = $file
            latestTime = $_.LastWriteTime
            earliestTime = $_.LastWriteTime
        }
    } else {
        $entry = $byCwd[$key]
        if ($_.LastWriteTime -gt $entry.latestTime) {
            $entry.latestFile = $file
            $entry.latestTime = $_.LastWriteTime
        }
        if ($_.LastWriteTime -lt $entry.earliestTime) {
            $entry.earliestTime = $_.LastWriteTime
        }
    }
}

Write-Host "Found $($byCwd.Count) distinct project cwd(s) across historical sessions."

# 2. Load (or init) the index.
if (Test-Path -LiteralPath $IndexPath) {
    $index = Get-Content -Raw -LiteralPath $IndexPath | ConvertFrom-Json
} else {
    $index = [pscustomobject]@{ projects = @() }
}
if (-not (Get-Member -InputObject $index -Name 'projects' -MemberType NoteProperty)) {
    $index | Add-Member -NotePropertyName projects -NotePropertyValue @() -Force
}
$projects = @($index.projects)
$existingPaths = @{}
foreach ($p in $projects) {
    if ($p.path) { $existingPaths[$p.path.ToLower()] = $true }
}

# 3. Add anything missing.
$whoami = Get-AamWhoAmI
$deviceId = if ($whoami -and $whoami.deviceId) { [string]$whoami.deviceId } else { '' }
$profileLabel = if ($whoami -and $whoami.profileLabel) { [string]$whoami.profileLabel } else { $null }

$added = 0
foreach ($key in $byCwd.Keys) {
    if ($existingPaths.ContainsKey($key)) { continue }
    $entry = $byCwd[$key]
    $sessionId = [System.IO.Path]::GetFileNameWithoutExtension($entry.latestFile)
    $title = Get-LatestAiTitle $entry.latestFile
    $newEntry = [pscustomobject]@{
        path           = $entry.cwd
        name           = (Split-Path -Path $entry.cwd -Leaf)
        lastSessionId  = $sessionId
        lastActive     = $entry.latestTime.ToString('yyyy-MM-ddTHH:mm:sszzz')
        created        = $entry.earliestTime.ToString('yyyy-MM-ddTHH:mm:sszzz')
        autoStatus     = $title
        statusOverride = $null
        # Unknown for pre-existing sessions - we have no reliable way to tell
        # which backend was active back when they were created. Gets filled
        # in automatically the next time this project has a SessionStart/End.
        authBackend    = $null
        deviceId       = $deviceId
        toolKind       = 'claude'
        profileLabel   = $profileLabel
    }
    $projects += $newEntry
    $existingPaths[$key] = $true
    $added++
    Write-Host "  + $($entry.cwd)"
}

$index.projects = $projects
($index | ConvertTo-Json -Depth 6) | Out-File -FilePath $IndexPath -Encoding utf8

Write-Host "Backfill complete: added $added new project(s). Index now has $($projects.Count) total."
