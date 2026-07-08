; Brisvia installer hooks.
; Before copying files, close the node and the mining engine so their .exe files in \binaries\
; aren't locked by Windows (which caused "Error opening file for writing: brisvia-worker.exe").
!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec 'taskkill /F /IM brisvia-worker.exe'
  nsExec::Exec 'taskkill /F /IM bitcoind.exe'
  Sleep 1000
!macroend
