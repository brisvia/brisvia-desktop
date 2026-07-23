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

# The app binary is signed in a dedicated step before bundling. If it is already validly signed, skip.
if ($name -eq 'brisvia-miner.exe') {
    $sig = Get-AuthenticodeSignature -FilePath $FilePath
    if ($sig.Status -eq 'Valid') {
        Write-Host "skip already-signed: $name"
        exit 0
    }
    Write-Host "note: $name is not signed yet; signing it here as a fallback"
}
elseif ($name -notlike '*-setup.exe') {
    # Anything that is neither a sidecar, nor the app binary, nor the NSIS installer is unexpected.
    throw "sign-wrapper: refusing to sign unexpected file '$name'. If this is legitimate, add it explicitly."
}

# --- Real signing with CodeSignTool, into a temp dir, then atomic replace ---
if (-not $env:CODESIGNTOOL_HOME) { throw 'sign-wrapper: CODESIGNTOOL_HOME is not set' }
foreach ($v in @('ES_USERNAME', 'ES_PASSWORD', 'ES_CREDENTIAL_ID', 'ES_TOTP_SECRET')) {
    if (-not (Get-Item "env:$v" -ErrorAction SilentlyContinue)) { throw "sign-wrapper: env $v is not set" }
}

$outDir = Join-Path $env:RUNNER_TEMP ("sign_" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$tool = Join-Path $env:CODESIGNTOOL_HOME 'CodeSignTool.bat'
if (-not (Test-Path $tool)) { throw "sign-wrapper: CodeSignTool.bat not found at $tool" }

# eSigner signs in the cloud; the TOTP secret lets CodeSignTool generate the OTP non-interactively.
# Output goes to a fresh dir so the original is never half-overwritten on failure. Args are not echoed.
& $tool sign `
    -username="$env:ES_USERNAME" `
    -password="$env:ES_PASSWORD" `
    -credential_id="$env:ES_CREDENTIAL_ID" `
    -totp_secret="$env:ES_TOTP_SECRET" `
    -input_file_path="$FilePath" `
    -output_dir_path="$outDir" | Out-Host
if ($LASTEXITCODE -ne 0) { throw "sign-wrapper: CodeSignTool failed for '$name' (exit $LASTEXITCODE)" }

$signed = Join-Path $outDir $name
if (-not (Test-Path $signed)) { throw "sign-wrapper: signed output not found for '$name' in $outDir" }

$sig = Get-AuthenticodeSignature -FilePath $signed
if ($sig.Status -ne 'Valid') { throw "sign-wrapper: signature is not valid for '$name' (status $($sig.Status))" }

Move-Item -Force -Path $signed -Destination $FilePath
Remove-Item -Recurse -Force -Path $outDir -ErrorAction SilentlyContinue
Write-Host "signed OK: $name"
