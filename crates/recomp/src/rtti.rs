//! 32-bit MSVC RTTI: type descriptors, complete object locators, vtables.
//!
//! Same method as `tools/rtti.py`; the expected counts for the Steam DX9 build are
//! 4,376 type descriptors, 4,597 vtables, 59,899 slots.

use altair_pe::Image;
use std::collections::{HashMap, HashSet};

pub struct Vtable {
    #[allow(dead_code)] // the translator will need it to name virtual functions
    pub addr: u32,
    /// Decorated name of the class (`.?AVFoo@bar@@`).
    pub class: String,
    /// Offset of the sub-object: non-zero for secondary vtables.
    pub offset: u32,
    pub slots: Vec<u32>,
}

pub struct Rtti {
    pub type_info_vftable: u32,
    /// Address of the type descriptor -> decorated name.
    pub types: HashMap<u32, String>,
    pub vtables: Vec<Vtable>,
}

/// `.?AVFoo@bar@@` -> `bar::Foo`. Templates stay decorated.
pub fn undecorate(name: &str) -> String {
    if name.len() < 6 || !name.starts_with(".?A") || !name.ends_with("@@") {
        return name.to_string();
    }
    let body = &name[4..name.len() - 2];
    if body.contains('?') {
        return name.to_string();
    }
    body.split('@').rev().collect::<Vec<_>>().join("::")
}

pub fn analyze(img: &Image) -> Rtti {
    let text = img.section(".text").expect(".text section");
    let (text_lo, text_hi) = (text.va, text.file_end());

    // 1. Type descriptors: ".?A" strings preceded by {pointer to type_info::vftable, 0}.
    let mut cands: Vec<(u32, u32, String)> = Vec::new();
    for s in img.data_sections() {
        let blob = img.section_bytes(s);
        let mut i = 0;
        while let Some(p) = find(blob, b".?A", i) {
            if p >= 8 {
                let vft = u32::from_le_bytes(blob[p - 8..p - 4].try_into().unwrap());
                let end = blob[p..].iter().position(|&b| b == 0).unwrap_or(0);
                let name = String::from_utf8_lossy(&blob[p..p + end]).into_owned();
                cands.push((s.va + p as u32 - 8, vft, name));
            }
            i = p + 1;
        }
    }
    let mut votes: HashMap<u32, usize> = HashMap::new();
    for c in &cands {
        *votes.entry(c.1).or_default() += 1;
    }
    let type_info_vftable = votes.into_iter().max_by_key(|&(_, n)| n).map(|(v, _)| v).unwrap_or(0);
    let types: HashMap<u32, String> =
        cands.into_iter().filter(|c| c.1 == type_info_vftable).map(|c| (c.0, c.2)).collect();

    // 2. Complete object locators: signature 0, known type descriptor, plausible hierarchy.
    let in_data = |a: u32| img.data_sections().any(|s| s.contains(a));
    let mut cols: HashMap<u32, (u32, u32)> = HashMap::new(); // address -> (offset, type descriptor)
    for s in img.data_sections() {
        let w: Vec<(u32, u32)> = img.words(s).collect();
        for i in 0..w.len().saturating_sub(4) {
            if w[i].1 != 0 || !types.contains_key(&w[i + 3].1) || !in_data(w[i + 4].1) {
                continue;
            }
            let chd = w[i + 4].1;
            let (Some(sig), Some(nbases), Some(parr)) = (img.u32(chd), img.u32(chd + 8), img.u32(chd + 12)) else {
                continue;
            };
            if sig != 0 || nbases == 0 || nbases >= 256 {
                continue;
            }
            let ok = (0..nbases).all(|k| {
                img.u32(parr + 4 * k)
                    .and_then(|bcd| img.u32(bcd))
                    .is_some_and(|td| types.contains_key(&td))
            });
            if ok {
                cols.insert(w[i].0, (w[i + 1].1, w[i + 3].1));
            }
        }
    }

    // 3. Vtables: a word that points to a COL, followed by code pointers.
    let mut vtables = Vec::new();
    for s in img.data_sections() {
        let w: Vec<(u32, u32)> = img.words(s).collect();
        let col_ptrs: HashSet<usize> = (0..w.len()).filter(|&i| cols.contains_key(&w[i].1)).collect();
        let mut idx: Vec<usize> = col_ptrs.iter().copied().collect();
        idx.sort_unstable();
        for i in idx {
            let mut slots = Vec::new();
            let mut j = i + 1;
            while j < w.len() && text_lo <= w[j].1 && w[j].1 < text_hi && !col_ptrs.contains(&j) {
                slots.push(w[j].1);
                j += 1;
            }
            if !slots.is_empty() {
                let (offset, td) = cols[&w[i].1];
                vtables.push(Vtable { addr: w[i].0 + 4, class: types[&td].clone(), offset, slots });
            }
        }
    }
    Rtti { type_info_vftable, types, vtables }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}
