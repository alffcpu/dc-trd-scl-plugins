//! Minimal WCX (Total Commander / Double Commander packer plugin) C-ABI
//! definitions. Only what this plugin needs. Layouts mirror Double Commander's
//! `sdk/wcxplugin.pas` and the Total Commander `wcxhead.h`.
//!
//! The TC/DC packer API is `__stdcall` on Windows, so both the exported plugin
//! functions and these callback typedefs use `extern "system"`: that is stdcall
//! on 32-bit Windows and plain C `cdecl` everywhere else (Win64, macOS, Linux),
//! which is exactly the ABI DC expects on each platform. (Using `extern "C"`
//! here would corrupt the stack in 32-bit Double Commander.)

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::os::raw::{c_char, c_int, c_uint, c_void};

// OpenArchive open modes.
pub const PK_OM_LIST: c_int = 0;
pub const PK_OM_EXTRACT: c_int = 1;

// ProcessFile operations.
pub const PK_SKIP: c_int = 0;
pub const PK_TEST: c_int = 1;
pub const PK_EXTRACT: c_int = 2;

// Error / status codes.
pub const E_SUCCESS: c_int = 0;
pub const E_END_ARCHIVE: c_int = 10;
pub const E_NO_MEMORY: c_int = 11;
pub const E_BAD_DATA: c_int = 12;
pub const E_BAD_ARCHIVE: c_int = 13;
pub const E_UNKNOWN_FORMAT: c_int = 14;
pub const E_EOPEN: c_int = 15;
pub const E_ECREATE: c_int = 16;
pub const E_ECLOSE: c_int = 17;
pub const E_EREAD: c_int = 18;
pub const E_EWRITE: c_int = 19;
pub const E_SMALL_BUF: c_int = 20;
pub const E_EABORTED: c_int = 21;
pub const E_NO_FILES: c_int = 22;
pub const E_TOO_MANY_FILES: c_int = 23;
pub const E_NOT_SUPPORTED: c_int = 24;

// PackFiles `flags`.
pub const PK_PACK_MOVE_FILES: c_int = 1; // delete the source files after packing
pub const PK_PACK_SAVE_PATHS: c_int = 2;
pub const PK_PACK_ENCRYPT: c_int = 4;

// GetPackerCaps flags.
pub const PK_CAPS_NEW: c_int = 1;
pub const PK_CAPS_MODIFY: c_int = 2;
pub const PK_CAPS_MULTIPLE: c_int = 4;
pub const PK_CAPS_DELETE: c_int = 8;
pub const PK_CAPS_OPTIONS: c_int = 16;
pub const PK_CAPS_MEMPACK: c_int = 32;
pub const PK_CAPS_BY_CONTENT: c_int = 64;
pub const PK_CAPS_SEARCHTEXT: c_int = 128;
pub const PK_CAPS_HIDE: c_int = 256;
pub const PK_CAPS_ENCRYPT: c_int = 512;

pub type tProcessDataProc = Option<unsafe extern "system" fn(*mut c_char, c_int) -> c_int>;
pub type tChangeVolProc = Option<unsafe extern "system" fn(*mut c_char, c_int) -> c_int>;

#[repr(C)]
pub struct tOpenArchiveData {
    pub ArcName: *mut c_char,
    pub OpenMode: c_int,
    pub OpenResult: c_int,
    pub CmtBuf: *mut c_char,
    pub CmtBufSize: c_int,
    pub CmtSize: c_int,
    pub CmtState: c_int,
}

#[repr(C)]
pub struct tHeaderData {
    pub ArcName: [c_char; 260],
    pub FileName: [c_char; 260],
    pub Flags: c_int,
    pub PackSize: c_int,
    pub UnpSize: c_int,
    pub HostOS: c_int,
    pub FileCRC: c_int,
    pub FileTime: c_int,
    pub UnpVer: c_int,
    pub Method: c_int,
    pub FileAttr: c_int,
    pub CmtBuf: *mut c_char,
    pub CmtBufSize: c_int,
    pub CmtSize: c_int,
    pub CmtState: c_int,
}

#[repr(C)]
pub struct tHeaderDataEx {
    pub ArcName: [c_char; 1024],
    pub FileName: [c_char; 1024],
    pub Flags: c_int,
    pub PackSize: c_uint,
    pub PackSizeHigh: c_uint,
    pub UnpSize: c_uint,
    pub UnpSizeHigh: c_uint,
    pub HostOS: c_int,
    pub FileCRC: c_int,
    pub FileTime: c_int,
    pub UnpVer: c_int,
    pub Method: c_int,
    pub FileAttr: c_int,
    pub CmtBuf: *mut c_char,
    pub CmtBufSize: c_int,
    pub CmtSize: c_int,
    pub CmtState: c_int,
    pub Reserved: [c_char; 1024],
}

#[repr(C)]
pub struct PackDefaultParamStruct {
    pub size: c_int,
    pub PluginInterfaceVersionLow: c_int,
    pub PluginInterfaceVersionHi: c_int,
    pub DefaultIniName: [c_char; 260],
}

/// Opaque archive handle returned by OpenArchive.
pub type HANDLE = *mut c_void;
