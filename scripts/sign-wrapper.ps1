# Tauri signCommand wrapper for SSL.com eSigner (CodeSignTool).
#
# Tauri calls this once per file it is about to bundle, passing the file path as the only argument.
# We must sign ONLY the app binary (brisvia-miner.exe) and the NSIS installer. The three sidecars
# (bitcoind.exe, bitcoin-cli.exe, brisvia-worker.exe) must NOT be signed: the build has a guard that
# extracts bitcoind.exe from the finished installer and compares its SHA-256 to the one compiled in
# this job; signing it would change its bytes and break that provenance guard forever.
#
# Safety rules:
#   - Skip the three sidecars by exact name (keep their hash).
#   - brisvia-miner.exe is signed BEFORE bundling in a separate step; if it already carries a valid
#     Authenticode signature, skip it here (do not double-sign).
#   - Sign the NSIS installer (*-setup.exe).
#   - FAIL on any unexpected name instead of signing it blindly or ignoring it.
#   - Sign into a temp dir, verify the result, then replace the original atomically.
#
# Env required for real signing (set from GitHub secrets, never printed):
#   ES_USERNAME, ES_PASSWORD, ES_CREDENTIAL_ID, ES_TOTP_SECRET, CODESIGNTOOL_HOME
# Dry-run mode (no signing, just record what Tauri asks to sign):
#   SIGN_DRYRUN=1 and SIGN_DRYRUN_LOG=<path>

param([Parameter(Mandatory = $true)][string]$FilePath)
$ErrorActionPreference = 'Stop'

$name = Split-Path $FilePath -Leaf
$sidecars = @('bitcoind.exe', 'bitcoin-cli.exe', 'brisvia-worker.exe')

# Log to a file that survives even a failed bundle. Tauri does NOT surface this script's output when the
# sign command fails (it only prints "failed to run powershell"), so without this the real error is invisible.
# An always() CI step prints this file after the build.
$log = if ($env:RUNNER_TEMP) { Join-Path $env:RUNNER_TEMP 'sign-wrapper.log' } else { Join-Path ([IO.Path]::GetTempPath()) 'sign-wrapper.log' }
function Log($m) { try { Add-Content -Path $log -Value $m } catch { } }
Log "--- called for: $name  (FilePath=$FilePath) ---"
# Any terminating error jumps here: record the exact line + message before dying, then re-throw. Tauri only
# shows "failed to run powershell", so this is the only way to see WHY the wrapper aborted in CI.
trap { Log ("TRAP line $($_.InvocationInfo.ScriptLineNumber): $($_.Exception.Message)"); break }
Log ("env seen -> CST=" + [bool]$env:CODESIGNTOOL_HOME + " USER=" + [bool]$env:ES_USERNAME + " PASS=" + [bool]$env:ES_PASSWORD + " CID=" + [bool]$env:ES_CREDENTIAL_ID + " TOTP=" + [bool]$env:ES_TOTP_SECRET)

# Dry-run: only log the path Tauri handed us, so we can see exactly what the pinned Tauri version
# tries to sign before we ever spend a signing operation.
if ($env:SIGN_DRYRUN -eq '1') {
    if ($env:SIGN_DRYRUN_LOG) { Add-Content -Path $env:SIGN_DRYRUN_LOG -Value $FilePath }
    Write-Host "dry-run, would consider: $name"
    exit 0
}

# Do not sign: the three node sidecars (their hash must stay identical for the packaged-node guard),
# the NSIS plugin DLLs and NSIS temp files. The dry-run showed Tauri also hands these to signCommand,
# but only the app binary and the final installer need an Authenticode signature; NSIS plugins live
# inside the installer and Windows never checks them on their own.
if ($sidecars -contains $name -or $name -like '*.dll' -or $name -like '*.tmp') {
    Write-Host "skip (not signed - sidecar / NSIS plugin / temp): $name"
    exit 0
}

# Only the app binary (brisvia-miner.exe) and the NSIS installer (*-setup.exe) get signed here; anything
# else is unexpected. We do NOT call Get-AuthenticodeSignature: in the console-less process Tauri spawns it
# aborts while loading the ACL type data ("member ... is already present"), which surfaces only as Tauri's
# opaque "failed to run powershell". The app binary is signed exactly once during the build (nothing to
# skip), and the dedicated 'Verify Authenticode' step re-checks every signature afterwards in a clean shell.
if ($name -ne 'brisvia-miner.exe' -and $name -notlike '*-setup.exe') {
    throw "sign-wrapper: refusing to sign unexpected file '$name'. If this is legitimate, add it explicitly."
}

# --- Real signing with CodeSignTool, into a temp dir, then atomic replace ---
if (-not $env:CODESIGNTOOL_HOME) { throw 'sign-wrapper: CODESIGNTOOL_HOME is not set' }
# CodeSignTool.bat finds its bundled Java/jar via CODE_SIGN_TOOL_PATH; without it, it uses paths relative
# to the current directory and fails ("The system cannot find the path specified") when called from here.
$env:CODE_SIGN_TOOL_PATH = $env:CODESIGNTOOL_HOME
foreach ($v in @('ES_USERNAME', 'ES_PASSWORD', 'ES_CREDENTIAL_ID', 'ES_TOTP_SECRET')) {
    if (-not (Get-Item "env:$v" -ErrorAction SilentlyContinue)) { throw "sign-wrapper: env $v is not set" }
}

$outDir = Join-Path $env:RUNNER_TEMP ("sign_" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$tool = Join-Path $env:CODESIGNTOOL_HOME 'CodeSignTool.bat'
if (-not (Test-Path $tool)) { throw "sign-wrapper: CodeSignTool.bat not found at $tool" }

# eSigner signs in the cloud; the TOTP secret lets CodeSignTool generate the OTP non-interactively.
# Output goes to a fresh dir so the original is never half-overwritten on failure. Args are not echoed.
# CAPTURE the output (do NOT pipe to Out-Host: in the console-less process Tauri spawns, Out-Host can throw
# and abort the whole sign — surfacing only as Tauri's opaque "failed to run powershell"). Everything is
# written to the log so an always() CI step can show exactly what CodeSignTool said.
Log "signing $name  (CODE_SIGN_TOOL_PATH=$env:CODE_SIGN_TOOL_PATH tool=$tool)"
$signOut = & $tool sign `
    -username="$env:ES_USERNAME" `
    -password="$env:ES_PASSWORD" `
    -credential_id="$env:ES_CREDENTIAL_ID" `
    -totp_secret="$env:ES_TOTP_SECRET" `
    -input_file_path="$FilePath" `
    -output_dir_path="$outDir" 2>&1
$signExit = $LASTEXITCODE
Log "CodeSignTool exit=$signExit"
Log ($signOut | Out-String)
if ($signExit -ne 0) { throw "sign-wrapper: CodeSignTool failed for '$name' (exit $signExit): $($signOut | Out-String)" }

$signed = Join-Path $outDir $name
if (-not (Test-Path $signed)) { throw "sign-wrapper: signed output not found for '$name' in $outDir" }

# Trust CodeSignTool's exit code (checked above) — do NOT verify with Get-AuthenticodeSignature here: in
# Tauri's spawned process it aborts loading the ACL type data. The 'Verify Authenticode' step re-checks
# every signature afterwards in a clean shell (Status=Valid + Subject = Fernando Del Collado).
Move-Item -Force -Path $signed -Destination $FilePath
Remove-Item -Recurse -Force -Path $outDir -ErrorAction SilentlyContinue
Write-Host "signed OK: $name"
