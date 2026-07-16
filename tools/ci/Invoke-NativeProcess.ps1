# Run a native executable and report what actually happened. Never guess.
#
# WHY THIS FILE EXISTS
# --------------------
# A diagnostic step wrote this:
#
#     & $exe --list 2>&1 | Tee-Object $log | Select-Object -First 10
#     $c = $LASTEXITCODE
#
# `Select-Object -First 10` stops the pipeline as soon as it has ten items. The pipeline never
# completes, so PowerShell never sets $LASTEXITCODE, so $c was $null, so `$c -ne 0` was true against
# nothing, and the step threw. The executable may well have loaded perfectly and been rejected by its
# own guard. The evidence read, in full: `exit= hex=0x`.
#
# That was the seventh self-inflicted bug in one day of building this diagnostic, and every one of them
# was in the tool rather than in the product. The pattern was treating the diagnostic as throwaway code
# while decisions about the product came out of it.
#
# So: the exit code comes from the Process OBJECT, never from $LASTEXITCODE, and the executable is never
# placed inside a pipeline. Output goes to files and is read back afterwards. Truncating output for
# display happens after the code is already captured, and cannot touch it.
#
# WHAT IT DISTINGUISHES, AND WHY THAT MATTERS
# -------------------------------------------
#   Launch failure  -- the process never started (missing file, bad path). There IS no exit code, and
#                      inventing one would be a lie. Reported as PROCESS_LAUNCH_FAILED.
#   Exit code       -- the process ran and returned a number. Including 0. Including negative ones.
#
# Conflating those two is how "it failed" becomes unactionable: they need opposite fixes.
#
# THE HEX CONVERSION IS NOT COSMETIC
# ----------------------------------
# Windows returns loader failures as large negative signed integers: 0xC0000139 arrives as
# -1073741511. Formatting that with '0x{0:X8}' -f directly gives 0xFFFFFFFFC0000139 or worse. It has to
# be reinterpreted as unsigned 32-bit first. The hex form is what makes a code searchable, so getting it
# wrong hides the answer in plain sight.
#
# Usage:
#     . tools/ci/Invoke-NativeProcess.ps1
#     $r = Invoke-NativeProcess -FilePath $exe -Arguments @('--list') -OutputDir $dir
#     if (-not $r.Launched) { throw $r.Error }
#     if ($r.ExitCode -ne 0) { throw "failed: $($r.ExitCodeHex)" }

Set-StrictMode -Version Latest

function ConvertTo-Win32Hex {
    <#
    .SYNOPSIS
        Turn a process exit code into the 0xXXXXXXXX form Windows documents.
    .DESCRIPTION
        Exit codes arrive as signed 32-bit integers. Loader failures are large negative numbers:
        0xC0000139 comes back as -1073741511. Reinterpreting the same bits as unsigned is the only way
        to get the form anyone can search for.
    #>
    param([Parameter(Mandatory)][int]$Code)
    $unsigned = [BitConverter]::ToUInt32([BitConverter]::GetBytes($Code), 0)
    return '0x{0:X8}' -f $unsigned
}

function ConvertTo-QuotedArgument {
    <#
    .SYNOPSIS
        Quote one argument the way the Windows command line actually needs it.
    .DESCRIPTION
        Start-Process -ArgumentList joins an array with spaces and does NOT quote the elements. So an
        argument containing a space silently becomes two arguments, and the program runs on something
        nobody asked for.

        That is not hypothetical. The preflight caught it on its first run:

            -File C:\Temp\Jose Perez\diagnostic tool\ok.ps1
            -> Processing -File 'C:\Temp\Jose' failed because the file does not
               have a '.ps1' extension

        Most people's usernames have a space in them, so this is the common case, not the edge case. It
        is the same class of bug already found in the installer's shutdown script, which is why this
        function exists rather than a "remember to quote" comment.

        The rules are Microsoft's own (CommandLineToArgvW's, in reverse): backslashes are literal unless
        they precede a quote, in which case they must be doubled; an embedded quote is escaped with a
        backslash; the whole thing is wrapped only when it needs to be.
    #>
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Argument)

    # An empty argument still has to reach the program, as an empty pair of quotes.
    if ($Argument -eq '') { return '""' }
    # Nothing that needs quoting: leave it alone, so simple command lines stay readable in logs.
    if ($Argument -notmatch '[\s"]') { return $Argument }

    $sb = [System.Text.StringBuilder]::new()
    [void]$sb.Append('"')
    $backslashes = 0
    foreach ($c in $Argument.ToCharArray()) {
        if ($c -eq '\') {
            $backslashes++
            continue
        }
        if ($c -eq '"') {
            # Backslashes before a quote are doubled, then the quote itself is escaped.
            [void]$sb.Append('\' * ($backslashes * 2 + 1))
            [void]$sb.Append('"')
            $backslashes = 0
            continue
        }
        [void]$sb.Append('\' * $backslashes)
        $backslashes = 0
        [void]$sb.Append($c)
    }
    # Trailing backslashes are doubled so the closing quote is not escaped by them.
    [void]$sb.Append('\' * ($backslashes * 2))
    [void]$sb.Append('"')
    return $sb.ToString()
}

function Invoke-NativeProcess {
    <#
    .SYNOPSIS
        Run an executable and return a structured result. No pipelines, no $LASTEXITCODE.
    .OUTPUTS
        A hashtable with:
          Launched        [bool]   did the process start at all
          Error           [string] why it did not, when Launched is false
          ExitCode        [int]    the real code, from the process object. $null if it never launched.
          ExitCodeHex     [string] the same code as 0xXXXXXXXX
          StdOut          [string[]] every line, not a preview
          StdErr          [string[]] every line, not a preview
          DurationSeconds [double]
          Command         [string]
          Arguments       [string[]]
          WorkingDirectory [string]
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$Arguments = @(),
        [string]$WorkingDirectory = (Get-Location).Path,
        [Parameter(Mandatory)][string]$OutputDir,
        [int]$TimeoutSeconds = 600
    )

    $result = @{
        Launched         = $false
        Error            = $null
        ExitCode         = $null
        ExitCodeHex      = $null
        StdOut           = @()
        StdErr           = @()
        DurationSeconds  = 0.0
        Command          = $FilePath
        Arguments        = $Arguments
        WorkingDirectory = $WorkingDirectory
    }

    if (-not (Test-Path -LiteralPath $OutputDir)) {
        New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
    }
    # A name unique per call: two runs in the same directory must not read each other's output. Using a
    # fixed name here would make a second call silently report the first one's results.
    $stamp = [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $outPath = Join-Path $OutputDir "proc-$stamp.stdout.txt"
    $errPath = Join-Path $OutputDir "proc-$stamp.stderr.txt"

    $started = Get-Date
    $process = $null
    try {
        # -PassThru gives the process object, which is where the exit code will come from.
        # -NoNewWindow keeps it attached. Redirection to FILES keeps the executable out of a pipeline,
        # which is the whole point of this function.
        $splat = @{
            FilePath               = $FilePath
            WorkingDirectory       = $WorkingDirectory
            NoNewWindow            = $true
            Wait                   = $true
            PassThru               = $true
            RedirectStandardOutput = $outPath
            RedirectStandardError  = $errPath
            ErrorAction            = 'Stop'
        }
        # Each argument is quoted BEFORE it goes in. Start-Process joins the array with spaces and does
        # not quote anything itself, so passing them raw silently splits any argument containing a
        # space -- see ConvertTo-QuotedArgument.
        #
        # An empty -ArgumentList is not the same as no -ArgumentList: passing @() makes some PowerShell
        # versions send an empty quoted argument. Only add it when there is something to send.
        if ($Arguments.Count -gt 0) {
            $splat['ArgumentList'] = @($Arguments | ForEach-Object { ConvertTo-QuotedArgument -Argument $_ })
        }
        $process = Start-Process @splat
    }
    catch {
        # The process never existed. There is no exit code to report, and making one up would be worse
        # than saying so: a missing file and a failing program need opposite fixes.
        $result.Error = "PROCESS_LAUNCH_FAILED: $($_.Exception.Message)"
        $result.DurationSeconds = ((Get-Date) - $started).TotalSeconds
        return $result
    }

    $result.Launched = $true
    $result.DurationSeconds = ((Get-Date) - $started).TotalSeconds

    # The code comes from the object. Not from $LASTEXITCODE, which is session state and can be clobbered
    # by anything that ran in between -- including the reading of the output files below.
    $result.ExitCode = [int]$process.ExitCode
    $result.ExitCodeHex = ConvertTo-Win32Hex -Code $result.ExitCode

    # Read the output AFTER the code is already captured. Nothing done here can affect it.
    foreach ($p in @(@{k = 'StdOut'; f = $outPath }, @{k = 'StdErr'; f = $errPath })) {
        if (Test-Path -LiteralPath $p.f) {
            $lines = @(Get-Content -LiteralPath $p.f -ErrorAction SilentlyContinue)
            $result[$p.k] = $lines
        }
    }
    return $result
}

function Write-NativeProcessReport {
    <#
    .SYNOPSIS
        Print a result for a human. Truncates for DISPLAY only.
    .DESCRIPTION
        Showing ten lines instead of thirty is a display choice. It happens here, long after the exit
        code was captured, and it cannot reach it. That separation is the entire lesson of this file.
    #>
    param(
        [Parameter(Mandatory)][hashtable]$Result,
        [int]$PreviewLines = 10
    )
    if (-not $Result.Launched) {
        Write-Host "  LAUNCH FAILED: $($Result.Error)"
        Write-Host "  command: $($Result.Command)"
        return
    }
    Write-Host "  exit=$($Result.ExitCode) hex=$($Result.ExitCodeHex) in $([math]::Round($Result.DurationSeconds, 1))s"
    Write-Host "  stdout: $($Result.StdOut.Count) lines, stderr: $($Result.StdErr.Count) lines"
    if ($Result.StdOut.Count -gt 0) {
        Write-Host "  --- stdout (first $PreviewLines of $($Result.StdOut.Count)) ---"
        $Result.StdOut | Select-Object -First $PreviewLines | ForEach-Object { Write-Host "    $_" }
    }
    if ($Result.StdErr.Count -gt 0) {
        Write-Host "  --- stderr (first $PreviewLines of $($Result.StdErr.Count)) ---"
        $Result.StdErr | Select-Object -First $PreviewLines | ForEach-Object { Write-Host "    $_" }
    }
}

function Get-CargoTestExecutable {
    <#
    .SYNOPSIS
        Find the ONE test executable in cargo's JSON output. Refuse zero, refuse many.
    .DESCRIPTION
        profile.test marks an artifact as a test binary. target.test only says a target CAN be tested --
        filtering on that matched nothing and cost a whole 40-minute run.

        Zero and more-than-one both throw on purpose. Zero means the build did not produce what we came
        for; picking "the last one" out of several means the diagnostic silently measures a binary
        nobody chose.
    #>
    param([Parameter(Mandatory)][string[]]$CargoJsonLines)

    $found = @()
    foreach ($line in $CargoJsonLines) {
        if ($line -notmatch '^\s*\{') { continue }
        try { $m = $line | ConvertFrom-Json } catch { continue }
        if ($m.PSObject.Properties.Name -notcontains 'reason') { continue }
        if ($m.reason -ne 'compiler-artifact') { continue }
        if (-not $m.executable) { continue }
        if ($m.PSObject.Properties.Name -notcontains 'profile') { continue }
        if ($m.profile.test -ne $true) { continue }
        $found += $m.executable
    }
    if ($found.Count -eq 0) { throw "NO_TEST_EXECUTABLE: cargo reported no artifact with profile.test = true" }
    if ($found.Count -gt 1) { throw "AMBIGUOUS_TEST_EXECUTABLE: $($found.Count) found: $($found -join ', ')" }
    return $found[0]
}
