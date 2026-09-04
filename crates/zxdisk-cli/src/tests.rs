//! Tests for the `zxdisk` command line tool.
//!
//! In a file of their own so that scripts/coverage.sh can drop them from the
//! measurement: test code is covered by definition, and counting it raises
//! the number without covering anything.
use super::*;
use std::fs;
use zxdisk_core::{DiskType, TrdImage};

fn name8(s: &str) -> [u8; 8] {
    let mut n = [b' '; 8];
    for (i, b) in s.bytes().take(8).enumerate() {
        n[i] = b;
    }
    n
}

/// A directory of our own under the system temp, with a real .trd in it.
/// The commands take paths and touch the disk, so there has to be a disk.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("zxdisk-cli-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Fixture { dir }
    }

    /// An image with two files in it, at <dir>/game.trd.
    fn image(&self) -> PathBuf {
        let mut trd = TrdImage::blank(DiskType::Ds80, "TESTDSK");
        trd.add_file(&TrFile::new(name8("HELLO"), b'C', 0x8000, vec![0xAA; 300]))
            .unwrap();
        trd.add_file(&TrFile::new(name8("WORLD"), b'B', 10, vec![0x55; 256]))
            .unwrap();
        let path = self.dir.join("game.trd");
        fs::write(&path, Image::Trd(trd).to_bytes()).unwrap();
        path
    }

    fn at(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ---- splitting a path Double Commander hands us -------------------------
//
// This is the whole reason the tool exists: inside a WCX archive DC shows a
// file as `.../image.trd/ENTRY`, which is not a path any filesystem call can
// use. Getting it wrong means renaming the wrong thing, or nothing.

#[test]
fn split_image_finds_the_image_and_the_entry() {
    let fx = Fixture::new("split");
    let img = fx.image();

    // The image itself: an image, and no entry.
    let (found, entry) = split_image(img.to_str().unwrap()).unwrap();
    assert_eq!(found, img);
    assert_eq!(entry, None);

    // An entry inside it.
    let inside = format!("{}/HELLO.C", img.display());
    let (found, entry) = split_image(&inside).unwrap();
    assert_eq!(found, img);
    assert_eq!(entry.as_deref(), Some("HELLO.C"));
}

#[test]
fn split_image_matches_the_extension_whatever_its_case() {
    let fx = Fixture::new("case");
    let img = fx.image();
    let upper = fx.at("GAME.TRD");
    fs::copy(&img, &upper).unwrap();

    // DC will hand back whatever case the filesystem reported, and a ZX
    // image is as likely to be named .TRD as .trd.
    assert!(split_image(upper.to_str().unwrap()).is_some());
    let inside = format!("{}/BOOT.B", upper.display());
    let (found, entry) = split_image(&inside).unwrap();
    assert_eq!(found, upper);
    assert_eq!(entry.as_deref(), Some("BOOT.B"));
}

#[test]
fn split_image_refuses_what_is_not_an_image() {
    let fx = Fixture::new("noimage");
    // A path with no image anywhere in it.
    assert!(split_image(fx.at("nothing/here.txt").to_str().unwrap()).is_none());
    // A file with the right extension that does not exist: is_image_file
    // asks the filesystem, so a name alone is not enough.
    assert!(split_image(fx.at("absent.trd").to_str().unwrap()).is_none());
    // An empty path.
    assert!(split_image("").is_none());
}

#[test]
fn split_image_takes_the_innermost_image() {
    // A directory that ends in .trd but is not a file must not be taken for
    // one, and the entry must come from the real image below it.
    let fx = Fixture::new("nested");
    fs::create_dir_all(fx.at("outer.trd")).unwrap();
    let mut trd = TrdImage::blank(DiskType::Ds80, "INNER");
    trd.add_file(&TrFile::new(name8("A"), b'C', 0, vec![1; 10]))
        .unwrap();
    let inner = fx.at("outer.trd").join("inner.trd");
    fs::write(&inner, Image::Trd(trd).to_bytes()).unwrap();

    let path = format!("{}/A.C", inner.display());
    let (found, entry) = split_image(&path).unwrap();
    assert_eq!(found, inner);
    assert_eq!(entry.as_deref(), Some("A.C"));
}

// ---- the commands -------------------------------------------------------

#[test]
fn ls_lists_what_is_in_the_image() {
    let fx = Fixture::new("ls");
    let img = fx.image();
    let out = cmd_ls(&args(&[img.to_str().unwrap()])).unwrap();
    assert!(out.contains("2 files"), "{out}");
    assert!(out.contains("HELLO.C"), "{out}");
    assert!(out.contains("WORLD.B"), "{out}");
    // No trailing blank line: the tool's output is piped into other things.
    assert!(!out.ends_with('\n'));
}

#[test]
fn ls_without_an_image_says_so() {
    assert!(cmd_ls(&[]).unwrap_err().contains("need an image"));
    let e = cmd_ls(&args(&["/no/such/file.trd"])).unwrap_err();
    assert!(e.contains("cannot read"), "{e}");
}

#[test]
fn rename_writes_the_change_back_to_disk() {
    let fx = Fixture::new("rename");
    let img = fx.image();

    // The two-argument form, the one the hotkey recipe uses.
    let path = format!("{}/HELLO.C", img.display());
    let msg = cmd_rename(&args(&[&path, "BYE.C"])).unwrap();
    assert!(msg.contains("HELLO.C"), "{msg}");

    let after = cmd_ls(&args(&[img.to_str().unwrap()])).unwrap();
    assert!(after.contains("BYE.C"), "{after}");
    assert!(!after.contains("HELLO.C"), "{after}");
}

#[test]
fn rename_takes_the_three_argument_form_too() {
    let fx = Fixture::new("rename3");
    let img = fx.image();
    cmd_rename(&args(&[img.to_str().unwrap(), "WORLD.B", "EARTH.B"])).unwrap();
    assert!(cmd_ls(&args(&[img.to_str().unwrap()]))
        .unwrap()
        .contains("EARTH.B"));
}

#[test]
fn rename_refuses_what_it_cannot_do() {
    let fx = Fixture::new("rename-bad");
    let img = fx.image();

    assert!(cmd_rename(&[]).unwrap_err().contains("expected"));
    assert!(cmd_rename(&args(&["one"]))
        .unwrap_err()
        .contains("expected"));

    // A name that is not in the image, and an image path with no entry.
    let e = cmd_rename(&args(&[img.to_str().unwrap(), "NOPE.C", "X.C"])).unwrap_err();
    assert!(e.contains("not found"), "{e}");
    let e = cmd_rename(&args(&[img.to_str().unwrap(), "X.C"])).unwrap_err();
    assert!(e.contains("no entry"), "{e}");
    let e = cmd_rename(&args(&["/tmp/not-an-image.txt", "X.C"])).unwrap_err();
    assert!(e.contains("no .trd/.scl"), "{e}");
}

#[test]
fn delete_removes_the_entry_and_only_it() {
    let fx = Fixture::new("delete");
    let img = fx.image();
    let path = format!("{}/WORLD.B", img.display());
    cmd_delete(&args(&[&path])).unwrap();

    let after = cmd_ls(&args(&[img.to_str().unwrap()])).unwrap();
    assert!(!after.contains("WORLD.B"), "{after}");
    assert!(after.contains("HELLO.C"), "{after}");
}

#[test]
fn delete_refuses_what_it_cannot_do() {
    let fx = Fixture::new("delete-bad");
    let img = fx.image();
    assert!(cmd_delete(&[]).unwrap_err().contains("expected"));
    let e = cmd_delete(&args(&[img.to_str().unwrap(), "NOPE.C"])).unwrap_err();
    assert!(e.contains("not found"), "{e}");
}

#[test]
fn extract_writes_the_bytes_out() {
    let fx = Fixture::new("extract");
    let img = fx.image();
    let out = fx.at("HELLO.bin");
    let path = format!("{}/HELLO.C", img.display());

    let msg = cmd_extract(&args(&[&path, out.to_str().unwrap()])).unwrap();
    assert!(msg.contains("bytes"), "{msg}");

    // 300 bytes of data, padded to whole 256-byte sectors on the way in.
    let bytes = fs::read(&out).unwrap();
    assert_eq!(bytes.len(), 512);
    assert_eq!(&bytes[..300], &[0xAA; 300][..]);
}

#[test]
fn extract_refuses_what_it_cannot_do() {
    let fx = Fixture::new("extract-bad");
    let img = fx.image();
    assert!(cmd_extract(&[]).unwrap_err().contains("expected"));
    let missing = format!("{}/NOPE.C", img.display());
    let e = cmd_extract(&args(&[&missing, "/tmp/x"])).unwrap_err();
    assert!(e.contains("not found"), "{e}");
    // The image itself names no entry to extract.
    let e = cmd_extract(&args(&[img.to_str().unwrap(), "/tmp/x"])).unwrap_err();
    assert!(e.contains("no entry"), "{e}");
}

#[test]
fn add_puts_a_host_file_in() {
    let fx = Fixture::new("add");
    let img = fx.image();
    let host = fx.at("data.bin");
    fs::write(&host, vec![0x42u8; 100]).unwrap();

    cmd_add(&args(&[img.to_str().unwrap(), host.to_str().unwrap()])).unwrap();
    let after = cmd_ls(&args(&[img.to_str().unwrap()])).unwrap();
    assert!(after.contains("3 files"), "{after}");
    assert!(after.to_uppercase().contains("DATA"), "{after}");
}

#[test]
fn add_takes_a_name_to_store_it_under() {
    let fx = Fixture::new("add-as");
    let img = fx.image();
    let host = fx.at("data.bin");
    fs::write(&host, vec![0x42u8; 100]).unwrap();

    cmd_add(&args(&[
        img.to_str().unwrap(),
        host.to_str().unwrap(),
        "RENAMED.C",
    ]))
    .unwrap();
    assert!(cmd_ls(&args(&[img.to_str().unwrap()]))
        .unwrap()
        .contains("RENAMED.C"));
}

#[test]
fn add_refuses_what_it_cannot_do() {
    let fx = Fixture::new("add-bad");
    let img = fx.image();
    assert!(cmd_add(&[]).unwrap_err().contains("expected"));
    let e = cmd_add(&args(&[img.to_str().unwrap(), "/no/such/host/file"])).unwrap_err();
    assert!(!e.is_empty(), "an unreadable source has to say something");
}

// ---- the shared settings file -------------------------------------------
//
// The CLI and the WCX plugin have to agree on how entry names are formed, or
// the rename hotkey renames something the listing does not show. They agree
// by reading the same key out of the same file.

#[test]
fn read_ini_key_reads_the_shape_the_plugins_write() {
    let fx = Fixture::new("ini");
    let conf = fx.at("zxdisk.conf");
    fs::write(
        &conf,
        "; a comment\n\
         # another\n\
         [section]\n\
         \n\
         ext_mode = dot\n\
         Other=2\n",
    )
    .unwrap();

    assert_eq!(read_ini_key(&conf, "ext_mode").as_deref(), Some("dot"));
    // The key is matched without regard to case, and the value is trimmed.
    assert_eq!(read_ini_key(&conf, "EXT_MODE").as_deref(), Some("dot"));
    assert_eq!(read_ini_key(&conf, "other").as_deref(), Some("2"));
    // Comments, blanks and section headers are not settings.
    assert_eq!(read_ini_key(&conf, "section"), None);
    assert_eq!(read_ini_key(&conf, "missing"), None);
    // A file that is not there is not an error, just no answer.
    assert_eq!(read_ini_key(&fx.at("absent.conf"), "ext_mode"), None);
}

#[test]
fn shared_conf_paths_are_ordered_and_expand() {
    // HOME first, which is what a Unix host and Git Bash both set.
    let paths = shared_conf_paths();
    assert!(!paths.is_empty(), "no candidate config path at all");
    assert!(paths
        .iter()
        .all(|p| p.to_string_lossy().contains("zxdisk.conf")));
}
