; yodawg NSIS install hooks.
;
; yodawg launches qemu-system-x86_64.exe from C:\Program Files\qemu (see
; qemu.rs::qemu_dir). If QEMU isn't already installed there, run the bundled
; QEMU setup silently so a fresh machine ends up with a working QEMU.
;
; yodawg's "Open in virt-viewer" button launches remote-viewer.exe, which it
; finds by scanning Program Files for a VirtViewer* dir (qemu.rs::viewer_binary).
; virt-viewer's official downloads are hard to track down, so we bundle the MSI
; and install it silently when no VirtViewer* install is already present.
;
; Both bundled installers (qemu-w64-setup.exe, virt-viewer.msi) ship as
; installer resources, so they land at $INSTDIR during install; we run them,
; then delete them (only needed at install time). This requires an elevated
; install — see installMode "perMachine" in tauri.conf.json — so the silent
; installs can write to Program Files without a second UAC prompt.

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

  ; virt-viewer installs to a version-stamped dir (e.g. "VirtViewer v11.0-256"),
  ; so glob the prefix to decide whether one is already present. FindFirst sets
  ; $1 (handle) and $2 (first matching name, "" if none).
  FindFirst $1 $2 "$PROGRAMFILES64\VirtViewer*"
  FindClose $1
  ${If} $2 == ""
    DetailPrint "virt-viewer not found — installing the bundled virt-viewer..."
    ; /qn = fully silent MSI; /norestart so it never reboots the machine mid-install.
    ExecWait '"$SYSDIR\msiexec.exe" /i "$INSTDIR\virt-viewer.msi" /qn /norestart' $0
    ${If} $0 <> 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "The bundled virt-viewer installer exited with code $0.$\nThe 'Open in virt-viewer' button won't work until virt-viewer is installed."
    ${EndIf}
  ${Else}
    DetailPrint "virt-viewer already installed ($2) — skipping."
  ${EndIf}

  ; The bundled installers are only needed during install; don't leave ~280 MB behind.
  Delete "$INSTDIR\qemu-w64-setup.exe"
  Delete "$INSTDIR\virt-viewer.msi"
!macroend
