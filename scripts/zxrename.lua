-- Double Commander hotkey script: rename the file the cursor is on INSIDE a ZX
-- .trd/.scl image (while browsing it with the WCX plugin), then refresh the panel.
--
-- This is the "nice" variant with automatic refresh, but it needs DC's Lua library
-- (liblua5.1) to be installed. The plain toolbar-button recipe in docs/DEVELOPMENT.md
-- needs no Lua but does not auto-refresh (you press Ctrl+R yourself).
--
-- Install: bind a hotkey via Configuration > Options > Hot Keys > command
-- cm_ExecuteScript, parameter = the full path to this file.

local ZXDISK = os.getenv('HOME') .. '/.local/bin/zxdisk'

-- %A = the real image path, %f = the entry name (e.g. spisok.CRD). %"0 = unescaped.
local image = DC.ExpandVar('%"0%A')
local entry = DC.ExpandVar('%"0%f')

if image == '' or entry == '' then
  Dialogs.MessageBox('Stand on a file inside a .trd/.scl image first.', 'ZX rename', 0)
  return
end

local ok, newname = Dialogs.InputQuery('ZX rename', 'New name for ' .. entry .. ':', false, entry)
if ok and newname ~= '' and newname ~= entry then
  -- os.execute is blocking (C system()), so the rename fully completes before we
  -- refresh the panel - avoids a race where cm_Refresh reads the stale listing.
  os.execute('"' .. ZXDISK .. '" rename "' .. image .. '" "' .. entry .. '" "' .. newname .. '"')
  DC.ExecuteCommand('cm_Refresh')
end
