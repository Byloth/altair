//! Traduzione di una funzione x86 in C.
//!
//! Ogni funzione guest diventa `void f_XXXXXXXX(Ctx *c)`:
//!  - i registri stanno nel contesto, la memoria guest si legge con `rdN`/`wrN`;
//!  - i salti interni sono `goto`, le jump table uno `switch` sugli indirizzi di destinazione;
//!  - una `call` spinge il vero indirizzo di ritorno sullo stack guest e chiama la funzione
//!    tradotta; `ret` lo toglie. Un salto verso un'altra funzione è una tail call;
//!  - i flag sono variabili locali calcolate dopo ogni istruzione che li scrive: il compilatore
//!    C elimina quelle che nessuno legge.
//! Le istruzioni non ancora supportate fanno fallire la traduzione dell'intera funzione, con
//! il nome dell'istruzione: `altair coverage` le conta.

use crate::discover::table_bound;
use altair_pe::Image;
use iced_x86::{
    ConditionCode, Decoder, DecoderOptions, FlowControl, Formatter, Instruction, IntelFormatter, Mnemonic, OpKind,
    Register,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

pub struct Func {
    pub start: u32,
    pub insns: BTreeMap<u32, Instruction>,
    labels: BTreeSet<u32>,
    tables: HashMap<u32, Vec<u32>>,
    /// Destinazioni di `call` dirette e di tail call: altre funzioni.
    pub calls: BTreeSet<u32>,
}

impl Func {
    pub fn empty(start: u32) -> Func {
        Func { start, insns: BTreeMap::new(), labels: BTreeSet::new(), tables: HashMap::new(), calls: BTreeSet::new() }
    }

    /// Contiene chiamate o salti indiretti la cui destinazione non è nota staticamente.
    pub fn has_opaque_indirect(&self) -> bool {
        self.insns.values().any(|i| match i.flow_control() {
            FlowControl::IndirectCall => true,
            FlowControl::IndirectBranch => !self.tables.contains_key(&(i.ip() as u32)),
            _ => false,
        })
    }
}

type R<T> = Result<T, String>;

/// Decodifica il corpo di una funzione seguendo il flusso di controllo da `start`.
/// `hard_start` dice se un indirizzo è l'inizio certo di un'altra funzione: un salto lì è una
/// tail call. Le partenze deboli restano codice interno (al più duplicato in due funzioni).
pub fn decode(img: &Image, start: u32, hard_start: &dyn Fn(u32) -> bool) -> R<Func> {
    let text_sec = img.section(".text").ok_or("manca .text")?;
    let text = img.section_bytes(text_sec);
    let (lo, hi) = (text_sec.va, text_sec.va + text.len() as u32);
    let mut f = Func { start, insns: BTreeMap::new(), labels: BTreeSet::new(), tables: HashMap::new(), calls: BTreeSet::new() };
    let external = |t: u32| t != start && hard_start(t);
    let mut work = vec![start];
    let mut hist: Vec<Instruction> = Vec::new();
    let mut ins = Instruction::default();
    while let Some(a) = work.pop() {
        if f.insns.contains_key(&a) {
            continue;
        }
        if a < lo || a >= hi {
            return Err(format!("salto fuori da .text ({a:#x})"));
        }
        hist.clear();
        let mut dec = Decoder::with_ip(32, &text[(a - lo) as usize..], a as u64, DecoderOptions::NONE);
        loop {
            if !dec.can_decode() {
                return Err("fine di .text".into());
            }
            dec.decode_out(&mut ins);
            if ins.is_invalid() {
                return Err(format!("byte non decodificabili a {:#x}", ins.ip()));
            }
            let addr = ins.ip() as u32;
            if addr != a && external(addr) {
                // Il codice prosegue dentro un'altra funzione: è una tail call verso di essa.
                f.calls.insert(addr);
                break;
            }
            if addr != a && f.insns.contains_key(&addr) {
                break;
            }
            f.insns.insert(addr, ins);
            let branch = |f: &mut Func, work: &mut Vec<u32>, t: u32| {
                if external(t) {
                    f.calls.insert(t);
                } else {
                    f.labels.insert(t);
                    work.push(t);
                }
            };
            match ins.flow_control() {
                FlowControl::Next | FlowControl::IndirectCall => hist.push(ins),
                FlowControl::ConditionalBranch => {
                    branch(&mut f, &mut work, ins.near_branch_target() as u32);
                    hist.push(ins);
                }
                FlowControl::UnconditionalBranch => {
                    if !matches!(ins.op0_kind(), OpKind::NearBranch32) {
                        return Err("salto far".into());
                    }
                    branch(&mut f, &mut work, ins.near_branch_target() as u32);
                    break;
                }
                FlowControl::IndirectBranch => {
                    if ins.op0_kind() == OpKind::Memory && ins.memory_index() != Register::None && ins.memory_index_scale() == 4 {
                        let table = ins.memory_displacement32();
                        let max = table_bound(img, &hist).unwrap_or(1024);
                        let entry = |k: i64| -> Option<u32> {
                            let v = img.u32((table as i64 + 4 * k) as u32)?;
                            (lo <= v && v < hi).then_some(v)
                        };
                        // Per le estensioni euristiche qui sotto: una voce vera punta vicino alla
                        // tabella, che MSVC mette subito dopo la funzione. Scarta i byte di
                        // padding che per caso formano un indirizzo di codice.
                        let near = |k: i64| entry(k).filter(|&v| v.abs_diff(table) < 0x2000);
                        let mut entries = Vec::new();
                        while entries.len() < max {
                            match entry(entries.len() as i64) {
                                Some(v) => entries.push(v),
                                None => break,
                            }
                        }
                        let negated = hist.iter().rev().take(3).any(|i| {
                            i.mnemonic() == Mnemonic::Neg && i.op0_register() == ins.memory_index()
                        });
                        if negated {
                            // `neg ecx; jmp [ecx*4 + T]`: la tabella si estende all'indietro da T.
                            entries.truncate(1);
                            entries.extend((1..=16).map_while(|k| near(-k)));
                        }
                        if entries.is_empty() {
                            // La base non è una voce valida: il `memcpy` di MSVC indicizza alcune
                            // tabelle da 1 e altre con indici negativi. Si prendono le voci
                            // valide contigue subito dopo e subito prima della base.
                            entries.extend((1..=8).map_while(near));
                            entries.extend((1..=8).map_while(|k| near(-k)));
                        }
                        if !entries.is_empty() {
                            for &e in &entries {
                                f.labels.insert(e);
                                work.push(e);
                            }
                            f.tables.insert(addr, entries);
                        }
                    }
                    break;
                }
                FlowControl::Call => {
                    if !matches!(ins.op0_kind(), OpKind::NearBranch32) {
                        return Err("call far".into());
                    }
                    f.calls.insert(ins.near_branch_target() as u32);
                    hist.push(ins);
                }
                FlowControl::Return | FlowControl::Interrupt | FlowControl::Exception => break,
                FlowControl::XbeginXabortXend => return Err("xbegin".into()),
            }
        }
    }
    // Dove il flusso prosegue su un'istruzione che non è la successiva nell'ordine di
    // emissione serve un'etichetta per il `goto` esplicito.
    let keys: Vec<u32> = f.insns.keys().copied().collect();
    for (i, a) in keys.iter().enumerate() {
        let ins = &f.insns[a];
        let next = ins.next_ip() as u32;
        if falls_through(ins) && f.insns.contains_key(&next) && keys.get(i + 1) != Some(&next) {
            f.labels.insert(next);
        }
    }
    if keys.first() != Some(&start) {
        f.labels.insert(start);
    }
    Ok(f)
}

fn falls_through(ins: &Instruction) -> bool {
    matches!(
        ins.flow_control(),
        FlowControl::Next | FlowControl::ConditionalBranch | FlowControl::Call | FlowControl::IndirectCall
    )
}

pub fn fn_name(addr: u32) -> String {
    format!("f_{addr:08x}")
}

fn reg32(r: Register) -> R<&'static str> {
    if !r.is_gpr() {
        return Err(format!("registro {r:?}"));
    }
    Ok(match r.full_register32() {
        Register::EAX => "c->eax",
        Register::ECX => "c->ecx",
        Register::EDX => "c->edx",
        Register::EBX => "c->ebx",
        Register::ESP => "c->esp",
        Register::EBP => "c->ebp",
        Register::ESI => "c->esi",
        Register::EDI => "c->edi",
        other => return Err(format!("registro {other:?}")),
    })
}

/// L'accumulatore della larghezza data: al, ax, eax.
fn acc_reg(bits: u32) -> Register {
    match bits {
        8 => Register::AL,
        16 => Register::AX,
        _ => Register::EAX,
    }
}

fn is_high(r: Register) -> bool {
    matches!(r, Register::AH | Register::CH | Register::DH | Register::BH)
}

fn rd_reg(r: Register) -> R<String> {
    let full = reg32(r)?;
    Ok(match r.size() {
        4 => full.to_string(),
        2 => format!("(u16){full}"),
        1 if is_high(r) => format!("(u8)({full} >> 8)"),
        1 => format!("(u8){full}"),
        _ => return Err(format!("registro {r:?}")),
    })
}

fn wr_reg(r: Register, v: &str) -> R<String> {
    let full = reg32(r)?;
    Ok(match r.size() {
        4 => format!("{full} = {v};"),
        // Scritture strette: vedi SET8L in runtime/altair.h e tests/compiler/.
        2 => format!("SET16({full}, {v});"),
        1 if is_high(r) => format!("SET8H({full}, {v});"),
        1 => format!("SET8L({full}, {v});"),
        _ => return Err(format!("registro {r:?}")),
    })
}

/// Indirizzo effettivo dell'operando di memoria, come espressione `u32`.
fn ea(ins: &Instruction) -> R<String> {
    let mut parts: Vec<String> = Vec::new();
    for r in [ins.memory_base(), ins.memory_index()] {
        if r != Register::None && r.size() != 4 {
            return Err("indirizzamento a 16 bit".into());
        }
    }
    if ins.memory_base() != Register::None {
        parts.push(reg32(ins.memory_base())?.to_string());
    }
    if ins.memory_index() != Register::None {
        parts.push(format!("{} * {}", reg32(ins.memory_index())?, ins.memory_index_scale()));
    }
    let disp = ins.memory_displacement32();
    if disp != 0 || parts.is_empty() {
        parts.push(format!("{disp:#x}u"));
    }
    match ins.memory_segment() {
        Register::FS => parts.push("c->fs_base".into()),
        Register::GS => return Err("segmento gs".into()),
        _ => {}
    }
    Ok(format!("(u32)({})", parts.join(" + ")))
}

fn bits(ins: &Instruction, k: u32) -> R<u32> {
    let n = match ins.op_kind(k) {
        OpKind::Register => ins.op_register(k).size() * 8,
        OpKind::Memory => ins.memory_size().size() * 8,
        _ => 0,
    };
    match n {
        8 | 16 | 32 => Ok(n as u32),
        _ => Err(format!("operando di {n} bit")),
    }
}

fn read(ins: &Instruction, k: u32, bits: u32) -> R<String> {
    Ok(match ins.op_kind(k) {
        OpKind::Register => rd_reg(ins.op_register(k))?,
        OpKind::Memory => format!("rd{bits}(c, {})", ea(ins)?),
        OpKind::Immediate8 | OpKind::Immediate16 | OpKind::Immediate32 | OpKind::Immediate8to16 | OpKind::Immediate8to32 => {
            let mask = if bits == 32 { u32::MAX } else { (1u32 << bits) - 1 };
            format!("(u{bits}){:#x}u", ins.immediate(k) as u32 & mask)
        }
        other => return Err(format!("operando {other:?}")),
    })
}

fn write(ins: &Instruction, k: u32, bits: u32, v: &str) -> R<String> {
    match ins.op_kind(k) {
        OpKind::Register => wr_reg(ins.op_register(k), v),
        OpKind::Memory => Ok(format!("wr{bits}(c, {}, {v});", ea(ins)?)),
        other => Err(format!("destinazione {other:?}")),
    }
}

fn cond(cc: ConditionCode) -> R<&'static str> {
    Ok(match cc {
        ConditionCode::o => "of",
        ConditionCode::no => "!of",
        ConditionCode::b => "cf",
        ConditionCode::ae => "!cf",
        ConditionCode::e => "zf",
        ConditionCode::ne => "!zf",
        ConditionCode::be => "(cf || zf)",
        ConditionCode::a => "(!cf && !zf)",
        ConditionCode::s => "sf",
        ConditionCode::ns => "!sf",
        ConditionCode::p => "pf",
        ConditionCode::np => "!pf",
        ConditionCode::l => "(sf != of)",
        ConditionCode::ge => "(sf == of)",
        ConditionCode::le => "(zf || sf != of)",
        ConditionCode::g => "(!zf && sf == of)",
        ConditionCode::None => return Err("condizione".into()),
    })
}

pub struct Emitter<'a> {
    pub f: &'a Func,
    /// La funzione di destinazione è definita nello stesso output?
    pub known: &'a dyn Fn(u32) -> bool,
}

impl Emitter<'_> {
    fn call_fn(&self, t: u32) -> String {
        if (self.known)(t) {
            format!("{}(c);", fn_name(t))
        } else {
            format!("altair_call_indirect(c, {t:#x}u);")
        }
    }

    /// Trasferimento del controllo a `t`: `goto` se è dentro la funzione, altrimenti tail call.
    fn go(&self, t: u32) -> String {
        if self.f.insns.contains_key(&t) {
            format!("goto L_{t:08x};")
        } else {
            format!("{{ {} return; }}", self.call_fn(t))
        }
    }

    pub fn emit(&self) -> R<String> {
        let f = self.f;
        let mut out = String::new();
        let mut fmt = IntelFormatter::new();
        let mut text = String::new();
        writeln!(out, "void {}(Ctx *ALTAIR_RESTRICT c) {{", fn_name(f.start)).unwrap();
        out.push_str("    u32 cf = 0, zf = 0, sf = 0, of = 0, pf = 0, af = 0;\n");
        let keys: Vec<u32> = f.insns.keys().copied().collect();
        if keys.first() != Some(&f.start) {
            // Codice sotto l'entry: il corpo è emesso in ordine di indirizzo, quindi si salta.
            writeln!(out, "    goto L_{:08x};", f.start).unwrap();
        }
        for (i, addr) in keys.iter().enumerate() {
            let ins = &f.insns[addr];
            text.clear();
            fmt.format(ins, &mut text);
            if f.labels.contains(addr) {
                writeln!(out, "L_{addr:08x}:;").unwrap();
            }
            let body = self.insn(ins).map_err(|e| format!("{e} ({text} a {addr:#x})"))?;
            writeln!(out, "    /* {addr:08x}  {text} */").unwrap();
            if !body.is_empty() {
                writeln!(out, "    {body}").unwrap();
            }
            if falls_through(ins) {
                let next = ins.next_ip() as u32;
                if keys.get(i + 1) != Some(&next) {
                    writeln!(out, "    {}", self.go(next)).unwrap();
                }
            }
        }
        out.push_str("}\n");
        Ok(out)
    }

    fn insn(&self, ins: &Instruction) -> R<String> {
        use Mnemonic::*;
        let m = ins.mnemonic();
        let addr = ins.ip() as u32;
        let next = ins.next_ip() as u32;
        let has_mem = (0..ins.op_count()).any(|k| ins.op_kind(k) == OpKind::Memory);
        if ins.has_lock_prefix() || (m == Xchg && has_mem) {
            // `xchg` con un operando in memoria è atomico anche senza prefisso.
            return self.atomic(ins);
        }
        // Operazioni binarie con flag: (corpo che calcola r da a e b, scrive il risultato?)
        let binary = |calc: &str, store: bool| -> R<String> {
            let n = bits(ins, 0)?;
            let (a, b) = (read(ins, 0, n)?, read(ins, 1, n)?);
            let w = if store { write(ins, 0, n, "r")? } else { String::new() };
            let calc = calc.replace("BITS", &n.to_string()).replace("MSB", &(n - 1).to_string());
            Ok(format!("{{ u{n} a = {a}, b = {b}, r; {calc} {w} }}"))
        };
        let unary = |calc: &str| -> R<String> {
            let n = bits(ins, 0)?;
            let a = read(ins, 0, n)?;
            let w = write(ins, 0, n, "r")?;
            let calc = calc.replace("BITS", &n.to_string()).replace("MSB", &(n - 1).to_string());
            Ok(format!("{{ u{n} a = {a}, r; {calc} {w} }}"))
        };
        let shift = |calc: &str| -> R<String> {
            let n = bits(ins, 0)?;
            let a = read(ins, 0, n)?;
            let cnt = read(ins, 1, 8)?;
            let w = write(ins, 0, n, "r")?;
            let calc = calc.replace("BITS", &n.to_string()).replace("MSB", &(n - 1).to_string()).replace("ST", &format!("s{n}"));
            Ok(format!("{{ u{n} a = {a}, r; u32 n = {cnt} & 31; if (n) {{ {calc} {w} }} }}"))
        };

        match ins.flow_control() {
            FlowControl::ConditionalBranch => {
                let t = ins.near_branch_target() as u32;
                let test = match m {
                    Jecxz => "c->ecx == 0".to_string(),
                    Loop => "--c->ecx != 0".to_string(),
                    Jcxz | Loope | Loopne => return Err(format!("{m:?}")),
                    _ => cond(ins.condition_code())?.to_string(),
                };
                return Ok(format!("if ({test}) {}", self.go(t)));
            }
            FlowControl::UnconditionalBranch => return Ok(self.go(ins.near_branch_target() as u32)),
            FlowControl::IndirectBranch => {
                let target = read(ins, 0, 32)?;
                return Ok(match self.f.tables.get(&addr) {
                    Some(entries) => {
                        let uniq: BTreeSet<u32> = entries.iter().copied().collect();
                        let cases: String = uniq.iter().map(|t| format!("case {t:#x}u: goto L_{t:08x}; ")).collect();
                        format!("{{ u32 t = {target}; switch (t) {{ {cases}default: altair_trap(c, \"jump table\", t); }} }}")
                    }
                    None => format!("{{ u32 t = {target}; altair_call_indirect(c, t); return; }}"),
                });
            }
            FlowControl::Call => {
                let t = ins.near_branch_target() as u32;
                return Ok(format!("c->esp -= 4; wr32(c, c->esp, {next:#x}u); {}", self.call_fn(t)));
            }
            FlowControl::IndirectCall => {
                let target = read(ins, 0, 32)?;
                return Ok(format!(
                    "{{ u32 t = {target}; c->esp -= 4; wr32(c, c->esp, {next:#x}u); altair_call_indirect(c, t); }}"
                ));
            }
            FlowControl::Return => {
                if m != Ret {
                    return Err(format!("{m:?}"));
                }
                let extra = if ins.op_count() == 1 { ins.immediate16() as u32 } else { 0 };
                return Ok(format!("c->esp += {}; return;", 4 + extra));
            }
            FlowControl::Interrupt | FlowControl::Exception => {
                return Ok(format!("altair_trap(c, \"{m:?}\", {addr:#x}u);"));
            }
            FlowControl::XbeginXabortXend => return Err("xbegin".into()),
            FlowControl::Next => {}
        }

        if format!("{m:?}").starts_with('F') {
            return self.x87(ins);
        }

        // setcc / cmovcc
        if ins.condition_code() != ConditionCode::None {
            let test = cond(ins.condition_code())?;
            return match ins.op_count() {
                1 => write(ins, 0, 8, &format!("({test}) ? 1 : 0")),
                2 => {
                    let n = bits(ins, 0)?;
                    Ok(format!("if ({test}) {{ {} }}", write(ins, 0, n, &read(ins, 1, n)?)?))
                }
                _ => Err(format!("{m:?}")),
            };
        }

        // Istruzioni di stringa: l'operando di destinazione o sorgente è implicito.
        let string_op = matches!(
            ins.op0_kind(),
            OpKind::MemoryESEDI | OpKind::MemorySegESI | OpKind::MemorySegEDI
        ) || matches!(ins.op1_kind(), OpKind::MemoryESEDI | OpKind::MemorySegESI);
        if string_op {
            return self.string(ins);
        }

        match m {
            Nop | Pause | Wait | Prefetchnta | Prefetcht0 | Prefetcht1 | Prefetcht2 => Ok(String::new()),
            Mov if ins.op1_kind() == OpKind::Register && ins.op1_register().is_segment_register() => {
                // Selettori di un processo a 32 bit su Windows a 64 bit. Il CRT li legge solo per
                // riempire il contesto di un'eccezione.
                let sel = match ins.op1_register() {
                    Register::CS => 0x23,
                    Register::FS => 0x53,
                    _ => 0x2b,
                };
                if ins.op0_kind() == OpKind::Memory {
                    Ok(format!("wr16(c, {}, {sel:#x});", ea(ins)?))
                } else {
                    Ok(format!("{} = {sel:#x}u;", reg32(ins.op0_register())?))
                }
            }
            Xlatb => Ok("SET8L(c->eax, rd8(c, c->ebx + (u8)c->eax));".into()),
            Mov => {
                let n = bits(ins, 0)?;
                write(ins, 0, n, &read(ins, 1, n)?)
            }
            Movzx => {
                let (n, s) = (bits(ins, 0)?, bits(ins, 1)?);
                write(ins, 0, n, &format!("(u{n}){}", read(ins, 1, s)?))
            }
            Movsx => {
                let (n, s) = (bits(ins, 0)?, bits(ins, 1)?);
                write(ins, 0, n, &format!("(u{n})(s{n})(s{s}){}", read(ins, 1, s)?))
            }
            Lea => {
                if ins.memory_segment() == Register::FS {
                    return Err("lea con fs".into());
                }
                let n = bits(ins, 0)?;
                write(ins, 0, n, &ea(ins)?)
            }
            Push => {
                let v = match ins.op0_kind() {
                    OpKind::Register if ins.op0_register().size() != 4 => return Err("push a 16 bit".into()),
                    OpKind::Register | OpKind::Memory => read(ins, 0, 32)?,
                    _ => format!("{:#x}u", ins.immediate(0) as u32),
                };
                Ok(format!("{{ u32 v = {v}; c->esp -= 4; wr32(c, c->esp, v); }}"))
            }
            Pop => {
                if bits(ins, 0)? != 32 {
                    return Err("pop a 16 bit".into());
                }
                Ok(format!("{{ u32 v = rd32(c, c->esp); c->esp += 4; {} }}", write(ins, 0, 32, "v")?))
            }
            Leave => Ok("c->esp = c->ebp; c->ebp = rd32(c, c->esp); c->esp += 4;".into()),
            Pushad => Ok("{ u32 sp = c->esp; wr32(c, sp - 4, c->eax); wr32(c, sp - 8, c->ecx); wr32(c, sp - 12, c->edx); \
                          wr32(c, sp - 16, c->ebx); wr32(c, sp - 20, sp); wr32(c, sp - 24, c->ebp); wr32(c, sp - 28, c->esi); \
                          wr32(c, sp - 32, c->edi); c->esp = sp - 32; }".into()),
            Popad => Ok("{ u32 sp = c->esp; c->edi = rd32(c, sp); c->esi = rd32(c, sp + 4); c->ebp = rd32(c, sp + 8); \
                         c->ebx = rd32(c, sp + 16); c->edx = rd32(c, sp + 20); c->ecx = rd32(c, sp + 24); \
                         c->eax = rd32(c, sp + 28); c->esp = sp + 32; }".into()),
            Add => binary("r = (u(BITS))(a + b); F_ADD(a, b, r, BITS)", true),
            Sub => binary("r = (u(BITS))(a - b); F_SUB(a, b, r, BITS)", true),
            Cmp => binary("r = (u(BITS))(a - b); F_SUB(a, b, r, BITS)", false),
            And => binary("r = a & b; F_LOGIC(r, BITS)", true),
            Test => binary("r = a & b; F_LOGIC(r, BITS)", false),
            Or => binary("r = a | b; F_LOGIC(r, BITS)", true),
            Xor => binary("r = a ^ b; F_LOGIC(r, BITS)", true),
            Adc => binary(
                "{ u64 w = (u64)a + b + cf; r = (u(BITS))w; cf = (u32)(w >> BITS) & 1; \
                 of = (((a ^ r) & (b ^ r)) >> MSB) & 1; af = ((a ^ b ^ r) >> 4) & 1; F_ZSP(r, BITS) }",
                true,
            ),
            Sbb => binary(
                "{ u32 ci = cf; r = (u(BITS))(a - b - ci); cf = (u64)a < (u64)b + ci; \
                 of = (((a ^ b) & (a ^ r)) >> MSB) & 1; af = ((a ^ b ^ r) >> 4) & 1; F_ZSP(r, BITS) }",
                true,
            ),
            Inc => unary("r = (u(BITS))(a + 1); of = ((~a & r) >> MSB) & 1; af = ((a ^ r) >> 4) & 1; F_ZSP(r, BITS)"),
            Dec => unary("r = (u(BITS))(a - 1); of = ((a & ~r) >> MSB) & 1; af = ((a ^ r) >> 4) & 1; F_ZSP(r, BITS)"),
            Neg => unary(
                "r = (u(BITS))(0 - a); cf = a != 0; of = ((a & r) >> MSB) & 1; af = ((a ^ r) >> 4) & 1; F_ZSP(r, BITS)",
            ),
            Not => unary("r = (u(BITS))~a;"),
            Shl | Sal => shift(
                "u64 w = (u64)a << n; r = (u(BITS))w; cf = (u32)(w >> BITS) & 1; of = ((r >> MSB) & 1) ^ cf; F_ZSP(r, BITS)",
            ),
            Shr => shift(
                "cf = (u32)(((u64)a >> (n - 1)) & 1); r = (u(BITS))((u64)a >> n); of = (a >> MSB) & 1; F_ZSP(r, BITS)",
            ),
            Sar => shift(
                "s64 w = (ST)a; cf = (u32)((w >> (n - 1)) & 1); r = (u(BITS))(w >> n); of = 0; F_ZSP(r, BITS)",
            ),
            Rol => shift(
                "n %= BITS; r = n ? (u(BITS))((a << n) | (a >> (BITS - n))) : a; cf = r & 1; of = ((r >> MSB) & 1) ^ cf;",
            ),
            Ror => shift(
                "n %= BITS; r = n ? (u(BITS))((a >> n) | (a << (BITS - n))) : a; cf = (r >> MSB) & 1; \
                 of = cf ^ ((r >> (MSB - 1)) & 1);",
            ),
            Imul => match ins.op_count() {
                1 => self.muldiv(ins),
                k => {
                    let n = bits(ins, 0)?;
                    let (a, b) = (read(ins, k - 2, n)?, read(ins, k - 1, n)?);
                    Ok(format!(
                        "{{ s64 w = (s64)(s{n}){a} * (s64)(s{n}){b}; u{n} r = (u{n})w; cf = of = (w != (s64)(s{n})r); \
                         F_ZSP(r, {n}) {} }}",
                        write(ins, 0, n, "r")?
                    ))
                }
            },
            Mul | Div | Idiv => self.muldiv(ins),
            Xadd => {
                let n = bits(ins, 0)?;
                Ok(format!(
                    "{{ u{n} a = {}, b = {}, r = (u{n})(a + b); F_ADD(a, b, r, {n}) {} {} }}",
                    read(ins, 0, n)?,
                    read(ins, 1, n)?,
                    write(ins, 1, n, "a")?,
                    write(ins, 0, n, "r")?
                ))
            }
            Cmpxchg => {
                let n = bits(ins, 0)?;
                let acc = acc_reg(n);
                Ok(format!(
                    "{{ u{n} a = {}, b = {}, r = (u{n})(a - b); F_SUB(a, b, r, {n}) if (zf) {{ {} }} else {{ {} }} }}",
                    rd_reg(acc)?,
                    read(ins, 0, n)?,
                    write(ins, 0, n, &read(ins, 1, n)?)?,
                    wr_reg(acc, "b")?
                ))
            }
            Shld | Shrd => {
                if bits(ins, 0)? != 32 {
                    return Err("shld/shrd a 16 bit".into());
                }
                let calc = if m == Shld {
                    "r = (a << n) | (b >> (32 - n)); cf = (a >> (32 - n)) & 1;"
                } else {
                    "r = (a >> n) | (b << (32 - n)); cf = (a >> (n - 1)) & 1;"
                };
                Ok(format!(
                    "{{ u32 a = {}, b = {}, r; u32 n = {} & 31; if (n) {{ {calc} of = ((a ^ r) >> 31) & 1; F_ZSP(r, 32) {} }} }}",
                    read(ins, 0, 32)?,
                    read(ins, 1, 32)?,
                    read(ins, 2, 8)?,
                    write(ins, 0, 32, "r")?
                ))
            }
            Rcl | Rcr => {
                // Rotazione attraverso il carry: su bits + 1 bit.
                let n = bits(ins, 0)?;
                let calc = if m == Rcl {
                    format!("w = ((w << k) | (w >> ({n} + 1 - k))) & mask; r = (u{n})w; cf = (u32)(w >> {n}) & 1; of = ((r >> {}) & 1) ^ cf;", n - 1)
                } else {
                    format!("w = ((w >> k) | (w << ({n} + 1 - k))) & mask; r = (u{n})w; cf = (u32)(w >> {n}) & 1; of = ((r >> {}) ^ (r >> {})) & 1;", n - 1, n - 2)
                };
                Ok(format!(
                    "{{ u{n} a = {}, r; u32 k = ({} & 31) % ({n} + 1); if (k) {{ u64 mask = (1ull << ({n} + 1)) - 1, w = ((u64)cf << {n}) | a; {calc} {} }} }}",
                    read(ins, 0, n)?,
                    read(ins, 1, 8)?,
                    write(ins, 0, n, "r")?
                ))
            }
            Bt | Bts | Btr | Btc if ins.op0_kind() == OpKind::Memory && ins.op1_kind() == OpKind::Register => {
                // Stringa di bit: l'indice (con segno) sceglie anche la parola, oltre al bit.
                if bits(ins, 0)? != 32 {
                    return Err("bt su stringa di bit a 16 bit".into());
                }
                let upd = match m {
                    Bt => "",
                    Bts => " wr32(c, p, w | bit);",
                    Btr => " wr32(c, p, w & ~bit);",
                    _ => " wr32(c, p, w ^ bit);",
                };
                Ok(format!(
                    "{{ u32 i = {}, p = {} + (u32)(((s32)i >> 5) * 4), bit = 1u << (i & 31), w = rd32(c, p); cf = (w & bit) != 0;{upd} }}",
                    read(ins, 1, 32)?,
                    ea(ins)?
                ))
            }
            Bts | Btr | Btc => {
                let n = bits(ins, 0)?;
                let op = match m {
                    Bts => "a | bit",
                    Btr => "a & ~bit",
                    _ => "a ^ bit",
                };
                Ok(format!(
                    "{{ u{n} a = {}, bit = (u{n})(1u << ({} & {})); cf = (a & bit) != 0; {} }}",
                    read(ins, 0, n)?,
                    read(ins, 1, 8)?,
                    n - 1,
                    write(ins, 0, n, &format!("(u{n})({op})"))?
                ))
            }
            Bsf | Bsr => {
                let n = bits(ins, 0)?;
                let f = if m == Bsf { "(u32)__builtin_ctz(a)" } else { "31u - (u32)__builtin_clz(a)" };
                Ok(format!("{{ u32 a = {}; zf = a == 0; if (a) {{ {} }} }}", read(ins, 1, n)?, write(ins, 0, n, f)?))
            }
            Pushfd => Ok("{ u32 v = EFLAGS(); c->esp -= 4; wr32(c, c->esp, v); }".into()),
            Popfd => Ok("{ u32 v = rd32(c, c->esp); c->esp += 4; SET_EFLAGS(v) }".into()),
            Lahf => Ok("SET8H(c->eax, EFLAGS());".into()),
            Sahf => Ok("{ u32 v = (EFLAGS() & ~0xffu) | ((c->eax >> 8) & 0xff); SET_EFLAGS(v) }".into()),
            Rdtsc => Ok("{ u64 t = altair_rdtsc(c); c->eax = (u32)t; c->edx = (u32)(t >> 32); }".into()),
            Cpuid => Ok("altair_cpuid(c);".into()),
            Xchg => {
                let n = bits(ins, 0)?;
                Ok(format!(
                    "{{ u{n} a = {}, b = {}; {} {} }}",
                    read(ins, 0, n)?,
                    read(ins, 1, n)?,
                    write(ins, 0, n, "b")?,
                    write(ins, 1, n, "a")?
                ))
            }
            Cdq => Ok("c->edx = (s32)c->eax < 0 ? 0xffffffffu : 0;".into()),
            Cwde => Ok("c->eax = (u32)(s32)(s16)c->eax;".into()),
            Cbw => Ok("SET16(c->eax, (s16)(s8)c->eax);".into()),
            Cwd => Ok("SET16(c->edx, (s16)c->eax < 0 ? 0xffffu : 0);".into()),
            Bswap => {
                let r = reg32(ins.op0_register())?;
                Ok(format!("{r} = __builtin_bswap32({r});"))
            }
            Bt => {
                let n = bits(ins, 0)?;
                Ok(format!("cf = ({} >> ({} & {})) & 1;", read(ins, 0, n)?, read(ins, 1, 8)?, n - 1))
            }
            Cld => Ok("c->df = 0;".into()),
            Std => Ok("c->df = 1;".into()),
            Clc => Ok("cf = 0;".into()),
            Stc => Ok("cf = 1;".into()),
            Cmc => Ok("cf = !cf;".into()),
            _ => self.sse(ins),
        }
    }

    /// `mul`, `imul`, `div`, `idiv` a un operando: accumulatore implicito, 8, 16 o 32 bit.
    fn muldiv(&self, ins: &Instruction) -> R<String> {
        use Mnemonic::*;
        let n = bits(ins, 0)?;
        let v = read(ins, 0, n)?;
        let addr = ins.ip() as u32;
        // (parte bassa, parte alta) dell'accumulatore doppio: ah:al, dx:ax, edx:eax.
        let (lo, hi) = match n {
            8 => (Register::AL, Register::AH),
            16 => (Register::AX, Register::DX),
            _ => (Register::EAX, Register::EDX),
        };
        let wide = format!("(((u64){} << {n}) | {})", rd_reg(hi)?, rd_reg(lo)?);
        let store = |q: &str, r: &str| -> R<String> { Ok(format!("{} {}", wr_reg(lo, q)?, wr_reg(hi, r)?)) };
        Ok(match ins.mnemonic() {
            Mul => format!(
                "{{ u64 w = (u64){} * {v}; {} cf = of = ((w >> {n}) != 0); }}",
                rd_reg(lo)?,
                store("(u32)w", &format!("(u32)(w >> {n})"))?
            ),
            Imul => format!(
                "{{ s64 w = (s64)(s{n}){} * (s64)(s{n}){v}; {} cf = of = (w != (s64)(s{n})w); }}",
                rd_reg(lo)?,
                store("(u32)w", &format!("(u32)((u64)w >> {n})"))?
            ),
            Div => format!(
                "{{ u64 n = {wide}; u64 d = {v}; if (d == 0 || (n / d) >> {n}) altair_trap(c, \"div\", {addr:#x}u); \
                 u64 q = n / d, r = n % d; {} }}",
                store("(u32)q", "(u32)r")?
            ),
            Idiv => {
                let sx = 64 - 2 * n; // estende il segno del dividendo a 2n bit (per 32 bit non serve)
                format!(
                    "{{ s64 n = (s64)({wide} << {sx}) >> {sx}; s64 d = (s{n}){v}; \
                     if (d == 0 || (n == INT64_MIN && d == -1)) altair_trap(c, \"idiv\", {addr:#x}u); \
                     s64 q = n / d, r = n % d; if (q != (s64)(s{n})q) altair_trap(c, \"idiv\", {addr:#x}u); {} }}",
                    store("(u32)q", "(u32)r")?
                )
            }
            other => return Err(format!("{other:?}")),
        })
    }

    /// Istruzioni con prefisso `lock` (e `xchg` con memoria): atomiche vere sulla memoria host.
    fn atomic(&self, ins: &Instruction) -> R<String> {
        use Mnemonic::*;
        let m = ins.mnemonic();
        let mem = (0..ins.op_count()).find(|&k| ins.op_kind(k) == OpKind::Memory).ok_or("lock senza operando in memoria")?;
        let n = bits(ins, mem)?;
        let p = format!("MP(u{n}, {})", ea(ins)?);
        const SEQ: &str = "__ATOMIC_SEQ_CST";
        let msb = n - 1;
        Ok(match m {
            Add | Sub | And | Or | Xor => {
                let (f, calc) = match m {
                    Add => ("add", format!("r = (u{n})(a + b); F_ADD(a, b, r, {n})")),
                    Sub => ("sub", format!("r = (u{n})(a - b); F_SUB(a, b, r, {n})")),
                    And => ("and", format!("r = a & b; F_LOGIC(r, {n})")),
                    Or => ("or", format!("r = a | b; F_LOGIC(r, {n})")),
                    _ => ("xor", format!("r = a ^ b; F_LOGIC(r, {n})")),
                };
                format!("{{ u{n} b = {}, a = __atomic_fetch_{f}({p}, b, {SEQ}), r; {calc} }}", read(ins, 1, n)?)
            }
            Inc => format!(
                "{{ u{n} a = __atomic_fetch_add({p}, 1, {SEQ}), r = (u{n})(a + 1); of = ((~a & r) >> {msb}) & 1; \
                 af = ((a ^ r) >> 4) & 1; F_ZSP(r, {n}) }}"
            ),
            Dec => format!(
                "{{ u{n} a = __atomic_fetch_sub({p}, 1, {SEQ}), r = (u{n})(a - 1); of = ((a & ~r) >> {msb}) & 1; \
                 af = ((a ^ r) >> 4) & 1; F_ZSP(r, {n}) }}"
            ),
            Not => format!("__atomic_fetch_xor({p}, (u{n})~0u, {SEQ});"),
            Xadd => format!(
                "{{ u{n} b = {}, a = __atomic_fetch_add({p}, b, {SEQ}), r = (u{n})(a + b); F_ADD(a, b, r, {n}) {} }}",
                read(ins, 1, n)?,
                write(ins, 1, n, "a")?
            ),
            Cmpxchg => {
                let acc = acc_reg(n);
                format!(
                    "{{ u{n} a = {}, b = a, r; __atomic_compare_exchange_n({p}, &b, {}, 0, {SEQ}, {SEQ}); \
                     r = (u{n})(a - b); F_SUB(a, b, r, {n}) if (!zf) {{ {} }} }}",
                    rd_reg(acc)?,
                    read(ins, 1, n)?,
                    wr_reg(acc, "b")?
                )
            }
            Xchg => {
                let reg = 1 - mem;
                format!(
                    "{{ u{n} v = __atomic_exchange_n({p}, {}, {SEQ}); {} }}",
                    read(ins, reg, n)?,
                    write(ins, reg, n, "v")?
                )
            }
            Bts | Btr | Btc => {
                if ins.op1_kind() == OpKind::Register {
                    return Err("lock bt su stringa di bit".into());
                }
                let f = match m {
                    Bts => "__atomic_fetch_or(P, bit, S)",
                    Btr => "__atomic_fetch_and(P, (T)~bit, S)",
                    _ => "__atomic_fetch_xor(P, bit, S)",
                }
                .replace('P', &p)
                .replace('S', SEQ)
                .replace('T', &format!("u{n}"));
                format!(
                    "{{ u{n} bit = (u{n})(1u << ({} & {})); u{n} a = {f}; cf = (a & bit) != 0; }}",
                    read(ins, 1, 8)?,
                    n - 1
                )
            }
            other => return Err(format!("lock {other:?}")),
        })
    }

    /// x87: stack di registri a 80 bit. Le operazioni sono in `runtime/altair_x87.h`.
    fn x87(&self, ins: &Instruction) -> R<String> {
        use Mnemonic::*;
        let m = ins.mnemonic();
        let mem = ins.op_count() > 0 && ins.op0_kind() == OpKind::Memory;
        let msize = if mem { ins.memory_size().size() } else { 0 };
        let st = |k: u32| -> R<String> {
            let (r, base) = (ins.op_register(k) as u32, Register::ST0 as u32);
            if ins.op_kind(k) == OpKind::Register && (base..base + 8).contains(&r) {
                Ok(format!("ST({})", r - base))
            } else {
                Err(format!("{m:?}"))
            }
        };
        let fload = || -> R<String> {
            let a = ea(ins)?;
            Ok(match msize {
                4 => format!("(f80)rdf32(c, {a})"),
                8 => format!("(f80)rdf64(c, {a})"),
                10 => format!("rdf80(c, {a})"),
                _ => return Err(format!("{m:?} con operando di {msize} byte")),
            })
        };
        let iload = || -> R<String> {
            let a = ea(ins)?;
            Ok(match msize {
                2 => format!("(f80)(s16)rd16(c, {a})"),
                4 => format!("(f80)(s32)rd32(c, {a})"),
                8 => format!("(f80)(s64)rd64(c, {a})"),
                _ => return Err(format!("{m:?} con operando di {msize} byte")),
            })
        };
        let push = |v: &str| Ok(format!("fpush(c, {v});"));

        // Aritmetica: (espressione su destinazione d e sorgente s, intera?, pop?)
        let arith = |expr: &str, int: bool, pop: bool| -> R<String> {
            let (d, s) = if mem {
                ("ST(0)".to_string(), if int { iload()? } else { fload()? })
            } else {
                match ins.op_count() {
                    2 => (st(0)?, st(1)?),
                    1 => ("ST(0)".to_string(), st(0)?),
                    _ => ("ST(1)".to_string(), "ST(0)".to_string()),
                }
            };
            Ok(format!("{{ f80 s = {s}, d = {d}; {d} = {expr};{} }}", if pop { " fpop(c);" } else { "" }))
        };
        // Confronti con ST0: (scrive i flag della CPU?, intero?, numero di pop)
        let compare = |cpu: bool, int: bool, pops: usize| -> R<String> {
            let other = if mem {
                if int { iload()? } else { fload()? }
            } else if m == Ftst {
                "0.0L".to_string()
            } else {
                match ins.op_count() {
                    2 => st(1)?,
                    1 => st(0)?,
                    _ => "ST(1)".to_string(),
                }
            };
            Ok(format!(
                "{{ f80 a = ST(0), b = {other}; {}(a, b) }}{}",
                if cpu { "X_COMI" } else { "X87_COM" },
                " fpop(c);".repeat(pops)
            ))
        };
        let store_int = |trunc: bool, pop: bool| -> R<String> {
            let n = msize * 8;
            if !matches!(n, 16 | 32 | 64) {
                return Err(format!("{m:?} con operando di {msize} byte"));
            }
            Ok(format!(
                "wr{n}(c, {}, (u{n})x87_toi(c, ST(0), {n}, {}));{}",
                ea(ins)?,
                trunc as u32,
                if pop { " fpop(c);" } else { "" }
            ))
        };
        let unary = |f: &str| Ok(format!("ST(0) = {f}(ST(0));"));

        match m {
            Fld if mem => push(&fload()?),
            Fld => Ok(format!("{{ f80 v = {}; fpush(c, v); }}", st(0)?)),
            Fild => push(&iload()?),
            Fst | Fstp => {
                let pop = if m == Fstp { " fpop(c);" } else { "" };
                if mem {
                    let a = ea(ins)?;
                    Ok(match msize {
                        4 => format!("wrf32(c, {a}, (float)ST(0));{pop}"),
                        8 => format!("wrf64(c, {a}, (double)ST(0));{pop}"),
                        10 => format!("wrf80(c, {a}, ST(0));{pop}"),
                        _ => return Err(format!("{m:?} con operando di {msize} byte")),
                    })
                } else {
                    Ok(format!("{} = ST(0);{pop}", st(0)?))
                }
            }
            Fist => store_int(false, false),
            Fistp => store_int(false, true),
            Fisttp => store_int(true, true),
            Fadd => arith("d + s", false, false),
            Faddp => arith("d + s", false, true),
            Fiadd => arith("d + s", true, false),
            Fsub => arith("d - s", false, false),
            Fsubp => arith("d - s", false, true),
            Fisub => arith("d - s", true, false),
            Fsubr => arith("s - d", false, false),
            Fsubrp => arith("s - d", false, true),
            Fisubr => arith("s - d", true, false),
            Fmul => arith("d * s", false, false),
            Fmulp => arith("d * s", false, true),
            Fimul => arith("d * s", true, false),
            Fdiv => arith("d / s", false, false),
            Fdivp => arith("d / s", false, true),
            Fidiv => arith("d / s", true, false),
            Fdivr => arith("s / d", false, false),
            Fdivrp => arith("s / d", false, true),
            Fidivr => arith("s / d", true, false),
            Fcom | Fucom | Ftst => compare(false, false, 0),
            Fcomp | Fucomp => compare(false, false, 1),
            Fcompp | Fucompp => compare(false, false, 2),
            Ficom => compare(false, true, 0),
            Ficomp => compare(false, true, 1),
            Fcomi | Fucomi => compare(true, false, 0),
            Fcomip | Fucomip => compare(true, false, 1),
            Fxch => {
                let i = match ins.op_count() {
                    2 => st(1)?,
                    1 => st(0)?,
                    _ => "ST(1)".to_string(),
                };
                Ok(format!("{{ f80 t = ST(0); ST(0) = {i}; {i} = t; }}"))
            }
            Fcmovb | Fcmove | Fcmovbe | Fcmovu | Fcmovnb | Fcmovne | Fcmovnbe | Fcmovnu => {
                Ok(format!("if ({}) ST(0) = {};", cond(ins.condition_code())?, st(1)?))
            }
            Fchs => Ok("ST(0) = -ST(0);".into()),
            Fabs => unary("fabsl"),
            Fsqrt => unary("sqrtl"),
            Frndint => Ok("ST(0) = x87_rnd(c, ST(0));".into()),
            Fsin => unary("sinl"),
            Fcos => unary("cosl"),
            Fsincos => Ok("{ f80 v = ST(0); ST(0) = sinl(v); fpush(c, cosl(v)); c->fpsw &= ~0x400u; }".into()),
            Fptan => Ok("{ ST(0) = tanl(ST(0)); fpush(c, 1.0L); c->fpsw &= ~0x400u; }".into()),
            Fpatan => Ok("{ f80 x = ST(0); fpop(c); ST(0) = atan2l(ST(0), x); }".into()),
            Fyl2x => Ok("{ f80 x = ST(0); fpop(c); ST(0) = ST(0) * log2l(x); }".into()),
            Fyl2xp1 => Ok("{ f80 x = ST(0); fpop(c); ST(0) = ST(0) * (log1pl(x) / 0xb.17217f7d1cf79acp-4L); }".into()),
            F2xm1 => Ok("ST(0) = expm1l(ST(0) * 0xb.17217f7d1cf79acp-4L);".into()),
            Fscale => Ok("ST(0) = x87_scale(ST(0), ST(1));".into()),
            Fprem => Ok("x87_prem(c, 0);".into()),
            Fprem1 => Ok("x87_prem(c, 1);".into()),
            Fxam => Ok("x87_xam(c);".into()),
            Fld1 => push("1.0L"),
            Fldz => push("0.0L"),
            Fldpi => push("0xc.90fdaa22168c235p-2L"),
            Fldl2t => push("0xd.49a784bcd1b8afep-2L"),
            Fldl2e => push("0xb.8aa3b295c17f0bcp-3L"),
            Fldlg2 => push("0x9.a209a84fbcff799p-5L"),
            Fldln2 => push("0xb.17217f7d1cf79acp-4L"),
            Fnstsw => {
                if mem {
                    Ok(format!("wr16(c, {}, (u16)X87_SW());", ea(ins)?))
                } else {
                    Ok("SET16(c->eax, X87_SW());".into())
                }
            }
            Fnstcw => Ok(format!("wr16(c, {}, (u16)c->fpcw);", ea(ins)?)),
            Fldcw => Ok(format!("altair_set_fpcw(c, rd16(c, {}));", ea(ins)?)),
            Fninit => Ok("{ c->fptop = 0; c->fpsw = 0; altair_set_fpcw(c, 0x37f); }".into()),
            Fnclex | Ffree | Fnop => Ok(String::new()),
            Fnstenv => Ok(format!("x87_env_store(c, {});", ea(ins)?)),
            Fldenv => Ok(format!("x87_env_load(c, {});", ea(ins)?)),
            Fnsave => Ok(format!("x87_save(c, {});", ea(ins)?)),
            Frstor => Ok(format!("x87_restore(c, {});", ea(ins)?)),
            Fincstp => Ok("fpop(c);".into()),
            Fdecstp => Ok("c->fptop = (c->fptop - 1) & 7;".into()),
            other => Err(format!("{other:?}")),
        }
    }

    /// MMX: registri MM come mantisse dei registri x87. Riusa le operazioni a corsie di SSE
    /// sulla metà bassa di un valore a 128 bit.
    fn mmx(&self, ins: &Instruction) -> R<String> {
        use Mnemonic::*;
        let m = ins.mnemonic();
        let kind = |k: u32| ins.op_kind(k);
        let is_mm = |k: u32| kind(k) == OpKind::Register && ins.op_register(k).is_mm();
        let mm = |k: u32| -> R<u32> {
            if is_mm(k) {
                Ok(ins.op_register(k) as u32 - Register::MM0 as u32)
            } else {
                Err(format!("{m:?}"))
            }
        };
        // Sorgente a 64 bit: registro MM o memoria.
        let src = |k: u32| -> R<String> {
            if kind(k) == OpKind::Memory {
                Ok(format!("rd64(c, {})", ea(ins)?))
            } else {
                Ok(format!("mmr(c, {})", mm(k)?))
            }
        };
        let imm = |k: u32| ins.immediate(k) as u32 & 0xff;
        let bin = |f: &str| -> R<String> {
            let d = mm(0)?;
            Ok(format!("mmw(c, {d}, x_{f}(xq(mmr(c, {d})), xq({})).q[0]);", src(1)?))
        };
        // Le varianti "alte" lavorano sulla metà alta dei 64 bit: si scorre e si usa quella bassa.
        let bin_high = |f: &str| -> R<String> {
            let d = mm(0)?;
            Ok(format!("mmw(c, {d}, x_{f}(xq(mmr(c, {d}) >> 32), xq({} >> 32)).q[0]);", src(1)?))
        };
        let pack = |f: &str| -> R<String> {
            let d = mm(0)?;
            Ok(format!("mmw(c, {d}, m_{f}(mmr(c, {d}), {}));", src(1)?))
        };
        let shift = |f: &str| -> R<String> {
            let d = mm(0)?;
            let n = if kind(1) == OpKind::Immediate8 { format!("{}", imm(1)) } else { src(1)? };
            Ok(format!("mmw(c, {d}, x_{f}(xq(mmr(c, {d})), {n}).q[0]);"))
        };
        match m {
            Emms => Ok("c->fptop = 0;".into()),
            Movq => {
                if is_mm(0) {
                    Ok(format!("mmw(c, {}, {});", mm(0)?, src(1)?))
                } else {
                    Ok(format!("wr64(c, {}, mmr(c, {}));", ea(ins)?, mm(1)?))
                }
            }
            Movntq => Ok(format!("wr64(c, {}, mmr(c, {}));", ea(ins)?, mm(1)?)),
            Movd => {
                if is_mm(0) {
                    Ok(format!("mmw(c, {}, (u64){});", mm(0)?, read(ins, 1, 32)?))
                } else {
                    write(ins, 0, 32, &format!("(u32)mmr(c, {})", mm(1)?))
                }
            }
            Pand => bin("and"),
            Pandn => bin("andn"),
            Por => bin("or"),
            Pxor => bin("xor"),
            Paddb => bin("paddb"),
            Paddw => bin("paddw"),
            Paddd => bin("paddd"),
            Paddq => bin("paddq"),
            Psubb => bin("psubb"),
            Psubw => bin("psubw"),
            Psubd => bin("psubd"),
            Psubq => bin("psubq"),
            Paddsw => bin("paddsw"),
            Psubsw => bin("psubsw"),
            Paddusw => bin("paddusw"),
            Psubusw => bin("psubusw"),
            Pmaxsw => bin("pmaxsw"),
            Pminsw => bin("pminsw"),
            Pmaddwd => bin("pmaddwd"),
            Pmullw => bin("pmullw"),
            Pmulhw => bin("pmulhw"),
            Pcmpeqb => bin("pcmpeqb"),
            Pcmpeqw => bin("pcmpeqw"),
            Pcmpeqd => bin("pcmpeqd"),
            Pcmpgtw => bin("pcmpgtw"),
            Pcmpgtd => bin("pcmpgtd"),
            Punpcklbw => bin("punpcklbw"),
            Punpcklwd => bin("punpcklwd"),
            Punpckldq => bin("unpcklps"),
            Punpckhbw => bin_high("punpcklbw"),
            Punpckhwd => bin_high("punpcklwd"),
            Punpckhdq => bin_high("unpcklps"),
            Packssdw => pack("packssdw"),
            Packsswb => pack("packsswb"),
            Packuswb => pack("packuswb"),
            Psllw => shift("psllw"),
            Psrlw => shift("psrlw"),
            Psraw => shift("psraw"),
            Pslld => shift("pslld"),
            Psrld => shift("psrld"),
            Psrad => shift("psrad"),
            Psllq => shift("psllq"),
            Psrlq => shift("psrlq"),
            Pshufw => Ok(format!("mmw(c, {}, m_pshufw({}, {}));", mm(0)?, src(1)?, imm(2))),
            Cvttps2pi | Cvtps2pi => {
                let s = if kind(1) == OpKind::Memory {
                    format!("xq(rd64(c, {}))", ea(ins)?)
                } else {
                    format!("c->xmm[{}]", ins.op1_register().number())
                };
                let conv = if m == Cvttps2pi { "x_f2i(s.f[I])" } else { "x_f2i(rintf(s.f[I]))" };
                Ok(format!(
                    "{{ Xmm s = {s}; mmw(c, {}, (u64){} | ((u64){} << 32)); }}",
                    mm(0)?,
                    conv.replace('I', "0"),
                    conv.replace('I', "1")
                ))
            }
            Cvtpi2ps => Ok(format!(
                "{{ u64 s = {}; c->xmm[{d}].f[0] = (float)(s32)s; c->xmm[{d}].f[1] = (float)(s32)(s >> 32); }}",
                src(1)?,
                d = ins.op0_register().number()
            )),
            other => Err(format!("MMX {other:?}")),
        }
    }

    /// SSE e SSE2 sui registri XMM. Le operazioni sono funzioni di `runtime/altair_sse.h`.
    fn sse(&self, ins: &Instruction) -> R<String> {
        use Mnemonic::*;
        let m = ins.mnemonic();
        let kind = |k: u32| ins.op_kind(k);
        if m == Emms || (0..ins.op_count()).any(|k| kind(k) == OpKind::Register && ins.op_register(k).is_mm()) {
            return self.mmx(ins);
        }
        let xmm = |k: u32| -> R<String> {
            let r = ins.op_register(k);
            if kind(k) == OpKind::Register && r.is_xmm() {
                Ok(format!("c->xmm[{}]", r.number()))
            } else {
                Err(format!("{m:?}"))
            }
        };
        // Sorgente a 128 bit, oppure i soli 32 o 64 bit bassi quando l'operando è più corto.
        let src128 = |k: u32| -> R<String> {
            if kind(k) == OpKind::Memory {
                Ok(format!("xld(c, {})", ea(ins)?))
            } else {
                xmm(k)
            }
        };
        let f32s = |k: u32| -> R<String> {
            if kind(k) == OpKind::Memory {
                Ok(format!("rdf32(c, {})", ea(ins)?))
            } else {
                Ok(format!("{}.f[0]", xmm(k)?))
            }
        };
        let f64s = |k: u32| -> R<String> {
            if kind(k) == OpKind::Memory {
                Ok(format!("rdf64(c, {})", ea(ins)?))
            } else {
                Ok(format!("{}.db[0]", xmm(k)?))
            }
        };
        let u32s = |k: u32| -> R<String> {
            if kind(k) == OpKind::Memory {
                Ok(format!("rd32(c, {})", ea(ins)?))
            } else {
                Ok(format!("{}.d[0]", xmm(k)?))
            }
        };
        let u64s = |k: u32| -> R<String> {
            if kind(k) == OpKind::Memory {
                Ok(format!("rd64(c, {})", ea(ins)?))
            } else {
                Ok(format!("{}.q[0]", xmm(k)?))
            }
        };
        let imm = |k: u32| ins.immediate(k) as u32 & 0xff;
        let packed2 = |f: &str| -> R<String> { Ok(format!("{d} = x_{f}({d}, {});", src128(1)?, d = xmm(0)?)) };
        let packed1 = |f: &str| -> R<String> { Ok(format!("{} = x_{f}({});", xmm(0)?, src128(1)?)) };
        let scalar32 = |expr: &str| -> R<String> {
            Ok(format!("{{ float x = {d}.f[0], y = {}; {d}.f[0] = {expr}; }}", f32s(1)?, d = xmm(0)?))
        };
        let scalar64 = |expr: &str| -> R<String> {
            Ok(format!("{{ double x = {d}.db[0], y = {}; {d}.db[0] = {expr}; }}", f64s(1)?, d = xmm(0)?))
        };
        let shift = |f: &str| -> R<String> {
            let n = match kind(1) {
                OpKind::Immediate8 => format!("{}", imm(1)),
                _ => format!("{}.q[0]", src128(1)?),
            };
            Ok(format!("{d} = x_{f}({d}, {n});", d = xmm(0)?))
        };

        match m {
            Movaps | Movups | Movapd | Movupd | Movdqa | Movdqu => {
                if kind(0) == OpKind::Memory {
                    Ok(format!("xst(c, {}, {});", ea(ins)?, xmm(1)?))
                } else {
                    Ok(format!("{} = {};", xmm(0)?, src128(1)?))
                }
            }
            Movss => match (kind(0), kind(1)) {
                (OpKind::Memory, _) => Ok(format!("wr32(c, {}, {}.d[0]);", ea(ins)?, xmm(1)?)),
                (_, OpKind::Memory) => Ok(format!("{} = xd({});", xmm(0)?, u32s(1)?)),
                _ => Ok(format!("{}.d[0] = {}.d[0];", xmm(0)?, xmm(1)?)),
            },
            Movsd => match (kind(0), kind(1)) {
                (OpKind::Memory, _) => Ok(format!("wr64(c, {}, {}.q[0]);", ea(ins)?, xmm(1)?)),
                (_, OpKind::Memory) => Ok(format!("{} = xq({});", xmm(0)?, u64s(1)?)),
                _ => Ok(format!("{}.q[0] = {}.q[0];", xmm(0)?, xmm(1)?)),
            },
            Movd => {
                if kind(0) == OpKind::Register && ins.op0_register().is_xmm() {
                    Ok(format!("{} = xd({});", xmm(0)?, read(ins, 1, 32)?))
                } else {
                    write(ins, 0, 32, &format!("{}.d[0]", xmm(1)?))
                }
            }
            Movq => {
                if kind(0) == OpKind::Memory {
                    Ok(format!("wr64(c, {}, {}.q[0]);", ea(ins)?, xmm(1)?))
                } else {
                    Ok(format!("{} = xq({});", xmm(0)?, u64s(1)?))
                }
            }
            Movlpd | Movlps | Movhpd | Movhps => {
                let lane = if matches!(m, Movlpd | Movlps) { 0 } else { 1 };
                if kind(0) == OpKind::Memory {
                    Ok(format!("wr64(c, {}, {}.q[{lane}]);", ea(ins)?, xmm(1)?))
                } else {
                    Ok(format!("{}.q[{lane}] = rd64(c, {});", xmm(0)?, ea(ins)?))
                }
            }
            Movhlps => packed2("movhlps"),
            Movlhps => packed2("movlhps"),
            Movmskps => write(ins, 0, 32, &format!("x_movmskps({})", xmm(1)?)),
            Movmskpd => write(ins, 0, 32, &format!("x_movmskpd({})", xmm(1)?)),
            Pmovmskb => write(ins, 0, 32, &format!("x_pmovmskb({})", xmm(1)?)),

            Addps => packed2("addps"),
            Subps => packed2("subps"),
            Mulps => packed2("mulps"),
            Divps => packed2("divps"),
            Minps => packed2("minps"),
            Maxps => packed2("maxps"),
            Addpd => packed2("addpd"),
            Subpd => packed2("subpd"),
            Mulpd => packed2("mulpd"),
            Divpd => packed2("divpd"),
            Minpd => packed2("minpd"),
            Maxpd => packed2("maxpd"),
            Andps | Andpd | Pand => packed2("and"),
            Andnps | Andnpd | Pandn => packed2("andn"),
            Orps | Orpd | Por => packed2("or"),
            Xorps | Xorpd | Pxor => packed2("xor"),
            Paddb => packed2("paddb"),
            Paddw => packed2("paddw"),
            Paddd => packed2("paddd"),
            Paddq => packed2("paddq"),
            Psubb => packed2("psubb"),
            Psubw => packed2("psubw"),
            Psubd => packed2("psubd"),
            Psubq => packed2("psubq"),
            Pcmpeqb => packed2("pcmpeqb"),
            Pcmpeqw => packed2("pcmpeqw"),
            Pcmpeqd => packed2("pcmpeqd"),
            Pcmpgtw => packed2("pcmpgtw"),
            Pcmpgtd => packed2("pcmpgtd"),
            Pmullw => packed2("pmullw"),
            Pmulhw => packed2("pmulhw"),
            Pmuludq => packed2("pmuludq"),
            Unpcklps => packed2("unpcklps"),
            Unpckhps => packed2("unpckhps"),
            Unpcklpd | Punpcklqdq => packed2("unpcklpd"),
            Unpckhpd | Punpckhqdq => packed2("unpckhpd"),
            Punpcklbw => packed2("punpcklbw"),
            Punpcklwd => packed2("punpcklwd"),
            Punpckldq => packed2("unpcklps"),
            Punpckhdq => packed2("unpckhps"),
            Sqrtps => packed1("sqrtps"),
            Rsqrtps => packed1("rsqrtps"),
            Rcpps => packed1("rcpps"),
            Cvtpd2ps => packed1("cvtpd2ps"),
            Cvtdq2ps => packed1("cvtdq2ps"),
            Cvtdq2pd => Ok(format!(
                "{} = x_cvtdq2pd({});",
                xmm(0)?,
                if kind(1) == OpKind::Memory { format!("xq(rd64(c, {}))", ea(ins)?) } else { xmm(1)? }
            )),
            // Con sorgente in memoria (con un registro MM passa da `mmx`).
            Cvtpi2ps => Ok(format!(
                "{{ u64 s = rd64(c, {}); {d}.f[0] = (float)(s32)s; {d}.f[1] = (float)(s32)(s >> 32); }}",
                ea(ins)?,
                d = xmm(0)?
            )),
            Movntps | Movntpd | Movntdq => Ok(format!("xst(c, {}, {});", ea(ins)?, xmm(1)?)),
            Paddsw => packed2("paddsw"),
            Psubsw => packed2("psubsw"),
            Paddusw => packed2("paddusw"),
            Psubusw => packed2("psubusw"),
            Pmaxsw => packed2("pmaxsw"),
            Pminsw => packed2("pminsw"),
            Pmaddwd => packed2("pmaddwd"),
            Cvttps2dq => packed1("cvttps2dq"),
            Cvtps2dq => packed1("cvtps2dq"),
            // L'operando in memoria di `cvtps2pd` è di soli 64 bit.
            Cvtps2pd => Ok(format!(
                "{} = x_cvtps2pd({});",
                xmm(0)?,
                if kind(1) == OpKind::Memory { format!("xq(rd64(c, {}))", ea(ins)?) } else { xmm(1)? }
            )),
            Shufps => Ok(format!("{d} = x_shufps({d}, {}, {});", src128(1)?, imm(2), d = xmm(0)?)),
            Shufpd => Ok(format!("{d} = x_shufpd({d}, {}, {});", src128(1)?, imm(2), d = xmm(0)?)),
            Pshufd => Ok(format!("{} = x_pshufd({}, {});", xmm(0)?, src128(1)?, imm(2))),
            Cmpps => Ok(format!("{d} = x_cmpps({d}, {}, {});", src128(1)?, imm(2), d = xmm(0)?)),
            Cmppd => Ok(format!("{d} = x_cmppd({d}, {}, {});", src128(1)?, imm(2), d = xmm(0)?)),
            Cmpss => Ok(format!("{d}.d[0] = x_cmpss({d}.f[0], {}, {});", f32s(1)?, imm(2), d = xmm(0)?)),
            Cmpsd => Ok(format!("{d}.q[0] = x_cmpsd({d}.db[0], {}, {});", f64s(1)?, imm(2), d = xmm(0)?)),

            Addss => scalar32("x + y"),
            Subss => scalar32("x - y"),
            Mulss => scalar32("x * y"),
            Divss => scalar32("x / y"),
            Minss => scalar32("x < y ? x : y"),
            Maxss => scalar32("x > y ? x : y"),
            Sqrtss => scalar32("sqrtf(y)"),
            Rsqrtss => scalar32("1.0f / sqrtf(y)"),
            Rcpss => scalar32("1.0f / y"),
            Addsd => scalar64("x + y"),
            Subsd => scalar64("x - y"),
            Mulsd => scalar64("x * y"),
            Divsd => scalar64("x / y"),
            Minsd => scalar64("x < y ? x : y"),
            Maxsd => scalar64("x > y ? x : y"),
            Sqrtsd => scalar64("sqrt(y)"),
            Comiss | Ucomiss => Ok(format!("{{ float a = {}.f[0], b = {}; X_COMI(a, b) }}", xmm(0)?, f32s(1)?)),
            Comisd | Ucomisd => Ok(format!("{{ double a = {}.db[0], b = {}; X_COMI(a, b) }}", xmm(0)?, f64s(1)?)),

            Cvtss2sd => Ok(format!("{}.db[0] = (double){};", xmm(0)?, f32s(1)?)),
            Cvtsd2ss => Ok(format!("{}.f[0] = (float){};", xmm(0)?, f64s(1)?)),
            Cvtsi2ss => Ok(format!("{}.f[0] = (float)(s32){};", xmm(0)?, read(ins, 1, 32)?)),
            Cvtsi2sd => Ok(format!("{}.db[0] = (double)(s32){};", xmm(0)?, read(ins, 1, 32)?)),
            Cvttss2si => write(ins, 0, 32, &format!("x_f2i({})", f32s(1)?)),
            Cvttsd2si => write(ins, 0, 32, &format!("x_f2i({})", f64s(1)?)),
            Cvtss2si => write(ins, 0, 32, &format!("x_f2i(rintf({}))", f32s(1)?)),
            Cvtsd2si => write(ins, 0, 32, &format!("x_f2i(rint({}))", f64s(1)?)),

            Pslldq => Ok(format!("{d} = x_pslldq({d}, {});", imm(1), d = xmm(0)?)),
            Psrldq => Ok(format!("{d} = x_psrldq({d}, {});", imm(1), d = xmm(0)?)),
            Psllq => shift("psllq"),
            Psrlq => shift("psrlq"),
            Pslld => shift("pslld"),
            Psrld => shift("psrld"),
            Psrad => shift("psrad"),
            Psllw => shift("psllw"),
            Psrlw => shift("psrlw"),
            Psraw => shift("psraw"),
            Pextrw => write(ins, 0, 32, &format!("(u32){}.w[{}]", xmm(1)?, imm(2) & 7)),
            Pinsrw => {
                let v = if kind(1) == OpKind::Memory { format!("rd16(c, {})", ea(ins)?) } else { read(ins, 1, 32)? };
                Ok(format!("{}.w[{}] = (u16){v};", xmm(0)?, imm(2) & 7))
            }
            Ldmxcsr => Ok(format!("altair_set_mxcsr(c, rd32(c, {}));", ea(ins)?)),
            Stmxcsr => Ok(format!("wr32(c, {}, c->mxcsr);", ea(ins)?)),
            other => Err(format!("{other:?}")),
        }
    }

    fn string(&self, ins: &Instruction) -> R<String> {
        use Mnemonic::*;
        let (kind, n) = match ins.mnemonic() {
            Movsb => ("movs", 8),
            Movsw => ("movs", 16),
            Movsd => ("movs", 32),
            Stosb => ("stos", 8),
            Stosw => ("stos", 16),
            Stosd => ("stos", 32),
            Lodsb => ("lods", 8),
            Lodsw => ("lods", 16),
            Lodsd => ("lods", 32),
            Scasb => ("scas", 8),
            Scasw => ("scas", 16),
            Scasd => ("scas", 32),
            Cmpsb => ("cmps", 8),
            Cmpsw => ("cmps", 16),
            Cmpsd => ("cmps", 32),
            other => return Err(format!("{other:?}")),
        };
        if ins.segment_prefix() == Register::FS || ins.segment_prefix() == Register::GS {
            return Err("istruzione di stringa con segmento".into());
        }
        let acc = match n {
            8 => Register::AL,
            16 => Register::AX,
            _ => Register::EAX,
        };
        let step = n / 8;
        let body = match kind {
            "movs" => format!("wr{n}(c, c->edi, rd{n}(c, c->esi)); c->esi += d; c->edi += d;"),
            "stos" => format!("wr{n}(c, c->edi, {}); c->edi += d;", rd_reg(acc)?),
            "lods" => format!("{} c->esi += d;", wr_reg(acc, &format!("rd{n}(c, c->esi)"))?),
            "scas" => format!(
                "{{ u{n} a = {}, b = rd{n}(c, c->edi), r = (u{n})(a - b); F_SUB(a, b, r, {n}) }} c->edi += d;",
                rd_reg(acc)?
            ),
            _ => format!(
                "{{ u{n} a = rd{n}(c, c->esi), b = rd{n}(c, c->edi), r = (u{n})(a - b); F_SUB(a, b, r, {n}) }} \
                 c->esi += d; c->edi += d;"
            ),
        };
        let compare = matches!(kind, "scas" | "cmps");
        let looped = if ins.has_repne_prefix() {
            if !compare {
                return Err("repne su istruzione non di confronto".into());
            }
            format!("while (c->ecx) {{ c->ecx--; {body} if (zf) break; }}")
        } else if ins.has_rep_prefix() || ins.has_repe_prefix() {
            if compare {
                format!("while (c->ecx) {{ c->ecx--; {body} if (!zf) break; }}")
            } else {
                format!("while (c->ecx) {{ {body} c->ecx--; }}")
            }
        } else {
            body
        };
        Ok(format!("{{ u32 d = c->df ? (u32)-{step} : {step}u; {looped} }}"))
    }
}

/// Il traduttore sa emettere questa istruzione, presa da sola?
pub fn supported(ins: &Instruction) -> bool {
    let f = Func { start: 0, insns: BTreeMap::new(), labels: BTreeSet::new(), tables: HashMap::new(), calls: BTreeSet::new() };
    Emitter { f: &f, known: &|_| true }.insn(ins).is_ok()
}

/// Traduce `f`; `known` dice quali funzioni saranno definite nello stesso file.
pub fn emit(f: &Func, known: &dyn Fn(u32) -> bool) -> R<String> {
    // `u(BITS)` nei modelli diventa il tipo senza segno della larghezza giusta.
    let c = Emitter { f, known }.emit()?;
    Ok(c.replace("u(8)", "u8").replace("u(16)", "u16").replace("u(32)", "u32"))
}
