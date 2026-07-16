; Brisvia installer hooks.
;
; Before copying anything, make sure OUR node is closed -- properly closed, not killed.
;
; What this used to be:
;
;     nsExec::Exec 'taskkill /F /IM brisvia-worker.exe'
;     nsExec::Exec 'taskkill /F /IM bitcoind.exe'
;     Sleep 1000
;
; Two lines, two serious bugs:
;
;   /F kills. Bitcoin Core flushes the chainstate when it shuts down, and its own documentation says
;   that can take minutes. Killing it mid-flush is how a block database ends up half-written. Measured
;   against the published 1.0.5 with its node running: not one line reached debug.log during the
;   update. No "Shutdown in progress...", no "Shutdown done". It was killed, and the user would have
;   found out on the next start -- with a repair, or with a resync from zero.
;
;   /IM matches by name, and `bitcoind.exe` is not our name. It is Bitcoin Core's. Anyone running their
;   own Bitcoin node had it killed by our installer. That is someone else's chain and someone else's
;   money, and we had no business touching it.
;
; The logic lives in a PowerShell script rather than inline: quoting a command that size inside a .nsh
; is the kind of fragility that hides bugs, and a separate script can be read and tested on its own.
;
; If the script cannot close the node cleanly, THE INSTALL ABORTS. Replacing files underneath a live
; node is the other way to corrupt a chain, so "install anyway and hope" is not an option. An installer
; that stops and says why is an annoyance; a corrupted chain is not something the user can undo.

; The script sits next to this hook, and both live under src-tauri/windows. Tauri canonicalizes the
; hook path (dunce::canonicalize) and !includes it by absolute path, so ${__FILEDIR__} is this file's
; real directory -- src-tauri/windows -- not the generated target/release/nsis/x64 that makensis runs
; from. The earlier `File "windows\..."` resolved against that generated directory and failed with
; "no files found". Verified against tauri-bundler 2.11 source. Frozen in a define so it is fixed at
; the point of definition, not re-evaluated wherever the macro is later inserted.
!define BRISVIA_SHUTDOWN_SCRIPT "${__FILEDIR__}\shutdown-brisvia-node.ps1"

; Fail at COMPILE time, with the exact path, if the script is not where we think. A wrong path becomes a
; clear makensis error instead of an installer that silently skips the safety step at a user's machine.
!if /FileExists "${BRISVIA_SHUTDOWN_SCRIPT}"
!else
  !error "Brisvia shutdown script not found at ${BRISVIA_SHUTDOWN_SCRIPT}"
!endif

; $PLUGINSDIR is NSIS's own temp folder: created on start, cleaned up on exit. The script is put there
; so it exists at run time next to the installer.
!macro NSIS_HOOK_PREINSTALL
  InitPluginsDir
  File "/oname=$PLUGINSDIR\shutdown-brisvia-node.ps1" "${BRISVIA_SHUTDOWN_SCRIPT}"

  DetailPrint "Closing the Brisvia node before installing..."
  ; Full path to powershell.exe, not a bare name: never depend on PATH, never risk running a different
  ; powershell.exe that happens to be found first.
  ; $INSTDIR is where our sidecar lives: the script identifies the node by full path, never by name.
  nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\shutdown-brisvia-node.ps1" -InstallDir "$INSTDIR"'
  Pop $0
  ${If} $0 != 0
    DetailPrint "The Brisvia node could not be closed cleanly (exit code $0)."
    ; A non-zero exit level so a silent (/S) install -- how the seven scenarios are tested -- reports the
    ; failure unambiguously instead of looking like a clean finish. 1603 is the conventional installer
    ; "fatal error" code.
    SetErrorLevel 1603
    ; The message is for a person running the installer by hand. In silent mode it would hang waiting for
    ; a click, so it is shown only when not silent; the abort and the error level happen either way.
    IfSilent +2
      MessageBox MB_OK|MB_ICONSTOP "Brisvia is still writing to disk and could not be closed safely.$\r$\n$\r$\nThe installation was stopped to protect your wallet and your copy of the chain.$\r$\n$\r$\nClose Brisvia, wait a few seconds, then run this installer again."
    Abort "Aborted: the node did not close cleanly. Installing now could corrupt the block database."
  ${EndIf}
  DetailPrint "The node is closed. Installing."
!macroend
