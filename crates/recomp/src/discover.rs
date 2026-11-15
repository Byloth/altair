//! Function discovery without symbols or relocations.
//!
//! Starts, in order of reliability:
//!  1. strong: entry point, exports, TLS callbacks and every direct `call` from the reached code;
//!  2. slots of the vtables found through RTTI;
//!  3. code addresses used as immediates;
//!  4. words in the data sections that point into the code.
//! Starts 2-4 are weak: they are accepted only if the trial decode does not run into
//! invalid bytes, instructions a game does not execute, or the middle of a known instruction.
//! 3 and 4 must also "look like a start". Method and limits: docs/analysis/functions.md.

use crate::rtti;
use altair_pe::Image;
use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction, Mnemonic, OpKind, Register};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Entry,
    Export,
    Tls,
    Vtable,
    Immediate,
    DataWord,
    /// Word in the data at an address not aligned to 4: packed tables of the CRT.
    Unaligned,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Entry => "entry",
            Kind::Export => "export",
            Kind::Tls => "TLS callback",
            Kind::Vtable => "vtable",
            Kind::Immediate => "immediate",
            Kind::DataWord => "data word",
            Kind::Unaligned => "unaligned word",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Origin {
    pub kind: Kind,
    /// Reached by a direct `call` starting from a start of that kind.
    pub via_call: bool,
}

const UNKNOWN: u8 = 0;
const INSN: u8 = 1;
const INSIDE: u8 = 2;
const TABLE: u8 = 3;

pub struct Discovery<'a> {
    img: &'a Image,
    pub lo: u32,
    pub hi: u32,
    text: &'a [u8],
    /// One byte per byte of `.text`: unknown, start of instruction, inside, jump table data.
    pub own: Vec<u8>,
    pub starts: BTreeMap<u32, Origin>,
    imms: HashSet<u32>,
    table_slots: HashSet<u32>,
    pub tables: usize,
    pub bounded_tables: usize,
    pub n_insn: usize,
    pub rejected: BTreeMap<String, usize>,
    strict: bool,
    /// `--why ADDR`: explains why this start is accepted or discarded.
    pub debug: Option<u32>,
    last: u32,
    boundaries_only: bool,
}

struct Trial {
    journal: Vec<(usize, usize)>,
    calls: Vec<u32>,
    imms: Vec<u32>,
    slots: Vec<u32>,
    tables: usize,
    bounded: usize,
}

fn never_executed(m: Mnemonic) -> bool {
    use Mnemonic::*;
    matches!(
        m,
        In | Out | Insb | Insd | Insw | Outsb | Outsd | Outsw | Les | Lds | Aaa | Aas | Aam | Aad | Daa | Das
            | Iret | Iretd | Hlt | Cli | Sti | Salc | Int1 | Into | Bound | Arpl | Retf | Int | Lgdt | Lidt
            | Enter | Clts | Loopne | Loope
    )
}

/// Number of entries of a jump table derived from the `cmp x, N; ja default` that precedes it.
/// For two-level tables (`movzx r, byte [r + B]`) the bound applies to the byte table,
/// and the entries are the maximum index plus one.
pub fn table_bound(img: &Image, hist: &[Instruction]) -> Option<usize> {
    let n = hist.len();
    let from = n.saturating_sub(10);
    let cmp_at = (from..n.saturating_sub(1)).rev().find(|&i| {
        hist[i].mnemonic() == Mnemonic::Cmp
            && matches!(hist[i].op1_kind(), OpKind::Immediate8to32 | OpKind::Immediate32)
            && hist[i + 1].mnemonic() == Mnemonic::Ja
    })?;
    let count = hist[cmp_at].immediate32() as usize + 1;
    if count > 4096 {
        return None;
    }
    let byte_table = hist[cmp_at + 2..].iter().find(|i| {
        i.mnemonic() == Mnemonic::Movzx
            && i.op1_kind() == OpKind::Memory
            && i.memory_index() == Register::None
            && i.memory_base() != Register::None
            && img.off(i.memory_displacement32()).is_some()
    });
    match byte_table {
        Some(i) => {
            let bytes = img.bytes(i.memory_displacement32(), count)?;
            Some(*bytes.iter().max()? as usize + 1)
        }
        None => Some(count),
    }
}

impl<'a> Discovery<'a> {
    pub fn new(img: &'a Image, strict: bool) -> Self {
        let text_sec = img.section(".text").expect(".text section");
        let text = img.section_bytes(text_sec);
        Discovery {
            img,
            lo: text_sec.va,
            hi: text_sec.va + text.len() as u32,
            text,
            own: vec![UNKNOWN; text.len()],
            starts: BTreeMap::new(),
            imms: HashSet::new(),
            table_slots: HashSet::new(),
            tables: 0,
            bounded_tables: 0,
            n_insn: 0,
            rejected: BTreeMap::new(),
            strict,
            debug: std::env::var("ALTAIR_WHY").ok().and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()),
            last: 0,
            boundaries_only: false,
        }
    }

    fn in_text(&self, a: u32) -> bool {
        self.lo <= a && a < self.hi
    }

    fn reject(&mut self, why: &str) {
        *self.rejected.entry(why.to_string()).or_default() += 1;
    }

    fn read_table(&self, t: u32, bound: Option<usize>) -> Vec<u32> {
        let max = bound.unwrap_or(1024);
        let mut out = Vec::new();
        while out.len() < max {
            let a = t + 4 * out.len() as u32;
            let Some(v) = self.img.u32(a) else { break };
            if !self.in_text(v) {
                break;
            }
            if self.in_text(a) && matches!(self.own[(a - self.lo) as usize], INSN | INSIDE) {
                break;
            }
            out.push(v);
        }
        out
    }

    /// Decodes the code reachable from `start`, marking `own`. On error it returns the
    /// reason; what has been marked stays in the `Trial` so it can be undone.
    fn explore(&mut self, start: u32, t: &mut Trial) -> Result<(), &'static str> {
        let mut work = vec![start];
        let mut hist: Vec<Instruction> = Vec::new();
        let mut ins = Instruction::default();
        while let Some(a) = work.pop() {
            if !self.in_text(a) {
                return Err("jump outside .text");
            }
            let o = (a - self.lo) as usize;
            match self.own[o] {
                INSN => continue,
                UNKNOWN => {}
                _ => return Err("inside an instruction or a table"),
            }
            hist.clear();
            let mut dec = Decoder::with_ip(32, &self.text[o..], a as u64, DecoderOptions::NONE);
            loop {
                if !dec.can_decode() {
                    return Err("undecodable bytes");
                }
                dec.decode_out(&mut ins);
                if ins.is_invalid() {
                    return Err("undecodable bytes");
                }
                let addr = ins.ip() as u32;
                self.last = addr;
                let p = (addr - self.lo) as usize;
                let len = ins.len();
                if self.own[p] == INSN {
                    break;
                }
                if self.own[p..p + len].iter().any(|&b| b != UNKNOWN) {
                    return Err("overlap");
                }
                let m = ins.mnemonic();
                // `push es`, `pop ds`…: these are the bytes 06, 07, 0E, 16, 17, 1E, 1F of the index
                // tables of two-level switches, not 32-bit code.
                let seg_stack = matches!(m, Mnemonic::Push | Mnemonic::Pop)
                    && ins.op0_kind() == OpKind::Register
                    && ins.op0_register().is_segment_register();
                if never_executed(m) || seg_stack || (m == Mnemonic::Int3 && addr == start) {
                    return Err("unlikely instruction");
                }
                self.own[p] = INSN;
                self.own[p + 1..p + len].fill(INSIDE);
                t.journal.push((p, len));

                match ins.flow_control() {
                    FlowControl::Next => {
                        for k in 0..ins.op_count() {
                            let v = match ins.op_kind(k) {
                                OpKind::Immediate32 => ins.immediate32(),
                                OpKind::Memory => ins.memory_displacement32(),
                                _ => continue,
                            };
                            if self.in_text(v) {
                                t.imms.push(v);
                            }
                        }
                        hist.push(ins);
                    }
                    FlowControl::ConditionalBranch => {
                        work.push(ins.near_branch_target() as u32);
                        hist.push(ins);
                    }
                    FlowControl::UnconditionalBranch => {
                        if !matches!(ins.op0_kind(), OpKind::NearBranch32 | OpKind::NearBranch16) {
                            return Err("unlikely instruction");
                        }
                        work.push(ins.near_branch_target() as u32);
                        break;
                    }
                    FlowControl::IndirectBranch => {
                        if ins.op0_kind() == OpKind::Memory
                            && ins.memory_index() != Register::None
                            && ins.memory_index_scale() == 4
                        {
                            let table = ins.memory_displacement32();
                            let bound = table_bound(self.img, &hist);
                            let entries = self.read_table(table, bound);
                            if !entries.is_empty() {
                                t.tables += 1;
                                t.bounded += bound.is_some() as usize;
                                t.slots.extend((0..entries.len() as u32).map(|i| table + 4 * i));
                                if self.in_text(table) {
                                    let q = (table - self.lo) as usize;
                                    let n = 4 * entries.len();
                                    if self.own[q..q + n].iter().any(|&b| b != UNKNOWN) {
                                        return Err("table over code");
                                    }
                                    self.own[q..q + n].fill(TABLE);
                                    t.journal.push((q, n));
                                }
                                work.extend(entries);
                            }
                        }
                        break;
                    }
                    FlowControl::Call => {
                        if !matches!(ins.op0_kind(), OpKind::NearBranch32 | OpKind::NearBranch16) {
                            return Err("unlikely instruction");
                        }
                        let target = ins.near_branch_target() as u32;
                        if !self.in_text(target) {
                            return Err("call outside .text");
                        }
                        t.calls.push(target);
                        hist.push(ins);
                    }
                    FlowControl::IndirectCall => hist.push(ins),
                    FlowControl::Return | FlowControl::Interrupt | FlowControl::Exception => break,
                    FlowControl::XbeginXabortXend => return Err("unlikely instruction"),
                }
            }
        }
        Ok(())
    }

    /// Explores a start; `None` if it was discarded (only for tentative starts).
    fn try_function(&mut self, start: u32, tentative: bool) -> Option<Vec<u32>> {
        let mut t = Trial { journal: Vec::new(), calls: Vec::new(), imms: Vec::new(), slots: Vec::new(), tables: 0, bounded: 0 };
        if let Err(why) = self.explore(start, &mut t) {
            if self.debug == Some(start) {
                eprintln!("[why] {start:#x} discarded: {why}, at instruction {:#x} (tentative: {tentative})", self.last);
            }
            if tentative {
                for &(p, n) in &t.journal {
                    self.own[p..p + n].fill(UNKNOWN);
                }
                self.reject(why);
                return None;
            }
            self.reject(&format!("[strong] {why}"));
        }
        self.n_insn += t.journal.iter().filter(|&&(p, _)| self.own[p] == INSN).count();
        self.tables += t.tables;
        self.bounded_tables += t.bounded;
        self.table_slots.extend(t.slots);
        self.imms.extend(t.imms);
        Some(t.calls)
    }

    /// Adds a function and, in a chain, the ones it calls directly.
    fn add(&mut self, addr: u32, kind: Kind, tentative: bool) -> bool {
        if self.starts.contains_key(&addr) {
            return true;
        }
        let Some(mut pending) = self.try_function(addr, tentative) else {
            return false;
        };
        self.starts.insert(addr, Origin { kind, via_call: false });
        while let Some(t) = pending.pop() {
            if self.starts.contains_key(&t) {
                continue;
            }
            if let Some(more) = self.try_function(t, tentative) {
                self.starts.insert(t, Origin { kind, via_call: true });
                pending.extend(more);
            }
        }
        true
    }

    fn looks_like_start(&self, a: u32) -> bool {
        let o = (a - self.lo) as usize;
        if o <= 5 {
            return true;
        }
        let t = self.text;
        // A function that is just `ret` or `ret n`: often it is the tail of another one, used as
        // a callback that does nothing.
        if matches!(t[o], 0xC3 | 0xC2) {
            return true;
        }
        // Aligned to 16, or right after padding, `ret`, `ret n`.
        if matches!(t[o - 1], 0xCC | 0xC3) || t[o - 3] == 0xC2 {
            return true;
        }
        // In the first round that is enough: only certain boundaries. An address that is merely
        // aligned can fall in the middle of a real function and, accepted first, would get it discarded.
        if self.boundaries_only {
            return t[o - 1] == 0x90 && (matches!(t[o - 2], 0x90 | 0xC3) || t[o - 4] == 0xC2);
        }
        if a % 16 == 0 {
            return true;
        }
        // After padding `nop`s: at least two, or one after a `ret`. A single 0x90 is often
        // the ModRM byte of an instruction (`mov edx, [eax+…]`).
        if t[o - 1] == 0x90 && (matches!(t[o - 2], 0x90 | 0xC3) || t[o - 4] == 0xC2) {
            return true;
        }
        // Loose rule: also right after `jmp rel32` or `jmp rel8`.
        !self.strict && (t[o - 5] == 0xE9 || t[o - 2] == 0xEB)
    }

    fn weak(&mut self, addr: u32, kind: Kind, need_look: bool) {
        if self.starts.contains_key(&addr) || !self.in_text(addr) {
            return;
        }
        let state = self.own[(addr - self.lo) as usize];
        if self.debug == Some(addr) {
            eprintln!("[why] {addr:#x} proposed as {:?}: state of the first byte {state}, looks like a start: {}", kind, self.looks_like_start(addr));
        }
        if state == INSIDE || state == TABLE {
            self.reject("inside an instruction or a table");
        } else if need_look && !self.looks_like_start(addr) {
            self.reject("does not look like a start");
        } else if state == INSN {
            // Code already reached (tail call, fall-through): it is an entry point all the same.
            self.starts.insert(addr, Origin { kind, via_call: false });
        } else {
            self.add(addr, kind, true);
        }
    }

    pub fn run(img: &'a Image, strict: bool, log: bool) -> (Self, rtti::Rtti) {
        let mut d = Discovery::new(img, strict);
        let say = |d: &Discovery, what: &str| {
            if log {
                eprintln!("{what}: functions={} instructions={}", d.starts.len(), d.n_insn);
            }
        };

        d.add(img.entry, Kind::Entry, false);
        for e in &img.exports {
            if d.in_text(e.addr) {
                d.add(e.addr, Kind::Export, false);
            }
        }
        for &cb in &img.tls_callbacks {
            d.add(cb, Kind::Tls, false);
        }
        say(&d, "strong starts");

        let r = rtti::analyze(img);
        for v in &r.vtables {
            for &s in &v.slots {
                d.weak(s, Kind::Vtable, false);
            }
        }
        say(&d, "vtable slots");

        let data_words: Vec<(u32, u32)> = img
            .data_sections()
            .flat_map(|s| img.words(s))
            .filter(|&(_, v)| d.in_text(v))
            .collect();
        // Two passes. In the first one a data word must "look like a start"; only when
        // that one finds nothing more does the second also accept entries of function pointer
        // tables that do not look like one. Order matters: a false start accepted first
        // takes up the bytes of a real function and gets it discarded.
        let mut seen: HashSet<u32> = HashSet::new();
        // Three passes, strictest first: (0) only starts preceded by padding or by a
        // `ret`; (1) also those that are merely aligned or after a `jmp`; (2) also entries of
        // pointer tables that do not look like starts.
        for tier in 0..3 {
            let relaxed = tier == 2;
            d.boundaries_only = tier == 0;
            if tier == 1 {
                seen.clear();
            }
            for round in 1..=8 {
                let before = d.starts.len();
                for (i, &(a, v)) in data_words.iter().enumerate() {
                    if d.table_slots.contains(&a) {
                        continue;
                    }
                    let neighbour = |j: usize, addr: u32| data_words.get(j).is_some_and(|&(b, w)| b == addr && d.starts.contains_key(&w));
                    let in_table = relaxed && ((i > 0 && neighbour(i - 1, a - 4)) || neighbour(i + 1, a + 4));
                    d.weak(v, Kind::DataWord, !in_table);
                }
                // Immediates after the data: they are the source with the most false positives
                // (numeric constants that happen to fall within the code range).
                let mut fresh: Vec<u32> = d.imms.difference(&seen).copied().collect();
                fresh.sort_unstable();
                for v in fresh {
                    seen.insert(v);
                    d.weak(v, Kind::Immediate, true);
                }
                say(&d, &format!("pass {tier}, round {round}: immediates and data"));
                if d.starts.len() == before {
                    break;
                }
            }
        }

        // The CRT math routines jump through pointer tables packed without
        // alignment (the targets are labels inside the routines). They are looked for
        // only in `.data`, and only targets that follow a `ret` or a `jmp`.
        if let Some(data) = img.section(".data") {
            let bytes = img.section_bytes(data);
            let mut found = Vec::new();
            for off in (0..bytes.len().saturating_sub(4)).filter(|o| o % 4 != 0) {
                let v = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
                if d.in_text(v) && v % 16 != 0 && !d.starts.contains_key(&v) {
                    found.push(v);
                }
            }
            found.sort_unstable();
            found.dedup();
            for v in found {
                let o = (v - d.lo) as usize;
                let t = d.text;
                let after_transfer = o > 5 && (t[o - 1] == 0xC3 || t[o - 3] == 0xC2 || t[o - 5] == 0xE9 || t[o - 2] == 0xEB);
                if after_transfer {
                    d.weak(v, Kind::Unaligned, false);
                }
            }
            // Proper tables: at least three consecutive code pointers, with a stride of 4, at an
            // unaligned address. Their entries are arbitrary labels inside the routines.
            for residue in 1..4usize {
                let mut run: Vec<u32> = Vec::new();
                let mut off = residue;
                loop {
                    let v = bytes.get(off..off + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
                    match v {
                        Some(v) if d.in_text(v) => run.push(v),
                        _ => {
                            if run.len() >= 3 {
                                for &t in &run {
                                    d.weak(t, Kind::Unaligned, false);
                                }
                            }
                            run.clear();
                            if v.is_none() {
                                break;
                            }
                        }
                    }
                    off += 4;
                }
            }
            say(&d, "unaligned words in .data");
        }
        (d, r)
    }

    /// (code, jump tables, unreached int3, unclassified) in bytes.
    pub fn text_breakdown(&self) -> (usize, usize, usize, usize) {
        let (mut code, mut table, mut int3, mut unknown) = (0, 0, 0, 0);
        for (i, &b) in self.own.iter().enumerate() {
            match b {
                INSN | INSIDE => code += 1,
                TABLE => table += 1,
                _ if self.text[i] == 0xCC => int3 += 1,
                _ => unknown += 1,
            }
        }
        (code, table, int3, unknown)
    }
}
