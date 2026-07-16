# Find the one artifact THIS job built, or fail loudly.
#
# WHY THIS FILE EXISTS AT ALL
# ---------------------------
# This function used to be pasted into the diagnostic workflow twice: once in the baseline half and once
# in the candidate half. They were supposed to be identical. They were not. Someone -- me -- improved the
# baseline's error messages and left the candidate's alone, and one of the edits left "$filter:" inside a
# double-quoted string. In PowerShell a colon after a variable name is a scope qualifier, the way $env:PATH
# works, so "$filter:" asks for the variable "filter:" in an unnamed scope and the whole step dies at parse
# time, before its first line runs.
#
# The colon was the symptom. The defect was that the diagnostic had two copies of its own tool and no way
# to notice they had drifted. A comparison whose two halves run different code compares nothing, and it had
# already burned an hour-long build to say so.
#
# So: one copy, dot-sourced by both halves from outside the workspace. They cannot drift because there is
# nothing to drift from.
#
# WHAT IT GUARDS
# --------------
# Three ways a staged binary lies about being this build's:
#   - it does not exist          -> the build silently produced nothing
#   - there are several          -> we cannot tell which one cargo will get
#   - it predates the job start  -> it is a leftover, cached or restored, not built here
# The last one is the one that matters: a stale bitcoind.exe from a previous run makes a broken build look
# healthy, which is precisely the failure this diagnostic exists to find.

Set-StrictMode -Version Latest

function Get-BuiltArtifact {
    <#
    .SYNOPSIS
        Return the single artifact matching $Filter under $Path that this job built. Throw otherwise.
    .PARAMETER JobStartUtc
        ISO-8601 instant the job began. Anything older than this was not built here. Passed explicitly
        rather than read from $env: so the function is testable without touching the environment.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Filter,
        [scriptblock]$Where,
        [Parameter(Mandatory)][string]$JobStartUtc
    )

    $all = @(Get-ChildItem -Path $Path -Recurse -Filter $Filter -ErrorAction SilentlyContinue)
    if ($Where) { $all = @($all | Where-Object $Where) }

    # ${Filter} and not $Filter, every time a colon follows. See the note at the top of this file.
    if ($all.Count -eq 0) {
        throw "FAIL: no ${Filter} was built by this job."
    }
    if ($all.Count -gt 1) {
        throw ("FAIL: {0} copies of {1}; cannot tell which is this build's: {2}" -f `
               $all.Count, $Filter, ($all.FullName -join ', '))
    }

    $start = [datetime]::Parse($JobStartUtc, [cultureinfo]::InvariantCulture,
                               [System.Globalization.DateTimeStyles]::AdjustToUniversal)
    if ($all[0].LastWriteTimeUtc -lt $start) {
        throw ("FAIL: {0} predates the job start ({1:o} < {2:o}): this build did not produce it." -f `
               $Filter, $all[0].LastWriteTimeUtc, $start)
    }
    return $all[0]
}
