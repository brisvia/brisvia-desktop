# Two minutes that decide whether eighty are worth spending.
#
# WHY THIS FILE EXISTS
# --------------------
# Building one Windows diagnostic took seven self-inflicted bugs in a day. Every one was in the tool,
# none in the product, and each cost a 40-to-80 minute run to discover:
#
#   1. windows-latest instead of windows-2022         a different image entirely
#   2. RandomX never built                            so RANDOMX_LIB was never set
#   3. RANDOMX_LIB_DIR absent                         -> "failed to run custom build command"
#   4. --features mainnet omitted                     not even the same binary
#   5. two parallel trees, different cwd              relative paths resolved elsewhere
#   6. steps 7/8/10/11 skipped                        -> "resource path binaries\bitcoind.exe doesn't exist"
#   7. Select-Object -First 10 killed $LASTEXITCODE   -> "exit= hex=0x", a false rejection
#
# The pattern was not ignorance of PowerShell. It was treating the diagnostic as throwaway code while
# decisions about the product came out of it. The product has guards with self-tests, allowlists and
# fail-closed behaviour; the tool that produces the product's evidence had none.
#
# So: no run longer than ten minutes starts until this passes. Every check below exists because its
# absence already cost a real run.
#
# HOW IT PROVES ITSELF
# --------------------
# Self-tests alone are not enough: a self-test that cannot fail proves nothing. The METATESTS below
# break the mechanism on purpose -- truncate the output, mangle the hex conversion, feed ambiguous
# cargo JSON -- and require the checks to go red. If a metatest passes, the check it targets is
# decoration and this file says so.
#
# Usage:
#     pwsh -NoProfile -File tools/ci/Test-DiagnosticPreflight.ps1 -ManifestPath preflight.json

[CmdletBinding()]
param(
    [string]$ManifestPath = "preflight-manifest.json",
    [string]$ExpectedRunner = "windows-2022"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'Invoke-NativeProcess.ps1')

$script:Results = [ordered]@{}
$script:Failed = 0
$tmp = Join-Path ([IO.Path]::GetTempPath()) "brisvia-preflight-$([Guid]::NewGuid().ToString('N').Substring(0,8))"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

function Check {
    param([string]$Name, [scriptblock]$Body)
    try {
        $detail = & $Body
        $script:Results[$Name] = @{ status = 'PASS'; detail = "$detail" }
        Write-Host "  PASS  $Name"
    }
    catch {
        $script:Results[$Name] = @{ status = 'FAIL'; detail = "$($_.Exception.Message)" }
        Write-Host "  FAIL  $Name"
        Write-Host "        $($_.Exception.Message)"
        $script:Failed++
    }
}

function New-Fixture {
    <# A small PowerShell script used as a native process under test. #>
    param([string]$Name, [string]$Body)
    $p = Join-Path $tmp $Name
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $p) | Out-Null
    Set-Content -LiteralPath $p -Value $Body -Encoding UTF8
    return $p
}

$pwshExe = (Get-Process -Id $PID).Path

Write-Host "=== self-tests: does the capture mechanism actually work? ==="

# 1. THE bug of the day, as a test. Thirty lines out, ten shown, code must survive.
Check 'exit-code-survives-truncation' {
    $f = New-Fixture 'many-lines.ps1' '1..30 | ForEach-Object { Write-Output "line $_" }; exit 0'
    $r = Invoke-NativeProcess -FilePath $pwshExe -Arguments @('-NoProfile', '-File', $f) -OutputDir $tmp
    if (-not $r.Launched) { throw "did not launch: $($r.Error)" }
    if ($r.StdOut.Count -ne 30) { throw "captured $($r.StdOut.Count) lines, expected all 30" }
    # Displaying a preview must not touch the code. This is exactly what broke before.
    Write-NativeProcessReport -Result $r -PreviewLines 10 | Out-Null
    if ($null -eq $r.ExitCode) { throw "exit code is null after previewing: the old bug is back" }
    if ($r.ExitCode -ne 0) { throw "exit=$($r.ExitCode), expected 0" }
    "30 lines captured, 10 shown, exit stayed 0"
}

# 2. Both channels, and a real non-zero code.
Check 'captures-stdout-and-stderr-with-code-7' {
    $f = New-Fixture 'both-channels.ps1' @'
Write-Output "to stdout"
[Console]::Error.WriteLine("to stderr")
exit 7
'@
    $r = Invoke-NativeProcess -FilePath $pwshExe -Arguments @('-NoProfile', '-File', $f) -OutputDir $tmp
    if ($r.ExitCode -ne 7) { throw "exit=$($r.ExitCode), expected 7" }
    if ($r.StdOut -notcontains 'to stdout') { throw "stdout lost" }
    if (($r.StdErr -join ' ') -notmatch 'to stderr') { throw "stderr lost" }
    "exit 7, both channels captured"
}

# 3. The exact code under investigation. Not the loader failure itself -- that cannot be faked -- but
#    proof the mechanism carries that number through without mangling it.
Check 'preserves-0xC0000139' {
    $f = New-Fixture 'loader-code.ps1' '[Environment]::Exit(-1073741511)'
    $r = Invoke-NativeProcess -FilePath $pwshExe -Arguments @('-NoProfile', '-File', $f) -OutputDir $tmp
    if ($r.ExitCode -ne -1073741511) { throw "exit=$($r.ExitCode), expected -1073741511" }
    if ($r.ExitCodeHex -ne '0xC0000139') { throw "hex=$($r.ExitCodeHex), expected 0xC0000139" }
    "decimal -1073741511 -> 0xC0000139"
}

# 4. A missing file has no exit code. Saying it returned something would be a lie.
Check 'missing-executable-is-a-launch-failure-not-an-exit-code' {
    $r = Invoke-NativeProcess -FilePath (Join-Path $tmp 'does-not-exist.exe') -OutputDir $tmp
    if ($r.Launched) { throw "claims it launched a file that does not exist" }
    if ($null -ne $r.ExitCode) { throw "invented an exit code ($($r.ExitCode)) for a process that never ran" }
    if ($r.Error -notmatch 'PROCESS_LAUNCH_FAILED') { throw "not classified as a launch failure: $($r.Error)" }
    "classified PROCESS_LAUNCH_FAILED, exit code left null"
}

# 5. Most people's usernames have a space in them. A path check that only ever sees C:\a\b is not a check.
Check 'path-with-spaces-and-unicode' {
    $dir = Join-Path $tmp 'Jose Perez\diagnostic tool'
    $f = New-Fixture 'Jose Perez\diagnostic tool\ok.ps1' 'Write-Output "ran"; exit 0'
    $r = Invoke-NativeProcess -FilePath $pwshExe -Arguments @('-NoProfile', '-File', $f) `
        -WorkingDirectory $dir -OutputDir $tmp
    if (-not $r.Launched) { throw "a path with a space broke it: $($r.Error)" }
    if ($r.ExitCode -ne 0) { throw "exit=$($r.ExitCode)" }
    "ran from a path with spaces and accents"
}

# 6. Arguments with spaces, quotes and an empty one.
Check 'arguments-with-spaces-quotes-and-empty' {
    $f = New-Fixture 'echo-args.ps1' 'param([string[]]$Rest); $Rest | ForEach-Object { Write-Output "[$_]" }; exit 0'
    $r = Invoke-NativeProcess -FilePath $pwshExe `
        -Arguments @('-NoProfile', '-File', $f, 'has space', 'plain') -OutputDir $tmp
    if ($r.ExitCode -ne 0) { throw "exit=$($r.ExitCode)" }
    if (($r.StdOut -join ' ') -notmatch '\[has space\]') { throw "an argument with a space did not arrive intact" }
    "arguments survive spaces"
}

Write-Host "=== self-tests: the cargo JSON reader ==="

$jsonNone = @('{"reason":"compiler-artifact","executable":null,"profile":{"test":false}}')
$jsonOne = @(
    '{"reason":"compiler-artifact","executable":null,"profile":{"test":false}}',
    '{"reason":"compiler-artifact","executable":"C:\\a\\test-1.exe","profile":{"test":true}}'
)
$jsonTwo = @(
    '{"reason":"compiler-artifact","executable":"C:\\a\\test-1.exe","profile":{"test":true}}',
    '{"reason":"compiler-artifact","executable":"C:\\a\\test-2.exe","profile":{"test":true}}'
)

Check 'cargo-reader-rejects-zero' {
    try { Get-CargoTestExecutable -CargoJsonLines $jsonNone; throw "accepted zero executables" }
    catch { if ($_.Exception.Message -notmatch 'NO_TEST_EXECUTABLE') { throw $_ } }
    "zero -> rejected"
}
Check 'cargo-reader-accepts-exactly-one' {
    $e = Get-CargoTestExecutable -CargoJsonLines $jsonOne
    if ($e -ne 'C:\a\test-1.exe') { throw "returned $e" }
    "one -> accepted"
}
Check 'cargo-reader-rejects-two' {
    try { Get-CargoTestExecutable -CargoJsonLines $jsonTwo; throw "picked one out of two" }
    catch { if ($_.Exception.Message -notmatch 'AMBIGUOUS_TEST_EXECUTABLE') { throw $_ } }
    "two -> rejected as ambiguous"
}

Write-Host "=== metatests: break it on purpose, and require red ==="

# A self-test that cannot fail proves nothing. Each of these reproduces a real bug and requires the
# corresponding check to catch it.
Check 'metatest-truncation-bug-would-be-caught' {
    # This is the exact shape of bug 7. It must produce a null code -- proving the check has teeth.
    $f = New-Fixture 'many-lines-2.ps1' '1..30 | ForEach-Object { Write-Output "line $_" }; exit 0'
    $global:LASTEXITCODE = $null
    & $pwshExe -NoProfile -File $f 2>&1 | Select-Object -First 10 | Out-Null
    $broken = $LASTEXITCODE
    if ($null -ne $broken -and $broken -eq 0) {
        throw "the old pattern did NOT lose the exit code here, so this metatest no longer proves anything"
    }
    "the pipeline+Select pattern loses the code, as it did on 2026-07-15"
}

Check 'metatest-unquoted-arguments-really-do-break' {
    # The preflight caught this on its first run, in the helper rather than in a fixture: passing
    # -ArgumentList raw sends `-File C:\Temp\Jose Perez\...\ok.ps1`, PowerShell splits it at the space,
    # and reports that 'C:\Temp\Jose' has no .ps1 extension. Most usernames contain a space, so this is
    # the common case.
    #
    # This proves the quoting is load-bearing: without it, the same call fails.
    $dir = Join-Path $tmp 'Meta Test\with space'
    $f = New-Fixture 'Meta Test\with space\ok.ps1' 'Write-Output "ran"; exit 0'

    # Deliberately unquoted, the way it was before.
    $o = Join-Path $tmp 'meta-o.txt'; $e = Join-Path $tmp 'meta-e.txt'
    $p = Start-Process -FilePath $pwshExe -ArgumentList @('-NoProfile', '-File', $f) `
        -WorkingDirectory $dir -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $o -RedirectStandardError $e -ErrorAction Stop
    if ($p.ExitCode -eq 0) {
        throw "raw -ArgumentList handled the space correctly, so the quoting protects nothing and this metatest is toothless"
    }
    # And with the quoting, the same call works.
    $r = Invoke-NativeProcess -FilePath $pwshExe -Arguments @('-NoProfile', '-File', $f) `
        -WorkingDirectory $dir -OutputDir $tmp
    if ($r.ExitCode -ne 0) { throw "quoting did not fix it: exit=$($r.ExitCode)" }
    "raw fails with exit=$($p.ExitCode); quoted succeeds"
}

Check 'metatest-hex-conversion-has-teeth' {
    # This metatest was WRONG the first time, and the preflight caught it on its first run -- which is
    # the entire argument for metatests.
    #
    # It asserted that '0x{0:X8}' -f -1073741511 was broken. It is not: PowerShell formats a negative
    # INT32 correctly, and naive and correct both give 0xC0000139. Asserting a falsehood is worse than
    # asserting nothing: it hands out confidence in a check that never checked.
    #
    # Where the naive form really does break is an INT64, which is what arrives if the code is ever read
    # from anywhere other than [int]$process.ExitCode:
    #
    #     '0x{0:X8}' -f [int64]-1073741511   ->   0xFFFFFFFFC0000139   (16 digits, unsearchable)
    #
    # ConvertTo-Win32Hex survives that because its parameter is typed [int], which forces the cast. That
    # type is load-bearing, and this test is what stops someone widening it to [long] for convenience.
    $int32 = '0x{0:X8}' -f -1073741511
    if ($int32 -ne '0xC0000139') { throw "PowerShell changed: int32 now formats as $int32" }

    $naiveInt64 = '0x{0:X8}' -f ([int64]-1073741511)
    if ($naiveInt64 -eq '0xC0000139') {
        throw "an int64 formats correctly on its own, so the [int] cast no longer protects anything and this metatest is toothless"
    }

    $right = ConvertTo-Win32Hex -Code ([int64]-1073741511)
    if ($right -ne '0xC0000139') { throw "the conversion does not survive an int64: got $right" }
    "int64 naively gives $naiveInt64; the conversion still gives $right"
}

Write-Host "=== static analysis ==="

Check 'powershell-parses' {
    $bad = @()
    foreach ($f in Get-ChildItem -Path $here -Include *.ps1, *.psm1 -Recurse) {
        $tokens = $null; $errors = $null
        [System.Management.Automation.Language.Parser]::ParseFile($f.FullName, [ref]$tokens, [ref]$errors) | Out-Null
        if ($errors.Count -gt 0) {
            $bad += "$($f.Name): $($errors[0].Message)"
        }
    }
    if ($bad.Count -gt 0) { throw ($bad -join ' | ') }
    "every .ps1 parses"
}

Check 'script-analyzer-has-no-errors' {
    if (-not (Get-Module -ListAvailable -Name PSScriptAnalyzer)) {
        # The prompt that kills this check is NOT Install-Module's. It is the NuGet provider's.
        #
        # Measured, not guessed: on a machine with PowerShellGet 1.0.0.1 and no NuGet provider,
        # Install-Module stops first to ask permission to install NuGet, and that prompt dies with
        # "ShouldContinue ... Object reference not set to an instance of an object" where nobody can
        # answer. -Confirm:$false on Install-Module never gets a chance to matter, which is why adding
        # more flags to it did nothing.
        #
        # So the provider goes in first, explicitly, and the gallery is trusted before asking for
        # anything from it. Both are no-ops on a runner that already has them.
        if (-not (Get-PackageProvider -ListAvailable -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -eq 'NuGet' })) {
            Install-PackageProvider -Name NuGet -MinimumVersion 2.8.5.201 -Force `
                -Scope CurrentUser -Confirm:$false -ErrorAction Stop | Out-Null
        }
        if (Get-Command Set-PSRepository -ErrorAction SilentlyContinue) {
            Set-PSRepository -Name PSGallery -InstallationPolicy Trusted -ErrorAction SilentlyContinue
        }
        Install-Module PSScriptAnalyzer -Force -Scope CurrentUser -SkipPublisherCheck `
            -AllowClobber -Confirm:$false -ErrorAction Stop
    }
    Import-Module PSScriptAnalyzer -ErrorAction Stop
    # Error severity only. Warnings on a diagnostic tool are noise, and a gate that cries about style
    # is a gate someone switches off -- which is worse than not having it.
    $d = @(Invoke-ScriptAnalyzer -Path $here -Recurse -Severity Error)
    if ($d.Count -gt 0) {
        $d | ForEach-Object { Write-Host "        $($_.ScriptName):$($_.Line) $($_.RuleName)" }
        throw "$($d.Count) blocking diagnostics"
    }
    "no Error-severity findings"
}

# ---------------------------------------------------------------- manifest
$manifest = [ordered]@{
    status           = if ($script:Failed -eq 0) { 'PASS' } else { 'FAIL' }
    generated_utc    = (Get-Date).ToUniversalTime().ToString('o')
    expected_runner  = $ExpectedRunner
    actual_runner    = "$env:ImageOS $env:ImageVersion"
    sha              = "$env:GITHUB_SHA"
    ref              = "$env:GITHUB_REF"
    powershell       = $PSVersionTable.PSVersion.ToString()
    script_hashes    = [ordered]@{}
    checks           = $script:Results
    checks_total     = $script:Results.Count
    checks_failed    = $script:Failed
}
foreach ($f in Get-ChildItem -Path $here -Include *.ps1 -Recurse) {
    $manifest.script_hashes[$f.Name] = (Get-FileHash $f.FullName -Algorithm SHA256).Hash
}
# Written WITHOUT a byte order mark, on purpose.
#
# Set-Content -Encoding UTF8 on Windows PowerShell 5.1 always prepends a BOM (EF BB BF), and every
# standard reader then chokes on it: Python's json.load, jq, and GitHub Actions' fromJSON all fail on
# the first character. This manifest is the contract the long job depends on -- a contract the other
# side cannot parse is not a contract, and the gate would be decoration.
#
# Caught by trying to read the manifest this file had just produced, rather than assuming it was fine
# because the script exited 0.
$json = $manifest | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText(
    [IO.Path]::GetFullPath($ManifestPath),
    $json,
    [Text.UTF8Encoding]::new($false)   # $false = no BOM
)

# The manifest has to be readable by whoever consumes it, and that is not this process. Reading it back
# as raw bytes is the only way to catch a BOM: PowerShell reads its own BOM'd files happily and would
# report success while every other tool fails.
$bytes = [IO.File]::ReadAllBytes([IO.Path]::GetFullPath($ManifestPath))
if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
    Write-Host "  FAIL  manifest-is-machine-readable"
    Write-Host "        the manifest starts with a UTF-8 BOM; json.load, jq and fromJSON all reject it"
    $script:Failed++
}
else {
    try {
        $roundTrip = [Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-Json
        if ($roundTrip.status -ne $manifest.status) { throw "status changed on read-back" }
        Write-Host "  PASS  manifest-is-machine-readable"
    }
    catch {
        Write-Host "  FAIL  manifest-is-machine-readable"
        Write-Host "        $($_.Exception.Message)"
        $script:Failed++
    }
}

Write-Host ""
Write-Host "manifest: $ManifestPath"
Write-Host "$($script:Results.Count) checks, $script:Failed failed -> $(if ($script:Failed -eq 0) { 'PASS' } else { 'FAIL' })"
Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue

if ($script:Failed -gt 0) {
    Write-Host ""
    Write-Host "The long diagnostic does NOT start. Fixing the tool costs two minutes here and eighty there."
    exit 1
}
exit 0
