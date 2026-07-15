# Ask Brisvia's node to close, and wait for it. Never kill it.
#
# WHY THIS FILE EXISTS
# --------------------
# The installer hook used to be two lines:
#
#     nsExec::Exec 'taskkill /F /IM brisvia-worker.exe'
#     nsExec::Exec 'taskkill /F /IM bitcoind.exe'
#     Sleep 1000
#
# Both of those are wrong, and the second one twice over.
#
#   /F kills. Bitcoin Core flushes the chainstate on shutdown and its own docs say that can take
#   minutes. Killing it mid-flush is how a block database ends up half-written, and the user finds out
#   on the next start -- if they are lucky, with a repair; if not, with a resync from zero. Measured
#   against the published 1.0.5: not one line reached debug.log during an update. No "Shutdown in
#   progress...", no "Shutdown done". It was killed, not stopped.
#
#   /IM matches by NAME. `bitcoind.exe` is not our name -- it is Bitcoin Core's. A user running their
#   own Bitcoin node got it killed by our installer. That is someone else's money and someone else's
#   chain, and we had no business touching it.
#
# So: find the node that is OURS, by full executable path. Ask it to stop through its own RPC. Wait for
# that exact process to exit. If it will not, ABORT the install -- because the alternative is replacing
# files underneath a process that is still writing to them.
#
# Exit 0 = no node of ours is running any more; safe to install.
# Exit 1 = a node of ours is still alive, or we could not talk to it. Do not install.
#
# Fail closed: every unknown path leads to exit 1. An installer that stops is an annoyance; an
# installer that corrupts a chain is not recoverable by the user.

param(
    # The install directory NSIS is about to write into. Our sidecar lives under it.
    [Parameter(Mandatory = $true)][string]$InstallDir,
    # Ceiling, not a wait. It returns as soon as the process is actually gone.
    [int]$TimeoutSeconds = 180
)

$ErrorActionPreference = 'Stop'
function Log($m) { Write-Host "[brisvia-shutdown] $m" }

try {
    # ---------------------------------------------------------------- find OUR node, and only ours
    # By full path, never by name. A user's own bitcoind.exe is not ours to touch.
    $ourExe = Join-Path $InstallDir 'binaries\bitcoind.exe'
    Log "our node would be: $ourExe"

    $procs = @(Get-CimInstance Win32_Process -Filter "Name = 'bitcoind.exe'" -ErrorAction SilentlyContinue |
               Where-Object { $_.ExecutablePath -and ($_.ExecutablePath -ieq $ourExe) })

    if ($procs.Count -eq 0) {
        # Either nothing is running, or what runs is somebody else's node. Both mean: nothing to do.
        $ajenos = @(Get-CimInstance Win32_Process -Filter "Name = 'bitcoind.exe'" -ErrorAction SilentlyContinue)
        if ($ajenos.Count -gt 0) {
            Log "there are $($ajenos.Count) bitcoind.exe running, none of them ours. Leaving them alone."
            foreach ($a in $ajenos) { Log "  not ours: $($a.ExecutablePath)" }
        } else {
            Log "no node running"
        }
        exit 0
    }

    foreach ($p in $procs) {
        $pid_ = $p.ProcessId
        $creado = $p.CreationDate
        Log "our node: PID $pid_, started $creado"
        Log "  command line: $($p.CommandLine)"

        # ------------------------------------------------------------ its REAL datadir, from its own
        # command line. Not guessed, not a hardcoded default: whatever this process was actually told.
        $datadir = $null
        if ($p.CommandLine -match '-datadir=(?:"([^"]+)"|([^\s"]+))') {
            $datadir = if ($Matches[1]) { $Matches[1] } else { $Matches[2] }
        }
        if (-not $datadir) {
            Log "FAIL: cannot read -datadir from its command line. Not guessing where its data lives."
            exit 1
        }
        Log "  datadir: $datadir"

        # The chain subfolder holds the cookie. Read it from the command line too.
        $chain = if ($p.CommandLine -match '-chain=([^\s"]+)') { $Matches[1] } else { $null }
        if (-not $chain) {
            Log "FAIL: cannot read -chain from its command line."
            exit 1
        }
        $sub = if ($chain -eq 'main') { '' } else { $chain }
        $cookie = if ($sub) { Join-Path $datadir "$sub\.cookie" } else { Join-Path $datadir '.cookie' }

        # ------------------------------------------------------------ its RPC port, from its own config
        $port = $null
        if ($p.CommandLine -match '-rpcport=(\d+)') { $port = [int]$Matches[1] }
        if (-not $port) {
            $conf = Join-Path $datadir 'bitcoin.conf'
            if (Test-Path $conf) {
                foreach ($l in Get-Content $conf) {
                    if ($l -match '^\s*rpcport\s*=\s*(\d+)') { $port = [int]$Matches[1] }
                }
            }
        }
        if (-not $port) {
            Log "FAIL: cannot determine its RPC port. Not guessing: a wrong port talks to somebody else."
            exit 1
        }
        Log "  RPC port: $port"

        if (-not (Test-Path $cookie)) {
            Log "FAIL: no cookie at $cookie. Cannot authenticate, so cannot ask it to stop."
            exit 1
        }
        $auth = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes((Get-Content $cookie -Raw).Trim()))

        # ------------------------------------------------------------ ask it to close, its own way
        # `stop` is Bitcoin Core's own orderly-shutdown request. It is what writes "Shutdown in
        # progress..." and then "Shutdown done" to debug.log -- the two lines that prove it closed
        # properly rather than being cut off.
        $t0 = Get-Date
        try {
            $r = Invoke-RestMethod -Uri "http://127.0.0.1:$port/" -Method Post -TimeoutSec 30 `
                 -Headers @{ Authorization = "Basic $auth" } -ContentType 'application/json' `
                 -Body '{"jsonrpc":"1.0","id":"installer","method":"stop","params":[]}'
            Log "  RPC stop sent at $($t0.ToString('HH:mm:ss.fff')) -> $($r.result)"
        } catch {
            Log "FAIL: the node did not accept `stop` ($_). Not killing it: it may be writing."
            exit 1
        }

        # ------------------------------------------------------------ wait for THIS process to be gone
        # Same PID AND same creation time. Windows reuses PIDs: a PID on its own would let us conclude
        # "it is gone" about a completely different program that happened to inherit the number.
        $limite = (Get-Date).AddSeconds($TimeoutSeconds)
        while ((Get-Date) -lt $limite) {
            $vivo = Get-CimInstance Win32_Process -Filter "ProcessId = $pid_" -ErrorAction SilentlyContinue
            if (-not $vivo -or $vivo.CreationDate -ne $creado) {
                $s = ((Get-Date) - $t0).TotalSeconds
                Log "  the node closed on its own after $([math]::Round($s,1))s"
                break
            }
            Start-Sleep -Milliseconds 250
        }
        $vivo = Get-CimInstance Win32_Process -Filter "ProcessId = $pid_" -ErrorAction SilentlyContinue
        if ($vivo -and $vivo.CreationDate -eq $creado) {
            Log "FAIL: still running after ${TimeoutSeconds}s. NOT killing it: taking long means writing."
            Log "      Aborting the install. Replacing files under a live node is what corrupts chains."
            exit 1
        }
    }

    # ---------------------------------------------------------------- the miner, ours and stateless
    # It holds no chain data, but while it holds its .exe open the installer cannot replace it. Also by
    # full path: `brisvia-worker.exe` is our name, but a path is still the honest way to ask.
    $worker = Join-Path $InstallDir 'binaries\brisvia-worker.exe'
    $ws = @(Get-CimInstance Win32_Process -Filter "Name = 'brisvia-worker.exe'" -ErrorAction SilentlyContinue |
            Where-Object { $_.ExecutablePath -and ($_.ExecutablePath -ieq $worker) })
    foreach ($w in $ws) {
        Log "miner still up (PID $($w.ProcessId)); it exits on its own once the node is gone. Waiting."
        $limite = (Get-Date).AddSeconds(30)
        while ((Get-Date) -lt $limite) {
            if (-not (Get-CimInstance Win32_Process -Filter "ProcessId = $($w.ProcessId)" -ErrorAction SilentlyContinue)) { break }
            Start-Sleep -Milliseconds 250
        }
        if (Get-CimInstance Win32_Process -Filter "ProcessId = $($w.ProcessId)" -ErrorAction SilentlyContinue) {
            Log "FAIL: the miner is still holding its file open. Aborting rather than installing on top."
            exit 1
        }
    }

    Log "OK: no node or miner of ours is running. Safe to install."
    exit 0
}
catch {
    # Anything unexpected means we do not know the state, and not knowing means do not install.
    Log "FAIL (unexpected): $_"
    exit 1
}
