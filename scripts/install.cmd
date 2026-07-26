@echo off
REM One-click launcher for the ZX Spectrum disk plugins installer (Double Commander).
REM Double-click this file - no admin rights needed. It runs install-core.ps1 with
REM the PowerShell execution policy bypassed for THIS process only (nothing is
REM changed system-wide). Any arguments are forwarded to install-core.ps1, e.g.:
REM     install.cmd -Mode basic
REM     install.cmd -Yes -Lang en -Dir "C:\somewhere"
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-core.ps1" %*
REM install-core.ps1 pauses itself on success; pause here only if it failed, so
REM the window does not vanish before you can read the error.
if errorlevel 1 pause
endlocal
