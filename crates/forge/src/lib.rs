//! Reading the `.forge` archives of Assassin's Creed (2007), container version 25.
//!
//! A Rust rewrite of `tools/forge.py`, which remains the reference it is compared against
//! (`altair-forge digest` versus `forge.py digest`). The format is described in
//! `docs/analysis/forge.md`; in short:
//!
//! ```text
//! Header      "scimitar\0", u32 version (25), u64 offset of HeaderFat, …
//! HeaderFat   u32 number of files, u32, u64, u64, u32, u32 number of tables, u64 first table
//! Table       u32 entries, u32, u64 offset of the entries, u64 next table (-1 = end), …
//! Entry       u64 offset of the record, u32 id, u32 size of the data
//! Record      "FILEDATA", char[128] name, … 0x1B8 bytes in all; then the data
//! Section     u64 marker, u16 version, u8 codec (1 = LZO1X), u16, u16, u16 chunks,
//!             chunks × (u16 decompressed, u16 on disk), then per chunk u32 checksum + data
//! ```
//!
//! Entry names and extracted data are game content: they do not go into the repository.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Marker of a compressed section.
pub const MARK: [u8; 8] = [0x33, 0xaa, 0xfb, 0x57, 0x99, 0xfa, 0x04, 0x10];
/// Size of the record that precedes an entry's data.
pub const RECORD: u64 = 0x1b8;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The file does not have the expected structure.
    Format(String),
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Format(m) => write!(f, "{m}"),
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;

fn bad<T>(what: impl Into<String>) -> Result<T> {
    Err(Error::Format(what.into()))
}

/// An entry of the archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Offset of the `FILEDATA` record.
    pub record: u64,
    pub id: u32,
    /// Size of the data on disk, after the record.
    pub size: u32,
}

/// A chunk of a section: decompressed bytes, bytes on disk, offset of the data.
#[derive(Clone, Copy, Debug)]
pub struct Chunk {
    pub unpacked: u16,
    pub stored: u16,
    pub offset: u64,
}

/// A compressed section of an entry's data.
#[derive(Clone, Debug)]
pub struct Section {
    pub codec: u8,
    pub chunks: Vec<Chunk>,
}

pub struct Forge {
    file: File,
    pub version: u32,
    pub file_count: u32,
    pub entries: Vec<Entry>,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}

impl Forge {
    pub fn open(path: impl AsRef<Path>) -> Result<Forge> {
        let mut forge = Forge { file: File::open(path)?, version: 0, file_count: 0, entries: Vec::new() };
        let head = forge.read(0, 0x1a)?;
        if &head[..9] != b"scimitar\0" {
            return bad("not a forge archive");
        }
        forge.version = u32_at(&head, 9);
        let fat = forge.read(u64_at(&head, 13), 0x28)?;
        forge.file_count = u32_at(&fat, 0);
        let mut table = u64_at(&fat, 0x20);
        while table != 0 && table != u64::MAX {
            let t = forge.read(table, 0x18)?;
            let (count, entries_at, next) = (u32_at(&t, 0) as usize, u64_at(&t, 8), u64_at(&t, 16));
            let raw = forge.read(entries_at, 16 * count)?;
            forge.entries.extend(raw.chunks_exact(16).map(|e| Entry { record: u64_at(e, 0), id: u32_at(e, 8), size: u32_at(e, 12) }));
            table = next;
        }
        Ok(forge)
    }

    /// Reads `len` bytes from `offset`; fewer, if the file ends first.
    pub fn read(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut buf = Vec::with_capacity(len);
        (&mut self.file).take(len as u64).read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn read_exact(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let buf = self.read(offset, len)?;
        if buf.len() != len {
            return bad(format!("end of file at {offset:#x}"));
        }
        Ok(buf)
    }

    /// The name of an entry, or `None` if the record does not start with `FILEDATA`.
    pub fn name(&mut self, entry: &Entry) -> Result<Option<String>> {
        let r = self.read(entry.record, 8 + 128)?;
        if r.len() < 8 || &r[..8] != b"FILEDATA" {
            return Ok(None);
        }
        // Latin-1: each byte is the character with the same code.
        Ok(Some(r[8..].iter().take_while(|b| **b != 0).map(|b| *b as char).collect()))
    }

    /// The compressed sections of an entry; empty if the data is raw.
    pub fn sections(&mut self, entry: &Entry) -> Result<Vec<Section>> {
        let mut pos = entry.record + RECORD;
        let end = pos + entry.size as u64;
        let mut out = Vec::new();
        while pos + 15 <= end && self.read(pos, 8)? == MARK {
            let head = self.read_exact(pos + 8, 9)?;
            let (codec, count) = (head[2], u16_at(&head, 7) as usize);
            let sizes = self.read_exact(pos + 17, 4 * count)?;
            let mut p = pos + 17 + 4 * count as u64;
            let mut chunks = Vec::with_capacity(count);
            for i in 0..count {
                let (unpacked, stored) = (u16_at(&sizes, 4 * i), u16_at(&sizes, 4 * i + 2));
                chunks.push(Chunk { unpacked, stored, offset: p + 4 });
                p += 4 + stored as u64;
            }
            out.push(Section { codec, chunks });
            pos = p;
        }
        Ok(out)
    }

    /// The decompressed data of a section.
    pub fn decompress(&mut self, section: &Section) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(section.chunks.iter().map(|c| c.unpacked as usize).sum());
        for c in &section.chunks {
            let raw = self.read_exact(c.offset, c.stored as usize)?;
            if c.stored == c.unpacked {
                out.extend_from_slice(&raw);
            } else {
                lzo1x_decompress(&raw, c.unpacked as usize, &mut out)?;
            }
        }
        Ok(out)
    }

    /// The raw data of an entry (for those without sections).
    pub fn raw(&mut self, entry: &Entry) -> Result<Vec<u8>> {
        self.read_exact(entry.record + RECORD, entry.size as usize)
    }
}

/// Decompresses an LZO1X block (public format) onto the end of `out`; it must produce `expect` bytes.
pub fn lzo1x_decompress(src: &[u8], expect: usize, out: &mut Vec<u8>) -> Result<()> {
    let base = out.len();
    let mut ip = 0usize;
    let get = |i: usize| -> Result<usize> { src.get(i).map(|b| *b as usize).ok_or(Error::Format("LZO: truncated data".into())) };
    // Copies `n` bytes from `dist` bytes back; the ranges may overlap.
    fn copy(out: &mut Vec<u8>, base: usize, dist: usize, n: usize) -> Result<()> {
        if dist == 0 || dist > out.len() - base {
            return bad("LZO: distance outside the buffer");
        }
        let mut p = out.len() - dist;
        for _ in 0..n {
            out.push(out[p]);
            p += 1;
        }
        Ok(())
    }
    fn literals(src: &[u8], ip: &mut usize, n: usize, out: &mut Vec<u8>) -> Result<()> {
        let s = src.get(*ip..*ip + n).ok_or(Error::Format("LZO: truncated data".into()))?;
        out.extend_from_slice(s);
        *ip += n;
        Ok(())
    }
    /// Extended length: a run of zeros is worth 255 each, then one byte.
    fn extend(src: &[u8], ip: &mut usize, bias: usize) -> Result<usize> {
        let mut t = 0;
        loop {
            let b = *src.get(*ip).ok_or(Error::Format("LZO: truncated data".into()))? as usize;
            *ip += 1;
            if b != 0 {
                return Ok(t + bias + b);
            }
            t += 255;
        }
    }
    #[derive(PartialEq)]
    enum State {
        Top,
        LiteralRun,
        Match,
        MatchDone,
        MatchNext,
    }
    let mut t = get(0)?;
    let mut state = State::Top;
    if t > 17 {
        ip = 1;
        t -= 17;
        state = if t < 4 { State::MatchNext } else { State::LiteralRun };
    }
    loop {
        if state == State::Top {
            t = get(ip)?;
            ip += 1;
            if t >= 16 {
                state = State::Match;
            } else {
                if t == 0 {
                    t = extend(src, &mut ip, 15)?;
                }
                t += 3;
                state = State::LiteralRun;
            }
        }
        if state == State::LiteralRun {
            literals(src, &mut ip, t, out)?;
            t = get(ip)?;
            ip += 1;
            if t >= 16 {
                state = State::Match;
            } else {
                copy(out, base, 1 + 0x800 + (t >> 2) + (get(ip)? << 2), 3)?;
                ip += 1;
                state = State::MatchDone;
            }
        }
        if state == State::Match {
            if t >= 64 {
                copy(out, base, 1 + ((t >> 2) & 7) + (get(ip)? << 3), (t >> 5) + 1)?;
                ip += 1;
            } else if t >= 32 {
                t &= 31;
                if t == 0 {
                    t = extend(src, &mut ip, 31)?;
                }
                let d = 1 + (get(ip)? >> 2) + (get(ip + 1)? << 6);
                ip += 2;
                copy(out, base, d, t + 2)?;
            } else if t >= 16 {
                let mut d = (t & 8) << 11;
                t &= 7;
                if t == 0 {
                    t = extend(src, &mut ip, 7)?;
                }
                d += (get(ip)? >> 2) + (get(ip + 1)? << 6);
                ip += 2;
                if d == 0 {
                    break; // end of stream
                }
                copy(out, base, d + 0x4000, t + 2)?;
            } else {
                copy(out, base, 1 + (t >> 2) + (get(ip)? << 2), 2)?;
                ip += 1;
            }
            state = State::MatchDone;
        }
        if state == State::MatchDone {
            t = get(ip - 2)? & 3;
            state = if t != 0 { State::MatchNext } else { State::Top };
        }
        if state == State::MatchNext {
            literals(src, &mut ip, t, out)?;
            t = get(ip)?;
            ip += 1;
            state = State::Match;
        }
    }
    if out.len() - base != expect {
        return bad(format!("LZO: {} bytes instead of {expect}", out.len() - base));
    }
    Ok(())
}

/// CRC-32 (IEEE, like zlib): it labels the object classes and is used for comparisons.
pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            c
        })
    });
    !data.iter().fold(!crc, |c, b| table[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_zlib() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32_update(crc32(b"1234"), b"56789"), 0xcbf4_3926);
    }

    #[test]
    fn lzo_literal_only_stream() {
        // A block of literals only: initial byte 17 + n, the literals, then the end of stream 11 00 00.
        let mut src = vec![17 + 5];
        src.extend_from_slice(b"ciao!");
        src.extend_from_slice(&[0x11, 0, 0]);
        let mut out = Vec::new();
        lzo1x_decompress(&src, 5, &mut out).unwrap();
        assert_eq!(out, b"ciao!");
    }

    #[test]
    fn lzo_rejects_wrong_length() {
        let mut src = vec![17 + 5];
        src.extend_from_slice(b"ciao!");
        src.extend_from_slice(&[0x11, 0, 0]);
        assert!(lzo1x_decompress(&src, 6, &mut Vec::new()).is_err());
    }
}
