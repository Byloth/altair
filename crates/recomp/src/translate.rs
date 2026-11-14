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
//! il nome dell'istruzione: `acrek coverage` le conta.

use crate::discover::table_bound;
use acrek_pe::Image;
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
            if addr != a && (f.insns.contains_key(&addr) || external(addr)) {
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
        2 => format!("{full} = ({full} & 0xffff0000u) | (u16)({v});"),
        1 if is_high(r) => format!("{full} = ({full} & 0xffff00ffu) | ((u32)(u8)({v}) << 8);"),
        1 => format!("{full} = ({full} & 0xffffff00u) | (u8)({v});"),
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
            format!("acrek_call_indirect(c, {t:#x}u);")
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
        writeln!(out, "void {}(Ctx *c) {{", fn_name(f.start)).unwrap();
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
        if ins.has_lock_prefix() {
            return Err("prefisso lock".into());
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
                        format!("{{ u32 t = {target}; switch (t) {{ {cases}default: acrek_trap(c, \"jump table\", t); }} }}")
                    }
                    None => format!("{{ u32 t = {target}; acrek_call_indirect(c, t); return; }}"),
                });
            }
            FlowControl::Call => {
                let t = ins.near_branch_target() as u32;
                return Ok(format!("c->esp -= 4; wr32(c, c->esp, {next:#x}u); {}", self.call_fn(t)));
            }
            FlowControl::IndirectCall => {
                let target = read(ins, 0, 32)?;
                return Ok(format!(
                    "{{ u32 t = {target}; c->esp -= 4; wr32(c, c->esp, {next:#x}u); acrek_call_indirect(c, t); }}"
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
                return Ok(format!("acrek_trap(c, \"{m:?}\", {addr:#x}u);"));
            }
            FlowControl::XbeginXabortXend => return Err("xbegin".into()),
            FlowControl::Next => {}
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
            Nop => Ok(String::new()),
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
            Shl => shift(
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
                1 => {
                    if bits(ins, 0)? != 32 {
                        return Err("imul a 8/16 bit".into());
                    }
                    Ok(format!(
                        "{{ s64 w = (s64)(s32)c->eax * (s64)(s32){}; c->eax = (u32)w; c->edx = (u32)(w >> 32); \
                         cf = of = (w != (s64)(s32)c->eax); }}",
                        read(ins, 0, 32)?
                    ))
                }
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
            Mul => {
                if bits(ins, 0)? != 32 {
                    return Err("mul a 8/16 bit".into());
                }
                Ok(format!(
                    "{{ u64 w = (u64)c->eax * {}; c->eax = (u32)w; c->edx = (u32)(w >> 32); cf = of = (c->edx != 0); }}",
                    read(ins, 0, 32)?
                ))
            }
            Div => {
                if bits(ins, 0)? != 32 {
                    return Err("div a 8/16 bit".into());
                }
                Ok(format!(
                    "{{ u64 n = ((u64)c->edx << 32) | c->eax; u32 d = {}; \
                     if (d == 0 || n / d > 0xffffffffu) acrek_trap(c, \"div\", {addr:#x}u); \
                     c->eax = (u32)(n / d); c->edx = (u32)(n % d); }}",
                    read(ins, 0, 32)?
                ))
            }
            Idiv => {
                if bits(ins, 0)? != 32 {
                    return Err("idiv a 8/16 bit".into());
                }
                Ok(format!(
                    "{{ s64 n = (s64)(((u64)c->edx << 32) | c->eax); s32 d = (s32){}; \
                     if (d == 0 || (n == INT64_MIN && d == -1)) acrek_trap(c, \"idiv\", {addr:#x}u); \
                     s64 q = n / d; if (q != (s64)(s32)q) acrek_trap(c, \"idiv\", {addr:#x}u); \
                     c->eax = (u32)q; c->edx = (u32)(n % d); }}",
                    read(ins, 0, 32)?
                ))
            }
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
            Cbw => Ok("c->eax = (c->eax & 0xffff0000u) | (u16)(s16)(s8)c->eax;".into()),
            Cwd => Ok("c->edx = (c->edx & 0xffff0000u) | ((s16)c->eax < 0 ? 0xffffu : 0);".into()),
            Bswap => {
                let r = reg32(ins.op0_register())?;
                Ok(format!("{r} = __builtin_bswap32({r});"))
            }
            Bt => {
                if ins.op0_kind() == OpKind::Memory && ins.op1_kind() == OpKind::Register {
                    return Err("bt su stringa di bit".into());
                }
                let n = bits(ins, 0)?;
                Ok(format!("cf = ({} >> ({} & {})) & 1;", read(ins, 0, n)?, read(ins, 1, 8)?, n - 1))
            }
            Cld => Ok("c->df = 0;".into()),
            Std => Ok("c->df = 1;".into()),
            Clc => Ok("cf = 0;".into()),
            Stc => Ok("cf = 1;".into()),
            Cmc => Ok("cf = !cf;".into()),
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

/// Traduce `f`; `known` dice quali funzioni saranno definite nello stesso file.
pub fn emit(f: &Func, known: &dyn Fn(u32) -> bool) -> R<String> {
    // `u(BITS)` nei modelli diventa il tipo senza segno della larghezza giusta.
    let c = Emitter { f, known }.emit()?;
    Ok(c.replace("u(8)", "u8").replace("u(16)", "u16").replace("u(32)", "u32"))
}
