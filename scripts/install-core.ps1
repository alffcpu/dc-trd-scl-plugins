<#
  Interactive installer for the ZX Spectrum disk plugins on Windows (PowerShell).

  It detects your Double Commander's bitness and installs the matching plugin
  (64-bit zxdisk.wcx64 or 32-bit zxdisk.wcx), registers the trd/scl extensions,
  and - for the "rename" variant - installs the zxdisk CLI and wires a
  Ctrl+Shift+R hotkey that renames a file in place inside an image and refreshes
  the panel. Double Commander ships the Lua 5.1 library, so auto-refresh works
  out of the box with nothing extra to install.

  Everything for the chosen variant goes into one folder, plus a generated
  uninstall.cmd (+ uninstall-core.ps1). Config edits (doublecmd.xml,
  shortcuts.scf) are idempotent, backed up first, and require Double Commander
  to be closed.

  Usage (no admin rights needed):
      powershell -ExecutionPolicy Bypass -File .\install-core.ps1
  or double-click install.cmd.

  Parameters (also used for testing / automation):
      -Lang ru|en           interface language
      -Dir  <path>          install directory
      -Mode basic|rename    install variant
      -Yes                  assume yes / take defaults, no prompts
      -ConfigDir <path>     Double Commander config dir (default the real one);
                            point at a copy to dry-run without touching the live one
      -Help                 show this help
#>
[CmdletBinding()]
param(
  [ValidateSet('ru', 'en')] [string]$Lang,
  [string]$Dir,
  [ValidateSet('basic', 'rename')] [string]$Mode,
  [string]$ConfigDir,
  [switch]$Yes,
  [Alias('h')] [switch]$Help
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

if ($Help) {
  Write-Host @'
ZX Spectrum disk plugins installer for Double Commander (Windows)

Usage (no admin rights needed):
    powershell -ExecutionPolicy Bypass -File .\install-core.ps1
  or double-click install.cmd.

Parameters:
    -Lang ru|en          interface language
    -Dir  <path>         install directory
    -Mode basic|rename   install variant
    -Yes                 assume yes / take defaults, no prompts
    -ConfigDir <path>    Double Commander config dir (default: the real one);
                         point at a copy to dry-run without touching the live one
    -Help                show this help
'@
  exit 0
}

# ---------------------------------------------------------------- constants ---
$Self = Split-Path -Parent $PSCommandPath
$Repo = Split-Path -Parent $Self
$LuaLib = 'lua5.1.dll'                                    # bundled with Double Commander

$AppData = $env:APPDATA
$DefaultConfigDir = Join-Path $AppData 'doublecmd'
$DefaultInstallDir = Join-Path $DefaultConfigDir 'plugins\wcx\zxdisk'
$ReuseConf = Join-Path $AppData 'zxdisk\zxdisk-install.conf'
$PluginConf = Join-Path $AppData 'zxdisk\zxdisk.conf'

if (-not $ConfigDir) { $ConfigDir = $DefaultConfigDir }

# ------------------------------------------------------------ localization ---
function L([string]$ru, [string]$en) { if ($script:LangSel -eq 'ru') { $ru } else { $en } }
$script:LangSel = $Lang

function T([string]$key) {
  switch ($key) {
    'title'        { L 'Установщик плагинов ZX Spectrum для Double Commander' 'ZX Spectrum disk plugins installer for Double Commander' }
    'dc_notice'    { L 'Важно: перед установкой закрой Double Commander (иначе он перезапишет правки конфига при выходе).' 'Important: close Double Commander before installing (otherwise it overwrites the config edits on quit).' }
    'variant_head' { L 'Вариант установки:' 'Install variant:' }
    'variant_1'    { L '1) basic  - только просмотр/извлечение/добавление/удаление (.trd,.scl)' '1) basic  - browse / extract / add / delete only (.trd,.scl)' }
    'variant_2'    { L '2) rename - плюс переименование по Ctrl+Shift+R (CLI + встроенная Lua)' '2) rename - also in-place rename via Ctrl+Shift+R (CLI + bundled Lua)' }
    'variant_ask'  { L 'Выбор 1 или 2' 'Choose 1 or 2' }
    'dir_ask'      { L 'Папка установки' 'Install directory' }
    'dc_still'     { L 'Double Commander всё ещё запущен - закрой его и запусти установщик снова.' 'Double Commander is still running - close it and re-run the installer.' }
    'plan'         { L 'План:' 'Plan:' }
    'p_variant'    { L '  вариант     : ' '  variant     : ' }
    'p_bits'       { L '  разрядность : ' '  bitness     : ' }
    'p_dir'        { L '  папка       : ' '  install dir : ' }
    'p_config'     { L '  конфиг DC   : ' '  config      : ' }
    'proceed'      { L 'Продолжить?' 'Proceed?' }
    'aborted'      { L 'прервано.' 'aborted.' }
    'reg_done'     { L 'прописаны расширения trd, scl -> ' 'registered: trd, scl -> ' }
    'reg_wlx'      { L 'просмотрщик экранов (6912/6144 по размеру) -> ' 'screen viewer (6912/6144 by size) -> ' }
    'hk_done'      { L 'хоткей: Ctrl+Shift+R -> переименование с авто-обновлением (встроенная Lua) через ' 'hotkey: Ctrl+Shift+R -> rename with auto-refresh (bundled Lua) via ' }
    'done_restart' { L 'Готово. Перезапусти Double Commander, чтобы он загрузил плагин.' 'Done. Restart Double Commander so it loads the plugin.' }
    'backups'      { L 'Бэкапы конфига: ' 'Config backups: ' }
    'uninstall'    { L 'Чтобы удалить всё позже, запусти:  ' 'To remove everything later, run:  ' }
    'no_plugin'    { L 'не найден плагин' 'plugin not found' }
    'prev_found'   { L 'Обнаружена предыдущая установка: ' 'A previous installation was found: ' }
    'prev_ask'     { L 'Удалить её перед установкой?' 'Remove it before installing?' }
    'prev_removing'{ L 'Удаляю предыдущую установку...' 'Removing the previous installation...' }
    'prev_failed'  { L 'Не удалось полностью удалить предыдущую установку - продолжаю.' 'Could not fully remove the previous installation - continuing.' }
    'bits_guess'   { L 'Не удалось определить разрядность Double Commander - использую разрядность ОС. Проверь строку "разрядность" в плане.' 'Could not detect Double Commander bitness - assuming the OS bitness. Check the "bitness" line in the plan.' }
    default        { $key }
  }
}

# --------------------------------------------------------------- helpers -----
function Say ([string]$m) { Write-Host $m }
function Ok  ([string]$m) { Write-Host $m -ForegroundColor Green }
function Warn([string]$m) { Write-Host $m -ForegroundColor Yellow }
function Die ([string]$m) { Write-Host ("error: " + $m) -ForegroundColor Red; exit 1 }

function Ask([string]$prompt, [string]$default) {
  if ($Yes) { return $default }
  $a = Read-Host ("{0} [{1}]" -f $prompt, $default)
  if ([string]::IsNullOrEmpty($a)) { $default } else { $a }
}
function Confirm([string]$prompt) {
  if ($Yes) { return $true }
  $a = Read-Host ("{0} [y/N]" -f $prompt)
  return ($a -match '^(y|yes|д|да)$')
}
function ConfirmYes([string]$prompt) {   # default Yes
  if ($Yes) { return $true }
  $a = Read-Host ("{0} [Y/n]" -f $prompt)
  return -not ($a -match '^(n|no|н|нет)$')
}

# Find a bundled binary: next to this script (release folder), in the repo dist,
# or in a dist next to the script.
function Locate-Bin([string]$name) {
  foreach ($c in @((Join-Path $Self $name), (Join-Path $Repo "dist\$name"), (Join-Path $Self "dist\$name"))) {
    if (Test-Path -LiteralPath $c) { return (Resolve-Path -LiteralPath $c).Path }
  }
  return $null
}

# Bitness of the installed Double Commander (falls back to the OS bitness).
function Get-DcExe {
  $cands = New-Object System.Collections.Generic.List[string]
  try {
    $p = Get-Process -Name doublecmd -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($p -and $p.Path) { $cands.Add($p.Path) }
  } catch {}
  foreach ($base in @($env:ProgramW6432, $env:ProgramFiles, ${env:ProgramFiles(x86)})) {
    if ($base) { $cands.Add((Join-Path $base 'Double Commander\doublecmd.exe')) }
  }
  foreach ($c in $cands) { if ($c -and (Test-Path -LiteralPath $c)) { return $c } }
  return $null
}
function Get-PeBitness([string]$exe) {
  try {
    $fs = [IO.File]::OpenRead($exe)
    try {
      $br = New-Object IO.BinaryReader($fs)
      $fs.Position = 0
      if ($br.ReadUInt16() -ne 0x5A4D) { return $null }        # 'MZ' DOS header
      $fs.Position = 0x3C
      $peOff = $br.ReadInt32()
      if ($peOff -le 0 -or $peOff -gt ($fs.Length - 6)) { return $null }
      $fs.Position = $peOff
      if ($br.ReadUInt32() -ne 0x00004550) { return $null }    # 'PE\0\0' signature
      $machine = $br.ReadUInt16()
      if ($machine -eq 0x8664 -or $machine -eq 0xAA64) { return 64 } else { return 32 }
    } finally { $fs.Close() }
  } catch { return $null }
}
function Get-DcBitness {
  $exe = Get-DcExe
  if ($exe) { $b = Get-PeBitness $exe; if ($b) { return $b } }
  # No Double Commander exe found (or unreadable): assume the OS bitness and warn,
  # so the user can catch a mismatch in the plan before proceeding.
  Warn (T 'bits_guess')
  if ([Environment]::Is64BitOperatingSystem) { return 64 } else { return 32 }
}

# -------------------------------------------------------------- XML edits ----
function Load-Xml([string]$path) {
  $doc = New-Object System.Xml.XmlDocument
  $doc.PreserveWhitespace = $false      # normalize + reindent on save (DC re-reads fine)
  $doc.Load($path)
  return $doc
}
function Save-Xml([System.Xml.XmlDocument]$doc, [string]$path) {
  $st = New-Object System.Xml.XmlWriterSettings
  $st.Indent = $true
  $st.IndentChars = '  '
  $st.NewLineChars = "`r`n"
  $st.Encoding = New-Object System.Text.UTF8Encoding($false)   # no BOM, matching DC
  # Write to a sibling temp file then swap it in, so a crash mid-write cannot
  # leave DC's config truncated.
  $tmp = "$path.tmp"
  $w = [System.Xml.XmlWriter]::Create($tmp, $st)
  try { $doc.Save($w) } finally { $w.Close() }
  Move-Item -LiteralPath $tmp -Destination $path -Force
}
function New-Child([System.Xml.XmlDocument]$doc, [System.Xml.XmlElement]$parent, [string]$name, [string]$text) {
  $e = $doc.CreateElement($name)
  if ($null -ne $text) { $e.InnerText = $text }
  [void]$parent.AppendChild($e)
  return $e
}

# Register our WCX plugin for trd + scl, replacing any prior handler for those
# extensions or any entry already pointing at our plugin file (idempotent).
function Register-Wcx([System.Xml.XmlDocument]$doc, [string]$pluginPath) {
  $root = $doc.DocumentElement
  # In current Double Commander the WCX list lives at <doublecmd><Plugins><WcxPlugins>,
  # not directly under the root - find it wherever it is, and create it under
  # <Plugins> (creating that too if need be) when the config has none yet.
  $plugins = $root.SelectSingleNode('.//WcxPlugins')
  if (-not $plugins) {
    $host_ = $root.SelectSingleNode('Plugins')
    if (-not $host_) { $host_ = New-Child $doc $root 'Plugins' $null }
    $plugins = New-Child $doc $host_ 'WcxPlugins' $null
  }

  $remove = @()
  foreach ($n in $plugins.SelectNodes('WcxPlugin')) {
    $ext = $n.SelectSingleNode('ArchiveExt')
    $pth = $n.SelectSingleNode('Path')
    if (($ext -and @('trd', 'scl') -contains $ext.InnerText.ToLower()) -or
        ($pth -and $pth.InnerText -eq $pluginPath)) {
      $remove += $n
    }
  }
  foreach ($n in $remove) { [void]$plugins.RemoveChild($n) }

  foreach ($ext in @('trd', 'scl')) {
    $wp = $doc.CreateElement('WcxPlugin')
    $wp.SetAttribute('Enabled', 'True')
    [void](New-Child $doc $wp 'ArchiveExt' $ext)
    [void](New-Child $doc $wp 'Path' $pluginPath)
    [void](New-Child $doc $wp 'Flags' '79')
    [void]$plugins.AppendChild($wp)
  }
}

# Register the WLX screen viewer under <Plugins><WlxPlugins>, detected by size
# (SIZE==6912|6144). Replaces any prior entry for our plugin or that detect string.
$WLX_DETECT = '(SIZE=6912)|(SIZE=6144)'
function Register-Wlx([System.Xml.XmlDocument]$doc, [string]$pluginPath) {
  $root = $doc.DocumentElement
  $plugins = $root.SelectSingleNode('.//WlxPlugins')
  if (-not $plugins) {
    $host_ = $root.SelectSingleNode('Plugins')
    if (-not $host_) { $host_ = New-Child $doc $root 'Plugins' $null }
    $plugins = New-Child $doc $host_ 'WlxPlugins' $null
  }
  foreach ($n in @($plugins.SelectNodes('WlxPlugin'))) {
    $pth = $n.SelectSingleNode('Path')
    $det = $n.SelectSingleNode('DetectString')
    if (($pth -and $pth.InnerText -eq $pluginPath) -or ($det -and $det.InnerText -eq $WLX_DETECT)) {
      [void]$plugins.RemoveChild($n)
    }
  }
  $wp = $doc.CreateElement('WlxPlugin')
  $wp.SetAttribute('Enabled', 'True')
  [void](New-Child $doc $wp 'Name' 'ZX Screen')
  [void](New-Child $doc $wp 'Path' $pluginPath)
  [void](New-Child $doc $wp 'DetectString' $WLX_DETECT)
  [void]$plugins.AppendChild($wp)
}

# Ensure DC has a Lua library configured; only set it if unset, so a user's own
# choice is respected. On Windows the bundled lua5.1.dll resolves by name.
# Note: the uninstaller intentionally does NOT revert this. We cannot tell whether
# a later DC/user change made 'lua5.1.dll' the deliberate choice, and clearing a
# working Lua path would be more harmful than leaving a harmless default in place.
function Ensure-Lua([System.Xml.XmlDocument]$doc, [string]$lib) {
  $root = $doc.DocumentElement
  $lua = $root.SelectSingleNode('Lua')
  if (-not $lua) { $lua = New-Child $doc $root 'Lua' $null }
  $ptl = $lua.SelectSingleNode('PathToLibrary')
  if (-not $ptl) { $ptl = New-Child $doc $lua 'PathToLibrary' $lib }
  elseif ([string]::IsNullOrWhiteSpace($ptl.InnerText)) { $ptl.InnerText = $lib }
}

# Set (replace) a Main-form hotkey in shortcuts.scf.
function Set-Hotkey([System.Xml.XmlDocument]$doc, [string]$shortcut, [string]$command, [string]$param) {
  $form = $doc.DocumentElement.SelectSingleNode("Hotkeys/Form[@Name='Main']")
  if (-not $form) {
    $hk = $doc.DocumentElement.SelectSingleNode('Hotkeys')
    if (-not $hk) { $hk = New-Child $doc $doc.DocumentElement 'Hotkeys' $null }
    $form = $doc.CreateElement('Form'); $form.SetAttribute('Name', 'Main'); [void]$hk.AppendChild($form)
  }
  $remove = @()
  foreach ($n in $form.SelectNodes('Hotkey')) {
    $sc = $n.SelectSingleNode('Shortcut')
    if ($sc -and $sc.InnerText -eq $shortcut) { $remove += $n }
  }
  foreach ($n in $remove) { [void]$form.RemoveChild($n) }

  $hkn = $doc.CreateElement('Hotkey')
  [void](New-Child $doc $hkn 'Shortcut' $shortcut)
  [void](New-Child $doc $hkn 'Command' $command)
  [void](New-Child $doc $hkn 'Param' $param)
  [void]$form.AppendChild($hkn)
}

# ------------------------------------------------------- generated files -----
# Write a text file as UTF-8 WITHOUT a BOM. Important for zxrename.lua (Lua 5.1
# does not skip a BOM and would raise a syntax error) and keeps the .conf files
# clean; all generated files here are plain ASCII anyway.
function Write-Utf8NoBom([string]$path, [string]$text) {
  [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
}

function Write-LuaScript([string]$luaPath, [string]$cliPath) {
  # The CLI path is baked in. To avoid the console window that os.execute would
  # flash (it runs through cmd.exe), the CLI is launched directly and hidden via
  # the Win32 API using LuaJIT's FFI (Double Commander ships LuaJIT). If FFI is
  # unavailable the script falls back to os.execute, so rename always works.
  $tpl = @'
-- Double Commander hotkey script (Windows): rename the file under the cursor
-- inside a ZX .trd/.scl image (browsed with the WCX plugin), then refresh the
-- panel. Installed copy - the CLI path is baked in by the installer.
--
-- The rename runs the zxdisk CLI. os.execute would go through cmd.exe and flash
-- a console window; instead we launch the CLI directly and HIDDEN via the Win32
-- API using LuaJIT's FFI (Double Commander ships LuaJIT). If FFI is unavailable
-- we fall back to os.execute, so rename still works either way.

local ZXDISK = [[__CLI__]]

local function run_hidden(exe, img, old, new)
  local ok, ffi = pcall(require, 'ffi')
  if not ok then return false end
  pcall(ffi.cdef, [[
    typedef struct {
      unsigned long  cb;
      void* lpReserved; void* lpDesktop; void* lpTitle;
      unsigned long dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
      unsigned short wShowWindow, cbReserved2;
      void* lpReserved2;
      void* hStdInput; void* hStdOutput; void* hStdError;
    } ZXSTARTUPINFOW;
    typedef struct { void* hProcess; void* hThread; unsigned long dwProcessId, dwThreadId; } ZXPROCINFO;
    int MultiByteToWideChar(unsigned int, unsigned long, const char*, int, wchar_t*, int);
    int CreateProcessW(const wchar_t*, wchar_t*, void*, void*, int, unsigned long, void*, const wchar_t*, void*, void*);
    unsigned long WaitForSingleObject(void*, unsigned long);
    int CloseHandle(void*);
  ]])
  local k32 = ffi.load('kernel32')
  local function towide(s)
    local n = k32.MultiByteToWideChar(65001, 0, s, -1, nil, 0)
    if n <= 0 then return nil end
    local buf = ffi.new('wchar_t[?]', n)
    k32.MultiByteToWideChar(65001, 0, s, -1, buf, n)
    return buf
  end
  local cmdline = '"' .. exe .. '" rename "' .. img .. '" "' .. old .. '" "' .. new .. '"'
  local wapp, wcmd = towide(exe), towide(cmdline)
  if not wapp or not wcmd then return false end
  local si = ffi.new('ZXSTARTUPINFOW')
  si.cb = ffi.sizeof('ZXSTARTUPINFOW')
  si.dwFlags = 0x00000001       -- STARTF_USESHOWWINDOW
  si.wShowWindow = 0            -- SW_HIDE
  local pi = ffi.new('ZXPROCINFO')
  local rc = k32.CreateProcessW(wapp, wcmd, nil, nil, 0, 0x08000000, nil, nil, si, pi)  -- CREATE_NO_WINDOW
  if rc == 0 then return false end
  k32.WaitForSingleObject(pi.hProcess, 0xFFFFFFFF)   -- INFINITE
  k32.CloseHandle(pi.hThread)
  k32.CloseHandle(pi.hProcess)
  return true
end

local image = DC.ExpandVar('%"0%A')
local entry = DC.ExpandVar('%"0%f')

if image == '' or entry == '' then
  Dialogs.MessageBox('Stand on a file inside a .trd/.scl image first.', 'ZX rename', 0)
  return
end

local ok, newname = Dialogs.InputQuery('ZX rename', 'New name for ' .. entry .. ':', false, entry)
if ok and newname ~= '' and newname ~= entry then
  local pok, done = pcall(run_hidden, ZXDISK, image, entry, newname)
  if not (pok and done) then
    -- Fallback: cmd.exe route (may briefly flash a console window).
    os.execute('""' .. ZXDISK .. '" rename "' .. image .. '" "' .. entry .. '" "' .. newname .. '""')
  end
  DC.ExecuteCommand('cm_Refresh')
end
'@
  $tpl = $tpl.Replace('__CLI__', $cliPath)
  Write-Utf8NoBom $luaPath $tpl
}

function Write-PluginConf {
  if (Test-Path -LiteralPath $PluginConf) { return }
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $PluginConf) | Out-Null
  $conf = @'
# zxdisk plugin settings - shared by the WCX plugin and the zxdisk CLI.
# Lines are key=value. Edit freely, then restart Double Commander.

# Extension chars shown/parsed after the TR-DOS type byte:
#   single - 1 char (the type byte only)
#   triple - always 3 chars (type + the 2 address bytes as letters)
#   smart  - 3 chars when both address bytes are printable ASCII, else 1
ext_mode=smart

# Geometry for a brand-new .trd created on copy-in:
#   640k (80x2) | 320k-ds (40x2) | 320k-ss (80x1) | 160k (40x1)
new_trd_geometry=640k

# Export files as .$C hobeta (17-byte header) instead of raw sectors: true|false
extract_hobeta=false

# Write a debug log (troubleshooting only): true|false
debug_log=false

# ZX screen viewer (WLX) zoom, 1..6. Also settable live with Shift+1..6.
screen_scale=2

# Border colour: 0..7 for a fixed ZX colour (0 black .. 7 white, no bright), or
# "auto" for the dominant screen colour (the default). Live: Alt+0..7 / Alt+8.
#screen_border_color=auto
'@
  Write-Utf8NoBom $PluginConf $conf
}

function Write-ReuseConf {
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $ReuseConf) | Out-Null
  $lines = @(
    "# zxdisk installer settings (reusable; edit or delete freely)"
    "lang=$script:LangSel"
    "mode=$Mode"
    "install_dir=$InstallDir"
    "config_dir=$ConfigDir"
    "plugin=$PluginDest"
    "cli=$CliDest"
    "lua=$LuaDest"
  ) -join "`r`n"
  Write-Utf8NoBom $ReuseConf ($lines + "`r`n")
  Copy-Item -LiteralPath $ReuseConf -Destination (Join-Path $InstallDir 'zxdisk-install.conf') -Force -ErrorAction SilentlyContinue
}

function PsLit([string]$s) { "'" + ($s -replace "'", "''") + "'" }

function Write-Uninstaller {
  $u = Join-Path $InstallDir 'uninstall-core.ps1'
  # param() must precede the first statement, so it goes above $ErrorActionPreference.
  $header = @(
    'param([switch]$Pause)   # -Pause: wait for a keypress at the end (used by uninstall.cmd)'
    '# Auto-generated uninstaller for the ZX Spectrum disk plugins (Windows).'
    '$ErrorActionPreference = ''Stop'''
    ('$Xml       = ' + (PsLit $Xml))
    ('$Scf       = ' + (PsLit $Scf))
    ('$InstallDir= ' + (PsLit $InstallDir))
    ('$Plugin    = ' + (PsLit $PluginDest))
    ('$Wlx       = ' + (PsLit $WlxDest))
    ('$ReuseConf = ' + (PsLit $ReuseConf))
  ) -join "`r`n"

  $body = @'

function Save-Xml($doc, $path) {
  $st = New-Object System.Xml.XmlWriterSettings
  $st.Indent = $true; $st.IndentChars = '  '; $st.NewLineChars = "`r`n"
  $st.Encoding = New-Object System.Text.UTF8Encoding($false)
  $tmp = "$path.tmp"
  $w = [System.Xml.XmlWriter]::Create($tmp, $st)
  try { $doc.Save($w) } finally { $w.Close() }
  Move-Item -LiteralPath $tmp -Destination $path -Force
}

if (Get-Process -Name doublecmd -ErrorAction SilentlyContinue) {
  Write-Host 'Double Commander is running - close it first, then re-run this uninstaller.' -ForegroundColor Yellow
  exit 1
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
if (Test-Path -LiteralPath $Xml) { Copy-Item -LiteralPath $Xml "$Xml.zxuninstall-$stamp" -Force }
if (Test-Path -LiteralPath $Scf) { Copy-Item -LiteralPath $Scf "$Scf.zxuninstall-$stamp" -Force }

if (Test-Path -LiteralPath $Xml) {
  $doc = New-Object System.Xml.XmlDocument; $doc.PreserveWhitespace = $false; $doc.Load($Xml)
  $wcx = $doc.DocumentElement.SelectSingleNode('.//WcxPlugins')
  if ($wcx) {
    foreach ($n in @($wcx.SelectNodes('WcxPlugin'))) {
      $pth = $n.SelectSingleNode('Path')
      if ($pth -and $pth.InnerText -eq $Plugin) { [void]$wcx.RemoveChild($n) }
    }
  }
  $wlxSec = $doc.DocumentElement.SelectSingleNode('.//WlxPlugins')  # not $wlx: PowerShell vars are case-insensitive and would clash with $Wlx
  if ($wlxSec) {
    foreach ($n in @($wlxSec.SelectNodes('WlxPlugin'))) {
      $pth = $n.SelectSingleNode('Path')
      if ($pth -and $pth.InnerText -eq $Wlx) { [void]$wlxSec.RemoveChild($n) }
    }
  }
  Save-Xml $doc $Xml
}

if (Test-Path -LiteralPath $Scf) {
  $doc = New-Object System.Xml.XmlDocument; $doc.PreserveWhitespace = $false; $doc.Load($Scf)
  $form = $doc.DocumentElement.SelectSingleNode("Hotkeys/Form[@Name='Main']")
  if ($form) {
    foreach ($n in @($form.SelectNodes('Hotkey'))) {
      $sc = $n.SelectSingleNode('Shortcut'); $pm = $n.SelectSingleNode('Param')
      # StartsWith, not -like: an install dir containing [ ] would be treated as a
      # wildcard by -like and the hotkey would never match.
      if ($sc -and $sc.InnerText -eq 'Ctrl+Shift+R' -and $pm -and
          $pm.InnerText.StartsWith($InstallDir, [System.StringComparison]::OrdinalIgnoreCase)) {
        [void]$form.RemoveChild($n)
      }
    }
  }
  Save-Xml $doc $Scf
}

Write-Host "Removing installed files in $InstallDir ..."
# uninstall.cmd first (it may be the batch running us - deleting a running .cmd is
# allowed on Windows), then the rest, then uninstall-core.ps1 (self) last.
foreach ($f in @('zxdisk.wcx64', 'zxdisk.wcx', 'zxdisk.wlx64', 'zxdisk.wlx', 'zxdisk.exe', 'zxrename.lua', 'zxdisk-install.conf', 'uninstall.cmd', 'uninstall-core.ps1')) {
  $p = Join-Path $InstallDir $f
  if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue; Write-Host "  removed $f" }
}
if (Test-Path -LiteralPath $ReuseConf) { Remove-Item -LiteralPath $ReuseConf -Force -ErrorAction SilentlyContinue }

if ((Test-Path -LiteralPath $InstallDir) -and -not (Get-ChildItem -LiteralPath $InstallDir -Force)) {
  Remove-Item -LiteralPath $InstallDir -Force -ErrorAction SilentlyContinue
  Write-Host "  removed empty $InstallDir"
}
Write-Host "Done. Restart Double Commander. (Config backups: *.zxuninstall-$stamp)"
if ($Pause) { [void](Read-Host "`nPress Enter to close") }
'@
  Write-Utf8NoBom $u ($header + $body + "`r`n")

  # One-click uninstaller launcher (double-click, no execution-policy prompt).
  # It cd's out of the install dir first so the folder isn't locked and can be
  # removed, then runs uninstall-core.ps1 with -Pause so the result stays on screen.
  # Built with explicit CRLF (batch files want DOS line endings).
  $cmd = @(
    '@echo off'
    'REM One-click uninstaller for the ZX Spectrum disk plugins. Double-click to run.'
    'cd /d "%~dp0.."'
    'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall-core.ps1" -Pause'
  ) -join "`r`n"
  Write-Utf8NoBom (Join-Path $InstallDir 'uninstall.cmd') ($cmd + "`r`n")
}

# =============================================================== run =========
$Xml = Join-Path $ConfigDir 'doublecmd.xml'
$Scf = Join-Path $ConfigDir 'shortcuts.scf'

# previous choices (reused as defaults)
$prev = @{}
if (Test-Path -LiteralPath $ReuseConf) {
  foreach ($line in Get-Content -LiteralPath $ReuseConf) {
    if ($line -match '^\s*([^#=]+?)\s*=\s*(.*)$') { $prev[$Matches[1]] = $Matches[2] }
  }
}

# language (English default; only -Lang overrides without asking)
if (-not $script:LangSel) {
  if ($Yes) {
    $script:LangSel = 'en'
  } else {
    Say 'Language / Язык:'
    Say '  1) Русский'
    Say '  2) English'
    if ((Ask '1 / 2' '2') -eq '1') { $script:LangSel = 'ru' } else { $script:LangSel = 'en' }
  }
}

Say ''
Say ("== " + (T 'title') + " ==")
Say ''
Warn (T 'dc_notice')
Say ''

# variant
if (-not $Mode) {
  Say (T 'variant_head')
  Say ("  " + (T 'variant_1'))
  Say ("  " + (T 'variant_2'))
  $def = if ($prev['mode'] -eq 'basic') { '1' } else { '2' }
  if ((Ask (T 'variant_ask') $def) -eq '1') { $Mode = 'basic' } else { $Mode = 'rename' }
}

# install dir
if (-not $Dir) {
  $def = if ($prev['install_dir']) { $prev['install_dir'] } else { $DefaultInstallDir }
  $Dir = Ask (T 'dir_ask') $def
}
$InstallDir = $Dir

# pick binaries by DC bitness
$Bits = Get-DcBitness
if ($Bits -eq 64) {
  $pluginSrcName = 'zxdisk.wcx64'; $wlxSrcName = 'zxdisk.wlx64'; $cliSrcName = 'zxdisk-x64.exe'
} else {
  $pluginSrcName = 'zxdisk.wcx';   $wlxSrcName = 'zxdisk.wlx';   $cliSrcName = 'zxdisk-x86.exe'
}
$PluginDest = Join-Path $InstallDir $pluginSrcName
$WlxDest = Join-Path $InstallDir $wlxSrcName
$CliDest = Join-Path $InstallDir 'zxdisk.exe'
$LuaDest = Join-Path $InstallDir 'zxrename.lua'

$PluginSrc = Locate-Bin $pluginSrcName
if (-not $PluginSrc) {
  Die ("$(T 'no_plugin'): $pluginSrcName - build it (scripts\build.sh in Git Bash) or use a release package")
}
# The screen viewer (WLX) is always installed; skip gracefully if an older
# package doesn't carry it.
$WlxSrc = Locate-Bin $wlxSrcName
$CliSrc = $null
if ($Mode -eq 'rename') {
  $CliSrc = Locate-Bin $cliSrcName
  if (-not $CliSrc) { Die ("missing $cliSrcName (CLI) - build it (scripts\build.sh) or use a release package") }
}

# config present?
if (-not (Test-Path -LiteralPath $Xml)) { Die "not found: $Xml  (launch Double Commander once so it creates its config)" }
if (-not (Test-Path -LiteralPath $Scf)) { Die "not found: $Scf  (launch Double Commander once so it creates its config)" }

# DC must be closed when editing the real config. Normalize both paths first so a
# trailing slash / relative form can't slip past the "is this the live config?"
# check and let us edit the running DC's config (which it then overwrites on quit).
$cfgNorm = try { [IO.Path]::GetFullPath($ConfigDir) } catch { $ConfigDir }
$defNorm = try { [IO.Path]::GetFullPath($DefaultConfigDir) } catch { $DefaultConfigDir }
if (($cfgNorm.TrimEnd('\','/') -eq $defNorm.TrimEnd('\','/')) -and
    (Get-Process -Name doublecmd -ErrorAction SilentlyContinue)) {
  Die (T 'dc_still')
}

# ---- offer to remove a previous installation ----
# A prior install leaves an uninstall-core.ps1 in its install dir (its path is
# recorded in the reusable config). If found, offer to run it first for a clean
# slate - this also clears files an old install left in a different folder.
$prevUnins = $null
$prevDir = $prev['install_dir']
if ($prevDir -and (Test-Path -LiteralPath (Join-Path $prevDir 'uninstall-core.ps1'))) {
  $prevUnins = Join-Path $prevDir 'uninstall-core.ps1'
} elseif (Test-Path -LiteralPath (Join-Path $InstallDir 'uninstall-core.ps1')) {
  $prevUnins = Join-Path $InstallDir 'uninstall-core.ps1'; $prevDir = $InstallDir
}
if ($prevUnins) {
  Warn ((T 'prev_found') + $prevDir)
  if (ConfirmYes (T 'prev_ask')) {
    Say (T 'prev_removing')
    # Run in a separate powershell.exe (its 'exit' cannot abort this installer).
    # The call operator passes $prevUnins as a single argument, so a path with
    # spaces (e.g. under "C:\Users\John Smith\...") is handled correctly.
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $prevUnins
    if ($LASTEXITCODE -ne 0) { Warn (T 'prev_failed') }
  }
}

# plan
Say ''
Say (T 'plan')
Say ((T 'p_variant') + $Mode)
Say ((T 'p_bits') + "$Bits-bit -> $pluginSrcName")
Say ((T 'p_dir') + $InstallDir)
Say ((T 'p_config') + $Xml)
Say ''
if (-not (Confirm (T 'proceed'))) { Die (T 'aborted') }

# ---- copy files ----
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
Copy-Item -LiteralPath $Xml "$Xml.zxinstall-$stamp" -Force
Copy-Item -LiteralPath $Scf "$Scf.zxinstall-$stamp" -Force

Copy-Item -LiteralPath $PluginSrc -Destination $PluginDest -Force
Ok "installed: $PluginDest"
if ($WlxSrc) {
  Copy-Item -LiteralPath $WlxSrc -Destination $WlxDest -Force
  Ok "installed: $WlxDest"
}
if ($Mode -eq 'rename') {
  Copy-Item -LiteralPath $CliSrc -Destination $CliDest -Force
  Ok "installed: $CliDest"
  Write-LuaScript $LuaDest $CliDest
  Ok "installed: $LuaDest"
}

# ---- config edits ----
$xmlDoc = Load-Xml $Xml
Register-Wcx $xmlDoc $PluginDest
if ($WlxSrc) { Register-Wlx $xmlDoc $WlxDest }
if ($Mode -eq 'rename') { Ensure-Lua $xmlDoc $LuaLib }
Save-Xml $xmlDoc $Xml
Ok ((T 'reg_done') + $PluginDest)
if ($WlxSrc) { Ok ((T 'reg_wlx') + $WlxDest) }

if ($Mode -eq 'rename') {
  $scfDoc = Load-Xml $Scf
  Set-Hotkey $scfDoc 'Ctrl+Shift+R' 'cm_ExecuteScript' $LuaDest
  Save-Xml $scfDoc $Scf
  Ok ((T 'hk_done') + $LuaDest)
}

Write-Uninstaller
Ok ("installed: " + (Join-Path $InstallDir 'uninstall.cmd') + " (+ uninstall-core.ps1)")
Write-ReuseConf
Ok "settings saved: $ReuseConf"
$hadConf = Test-Path -LiteralPath $PluginConf
Write-PluginConf
if ($hadConf) { Ok "plugin settings kept: $PluginConf" } else { Ok "plugin settings: $PluginConf" }

# ---- summary ----
Say ''
Ok (T 'done_restart')
Say ((T 'backups') + "*.zxinstall-$stamp")
Say ''
Say ((T 'uninstall') + '"' + (Join-Path $InstallDir 'uninstall.cmd') + '"')

if (-not $Yes) { [void](Read-Host "`nPress Enter to close") }
