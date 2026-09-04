//! Double Commander WCX packer plugin for ZX Spectrum `.trd` and `.scl` disk
//! images. This is a thin, panic-safe C-ABI shell over `zxdisk-core`.
//!
//! Double Commander drives it like an archive:
//!   * browse   - OpenArchive -> (ReadHeaderEx + ProcessFile PK_SKIP)* -> CloseArchive
//!   * extract  - same loop with ProcessFile PK_EXTRACT for wanted files
//!   * add      - PackFiles (copy host files into the image)
//!   * delete   - DeleteFiles
//!
//! Deleted TR-DOS files are surfaced under a virtual `deleted\` folder and can be
//! extracted (recovered); they are read-only.

// The exported functions share one safety contract - they are C-ABI entry
// points called by Double Commander exactly as the WCX SDK specifies (valid
// NUL-terminated strings, handles previously returned by OpenArchive) - so a
// per-function `# Safety` section would repeat that eleven times.
#![allow(clippy::missing_safety_doc)]

pub mod wcx;

use std::collections::HashSet;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_uint};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use wcx::*;
use zxdisk_core::{DiskType, Image, TrFile};

// ---------------------------------------------------------------------------
// FFI string encoding
//
// Double Commander has no Unicode WCX entry points here, so on Windows it hands
// us paths/names in the *system ANSI code page* (e.g. CP1251 on Russian
// Windows), not UTF-8. Decoding those bytes as UTF-8 turns every non-ASCII byte
// into U+FFFD and the path stops existing, so Cyrillic profiles/paths fail
// entirely. Convert through the code page instead. On macOS/Linux DC already
// uses UTF-8, so there the conversion is a plain UTF-8 round-trip.
// ---------------------------------------------------------------------------

/// Decode NUL-free FFI string bytes into a Rust `String`.
#[cfg(windows)]
fn decode_ffi(bytes: &[u8]) -> String {
    win_cp::ansi_to_string(bytes)
}
#[cfg(not(windows))]
fn decode_ffi(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Encode a Rust `&str` into FFI string bytes (no NUL).
#[cfg(windows)]
fn encode_ffi(s: &str) -> Vec<u8> {
    win_cp::string_to_ansi(s)
}
#[cfg(not(windows))]
fn encode_ffi(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

#[cfg(windows)]
mod win_cp {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, WideCharToMultiByte};
    // Code page 0 == CP_ACP, the system ANSI code page.
    const CP_ACP: u32 = 0;

    /// Decode bytes in the system ANSI code page to a `String`.
    pub fn ansi_to_string(bytes: &[u8]) -> String {
        if bytes.is_empty() {
            return String::new();
        }
        unsafe {
            let n = MultiByteToWideChar(
                CP_ACP,
                0,
                bytes.as_ptr(),
                bytes.len() as i32,
                core::ptr::null_mut(),
                0,
            );
            if n <= 0 {
                return String::from_utf8_lossy(bytes).into_owned();
            }
            let mut wide = vec![0u16; n as usize];
            MultiByteToWideChar(
                CP_ACP,
                0,
                bytes.as_ptr(),
                bytes.len() as i32,
                wide.as_mut_ptr(),
                n,
            );
            String::from_utf16_lossy(&wide)
        }
    }

    /// Encode a `&str` to the system ANSI code page (best effort; characters the
    /// page cannot represent become the default replacement char).
    pub fn string_to_ansi(s: &str) -> Vec<u8> {
        if s.is_empty() {
            return Vec::new();
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let n = WideCharToMultiByte(
                CP_ACP,
                0,
                wide.as_ptr(),
                wide.len() as i32,
                core::ptr::null_mut(),
                0,
                core::ptr::null(),
                core::ptr::null_mut(),
            );
            if n <= 0 {
                return s.as_bytes().to_vec();
            }
            let mut out = vec![0u8; n as usize];
            WideCharToMultiByte(
                CP_ACP,
                0,
                wide.as_ptr(),
                wide.len() as i32,
                out.as_mut_ptr(),
                n,
                core::ptr::null(),
                core::ptr::null_mut(),
            );
            out
        }
    }
}

/// A fixed, valid DOS date/time (1990-01-01 12:00) - TR-DOS has no timestamps.
const DOS_TIME: c_int = 0x1421_6000;
/// Virtual folder that deleted files are listed under.
const DELETED_DIR: &str = "deleted\\";

/// Path to the plugin ini Double Commander hands us via PackSetDefaultParams.
static PLUGIN_INI: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Read a setting value, first match wins:
///   * environment variable `env_name`
///   * key `key` in the DC-provided plugin ini
///   * key `key` in a fallback config file (see `fallback_config_paths`)
fn config_value(key: &str, env_name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(env_name) {
        if !v.is_empty() {
            return Some(v);
        }
    }
    if let Some(path) = PLUGIN_INI.lock().ok().and_then(|g| g.clone()) {
        if let Some(v) = read_ini_key(&path, key) {
            return Some(v);
        }
    }
    for p in fallback_config_paths() {
        if let Some(v) = read_ini_key(&p, key) {
            return Some(v);
        }
    }
    None
}

/// Whether extraction should produce metadata-preserving `.hobeta` files instead
/// of plain raw data. Default OFF (extract raw).
fn extract_hobeta() -> bool {
    config_value("extract_hobeta", "ZXDISK_WCX_HOBETA")
        .map(|v| truthy(&v))
        .unwrap_or(false)
}

/// Apply the `ext_mode` setting (1-char / 3-char / smart) to the process-wide
/// default used for displaying and matching file names. Called before listing
/// and before name-matching operations.
fn apply_ext_mode() {
    // Accept the generic `ZXDISK_EXT_MODE` too (the name the CLI uses), so the
    // plugin listing and the CLI-driven rename agree on how names are formed.
    let v = config_value("ext_mode", "ZXDISK_WCX_EXT_MODE").or_else(|| {
        std::env::var("ZXDISK_EXT_MODE")
            .ok()
            .filter(|s| !s.is_empty())
    });
    if let Some(v) = v {
        if let Some(m) = zxdisk_core::ExtMode::parse(&v) {
            zxdisk_core::set_default_ext_mode(m);
        }
    }
}

/// Geometry to use when creating a brand-new TRD image. Default 640K (80x2).
fn new_trd_geometry() -> DiskType {
    config_value("new_trd_geometry", "ZXDISK_WCX_TRD_GEOMETRY")
        .and_then(|v| parse_geometry(&v))
        .unwrap_or(DiskType::Ds80)
}

/// Parse a geometry setting value into a [`DiskType`]. Accepts a few friendly
/// aliases; unknown values return `None` (caller falls back to the default).
fn parse_geometry(v: &str) -> Option<DiskType> {
    match v.trim().to_ascii_lowercase().as_str() {
        "640k" | "640" | "80x2" | "ds80" => Some(DiskType::Ds80),
        "320k-ds" | "40x2" | "ds40" => Some(DiskType::Ds40),
        "320k-ss" | "80x1" | "ss80" => Some(DiskType::Ss80),
        "160k" | "160" | "40x1" | "ss40" => Some(DiskType::Ss40),
        _ => None,
    }
}

fn truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Read an ini-style `key = value` from a file, ignoring comments and sections.
fn read_ini_key(path: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with([';', '#', '[']) {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn fallback_config_paths() -> Vec<String> {
    let mut out = Vec::new();
    // Unix-style home (also set under Git Bash / MSYS on Windows).
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().into_owned();
        out.push(format!("{home}/.config/zxdisk.conf")); // shared by both plugins
        out.push(format!("{home}/.config/zxdisk-wcx.conf"));
        out.push(format!("{home}/.config/doublecmd/zxdisk-wcx.conf"));
        out.push(format!(
            "{home}/Library/Application Support/doublecmd/zxdisk-wcx.conf"
        ));
    }
    // Windows: the plugin runs inside doublecmd.exe, where HOME is usually unset,
    // so also look under the user profile and roaming AppData (where
    // install-core.ps1 writes the shared zxdisk.conf). Forward slashes are fine
    // for std::fs.
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let p = profile.to_string_lossy().into_owned();
        out.push(format!("{p}/.config/zxdisk.conf"));
        out.push(format!("{p}/.config/zxdisk-wcx.conf"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let a = appdata.to_string_lossy().into_owned();
        out.push(format!("{a}/zxdisk/zxdisk.conf")); // shared, written by install-core.ps1
        out.push(format!("{a}/zxdisk/zxdisk-wcx.conf"));
        out.push(format!("{a}/doublecmd/zxdisk-wcx.conf"));
    }
    out
}

/// In-archive display name for an entry, matching the extraction mode.
fn entry_name(e: &TrFile, hobeta: bool) -> String {
    if hobeta {
        e.hobeta_name()
    } else {
        e.display_name()
    }
}

/// Bytes written when extracting an entry.
fn encode_entry(e: &TrFile, hobeta: bool) -> Vec<u8> {
    if hobeta {
        zxdisk_core::hobeta::wrap(e)
    } else {
        e.data.clone()
    }
}

/// Length of the bytes `encode_entry` would produce, without allocating them.
/// (hobeta pads its data to a whole sector, so mirror that here.)
fn extracted_len(e: &TrFile, hobeta: bool) -> usize {
    if hobeta {
        let padded = e.data.len().div_ceil(256) * 256;
        zxdisk_core::hobeta::HEADER_LEN + padded
    } else {
        e.data.len()
    }
}

/// Append a diagnostic line to the debug log, but only when the `debug_log`
/// setting is on (default OFF, so normal use writes nothing). The log path is
/// `debug_log_path` if set, else `~/zxdisk-wcx.log`.
fn debug_log(msg: &str) {
    if !config_value("debug_log", "ZXDISK_WCX_DEBUG")
        .map(|v| truthy(&v))
        .unwrap_or(false)
    {
        return;
    }
    let path = match config_value("debug_log_path", "ZXDISK_WCX_LOG") {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => match std::env::var_os("HOME") {
            Some(h) => std::path::Path::new(&h).join("zxdisk-wcx.log"),
            None => return,
        },
    };
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// Add every file in `list` (relative to `src`) into the image at `packed`,
/// creating the image if it does not exist. Pure (no FFI) so it is unit-testable.
fn do_pack(packed: &str, sub: &str, src: &str, list: &[String], flags: c_int) -> c_int {
    apply_ext_mode();
    // We do not support packing into a sub-folder of the image (e.g. the virtual
    // `deleted\` view). Silently dropping such files into the root would be
    // surprising, so refuse instead.
    if !sub.trim_matches(['\\', '/']).is_empty() {
        return E_NOT_SUPPORTED;
    }
    let ext = ext_of(packed);
    let mut image = match std::fs::read(packed) {
        Ok(bytes) => match Image::from_bytes(&bytes, Some(&ext)) {
            Ok(img) => img,
            Err(_) => return E_BAD_ARCHIVE,
        },
        // Only a genuine "no such file" means "create a new image". Any other
        // read failure (sharing violation, permissions, transient I/O) must NOT
        // be treated as absent - clobbering an existing image with a blank one
        // would destroy the user's data.
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            Image::blank_for_ext_with(&ext, new_trd_geometry())
        }
        Err(_) => return E_EOPEN,
    };
    // Remember the source paths so a "move" operation can delete them afterwards.
    let mut sources: Vec<String> = Vec::with_capacity(list.len());
    for rel in list {
        let full = if src.is_empty() {
            rel.clone()
        } else {
            Path::new(src).join(rel).to_string_lossy().into_owned()
        };
        let data = match std::fs::read(&full) {
            Ok(d) => d,
            Err(_) => return E_EREAD,
        };
        // A hobeta file (valid 17-byte header) restores full metadata; any other
        // file is imported as raw data with its type inferred from the extension.
        let file = match zxdisk_core::hobeta::parse(&data) {
            Some(f) => f,
            None => TrFile::from_host_filename(&basename(rel), data),
        };
        if let Err(e) = image.add_file(&file) {
            return map_core_err(e);
        }
        sources.push(full);
    }
    if std::fs::write(packed, image.to_bytes()).is_err() {
        return E_EWRITE;
    }
    // PK_PACK_MOVE_FILES (F6 "move to archive"): the plugin owns deleting the
    // originals once they are safely inside the image.
    if flags & PK_PACK_MOVE_FILES != 0 {
        for full in &sources {
            let _ = std::fs::remove_file(full);
        }
    }
    E_SUCCESS
}

/// Delete every named file from the image at `packed`. Pure (no FFI).
fn do_delete(packed: &str, list: &[String]) -> c_int {
    apply_ext_mode();
    let ext = ext_of(packed);
    let bytes = match std::fs::read(packed) {
        Ok(b) => b,
        Err(_) => return E_EOPEN,
    };
    let mut image = match Image::from_bytes(&bytes, Some(&ext)) {
        Ok(img) => img,
        Err(_) => return E_BAD_ARCHIVE,
    };
    let mut skipped = 0usize;
    for name in list {
        let name = name.replace('/', "\\");
        // Recovered files live in the virtual deleted\ folder; they are read-only.
        if name.starts_with(DELETED_DIR) {
            skipped += 1;
            continue;
        }
        image.delete_file(&name);
    }
    // If the selection was nothing but already-deleted (recovered) entries there
    // is nothing real to remove - say so rather than silently "succeeding".
    if !list.is_empty() && skipped == list.len() {
        return E_NOT_SUPPORTED;
    }
    match std::fs::write(packed, image.to_bytes()) {
        Ok(_) => E_SUCCESS,
        Err(_) => E_EWRITE,
    }
}

/// Run an FFI body, converting any panic into an error code instead of letting
/// it unwind across the C ABI (which would be undefined behaviour).
macro_rules! guard {
    ($default:expr, $body:block) => {{
        match catch_unwind(AssertUnwindSafe(|| $body)) {
            Ok(v) => v,
            Err(_) => $default,
        }
    }};
}

/// Per-open-archive state, boxed and handed back to DC as an opaque HANDLE.
struct ArcState {
    arc_name: String,
    entries: Vec<TrFile>,
    /// In-archive display names, aligned 1:1 with `entries`.
    names: Vec<String>,
    /// Index of the next entry ReadHeader will report.
    index: usize,
    /// Index of the entry the last ReadHeader reported (target of ProcessFile).
    current: usize,
    /// Whether this archive was opened in hobeta extraction mode.
    hobeta: bool,
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

unsafe fn cstr_to_string(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        decode_ffi(CStr::from_ptr(p).to_bytes())
    }
}

unsafe fn cstr_opt(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        Some(cstr_to_string(p))
    }
}

/// Zero a fixed C char array then copy `s` in, encoded for the FFI (system code
/// page on Windows), NUL-terminated and truncated to fit.
fn write_carray<const N: usize>(buf: &mut [c_char; N], s: &str) {
    for b in buf.iter_mut() {
        *b = 0;
    }
    let bytes = encode_ffi(s);
    let n = bytes.len().min(N.saturating_sub(1));
    for i in 0..n {
        buf[i] = bytes[i] as c_char;
    }
}

/// Parse a double-NUL-terminated list of C strings (the WCX AddList/DeleteList).
unsafe fn parse_double_null(mut p: *const c_char) -> Vec<String> {
    let mut out = Vec::new();
    if p.is_null() {
        return out;
    }
    loop {
        let bytes = CStr::from_ptr(p).to_bytes();
        if bytes.is_empty() {
            break;
        }
        out.push(decode_ffi(bytes));
        p = p.add(bytes.len() + 1);
    }
    out
}

fn ext_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string()
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|e| e.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Read at most `max` bytes from the front of a file (for content detection).
fn read_prefix(path: &str, max: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Build the in-archive display names, prefixing deleted files with `deleted\`
/// and disambiguating collisions so extraction targets stay unique.
fn build_names(entries: &[TrFile], hobeta: bool) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut names = Vec::with_capacity(entries.len());
    for e in entries {
        let base = entry_name(e, hobeta);
        let mut name = if e.deleted {
            format!("{DELETED_DIR}{base}")
        } else {
            base
        };
        if seen.contains(&name) {
            let (stem, ext) = match name.rfind('.') {
                Some(i) => (name[..i].to_string(), name[i..].to_string()),
                None => (name.clone(), String::new()),
            };
            let mut k = 2;
            loop {
                let candidate = format!("{stem}_{k}{ext}");
                if !seen.contains(&candidate) {
                    name = candidate;
                    break;
                }
                k += 1;
            }
        }
        seen.insert(name.clone());
        names.push(name);
    }
    names
}

fn load_state(path: &str) -> Result<ArcState, c_int> {
    apply_ext_mode();
    let bytes = std::fs::read(path).map_err(|_| E_EOPEN)?;
    let ext = ext_of(path);
    let image = Image::from_bytes(&bytes, Some(&ext)).map_err(|_| E_UNKNOWN_FORMAT)?;
    let entries = image.entries();
    let hobeta = extract_hobeta();
    let names = build_names(&entries, hobeta);
    Ok(ArcState {
        arc_name: path.to_string(),
        entries,
        names,
        index: 0,
        current: 0,
        hobeta,
    })
}

/// Resolve the extraction target path from ProcessFile's DestPath/DestName.
unsafe fn build_dest(dest_path: *const c_char, dest_name: *const c_char) -> Option<PathBuf> {
    let path = cstr_opt(dest_path);
    let name = cstr_opt(dest_name);
    match (path, name) {
        (Some(p), Some(n)) if !p.is_empty() => Some(Path::new(&p).join(&n)),
        (_, Some(n)) => Some(PathBuf::from(n)),
        (Some(p), None) => Some(PathBuf::from(p)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// WCX exports
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "system" fn OpenArchive(data: *mut tOpenArchiveData) -> HANDLE {
    guard!(std::ptr::null_mut(), {
        if data.is_null() {
            return std::ptr::null_mut();
        }
        let d = &mut *data;
        let path = cstr_to_string(d.ArcName);
        debug_log(&format!("OpenArchive mode={} path={path:?}", d.OpenMode));
        match load_state(&path) {
            Ok(state) => Box::into_raw(Box::new(state)) as HANDLE,
            Err(code) => {
                debug_log(&format!("OpenArchive failed rc={code}"));
                d.OpenResult = code;
                std::ptr::null_mut()
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "system" fn ReadHeaderEx(h: HANDLE, hdr: *mut tHeaderDataEx) -> c_int {
    guard!(E_BAD_ARCHIVE, {
        if h.is_null() || hdr.is_null() {
            return E_BAD_ARCHIVE;
        }
        let st = &mut *(h as *mut ArcState);
        if st.index >= st.entries.len() {
            return E_END_ARCHIVE;
        }
        st.current = st.index;
        let e = &st.entries[st.index];
        let size = extracted_len(e, st.hobeta) as c_uint;
        let hd = &mut *hdr;
        write_carray(&mut hd.ArcName, &st.arc_name);
        write_carray(&mut hd.FileName, &st.names[st.index]);
        hd.Flags = 0;
        hd.PackSize = size;
        hd.PackSizeHigh = 0;
        hd.UnpSize = size;
        hd.UnpSizeHigh = 0;
        hd.HostOS = 0;
        hd.FileCRC = 0;
        hd.FileTime = DOS_TIME;
        hd.UnpVer = 0;
        hd.Method = 0;
        hd.FileAttr = if e.deleted { 0x21 } else { 0x20 }; // archive; ro for deleted
        hd.CmtBuf = std::ptr::null_mut();
        hd.CmtBufSize = 0;
        hd.CmtSize = 0;
        hd.CmtState = 0;
        hd.Reserved = [0; 1024]; // defined "fill with 0"; do not leak stack bytes to DC
        E_SUCCESS
    })
}

#[no_mangle]
pub unsafe extern "system" fn ReadHeader(h: HANDLE, hdr: *mut tHeaderData) -> c_int {
    guard!(E_BAD_ARCHIVE, {
        if h.is_null() || hdr.is_null() {
            return E_BAD_ARCHIVE;
        }
        let st = &mut *(h as *mut ArcState);
        if st.index >= st.entries.len() {
            return E_END_ARCHIVE;
        }
        st.current = st.index;
        let e = &st.entries[st.index];
        let size = extracted_len(e, st.hobeta) as c_int;
        let hd = &mut *hdr;
        write_carray(&mut hd.ArcName, &st.arc_name);
        write_carray(&mut hd.FileName, &st.names[st.index]);
        hd.Flags = 0;
        hd.PackSize = size;
        hd.UnpSize = size;
        hd.HostOS = 0;
        hd.FileCRC = 0;
        hd.FileTime = DOS_TIME;
        hd.UnpVer = 0;
        hd.Method = 0;
        hd.FileAttr = if e.deleted { 0x21 } else { 0x20 };
        hd.CmtBuf = std::ptr::null_mut();
        hd.CmtBufSize = 0;
        hd.CmtSize = 0;
        hd.CmtState = 0;
        E_SUCCESS
    })
}

#[no_mangle]
pub unsafe extern "system" fn ProcessFile(
    h: HANDLE,
    operation: c_int,
    dest_path: *mut c_char,
    dest_name: *mut c_char,
) -> c_int {
    guard!(E_BAD_ARCHIVE, {
        if h.is_null() {
            return E_BAD_ARCHIVE;
        }
        let st = &mut *(h as *mut ArcState);
        let idx = st.current;
        let result = if operation == PK_EXTRACT {
            if idx >= st.entries.len() {
                E_BAD_ARCHIVE
            } else {
                match build_dest(dest_path, dest_name) {
                    Some(p) => {
                        let bytes = encode_entry(&st.entries[idx], st.hobeta);
                        match std::fs::write(&p, &bytes) {
                            Ok(_) => E_SUCCESS,
                            Err(_) => E_EWRITE,
                        }
                    }
                    None => E_EWRITE,
                }
            }
        } else {
            E_SUCCESS // PK_SKIP / PK_TEST
        };
        st.index = idx + 1;
        result
    })
}

#[no_mangle]
pub unsafe extern "system" fn CloseArchive(h: HANDLE) -> c_int {
    guard!(E_SUCCESS, {
        if !h.is_null() {
            drop(Box::from_raw(h as *mut ArcState));
        }
        E_SUCCESS
    })
}

#[no_mangle]
pub extern "system" fn GetPackerCaps() -> c_int {
    let caps =
        PK_CAPS_NEW | PK_CAPS_MODIFY | PK_CAPS_MULTIPLE | PK_CAPS_DELETE | PK_CAPS_BY_CONTENT;
    debug_log(&format!("GetPackerCaps -> {caps}"));
    caps
}

#[no_mangle]
pub unsafe extern "system" fn PackFiles(
    packed_file: *mut c_char,
    sub_path: *mut c_char,
    src_path: *mut c_char,
    add_list: *mut c_char,
    flags: c_int,
) -> c_int {
    guard!(E_BAD_ARCHIVE, {
        let packed = cstr_to_string(packed_file);
        let sub = cstr_to_string(sub_path);
        let src = cstr_to_string(src_path);
        let list = parse_double_null(add_list);
        debug_log(&format!(
            "PackFiles packed={packed:?} sub={sub:?} src={src:?} flags={flags} list={list:?}"
        ));
        let rc = do_pack(&packed, &sub, &src, &list, flags);
        debug_log(&format!("PackFiles -> {rc}"));
        rc
    })
}

#[no_mangle]
pub unsafe extern "system" fn DeleteFiles(
    packed_file: *mut c_char,
    delete_list: *mut c_char,
) -> c_int {
    guard!(E_BAD_ARCHIVE, {
        let packed = cstr_to_string(packed_file);
        let list = parse_double_null(delete_list);
        debug_log(&format!("DeleteFiles packed={packed:?} list={list:?}"));
        let rc = do_delete(&packed, &list);
        debug_log(&format!("DeleteFiles -> {rc}"));
        rc
    })
}

#[no_mangle]
pub unsafe extern "system" fn CanYouHandleThisFile(name: *mut c_char) -> c_int {
    guard!(0, {
        let path = cstr_to_string(name);
        match read_prefix(&path, zxdisk_core::trd::INFO_SECTOR + 256) {
            Ok(prefix) => {
                if Image::detect(&prefix).is_some() {
                    1
                } else {
                    0
                }
            }
            Err(_) => 0,
        }
    })
}

#[no_mangle]
pub unsafe extern "system" fn SetChangeVolProc(_h: HANDLE, _proc: tChangeVolProc) {
    // Single-volume images: nothing to do.
}

#[no_mangle]
pub unsafe extern "system" fn SetProcessDataProc(_h: HANDLE, _proc: tProcessDataProc) {
    // Progress reporting is not used. Crucially, do NOT dereference the handle:
    // for PackFiles / DeleteFiles, Double Commander calls this with a handle that
    // is NOT one of our OpenArchive states (there is no open archive for those
    // operations), so casting it to *mut ArcState and writing through it corrupts
    // memory and crashes DC. Leaving it a no-op is correct and safe.
}

#[no_mangle]
pub unsafe extern "system" fn PackSetDefaultParams(dps: *mut PackDefaultParamStruct) {
    guard!((), {
        if dps.is_null() {
            return;
        }
        let ini = cstr_to_string((*dps).DefaultIniName.as_ptr());
        debug_log(&format!("PackSetDefaultParams ini={ini:?}"));
        if !ini.is_empty() {
            if let Ok(mut g) = PLUGIN_INI.lock() {
                *g = Some(ini);
            }
        }
    })
}

fn map_core_err(e: zxdisk_core::Error) -> c_int {
    use zxdisk_core::Error::*;
    match e {
        TooManyFiles => E_TOO_MANY_FILES,
        // The image is full / the file will not fit - a write-side limit, not an
        // out-of-memory condition, so E_EWRITE reads correctly in DC's UI.
        DiskFull | FileTooBig => E_EWRITE,
        UnknownFormat => E_UNKNOWN_FORMAT,
        BadArchive(_) => E_BAD_ARCHIVE,
        NotFound => E_NO_FILES,
    }
}

#[cfg(test)]
mod tests;
