//! `zxdisk` - a small command-line tool for ZX Spectrum `.trd`/`.scl` images.
//!
//! It is handy on its own and is the engine behind the "rename from a hotkey"
//! recipe: inside a WCX archive Double Commander exposes a file's full path as
//! `.../image.trd/ENTRY`, which this tool splits into the real image file and the
//! entry name.
//!
//!   zxdisk ls      <image>
//!   zxdisk rename  <image.trd/OLD> <NEW>        (or: rename <image> <OLD> <NEW>)
//!   zxdisk delete  <image.trd/ENTRY>            (or: delete <image> <ENTRY>)
//!   zxdisk extract <image.trd/ENTRY> <outfile>
//!   zxdisk add     <image> <hostfile> [asname]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use zxdisk_core::{Image, TrFile};

fn main() -> ExitCode {
    apply_ext_mode();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[args.len().min(1)..];

    let result = match cmd {
        "ls" | "list" => cmd_ls(rest),
        "rename" | "ren" => cmd_rename(rest),
        "delete" | "del" | "rm" => cmd_delete(rest),
        "extract" | "get" => cmd_extract(rest),
        "add" | "put" => cmd_add(rest),
        "-h" | "--help" | "help" | "" => {
            usage();
            return ExitCode::SUCCESS;
        }
        other => Err(format!("unknown command: {other}")),
    };

    match result {
        Ok(msg) => {
            if !msg.is_empty() {
                println!("{msg}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("zxdisk: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "usage:\n  zxdisk ls <image>\n  zxdisk rename <image.trd/OLD> <NEW>   (or: rename <image> <OLD> <NEW>)\n  zxdisk delete <image.trd/ENTRY>       (or: delete <image> <ENTRY>)\n  zxdisk extract <image.trd/ENTRY> <outfile>\n  zxdisk add <image> <hostfile> [asname]"
    );
}

// --- commands --------------------------------------------------------------

fn cmd_ls(args: &[String]) -> Result<String, String> {
    let image = args.first().ok_or("ls: need an image path")?;
    let img = load_image(Path::new(image))?;
    let mut out = format!("{:?} ({} files)\n", img.format(), img.entries().len());
    for e in img.entries() {
        out.push_str(&format!(
            "{:<12} {:>7} {:>8} {}\n",
            e.display_name(),
            e.start,
            e.length,
            if e.deleted { "deleted" } else { "" }
        ));
    }
    Ok(out.trim_end().to_string())
}

fn cmd_rename(args: &[String]) -> Result<String, String> {
    let (image, old, new) = match args {
        [path, new] => {
            let (img, entry) = split_image(path).ok_or("rename: no .trd/.scl in path")?;
            let entry = entry.ok_or("rename: path has no entry to rename")?;
            (img, entry, new.clone())
        }
        [image, old, new] => (PathBuf::from(image), old.clone(), new.clone()),
        _ => return Err("rename: expected <image.trd/OLD> <NEW>  or  <image> <OLD> <NEW>".into()),
    };
    let mut img = load_image(&image)?;
    if !img.rename_file(&old, &new) {
        return Err(format!("rename: '{old}' not found in {}", image.display()));
    }
    save_image(&image, &img)?;
    Ok(format!("renamed '{old}' -> '{new}'"))
}

fn cmd_delete(args: &[String]) -> Result<String, String> {
    let (image, name) = match args {
        [path] => {
            let (img, entry) = split_image(path).ok_or("delete: no .trd/.scl in path")?;
            (img, entry.ok_or("delete: path has no entry")?)
        }
        [image, name] => (PathBuf::from(image), name.clone()),
        _ => return Err("delete: expected <image.trd/ENTRY>  or  <image> <ENTRY>".into()),
    };
    let mut img = load_image(&image)?;
    if !img.delete_file(&name) {
        return Err(format!("delete: '{name}' not found"));
    }
    save_image(&image, &img)?;
    Ok(format!("deleted '{name}'"))
}

fn cmd_extract(args: &[String]) -> Result<String, String> {
    let (path, out) = match args {
        [path, out] => (path, out),
        _ => return Err("extract: expected <image.trd/ENTRY> <outfile>".into()),
    };
    let (image, entry) = split_image(path).ok_or("extract: no .trd/.scl in path")?;
    let entry = entry.ok_or("extract: path has no entry")?;
    let img = load_image(&image)?;
    let e = img
        .entries()
        .into_iter()
        .find(|e| e.display_name() == entry)
        .ok_or_else(|| format!("extract: '{entry}' not found"))?;
    std::fs::write(out, &e.data).map_err(|e| format!("write failed: {e}"))?;
    Ok(format!("extracted '{entry}' -> {out} ({} bytes)", e.data.len()))
}

fn cmd_add(args: &[String]) -> Result<String, String> {
    let (image, hostfile, asname) = match args {
        [image, hostfile] => (image, hostfile, None),
        [image, hostfile, asname] => (image, hostfile, Some(asname.clone())),
        _ => return Err("add: expected <image> <hostfile> [asname]".into()),
    };
    let data = std::fs::read(hostfile).map_err(|e| format!("read {hostfile}: {e}"))?;
    let basename = asname.unwrap_or_else(|| {
        Path::new(hostfile)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(hostfile)
            .to_string()
    });
    let file = match zxdisk_core::hobeta::parse(&data) {
        Some(f) => f,
        None => TrFile::from_host_filename(&basename, data),
    };
    let mut img = load_image(Path::new(image))?;
    img.add_file(&file).map_err(|e| format!("add failed: {e}"))?;
    save_image(Path::new(image), &img)?;
    Ok(format!("added '{}'", file.display_name()))
}

// --- helpers ---------------------------------------------------------------

fn is_image_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("trd") || e.eq_ignore_ascii_case("scl"))
            .unwrap_or(false)
}

/// Split a path like `/a/b/game.trd/BOOT.B` into (`/a/b/game.trd`, `BOOT.B`).
/// If the path itself is an image file, the entry is `None`.
fn split_image(path: &str) -> Option<(PathBuf, Option<String>)> {
    let p = Path::new(path);
    if is_image_file(p) {
        return Some((p.to_path_buf(), None));
    }
    let mut cur = p;
    while let Some(parent) = cur.parent() {
        if is_image_file(parent) {
            let entry = p.file_name()?.to_string_lossy().into_owned();
            return Some((parent.to_path_buf(), Some(entry)));
        }
        cur = parent;
    }
    None
}

fn load_image(path: &Path) -> Result<Image, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str());
    Image::from_bytes(&bytes, ext).map_err(|e| format!("{}: {e}", path.display()))
}

fn save_image(path: &Path, img: &Image) -> Result<(), String> {
    std::fs::write(path, img.to_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Match the extension mode used by the plugins so entry names line up.
fn apply_ext_mode() {
    // Honour both the generic and the WCX-specific env var name, then the shared
    // zxdisk.conf, so the CLI-driven rename and the WCX plugin listing agree on
    // how entry names are formed.
    let val = std::env::var("ZXDISK_EXT_MODE")
        .ok()
        .or_else(|| std::env::var("ZXDISK_WCX_EXT_MODE").ok())
        .filter(|s| !s.is_empty())
        .or_else(|| shared_conf_paths().iter().find_map(|p| read_ini_key(p, "ext_mode")));
    if let Some(v) = val {
        if let Some(m) = zxdisk_core::ExtMode::parse(&v) {
            zxdisk_core::set_default_ext_mode(m);
        }
    }
}

/// Candidate locations of the shared `zxdisk.conf`, in priority order. Mirrors
/// the WCX plugin's fallbacks (HOME on unix / Git Bash, then the Windows user
/// profile and roaming AppData) so the CLI and the plugin agree on settings.
fn shared_conf_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(h) = std::env::var_os("HOME") {
        out.push(Path::new(&h).join(".config/zxdisk.conf"));
    }
    if let Some(p) = std::env::var_os("USERPROFILE") {
        out.push(Path::new(&p).join(".config/zxdisk.conf"));
    }
    if let Some(a) = std::env::var_os("APPDATA") {
        out.push(Path::new(&a).join("zxdisk/zxdisk.conf"));
    }
    out
}

fn read_ini_key(path: &Path, key: &str) -> Option<String> {
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

// The tests live in src/tests.rs rather than here. They are as long as the
// program, and keeping them in a file of their own means the coverage report
// can leave them out of its own denominator - a suite that counts itself
// flatters the number it prints.
#[cfg(test)]
mod tests;
