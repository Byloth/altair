//! Scoperta delle funzioni senza simboli né rilocazioni.
//!
//! Partenze, in ordine di affidabilità:
//!  1. forti: entry point, export, callback TLS e ogni `call` diretta dal codice raggiunto;
//!  2. slot delle vtable trovate dall'RTTI;
//!  3. indirizzi di codice usati come immediati;
//!  4. parole nelle sezioni dati che puntano nel codice.
//! Le partenze 2-4 sono deboli: si accettano solo se la decodifica di prova non incontra
//! byte non validi, istruzioni che un gioco non esegue, o il mezzo di un'istruzione nota.
//! Le 3 e 4 devono anche "sembrare un inizio". Metodo e limiti: docs/analisi/funzioni.md.

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
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Entry => "entry",
            Kind::Export => "export",
            Kind::Tls => "callback TLS",
            Kind::Vtable => "vtable",
            Kind::Immediate => "immediato",
            Kind::DataWord => "parola nei dati",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Origin {
    pub kind: Kind,
    /// Raggiunta da una `call` diretta a partire da una partenza di quel tipo.
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
    /// Un byte per byte di `.text`: ignoto, inizio di istruzione, interno, dati di jump table.
    pub own: Vec<u8>,
    pub starts: BTreeMap<u32, Origin>,
    imms: HashSet<u32>,
    table_slots: HashSet<u32>,
    pub tables: usize,
    pub bounded_tables: usize,
    pub n_insn: usize,
    pub rejected: BTreeMap<String, usize>,
    strict: bool,
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
            | Xlatb | Popa | Popad | Pusha | Pushad
    )
}

/// Numero di voci di una jump table ricavato dal `cmp x, N; ja default` che la precede.
/// Per le tabelle a due livelli (`movzx r, byte [r + B]`) il limite vale per la tabella di
/// byte, e le voci sono il massimo indice più uno.
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
        let text_sec = img.section(".text").expect("sezione .text");
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

    /// Decodifica il codice raggiungibile da `start`, marcando `own`. In caso di errore
    /// ritorna il motivo; ciò che è stato marcato resta nel `Trial` per essere annullato.
    fn explore(&mut self, start: u32, t: &mut Trial) -> Result<(), &'static str> {
        let mut work = vec![start];
        let mut hist: Vec<Instruction> = Vec::new();
        let mut ins = Instruction::default();
        while let Some(a) = work.pop() {
            if !self.in_text(a) {
                return Err("salto fuori da .text");
            }
            let o = (a - self.lo) as usize;
            match self.own[o] {
                INSN => continue,
                UNKNOWN => {}
                _ => return Err("dentro un'istruzione o una tabella"),
            }
            hist.clear();
            let mut dec = Decoder::with_ip(32, &self.text[o..], a as u64, DecoderOptions::NONE);
            loop {
                if !dec.can_decode() {
                    return Err("byte non decodificabili");
                }
                dec.decode_out(&mut ins);
                if ins.is_invalid() {
                    return Err("byte non decodificabili");
                }
                let addr = ins.ip() as u32;
                let p = (addr - self.lo) as usize;
                let len = ins.len();
                if self.own[p] == INSN {
                    break;
                }
                if self.own[p..p + len].iter().any(|&b| b != UNKNOWN) {
                    return Err("sovrapposizione");
                }
                let m = ins.mnemonic();
                if never_executed(m) || (m == Mnemonic::Int3 && addr == start) {
                    return Err("istruzione improbabile");
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
                            return Err("istruzione improbabile");
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
                                        return Err("tabella sopra codice");
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
                            return Err("istruzione improbabile");
                        }
                        let target = ins.near_branch_target() as u32;
                        if !self.in_text(target) {
                            return Err("call fuori da .text");
                        }
                        t.calls.push(target);
                        hist.push(ins);
                    }
                    FlowControl::IndirectCall => hist.push(ins),
                    FlowControl::Return | FlowControl::Interrupt | FlowControl::Exception => break,
                    FlowControl::XbeginXabortXend => return Err("istruzione improbabile"),
                }
            }
        }
        Ok(())
    }

    /// Esplora una partenza; `None` se è stata scartata (solo per le partenze di prova).
    fn try_function(&mut self, start: u32, tentative: bool) -> Option<Vec<u32>> {
        let mut t = Trial { journal: Vec::new(), calls: Vec::new(), imms: Vec::new(), slots: Vec::new(), tables: 0, bounded: 0 };
        if let Err(why) = self.explore(start, &mut t) {
            if tentative {
                for &(p, n) in &t.journal {
                    self.own[p..p + n].fill(UNKNOWN);
                }
                self.reject(why);
                return None;
            }
            self.reject(&format!("[forte] {why}"));
        }
        self.n_insn += t.journal.iter().filter(|&&(p, _)| self.own[p] == INSN).count();
        self.tables += t.tables;
        self.bounded_tables += t.bounded;
        self.table_slots.extend(t.slots);
        self.imms.extend(t.imms);
        Some(t.calls)
    }

    /// Aggiunge una funzione e, a catena, quelle che chiama direttamente.
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
        // Allineata a 16, oppure subito dopo padding, `ret`, `ret n`.
        if a % 16 == 0 || matches!(t[o - 1], 0xCC | 0xC3 | 0x90) || t[o - 3] == 0xC2 {
            return true;
        }
        // Regola larga: anche subito dopo `jmp rel32` o `jmp rel8`.
        !self.strict && (t[o - 5] == 0xE9 || t[o - 2] == 0xEB)
    }

    fn weak(&mut self, addr: u32, kind: Kind, need_look: bool) {
        if self.starts.contains_key(&addr) || !self.in_text(addr) {
            return;
        }
        let state = self.own[(addr - self.lo) as usize];
        if state == INSIDE || state == TABLE {
            self.reject("dentro un'istruzione o una tabella");
        } else if need_look && !self.looks_like_start(addr) {
            self.reject("non sembra un inizio");
        } else if state == INSN {
            // Codice già raggiunto (tail call, fall-through): è comunque un punto d'ingresso.
            self.starts.insert(addr, Origin { kind, via_call: false });
        } else {
            self.add(addr, kind, true);
        }
    }

    pub fn run(img: &'a Image, strict: bool, log: bool) -> (Self, rtti::Rtti) {
        let mut d = Discovery::new(img, strict);
        let say = |d: &Discovery, what: &str| {
            if log {
                eprintln!("{what}: funzioni={} istruzioni={}", d.starts.len(), d.n_insn);
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
        say(&d, "partenze forti");

        let r = rtti::analyze(img);
        for v in &r.vtables {
            for &s in &v.slots {
                d.weak(s, Kind::Vtable, false);
            }
        }
        say(&d, "slot delle vtable");

        let data_words: Vec<(u32, u32)> = img
            .data_sections()
            .flat_map(|s| img.words(s))
            .filter(|&(_, v)| d.in_text(v))
            .collect();
        let mut seen: HashSet<u32> = HashSet::new();
        for round in 1..=8 {
            let before = d.starts.len();
            let mut fresh: Vec<u32> = d.imms.difference(&seen).copied().collect();
            fresh.sort_unstable();
            for v in fresh {
                seen.insert(v);
                d.weak(v, Kind::Immediate, true);
            }
            for &(a, v) in &data_words {
                if !d.table_slots.contains(&a) {
                    d.weak(v, Kind::DataWord, true);
                }
            }
            say(&d, &format!("giro {round}: immediati e dati"));
            if d.starts.len() == before {
                break;
            }
        }
        (d, r)
    }

    /// (codice, jump table, int3 non raggiunti, non classificati) in byte.
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
