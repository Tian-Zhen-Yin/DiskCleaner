# Debug Session: entrypoint-not-found

Status: OPEN

Symptom:
- `target\debug\disk-clear-tool.exe` exits with `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`.

Hypotheses:
1. The rebuilt executable is still importing an API by ordinal/name that is not exported by the runtime DLL actually loaded by Windows.
2. Cargo/MSVC is still linking against old Windows SDK import libraries despite SDK 22621 being installed.
3. A stale or wrong DLL is being resolved from the app/current/PATH directories before System32.
4. The generated executable is stale or target cleanup/rebuild did not fully happen after dependency/config changes.
5. A WebView2/VC runtime DLL loaded after process startup is mismatched and terminates with entrypoint-not-found.

Evidence Log:
- New SDK 22621 is installed under `E:\Windows Kits\10\Lib\10.0.22621.0`.
- PE import validation loaded every dependent DLL and checked every imported function with `GetProcAddress`.
- Result: only missing import is `comctl32.dll!TaskDialogIndirect`.
- This points to the executable loading old Common Controls instead of Common Controls v6.

Fix Applied:
- Added `Microsoft.Windows.Common-Controls` v6 dependency to `src-tauri/app.manifest`.

Next Verification:
- Rebuild after manifest change and rerun `npm run tauri dev` from an administrator PowerShell.
