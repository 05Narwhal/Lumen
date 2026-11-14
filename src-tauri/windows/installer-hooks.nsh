; Lumen's hooks into Tauri's NSIS installer (bundle.windows.nsis.installerHooks).

; How long nothing Restart Manager refuses to shut may hold the program before the install goes on,
; and how long the install waits for that at most.
!define LUMEN_RM_CLEAR_MS 2000
!define LUMEN_RM_MAX_MS 30000

; An update started by Lumen passes `/LUMENPID=<its process id>` (updater::installer_args). Tauri's
; running-app check, which comes right after this hook, force-shuts every process using lumen.exe
; through Restart Manager and aborts the install silently if that shutdown fails. It failed in the
; Round 17 e2e on 1 close in 5: just after Lumen exits, Windows' Inventory and Compatibility
; Appraisal service (InventorySvc) reads the program for a moment, Restart Manager will not touch a
; critical service (reboot reason 1, `RmShutdown` 350), and that refuses the whole request.
; So first wait for that one process to end, at most 10 s, then until Restart Manager has reported no
; reboot reason for LUMEN_RM_CLEAR_MS. Other holders, like a running `--mcp` bridge, have no reason
; and stay Tauri's check's to shut. A manual install has no /LUMENPID and skips this.
!macro NSIS_HOOK_PREINSTALL
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  ClearErrors
  ${GetOptions} $CMDLINE "/LUMENPID=" $0
  ${IfNot} ${Errors}
  ${AndIf} $0 != ""
    ; SYNCHRONIZE (0x00100000) is all a wait needs.
    System::Call 'kernel32::OpenProcess(i 0x00100000, i 0, i r0) p .r1'
    ${If} $1 != 0
      System::Call 'kernel32::WaitForSingleObject(p r1, i 10000) i'
      System::Call 'kernel32::CloseHandle(p r1)'
    ${EndIf}
    !insertmacro RestartManager_StartSession $4
    ${If} $4 != ""
      !insertmacro RestartManager_RegisterFile $4 "$INSTDIR\${MAINBINARYNAME}.exe"
      ${If} $0 = 0
        System::Call 'kernel32::GetTickCount() i .r1'
        StrCpy $2 $1
        ${Do}
          System::Call 'kernel32::GetTickCount() i .r3'
          System::Call 'RSTRTMGR::RmGetList(i r4, *i .r5, *i .r6, p 0, *i .r7) i .r0'
          ${If} $7 <> 0
            StrCpy $2 $3
          ${EndIf}
          IntOp $0 $3 - $2
          ${IfThen} $0 >= ${LUMEN_RM_CLEAR_MS} ${|} ${ExitDo} ${|}
          IntOp $0 $3 - $1
          ${IfThen} $0 >= ${LUMEN_RM_MAX_MS} ${|} ${ExitDo} ${|}
          Sleep 100
        ${Loop}
      ${EndIf}
      !insertmacro RestartManager_EndSession $4
    ${EndIf}
  ${EndIf}
  ClearErrors
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

; Round 18 A2: Windows' Installed apps opens Lumen Setup's remove screen instead of the stock
; uninstaller. The template has just written UninstallString (uninstall.exe); this runs after it, on
; every install and update, so an update keeps it so. The stock installer, reinstalling over Lumen,
; appends `/UPDATE`, `/P` and `_?=<dir>` to UninstallString; lumen-setup.exe hands those straight to
; uninstall.exe with no window and waits, so that path behaves as before. No slim Setup in this
; build: the stock entry stays, and a QuietUninstallString left by an earlier build goes.
!macro NSIS_HOOK_POSTINSTALL
  ${If} ${FileExists} "$INSTDIR\lumen-setup.exe"
    WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\lumen-setup.exe$\" --uninstall"
    WriteRegStr SHCTX "${UNINSTKEY}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
  ${Else}
    DeleteRegValue SHCTX "${UNINSTKEY}" "QuietUninstallString"
  ${EndIf}
!macroend
