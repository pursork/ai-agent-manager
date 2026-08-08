<#
.SYNOPSIS
    Claude Code hook script (SessionStart / SessionEnd) that keeps
    ~/.claude/project-index.json up to date automatically.

.DESCRIPTION
    Reads the hook's JSON payload from stdin, figures out which event fired
    (from "hook_event_name" in the payload itself, so the SAME script can be
    registered for both SessionStart and SessionEnd), and upserts an entry
    for the session's cwd into the central project index.

    - SessionStart: registers/touches the project (name, path, lastSessionId,
      lastActive). Never touches autoStatus.
    - SessionEnd: additionally scans the session transcript for the most
      recent "ai-title" record (Claude Code auto-generates these) and stores
      it as autoStatus - a free one-line status with zero extra effort.

    Designed to never break a session: all failures are swallowed and logged
    to track-session.log next to this script; the script always exits 0.

    This copy is bundled with ai-agent-manager (`aam skills install-bundled
    project-tracker`) and additionally shells out to `aam whoami --tool
    claude` to fill in `deviceId`/`profileLabel` -- the cross-device fields
    `docs/05-session-memory-bank-module.md` §5.2 defines, which only aam
    itself can compute (device identity lives in aam-vault's DPAPI-encrypted
    store; Profile lookup needs aam's own registry). If `aam` isn't on PATH
    or the call fails for any reason, these fields are left blank -- same
    as any pre-aam record -- never a fatal error for the hook.
#>

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$LogPath   = Join-Path $ScriptDir 'track-session.log'
$IndexPath = Join-Path $HOME '.claude\project-index.json'

# Paths (prefix match, case-insensitive) to never auto-track.
$ExcludePrefixes = @(
    (Join-Path $HOME '.claude')
)

function Write-Log($msg) {
    try {
        "$(Get-Date -Format o)  $msg" | Out-File -FilePath $LogPath -Append -Encoding utf8
    } catch { }
}

function Get-CurrentAuthBackend {
    # Best-effort detection of which backend this session is talking to, based
    # on the env vars Claude Code itself uses to pick an auth method (see
    # https://code.claude.com/docs/en/authentication.md precedence order).
    # Purely informational - used to warn on resume, not to gate anything.
    if ($env:CLAUDE_CODE_USE_BEDROCK -and $env:CLAUDE_CODE_USE_BEDROCK -notin @('0', 'false')) { return 'bedrock' }
    if ($env:CLAUDE_CODE_USE_VERTEX -and $env:CLAUDE_CODE_USE_VERTEX -notin @('0', 'false')) { return 'vertex' }
    if ($env:ANTHROPIC_BASE_URL -and $env:ANTHROPIC_BASE_URL -notmatch 'api\.anthropic\.com') { return "custom:$($env:ANTHROPIC_BASE_URL)" }
    if ($env:ANTHROPIC_AUTH_TOKEN) { return 'auth-token' }
    if ($env:ANTHROPIC_API_KEY) { return 'api-key' }
    return 'oauth-subscription'
}

function Get-AamWhoAmI {
    # `aam whoami` inherits CLAUDE_CONFIG_DIR down the process chain from
    # `aam claude <label>` (if that's how this session was launched), so no
    # arguments beyond --tool are needed. Never lets a missing/failing `aam`
    # break the hook -- returns $null on any problem, same tier of "optional
    # enrichment" as Get-LatestAiTitle below.
    try {
        $json = & aam whoami --tool claude 2>$null
        if (-not $json) { return $null }
        return $json | ConvertFrom-Json
    } catch {
        return $null
    }
}

function Get-LatestAiTitle([string]$transcriptPath) {
    if (-not $transcriptPath -or -not (Test-Path -LiteralPath $transcriptPath)) { return $null }
    try {
        $matches = Select-String -LiteralPath $transcriptPath -Pattern '"type"\s*:\s*"ai-title"' -SimpleMatch:$false
        if (-not $matches) { return $null }
        $lastLine = $matches[-1].Line
        $obj = $lastLine | ConvertFrom-Json
        if ($obj.aiTitle) { return [string]$obj.aiTitle }
    } catch {
        Write-Log "Get-LatestAiTitle failed: $_"
    }
    return $null
}

function Set-NoteProperty($obj, [string]$name, $value) {
    if (Get-Member -InputObject $obj -Name $name -MemberType NoteProperty) {
        $obj.$name = $value
    } else {
        $obj | Add-Member -NotePropertyName $name -NotePropertyValue $value -Force
    }
}

try {
    $raw = [Console]::In.ReadToEnd()
    if (-not $raw) { Write-Log 'Empty stdin, exiting.'; exit 0 }

    $payload = $raw | ConvertFrom-Json

    $eventName = $payload.hook_event_name
    $cwd       = $payload.cwd
    $sessionId = $payload.session_id
    $transcript = $payload.transcript_path

    if (-not $cwd) { Write-Log "No cwd in payload, event=$eventName, skipping."; exit 0 }

    foreach ($prefix in $ExcludePrefixes) {
        if ($cwd.ToLower().StartsWith($prefix.ToLower())) {
            Write-Log "cwd '$cwd' matches exclude prefix '$prefix', skipping."
            exit 0
        }
    }

    $mutex = New-Object System.Threading.Mutex($false, 'Global\ClaudeProjectIndexMutex')
    $acquired = $false
    try {
        $acquired = $mutex.WaitOne(5000)
        if (-not $acquired) { Write-Log 'Could not acquire index mutex within 5s, skipping this update.'; exit 0 }

        if (Test-Path -LiteralPath $IndexPath) {
            $index = Get-Content -Raw -LiteralPath $IndexPath | ConvertFrom-Json
        } else {
            $index = [pscustomobject]@{ projects = @() }
        }
        if (-not (Get-Member -InputObject $index -Name 'projects' -MemberType NoteProperty)) {
            $index | Add-Member -NotePropertyName projects -NotePropertyValue @() -Force
        }

        $projects = @($index.projects)
        $now = (Get-Date).ToString('yyyy-MM-ddTHH:mm:sszzz')

        $existing = $projects | Where-Object { $_.path -and ($_.path.ToLower() -eq $cwd.ToLower()) } | Select-Object -First 1
        $authBackend = Get-CurrentAuthBackend
        $whoami = Get-AamWhoAmI
        $deviceId = if ($whoami -and $whoami.deviceId) { [string]$whoami.deviceId } else { '' }
        $profileLabel = if ($whoami -and $whoami.profileLabel) { [string]$whoami.profileLabel } else { $null }

        if ($existing) {
            $existing.lastActive = $now
            if ($sessionId) { $existing.lastSessionId = $sessionId }
            Set-NoteProperty $existing 'authBackend' $authBackend
            Set-NoteProperty $existing 'deviceId' $deviceId
            Set-NoteProperty $existing 'toolKind' 'claude'
            Set-NoteProperty $existing 'profileLabel' $profileLabel
            if ($eventName -eq 'SessionEnd') {
                $title = Get-LatestAiTitle $transcript
                if ($title) { $existing.autoStatus = $title }
            }
            Write-Log "Updated existing entry for '$cwd' (event=$eventName, authBackend=$authBackend, profileLabel=$profileLabel)."
        } else {
            $name = Split-Path -Path $cwd -Leaf
            $newEntry = [pscustomobject]@{
                path            = $cwd
                name            = $name
                lastSessionId   = $sessionId
                lastActive      = $now
                created         = $now
                autoStatus      = $null
                statusOverride  = $null
                authBackend     = $authBackend
                deviceId        = $deviceId
                toolKind        = 'claude'
                profileLabel    = $profileLabel
            }
            if ($eventName -eq 'SessionEnd') {
                $title = Get-LatestAiTitle $transcript
                if ($title) { $newEntry.autoStatus = $title }
            }
            $projects += $newEntry
            Write-Log "Created new entry for '$cwd' (event=$eventName, authBackend=$authBackend, profileLabel=$profileLabel)."
        }

        $index.projects = $projects

        $tmpPath = "$IndexPath.tmp"
        ($index | ConvertTo-Json -Depth 6) | Out-File -FilePath $tmpPath -Encoding utf8
        Move-Item -LiteralPath $tmpPath -Destination $IndexPath -Force
    } finally {
        if ($acquired) { $mutex.ReleaseMutex() }
        $mutex.Dispose()
    }
} catch {
    Write-Log "Unhandled error: $_"
}

exit 0
