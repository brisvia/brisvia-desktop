# Drive shutdown-brisvia-node.ps1 through one of the seven installer scenarios, with a real bitcoind.
#
# WHY THIS EXISTS
# --------------
# The Rust unit tests exercise wait_for_node_exit -- the in-process wait loop. They do NOT exercise the
# PowerShell script the installer actually runs: identifying OUR node by full path, reading its datadir,
# chain and rpcport, sending RPC stop, waiting for that exact PID+creation-time to be gone, and never
# touching a stranger's Bitcoin Core. That logic only runs against real processes, so this tests it
# against real bitcoind nodes.
#
# THE ONE RULE THAT MAKES THIS HONEST
# -----------------------------------
# Test nodes are launched the way the APP launches them: `bitcoind -datadir=<dd>` on the command line,
# with chain and rpcport in bitcoin.conf. Not `-chain=regtest` on the command line. Launching the
# convenient way would have passed while the shipped installer aborted every update, because the script
# reads chain from the conf (the app never puts it on the command line). The harness must reproduce the
# real shape or it proves nothing about the real installer.
#
# Every scenario emits JSONL events and a final verdict. Fail-closed: an unknown state is a failure.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet(
        'solo-brisvia', 'brisvia-plus-foreign', 'foreign-only', 'unicode-datadir',
        'stale-reused-pid', 'bad-rpc-channel', 'multiple-valid', 'multiple-one-ambiguous')]
    [string]$Scenario,
    [Parameter(Mandatory)][string]$BitcoindExe,   # a real bitcoind.exe, from the build
    [Parameter(Mandatory)][string]$ShutdownScript, # src-tauri/windows/shutdown-brisvia-node.ps1
    [Parameter(Mandatory)][string]$WorkRoot,       # a clean, empty directory for this scenario
    [int]$Port1 = 43101,
    [int]$Port2 = 43102
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$evidence = Join-Path $WorkRoot 'evidence.jsonl'
$script:t0 = Get-Date

function Emit($event, $data) {
    # One JSONL line per event. UTC and elapsed so the chronology can be read back:
    # rpc_stop_sent -> shutdown_in_progress -> shutdown_done -> pid_absent -> hook_done
    $rec = [ordered]@{
        ts_utc     = (Get-Date).ToUniversalTime().ToString('o')
        elapsed_ms = [int]((Get-Date) - $script:t0).TotalMilliseconds
        scenario   = $Scenario
        event      = $event
    }
    if ($data) { foreach ($k in $data.Keys) { $rec[$k] = $data[$k] } }
    ($rec | ConvertTo-Json -Compress -Depth 6) | Out-File -Append -Encoding utf8 $evidence
    Write-Host "  [$event] $($data | ConvertTo-Json -Compress -Depth 4)"
}

# --- launch a bitcoind exactly the way the app does: -datadir on the cmdline, chain/rpcport in the conf
function Start-Node($exePath, $dataDir, $port, $chain = 'regtest') {
    New-Item -ItemType Directory -Force -Path (Split-Path $exePath) | Out-Null
    # Only copy if it is not already there. Two instances share one install path (multiple-*), and the
    # first process holds the .exe open -- re-copying it fails with "being used by another process".
    if (-not (Test-Path $exePath)) { Copy-Item $BitcoindExe $exePath -Force }
    New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
    # The app's conf shape: top-level chain=, and rpcport under the [chain] section. The script reads
    # chain from HERE (not the command line) -- the fallback this harness must exercise.
    @(
        "chain=$chain"
        "server=1"
        "[$chain]"
        "rpcport=$port"
        "rpcbind=127.0.0.1"
        "rpcallowip=127.0.0.1"
    ) | Out-File -Encoding ascii (Join-Path $dataDir 'bitcoin.conf')

    # -maxtipage so regtest's 2011 genesis is not treated as "still syncing"; the app passes the same in
    # its isolated runs. Only -datadir goes on the command line, like the app.
    #
    # The whole -datadir=<path> is wrapped in quotes: Start-Process -ArgumentList does NOT quote array
    # elements, so a spaced/unicode path ("John data ü") would reach bitcoind split into several
    # arguments -- it would use the wrong datadir and never come up on RPC. That was the unicode-datadir
    # failure. Same trap as Start-Process in the diagnostic; it does not quote spaces.
    $p = Start-Process -FilePath $exePath `
        -ArgumentList "`"-datadir=$dataDir`"", "-maxtipage=3153600000" `
        -PassThru -WindowStyle Hidden
    return $p
}

# --- wait until RPC actually answers (cookie written + getblockchaininfo returns)
function Wait-Rpc($dataDir, $port, $chain = 'regtest', $timeoutSec = 60) {
    $sub = if ($chain -eq 'main') { '' } else { $chain }
    $cookie = if ($sub) { Join-Path $dataDir "$sub\.cookie" } else { Join-Path $dataDir '.cookie' }
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path $cookie) {
            try {
                $auth = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes((Get-Content $cookie -Raw).Trim()))
                $r = Invoke-RestMethod -Uri "http://127.0.0.1:$port/" -Method Post -TimeoutSec 5 `
                    -Headers @{ Authorization = "Basic $auth" } -ContentType 'application/json' `
                    -Body '{"jsonrpc":"1.0","id":"harness","method":"getblockchaininfo","params":[]}'
                if ($r.result) { return $true }
            } catch { }
        }
        Start-Sleep -Milliseconds 300
    }
    return $false
}

function Rpc-Alive($port, $dataDir, $chain = 'regtest') {
    $sub = if ($chain -eq 'main') { '' } else { $chain }
    $cookie = if ($sub) { Join-Path $dataDir "$sub\.cookie" } else { Join-Path $dataDir '.cookie' }
    if (-not (Test-Path $cookie)) { return $false }
    try {
        $auth = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes((Get-Content $cookie -Raw).Trim()))
        $r = Invoke-RestMethod -Uri "http://127.0.0.1:$port/" -Method Post -TimeoutSec 5 `
            -Headers @{ Authorization = "Basic $auth" } -ContentType 'application/json' `
            -Body '{"jsonrpc":"1.0","id":"harness","method":"uptime","params":[]}'
        return $null -ne $r
    } catch { return $false }
}

function Proc-Info($pid_) {
    Get-CimInstance Win32_Process -Filter "ProcessId = $pid_" -ErrorAction SilentlyContinue
}

# --- run the shipped shutdown script, capture exit code from the process object (never via a pipeline)
function Invoke-Shutdown($installDir) {
    Emit 'hook_start' @{ install_dir = $installDir }
    $ps = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
    $out = Join-Path $WorkRoot 'shutdown.out.txt'
    # Both paths quoted: Start-Process -ArgumentList does not quote array elements, so a spaced InstallDir
    # ("John Smith ü\Brisvia Sim") arrived split and "Smith" landed on -TimeoutSeconds ("cannot convert
    # to Int32"). The real NSIS hook already quotes -InstallDir "$INSTDIR"; the harness must match it.
    $p = Start-Process -FilePath $ps `
        -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$ShutdownScript`"", '-InstallDir', "`"$installDir`"" `
        -NoNewWindow -Wait -PassThru -RedirectStandardOutput $out
    $code = $p.ExitCode
    $log = if (Test-Path $out) { Get-Content $out -Raw } else { '' }
    Emit 'hook_done' @{ exit_code = $code }
    if ($log -match 'RPC stop sent')        { Emit 'rpc_stop_sent' @{} }
    if ($log -match 'closed on its own')     { Emit 'shutdown_done' @{} }
    return @{ code = $code; log = $log }
}

$spawned = New-Object System.Collections.ArrayList
function Track($p) { [void]$spawned.Add($p); return $p }

$fail = 0
function Check($name, $cond, $detail) {
    if ($cond) { Emit 'assert_pass' @{ check = $name; detail = $detail } }
    else { Emit 'assert_fail' @{ check = $name; detail = $detail }; $script:fail++ }
}

try {
    $install = Join-Path $WorkRoot 'brisvia'
    $ourExe  = Join-Path $install 'binaries\bitcoind.exe'
    $ourDd   = Join-Path $WorkRoot 'brisvia-data'
    $foreignExe = Join-Path $WorkRoot 'other\bitcoind.exe'
    $foreignDd  = Join-Path $WorkRoot 'other-data'

    switch ($Scenario) {

        'solo-brisvia' {
            $node = Track (Start-Node $ourExe $ourDd $Port1)
            Check 'node_up' (Wait-Rpc $ourDd $Port1) 'our node answers RPC before the hook runs'
            $before = $node.Id
            $r = Invoke-Shutdown $install
            Check 'exit_zero' ($r.code -eq 0) "exit=$($r.code), expected 0"
            Check 'pid_gone' (-not (Proc-Info $before)) "PID $before must be gone"
            Check 'datadir_kept' (Test-Path $ourDd) 'the datadir must not be deleted'
        }

        'brisvia-plus-foreign' {
            $ours = Track (Start-Node $ourExe $ourDd $Port1)
            $foreign = Track (Start-Node $foreignExe $foreignDd $Port2)
            Check 'ours_up' (Wait-Rpc $ourDd $Port1) 'our node answers'
            Check 'foreign_up' (Wait-Rpc $foreignDd $Port2) 'the foreign node answers'
            $ourPid = $ours.Id; $foreignPid = $foreign.Id
            $foreignCreated = (Proc-Info $foreignPid).CreationDate
            $r = Invoke-Shutdown $install
            Check 'exit_zero' ($r.code -eq 0) "exit=$($r.code)"
            Check 'ours_gone' (-not (Proc-Info $ourPid)) "our PID $ourPid gone"
            $fi = Proc-Info $foreignPid
            Check 'foreign_same_pid_alive' ($fi -and $fi.CreationDate -eq $foreignCreated) "foreign PID $foreignPid still the same process"
            Check 'foreign_still_responds' (Rpc-Alive $Port2 $foreignDd) 'foreign node still answers RPC'
        }

        'foreign-only' {
            $foreign = Track (Start-Node $foreignExe $foreignDd $Port2)
            Check 'foreign_up' (Wait-Rpc $foreignDd $Port2) 'the foreign node answers'
            $foreignPid = $foreign.Id
            $foreignCreated = (Proc-Info $foreignPid).CreationDate
            $r = Invoke-Shutdown $install   # our install dir has no node
            Check 'exit_zero' ($r.code -eq 0) "exit=$($r.code): nothing of ours to close"
            $fi = Proc-Info $foreignPid
            Check 'foreign_untouched' ($fi -and $fi.CreationDate -eq $foreignCreated) "foreign PID $foreignPid untouched"
            Check 'foreign_still_responds' (Rpc-Alive $Port2 $foreignDd) 'foreign node still answers'
        }

        'unicode-datadir' {
            # Spaces, accents and non-latin, in both the exe path and the datadir. This is the case the
            # CommandLineToArgvW rewrite exists for: a regex split on spaces returned C:\Users\John and
            # aborted the install of anyone with a space in their path.
            $uInstall = Join-Path $WorkRoot 'John Smith ü\Brisvia Sim'
            $uExe = Join-Path $uInstall 'binaries\bitcoind.exe'
            $uDd  = Join-Path $WorkRoot 'John data ü\brisvia chain'
            $node = Track (Start-Node $uExe $uDd $Port1)
            Check 'node_up' (Wait-Rpc $uDd $Port1) 'node with a spaced/unicode datadir answers'
            $before = $node.Id
            $r = Invoke-Shutdown $uInstall
            Check 'exit_zero' ($r.code -eq 0) "exit=$($r.code)"
            Check 'identified_and_closed' (-not (Proc-Info $before)) "PID $before identified through the spaced path and closed"
        }

        'stale-reused-pid' {
            # The script keys on the live process's full path, never on a stored PID. Prove it ignores a
            # pid file: a foreign node is running, our datadir holds a bitcoind.pid pointing AT it, and
            # nothing runs from our path. The script must not touch the foreign node.
            $foreign = Track (Start-Node $foreignExe $foreignDd $Port2)
            Check 'foreign_up' (Wait-Rpc $foreignDd $Port2) 'foreign node answers'
            $foreignPid = $foreign.Id
            $foreignCreated = (Proc-Info $foreignPid).CreationDate
            New-Item -ItemType Directory -Force -Path $ourDd | Out-Null
            "$foreignPid" | Out-File -Encoding ascii (Join-Path $ourDd 'bitcoind.pid')
            $r = Invoke-Shutdown $install
            Check 'exit_zero' ($r.code -eq 0) "exit=$($r.code): a stored PID is not a reason to act"
            $fi = Proc-Info $foreignPid
            Check 'foreign_not_touched_by_stored_pid' ($fi -and $fi.CreationDate -eq $foreignCreated) "foreign PID $foreignPid untouched despite being named in our pid file"
        }

        'bad-rpc-channel' {
            # THE safety test. Our node is running, but the shutdown channel is broken: the cookie is
            # gone, so the script cannot authenticate. It must fail closed -- exit non-zero, node STILL
            # ALIVE (never killed), datadir intact -- so the installer aborts rather than replacing files
            # under a live node.
            $node = Track (Start-Node $ourExe $ourDd $Port1)
            Check 'node_up' (Wait-Rpc $ourDd $Port1) 'our node answers before we break the channel'
            $before = $node.Id
            $beforeCreated = (Proc-Info $before).CreationDate
            Remove-Item (Join-Path $ourDd 'regtest\.cookie') -Force -ErrorAction SilentlyContinue
            $r = Invoke-Shutdown $install
            Check 'exit_nonzero' ($r.code -ne 0) "exit=$($r.code), expected non-zero (fail closed)"
            $ni = Proc-Info $before
            Check 'node_still_alive' ($ni -and $ni.CreationDate -eq $beforeCreated) "node PID $before must still be alive -- never killed"
        }

        'multiple-valid' {
            # Two nodes from the SAME install path, each with its own datadir and RPC, both resolvable.
            # Two instances is NOT ambiguity: the shutdown resolves both in phase 1, then closes both. The
            # install may continue only once both are gone.
            $a = Track (Start-Node $ourExe $ourDd $Port1)
            $ourDd2 = Join-Path $WorkRoot 'brisvia-data-2'
            $bExe = Join-Path $WorkRoot 'brisvia-2\binaries\bitcoind.exe'  # same install dir logically; second datadir
            $b = Track (Start-Node $ourExe $ourDd2 $Port2)
            Check 'both_up' ((Wait-Rpc $ourDd $Port1) -and (Wait-Rpc $ourDd2 $Port2)) 'two nodes from our path are up'
            $aPid = $a.Id; $bPid = $b.Id
            $r = Invoke-Shutdown $install
            Check 'exit_zero' ($r.code -eq 0) "exit=$($r.code): both resolvable, both should close"
            Check 'both_closed' ((-not (Proc-Info $aPid)) -and (-not (Proc-Info $bPid))) "both PIDs ($aPid, $bPid) must be gone"
        }

        'multiple-one-ambiguous' {
            # Two nodes from our path, but ONE cannot be resolved: its bitcoin.conf is gone, so its chain
            # and port are unknown. Phase 1 must abort BEFORE stopping anything, so BOTH stay alive. This
            # is the flaw the two-phase design fixes: never close one and then discover the other is
            # unresolvable, leaving a half-closed set.
            $a = Track (Start-Node $ourExe $ourDd $Port1)
            $ourDd2 = Join-Path $WorkRoot 'brisvia-data-2'
            $b = Track (Start-Node $ourExe $ourDd2 $Port2)
            Check 'both_up' ((Wait-Rpc $ourDd $Port1) -and (Wait-Rpc $ourDd2 $Port2)) 'two nodes from our path are up'
            $aPid = $a.Id; $bPid = $b.Id
            $aCreated = (Proc-Info $aPid).CreationDate
            $bCreated = (Proc-Info $bPid).CreationDate
            # Make the SECOND one unresolvable: remove its conf so chain/port cannot be read.
            Remove-Item (Join-Path $ourDd2 'bitcoin.conf') -Force -ErrorAction SilentlyContinue
            $r = Invoke-Shutdown $install
            Check 'exit_nonzero' ($r.code -ne 0) "exit=$($r.code), expected non-zero: one is ambiguous, abort"
            $ai = Proc-Info $aPid; $bi = Proc-Info $bPid
            Check 'nothing_stopped' (($ai -and $ai.CreationDate -eq $aCreated) -and ($bi -and $bi.CreationDate -eq $bCreated)) `
                "BOTH PIDs must still be alive -- phase 1 aborts before touching anything"
        }
    }

    # $(...) not (...): the grouping operator does not allow an `if` statement inside it and PowerShell
    # tried to run `if` as a command -- "the term 'if' is not recognized". The scenario logic had already
    # passed every assertion; only this verdict line threw. Computed in a variable to leave no doubt.
    $verdict = if ($fail -eq 0) { 'PASS' } else { 'FAIL' }
    Emit 'verdict' @{ failures = $fail; result = $verdict }
}
finally {
    # Teardown kills ONLY the PIDs this harness spawned, after the assertions ran. This is test cleanup,
    # not the product, and it never uses taskkill /F against anything the harness did not start.
    foreach ($p in $spawned) {
        try { if (Proc-Info $p.Id) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } } catch { }
    }
}

if ($fail -gt 0) { exit 1 }
exit 0
