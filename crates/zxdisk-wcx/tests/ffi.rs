//! End-to-end exercise of the WCX C-ABI entry points, the way Double Commander
//! drives them: list, extract, add (pack), delete, and content detection.

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicUsize, Ordering};

use zxdisk_core::{DiskType, TrFile, TrdImage};
use zxdisk_wcx::wcx::*;
use zxdisk_wcx::*;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn tmp_path(ext: &str) -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("zxwcx-{}-{}.{}", std::process::id(), n, ext))
}

fn name8(s: &str) -> [u8; 8] {
    let mut n = [b' '; 8];
    for (i, b) in s.bytes().take(8).enumerate() {
        n[i] = b;
    }
    n
}

fn carray_str(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Build a TRD with two live files, then delete one so a recoverable entry
/// exists, and write it to a temp file. Returns (path, original data of GAME).
fn make_test_trd() -> (std::path::PathBuf, Vec<u8>) {
    let mut trd = TrdImage::blank(DiskType::Ds80, "TESTDISK");
    let game = vec![0xABu8; 500];
    trd.add_file(&TrFile::new(name8("GAME"), b'C', 0x8000, game.clone()))
        .unwrap();
    trd.add_file(&TrFile::new(name8("LOADER"), b'B', 10, vec![0x11u8; 100]))
        .unwrap();
    trd.mark_deleted("LOADER.B"); // becomes a recoverable deleted entry
    let path = tmp_path("trd");
    std::fs::write(&path, trd.to_bytes()).unwrap();
    (path, game)
}

/// List every entry an archive reports through the open/read/skip/close cycle.
unsafe fn list_names(path: &std::path::Path) -> Vec<String> {
    let cpath = CString::new(path.to_str().unwrap()).unwrap();
    let mut oad: tOpenArchiveData = std::mem::zeroed();
    oad.ArcName = cpath.as_ptr() as *mut c_char;
    oad.OpenMode = PK_OM_LIST;
    let h = OpenArchive(&mut oad);
    assert!(!h.is_null(), "OpenArchive failed: {}", oad.OpenResult);

    let mut names = Vec::new();
    loop {
        let mut hdr: tHeaderDataEx = std::mem::zeroed();
        let r = ReadHeaderEx(h, &mut hdr);
        if r == E_END_ARCHIVE {
            break;
        }
        assert_eq!(r, E_SUCCESS);
        names.push(carray_str(&hdr.FileName));
        assert_eq!(ProcessFile(h, PK_SKIP, std::ptr::null_mut(), std::ptr::null_mut()), E_SUCCESS);
    }
    CloseArchive(h);
    names
}

#[test]
fn list_reports_live_and_deleted() {
    unsafe {
        let (path, _) = make_test_trd();
        let names = list_names(&path);
        assert!(names.contains(&"GAME.C".to_string()), "names: {names:?}");
        // The erased LOADER is surfaced under the virtual deleted\ folder.
        assert!(
            names.iter().any(|n| n.starts_with("deleted\\")),
            "expected a deleted entry, got: {names:?}"
        );
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn extract_recovers_file_bytes() {
    unsafe {
        let (path, game) = make_test_trd();
        let cpath = CString::new(path.to_str().unwrap()).unwrap();
        let mut oad: tOpenArchiveData = std::mem::zeroed();
        oad.ArcName = cpath.as_ptr() as *mut c_char;
        oad.OpenMode = PK_OM_EXTRACT;
        let h = OpenArchive(&mut oad);
        assert!(!h.is_null());

        let dest = tmp_path("bin");
        let cdest = CString::new(dest.to_str().unwrap()).unwrap();
        loop {
            let mut hdr: tHeaderDataEx = std::mem::zeroed();
            if ReadHeaderEx(h, &mut hdr) == E_END_ARCHIVE {
                break;
            }
            let op = if carray_str(&hdr.FileName) == "GAME.C" {
                PK_EXTRACT
            } else {
                PK_SKIP
            };
            let dp = if op == PK_EXTRACT {
                cdest.as_ptr() as *mut c_char
            } else {
                std::ptr::null_mut()
            };
            assert_eq!(ProcessFile(h, op, std::ptr::null_mut(), dp), E_SUCCESS);
        }
        CloseArchive(h);

        let extracted = std::fs::read(&dest).unwrap();
        // Data is sector-padded to 512 bytes; the payload matches.
        assert_eq!(extracted.len(), 512);
        assert_eq!(&extracted[..500], &game[..]);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&dest).ok();
    }
}

#[test]
fn pack_adds_a_file() {
    unsafe {
        let (path, _) = make_test_trd();

        // A host file "ADDED.C" in a unique dir (name kept <= 8 chars so it is
        // not truncated by the TR-DOS filename limit).
        let srcdir = std::env::temp_dir().join(format!(
            "zxwcx-packtest-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&srcdir).unwrap();
        std::fs::write(srcdir.join("ADDED.C"), vec![0x77u8; 260]).unwrap();

        let cpacked = CString::new(path.to_str().unwrap()).unwrap();
        let csrc = CString::new(srcdir.to_str().unwrap()).unwrap();
        // AddList is a double-NUL-terminated list.
        let add_list = b"ADDED.C\0\0";

        let r = PackFiles(
            cpacked.as_ptr() as *mut c_char,
            std::ptr::null_mut(),
            csrc.as_ptr() as *mut c_char,
            add_list.as_ptr() as *mut c_char,
            0,
        );
        assert_eq!(r, E_SUCCESS);

        let names = list_names(&path);
        assert!(names.contains(&"ADDED.C".to_string()), "after pack: {names:?}");

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir_all(&srcdir).ok();
    }
}

#[test]
fn delete_makes_file_recoverable() {
    unsafe {
        let (path, _) = make_test_trd();
        let cpacked = CString::new(path.to_str().unwrap()).unwrap();
        let mut del = b"GAME.C".to_vec();
        del.push(0);
        del.push(0);
        let r = DeleteFiles(cpacked.as_ptr() as *mut c_char, del.as_ptr() as *mut c_char);
        assert_eq!(r, E_SUCCESS);

        let names = list_names(&path);
        // GAME is no longer a live entry, but survives under deleted\.
        assert!(!names.contains(&"GAME.C".to_string()), "still live: {names:?}");
        assert!(
            names.iter().any(|n| n.starts_with("deleted\\") && n.contains("AME")),
            "GAME not recoverable: {names:?}"
        );
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn pack_hobeta_preserves_metadata() {
    unsafe {
        // A hobeta file on disk carrying a known start address and length.
        let orig = TrFile::new(name8("SPRITE"), b'C', 0x7B00, vec![0x33u8; 400]);
        let hob = zxdisk_core::hobeta::wrap(&orig);
        let dir = std::env::temp_dir().join(format!(
            "zxwcx-hob-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SPRITE.$C"), &hob).unwrap();

        // Pack it into a fresh TRD through the plugin.
        let trdpath = tmp_path("trd");
        let cpacked = CString::new(trdpath.to_str().unwrap()).unwrap();
        let csrc = CString::new(dir.to_str().unwrap()).unwrap();
        let add = b"SPRITE.$C\0\0";
        assert_eq!(
            PackFiles(
                cpacked.as_ptr() as *mut c_char,
                std::ptr::null_mut(),
                csrc.as_ptr() as *mut c_char,
                add.as_ptr() as *mut c_char,
                0,
            ),
            E_SUCCESS
        );

        // Read it back with the core lib: metadata must have survived import.
        let trd = TrdImage::from_bytes(&std::fs::read(&trdpath).unwrap()).unwrap();
        let e = trd.entries();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].display_name(), "SPRITE.C");
        assert_eq!(e[0].start, 0x7B00);
        assert_eq!(e[0].length, 400);
        assert_eq!(&e[0].data[..400], &vec![0x33u8; 400][..]);

        std::fs::remove_file(&trdpath).ok();
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
fn content_detection() {
    unsafe {
        let (path, _) = make_test_trd();
        let cpath = CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(CanYouHandleThisFile(cpath.as_ptr() as *mut c_char), 1);

        let junk = tmp_path("dat");
        std::fs::write(&junk, b"this is not a disk image at all, nope").unwrap();
        let cjunk = CString::new(junk.to_str().unwrap()).unwrap();
        assert_eq!(CanYouHandleThisFile(cjunk.as_ptr() as *mut c_char), 0);

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&junk).ok();
    }
}
