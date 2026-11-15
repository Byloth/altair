//! Lettura degli archivi `.forge` di Assassin's Creed (2007), contenitore versione 25.
//!
//! Riscrittura in Rust di `tools/forge.py`, che resta il riferimento con cui si confronta
//! (`altair-forge digest` contro `forge.py digest`). Il formato è descritto in
//! `docs/analisi/forge.md`; qui in breve:
//!
//! ```text
//! Header      "scimitar\0", u32 versione (25), u64 offset di HeaderFat, …
//! HeaderFat   u32 numero di file, u32, u64, u64, u32, u32 numero di tabelle, u64 prima tabella
//! Tabella     u32 voci, u32, u64 offset delle voci, u64 tabella successiva (-1 = fine), …
//! Voce        u64 offset del record, u32 id, u32 dimensione dei dati
//! Record      "FILEDATA", char[128] nome, … 0x1B8 byte in tutto; poi i dati
//! Sezione     u64 marcatore, u16 versione, u8 codec (1 = LZO1X), u16, u16, u16 blocchi,
//!             blocchi × (u16 decompressi, u16 su disco), poi per blocco u32 checksum + dati
//! ```
//!
//! I nomi delle voci e i dati estratti sono contenuti del gioco: non vanno nel repository.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Marcatore di una sezione compressa.
pub const MARK: [u8; 8] = [0x33, 0xaa, 0xfb, 0x57, 0x99, 0xfa, 0x04, 0x10];
/// Dimensione del record che precede i dati di una voce.
pub const RECORD: u64 = 0x1b8;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// Il file non ha la struttura attesa.
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

/// Una voce dell'archivio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Offset del record `FILEDATA`.
    pub record: u64,
    pub id: u32,
    /// Dimensione dei dati su disco, dopo il record.
    pub size: u32,
}

/// Un blocco di una sezione: byte decompressi, byte su disco, offset dei dati.
#[derive(Clone, Copy, Debug)]
pub struct Chunk {
    pub unpacked: u16,
    pub stored: u16,
    pub offset: u64,
}

/// Una sezione compressa dei dati di una voce.
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
            return bad("non è un archivio forge");
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

    /// Legge `len` byte da `offset`; meno, se il file finisce prima.
    pub fn read(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut buf = Vec::with_capacity(len);
        (&mut self.file).take(len as u64).read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn read_exact(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let buf = self.read(offset, len)?;
        if buf.len() != len {
            return bad(format!("fine del file a {offset:#x}"));
        }
        Ok(buf)
    }

    /// Il nome di una voce, o `None` se il record non comincia con `FILEDATA`.
    pub fn name(&mut self, entry: &Entry) -> Result<Option<String>> {
        let r = self.read(entry.record, 8 + 128)?;
        if r.len() < 8 || &r[..8] != b"FILEDATA" {
            return Ok(None);
        }
        // Latin-1: ogni byte è il carattere con lo stesso codice.
        Ok(Some(r[8..].iter().take_while(|b| **b != 0).map(|b| *b as char).collect()))
    }

    /// Le sezioni compresse di una voce; vuoto se i dati sono grezzi.
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

    /// I dati decompressi di una sezione.
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

    /// I dati grezzi di una voce (per quelle senza sezioni).
    pub fn raw(&mut self, entry: &Entry) -> Result<Vec<u8>> {
        self.read_exact(entry.record + RECORD, entry.size as usize)
    }
}

/// Decomprime un blocco LZO1X (formato pubblico) in coda a `out`; deve produrre `expect` byte.
pub fn lzo1x_decompress(src: &[u8], expect: usize, out: &mut Vec<u8>) -> Result<()> {
    let base = out.len();
    let mut ip = 0usize;
    let get = |i: usize| -> Result<usize> { src.get(i).map(|b| *b as usize).ok_or(Error::Format("LZO: dati troncati".into())) };
    // Copia `n` byte da `dist` byte indietro; gli intervalli possono sovrapporsi.
    fn copy(out: &mut Vec<u8>, base: usize, dist: usize, n: usize) -> Result<()> {
        if dist == 0 || dist > out.len() - base {
            return bad("LZO: distanza fuori dal buffer");
        }
        let mut p = out.len() - dist;
        for _ in 0..n {
            out.push(out[p]);
            p += 1;
        }
        Ok(())
    }
    fn literals(src: &[u8], ip: &mut usize, n: usize, out: &mut Vec<u8>) -> Result<()> {
        let s = src.get(*ip..*ip + n).ok_or(Error::Format("LZO: dati troncati".into()))?;
        out.extend_from_slice(s);
        *ip += n;
        Ok(())
    }
    /// Lunghezza estesa: una serie di zeri vale 255 l'uno, poi un byte.
    fn extend(src: &[u8], ip: &mut usize, bias: usize) -> Result<usize> {
        let mut t = 0;
        loop {
            let b = *src.get(*ip).ok_or(Error::Format("LZO: dati troncati".into()))? as usize;
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
                    break; // fine del flusso
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
        return bad(format!("LZO: {} byte invece di {expect}", out.len() - base));
    }
    Ok(())
}

/// CRC-32 (IEEE, come zlib): etichetta le classi degli oggetti e serve ai confronti.
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
        // Un blocco di soli letterali: byte iniziale 17 + n, i letterali, poi il fine flusso 11 00 00.
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
