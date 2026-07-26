#!/usr/bin/env bash
#
# Interactive installer for the ZX Spectrum disk plugins (console script).
# Works on macOS and Linux; it detects the OS and uses the right config paths,
# Lua library and Double Commander process name.
#
# Asks the language (Russian / English), which variant to install, and where:
#   basic   - just the WCX plugin: browse/extract/add/delete .trd/.scl images.
#   rename  - also in-place rename via Ctrl+Shift+R (installs the CLI and a Lua
#             hotkey script; auto-refresh needs LuaJIT, see the note it prints).
#
# Everything for the chosen variant goes into one folder, plus a generated
# uninstall.sh. The chosen settings are saved to a reusable config
# (~/.config/zxdisk-install.conf) and pre-filled on the next run; the uninstaller
# reads it too. Config edits (doublecmd.xml, shortcuts.scf) are idempotent,
# backed up first, and require Double Commander to be closed.
#
# Flags (also used for testing / automation):
#   --lang ru|en        interface language
#   --dir PATH          install directory
#   --mode basic|rename install variant
#   --yes               assume yes / take defaults, no prompts
#   --config-dir PATH   Double Commander config dir (default the real one);
#                       point at a copy to dry-run without touching the live one
#   -h | --help         this help
set -euo pipefail

SELF="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$SELF/.." && pwd)"
DIST="$REPO/dist"

# colors, disabled when stdout is not a terminal, TERM=dumb, or NO_COLOR is set
if [ -t 1 ] && [ "${TERM:-}" != dumb ] && [ -z "${NO_COLOR:-}" ]; then
  C_RESET=$'\033[0m'; C_BOLD=$'\033[1m'; C_RED=$'\033[31m'
  C_GRN=$'\033[32m'; C_YEL=$'\033[33m'; C_CYN=$'\033[36m'
else
  C_RESET=; C_BOLD=; C_RED=; C_GRN=; C_YEL=; C_CYN=
fi

# find a built binary in the repo dist/, next to this script (flat release
# folder), or in a dist/ next to it - so the same installer works from the repo
# and from a packaged release.
locate_bin() { # $1 filename -> echoes path, returns 1 if not found
  local c
  for c in "$DIST/$1" "$SELF/$1" "$SELF/dist/$1"; do
    [ -f "$c" ] && { echo "$c"; return 0; }
  done
  return 1
}

REUSE_CONF="$HOME/.config/zxdisk-install.conf"
PLUGIN_CONF="$HOME/.config/zxdisk.conf"
TOOLBAR_GUID="{A17E5C10-DC01-4E5A-9F00-5A78C0FFEE01}"
LUAJIT_ARM="/opt/homebrew/opt/luajit/lib/libluajit-5.1.dylib"
LUAJIT_INTEL="/usr/local/opt/luajit/lib/libluajit-5.1.dylib"

# ---------- platform ----------
case "$(uname -s)" in
  Darwin) OS=mac ;;
  Linux)  OS=linux ;;
  *) printf 'error: unsupported OS: %s (macOS and Linux only)\n' "$(uname -s)" >&2; exit 1 ;;
esac
if [ "$OS" = mac ]; then
  DEFAULT_CONFIG_DIR="$HOME/Library/Preferences/doublecmd"
  DEFAULT_INSTALL_DIR="$HOME/Library/Application Support/doublecmd/plugins/wcx/zxdisk"
  DC_MATCH='Double Commander.app/Contents/MacOS/doublecmd'
  LUA_DEFAULT_NAME='liblua5.1.dylib'
  LUA_INSTALL_HINT='brew install luajit'
else
  DEFAULT_CONFIG_DIR="$HOME/.config/doublecmd"
  DEFAULT_INSTALL_DIR="$HOME/.config/doublecmd/plugins/wcx/zxdisk"
  # Anchor the binary name (pgrep -f matches the WHOLE command line as an ERE):
  # a bare 'doublecmd' would also match this very installer when --dir/--config-dir
  # point at ~/.config/doublecmd/..., killing the install with a false
  # "Double Commander is running".
  DC_MATCH='(^|/)doublecmd( |$)'
  LUA_DEFAULT_NAME='liblua5.1.so'
  LUA_INSTALL_HINT='sudo apt install luajit   # or: dnf install luajit / pacman -S luajit'
fi

INSTALL_DIR=""
MODE=""
LANG_SEL=""
ASSUME_YES=0
CONFIG_DIR="$DEFAULT_CONFIG_DIR"

# ---------- localization ----------
# L <russian> <english> -> prints the one for the current language (no newline)
L() { if [ "$LANG_SEL" = ru ]; then printf '%s' "$1"; else printf '%s' "$2"; fi; }
t() {
  case "$1" in
    title)        L "Установщик плагинов ZX Spectrum для Double Commander" "ZX Spectrum disk plugins installer for Double Commander" ;;
    dc_notice)    L "Важно: перед установкой закрой Double Commander (иначе он перезапишет правки конфига при выходе)." "Important: close Double Commander before installing (otherwise it overwrites the config edits on quit)." ;;
    variant_head) L "Вариант установки:" "Install variant:" ;;
    variant_1)    L "1) basic  - только просмотр/извлечение/добавление/удаление (.trd,.scl)" "1) basic  - browse / extract / add / delete only (.trd,.scl)" ;;
    variant_2)    L "2) rename - плюс переименование по Ctrl+Shift+R (CLI + Lua)" "2) rename - also in-place rename via Ctrl+Shift+R (CLI + Lua)" ;;
    variant_ask)  L "Выбор 1 или 2" "Choose 1 or 2" ;;
    dir_ask)      L "Папка установки" "Install directory" ;;
    dc_run)       L "Double Commander запущен, его надо закрыть для правки конфига. Закрыть сейчас?" "Double Commander is running and must be closed to edit its config. Quit it now?" ;;
    dc_still)     L "Double Commander всё ещё запущен - закрой и запусти снова." "Double Commander is still running - quit it and re-run." ;;
    close_rerun)  L "Закрой Double Commander и запусти снова." "Close Double Commander and re-run." ;;
    build_ask)    L "dist/zxdisk.wcx не найден - собрать сейчас через scripts/build.sh?" "dist/zxdisk.wcx missing - build now with scripts/build.sh?" ;;
    plan)         L "План:" "Plan:" ;;
    p_variant)    L "  вариант     : " "  variant     : " ;;
    p_dir)        L "  папка       : " "  install dir : " ;;
    p_config)     L "  конфиг DC   : " "  config      : " ;;
    p_luajit)     L "  lua 5.1     : " "  lua 5.1     : " ;;
    luajit_no)    L "не найдена (для авто-обновления нужна Lua 5.1, см. заметку ниже)" "not found (auto-refresh needs a Lua 5.1 lib, see note below)" ;;
    lua_ask)      L "LuaJIT не найден. Установить его сейчас через 'brew install luajit' (нужно для авто-обновления после переименования)?" "LuaJIT not found. Install it now with 'brew install luajit' (needed for auto-refresh after rename)?" ;;
    lua_installing) L "Устанавливаю LuaJIT через Homebrew..." "Installing LuaJIT via Homebrew..." ;;
    lua_failed)   L "Не удалось установить LuaJIT - продолжаю без авто-обновления." "Could not install LuaJIT - continuing without auto-refresh." ;;
    lua_pending)  L "(ещё не установлен - см. заметку ниже)" "(not installed yet - see note below)" ;;
    hk_pending)   L "хоткей: Ctrl+Shift+R -> Lua (заработает, как только установишь Lua 5.1 и перезапустишь DC); пока переименовывай тулбар-кнопкой" "hotkey: Ctrl+Shift+R -> Lua (works once you install a Lua 5.1 lib and restart DC); meanwhile rename with the toolbar button" ;;
    proceed)      L "Продолжить?" "Proceed?" ;;
    aborted)      L "прервано." "aborted." ;;
    reg_done)     L "прописаны расширения trd, scl -> " "registered: trd, scl -> " ;;
    hk_lua)       L "хоткей: Ctrl+Shift+R -> Lua-переименование (авто-обновление) через " "hotkey: Ctrl+Shift+R -> Lua rename (auto-refresh) via " ;;
    hk_cli)       L "хоткей: Ctrl+Shift+R -> переименование через CLI (обновление вручную; LuaJIT пока нет)" "hotkey: Ctrl+Shift+R -> CLI rename (manual refresh; no LuaJIT yet)" ;;
    done_restart) L "Готово. Перезапусти Double Commander, чтобы он перечитал плагин." "Done. Restart Double Commander so it reloads the plugin." ;;
    backups)      L "Бэкапы конфига: " "Config backups: " ;;
    note_head)    L "ЗАМЕЧАНИЕ - авто-обновление списка после переименования требует LuaJIT (Lua 5.1). Установи так:" "NOTE - auto-refresh after rename needs LuaJIT (Lua 5.1). Install it with:" ;;
    note_body)    L "После установки просто перезапусти Double Commander - авто-обновление заработает само, повторно запускать установщик НЕ нужно. Пока Lua нет, переименовывай тулбар-кнопкой (список обновляй Ctrl+R)." "After installing it, just restart Double Commander - auto-refresh will then work; no need to re-run this installer. Until then, rename with the toolbar button (refresh the list with Ctrl+R)." ;;
    uninstall)    L "Чтобы удалить всё позже, запусти:  " "To remove everything later, run:  " ;;
    prev_found)   L "Обнаружена предыдущая установка: " "A previous installation was found: " ;;
    prev_ask)     L "Удалить её перед установкой?" "Remove it before installing?" ;;
    prev_removing) L "Удаляю предыдущую установку..." "Removing the previous installation..." ;;
    prev_failed)  L "Не удалось полностью удалить предыдущую установку - продолжаю." "Could not fully remove the previous installation - continuing." ;;
  esac
}

# ---------- small helpers ----------
say()  { printf '%s\n' "$*"; }
ok()   { printf '%s%s%s\n' "$C_GRN" "$*" "$C_RESET"; }
warn() { printf '%s%s%s\n' "$C_YEL" "$*" "$C_RESET"; }
die()  { printf '%serror:%s %s\n' "$C_RED" "$C_RESET" "$*" >&2; exit 1; }

ask() { # $1 prompt  $2 default  -> echoes the answer
  local a
  if [ "$ASSUME_YES" = 1 ]; then echo "$2"; return; fi
  read -r -p "$1 [$2]: " a </dev/tty || true
  echo "${a:-$2}"
}
confirm() { # $1 prompt  -> 0 yes / 1 no  (default no)
  if [ "$ASSUME_YES" = 1 ]; then return 0; fi
  local a
  read -r -p "$1 [y/N]: " a </dev/tty || true
  case "$a" in y|Y|yes|YES|д|Д|да) return 0 ;; *) return 1 ;; esac
}
confirm_y() { # $1 prompt  -> 0 yes / 1 no  (default yes)
  if [ "$ASSUME_YES" = 1 ]; then return 0; fi
  local a
  read -r -p "$1 [Y/n]: " a </dev/tty || true
  case "$a" in n|N|no|NO|н|Н|нет) return 1 ;; *) return 0 ;; esac
}
usage() { awk 'NR>1 && /^set -euo/{exit} NR>1{sub(/^# ?/,"");print}' "$0"; exit 0; }

# ---------- flags ----------
while [ $# -gt 0 ]; do
  case "$1" in
    --lang)       LANG_SEL="${2:-}"; shift 2 ;;
    --dir)        INSTALL_DIR="${2:-}"; shift 2 ;;
    --mode)       MODE="${2:-}"; shift 2 ;;
    --config-dir) CONFIG_DIR="${2:-}"; shift 2 ;;
    --yes|-y)     ASSUME_YES=1; shift ;;
    -h|--help)    usage ;;
    *) die "unknown option: $1" ;;
  esac
done

XML="$CONFIG_DIR/doublecmd.xml"
SCF="$CONFIG_DIR/shortcuts.scf"

# ---------- reusable config: load previous choices as defaults ----------
conf_get() { sed -n "s/^$1=//p" "$REUSE_CONF" 2>/dev/null | tail -1; }
PREV_DIR="";  PREV_MODE="";  PREV_LANG=""
if [ -f "$REUSE_CONF" ]; then
  PREV_DIR="$(conf_get install_dir)"
  PREV_MODE="$(conf_get mode)"
  PREV_LANG="$(conf_get lang)"
fi

# ---------- lua 5.1 detection (luajit preferred, then system liblua5.1) ----------
detect_lua() { # echoes a usable Lua 5.1 shared lib path if found, else nothing
  local p pfx l
  if [ "$OS" = mac ]; then
    for p in "$LUAJIT_ARM" "$LUAJIT_INTEL"; do
      [ -e "$p" ] && { echo "$p"; return 0; }
    done
    if command -v brew >/dev/null 2>&1; then
      pfx="$(brew --prefix luajit 2>/dev/null || true)"
      [ -n "$pfx" ] && [ -e "$pfx/lib/libluajit-5.1.dylib" ] && echo "$pfx/lib/libluajit-5.1.dylib"
    fi
  else
    for p in \
      /usr/lib/x86_64-linux-gnu/libluajit-5.1.so.2 \
      /usr/lib/aarch64-linux-gnu/libluajit-5.1.so.2 \
      /usr/lib64/libluajit-5.1.so.2 \
      /usr/lib/libluajit-5.1.so.2 \
      /usr/local/lib/libluajit-5.1.so.2 \
      /usr/lib/x86_64-linux-gnu/liblua5.1.so.0 \
      /usr/lib/aarch64-linux-gnu/liblua5.1.so.0 \
      /usr/lib64/liblua5.1.so.0 \
      /usr/lib/liblua5.1.so.0 ; do
      [ -e "$p" ] && { echo "$p"; return 0; }
    done
    if command -v ldconfig >/dev/null 2>&1; then
      l="$(ldconfig -p 2>/dev/null | grep -m1 -E 'libluajit-5\.1\.so' | sed -n 's/.*=> //p')"
      [ -z "$l" ] && l="$(ldconfig -p 2>/dev/null | grep -m1 -E 'liblua5\.1\.so' | sed -n 's/.*=> //p')"
      [ -n "$l" ] && echo "$l"
    fi
  fi
  return 0
}
# canonical path a Lua 5.1 lib WILL live at once installed, so we can point DC at
# it up front - then installing the lib + restarting DC is enough, no re-run.
expected_lua() {
  if [ "$OS" = mac ]; then
    case "$(uname -m)" in arm64) echo "$LUAJIT_ARM" ;; *) echo "$LUAJIT_INTEL" ;; esac
  else
    echo "libluajit-5.1.so.2"   # resolved via the dynamic linker once installed
  fi
}

# ---------- perl-based, idempotent config edits ----------
ensure_wcx_section() {
  grep -q '</WcxPlugins>' "$XML" && return 0
  if grep -q '<WcxPlugins/>' "$XML"; then
    perl -0777 -i -pe 's{<WcxPlugins/>}{<WcxPlugins>\n    </WcxPlugins>}s' "$XML"
  else
    perl -0777 -i -pe 's{(\s*</doublecmd>)}{"\n    <WcxPlugins>\n    </WcxPlugins>".$1}se' "$XML"
  fi
}
wcx_register() { # $1 ext  (remove any handler for this ext, then add ours - one per ext)
  EXT="$1" perl -0777 -i -pe '
    my $ext=quotemeta $ENV{EXT};
    s{\s*<WcxPlugin[^>]*>(?:(?!</WcxPlugin>).)*?<ArchiveExt>$ext</ArchiveExt>(?:(?!</WcxPlugin>).)*?</WcxPlugin>}{}gs;
  ' "$XML"
  EXT="$1" WCX="$WCX" perl -0777 -i -pe '
    my $b="      <WcxPlugin Enabled=\"True\">\n        <ArchiveExt>$ENV{EXT}</ArchiveExt>\n        <Path>$ENV{WCX}</Path>\n        <Flags>79</Flags>\n      </WcxPlugin>\n";
    s{(\s*</WcxPlugins>)}{"\n".$b.$1}se;
  ' "$XML"
}
ensure_wlx_section() {
  grep -q '</WlxPlugins>' "$XML" && return 0
  if grep -q '<WlxPlugins/>' "$XML"; then
    perl -0777 -i -pe 's{<WlxPlugins/>}{<WlxPlugins>\n    </WlxPlugins>}s' "$XML"
  elif grep -q '</Plugins>' "$XML"; then
    # WlxPlugins lives inside <Plugins> (that is where DC's loader reads it), so
    # create it there - not as a stray child of the root.
    perl -0777 -i -pe 's{(\s*</Plugins>)}{"\n      <WlxPlugins>\n      </WlxPlugins>".$1}se' "$XML"
  else
    perl -0777 -i -pe 's{(\s*</doublecmd>)}{"\n    <Plugins>\n      <WlxPlugins>\n      </WlxPlugins>\n    </Plugins>".$1}se' "$XML"
  fi
}
wlx_register() { # remove our old entry, then add ours FIRST in the list
  # DC's viewer tries WLX plugins in list order and takes the first whose detect
  # string matches; a catch-all viewer (e.g. MacPreview, DetectString (EXT!="")))
  # would otherwise grab our .scr, so we insert ours at the top of <WlxPlugins>.
  # Note: <WlxPlugin(?:\s[^>]*)?> so this never matches the container <WlxPlugins>
  # tag (a plain [^>]* would, via the trailing 's', and eat it - fatal when our
  # entry is first in the list, with no </WlxPlugin> between it and the container).
  WLX="$WLX" perl -0777 -i -pe '
    my $wlx=quotemeta $ENV{WLX};
    s{\s*<WlxPlugin(?:\s[^>]*)?>(?:(?!</WlxPlugin>).)*?<Path>$wlx</Path>(?:(?!</WlxPlugin>).)*?</WlxPlugin>}{}gs;
  ' "$XML"
  WLX="$WLX" DETECT='(SIZE=6912)|(SIZE=6144)' perl -0777 -i -pe '
    my $b="      <WlxPlugin Enabled=\"True\">\n        <Name>ZX Screen</Name>\n        <Path>$ENV{WLX}</Path>\n        <DetectString>$ENV{DETECT}</DetectString>\n      </WlxPlugin>\n";
    s{(<WlxPlugins>)}{$1."\n".$b}se;
  ' "$XML"
}
set_lua_path() { # $1 dylib path
  if grep -q '<PathToLibrary>' "$XML"; then
    LUAJIT="$1" perl -0777 -i -pe 's{<PathToLibrary>.*?</PathToLibrary>}{"<PathToLibrary>$ENV{LUAJIT}</PathToLibrary>"}se' "$XML"
  else
    LUAJIT="$1" perl -0777 -i -pe 's{(\s*</doublecmd>)}{"\n  <Lua>\n    <PathToLibrary>$ENV{LUAJIT}</PathToLibrary>\n  </Lua>".$1}se' "$XML"
  fi
}
toolbar_add() { # $1 cli path  (remove our old button by GUID, then add fresh)
  GUID="$TOOLBAR_GUID" perl -0777 -i -pe '
    my $g=quotemeta $ENV{GUID};
    s{\s*<Program>(?:(?!</Program>).)*?$g(?:(?!</Program>).)*?</Program>}{}gs;
  ' "$XML"
  GUID="$TOOLBAR_GUID" CLI="$1" perl -0777 -i -pe '
    my $b="        <Program>\n          <ID>$ENV{GUID}</ID>\n          <Icon>cm_rename</Icon>\n          <Command>$ENV{CLI}</Command>\n          <Params>rename %A %f %[New name for ZX file;%f]</Params>\n        </Program>\n";
    s{(\s*</Row>\s*</MainToolbar>)}{"\n".$b.$1}se;
  ' "$XML"
}
hotkey_set() { # $1 command  $2 param
  perl -0777 -i -pe '
    s{\s*<Hotkey>(?:(?!</Hotkey>).)*?<Shortcut>Ctrl\+Shift\+R</Shortcut>(?:(?!</Hotkey>).)*?</Hotkey>}{}gs;
  ' "$SCF"
  HKCMD="$1" HKPARAM="$2" perl -0777 -i -pe '
    my $b="      <Hotkey>\n        <Shortcut>Ctrl+Shift+R</Shortcut>\n        <Command>$ENV{HKCMD}</Command>\n        <Param>$ENV{HKPARAM}</Param>\n      </Hotkey>\n";
    s{(<Form Name="Main">)}{$1."\n".$b}se;
  ' "$SCF"
}

# ---------- generated files ----------
write_lua() { # writes $LUA with the CLI path baked in
  cat > "$LUA" <<EOF
-- Double Commander hotkey script: rename the file under the cursor inside a ZX
-- .trd/.scl image (while browsing it with the WCX plugin), then refresh the panel.
-- Installed copy - the CLI path is baked in by the installer.

local ZXDISK = '$CLI'

local image = DC.ExpandVar('%"0%A')
local entry = DC.ExpandVar('%"0%f')

if image == '' or entry == '' then
  Dialogs.MessageBox('Stand on a file inside a .trd/.scl image first.', 'ZX rename', 0)
  return
end

local ok, newname = Dialogs.InputQuery('ZX rename', 'New name for ' .. entry .. ':', false, entry)
if ok and newname ~= '' and newname ~= entry then
  -- os.execute is blocking, so the rename fully completes before we refresh.
  os.execute('"' .. ZXDISK .. '" rename "' .. image .. '" "' .. entry .. '" "' .. newname .. '"')
  DC.ExecuteCommand('cm_Refresh')
end
EOF
}
write_conf() { # reusable install config, stable location + copy in install dir
  mkdir -p "$(dirname "$REUSE_CONF")"
  {
    echo "# zxdisk installer settings (reusable; edit or delete freely)"
    echo "lang=$LANG_SEL"
    echo "mode=$MODE"
    echo "install_dir=$INSTALL_DIR"
    echo "config_dir=$CONFIG_DIR"
    echo "wcx=$WCX"
    echo "wlx=$WLX"
    echo "cli=$CLI"
    echo "lua=$LUA"
    echo "luajit=$LUAJIT"
    echo "toolbar_guid=$TOOLBAR_GUID"
  } > "$REUSE_CONF"
  cp "$REUSE_CONF" "$INSTALL_DIR/zxdisk-install.conf" 2>/dev/null || true
}
write_plugin_conf() { # create the plugin settings file with defaults, if absent
  [ -f "$PLUGIN_CONF" ] && return 0
  mkdir -p "$(dirname "$PLUGIN_CONF")"
  cat > "$PLUGIN_CONF" <<'EOF'
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
EOF
}
write_uninstaller() { # writes $INSTALL_DIR/uninstall.sh with baked-in paths
  local u="$INSTALL_DIR/uninstall.sh"
  {
    printf '#!/usr/bin/env bash\n'
    printf '# Auto-generated uninstaller for the ZX Spectrum disk plugins.\n'
    printf 'set -euo pipefail\n'
    printf 'XML=%q\n'         "$XML"
    printf 'SCF=%q\n'         "$SCF"
    printf 'INSTALL_DIR=%q\n' "$INSTALL_DIR"
    printf 'WCX=%q\n'         "$WCX"
    printf 'WLX=%q\n'         "$WLX"
    printf 'GUID=%q\n'        "$TOOLBAR_GUID"
    printf 'LUAJIT=%q\n'      "$LUAJIT"
    printf 'REUSE_CONF=%q\n'  "$REUSE_CONF"
    printf 'DC_MATCH=%q\n'    "$DC_MATCH"
    printf 'LUA_DEFAULT_NAME=%q\n' "$LUA_DEFAULT_NAME"
    cat <<'BODY'
export XML SCF INSTALL_DIR WCX WLX GUID LUAJIT DC_MATCH LUA_DEFAULT_NAME
STAMP="$(date +%Y%m%d-%H%M%S)"

if pgrep -f "$DC_MATCH" >/dev/null 2>&1; then
  echo "Double Commander is running - quit it first, then re-run this uninstaller."
  exit 1
fi

echo "Removing config entries ..."
[ -f "$XML" ] && cp -p "$XML" "$XML.zxuninstall-$STAMP"
[ -f "$SCF" ] && cp -p "$SCF" "$SCF.zxuninstall-$STAMP"

if [ -f "$XML" ]; then
  WCX="$WCX" perl -0777 -i -pe '
    my $wcx=quotemeta $ENV{WCX};
    s{\s*<WcxPlugin[^>]*>(?:(?!</WcxPlugin>).)*?<Path>$wcx</Path>(?:(?!</WcxPlugin>).)*?</WcxPlugin>}{}gs;
  ' "$XML"
  if [ -n "${WLX:-}" ]; then
    WLX="$WLX" perl -0777 -i -pe '
      my $wlx=quotemeta $ENV{WLX};
      s{\s*<WlxPlugin(?:\s[^>]*)?>(?:(?!</WlxPlugin>).)*?<Path>$wlx</Path>(?:(?!</WlxPlugin>).)*?</WlxPlugin>}{}gs;
    ' "$XML"
  fi
  GUID="$GUID" perl -0777 -i -pe '
    my $g=quotemeta $ENV{GUID};
    s{\s*<Program>(?:(?!</Program>).)*?$g(?:(?!</Program>).)*?</Program>}{}gs;
  ' "$XML"
  if [ -n "$LUAJIT" ]; then
    LUAJIT="$LUAJIT" perl -0777 -i -pe '
      my $l=quotemeta $ENV{LUAJIT};
      s{<PathToLibrary>$l</PathToLibrary>}{<PathToLibrary>$ENV{LUA_DEFAULT_NAME}</PathToLibrary>}s;
    ' "$XML"
  fi
fi

if [ -f "$SCF" ]; then
  INSTALL_DIR="$INSTALL_DIR" GUID="$GUID" perl -0777 -i -pe '
    my $inst=quotemeta $ENV{INSTALL_DIR}; my $g=quotemeta $ENV{GUID};
    s{\s*<Hotkey>(?:(?!</Hotkey>).)*?<Shortcut>Ctrl\+Shift\+R</Shortcut>(?:(?!</Hotkey>).)*?(?:$inst|$g)(?:(?!</Hotkey>).)*?</Hotkey>}{}gs;
  ' "$SCF"
fi

echo "Removing installed files in $INSTALL_DIR ..."
for f in zxdisk.wcx zxdisk.wlx zxdisk zxrename.lua zxdisk-install.conf; do
  [ -e "$INSTALL_DIR/$f" ] && rm -f "$INSTALL_DIR/$f" && echo "  removed $f"
done
rm -f "$REUSE_CONF" 2>/dev/null || true

SELF="$INSTALL_DIR/uninstall.sh"
rm -f "$SELF"
rmdir "$INSTALL_DIR" 2>/dev/null && echo "  removed empty $INSTALL_DIR" || true

echo "Done. Restart Double Commander. (Config backups: *.zxuninstall-$STAMP)"
BODY
  } > "$u"
  chmod +x "$u"
}

# ---------- run ----------

# language
if [ -z "$LANG_SEL" ]; then
  # English is the default; a previous choice does not override it (only --lang does)
  if [ "$ASSUME_YES" = 1 ]; then
    LANG_SEL=en
  else
    say "Language / Язык:"
    say "  1) Русский"
    say "  2) English"
    case "$(ask "1 / 2" "2")" in
      1) LANG_SEL=ru ;; *) LANG_SEL=en ;;
    esac
  fi
fi
[ "$LANG_SEL" = ru ] || [ "$LANG_SEL" = en ] || die "lang must be ru or en"

say ""
say "${C_BOLD}${C_CYN}== $(t title) ==${C_RESET}"
say ""
warn "$(t dc_notice)"
say ""

# variant
if [ -z "$MODE" ]; then
  say "$(t variant_head)"
  say "  $(t variant_1)"
  say "  $(t variant_2)"
  case "$(ask "$(t variant_ask)" "$( [ "$PREV_MODE" = basic ] && echo 1 || echo 2 )")" in
    1) MODE=basic ;; *) MODE=rename ;;
  esac
fi
[ "$MODE" = basic ] || [ "$MODE" = rename ] || die "mode must be basic or rename"

# install dir
if [ -z "$INSTALL_DIR" ]; then
  INSTALL_DIR="$(ask "$(t dir_ask)" "${PREV_DIR:-$DEFAULT_INSTALL_DIR}")"
fi

WCX="$INSTALL_DIR/zxdisk.wcx"
CLI="$INSTALL_DIR/zxdisk"
LUA="$INSTALL_DIR/zxrename.lua"
WLX="$INSTALL_DIR/zxdisk.wlx"

# built binaries present? (repo dist/, or bundled next to the installer in a release)
WCX_SRC="$(locate_bin zxdisk.wcx || true)"
if [ -z "$WCX_SRC" ]; then
  if command -v cargo >/dev/null 2>&1 && [ -x "$REPO/scripts/build.sh" ] && confirm "$(t build_ask)"; then
    "$REPO/scripts/build.sh"
    WCX_SRC="$(locate_bin zxdisk.wcx || true)"
  fi
fi
[ -n "$WCX_SRC" ] || die "missing zxdisk.wcx - build it (./scripts/build.sh) or use a release package"
CLI_SRC=""
if [ "$MODE" = rename ]; then
  CLI_SRC="$(locate_bin zxdisk || true)"
  [ -n "$CLI_SRC" ] || die "missing zxdisk (CLI) - build it (./scripts/build.sh) or use a release package"
fi
# The .scr screen viewer (WLX), macOS + Linux (Qt builds of DC); installed in
# both variants when the binary is present (absent in older packages -> skipped).
WLX_SRC="$(locate_bin zxdisk.wlx || true)"

# config present?
[ -f "$XML" ] || die "not found: $XML  (launch Double Commander once so it creates its config)"
[ -f "$SCF" ] || die "not found: $SCF  (launch Double Commander once so it creates its config)"

# DC must be closed when we edit the real config
if [ "$CONFIG_DIR" = "$DEFAULT_CONFIG_DIR" ] && pgrep -f "$DC_MATCH" >/dev/null 2>&1; then
  if [ "$OS" = mac ] && confirm "$(t dc_run)"; then
    osascript -e 'tell application "Double Commander" to quit' >/dev/null 2>&1 || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -f "$DC_MATCH" >/dev/null 2>&1 || break; sleep 0.5; done
    pgrep -f "$DC_MATCH" >/dev/null 2>&1 && die "$(t dc_still)"
  else
    die "$(t close_rerun)"
  fi
fi

# ---------- offer to remove a previous installation ----------
# A prior install leaves an uninstall.sh in its install dir (its path is recorded
# in the reusable config). If one is found, offer to run it first for a clean
# slate - this also removes files an old install left in a different folder.
PREV_UNINS=""
if [ -n "$PREV_DIR" ] && [ -f "$PREV_DIR/uninstall.sh" ]; then
  PREV_UNINS="$PREV_DIR/uninstall.sh"
elif [ -f "$INSTALL_DIR/uninstall.sh" ]; then
  PREV_UNINS="$INSTALL_DIR/uninstall.sh"; PREV_DIR="$INSTALL_DIR"
fi
if [ -n "$PREV_UNINS" ]; then
  warn "$(t prev_found)$PREV_DIR"
  if confirm_y "$(t prev_ask)"; then
    say "$(t prev_removing)"
    # Runs in its own process, so its exit does not abort this installer.
    bash "$PREV_UNINS" || warn "$(t prev_failed)"
  fi
fi

# lua 5.1 (rename only). If missing and we can install it (macOS + Homebrew),
# offer to - with confirmation, and never in non-interactive (--yes) mode.
LUAJIT=""
if [ "$MODE" = rename ]; then
  LUAJIT="$(detect_lua)"
  if [ -z "$LUAJIT" ] && [ "$OS" = mac ] && [ "$ASSUME_YES" != 1 ] && command -v brew >/dev/null 2>&1; then
    if confirm_y "$(t lua_ask)"; then
      warn "$(t lua_installing)"
      brew install luajit || warn "$(t lua_failed)"
      LUAJIT="$(detect_lua)"
    fi
  fi
fi
# In rename mode we always wire the Lua hotkey and point DC at a Lua path. If no
# lib is installed yet, use the canonical path it will land at, so the user only
# has to install it and restart DC later - no re-running the installer.
LUA_PRESENT=1
if [ "$MODE" = rename ] && [ -z "$LUAJIT" ]; then
  LUA_PRESENT=0
  LUAJIT="$(expected_lua)"
fi

say ""
say "${C_BOLD}$(t plan)${C_RESET}"
say "$(t p_variant)$MODE"
say "$(t p_dir)$INSTALL_DIR"
say "$(t p_config)$XML"
if [ "$MODE" = rename ]; then
  if [ "$LUA_PRESENT" = 1 ]; then say "$(t p_luajit)$LUAJIT"; else say "$(t p_luajit)$LUAJIT $(t lua_pending)"; fi
fi
say ""
confirm "$(t proceed)" || die "$(t aborted)"

# ---------- copy files ----------
mkdir -p "$INSTALL_DIR"
STAMP="$(date +%Y%m%d-%H%M%S)"
cp -p "$XML" "$XML.zxinstall-$STAMP"
cp -p "$SCF" "$SCF.zxinstall-$STAMP"

cp "$WCX_SRC" "$WCX"; chmod +x "$WCX"; ok "installed: $WCX"
if [ "$MODE" = rename ]; then
  cp "$CLI_SRC" "$CLI"; chmod +x "$CLI"; ok "installed: $CLI"
  write_lua;                                  ok "installed: $LUA"
fi
if [ -n "$WLX_SRC" ]; then
  cp "$WLX_SRC" "$WLX"; chmod +x "$WLX";      ok "installed: $WLX"
fi

# ---------- config edits ----------
ensure_wcx_section
wcx_register trd
wcx_register scl
ok "$(t reg_done)$WCX"

if [ -n "$WLX_SRC" ]; then
  ensure_wlx_section
  wlx_register
  ok "$(L 'просмотрщик экранов .scr зарегистрирован (F3)' 'screen viewer (.scr) registered (F3)')"
fi

if [ "$MODE" = rename ]; then
  toolbar_add "$CLI"                       # manual-rename button (works without Lua)
  set_lua_path "$LUAJIT"                    # point DC at the Lua lib (resolved or canonical)
  hotkey_set cm_ExecuteScript "$LUA"        # Ctrl+Shift+R -> Lua rename + auto-refresh
  if [ "$LUA_PRESENT" = 1 ]; then ok "$(t hk_lua)$LUAJIT"; else warn "$(t hk_pending)"; fi
fi

write_uninstaller
ok "installed: $INSTALL_DIR/uninstall.sh"
write_conf
ok "settings saved: $REUSE_CONF"
had_plugin_conf=0; [ -f "$PLUGIN_CONF" ] && had_plugin_conf=1
write_plugin_conf
[ "$had_plugin_conf" = 1 ] && ok "plugin settings kept: $PLUGIN_CONF" || ok "plugin settings: $PLUGIN_CONF"

# ---------- summary ----------
say ""
say "${C_GRN}${C_BOLD}$(t done_restart)${C_RESET}"
say "$(t backups)*.zxinstall-$STAMP"
if [ "$MODE" = rename ] && [ "$LUA_PRESENT" = 0 ]; then
  say ""
  warn "$(t note_head)"
  warn "    $LUA_INSTALL_HINT"
  warn "$(t note_body)"
fi
say ""
say "${C_CYN}$(t uninstall)\"$INSTALL_DIR/uninstall.sh\"${C_RESET}"
