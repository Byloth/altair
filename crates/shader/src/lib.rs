//! Traduzione degli shader Direct3D 9 (bytecode shader model 3.0) in WGSL.
//!
//! Convenzioni del WGSL prodotto, che il renderer deve rispettare:
//!  - costanti: `@group(0) @binding(0)` per il vertex shader, `@group(3) @binding(0)` per il
//!    pixel shader (gruppi separati: ciascuno si riaggancia solo quando cambiano le sue),
//!    entrambe `Consts { misc: vec4f, b: vec4u, i: array<vec4i, 16>, c: array<vec4f, 256> }`:
//!    le costanti float stanno in fondo, così il renderer può caricarne solo il tratto che
//!    lo shader usa (`Shader::float_consts`);
//!    `b.x` ha un bit per registro booleano; `misc.xy` è la correzione di mezzo pixel, in unità
//!    di clip, che il vertex shader somma alla posizione;
//!  - texture: `@group(1) @binding(2n)` la texture del campionatore `sN`, `@binding(2n+1)` il
//!    suo sampler; i campionatori del vertex shader stanno nel gruppo 2;
//!  - attributi dei vertici e variabili tra i due stadi: una `@location` fissa per ogni
//!    semantica (`attribute_location`, `varying_location`);
//!  - ogni registro è un `vec4f`; le maschere di scrittura diventano assegnazioni per
//!    componente, perché WGSL non permette `r0.xy = …`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Vertex,
    Pixel,
}

/// Semantiche di D3DDECLUSAGE.
pub const USAGE_NAMES: [&str; 14] =
    ["position", "blendweight", "blendindices", "normal", "psize", "texcoord", "tangent", "binormal", "tessfactor", "positiont", "color", "fog", "depth", "sample"];

/// `@location` di un attributo di vertice, per (usage, indice). wgpu ne offre 16.
pub fn attribute_location(usage: u32, index: u32) -> Option<u32> {
    Some(match (usage, index) {
        (0, 0) => 0,
        (1, 0) => 1,
        (2, 0) => 2,
        (3, 0) => 3,
        (6, 0) => 4,
        (7, 0) => 5,
        (10, i) if i < 2 => 6 + i,
        (5, i) if i < 8 => 8 + i,
        _ => return None,
    })
}

/// `@location` di una variabile passata dal vertex al pixel shader, per (usage, indice).
pub fn varying_location(usage: u32, index: u32) -> Option<u32> {
    Some(match (usage, index) {
        (5, i) if i < 10 => i,
        (10, i) if i < 2 => 10 + i,
        (3, 0) => 12,
        (6, 0) => 13,
        (7, 0) => 14,
        (11, 0) | (12, 0) => 15,
        (0, i) if (1..3).contains(&i) => 15 + i,
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
enum Reg {
    Temp,
    Input,
    Const,
    Addr, // a0 nel vertex shader
    RastOut,
    AttrOut,
    Output, // o# (vs_3_0)
    ConstInt,
    ColorOut,
    DepthOut,
    Sampler,
    ConstBool,
    Loop,
    Misc, // vPos, vFace
    Other(u32),
}

fn reg_type(tok: u32) -> Reg {
    match ((tok >> 28) & 7) | ((tok >> 8) & 0x18) {
        0 => Reg::Temp,
        1 => Reg::Input,
        2 => Reg::Const,
        3 => Reg::Addr,
        4 => Reg::RastOut,
        5 => Reg::AttrOut,
        6 => Reg::Output,
        7 => Reg::ConstInt,
        8 => Reg::ColorOut,
        9 => Reg::DepthOut,
        10 => Reg::Sampler,
        14 => Reg::ConstBool,
        15 => Reg::Loop,
        17 => Reg::Misc,
        n => Reg::Other(n),
    }
}

#[derive(Clone, Copy, Debug)]
struct Src {
    reg: Reg,
    num: u32,
    swizzle: [u8; 4],
    modifier: u32,
    /// Indirizzamento relativo: (registro, componente) dell'indice.
    rel: Option<(Reg, u8)>,
}

#[derive(Clone, Copy, Debug)]
struct Dst {
    reg: Reg,
    num: u32,
    mask: u32,
    saturate: bool,
}

#[derive(Debug)]
struct Ins {
    op: u32,
    control: u32,
    dst: Option<Dst>,
    src: Vec<Src>,
}

#[derive(Debug, Clone, Copy)]
pub struct Decl {
    pub usage: u32,
    pub index: u32,
    pub mask: u32,
    num: u32,
}

pub struct Shader {
    pub stage: Stage,
    pub version: (u32, u32),
    ins: Vec<Ins>,
    pub inputs: Vec<Decl>,
    pub outputs: Vec<Decl>,
    /// numero del campionatore -> tipo (2 = 2D, 3 = cubo, 4 = volume)
    pub samplers: BTreeMap<u32, u32>,
    defs: BTreeMap<u32, [f32; 4]>,
    defi: BTreeMap<u32, [i32; 4]>,
    defb: BTreeMap<u32, bool>,
    pub opcodes: BTreeMap<u32, usize>,
    /// Numero di registri costanti float che lo shader può leggere, da c0 (al più 256).
    pub float_consts: u32,
}

pub type R<T> = Result<T, String>;

const OP_NAMES: &[(u32, &str)] = &[
    (0, "nop"), (1, "mov"), (2, "add"), (3, "sub"), (4, "mad"), (5, "mul"), (6, "rcp"), (7, "rsq"), (8, "dp3"), (9, "dp4"),
    (10, "min"), (11, "max"), (12, "slt"), (13, "sge"), (14, "exp"), (15, "log"), (16, "lit"), (17, "dst"), (18, "lrp"),
    (19, "frc"), (20, "m4x4"), (21, "m4x3"), (22, "m3x4"), (23, "m3x3"), (24, "m3x2"), (25, "call"), (26, "callnz"),
    (27, "loop"), (28, "ret"), (29, "endloop"), (30, "label"), (31, "dcl"), (32, "pow"), (33, "crs"), (34, "sgn"),
    (35, "abs"), (36, "nrm"), (37, "sincos"), (38, "rep"), (39, "endrep"), (40, "if"), (41, "ifc"), (42, "else"),
    (43, "endif"), (44, "break"), (45, "breakc"), (46, "mova"), (47, "defb"), (48, "defi"), (65, "texkill"),
    (66, "texld"), (78, "expp"), (79, "logp"), (80, "cnd"), (81, "def"), (88, "cmp"), (89, "bem"), (90, "dp2add"),
    (91, "dsx"), (92, "dsy"), (93, "texldd"), (94, "setp"), (95, "texldl"), (96, "breakp"),
];

pub fn op_name(op: u32) -> String {
    OP_NAMES.iter().find(|(o, _)| *o == op).map_or(format!("op{op}"), |(_, n)| n.to_string())
}

pub fn parse(code: &[u8]) -> R<Shader> {
    let toks: Vec<u32> = code.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
    let v = *toks.first().ok_or("shader vuoto")?;
    let stage = match v >> 16 {
        0xfffe => Stage::Vertex,
        0xffff => Stage::Pixel,
        _ => return Err(format!("intestazione sconosciuta {v:#x}")),
    };
    let version = ((v >> 8) & 0xff, v & 0xff);
    if version.0 != 3 {
        return Err(format!("shader model {}.{} non gestito", version.0, version.1));
    }
    let mut sh = Shader {
        stage,
        version,
        ins: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        samplers: BTreeMap::new(),
        defs: BTreeMap::new(),
        defi: BTreeMap::new(),
        defb: BTreeMap::new(),
        opcodes: BTreeMap::new(),
        float_consts: 0,
    };
    let mut p = 1;
    while p < toks.len() {
        let t = toks[p];
        let op = t & 0xffff;
        if op == 0xffff {
            break;
        }
        if op == 0xfffe {
            p += 1 + (t >> 16) as usize;
            continue;
        }
        let len = ((t >> 24) & 0xf) as usize;
        let args = toks.get(p + 1..p + 1 + len).ok_or("istruzione troncata")?;
        p += 1 + len;
        *sh.opcodes.entry(op).or_default() += 1;
        match op {
            31 => {
                let (u, d) = (args[0], args[1]);
                let (reg, num, mask) = (reg_type(d), d & 0x7ff, (d >> 16) & 0xf);
                let decl = Decl { usage: u & 0x1f, index: (u >> 16) & 0xf, mask, num };
                match reg {
                    Reg::Sampler => {
                        sh.samplers.insert(num, (u >> 27) & 0xf);
                    }
                    Reg::Input => sh.inputs.push(decl),
                    Reg::Output => sh.outputs.push(decl),
                    Reg::Misc => {}
                    other => return Err(format!("dcl su registro {other:?}")),
                }
            }
            81 => {
                sh.defs.insert(args[0] & 0x7ff, [1, 2, 3, 4].map(|i| f32::from_bits(args[i])));
            }
            48 => {
                sh.defi.insert(args[0] & 0x7ff, [1, 2, 3, 4].map(|i| args[i] as i32));
            }
            47 => {
                sh.defb.insert(args[0] & 0x7ff, args[1] != 0);
            }
            _ => {
                if t & (1 << 28) != 0 {
                    return Err(format!("{} con predicato", op_name(op)));
                }
                let has_dst = !matches!(op, 0 | 25..=30 | 38..=45 | 65 | 96);
                let mut i = 0;
                let mut dst = None;
                if has_dst {
                    let d = *args.first().ok_or("manca la destinazione")?;
                    if d & (1 << 13) != 0 {
                        return Err("destinazione con indirizzamento relativo".into());
                    }
                    dst = Some(Dst { reg: reg_type(d), num: d & 0x7ff, mask: (d >> 16) & 0xf, saturate: (d >> 20) & 1 != 0 });
                    i = 1;
                }
                let mut src = Vec::new();
                while i < args.len() {
                    let s = args[i];
                    i += 1;
                    let mut rel = None;
                    if s & (1 << 13) != 0 {
                        let r = *args.get(i).ok_or("manca il registro di indirizzamento")?;
                        i += 1;
                        rel = Some((reg_type(r), ((r >> 16) & 3) as u8));
                    }
                    src.push(Src {
                        reg: reg_type(s),
                        num: s & 0x7ff,
                        swizzle: [0, 2, 4, 6].map(|sh| ((s >> (16 + sh)) & 3) as u8),
                        modifier: (s >> 24) & 0xf,
                        rel,
                    });
                }
                sh.ins.push(Ins { op, control: (t >> 16) & 0xff, dst, src });
            }
        }
    }
    // Quanti registri float legge: con l'indirizzamento relativo, tutti. Le istruzioni a
    // matrice leggono fino a tre registri oltre quello nominato.
    sh.float_consts = sh.ins.iter().flat_map(|i| &i.src).filter(|s| s.reg == Reg::Const).map(|s| if s.rel.is_some() { 256 } else { (s.num + 4).min(256) }).max().unwrap_or(0);
    Ok(sh)
}

const XYZW: [char; 4] = ['x', 'y', 'z', 'w'];

struct Emit<'a> {
    sh: &'a Shader,
    temps: BTreeSet<u32>,
    out_regs: BTreeSet<(Reg, u32)>,
    uses_a0: bool,
    uses_vpos: bool,
    uses_vface: bool,
    body: String,
    indent: usize,
    loops: usize,
}

impl Emit<'_> {
    fn u(&self) -> &'static str {
        if self.sh.stage == Stage::Vertex { "vc" } else { "pc" }
    }

    /// Nome WGSL di un registro, senza swizzle.
    fn reg(&mut self, reg: Reg, num: u32, rel: Option<(Reg, u8)>) -> R<String> {
        Ok(match reg {
            Reg::Temp => {
                self.temps.insert(num);
                format!("r{num}")
            }
            Reg::Input => format!("v{num}"),
            Reg::Const => match rel {
                Some((r, comp)) => {
                    let idx = match r {
                        Reg::Addr => {
                            self.uses_a0 = true;
                            format!("a0.{}", XYZW[comp as usize])
                        }
                        Reg::Loop => "aL".to_string(),
                        other => return Err(format!("indirizzamento relativo con {other:?}")),
                    };
                    format!("{}.c[u32(clamp({idx} + {num}, 0, 255))]", self.u())
                }
                None if self.sh.defs.contains_key(&num) => format!("c{num}"),
                None => format!("{}.c[{num}]", self.u()),
            },
            Reg::ConstInt => {
                if self.sh.defi.contains_key(&num) { format!("i{num}") } else { format!("{}.i[{num}]", self.u()) }
            }
            Reg::Addr if self.sh.stage == Stage::Vertex => {
                self.uses_a0 = true;
                "a0".into()
            }
            Reg::Output | Reg::RastOut | Reg::AttrOut => {
                self.out_regs.insert((Reg::Output, num));
                format!("o{num}")
            }
            Reg::ColorOut => {
                self.out_regs.insert((Reg::ColorOut, num));
                format!("oC{num}")
            }
            Reg::DepthOut => {
                self.out_regs.insert((Reg::DepthOut, 0));
                "oDepth".into()
            }
            Reg::Misc => {
                if num == 0 {
                    self.uses_vpos = true;
                    "vPos".into()
                } else {
                    self.uses_vface = true;
                    "vFace".into()
                }
            }
            Reg::Loop => "vec4f(f32(aL))".into(),
            other => return Err(format!("registro {other:?}")),
        })
    }

    /// Un operando sorgente come espressione `vec4f`, con swizzle e modificatore applicati.
    fn src(&mut self, s: &Src) -> R<String> {
        self.src_off(s, 0)
    }

    /// Come `src`, ma sul registro `num + offset` (righe di una matrice).
    fn src_off(&mut self, s: &Src, offset: u32) -> R<String> {
        if s.reg == Reg::Const && s.rel.is_none() && offset > 0 && self.sh.defs.contains_key(&(s.num + offset)) {
            // riga definita con `def`: si usa la costante locale
        }
        let base = self.reg(s.reg, s.num + offset, s.rel)?;
        let sw: String = s.swizzle.iter().map(|&c| XYZW[c as usize]).collect();
        let e = if sw == "xyzw" { base } else { format!("{base}.{sw}") };
        Ok(match s.modifier {
            0 => e,
            1 => format!("(-{e})"),
            11 => format!("abs({e})"),
            12 => format!("(-abs({e}))"),
            m => return Err(format!("modificatore di sorgente {m}")),
        })
    }

    fn line(&mut self, text: &str) {
        let _ = writeln!(self.body, "{}{text}", "    ".repeat(self.indent));
    }

    /// Scrive `value` (un `vec4f`) nella destinazione, rispettando maschera e saturazione.
    fn store(&mut self, d: &Dst, value: &str) -> R<()> {
        if d.reg == Reg::Addr && self.sh.stage == Stage::Vertex {
            self.uses_a0 = true;
            self.line(&format!("a0 = vec4i(round({value}));"));
            return Ok(());
        }
        let name = self.reg(d.reg, d.num, None)?;
        let v = if d.saturate { format!("saturate({value})") } else { value.to_string() };
        if d.reg == Reg::DepthOut {
            self.line(&format!("oDepth = ({v}).x;"));
        } else if d.mask == 0xf {
            self.line(&format!("{name} = {v};"));
        } else {
            self.line(&format!("{{ let t = {v};"));
            for i in 0..4 {
                if d.mask & (1 << i) != 0 {
                    let c = XYZW[i];
                    self.line(&format!("  {name}.{c} = t.{c};"));
                }
            }
            self.line("}");
        }
        Ok(())
    }

    fn sample(&mut self, ins: &Ins) -> R<String> {
        let coord = self.src(&ins.src[0])?;
        let n = ins.src[1].num;
        let kind = *self.sh.samplers.get(&n).ok_or(format!("campionatore s{n} non dichiarato"))?;
        let (uv, bias_w) = match kind {
            2 => ("xy", "w"),
            3 | 4 => ("xyz", "w"),
            k => return Err(format!("tipo di campionatore {k}")),
        };
        let (t, s) = (format!("t{n}"), format!("s{n}"));
        Ok(match (ins.op, self.sh.stage) {
            (95, _) | (66, Stage::Vertex) => format!("textureSampleLevel({t}, {s}, ({coord}).{uv}, ({coord}).{bias_w})"),
            (93, _) => {
                let (dx, dy) = (self.src(&ins.src[2])?, self.src(&ins.src[3])?);
                format!("textureSampleGrad({t}, {s}, ({coord}).{uv}, ({dx}).{uv}, ({dy}).{uv})")
            }
            (66, _) if ins.control & 1 != 0 => format!("textureSample({t}, {s}, ({coord}).{uv} / ({coord}).w)"),
            (66, _) if ins.control & 2 != 0 => format!("textureSampleBias({t}, {s}, ({coord}).{uv}, ({coord}).w)"),
            _ => format!("textureSample({t}, {s}, ({coord}).{uv})"),
        })
    }

    fn compare(control: u32) -> R<&'static str> {
        Ok(match control & 7 {
            1 => ">",
            2 => "==",
            3 => ">=",
            4 => "<",
            5 => "!=",
            6 => "<=",
            c => return Err(format!("confronto {c}")),
        })
    }

    fn ins(&mut self, ins: &Ins) -> R<()> {
        let s = |e: &mut Self, i: usize| -> R<String> { e.src(ins.src.get(i).ok_or("operando mancante")?) };
        let value = match ins.op {
            0 => return Ok(()),
            1 | 46 => s(self, 0)?,
            2 => format!("({} + {})", s(self, 0)?, s(self, 1)?),
            3 => format!("({} - {})", s(self, 0)?, s(self, 1)?),
            4 => format!("({} * {} + {})", s(self, 0)?, s(self, 1)?, s(self, 2)?),
            5 => format!("({} * {})", s(self, 0)?, s(self, 1)?),
            6 => format!("(vec4f(1.0) / {})", s(self, 0)?),
            7 => format!("inverseSqrt(abs({}))", s(self, 0)?),
            8 => format!("vec4f(dot(({}).xyz, ({}).xyz))", s(self, 0)?, s(self, 1)?),
            9 => format!("vec4f(dot({}, {}))", s(self, 0)?, s(self, 1)?),
            10 => format!("min({}, {})", s(self, 0)?, s(self, 1)?),
            11 => format!("max({}, {})", s(self, 0)?, s(self, 1)?),
            12 => format!("vec4f({} < {})", s(self, 0)?, s(self, 1)?),
            13 => format!("vec4f({} >= {})", s(self, 0)?, s(self, 1)?),
            14 | 78 => format!("exp2({})", s(self, 0)?),
            15 | 79 => format!("log2(abs({}))", s(self, 0)?),
            18 => format!("mix({}, {}, {})", s(self, 2)?, s(self, 1)?, s(self, 0)?),
            19 => format!("fract({})", s(self, 0)?),
            20..=24 => {
                // Prodotto vettore × matrice: una riga della matrice per componente.
                let (rows, three) = match ins.op {
                    20 => (4, false),
                    21 => (3, false),
                    22 => (4, true),
                    23 => (3, true),
                    _ => (2, true),
                };
                let v = s(self, 0)?;
                let mut parts = Vec::new();
                for k in 0..4 {
                    if k < rows {
                        let row = self.src_off(&ins.src[1], k)?;
                        parts.push(if three { format!("dot(({v}).xyz, ({row}).xyz)") } else { format!("dot({v}, {row})") });
                    } else {
                        parts.push("0.0".into());
                    }
                }
                format!("vec4f({})", parts.join(", "))
            }
            32 => format!("pow(abs({}), {})", s(self, 0)?, s(self, 1)?),
            33 => format!("vec4f(cross(({}).xyz, ({}).xyz), 0.0)", s(self, 0)?, s(self, 1)?),
            34 => format!("sign({})", s(self, 0)?),
            35 => format!("abs({})", s(self, 0)?),
            36 => {
                let v = s(self, 0)?;
                format!("({v} * inverseSqrt(dot(({v}).xyz, ({v}).xyz)))")
            }
            37 => {
                let v = s(self, 0)?;
                format!("vec4f(cos(({v}).x), sin(({v}).x), 0.0, 0.0)")
            }
            88 => format!("select({}, {}, {} >= vec4f(0.0))", s(self, 2)?, s(self, 1)?, s(self, 0)?),
            90 => format!("vec4f(dot(({}).xy, ({}).xy) + ({}).x)", s(self, 0)?, s(self, 1)?, s(self, 2)?),
            91 => format!("dpdx({})", s(self, 0)?),
            92 => format!("dpdy({})", s(self, 0)?),
            66 | 93 | 95 => {
                // Lo swizzle del campionatore si applica al risultato.
                let call = self.sample(ins)?;
                let sw: String = ins.src[1].swizzle.iter().map(|&c| XYZW[c as usize]).collect();
                if sw == "xyzw" { call } else { format!("{call}.{sw}") }
            }
            65 => {
                // texkill: l'operando è nella posizione della destinazione.
                let d = ins.src.first().ok_or("texkill senza operando")?;
                let r = self.reg(d.reg, d.num, None)?;
                self.line(&format!("if (any(({r}).xyz < vec3f(0.0))) {{ discard; }}"));
                return Ok(());
            }
            40 => {
                let b = &ins.src[0];
                if b.reg != Reg::ConstBool {
                    return Err("if su un registro non booleano".into());
                }
                let cond = match self.sh.defb.get(&b.num) {
                    Some(v) => v.to_string(),
                    None => format!("(({}.b.x >> {}u) & 1u) != 0u", self.u(), b.num),
                };
                let cond = if b.modifier == 13 { format!("!({cond})") } else { cond };
                self.line(&format!("if ({cond}) {{"));
                self.indent += 1;
                return Ok(());
            }
            41 => {
                let (a, b) = (s(self, 0)?, s(self, 1)?);
                self.line(&format!("if (({a}).x {} ({b}).x) {{", Self::compare(ins.control)?));
                self.indent += 1;
                return Ok(());
            }
            42 => {
                self.indent = self.indent.saturating_sub(1);
                self.line("} else {");
                self.indent += 1;
                return Ok(());
            }
            43 | 39 | 29 => {
                self.indent = self.indent.saturating_sub(1);
                self.line("}");
                return Ok(());
            }
            38 => {
                let n = self.reg(Reg::ConstInt, ins.src[0].num, None)?;
                self.loops += 1;
                let k = self.loops;
                self.line(&format!("for (var k{k} = 0; k{k} < ({n}).x; k{k}++) {{"));
                self.indent += 1;
                return Ok(());
            }
            27 => {
                let n = self.reg(Reg::ConstInt, ins.src[1].num, None)?;
                self.loops += 1;
                let k = self.loops;
                self.line(&format!("aL = ({n}).y; for (var k{k} = 0; k{k} < ({n}).x; k{k}++) {{"));
                self.indent += 1;
                // L'incremento di aL va alla fine del corpo: lo si mette qui come `continuing`.
                self.line(&format!("continuing {{ aL = aL + ({n}).z; }}"));
                return Err("loop con aL (da verificare prima di abilitarlo)".into());
            }
            44 => {
                self.line("break;");
                return Ok(());
            }
            45 => {
                let (a, b) = (s(self, 0)?, s(self, 1)?);
                self.line(&format!("if (({a}).x {} ({b}).x) {{ break; }}", Self::compare(ins.control)?));
                return Ok(());
            }
            op => return Err(op_name(op)),
        };
        let d = ins.dst.as_ref().ok_or("manca la destinazione")?;
        self.store(d, &value)
    }
}

fn float(v: f32) -> String {
    if v.is_nan() {
        "0.0".into()
    } else if v.is_infinite() {
        if v > 0.0 { "3.4028235e38" } else { "-3.4028235e38" }.into()
    } else {
        format!("{v:?}")
    }
}

const CONSTS: &str = "struct Consts { misc: vec4f, b: vec4u, i: array<vec4i, 16>, c: array<vec4f, 256> }\n";

/// Varianti di uno stesso shader, decise dallo stato del device al momento del draw.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct Options {
    /// Maschera di `@location` degli attributi `UBYTE4`: arrivano normalizzati, vanno × 255.
    pub scale255: u32,
    /// Maschera di `@location` degli attributi `SHORT2`/`SHORT4`: vanno × 32767.
    pub scale32767: u32,
    /// Test alfa (D3DCMPFUNC, 0 = spento): confronta `oC0.a` con `pc.misc.z` e scarta il pixel.
    pub alpha_func: u32,
}

impl Shader {
    /// Indici dei render target su cui il pixel shader scrive.
    pub fn color_outputs(&self) -> Vec<u32> {
        let set: BTreeSet<u32> = self.ins.iter().filter_map(|i| i.dst).filter(|d| d.reg == Reg::ColorOut).map(|d| d.num).collect();
        set.into_iter().collect()
    }
    pub fn writes_depth(&self) -> bool {
        self.ins.iter().filter_map(|i| i.dst).any(|d| d.reg == Reg::DepthOut)
    }
}

/// Traduce uno shader in un modulo WGSL con un solo punto d'ingresso, `main`.
pub fn translate(sh: &Shader) -> R<String> {
    translate_with(sh, &Options::default())
}

pub fn translate_with(sh: &Shader, opt: &Options) -> R<String> {
    let mut e = Emit { sh, temps: BTreeSet::new(), out_regs: BTreeSet::new(), uses_a0: false, uses_vpos: false, uses_vface: false, body: String::new(), indent: 1, loops: 0 };
    for ins in &sh.ins {
        e.ins(ins).map_err(|m| format!("{m} (istruzione {})", op_name(ins.op)))?;
    }
    if e.indent != 1 {
        return Err("blocchi di controllo non bilanciati".into());
    }
    let vertex = sh.stage == Stage::Vertex;
    let mut w = String::new();
    w.push_str(CONSTS);
    let _ = writeln!(w, "@group({}) @binding(0) var<uniform> {}: Consts;", if vertex { 0 } else { 3 }, e.u());
    let group = if vertex { 2 } else { 1 };
    for (&n, &kind) in &sh.samplers {
        let ty = match kind {
            2 => "texture_2d<f32>",
            3 => "texture_cube<f32>",
            4 => "texture_3d<f32>",
            k => return Err(format!("tipo di campionatore {k}")),
        };
        let _ = writeln!(w, "@group({group}) @binding({}) var t{n}: {ty};\n@group({group}) @binding({}) var s{n}: sampler;", 2 * n, 2 * n + 1);
    }

    // Ingressi: un campo per dichiarazione; più dichiarazioni possono condividere un registro.
    let loc = |d: &Decl, input: bool| -> R<u32> {
        let f = if vertex && input { attribute_location } else { varying_location };
        f(d.usage, d.index).ok_or(format!(
            "semantica {}{} senza posizione assegnata",
            USAGE_NAMES.get(d.usage as usize).unwrap_or(&"?"),
            d.index
        ))
    };
    w.push_str("struct In {\n");
    for (k, d) in sh.inputs.iter().enumerate() {
        let _ = writeln!(w, "    @location({}) i{k}: vec4f,", loc(d, true)?);
    }
    if e.uses_vpos {
        w.push_str("    @builtin(position) frag_coord: vec4f,\n");
    }
    if e.uses_vface {
        w.push_str("    @builtin(front_facing) front: bool,\n");
    }
    if sh.inputs.is_empty() && !e.uses_vpos && !e.uses_vface {
        w.push_str("    @builtin(vertex_index) unused: u32,\n");
    }
    w.push_str("}\nstruct Out {\n");
    if vertex {
        w.push_str("    @builtin(position) position: vec4f,\n");
        for (k, d) in sh.outputs.iter().enumerate() {
            if d.usage == 0 && d.index == 0 || d.usage == 4 {
                continue; // posizione e dimensione del punto
            }
            let _ = writeln!(w, "    @location({}) o{k}: vec4f,", loc(d, false)?);
        }
    } else {
        for &(reg, n) in &e.out_regs {
            match reg {
                Reg::ColorOut => {
                    let _ = writeln!(w, "    @location({n}) c{n}: vec4f,");
                }
                Reg::DepthOut => w.push_str("    @builtin(frag_depth) depth: f32,\n"),
                _ => {}
            }
        }
        if !e.out_regs.iter().any(|(r, _)| *r == Reg::ColorOut) {
            w.push_str("    @location(0) c0: vec4f,\n");
        }
    }
    w.push_str("}\n");
    if !vertex && sh.inputs.is_empty() && !e.uses_vpos && !e.uses_vface {
        // Un pixel shader senza ingressi non può avere `vertex_index`: si toglie il segnaposto.
        w = w.replace("    @builtin(vertex_index) unused: u32,\n", "    @builtin(position) unused: vec4f,\n");
    }

    let _ = writeln!(w, "@{} fn main(input: In) -> Out {{", if vertex { "vertex" } else { "fragment" });
    for (n, v) in &sh.defs {
        let _ = writeln!(w, "    let c{n} = vec4f({}, {}, {}, {});", float(v[0]), float(v[1]), float(v[2]), float(v[3]));
    }
    for (n, v) in &sh.defi {
        let _ = writeln!(w, "    let i{n} = vec4i({}, {}, {}, {});", v[0], v[1], v[2], v[3]);
    }
    for n in &e.temps {
        let _ = writeln!(w, "    var r{n} = vec4f(0.0);");
    }
    if e.uses_a0 {
        w.push_str("    var a0 = vec4i(0);\n");
    }
    // Registri di ingresso, ricostruiti dalle dichiarazioni.
    let input_regs: BTreeSet<u32> = sh.inputs.iter().map(|d| d.num).collect();
    for n in &input_regs {
        let _ = writeln!(w, "    var v{n} = vec4f(0.0, 0.0, 0.0, 1.0);");
        for (k, d) in sh.inputs.iter().enumerate().filter(|(_, d)| d.num == *n) {
            for i in 0..4 {
                if d.mask & (1 << i) != 0 {
                    let c = XYZW[i];
                    let l = if vertex { attribute_location(d.usage, d.index).unwrap_or(31) } else { 31 };
                    let scale = if opt.scale255 & (1 << l) != 0 {
                        " * 255.0"
                    } else if opt.scale32767 & (1 << l) != 0 {
                        " * 32767.0"
                    } else {
                        ""
                    };
                    let _ = writeln!(w, "    v{n}.{c} = input.i{k}.{c}{scale};");
                }
            }
        }
    }
    if e.uses_vpos {
        w.push_str("    let vPos = vec4f(floor(input.frag_coord.xy), 0.0, 0.0);\n");
    }
    if e.uses_vface {
        w.push_str("    let vFace = vec4f(select(-1.0, 1.0, input.front));\n");
    }
    let out_regs: BTreeSet<u32> = if vertex { sh.outputs.iter().map(|d| d.num).collect() } else { BTreeSet::new() };
    for n in out_regs.iter().chain(e.out_regs.iter().filter(|(r, n)| *r == Reg::Output && !out_regs.contains(n)).map(|(_, n)| n)) {
        let _ = writeln!(w, "    var o{n} = vec4f(0.0);");
    }
    for &(reg, n) in &e.out_regs {
        match reg {
            Reg::ColorOut => {
                let _ = writeln!(w, "    var oC{n} = vec4f(0.0);");
            }
            Reg::DepthOut => w.push_str("    var oDepth = 0.0;\n"),
            _ => {}
        }
    }
    w.push_str(&e.body);
    w.push_str("    var output: Out;\n");
    if vertex {
        let pos = sh.outputs.iter().find(|d| d.usage == 0 && d.index == 0).ok_or("il vertex shader non dichiara la posizione")?;
        let _ = writeln!(w, "    output.position = o{};", pos.num);
        w.push_str("    output.position = vec4f(output.position.xy + vc.misc.xy * output.position.w, output.position.zw);\n");
        for (k, d) in sh.outputs.iter().enumerate() {
            if d.usage == 0 && d.index == 0 || d.usage == 4 {
                continue;
            }
            let _ = writeln!(w, "    output.o{k} = o{};", d.num);
        }
    } else {
        let test = match opt.alpha_func {
            1 => Some("false".to_string()),
            f @ 2..=7 => Some(format!("oC0.w {} pc.misc.z", ["<", "==", "<=", ">", "!=", ">="][f as usize - 2])),
            _ => None,
        };
        if let (Some(t), true) = (test, e.out_regs.contains(&(Reg::ColorOut, 0))) {
            let _ = writeln!(w, "    if (!({t})) {{ discard; }}");
        }
        let mut any = false;
        for &(reg, n) in &e.out_regs {
            match reg {
                Reg::ColorOut => {
                    any = true;
                    let _ = writeln!(w, "    output.c{n} = oC{n};");
                }
                Reg::DepthOut => w.push_str("    output.depth = oDepth;\n"),
                _ => {}
            }
        }
        if !any {
            w.push_str("    output.c0 = vec4f(0.0);\n");
        }
    }
    w.push_str("    return output;\n}\n");
    Ok(w)
}

/// Controlla il WGSL con naga, lo stesso validatore che usa wgpu.
pub fn validate(wgsl: &str) -> R<()> {
    let module = naga::front::wgsl::parse_str(wgsl).map_err(|e| format!("analisi WGSL: {}", e.emit_to_string(wgsl)))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| format!("validazione WGSL: {}", e.emit_to_string(wgsl)))?;
    Ok(())
}
