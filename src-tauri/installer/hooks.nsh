; yodawg NSIS install hooks.
;
; yodawg launches qemu-system-x86_64.exe from C:\Program Files\qemu (see
; qemu.rs::qemu_dir). If QEMU isn't already installed there, run the bundled
; QEMU setup silently so a fresh machine ends up with a working QEMU.
;
; The QEMU setup (qemu-w64-setup.exe) ships as an installer resource, so it
; lands at $INSTDIR during install; we run it, then delete it (it's only needed
; at install time). This requires an elevated install — see installMode
; "perMachine" in tauri.conf.json — so the silent QEMU install can write to
; Program Files without a second UAC prompt.

!include LogicLib.nsh

!macro NSIS_HOOK_POSTINSTALL
  ${IfNot} ${FileExists} "$PROGRAMFILES64\qemu\qemu-system-x86_64.exe"
    DetailPrint "QEMU not found — installing the bundled QEMU (this may take a minute)..."
    ; /S = silent NSIS install; QEMU's installer defaults to $PROGRAMFILES64\qemu.
    ExecWait '"$INSTDIR\qemu-w64-setup.exe" /S' $0
    ${If} $0 <> 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "The bundled QEMU installer exited with code $0.$\nyodawg needs QEMU at C:\Program Files\qemu — you may need to install it manually."
    ${EndIf}
  ${Else}
    DetailPrint "QEMU already installed — skipping."
  ${EndIf}
  ; The bundled setup is only needed during install; don't leave 200 MB behind.
  Delete "$INSTDIR\qemu-w64-setup.exe"
!macroend
