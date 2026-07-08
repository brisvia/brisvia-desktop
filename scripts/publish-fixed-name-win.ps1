# Publishes a fixed (versionless) copy of the Windows installer to a GitHub release,
# so the permanent download link keeps working across every version:
#   https://github.com/brisvia/brisvia-desktop/releases/latest/download/Brisvia-Miner-Windows.exe
#
# Run it AFTER `npm run tauri build` and after the release tag exists.
# Usage:  .\scripts\publish-fixed-name-win.ps1 -Tag v0.1.3
param([Parameter(Mandatory = $true)][string]$Tag)

$repo = "brisvia/brisvia-desktop"

# Find the freshly built NSIS installer (name carries the version, e.g. Brisvia.Miner_0.1.3_x64-setup.exe)
$exe = Get-ChildItem -Path "src-tauri\target\release\bundle" -Recurse -Filter "*-setup.exe" -ErrorAction SilentlyContinue |
       Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $exe) {
    Write-Error "Could not find the *-setup.exe installer. Run 'npm run tauri build' first."
    exit 1
}

$fixed = "Brisvia-Miner-Windows.exe"
Copy-Item $exe.FullName $fixed -Force
Write-Host "Copiado '$($exe.Name)' -> $fixed"

gh release upload $Tag --repo $repo $fixed --clobber
if ($LASTEXITCODE -eq 0) {
    Write-Host "OK: subido $fixed al release $Tag"
    Write-Host "Link permanente: https://github.com/$repo/releases/latest/download/$fixed"
} else {
    Write-Error "Upload to GitHub failed."
    exit 1
}
