//! Reading of PE32 executables without relocations: sections, imports, exports, TLS.
//!
//! All exposed addresses are absolute (image base already added).

use sha1::{Digest, Sha1};
use std::fmt;
use std::path::Path;

/// SHA-1 of the only supported executable: `AssassinsCreed_Dx9.exe`, Steam and GOG builds.
pub const AC1_DX9_STEAM_SHA1: &str = "a9ea476e70f076709cca59a802e26ec894432120";

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub va: u32,
    pub vsize: u32,
    pub raw: u32,
    pub rsize: u32,
}

impl Section {
    /// End of the part present in the file.
    pub fn file_end(&self) -> u32 {
        self.va + self.rsize
    }

    pub fn end(&self) -> u32 {
        self.va + self.vsize.max(self.rsize)
    }

    pub fn contains(&self, addr: u32) -> bool {
        self.va <= addr && addr < self.end()
    }
}

#[derive(Debug, Clone)]
pub struct Import {
    pub dll: String,
    /// Name of the function, or `#N` for ordinal-only imports.
    pub name: String,
    pub ordinal: Option<u16>,
    /// Address of the IAT cell.
    pub iat: u32,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub name: String,
    pub addr: u32,
}

pub struct Image {
    pub data: Vec<u8>,
    pub base: u32,
    pub entry: u32,
    pub timestamp: u32,
    pub sections: Vec<Section>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    pub tls_callbacks: Vec<u32>,
    /// (start and end of the template, address of the index, trailing zero bytes)
    tls: Option<(u32, u32, u32, u32)>,
}

fn rd16(d: &[u8], o: usize) -> Result<u16> {
    match d.get(o..o + 2) {
        Some(b) => Ok(u16::from_le_bytes([b[0], b[1]])),
        None => err(format!("read past the end of the file at {o:#x}")),
    }
}

fn rd32(d: &[u8], o: usize) -> Result<u32> {
    match d.get(o..o + 4) {
        Some(b) => Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        None => err(format!("read past the end of the file at {o:#x}")),
    }
}

impl Image {
    pub fn load(path: impl AsRef<Path>) -> Result<Image> {
        let path = path.as_ref();
        let data = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        Image::parse(data)
    }

    pub fn parse(data: Vec<u8>) -> Result<Image> {
        let d = &data[..];
        let pe = rd32(d, 0x3c)? as usize;
        if d.get(pe..pe + 4) != Some(b"PE\0\0") {
            return err("not a PE");
        }
        let nsec = rd16(d, pe + 6)? as usize;
        let timestamp = rd32(d, pe + 8)?;
        let opt_size = rd16(d, pe + 20)? as usize;
        let opt = pe + 24;
        if rd16(d, opt)? != 0x10b {
            return err("only PE32 is supported");
        }
        let entry_rva = rd32(d, opt + 16)?;
        let base = rd32(d, opt + 28)?;
        let dir = |i: usize| -> Result<(u32, u32)> {
            Ok((rd32(d, opt + 96 + 8 * i)?, rd32(d, opt + 100 + 8 * i)?))
        };
        if dir(5)? != (0, 0) {
            return err("the image has relocations: unexpected case");
        }

        let mut sections = Vec::with_capacity(nsec);
        for i in 0..nsec {
            let s = opt + opt_size + 40 * i;
            let raw_name = d.get(s..s + 8).ok_or_else(|| Error("section table truncated".into()))?;
            let name = String::from_utf8_lossy(raw_name).trim_end_matches('\0').to_string();
            sections.push(Section {
                name,
                vsize: rd32(d, s + 8)?,
                va: base + rd32(d, s + 12)?,
                rsize: rd32(d, s + 16)?,
                raw: rd32(d, s + 20)?,
            });
        }

        let mut img = Image {
            data,
            base,
            entry: base + entry_rva,
            timestamp,
            sections,
            imports: Vec::new(),
            exports: Vec::new(),
            tls_callbacks: Vec::new(),
            tls: None,
        };
        let (imp_rva, _) = (rd32(&img.data, opt + 96 + 8)?, 0);
        let (exp_rva, _) = (rd32(&img.data, opt + 96)?, 0);
        let (tls_rva, _) = (rd32(&img.data, opt + 96 + 72)?, 0);
        if imp_rva != 0 {
            img.imports = img.parse_imports(base + imp_rva)?;
        }
        if exp_rva != 0 {
            img.exports = img.parse_exports(base + exp_rva)?;
        }
        if tls_rva != 0 {
            let d = base + tls_rva;
            if let (Some(start), Some(end), Some(index), Some(zero)) = (img.u32(d), img.u32(d + 4), img.u32(d + 8), img.u32(d + 16)) {
                img.tls = Some((start, end, index, zero));
            }
            let mut cb = img.u32(base + tls_rva + 12).unwrap_or(0);
            while cb != 0 {
                match img.u32(cb) {
                    Some(f) if f != 0 => img.tls_callbacks.push(f),
                    _ => break,
                }
                cb += 4;
            }
        }
        Ok(img)
    }

    fn parse_imports(&self, mut desc: u32) -> Result<Vec<Import>> {
        let need = |a: u32| self.u32(a).ok_or_else(|| Error(format!("import table: {a:#x} not readable")));
        let mut out = Vec::new();
        loop {
            let oft = need(desc)?;
            let name = need(desc + 12)?;
            let ft = need(desc + 16)?;
            if name == 0 {
                break;
            }
            let dll = self.cstr(self.base + name).unwrap_or_default().to_ascii_lowercase();
            let thunks = self.base + if oft != 0 { oft } else { ft };
            let mut i = 0;
            loop {
                let e = need(thunks + 4 * i)?;
                if e == 0 {
                    break;
                }
                let (fname, ordinal) = if e & 0x8000_0000 != 0 {
                    (format!("#{}", e & 0xffff), Some((e & 0xffff) as u16))
                } else {
                    (self.cstr(self.base + e + 2).unwrap_or_default(), None)
                };
                out.push(Import { dll: dll.clone(), name: fname, ordinal, iat: self.base + ft + 4 * i });
                i += 1;
            }
            desc += 20;
        }
        Ok(out)
    }

    fn parse_exports(&self, dir: u32) -> Result<Vec<Export>> {
        let need = |a: u32| self.u32(a).ok_or_else(|| Error(format!("export table: {a:#x} not readable")));
        let nfunc = need(dir + 20)?;
        let nnames = need(dir + 24)?;
        let funcs = self.base + need(dir + 28)?;
        let names = self.base + need(dir + 32)?;
        let ords = self.base + need(dir + 36)?;
        let mut out: Vec<Export> = Vec::new();
        let mut named = vec![false; nfunc as usize];
        for i in 0..nnames {
            let idx = self.u16(ords + 2 * i).unwrap_or(0) as u32;
            let rva = need(funcs + 4 * idx)?;
            if rva != 0 && idx < nfunc {
                named[idx as usize] = true;
                let name = self.cstr(self.base + need(names + 4 * i)?).unwrap_or_default();
                out.push(Export { name, addr: self.base + rva });
            }
        }
        for i in 0..nfunc {
            let rva = need(funcs + 4 * i)?;
            if rva != 0 && !named[i as usize] {
                out.push(Export { name: format!("#{i}"), addr: self.base + rva });
            }
        }
        out.sort_by_key(|e| e.addr);
        Ok(out)
    }

    /// The TLS directory: (start and end of the template, address of the index, trailing zero bytes).
    pub fn tls_directory(&self) -> Option<(u32, u32, u32, u32)> {
        self.tls
    }

    pub fn sha1(&self) -> String {
        Sha1::digest(&self.data).iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    pub fn section_of(&self, addr: u32) -> Option<&Section> {
        self.sections.iter().find(|s| s.contains(addr))
    }

    /// File offset of the address, if it falls in the part of the section present on disk.
    pub fn off(&self, addr: u32) -> Option<usize> {
        self.sections
            .iter()
            .find(|s| s.va <= addr && addr < s.file_end())
            .map(|s| (addr - s.va + s.raw) as usize)
    }

    pub fn bytes(&self, addr: u32, len: usize) -> Option<&[u8]> {
        let o = self.off(addr)?;
        self.data.get(o..o + len)
    }

    /// The on-disk bytes of a section.
    pub fn section_bytes(&self, s: &Section) -> &[u8] {
        let end = ((s.raw + s.rsize) as usize).min(self.data.len());
        &self.data[s.raw as usize..end]
    }

    pub fn u16(&self, addr: u32) -> Option<u16> {
        self.bytes(addr, 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&self, addr: u32) -> Option<u32> {
        self.bytes(addr, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn cstr(&self, addr: u32) -> Option<String> {
        let o = self.off(addr)?;
        let end = self.data[o..].iter().position(|&b| b == 0)?;
        Some(String::from_utf8_lossy(&self.data[o..o + end]).into_owned())
    }

    /// Sections that may contain code pointers: all except code, resources and imports.
    pub fn data_sections(&self) -> impl Iterator<Item = &Section> {
        self.sections.iter().filter(|s| !matches!(s.name.as_str(), ".text" | ".rsrc" | ".idata"))
    }

    /// The aligned 32-bit words of a section, with their address.
    pub fn words<'a>(&'a self, s: &'a Section) -> impl Iterator<Item = (u32, u32)> + 'a {
        self.section_bytes(s)
            .chunks_exact(4)
            .enumerate()
            .map(move |(i, b)| (s.va + 4 * i as u32, u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
    }
}
