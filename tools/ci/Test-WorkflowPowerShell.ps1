# Parse every inline PowerShell block in a workflow file, without running any of it.
#
# WHY
# ---
# The diagnostic workflow died at "CANDIDATE - stage binaries" on a parse error: "$filter:" inside a
# double-quoted string, where the colon reads as a scope qualifier the way $env:PATH does. A parse error
# kills the whole step before its first line, so the failure arrived after an hour of compiling and pointed
# at nothing in particular.
#
# Nothing checked this. The preflight validated the .ps1 files under tools/ci, which are real files a parser
# can be pointed at, and never looked at the PowerShell pasted inside the YAML -- which is most of it.
# This closes that gap: every `shell: pwsh` block gets parsed before anything is dispatched.
#
# A parse error is the cheapest failure in the world to catch and one of the most expensive to catch late.
#
# ${{ }} IS NOT POWERSHELL
# -----------------------
# GitHub substitutes its own expressions before the shell ever sees them. Left alone they are a syntax error
# in every language, so they are replaced with a harmless literal first. Substituting rather than deleting
# keeps `if (${{ x }} -eq 1)` parseable instead of turning it into `if ( -eq 1)`.

[CmdletBinding()]
param(
    # Not mandatory: -SelfTest runs standalone, with no workflow to point at.
    [string]$WorkflowPath,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Test-PowerShellParses {
    <#  Returns $null when it parses, or the first error message when it does not.  #>
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Code)

    $clean = [regex]::Replace($Code, '\$\{\{[^}]*\}\}', "'gha_expression'")
    $parseErrors = $null
    [void][System.Management.Automation.Language.Parser]::ParseInput($clean, [ref]$null, [ref]$parseErrors)
    if ($parseErrors -and $parseErrors.Count -gt 0) {
        return ($parseErrors[0].Message -split "`n")[0]
    }
    return $null
}

function Get-InlinePowerShellBlock {
    <#
      Pull `run: |` blocks whose shell is pwsh out of a workflow, with the step name and the line the block
      starts on. Deliberately a small indentation-aware reader and not a YAML library: this has to run on a
      bare runner before anything is installed, and it only needs to find blocks, not understand YAML.
    #>
    # AllowEmptyString: a workflow has blank lines, and Mandatory rejects an array containing one.
    param([Parameter(Mandatory)][AllowEmptyString()][string[]]$Lines)

    $blocks = @()
    $name = '(unnamed)'
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i] -match '^\s*-?\s*name:\s*(.+?)\s*$') { $name = $Matches[1].Trim("'", '"') }
        if ($Lines[$i] -notmatch '^(\s*)run:\s*\|\s*$') { continue }

        # Read the indent NOW. Every -match below overwrites $Matches, and reading it after the shell loop
        # got the length of the word "pwsh" instead of the indent -- which made the body reader swallow the
        # next step's "- name:" line and report 23 parse errors in blocks that were fine. The one real error
        # was in there too, indistinguishable from the noise.
        $indent = $Matches[1].Length

        # A step is pwsh unless it says otherwise. Look for its shell: in the surrounding step.
        $shell = 'pwsh'
        for ($j = [Math]::Max(0, $i - 8); $j -lt [Math]::Min($Lines.Count, $i + 3); $j++) {
            if ($Lines[$j] -match '^\s*shell:\s*(\S+)') { $shell = $Matches[1] }
        }
        $body = @()
        for ($k = $i + 1; $k -lt $Lines.Count; $k++) {
            $l = $Lines[$k]
            if ($l.Trim() -eq '') { $body += ''; continue }
            $actual = $l.Length - $l.TrimStart().Length
            if ($actual -le $indent) { break }
            $body += $l
        }
        if ($shell -match 'pwsh|powershell') {
            $blocks += [pscustomobject]@{
                Name = $name; Line = $i + 1; Code = ($body -join "`n")
            }
        }
    }
    return $blocks
}

# ---------------------------------------------------------------------------------------------------

if ($SelfTest) {
    # A checker nobody has seen fail is not a checker. These assert it has teeth before it is trusted.
    $failures = 0

    $real = 'function T($path, $filter) { throw "FAIL: copies of $filter: bad" }'
    $r = Test-PowerShellParses -Code $real
    if ($null -eq $r) { Write-Host '  FAIL  metatest: the real bug parses clean; the check is blind'; $failures++ }
    else { Write-Host "  PASS  metatest-catches-the-real-bug  ($r)" }

    $fixed = 'function T($path, $filter) { throw "FAIL: copies of ${filter}: fine" }'
    if ($null -ne (Test-PowerShellParses -Code $fixed)) {
        Write-Host '  FAIL  metatest: the fix does not parse'; $failures++
    } else { Write-Host '  PASS  metatest-the-fix-parses' }

    if ($null -ne (Test-PowerShellParses -Code 'if (${{ matrix.os }} -eq 1) { "x" }')) {
        Write-Host '  FAIL  metatest: a GitHub expression is reported as a syntax error'; $failures++
    } else { Write-Host '  PASS  metatest-github-expressions-are-not-powershell' }

    # @() around the call: one result comes back as a bare object, and .Count on it throws under StrictMode.
    $b = @(Get-InlinePowerShellBlock -Lines @(
        '      - name: a step', '        shell: pwsh', '        run: |', '          Write-Host "hi"',
        '      - name: bash step', '        shell: bash', '        run: |', '          echo hi'))
    if ($b.Count -ne 1) { Write-Host "  FAIL  metatest: found $($b.Count) pwsh blocks, expected 1"; $failures++ }
    elseif ($b[0].Name -ne 'a step') { Write-Host "  FAIL  metatest: named it '$($b[0].Name)'"; $failures++ }
    else { Write-Host '  PASS  metatest-finds-pwsh-blocks-and-skips-bash' }

    # The reader must stop at the next step, not swallow it. This is the false-positive bug, pinned: the
    # body is one line, and if "- name:" leaks in, the block no longer parses.
    $b2 = @(Get-InlinePowerShellBlock -Lines @(
        '      - name: first', '        shell: pwsh', '        run: |', '          Write-Host "one"',
        '      - name: second', '        shell: pwsh', '        run: |', '          Write-Host "two"'))
    if ($b2.Count -ne 2) {
        Write-Host "  FAIL  metatest: found $($b2.Count) blocks, expected 2"; $failures++
    } elseif ($b2[0].Code -match '- name:') {
        Write-Host '  FAIL  metatest: the body reader swallowed the next step'; $failures++
    } elseif ($null -ne (Test-PowerShellParses -Code $b2[0].Code)) {
        Write-Host '  FAIL  metatest: a clean one-line block is reported as a parse error'; $failures++
    } else { Write-Host '  PASS  metatest-body-reader-stops-at-the-next-step' }

    Write-Host ''
    if ($failures) { Write-Host "SELF-TEST FAILED ($failures)"; exit 1 }
    Write-Host 'self-test OK: the check has teeth'
    if (-not $WorkflowPath) { exit 0 }
}

if (-not (Test-Path $WorkflowPath)) { Write-Host "no such workflow: $WorkflowPath"; exit 1 }

# NO TABS. A path written through one escaping layer too many turns C:\tools into C: + TAB + ools, and
# the file still parses -- a TAB is legal PowerShell whitespace. It cost a full Bitcoin Core build once,
# then cost five more builds when I did it again in the same day. Nothing here is ever meant to contain
# one, so a tab inside an inline block is a mangled path until proven otherwise.
$withTabs = @()
$n = 0
foreach ($l in (Get-Content -LiteralPath $WorkflowPath)) {
    $n++
    if ($l -match "`t") { $withTabs += "  line ${n}: " + ($l -replace "`t", '<<<TAB>>>').Trim() }
}
if ($withTabs.Count -gt 0) {
    Write-Host "TAB CHARACTERS in $WorkflowPath"
    $withTabs | ForEach-Object { Write-Host $_ }
    Write-Host ''
    Write-Host "A tab here is almost always a backslash that went through one escape too many:"
    Write-Host '  "C:\tools\x.ps1"  ->  C: + TAB + ools\x.ps1'
    Write-Host "It parses fine and resolves to nothing. Write the file directly instead of through a"
    Write-Host "script that escapes it."
    exit 1
}

$fileLines = Get-Content -LiteralPath $WorkflowPath
$blocks = Get-InlinePowerShellBlock -Lines $fileLines
Write-Host "$WorkflowPath"
Write-Host "  $($blocks.Count) inline PowerShell blocks"
Write-Host ''

$bad = 0
foreach ($b in $blocks) {
    $e = Test-PowerShellParses -Code $b.Code
    if ($e) {
        Write-Host "  PARSE ERROR  line $($b.Line): $($b.Name)"
        Write-Host "               $e"
        $bad++
    }
}

if ($bad) {
    Write-Host ''
    Write-Host "FAIL: $bad of $($blocks.Count) blocks do not parse. They would die before their first line."
    exit 1
}
Write-Host "  all $($blocks.Count) parse."
exit 0
