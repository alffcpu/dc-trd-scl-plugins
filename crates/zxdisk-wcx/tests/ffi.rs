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
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
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
        assert_eq!(
            ProcessFile(h, PK_SKIP, std::ptr::null_mut(), std::ptr::null_mut()),
            E_SUCCESS
        );
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
        assert!(
            names.contains(&"ADDED.C".to_string()),
            "after pack: {names:?}"
        );

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
        assert!(
            !names.contains(&"GAME.C".to_string()),
            "still live: {names:?}"
        );
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("deleted\\") && n.contains("AME")),
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

/// An unreadable existing image must not be replaced by a blank one.
///
/// This is the July review's H2, the one data-loss path in the plugin: `do_pack`
/// used to treat *any* read failure as "no such image, make a new one", so a
/// sharing violation or a permission problem turned the user's disk image into
/// an empty one on the next copy-in. The fix distinguishes NotFound from
/// everything else; nothing checked that it stayed fixed.
///
/// Unix only: it needs a file the process cannot read, and dropping the mode
/// bits is the portable way to get one. Windows has the same code path and
/// reaches it through a sharing violation instead, which a test cannot arrange
/// as simply.
#[test]
#[cfg(unix)]
fn packing_into_an_unreadable_image_refuses_rather_than_blanking_it() {
    use std::os::unix::fs::PermissionsExt;

    unsafe {
        let (path, _) = make_test_trd();
        let before = std::fs::read(&path).unwrap();
        assert!(before.iter().any(|&b| b != 0), "the fixture is not blank");

        let srcdir = std::env::temp_dir().join(format!(
            "zxwcx-unreadable-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&srcdir).unwrap();
        std::fs::write(srcdir.join("ADDED.C"), vec![0x77u8; 260]).unwrap();

        // Readable by nobody, including us.
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let cpacked = CString::new(path.to_str().unwrap()).unwrap();
        let csrc = CString::new(srcdir.to_str().unwrap()).unwrap();
        let add_list = b"ADDED.C\0\0";
        let r = PackFiles(
            cpacked.as_ptr() as *mut c_char,
            std::ptr::null_mut(),
            csrc.as_ptr() as *mut c_char,
            add_list.as_ptr() as *mut c_char,
            0,
        );

        // Put the mode back before asserting, so a failure does not leave an
        // unreadable file behind for the next run.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();

        assert_eq!(
            r, E_EOPEN,
            "an unreadable image must be an error, not a new disk"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the image on disk was modified"
        );

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir_all(&srcdir).ok();
    }
}

/// A path that is not there at all IS the "make a new image" case, which is the
/// other half of the same decision: refusing everything would break copy-in to
/// a new disk, which is a feature people use.
#[test]
fn packing_into_a_path_that_does_not_exist_creates_the_image() {
    unsafe {
        let fresh = std::env::temp_dir().join(format!(
            "zxwcx-fresh-{}-{}.trd",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::remove_file(&fresh).ok();

        let srcdir = std::env::temp_dir().join(format!(
            "zxwcx-freshsrc-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&srcdir).unwrap();
        std::fs::write(srcdir.join("NEW.C"), vec![0x11u8; 300]).unwrap();

        let cpacked = CString::new(fresh.to_str().unwrap()).unwrap();
        let csrc = CString::new(srcdir.to_str().unwrap()).unwrap();
        let add_list = b"NEW.C\0\0";
        let r = PackFiles(
            cpacked.as_ptr() as *mut c_char,
            std::ptr::null_mut(),
            csrc.as_ptr() as *mut c_char,
            add_list.as_ptr() as *mut c_char,
            0,
        );
        assert_eq!(r, E_SUCCESS);
        assert!(fresh.is_file(), "no image was created");
        assert!(list_names(&fresh).contains(&"NEW.C".to_string()));

        std::fs::remove_file(&fresh).ok();
        std::fs::remove_dir_all(&srcdir).ok();
    }
}

/// F6 "move to archive": the plugin owns deleting the originals, and only once
/// they are safely inside the image. Untested until now, and it deletes the
/// user's files - the one operation here that cannot be undone from the file
/// manager.
#[test]
fn move_to_archive_deletes_the_sources_and_only_after_writing_them() {
    unsafe {
        let (path, _) = make_test_trd();
        let srcdir = std::env::temp_dir().join(format!(
            "zxwcx-move-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&srcdir).unwrap();
        let moved = srcdir.join("MOVED.C");
        let kept = srcdir.join("KEPT.C");
        std::fs::write(&moved, vec![0x33u8; 260]).unwrap();
        std::fs::write(&kept, vec![0x44u8; 260]).unwrap();

        let cpacked = CString::new(path.to_str().unwrap()).unwrap();
        let csrc = CString::new(srcdir.to_str().unwrap()).unwrap();
        let add_list = b"MOVED.C\0\0";
        let r = PackFiles(
            cpacked.as_ptr() as *mut c_char,
            std::ptr::null_mut(),
            csrc.as_ptr() as *mut c_char,
            add_list.as_ptr() as *mut c_char,
            PK_PACK_MOVE_FILES,
        );
        assert_eq!(r, E_SUCCESS);

        // In the image...
        assert!(list_names(&path).contains(&"MOVED.C".to_string()));
        // ...and gone from disk, but only the one that was asked for.
        assert!(!moved.exists(), "the source was not removed");
        assert!(kept.exists(), "a file nobody named was removed");

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir_all(&srcdir).ok();
    }
}

/// Without the flag the sources stay. The two halves are one decision and both
/// have to be pinned, or a wrong flag test passes by doing nothing.
#[test]
fn packing_without_the_move_flag_leaves_the_sources_alone() {
    unsafe {
        let (path, _) = make_test_trd();
        let srcdir = std::env::temp_dir().join(format!(
            "zxwcx-copy-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&srcdir).unwrap();
        let src = srcdir.join("COPIED.C");
        std::fs::write(&src, vec![0x55u8; 260]).unwrap();

        let cpacked = CString::new(path.to_str().unwrap()).unwrap();
        let csrc = CString::new(srcdir.to_str().unwrap()).unwrap();
        let add_list = b"COPIED.C\0\0";
        let r = PackFiles(
            cpacked.as_ptr() as *mut c_char,
            std::ptr::null_mut(),
            csrc.as_ptr() as *mut c_char,
            add_list.as_ptr() as *mut c_char,
            0,
        );
        assert_eq!(r, E_SUCCESS);
        assert!(src.exists(), "a plain copy deleted the source");

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir_all(&srcdir).ok();
    }
}

/// A disk with no room left has to say so. This is the error the user actually
/// meets - a 640K floppy fills quickly - and the mapping from the library's
/// DiskFull to the code Double Commander shows was untested.
#[test]
fn a_full_disk_is_reported_as_a_write_error_and_changes_nothing() {
    unsafe {
        // A small geometry, filled until the library refuses.
        let mut trd = TrdImage::blank(DiskType::Ss40, "FULLDISK");
        let mut n = 0;
        while trd
            .add_file(&TrFile::new(
                name8(&format!("F{n}")),
                b'C',
                0,
                vec![0x66u8; 4096],
            ))
            .is_ok()
        {
            n += 1;
            assert!(n < 500, "this disk never filled up");
        }
        assert!(n > 0, "nothing fitted on the disk at all");
        let path = tmp_path("trd");
        std::fs::write(&path, trd.to_bytes()).unwrap();
        let before = std::fs::read(&path).unwrap();

        let srcdir = std::env::temp_dir().join(format!(
            "zxwcx-full-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&srcdir).unwrap();
        std::fs::write(srcdir.join("ONEMORE.C"), vec![0x77u8; 4096]).unwrap();

        let cpacked = CString::new(path.to_str().unwrap()).unwrap();
        let csrc = CString::new(srcdir.to_str().unwrap()).unwrap();
        let add_list = b"ONEMORE.C\0\0";
        let r = PackFiles(
            cpacked.as_ptr() as *mut c_char,
            std::ptr::null_mut(),
            csrc.as_ptr() as *mut c_char,
            add_list.as_ptr() as *mut c_char,
            0,
        );

        assert_eq!(r, E_EWRITE, "a full disk must be a write error");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the image was written despite the failure"
        );

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir_all(&srcdir).ok();
    }
}

/// The ANSI ReadHeader, which 32-bit Total Commander uses. It is a second copy
/// of ReadHeaderEx with a narrower struct, so it can drift from it - and only
/// the Ex one had a test.
#[test]
fn the_ansi_read_header_reports_the_same_entries_as_the_wide_one() {
    unsafe {
        let (path, _) = make_test_trd();
        let wide = list_names(&path);

        let cpath = CString::new(path.to_str().unwrap()).unwrap();
        let mut oad: tOpenArchiveData = std::mem::zeroed();
        oad.ArcName = cpath.as_ptr() as *mut c_char;
        oad.OpenMode = PK_OM_LIST;
        let h = OpenArchive(&mut oad);
        assert!(!h.is_null(), "OpenArchive failed: {}", oad.OpenResult);

        let mut narrow = Vec::new();
        loop {
            let mut hdr: tHeaderData = std::mem::zeroed();
            let r = ReadHeader(h, &mut hdr);
            if r == E_END_ARCHIVE {
                break;
            }
            assert_eq!(r, E_SUCCESS, "ReadHeader failed");
            assert!(hdr.UnpSize > 0, "an entry with no size");
            narrow.push(carray_str(&hdr.FileName));
            assert_eq!(
                ProcessFile(h, PK_SKIP, std::ptr::null_mut(), std::ptr::null_mut()),
                E_SUCCESS
            );
            assert!(narrow.len() < 100, "ReadHeader never ended");
        }
        CloseArchive(h);

        assert_eq!(narrow, wide, "the two headers disagree about the contents");

        // A null handle or header is a refusal, not a dereference: DC has been
        // known to call these after a failed open.
        let mut hdr: tHeaderData = std::mem::zeroed();
        assert_eq!(ReadHeader(std::ptr::null_mut(), &mut hdr), E_BAD_ARCHIVE);

        std::fs::remove_file(&path).ok();
    }
}
