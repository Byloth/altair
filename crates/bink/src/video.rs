//! Bink 1 video, revisions `f` to `i`.
//!
//! Each plane (Y, then Cr, then Cb from revision `h` on) is divided into 8×8 blocks, read row
//! by row. At the start of each row of blocks the stream carries, for nine "bundles" of values,
//! as many as that row needs: block types, 16×16 block types, colours, two-colour
//! patterns, X and Y motion offsets, intra and inter DC components, run lengths. Then the
//! blocks consume the values from the bundles, and read from the stream only what is their own
//! (DCT coefficients, residues).

use crate::bits::Bits;
use crate::tables::{INTER_QUANT, INTRA_QUANT, PATTERNS, SCAN, TREE_BITS, TREE_LENS};
use crate::{Error, Result};

// Block types.
const SKIP: i16 = 0;
const SCALED: i16 = 1;
const MOTION: i16 = 2;
const RUN: i16 = 3;
const RESIDUE: i16 = 4;
const INTRA: i16 = 5;
const FILL: i16 = 6;
const INTER: i16 = 7;
const PATTERN: i16 = 8;
const RAW: i16 = 9;

// The bundles, in the order in which they appear in the stream.
const B_TYPES: usize = 0;
const B_SUB_TYPES: usize = 1;
const B_COLORS: usize = 2;
const B_PATTERN: usize = 3;
const B_X_OFF: usize = 4;
const B_Y_OFF: usize = 5;
const B_INTRA_DC: usize = 6;
const B_INTER_DC: usize = 7;
const B_RUN: usize = 8;

/// One of the sixteen fixed Huffman codes: for each prefix of `bits` bits, (leaf, length).
struct Vlc {
    bits: u32,
    table: Vec<(u8, u8)>,
}

fn build_vlcs() -> Vec<Vlc> {
    (0..16)
        .map(|t| {
            let bits = TREE_LENS[t][15] as u32;
            let mut table = vec![(0u8, 0u8); 1 << bits];
            for leaf in 0..16 {
                let (code, len) = (TREE_BITS[t][leaf] as usize, TREE_LENS[t][leaf] as u32);
                // The code occupies the low bits: all the prefixes that end with it.
                for high in 0..1usize << (bits - len) {
                    table[code | high << len] = (leaf as u8, len as u8);
                }
            }
            Vlc { bits, table }
        })
        .collect()
}

/// A tree as the stream describes it: which of the sixteen codes, and which symbol
/// each leaf corresponds to.
#[derive(Clone, Copy)]
struct Tree {
    vlc: usize,
    syms: [u8; 16],
}

impl Default for Tree {
    fn default() -> Self {
        Tree { vlc: 0, syms: std::array::from_fn(|i| i as u8) }
    }
}

/// Merges two consecutive lists of `size` elements, choosing at each step which one to take from.
fn merge(gb: &mut Bits, dst: &mut [u8], src: &[u8], size: usize) {
    let (mut a, mut b, mut o) = (0, size, 0);
    while a < size && b < 2 * size {
        if !gb.bit() {
            dst[o] = src[a];
            a += 1;
        } else {
            dst[o] = src[b];
            b += 1;
        }
        o += 1;
    }
    while a < size {
        dst[o] = src[a];
        a += 1;
        o += 1;
    }
    while b < 2 * size {
        dst[o] = src[b];
        b += 1;
        o += 1;
    }
}

fn read_tree(gb: &mut Bits) -> Result<Tree> {
    if gb.left() < 4 {
        return Err(Error::Invalid("truncated tree"));
    }
    let mut tree = Tree { vlc: gb.get(4) as usize, ..Default::default() };
    if tree.vlc == 0 {
        return Ok(tree);
    }
    if gb.bit() {
        // The first symbols are listed; the others follow in ascending order.
        let mut len = gb.get(3) as usize;
        let mut seen = [false; 16];
        for i in 0..=len {
            tree.syms[i] = gb.get(4) as u8;
            seen[tree.syms[i] as usize] = true;
        }
        for i in 0..16 {
            if len >= 15 {
                break;
            }
            if !seen[i] {
                len += 1;
                tree.syms[len] = i as u8;
            }
        }
    } else {
        // A permutation built by merging progressively longer lists.
        let len = gb.get(2);
        let mut a: [u8; 16] = std::array::from_fn(|i| i as u8);
        let mut b = [0u8; 16];
        for i in 0..=len {
            let size = 1usize << i;
            for t in (0..16).step_by(size << 1) {
                merge(gb, &mut b[t..t + 2 * size], &a[t..t + 2 * size], size);
            }
            std::mem::swap(&mut a, &mut b);
        }
        tree.syms = a;
    }
    Ok(tree)
}

/// A bundle: the values of one kind of data already decoded and not yet consumed.
#[derive(Default)]
struct Bundle {
    /// Bits that encode how many values arrive at each row.
    len: u32,
    tree: Tree,
    data: Vec<i16>,
    /// Next value to consume.
    pos: usize,
    /// The stream has said there are no more of them for this plane.
    done: bool,
    /// Maximum capacity: beyond it, the stream is broken.
    cap: usize,
}

impl Bundle {
    /// How many new values to read, or `None` if it is not the time (there are still some, or they are finished).
    fn count(&mut self, gb: &mut Bits) -> Result<Option<usize>> {
        if self.done || self.data.len() > self.pos {
            return Ok(None);
        }
        let n = gb.get(self.len) as usize;
        if n == 0 {
            self.done = true;
            return Ok(None);
        }
        if self.data.len() + n > self.cap {
            return Err(Error::Invalid("too many values in a bundle"));
        }
        Ok(Some(n))
    }

    fn next(&mut self) -> Result<i16> {
        let v = *self.data.get(self.pos).ok_or(Error::Invalid("bundle exhausted"))?;
        self.pos += 1;
        Ok(v)
    }
}

/// A plane of the picture, with rows whose length is a multiple of 8.
#[derive(Clone)]
struct Plane {
    stride: usize,
    data: Vec<u8>,
}

/// A decoded frame, in YUV 4:2:0.
pub struct Frame<'a> {
    pub width: usize,
    pub height: usize,
    planes: &'a [Plane; 3],
}

impl Frame<'_> {
    /// Width and height of plane `i` (0 = Y, 1 = Cb, 2 = Cr).
    pub fn plane_size(&self, i: usize) -> (usize, usize) {
        if i == 0 { (self.width, self.height) } else { ((self.width + 1) / 2, (self.height + 1) / 2) }
    }

    /// A row of plane `i`.
    pub fn row(&self, i: usize, y: usize) -> &[u8] {
        let p = &self.planes[i];
        &p.data[y * p.stride..y * p.stride + self.plane_size(i).0]
    }

    /// Appends the three planes to `out` one after another (Y, Cb, Cr), without padding: the `yuv420p` format.
    pub fn write_yuv420(&self, out: &mut Vec<u8>) {
        for i in 0..3 {
            for y in 0..self.plane_size(i).1 {
                out.extend_from_slice(self.row(i, y));
            }
        }
    }
}

pub struct VideoDecoder {
    width: usize,
    height: usize,
    revision: u8,
    vlcs: Vec<Vlc>,
    bundles: [Bundle; 9],
    /// Trees of the colours' high nibble, chosen by the previous high nibble.
    col_high: [Tree; 16],
    col_last: usize,
    current: [Plane; 3],
    previous: [Plane; 3],
    has_previous: bool,
}

/// Integer part of the base-2 logarithm.
fn log2(v: usize) -> u32 {
    usize::BITS - 1 - v.leading_zeros()
}

impl VideoDecoder {
    pub fn new(width: u32, height: u32, revision: u8) -> VideoDecoder {
        let (width, height) = (width as usize, height as usize);
        let plane = |chroma: bool| {
            let (bw, bh) = if chroma { ((width + 15) >> 4, (height + 15) >> 4) } else { ((width + 7) >> 3, (height + 7) >> 3) };
            Plane { stride: bw * 8, data: vec![0; bw * 8 * bh * 8] }
        };
        let planes = [plane(false), plane(true), plane(true)];
        let blocks = ((width + 7) >> 3) * ((height + 7) >> 3);
        VideoDecoder {
            width,
            height,
            revision,
            vlcs: build_vlcs(),
            bundles: std::array::from_fn(|_| Bundle { cap: blocks * 64, ..Default::default() }),
            col_high: [Tree::default(); 16],
            col_last: 0,
            previous: planes.clone(),
            current: planes,
            has_previous: false,
        }
    }

    /// The last decoded frame.
    pub fn current(&self) -> Frame<'_> {
        Frame { width: self.width, height: self.height, planes: &self.current }
    }

    /// Decodes the video packet of a frame.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Frame<'_>> {
        std::mem::swap(&mut self.current, &mut self.previous);
        let mut gb = Bits::new(packet);
        if self.revision >= b'i' {
            gb.skip(32);
        }
        let total = packet.len() * 8;
        // From revision `h` on the stream carries Y, Cr, Cb; our planes are Y, Cb, Cr.
        let order = if self.revision >= b'h' { [0, 2, 1] } else { [0, 1, 2] };
        let mut decoded = 0;
        for (n, &plane) in order.iter().enumerate() {
            self.decode_plane(&mut gb, plane, n != 0)?;
            decoded += 1;
            if gb.position() >= total {
                break;
            }
        }
        // A frame may stop early (luma only): the missing planes stay as they were.
        for &plane in &order[decoded..] {
            let (cur, prev) = (&mut self.current[plane], &self.previous[plane]);
            cur.data.copy_from_slice(&prev.data);
        }
        self.has_previous = true;
        Ok(Frame { width: self.width, height: self.height, planes: &self.current })
    }

    #[inline]
    fn huff(&self, gb: &mut Bits, tree: &Tree) -> u8 {
        let vlc = &self.vlcs[tree.vlc];
        let (leaf, len) = vlc.table[gb.peek(vlc.bits) as usize];
        gb.skip(len as usize);
        tree.syms[leaf as usize]
    }

    fn read_bundle_header(&mut self, gb: &mut Bits, n: usize) -> Result<()> {
        if n == B_COLORS {
            for i in 0..16 {
                self.col_high[i] = read_tree(gb)?;
            }
            self.col_last = 0;
        }
        if n != B_INTRA_DC && n != B_INTER_DC {
            self.bundles[n].tree = read_tree(gb)?;
        }
        let b = &mut self.bundles[n];
        b.data.clear();
        b.pos = 0;
        b.done = false;
        Ok(())
    }

    /// Block types: values below 12, or repetitions of the last one.
    fn read_block_types(&mut self, gb: &mut Bits, n: usize) -> Result<()> {
        const RLE: [usize; 4] = [4, 8, 12, 32];
        let Some(count) = self.bundles[n].count(gb)? else { return Ok(()) };
        let tree = self.bundles[n].tree;
        if gb.bit() {
            let v = gb.get(4) as i16;
            let b = &mut self.bundles[n];
            b.data.extend(std::iter::repeat(v).take(count));
            return Ok(());
        }
        let end = self.bundles[n].data.len() + count;
        let mut last = 0i16;
        while self.bundles[n].data.len() < end {
            let v = self.huff(gb, &tree) as i16;
            let b = &mut self.bundles[n];
            if v < 12 {
                last = v;
                b.data.push(v);
            } else {
                let run = RLE[(v - 12) as usize];
                if end - b.data.len() < run {
                    return Err(Error::Invalid("block type repetition past the row"));
                }
                b.data.extend(std::iter::repeat(last).take(run));
            }
        }
        Ok(())
    }

    /// Colours: high nibble with a tree chosen by the previous high nibble, then low nibble.
    fn read_colors(&mut self, gb: &mut Bits) -> Result<()> {
        let Some(count) = self.bundles[B_COLORS].count(gb)? else { return Ok(()) };
        let tree = self.bundles[B_COLORS].tree;
        let repeat = gb.bit();
        let mut i = 0;
        while i < count {
            if gb.left() < 2 {
                return Err(Error::Invalid("truncated colours"));
            }
            let high_tree = self.col_high[self.col_last];
            self.col_last = self.huff(gb, &high_tree) as usize;
            let mut v = (self.col_last << 4) as i32 | self.huff(gb, &tree) as i32;
            if self.revision < b'i' {
                // Up to revision `h` the value is sign and magnitude, around 0x80.
                let sign = (v as i8 >> 7) as i32;
                v = ((v & 0x7f) ^ sign) - sign + 0x80;
            }
            let b = &mut self.bundles[B_COLORS];
            if repeat {
                b.data.extend(std::iter::repeat(v as i16).take(count));
                break;
            }
            b.data.push(v as i16);
            i += 1;
        }
        Ok(())
    }

    fn read_patterns(&mut self, gb: &mut Bits) -> Result<()> {
        let Some(count) = self.bundles[B_PATTERN].count(gb)? else { return Ok(()) };
        let tree = self.bundles[B_PATTERN].tree;
        for _ in 0..count {
            if gb.left() < 2 {
                return Err(Error::Invalid("truncated patterns"));
            }
            let low = self.huff(gb, &tree) as i16;
            let high = self.huff(gb, &tree) as i16;
            self.bundles[B_PATTERN].data.push(low | high << 4);
        }
        Ok(())
    }

    /// Motion offsets: magnitude with a tree, then the sign if the magnitude is not zero.
    fn read_motion_values(&mut self, gb: &mut Bits, n: usize) -> Result<()> {
        let Some(count) = self.bundles[n].count(gb)? else { return Ok(()) };
        let tree = self.bundles[n].tree;
        let repeat = gb.bit();
        let mut i = 0;
        while i < count {
            let mut v = if repeat { gb.get(4) as i16 } else { self.huff(gb, &tree) as i16 };
            if v != 0 && gb.bit() {
                v = -v;
            }
            let b = &mut self.bundles[n];
            if repeat {
                b.data.extend(std::iter::repeat(v).take(count));
                break;
            }
            b.data.push(v);
            i += 1;
        }
        Ok(())
    }

    /// DC components: the first written out in full, the others as differences in groups of eight.
    fn read_dcs(&mut self, gb: &mut Bits, n: usize, signed: bool) -> Result<()> {
        const START_BITS: u32 = 11;
        let Some(count) = self.bundles[n].count(gb)? else { return Ok(()) };
        let mut v = gb.get(START_BITS - signed as u32) as i32;
        if v != 0 && signed && gb.bit() {
            v = -v;
        }
        let b = &mut self.bundles[n];
        b.data.push(v as i16);
        let mut left = count - 1;
        while left > 0 {
            let group = left.min(8);
            let size = gb.get(4);
            for _ in 0..group {
                if size != 0 {
                    let mut d = gb.get(size) as i32;
                    if d != 0 && gb.bit() {
                        d = -d;
                    }
                    v += d;
                    if !(-32768..=32767).contains(&v) {
                        return Err(Error::Invalid("DC component out of range"));
                    }
                }
                b.data.push(v as i16);
            }
            left -= group;
        }
        Ok(())
    }

    fn read_runs(&mut self, gb: &mut Bits) -> Result<()> {
        let Some(count) = self.bundles[B_RUN].count(gb)? else { return Ok(()) };
        let tree = self.bundles[B_RUN].tree;
        if gb.bit() {
            let v = gb.get(4) as i16;
            self.bundles[B_RUN].data.extend(std::iter::repeat(v).take(count));
        } else {
            for _ in 0..count {
                let v = self.huff(gb, &tree) as i16;
                self.bundles[B_RUN].data.push(v);
            }
        }
        Ok(())
    }

    fn decode_plane(&mut self, gb: &mut Bits, plane: usize, chroma: bool) -> Result<()> {
        let (bw, bh) = if chroma { ((self.width + 15) >> 4, (self.height + 15) >> 4) } else { ((self.width + 7) >> 3, (self.height + 7) >> 3) };
        let width = ((self.width >> chroma as usize).max(8) + 7) & !7;
        let stride = self.current[plane].stride;

        // How many bits are needed to say "how many values in this row", for each bundle.
        let common = log2((width >> 3) + 511) + 1;
        let lens = [common, log2((width >> 4) + 511) + 1, log2(bw * 64 + 511) + 1, log2((bw << 3) + 511) + 1, common, common, common, common, log2(bw * 48 + 511) + 1];
        for (n, len) in lens.iter().enumerate() {
            self.bundles[n].len = *len;
            self.read_bundle_header(gb, n)?;
        }

        // The farthest reference allowed: the corner of the last block.
        let ref_end = ((bw - 1) + stride * (bh - 1)) * 8;
        let mut dct = [0i32; 64];
        let mut small = [0u8; 64];
        for by in 0..bh {
            self.read_block_types(gb, B_TYPES)?;
            self.read_block_types(gb, B_SUB_TYPES)?;
            self.read_colors(gb)?;
            self.read_patterns(gb)?;
            self.read_motion_values(gb, B_X_OFF)?;
            self.read_motion_values(gb, B_Y_OFF)?;
            self.read_dcs(gb, B_INTRA_DC, false)?;
            self.read_dcs(gb, B_INTER_DC, true)?;
            self.read_runs(gb)?;

            let mut bx = 0;
            while bx < bw {
                let at = by * 8 * stride + bx * 8;
                let kind = self.bundles[B_TYPES].next()?;
                // A 16×16 block takes four slots: on odd rows and columns it is already done.
                if (by & 1 != 0 || bx & 1 != 0) && kind == SCALED {
                    bx += 2;
                    continue;
                }
                match kind {
                    SKIP => self.copy_block(plane, at, at as isize)?,
                    SCALED => {
                        let sub = self.bundles[B_SUB_TYPES].next()?;
                        match sub {
                            RUN => self.run_block(gb, &mut small, 8, 0)?,
                            INTRA => {
                                self.read_dct(gb, &mut dct, B_INTRA_DC, &INTRA_QUANT)?;
                                idct_put(&mut small, 8, &mut dct);
                            }
                            FILL => {
                                let v = self.bundles[B_COLORS].next()? as u8;
                                let dst = &mut self.current[plane].data;
                                for y in 0..16 {
                                    dst[at + y * stride..at + y * stride + 16].fill(v);
                                }
                            }
                            PATTERN => self.pattern_block(&mut small, 8, 0)?,
                            RAW => {
                                for p in small.iter_mut() {
                                    *p = self.bundles[B_COLORS].next()? as u8;
                                }
                            }
                            _ => return Err(Error::Invalid("unknown 16×16 block type")),
                        }
                        if sub != FILL {
                            // Each pixel of the small block becomes a 2×2 square.
                            let dst = &mut self.current[plane].data;
                            for y in 0..8 {
                                for x in 0..8 {
                                    let v = small[y * 8 + x];
                                    let o = at + 2 * y * stride + 2 * x;
                                    dst[o] = v;
                                    dst[o + 1] = v;
                                    dst[o + stride] = v;
                                    dst[o + stride + 1] = v;
                                }
                            }
                        }
                        bx += 1;
                    }
                    MOTION => self.motion_block(plane, at, ref_end)?,
                    RUN => {
                        let mut dst = std::mem::take(&mut self.current[plane].data);
                        let r = self.run_block(gb, &mut dst, stride, at);
                        self.current[plane].data = dst;
                        r?;
                    }
                    RESIDUE => {
                        self.motion_block(plane, at, ref_end)?;
                        let mut block = [0i16; 64];
                        let masks = gb.get(7) as i32;
                        read_residue(gb, &mut block, masks);
                        let dst = &mut self.current[plane].data;
                        for y in 0..8 {
                            for x in 0..8 {
                                let p = &mut dst[at + y * stride + x];
                                *p = p.wrapping_add(block[y * 8 + x] as u8);
                            }
                        }
                    }
                    INTRA => {
                        self.read_dct(gb, &mut dct, B_INTRA_DC, &INTRA_QUANT)?;
                        idct_put(&mut self.current[plane].data[at..], stride, &mut dct);
                    }
                    FILL => {
                        let v = self.bundles[B_COLORS].next()? as u8;
                        let dst = &mut self.current[plane].data;
                        for y in 0..8 {
                            dst[at + y * stride..at + y * stride + 8].fill(v);
                        }
                    }
                    INTER => {
                        self.motion_block(plane, at, ref_end)?;
                        self.read_dct(gb, &mut dct, B_INTER_DC, &INTER_QUANT)?;
                        idct(&mut dct);
                        let dst = &mut self.current[plane].data;
                        for y in 0..8 {
                            for x in 0..8 {
                                let p = &mut dst[at + y * stride + x];
                                *p = p.wrapping_add(dct[y * 8 + x] as u8);
                            }
                        }
                    }
                    PATTERN => {
                        let mut dst = std::mem::take(&mut self.current[plane].data);
                        let r = self.pattern_block(&mut dst, stride, at);
                        self.current[plane].data = dst;
                        r?;
                    }
                    RAW => {
                        for y in 0..8 {
                            for x in 0..8 {
                                let v = self.bundles[B_COLORS].next()? as u8;
                                self.current[plane].data[at + y * stride + x] = v;
                            }
                        }
                    }
                    _ => return Err(Error::Invalid("unknown block type")),
                }
                bx += 1;
            }
        }
        // The next plane starts at a multiple of 32 bits.
        gb.align32();
        Ok(())
    }

    /// Copies an 8×8 block from the previous frame (or, on the first one, from the one being built).
    fn copy_block(&mut self, plane: usize, at: usize, from: isize) -> Result<()> {
        let stride = self.current[plane].stride;
        let from = from as usize;
        if self.has_previous {
            let (dst, src) = (&mut self.current[plane].data, &self.previous[plane].data);
            for y in 0..8 {
                dst[at + y * stride..at + y * stride + 8].copy_from_slice(&src[from + y * stride..from + y * stride + 8]);
            }
        } else if from != at {
            let dst = &mut self.current[plane].data;
            let mut tmp = [0u8; 64];
            for y in 0..8 {
                tmp[y * 8..y * 8 + 8].copy_from_slice(&dst[from + y * stride..from + y * stride + 8]);
            }
            for y in 0..8 {
                dst[at + y * stride..at + y * stride + 8].copy_from_slice(&tmp[y * 8..y * 8 + 8]);
            }
        }
        Ok(())
    }

    /// Copies a block displaced by (x, y) taken from the bundles.
    fn motion_block(&mut self, plane: usize, at: usize, ref_end: usize) -> Result<()> {
        let x = self.bundles[B_X_OFF].next()? as isize;
        let y = self.bundles[B_Y_OFF].next()? as isize;
        let from = at as isize + x + y * self.current[plane].stride as isize;
        if from < 0 || from as usize > ref_end {
            return Err(Error::Invalid("motion offset outside the picture"));
        }
        self.copy_block(plane, at, from)
    }

    /// Run block: the pixels are visited in one of the sixteen orders, in stretches of a single
    /// colour or of listed colours.
    fn run_block(&mut self, gb: &mut Bits, dst: &mut [u8], stride: usize, at: usize) -> Result<()> {
        if gb.left() < 4 {
            return Err(Error::Invalid("truncated run block"));
        }
        let scan = &PATTERNS[gb.get(4) as usize];
        let mut put = |i: usize, v: u8| {
            let s = scan[i] as usize;
            dst[at + (s >> 3) * stride + (s & 7)] = v;
        };
        let mut i = 0usize;
        loop {
            let run = self.bundles[B_RUN].next()? as usize + 1;
            if i + run > 64 {
                return Err(Error::Invalid("run past the block"));
            }
            if gb.bit() {
                let v = self.bundles[B_COLORS].next()? as u8;
                for k in 0..run {
                    put(i + k, v);
                }
            } else {
                for k in 0..run {
                    put(i + k, self.bundles[B_COLORS].next()? as u8);
                }
            }
            i += run;
            if i >= 63 {
                break;
            }
        }
        if i == 63 {
            put(63, self.bundles[B_COLORS].next()? as u8);
        }
        Ok(())
    }

    /// Two-colour block: eight rows of eight bits that choose between the two.
    fn pattern_block(&mut self, dst: &mut [u8], stride: usize, at: usize) -> Result<()> {
        let col = [self.bundles[B_COLORS].next()? as u8, self.bundles[B_COLORS].next()? as u8];
        for y in 0..8 {
            let mut v = self.bundles[B_PATTERN].next()?;
            for x in 0..8 {
                dst[at + y * stride + x] = col[(v & 1) as usize];
                v >>= 1;
            }
        }
        Ok(())
    }

    /// Reads a DCT block: DC component from the bundle, coefficients from the stream, then the
    /// inverse quantization.
    fn read_dct(&mut self, gb: &mut Bits, block: &mut [i32; 64], dc: usize, quant: &[[i32; 64]; 16]) -> Result<()> {
        *block = [0; 64];
        block[0] = self.bundles[dc].next()? as i32;
        let mut coef_idx = [0usize; 64];
        let (count, q) = read_dct_coeffs(gb, block, &mut coef_idx)?;
        let quant = &quant[q];
        let scale = |v: i32, q: i32| (v as u32).wrapping_mul(q as u32) as i32 >> 11;
        block[0] = scale(block[0], quant[0]);
        for &idx in &coef_idx[..count] {
            let at = SCAN[idx] as usize;
            block[at] = scale(block[at], quant[idx]);
        }
        Ok(())
    }
}

/// Coefficients of a DCT block, by bit planes from the highest. Two lists work together:
/// groups of coefficients still to be opened and single coefficients waiting for their bit.
/// Returns how many coefficients were read and the quantization index.
fn read_dct_coeffs(gb: &mut Bits, block: &mut [i32; 64], coef_idx: &mut [usize; 64]) -> Result<(usize, usize)> {
    if gb.left() < 4 {
        return Err(Error::Invalid("truncated coefficients"));
    }
    let mut coef_list = [0usize; 128];
    let mut mode_list = [0u8; 128];
    let (mut start, mut end) = (64usize, 64usize);
    for (c, m) in [(4, 0), (24, 0), (44, 0), (1, 3), (2, 3), (3, 3)] {
        coef_list[end] = c;
        mode_list[end] = m;
        end += 1;
    }
    let mut count = 0usize;
    let read_value = |gb: &mut Bits, bits: i32| -> i32 {
        if bits == 0 {
            1 - 2 * gb.get(1) as i32
        } else {
            let t = (gb.get(bits as u32) | 1 << bits) as i32;
            if gb.bit() { -t } else { t }
        }
    };
    let mut bits = gb.get(4) as i32 - 1;
    while bits >= 0 {
        let mut pos = start;
        while pos < end {
            if (mode_list[pos] == 0 && coef_list[pos] == 0) || !gb.bit() {
                pos += 1;
                continue;
            }
            let mut ccoef = coef_list[pos];
            let mode = mode_list[pos];
            match mode {
                0 | 2 => {
                    if mode == 0 {
                        coef_list[pos] = ccoef + 4;
                        mode_list[pos] = 1;
                    } else {
                        coef_list[pos] = 0;
                        mode_list[pos] = 0;
                        pos += 1;
                    }
                    for _ in 0..4 {
                        if gb.bit() {
                            start -= 1;
                            coef_list[start] = ccoef;
                            mode_list[start] = 3;
                        } else {
                            block[SCAN[ccoef] as usize] = read_value(gb, bits);
                            coef_idx[count] = ccoef;
                            count += 1;
                        }
                        ccoef += 1;
                    }
                }
                1 => {
                    mode_list[pos] = 2;
                    for _ in 0..3 {
                        ccoef += 4;
                        coef_list[end] = ccoef;
                        mode_list[end] = 2;
                        end += 1;
                    }
                }
                _ => {
                    block[SCAN[ccoef] as usize] = read_value(gb, bits);
                    coef_idx[count] = ccoef;
                    count += 1;
                    coef_list[pos] = 0;
                    mode_list[pos] = 0;
                    pos += 1;
                }
            }
        }
        bits -= 1;
    }
    Ok((count, gb.get(4) as usize))
}

/// Residue of a block with motion: same lists as the DCT coefficients, but each bit
/// plane adds a mask to the values that are already non-zero. `masks` limits how many arrive.
fn read_residue(gb: &mut Bits, block: &mut [i16; 64], mut masks: i32) {
    let mut coef_list = [0usize; 128];
    let mut mode_list = [0u8; 128];
    let (mut start, mut end) = (64usize, 64usize);
    for (c, m) in [(4, 0), (24, 0), (44, 0), (0, 2)] {
        coef_list[end] = c;
        mode_list[end] = m;
        end += 1;
    }
    let mut nonzero = [0usize; 64];
    let mut nonzero_count = 0usize;
    let mut mask = 1i16 << gb.get(3);
    while mask != 0 {
        for &at in &nonzero[..nonzero_count] {
            if !gb.bit() {
                continue;
            }
            if block[at] < 0 {
                block[at] -= mask;
            } else {
                block[at] += mask;
            }
            masks -= 1;
            if masks < 0 {
                return;
            }
        }
        let mut pos = start;
        while pos < end {
            if (mode_list[pos] == 0 && coef_list[pos] == 0) || !gb.bit() {
                pos += 1;
                continue;
            }
            let mut ccoef = coef_list[pos];
            let mode = mode_list[pos];
            match mode {
                0 | 2 => {
                    if mode == 0 {
                        coef_list[pos] = ccoef + 4;
                        mode_list[pos] = 1;
                    } else {
                        coef_list[pos] = 0;
                        mode_list[pos] = 0;
                        pos += 1;
                    }
                    for _ in 0..4 {
                        if gb.bit() {
                            start -= 1;
                            coef_list[start] = ccoef;
                            mode_list[start] = 3;
                        } else {
                            let at = SCAN[ccoef] as usize;
                            nonzero[nonzero_count] = at;
                            nonzero_count += 1;
                            block[at] = if gb.bit() { -mask } else { mask };
                            masks -= 1;
                            if masks < 0 {
                                return;
                            }
                        }
                        ccoef += 1;
                    }
                }
                1 => {
                    mode_list[pos] = 2;
                    for _ in 0..3 {
                        ccoef += 4;
                        coef_list[end] = ccoef;
                        mode_list[end] = 2;
                        end += 1;
                    }
                }
                _ => {
                    let at = SCAN[ccoef] as usize;
                    nonzero[nonzero_count] = at;
                    nonzero_count += 1;
                    block[at] = if gb.bit() { -mask } else { mask };
                    coef_list[pos] = 0;
                    mode_list[pos] = 0;
                    pos += 1;
                    masks -= 1;
                    if masks < 0 {
                        return;
                    }
                }
            }
        }
        mask >>= 1;
    }
}

// --- inverse transform: integer arithmetic, must produce the same bytes as the reference -------

const A1: i32 = 2896; // (1/√2) × 4096
const A2: i32 = 2217;
const A3: i32 = 3784;
const A4: i32 = -5352;

#[inline]
fn mul(x: i32, y: i32) -> i32 {
    (x as u32).wrapping_mul(y as u32) as i32 >> 11
}

/// An eight-point pass over `s`, read with stride `step` from `at`. Returns the eight results.
#[inline]
fn transform(s: &[i32], at: usize, step: usize) -> [i32; 8] {
    let v = |i: usize| s[at + i * step];
    let a0 = v(0).wrapping_add(v(4));
    let a1 = v(0).wrapping_sub(v(4));
    let a2 = v(2).wrapping_add(v(6));
    let a3 = mul(A1, v(2).wrapping_sub(v(6)));
    let a4 = v(5).wrapping_add(v(3));
    let a5 = v(5).wrapping_sub(v(3));
    let a6 = v(1).wrapping_add(v(7));
    let a7 = v(1).wrapping_sub(v(7));
    let b0 = a4.wrapping_add(a6);
    let b1 = mul(A3, a5.wrapping_add(a7));
    let b2 = mul(A4, a5).wrapping_sub(b0).wrapping_add(b1);
    let b3 = mul(A1, a6.wrapping_sub(a4)).wrapping_sub(b2);
    let b4 = mul(A2, a7).wrapping_add(b3).wrapping_sub(b1);
    [
        a0.wrapping_add(a2).wrapping_add(b0),
        a1.wrapping_add(a3).wrapping_sub(a2).wrapping_add(b2),
        a1.wrapping_sub(a3).wrapping_add(a2).wrapping_add(b3),
        a0.wrapping_sub(a2).wrapping_sub(b4),
        a0.wrapping_sub(a2).wrapping_add(b4),
        a1.wrapping_sub(a3).wrapping_add(a2).wrapping_sub(b3),
        a1.wrapping_add(a3).wrapping_sub(a2).wrapping_sub(b2),
        a0.wrapping_add(a2).wrapping_sub(b0),
    ]
}

/// The columns: without rounding. A column with only the DC component is constant.
fn columns(block: &[i32; 64]) -> [i32; 64] {
    let mut temp = [0i32; 64];
    for x in 0..8 {
        if (1..8).all(|y| block[x + 8 * y] == 0) {
            for y in 0..8 {
                temp[x + 8 * y] = block[x];
            }
        } else {
            for (y, v) in transform(block, x, 8).into_iter().enumerate() {
                temp[x + 8 * y] = v;
            }
        }
    }
    temp
}

#[inline]
fn round(v: i32) -> i32 {
    v.wrapping_add(0x7f) >> 8
}

/// In-place inverse transform.
fn idct(block: &mut [i32; 64]) {
    let temp = columns(block);
    for y in 0..8 {
        for (x, v) in transform(&temp, 8 * y, 1).into_iter().enumerate() {
            block[8 * y + x] = round(v);
        }
    }
}

/// Inverse transform written out as pixels (truncated to 8 bits, without saturation).
fn idct_put(dst: &mut [u8], stride: usize, block: &mut [i32; 64]) {
    let temp = columns(block);
    for y in 0..8 {
        for (x, v) in transform(&temp, 8 * y, 1).into_iter().enumerate() {
            dst[y * stride + x] = round(v) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlc_tables_are_complete_prefix_codes() {
        for (t, vlc) in build_vlcs().iter().enumerate() {
            assert!(vlc.table.iter().all(|(_, len)| *len > 0), "tree {t}: prefix without a leaf");
            let mut seen = [false; 16];
            for (leaf, _) in &vlc.table {
                seen[*leaf as usize] = true;
            }
            assert!(seen.iter().all(|s| *s), "tree {t}: unreachable leaf");
        }
    }

    #[test]
    fn idct_of_dc_only_is_flat() {
        let mut block = [0i32; 64];
        block[0] = 8 * 256;
        idct(&mut block);
        assert!(block.iter().all(|v| *v == block[0]));
        assert_eq!(block[0], 8);
    }

    #[test]
    fn tree_zero_is_identity() {
        let data = [0u8; 4];
        let tree = read_tree(&mut Bits::new(&data)).unwrap();
        assert_eq!(tree.vlc, 0);
        assert_eq!(tree.syms[5], 5);
    }
}
