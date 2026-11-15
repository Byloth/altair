//! The Animus fonts, read from the user's game data.
//!
//! The diagnostics panel (`overlay.rs`) wants the same font as the Animus interface.
//! That font belongs to Ubisoft and cannot be in the repository: it is read
//! at startup from `DataPC.forge`, in the game folder, as the game itself does. If reading
//! fails (different archive, unexpected format) the panel falls back on its own 5×7 glyphs.
//!
//! Formats, verified on `DataPC.forge` (details in `docs/analysis/forge.md`):
//! - `MagmaMftFile`: after the string `PixmapFont`, a `u16` glyph count and a 30-byte
//!   record per glyph: code, width, height, x and y offset from the baseline
//!   (signed), advance, four `f32` (u0, v0, u1, v1 in the atlas), a `u16` set to zero;
//! - `TextureMap`: width and height after name, id and class; the data, DXT3, 76 bytes further on,
//!   with the rows from bottom to top. The glyph is entirely in the alpha channel.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The archive entry that contains the fonts and their images.
const ENTRY: &str = "Game Bootstrap Settings";
/// Objects: (font, image). Body text and titles.
const TEXT: (u32, u32) = (0x4eba_10ab, 0x4719_5956); // "AnimusText PC"
const TITLE: (u32, u32) = (0x87e5_99f1, 0x87e5_99ec); // "Animus Title"

#[derive(Clone, Copy)]
pub struct Glyph {
    pub w: f32,
    pub h: f32,
    pub dx: f32,
    /// How far the top edge is above the baseline.
    pub dy: f32,
    pub advance: f32,
    /// In the shared atlas, in texture coordinates.
    pub uv: [f32; 4],
}

pub struct Face {
    pub glyphs: HashMap<char, Glyph>,
    /// Capital height, in atlas pixels.
    pub cap: f32,
}

pub struct Fonts {
    pub text: Face,
    pub title: Face,
    /// Shared atlas, one byte of coverage per pixel.
    pub atlas: Vec<u8>,
    pub size: (u32, u32),
}

static GAME_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
static FONTS: OnceLock<Option<Fonts>> = OnceLock::new();

pub fn set_game_dir(dir: &std::path::Path) {
    let _ = GAME_DIR.set(dir.to_path_buf());
}

/// The fonts, read on first request.
pub fn get() -> Option<&'static Fonts> {
    FONTS
        .get_or_init(|| {
            let start = std::time::Instant::now();
            match GAME_DIR.get().ok_or("unknown game folder".to_string()).and_then(|d| load(d)) {
                Ok(f) => {
                    eprintln!("[overlay] Animus fonts read from DataPC.forge in {} ms ({}x{})", start.elapsed().as_millis(), f.size.0, f.size.1);
                    Some(f)
                }
                Err(e) => {
                    eprintln!("[overlay] Animus fonts not available ({e}): using the dot-matrix ones");
                    None
                }
            }
        })
        .as_ref()
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// The image of a DXT3 `TextureMap`: (width, height, alpha per pixel, from the top).
fn texture(obj: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let base = 12 + u32_at(obj, 8)? as usize + 1 + 8;
    let (w, h, len) = (u32_at(obj, base)?, u32_at(obj, base + 4)?, u32_at(obj, base + 71)? as usize);
    if w == 0 || h == 0 || w % 4 != 0 || h % 4 != 0 || w > 4096 || h > 4096 || len != (w * h) as usize {
        return None;
    }
    let data = obj.get(base + 75..base + 75 + len)?;
    let mut alpha = vec![0u8; len];
    for (i, block) in data.chunks_exact(16).enumerate() {
        let (bx, by) = (i as u32 % (w / 4) * 4, i as u32 / (w / 4) * 4);
        let bits = u64::from_le_bytes(block[..8].try_into().ok()?);
        for p in 0..16u32 {
            // The rows of the file go from bottom to top.
            let (x, y) = (bx + p % 4, h - 1 - (by + p / 4));
            alpha[(y * w + x) as usize] = ((bits >> (4 * p)) & 15) as u8 * 17;
        }
    }
    Some((w, h, alpha))
}

/// The glyphs of a `MagmaMftFile`, with coordinates still relative to their own image.
fn glyphs(obj: &[u8]) -> Option<HashMap<char, Glyph>> {
    let mark = obj.windows(10).position(|w| w == b"PixmapFont")? + 10;
    let count = u16_at(obj, mark)? as usize;
    let mut out = HashMap::new();
    for i in 0..count {
        let r = obj.get(mark + 2 + 30 * i..mark + 32 + 30 * i)?;
        let f = |k: usize| f32::from_le_bytes(r[12 + 4 * k..16 + 4 * k].try_into().unwrap());
        let s = |k: usize| u16::from_le_bytes([r[k], r[k + 1]]);
        let Some(ch) = char::from_u32(s(0) as u32) else { continue };
        out.insert(ch, Glyph { w: s(2) as f32 + 1.0, h: s(4) as f32 + 1.0, dx: s(6) as i16 as f32, dy: s(8) as i16 as f32, advance: s(10) as f32, uv: [f(0), f(1), f(2), f(3)] });
    }
    (out.len() > 60).then_some(out)
}

fn load(dir: &std::path::Path) -> Result<Fonts, String> {
    let mut forge = altair_forge::Forge::open(dir.join("DataPC.forge")).map_err(|e| e.to_string())?;
    let entries = forge.entries.clone();
    let mut found = None;
    for e in &entries {
        if forge.name(e).map_err(|e| e.to_string())?.as_deref() == Some(ENTRY) {
            found = Some(*e);
            break;
        }
    }
    let entry = found.ok_or("font entry not found")?;
    let sections = forge.sections(&entry).map_err(|e| e.to_string())?;
    if sections.len() != 2 {
        return Err("entry with an unexpected number of sections".into());
    }
    let index = forge.decompress(&sections[0]).map_err(|e| e.to_string())?;
    let data = forge.decompress(&sections[1]).map_err(|e| e.to_string())?;
    let count = u16_at(&index, 0).ok_or("empty index")? as usize;
    let mut objects = HashMap::new();
    let mut pos = 0usize;
    for i in 0..count {
        let (id, size) = (u32_at(&index, 2 + 8 * i).ok_or("short index")?, u32_at(&index, 6 + 8 * i).ok_or("short index")? as usize);
        objects.insert(id, data.get(pos..pos + size).ok_or("object past the end of the data")?);
        pos += size;
    }
    let part = |ids: (u32, u32)| -> Result<(HashMap<char, Glyph>, (u32, u32, Vec<u8>)), String> {
        let g = glyphs(objects.get(&ids.0).ok_or("font not found")?).ok_or("glyph table not recognised")?;
        let t = texture(objects.get(&ids.1).ok_or("font image not found")?).ok_or("font image not recognised")?;
        Ok((g, t))
    };
    let (mut text, (tw, th, ta)) = part(TEXT)?;
    let (mut title, (lw, lh, la)) = part(TITLE)?;
    // A single atlas: the titles on top, the text below.
    let (w, h) = (tw.max(lw), th + lh);
    let mut atlas = vec![0u8; (w * h) as usize];
    for y in 0..lh {
        atlas[(y * w) as usize..(y * w + lw) as usize].copy_from_slice(&la[(y * lw) as usize..((y + 1) * lw) as usize]);
    }
    for y in 0..th {
        atlas[((lh + y) * w) as usize..((lh + y) * w + tw) as usize].copy_from_slice(&ta[(y * tw) as usize..((y + 1) * tw) as usize]);
    }
    let remap = |g: &mut HashMap<char, Glyph>, gw: u32, gh: u32, y0: u32| {
        for v in g.values_mut() {
            let [u0, v0, u1, v1] = v.uv;
            v.uv = [u0 * gw as f32 / w as f32, (v0 * gh as f32 + y0 as f32) / h as f32, u1 * gw as f32 / w as f32, (v1 * gh as f32 + y0 as f32) / h as f32];
        }
    };
    remap(&mut title, lw, lh, 0);
    remap(&mut text, tw, th, lh);
    let cap = |g: &HashMap<char, Glyph>| g.get(&'H').map(|g| g.dy).filter(|&c| c > 4.0).ok_or("font without the H");
    Ok(Fonts { text: Face { cap: cap(&text)?, glyphs: text }, title: Face { cap: cap(&title)?, glyphs: title }, atlas, size: (w, h) })
}
