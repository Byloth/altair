//! File system: percorsi di Windows sul file system dell'host, e le API sui file.
//!
//! `C:\AC1\…` è la cartella del gioco, in sola lettura. Tutto il resto di `C:\` vive in una
//! cartella scrivibile del progetto (`run/c/`): profili, salvataggi, impostazioni.
//! I nomi sono confrontati senza distinguere maiuscole e minuscole, come su Windows.

use crate::win32::{Call, Runtime};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const GAME_DIR: &str = "C:\\AC1";
const INVALID: u32 = 0xffff_ffff;

pub struct FsState {
    game_dir: PathBuf,
    c_root: PathBuf,
    cwd: String,
    files: HashMap<u32, File>,
    finds: HashMap<u32, Vec<(String, u64, bool)>>,
    quiet: bool,
}

impl FsState {
    pub fn new(game_dir: PathBuf, c_root: PathBuf) -> FsState {
        let _ = std::fs::create_dir_all(&c_root);
        FsState { game_dir, c_root, cwd: GAME_DIR.into(), files: HashMap::new(), finds: HashMap::new(), quiet: std::env::var_os("ALTAIR_QUIET_FS").is_some() }
    }

    /// Percorso guest assoluto e normalizzato, come lista di componenti dopo `C:`.
    fn components(&self, guest: &str) -> Option<Vec<String>> {
        let g = guest.replace('/', "\\");
        let full = if g.len() >= 2 && g.as_bytes()[1] == b':' {
            g
        } else if g.starts_with('\\') {
            format!("C:{g}")
        } else {
            format!("{}\\{}", self.cwd, g)
        };
        if !full[..2].eq_ignore_ascii_case("c:") {
            return None;
        }
        let mut out: Vec<String> = Vec::new();
        for part in full[2..].split('\\') {
            match part {
                "" | "." => {}
                ".." => {
                    out.pop();
                }
                p => out.push(p.to_string()),
            }
        }
        Some(out)
    }

    /// Percorso host di un percorso guest e se è scrivibile.
    pub fn resolve(&self, guest: &str) -> Option<(PathBuf, bool)> {
        let comps = self.components(guest)?;
        let in_game = comps.first().is_some_and(|c| c.eq_ignore_ascii_case("AC1"));
        let (mut path, rest) = if in_game { (self.game_dir.clone(), &comps[1..]) } else { (self.c_root.clone(), &comps[..]) };
        for c in rest {
            let exact = path.join(c);
            if exact.exists() {
                path = exact;
                continue;
            }
            let found = std::fs::read_dir(&path).ok().and_then(|rd| {
                rd.flatten().map(|e| e.file_name()).find(|n| n.to_string_lossy().eq_ignore_ascii_case(c))
            });
            path = match found {
                Some(n) => path.join(n),
                None => exact,
            };
        }
        Some((path, !in_game))
    }

    fn full_guest(&self, guest: &str) -> String {
        match self.components(guest) {
            Some(c) => format!("C:\\{}", c.join("\\")),
            None => guest.to_string(),
        }
    }
}

fn wildcard(pat: &[u8], name: &[u8]) -> bool {
    match (pat.first(), name.first()) {
        (None, None) => true,
        (Some(b'*'), _) => wildcard(&pat[1..], name) || (!name.is_empty() && wildcard(pat, &name[1..])),
        (Some(b'?'), Some(_)) => wildcard(&pat[1..], &name[1..]),
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && wildcard(&pat[1..], &name[1..]),
        _ => false,
    }
}

fn filetime(t: std::io::Result<std::time::SystemTime>) -> u64 {
    let unix = t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).unwrap_or_default();
    (unix.as_nanos() / 100) as u64 + 116_444_736_000_000_000
}

fn fail(c: &Call, err: u32) -> u32 {
    c.set_last_error(err);
    INVALID
}

pub fn create_file_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let name = rt.mem.cstr(c.arg(0));
    let (access, disposition) = (c.arg(1), c.arg(4));
    let write = access & 0x4000_0000 != 0 || matches!(disposition, 1 | 2 | 5);
    let Some((path, writable)) = rt.fs.resolve(&name) else { return fail(c, 3) };
    if write && !writable {
        eprintln!("[file] RIFIUTATA la scrittura nella cartella del gioco: {name}");
        return fail(c, 5);
    }
    let mut o = OpenOptions::new();
    o.read(true);
    if write {
        o.write(true);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    match disposition {
        1 => o.create_new(true),
        2 => o.create(true).truncate(true),
        4 => o.create(write),
        5 => o.truncate(true),
        _ => &mut o,
    };
    match o.open(&path) {
        Ok(f) if path.is_dir() => {
            drop(f);
            fail(c, 5)
        }
        Ok(f) => {
            let h = rt.fresh_handle();
            if !rt.fs.quiet {
                eprintln!("[file] apre {name}{}", if write { " (scrittura)" } else { "" });
            }
            rt.fs.files.insert(h, f);
            h
        }
        Err(e) => {
            if !rt.fs.quiet {
                eprintln!("[file] non trovato: {name} ({})", e.kind());
            }
            fail(c, 2)
        }
    }
}

pub fn read_file(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (h, buf, n, done, ov) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    let mem = rt.mem;
    let Some(f) = rt.fs.files.get_mut(&h) else {
        c.set_last_error(6);
        return 0;
    };
    if ov != 0 {
        // Lettura "sovrapposta": la eseguiamo subito, alla posizione indicata.
        let pos = mem.r32(ov + 8) as u64 | (mem.r32(ov + 12) as u64) << 32;
        let _ = f.seek(SeekFrom::Start(pos));
    }
    // SAFETY: il buffer è memoria guest valida per `n` byte.
    let dst = unsafe { std::slice::from_raw_parts_mut(mem.ptr(buf), n as usize) };
    let mut got = 0;
    while got < dst.len() {
        match f.read(&mut dst[got..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => got += k,
        }
    }
    if done != 0 {
        mem.w32(done, got as u32);
    }
    if ov != 0 {
        mem.w32(ov, 0);
        mem.w32(ov + 4, got as u32);
    }
    1
}

pub fn write_file(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (h, buf, n, done) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let data = rt.mem.bytes(buf, n);
    let ok = match h {
        0x11 | 0x12 => {
            eprint!("{}", String::from_utf8_lossy(data));
            true
        }
        _ => rt.fs.files.get_mut(&h).is_some_and(|f| f.write_all(data).is_ok()),
    };
    if done != 0 {
        rt.mem.w32(done, if ok { n } else { 0 });
    }
    ok as u32
}

pub fn set_file_pointer(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (h, lo, phi, method) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let hi = if phi != 0 { rt.mem.r32(phi) as i32 as i64 } else { (lo as i32 as i64) >> 32 };
    let off = (hi << 32) | lo as i64;
    let Some(f) = rt.fs.files.get_mut(&h) else { return fail(c, 6) };
    let whence = match method {
        0 => SeekFrom::Start(off as u64),
        1 => SeekFrom::Current(off),
        _ => SeekFrom::End(off),
    };
    match f.seek(whence) {
        Ok(p) => {
            if phi != 0 {
                rt.mem.w32(phi, (p >> 32) as u32);
            }
            c.set_last_error(0);
            p as u32
        }
        Err(_) => fail(c, 131),
    }
}

pub fn get_file_size(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(len) = rt.fs.files.get(&c.arg(0)).and_then(|f| f.metadata().ok()).map(|m| m.len()) else { return fail(c, 6) };
    if c.arg(1) != 0 {
        rt.mem.w32(c.arg(1), (len >> 32) as u32);
    }
    len as u32
}

pub fn get_file_size_ex(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(len) = rt.fs.files.get(&c.arg(0)).and_then(|f| f.metadata().ok()).map(|m| m.len()) else { return 0 };
    rt.mem.w64(c.arg(1), len);
    1
}

pub fn flush_file_buffers(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.fs.files.get_mut(&c.arg(0)).is_some_and(|f| f.flush().is_ok()) as u32
}

pub fn set_end_of_file(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.fs.files.get_mut(&c.arg(0)).is_some_and(|f| f.stream_position().is_ok_and(|p| f.set_len(p).is_ok())) as u32
}

/// Chiude un handle di file o di ricerca, se lo è. True se lo era.
pub fn close(rt: &mut Runtime, h: u32) -> bool {
    rt.fs.files.remove(&h).is_some() | rt.fs.finds.remove(&h).is_some()
}

pub fn get_file_attributes_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let name = rt.mem.cstr(c.arg(0));
    match rt.fs.resolve(&name).map(|(p, _)| p) {
        Some(p) if p.is_dir() => 0x10,
        Some(p) if p.exists() => 0x80,
        _ => fail(c, 2),
    }
}

pub fn create_directory_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let name = rt.mem.cstr(c.arg(0));
    match rt.fs.resolve(&name) {
        Some((p, true)) => std::fs::create_dir_all(p).is_ok() as u32,
        _ => 0,
    }
}

/// `SHCreateDirectoryExA(hwnd, path, sa)`: 0 = riuscita.
pub fn sh_create_directory_ex_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let name = rt.mem.cstr(c.arg(1));
    match rt.fs.resolve(&name) {
        Some((p, true)) if std::fs::create_dir_all(&p).is_ok() => 0,
        _ => 5,
    }
}

pub fn delete_file_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let name = rt.mem.cstr(c.arg(0));
    match rt.fs.resolve(&name) {
        Some((p, true)) => std::fs::remove_file(p).is_ok() as u32,
        _ => 0,
    }
}

pub fn get_current_directory_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let cwd = rt.fs.cwd.clone();
    if c.arg(0) > cwd.len() as u32 {
        rt.mem.write_cstr(c.arg(1), cwd.as_bytes());
        cwd.len() as u32
    } else {
        cwd.len() as u32 + 1
    }
}

pub fn set_current_directory_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let name = rt.mem.cstr(c.arg(0));
    rt.fs.cwd = rt.fs.full_guest(&name);
    1
}

pub fn get_full_path_name_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let full = rt.fs.full_guest(&rt.mem.cstr(c.arg(0)));
    if c.arg(1) <= full.len() as u32 {
        return full.len() as u32 + 1;
    }
    rt.mem.write_cstr(c.arg(2), full.as_bytes());
    if c.arg(3) != 0 {
        let name_at = full.rfind('\\').map_or(0, |i| i + 1) as u32;
        rt.mem.w32(c.arg(3), c.arg(2) + name_at);
    }
    full.len() as u32
}

/// Cartelle speciali (`CSIDL_*`), tutte sotto `C:\Users\Player`.
pub fn sh_get_special_folder_path_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let path = match c.arg(2) & 0xff {
        0x05 => "C:\\Users\\Player\\Documents",
        0x1a => "C:\\Users\\Player\\AppData\\Roaming",
        0x1c => "C:\\Users\\Player\\AppData\\Local",
        0x23 => "C:\\ProgramData",
        0x28 => "C:\\Users\\Player",
        other => {
            eprintln!("SHGetSpecialFolderPathA: CSIDL {other:#x} non previsto, uso Documents");
            "C:\\Users\\Player\\Documents"
        }
    };
    if let Some((host, true)) = rt.fs.resolve(path) {
        let _ = std::fs::create_dir_all(host);
    }
    rt.mem.write_cstr(c.arg(1), path.as_bytes());
    1
}

fn write_find_data(rt: &Runtime, p: u32, entry: &(String, u64, bool)) {
    let (name, size, dir) = entry;
    rt.mem.fill(p, 320, 0);
    rt.mem.w32(p, if *dir { 0x10 } else { 0x80 });
    rt.mem.w32(p + 28, (size >> 32) as u32);
    rt.mem.w32(p + 32, *size as u32);
    let n = name.len().min(259);
    rt.mem.write_cstr(p + 44, &name.as_bytes()[..n]);
}

pub fn find_first_file_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let pattern = rt.mem.cstr(c.arg(0));
    let (dir, pat) = match pattern.rfind(['\\', '/']) {
        Some(i) => (&pattern[..i], &pattern[i + 1..]),
        None => (".", pattern.as_str()),
    };
    let Some((host, _)) = rt.fs.resolve(dir) else { return fail(c, 3) };
    let mut entries: Vec<(String, u64, bool)> = std::fs::read_dir(&host)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let md = e.metadata().ok()?;
                    wildcard(pat.as_bytes(), name.as_bytes()).then(|| (name, md.len(), md.is_dir()))
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    entries.reverse();
    if !rt.fs.quiet {
        eprintln!("[file] cerca {pattern}: {} risultati", entries.len());
    }
    let Some(first) = entries.pop() else { return fail(c, 2) };
    write_find_data(rt, c.arg(1), &first);
    let h = rt.fresh_handle();
    rt.fs.finds.insert(h, entries);
    h
}

pub fn find_next_file_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    match rt.fs.finds.get_mut(&c.arg(0)).and_then(Vec::pop) {
        Some(e) => {
            write_find_data(rt, c.arg(1), &e);
            1
        }
        None => {
            c.set_last_error(18); // ERROR_NO_MORE_FILES
            0
        }
    }
}

pub fn find_close(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.fs.finds.remove(&c.arg(0));
    1
}

pub fn get_file_time(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(md) = rt.fs.files.get(&c.arg(0)).and_then(|f| f.metadata().ok()) else { return 0 };
    let t = filetime(md.modified());
    for i in 1..=3 {
        if c.arg(i) != 0 {
            rt.mem.w64(c.arg(i), t);
        }
    }
    1
}

pub fn get_drive_type_a(_: &mut Runtime, _: &mut Call) -> u32 {
    3 // DRIVE_FIXED
}

#[allow(dead_code)]
pub fn host_path(rt: &Runtime, guest: &str) -> Option<PathBuf> {
    rt.fs.resolve(guest).map(|(p, _)| p)
}

#[allow(dead_code)]
fn _unused(_: &Path) {}

pub fn get_logical_drive_strings_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    if c.arg(0) >= 5 {
        rt.mem.write(c.arg(1), b"C:\\\0\0");
    }
    4
}
