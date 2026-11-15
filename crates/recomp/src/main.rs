//! `altair`: analysis and recompilation tools for Assassin's Creed (2007), Steam DX9 build.

mod discover;
mod rtti;
mod translate;

use altair_pe::{Image, AC1_DX9_STEAM_SHA1};
use discover::{Discovery, Kind};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "\
usage: altair <command> <AssassinsCreed_Dx9.exe> [options]

  info                      hash, sections, imports, exports
  rtti                      counts of type descriptors and vtables
  discover [--strict] [--out FILE.json]
                            function discovery; with --out it saves the list
                            (it only contains addresses, but still belongs in private/)

  translate --out FILE.c [--roots FILE] ADDR...
                            translates the given functions and the ones they call into C
  image --out DIR [--window KB] [--big N] [--hooks FILE]
  icon --out FILE.ico       extracts the executable's icon (for the Windows build; stays in build/)
                            translates the whole image into DIR: one C file per address
                            window (64 KB), table.c and a Makefile. Functions with more than
                            N instructions (5000) are not optimized by the compiler
  candidates [--max N]      functions suited to the differential test: the whole closure is
                            translatable, with no imports and no indirect calls
  coverage [--list]         how many functions translate today, and what blocks the others;
                            --list lists every function not translated with origin and reason
  census                    how many times each instruction appears in the reached code,
                            and how many of those the translator can already emit

Common option: --any-exe    accepts an executable whose hash differs from the Steam build's";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (Some(cmd), Some(path)) = (args.first(), args.get(1)) else {
        return Err(USAGE.into());
    };
    let img = Image::load(path)?;
    let sha = img.sha1();
    if sha != AC1_DX9_STEAM_SHA1 && !flag(args, "--any-exe") {
        return Err(format!(
            "SHA-1 {sha}: this is not AssassinsCreed_Dx9.exe from the Steam build ({AC1_DX9_STEAM_SHA1}).\n\
             Use --any-exe to analyze it anyway."
        )
        .into());
    }
    match cmd.as_str() {
        "info" => info(&img, &sha),
        "rtti" => rtti_stats(&img),
        "icon" => icon_cmd(&img, value(args, "--out").ok_or("missing --out FILE.ico")?)?,
        "discover" => discover_cmd(&img, flag(args, "--strict"), value(args, "--out"))?,
        "translate" => translate_cmd(&img, args)?,
        "coverage" => coverage_cmd(&img, flag(args, "--list")),
        "image" => image_cmd(&img, args)?,
        "census" => census_cmd(&img),
        "candidates" => candidates_cmd(&img, value(args, "--max").and_then(|v| v.parse().ok()).unwrap_or(500)),
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn info(img: &Image, sha: &str) {
    println!("sha1        {sha}");
    println!("size        {} bytes", img.data.len());
    println!("image base  {:#x}   entry {:#x}   timestamp {}", img.base, img.entry, img.timestamp);
    println!("sections:");
    for s in &img.sections {
        println!("  {:<8} va={:#010x} vsize={:#09x} rsize={:#09x}", s.name, s.va, s.vsize, s.rsize);
    }
    let mut dlls: BTreeMap<&str, usize> = BTreeMap::new();
    for i in &img.imports {
        *dlls.entry(&i.dll).or_default() += 1;
    }
    println!("imports     {} functions from {} DLLs", img.imports.len(), dlls.len());
    for (dll, n) in dlls {
        println!("  {n:4}  {dll}");
    }
    println!("exports     {}", img.exports.len());
    println!("TLS callbacks {}", img.tls_callbacks.len());
}

fn rtti_stats(img: &Image) {
    let r = rtti::analyze(img);
    let slots: Vec<u32> = r.vtables.iter().flat_map(|v| v.slots.iter().copied()).collect();
    let distinct: HashSet<u32> = slots.iter().copied().collect();
    let classes: HashSet<&str> = r.vtables.iter().map(|v| v.class.as_str()).collect();
    println!("type_info::vftable  {:#x}", r.type_info_vftable);
    println!("type descriptors    {}", r.types.len());
    println!("vtables             {}", r.vtables.len());
    println!("  total slots       {}", slots.len());
    println!("  distinct functions {}", distinct.len());
    println!("  classes w/ vtable {}", classes.len());
    println!("  secondary         {}", r.vtables.iter().filter(|v| v.offset != 0).count());
    if let Some(v) = r.vtables.iter().max_by_key(|v| v.slots.len()) {
        println!("  largest           {} ({} slots)", rtti::undecorate(&v.class), v.slots.len());
    }
}

fn discover_cmd(img: &Image, strict: bool, out: Option<&str>) -> std::io::Result<()> {
    let t0 = std::time::Instant::now();
    let (d, r) = Discovery::run(img, strict, true);
    let (code, table, int3, unknown) = d.text_breakdown();
    println!("functions           {}", d.starts.len());
    println!("instructions        {}", d.n_insn);
    println!("jump tables         {} ({} of them with a bound from the cmp)", d.tables, d.bounded_tables);
    println!("by origin:");
    let kinds = [Kind::Entry, Kind::Export, Kind::Tls, Kind::Vtable, Kind::Immediate, Kind::DataWord, Kind::Unaligned];
    for k in kinds {
        let direct = d.starts.values().filter(|o| o.kind == k && !o.via_call).count();
        let called = d.starts.values().filter(|o| o.kind == k && o.via_call).count();
        if direct + called > 0 {
            println!("  {:<16} {direct:>6}  (+{called} reached by call)", k.label());
        }
    }
    let total = d.own.len() as f64;
    println!(".text ({} bytes):", d.own.len());
    for (what, n) in [("code", code), ("jump tables", table), ("unreached int3", int3), ("unclassified", unknown)] {
        println!("  {what:<20} {n:>9}  {:5.1}%", 100.0 * n as f64 / total);
    }
    println!("rejections:");
    let mut rej: Vec<_> = d.rejected.iter().collect();
    rej.sort_by_key(|&(_, n)| std::cmp::Reverse(*n));
    for (why, n) in rej {
        println!("  {n:>6}  {why}");
    }
    let missing = r
        .vtables
        .iter()
        .flat_map(|v| v.slots.iter())
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|s| !d.starts.contains_key(s))
        .count();
    println!("vtable slots not accepted: {missing}");
    println!("time                {:.2} s", t0.elapsed().as_secs_f64());

    if let Some(path) = out {
        let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(f, "{{\"functions\": [")?;
        let n = d.starts.len();
        for (i, (addr, o)) in d.starts.iter().enumerate() {
            let label = if o.via_call { format!("{} → call", o.kind.label()) } else { o.kind.label().to_string() };
            writeln!(f, "[{addr}, \"{label}\"]{}", if i + 1 < n { "," } else { "" })?;
        }
        writeln!(f, "]}}")?;
        eprintln!("list written to {path}");
    }
    Ok(())
}

fn origin_label(o: &discover::Origin) -> String {
    if o.via_call { format!("{} → call", o.kind.label()) } else { o.kind.label().to_string() }
}

/// The certain starts: a jump to one of these is a tail call, not internal code.
fn hard_starts(d: &Discovery) -> HashSet<u32> {
    d.starts
        .iter()
        .filter(|(_, o)| o.via_call || matches!(o.kind, Kind::Entry | Kind::Export | Kind::Tls | Kind::Vtable))
        .map(|(&a, _)| a)
        .collect()
}

fn translate_cmd(img: &Image, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let out = value(args, "--out").ok_or("missing --out FILE.c")?;
    let mut roots = Vec::new();
    for a in args.iter().skip(2).filter(|a| a.starts_with("0x")) {
        roots.push(u32::from_str_radix(&a[2..], 16)?);
    }
    if let Some(file) = value(args, "--roots") {
        for line in std::fs::read_to_string(file)?.lines() {
            if let Some(hex) = line.trim().strip_prefix("0x") {
                roots.push(u32::from_str_radix(hex, 16)?);
            }
        }
    }
    if roots.is_empty() {
        return Err("no address to translate".into());
    }
    let (d, _) = Discovery::run(img, false, false);
    let mut hard = hard_starts(&d);
    hard.extend(&roots);
    let is_hard = |a: u32| hard.contains(&a);

    // Closure over direct calls; untranslatable functions are left out.
    let mut funcs: BTreeMap<u32, translate::Func> = BTreeMap::new();
    let mut failed: BTreeMap<u32, String> = BTreeMap::new();
    let mut pending = roots.clone();
    while let Some(a) = pending.pop() {
        if funcs.contains_key(&a) || failed.contains_key(&a) {
            continue;
        }
        match translate::decode(img, a, &is_hard).and_then(|f| translate::emit(&f, &|_| false).map(|_| f)) {
            Ok(f) => {
                pending.extend(f.calls.iter().copied());
                funcs.insert(a, f);
            }
            Err(e) => {
                failed.insert(a, e);
            }
        }
    }
    let known = |a: u32| funcs.contains_key(&a);
    let mut c = String::from("/* Generated by `altair translate`: do not edit and do not publish. */\n#include \"altair.h\"\n\n");
    for a in funcs.keys() {
        c += &format!("void {}(Ctx *c);\n", translate::fn_name(*a));
    }
    c.push('\n');
    let mut insns = 0;
    for f in funcs.values() {
        insns += f.insns.len();
        c += &translate::emit(f, &known)?;
        c.push('\n');
    }
    c += "const AltairEntry altair_functions[] = {\n";
    for a in funcs.keys() {
        c += &format!("    {{{a:#x}u, {}}},\n", translate::fn_name(*a));
    }
    c += &format!("}};\nconst u32 altair_function_count = {};\n", funcs.len());
    std::fs::write(out, c)?;
    eprintln!("{} functions, {insns} instructions translated into {out}", funcs.len());
    for (a, e) in &failed {
        eprintln!("  not translated {a:#010x}: {e}");
    }
    for r in &roots {
        if failed.contains_key(r) {
            return Err(format!("the requested function {r:#x} is not translatable").into());
        }
    }
    Ok(())
}

fn coverage_cmd(img: &Image, list: bool) {
    let mut failures: Vec<(u32, String, String)> = Vec::new();
    let (d, _) = Discovery::run(img, false, false);
    let hard = hard_starts(&d);
    let is_hard = |a: u32| hard.contains(&a);
    let (mut ok, mut ok_insns, mut total_insns) = (0usize, 0usize, 0usize);
    let mut blockers: HashMap<String, usize> = HashMap::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for &a in d.starts.keys() {
        match translate::decode(img, a, &is_hard) {
            Ok(f) => {
                total_insns += f.insns.len();
                seen.extend(f.insns.keys());
                match translate::emit(&f, &|_| true) {
                    Ok(_) => {
                        ok += 1;
                        ok_insns += f.insns.len();
                    }
                    Err(e) => {
                        *blockers.entry(e.split(' ').next().unwrap_or("?").to_string()).or_default() += 1;
                        failures.push((a, origin_label(&d.starts[&a]), e));
                    }
                }
            }
            Err(e) => {
                *blockers.entry(format!("[decode] {}", e.split(" (").next().unwrap_or("?"))).or_default() += 1;
                failures.push((a, origin_label(&d.starts[&a]), format!("[decode] {e}")));
            }
        }
    }
    let n = d.starts.len();
    println!("functions translated {ok} of {n} ({:.1}%)", 100.0 * ok as f64 / n as f64);
    println!("instructions        {ok_insns} of {total_insns} ({:.1}%), counted per function", 100.0 * ok_insns as f64 / total_insns as f64);
    println!("distinct instructions {}", seen.len());
    println!("first obstacle of the functions not translated:");
    let mut b: Vec<_> = blockers.into_iter().collect();
    b.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (why, n) in b.iter().take(40) {
        println!("  {n:>6}  {why}");
    }
    if b.len() > 40 {
        println!("  ... and {} more reasons", b.len() - 40);
    }
    if list {
        println!("functions not translated:");
        for (a, origin, why) in &failures {
            println!("  {a:#010x}  {origin:<26} {why}");
        }
    }
}

fn candidates_cmd(img: &Image, max: usize) {
    use iced_x86::Mnemonic;
    struct Info {
        insns: usize,
        calls: Vec<u32>,
        opaque: bool,
        mnemonics: BTreeSet<Mnemonic>,
    }
    let (d, _) = Discovery::run(img, false, false);
    let hard = hard_starts(&d);
    let is_hard = |a: u32| hard.contains(&a);
    let mut info: HashMap<u32, Info> = HashMap::new();
    for &a in d.starts.keys() {
        if let Ok(f) = translate::decode(img, a, &is_hard) {
            if translate::emit(&f, &|_| true).is_ok() {
                info.insert(a, Info {
                    insns: f.insns.len(),
                    calls: f.calls.iter().copied().collect(),
                    opaque: f.has_opaque_indirect(),
                    mnemonics: f.insns.values().map(|i| i.mnemonic()).collect(),
                });
            }
        }
    }
    // Results that cannot be compared with the emulator: time, CPU identity, and the x87
    // transcendental functions, which the emulator computes in double precision.
    const UNCOMPARABLE: [Mnemonic; 12] = [
        Mnemonic::Rdtsc, Mnemonic::Cpuid, Mnemonic::Fsin, Mnemonic::Fcos, Mnemonic::Fsincos, Mnemonic::Fptan,
        Mnemonic::Fpatan, Mnemonic::Fyl2x, Mnemonic::Fyl2xp1, Mnemonic::F2xm1, Mnemonic::Fprem, Mnemonic::Fprem1,
    ];
    // Closure over direct calls: small, fully translatable, with no unknown targets.
    let mut good: Vec<(u32, BTreeSet<Mnemonic>)> = Vec::new();
    for (&a, root) in &info {
        if root.insns < 4 {
            continue;
        }
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        let mut pending = vec![a];
        let mut total = 0;
        let mut ok = true;
        let mut mn = BTreeSet::new();
        while let Some(t) = pending.pop() {
            if !seen.insert(t) {
                continue;
            }
            match info.get(&t) {
                Some(i) if !i.opaque && !i.mnemonics.iter().any(|m| UNCOMPARABLE.contains(m)) => {
                    total += i.insns;
                    mn.extend(i.mnemonics.iter().copied());
                    pending.extend(i.calls.iter().copied());
                }
                _ => ok = false,
            }
            if !ok || seen.len() > 12 || total > 3000 {
                ok = false;
                break;
            }
        }
        if ok {
            good.push((a, mn));
        }
    }
    good.sort_by_key(|(a, _)| *a);
    // First a few functions for each kind of instruction, starting from the rarest; then the
    // others in pseudo-random but repeatable order.
    let mut count: HashMap<Mnemonic, usize> = HashMap::new();
    for (_, mn) in &good {
        for m in mn {
            *count.entry(*m).or_default() += 1;
        }
    }
    let mut rare: Vec<(Mnemonic, usize)> = count.into_iter().collect();
    rare.sort_by_key(|&(m, n)| (n, m as u32));
    let mut chosen: BTreeSet<u32> = BTreeSet::new();
    for (m, _) in rare {
        for (a, _) in good.iter().filter(|(_, mn)| mn.contains(&m)).take(4) {
            if chosen.len() < max {
                chosen.insert(*a);
            }
        }
    }
    let mut rest: Vec<u32> = good.iter().map(|(a, _)| *a).filter(|a| !chosen.contains(a)).collect();
    rest.sort_by_key(|a| a.wrapping_mul(0x9E37_79B1));
    for a in rest {
        if chosen.len() >= max {
            break;
        }
        chosen.insert(a);
    }
    eprintln!("{} suitable functions out of {} translatable; {} chosen", good.len(), info.len(), chosen.len());
    for a in chosen {
        println!("{a:#x}");
    }
}

fn census_cmd(img: &Image) {
    use iced_x86::Instruction;
    let (d, _) = Discovery::run(img, false, false);
    let hard = hard_starts(&d);
    let is_hard = |a: u32| hard.contains(&a);
    let mut seen: HashMap<u32, Instruction> = HashMap::new();
    for &a in d.starts.keys() {
        if let Ok(f) = translate::decode(img, a, &is_hard) {
            seen.extend(f.insns.iter().map(|(k, v)| (*k, *v)));
        }
    }
    // (total, untranslatable) per mnemonic, over the distinct instructions.
    let mut count: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for ins in seen.values() {
        let e = count.entry(format!("{:?}", ins.mnemonic()).to_lowercase()).or_default();
        e.0 += 1;
        if !translate::supported(ins) {
            e.1 += 1;
        }
    }
    let total: usize = count.values().map(|c| c.0).sum();
    let missing: usize = count.values().map(|c| c.1).sum();
    println!("distinct instructions {total}, of which not yet translatable {missing} ({:.1}%)", 100.0 * missing as f64 / total as f64);
    println!("{:>9} {:>9}  instruction", "total", "missing");
    let mut rows: Vec<_> = count.into_iter().collect();
    rows.sort_by_key(|(_, c)| std::cmp::Reverse((c.1, c.0)));
    for (m, (n, miss)) in rows {
        println!("{n:>9} {miss:>9}  {m}");
    }
}

/// Writes the file only if its content changed, so `make` does not recompile what is unchanged.
fn write_if_changed(path: &std::path::Path, content: &str) -> std::io::Result<bool> {
    if std::fs::read(path).is_ok_and(|old| old == content.as_bytes()) {
        return Ok(false);
    }
    std::fs::write(path, content)?;
    Ok(true)
}

fn image_cmd(img: &Image, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let t0 = std::time::Instant::now();
    let out = std::path::PathBuf::from(value(args, "--out").ok_or("missing --out DIR")?);
    let window = value(args, "--window").and_then(|v| v.parse::<u32>().ok()).unwrap_or(64) * 1024;
    let big = value(args, "--big").and_then(|v| v.parse::<usize>().ok()).unwrap_or(5000);
    std::fs::create_dir_all(&out)?;
    // Hooks: guest functions that on entry go through the runtime (`altair_hook`), which can
    // observe or replace them. One hexadecimal address per line, the rest is a comment.
    let mut hooks: BTreeSet<u32> = BTreeSet::new();
    if let Some(file) = value(args, "--hooks") {
        for line in std::fs::read_to_string(file)?.lines() {
            if let Some(word) = line.split('#').next().and_then(|l| l.split_whitespace().next()) {
                hooks.insert(u32::from_str_radix(word.trim_start_matches("0x"), 16).map_err(|_| format!("{file}: invalid address: {word}"))?);
            }
        }
    }
    let mut hooked = 0;

    let (d, _) = Discovery::run(img, false, false);
    let hard = hard_starts(&d);
    let is_hard = |a: u32| hard.contains(&a);
    let mut funcs: BTreeMap<u32, translate::Func> = BTreeMap::new();
    for &a in d.starts.keys() {
        match translate::decode(img, a, &is_hard) {
            Ok(f) => {
                funcs.insert(a, f);
            }
            Err(e) => eprintln!("not decoded {a:#010x}: {e}"),
        }
    }
    // Safety net: every target of a direct `call` in the translated code must be a
    // translated function, even if discovery had not found it (the translator follows
    // jump tables slightly differently and reaches a few more cases).
    let mut extra = 0;
    loop {
        let missing: BTreeSet<u32> =
            funcs.values().flat_map(|f| f.calls.iter().copied()).filter(|t| !funcs.contains_key(t) && d.lo <= *t && *t < d.hi).collect();
        if missing.is_empty() {
            break;
        }
        for a in missing {
            match translate::decode(img, a, &is_hard).and_then(|f| translate::emit(&f, &|_| true).map(|_| f)) {
                Ok(f) => {
                    funcs.insert(a, f);
                    extra += 1;
                }
                Err(e) => {
                    eprintln!("call target not translatable {a:#010x}: {e}");
                    // An empty function avoids retrying forever; at runtime it will be a trap.
                    funcs.insert(a, translate::Func::empty(a));
                }
            }
        }
    }
    let known = |a: u32| funcs.contains_key(&a) && !funcs[&a].insns.is_empty();

    // One file per address window: a function lives in the file of the window it starts in.
    let mut parts: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &a in funcs.keys() {
        parts.entry(d.lo + (a - d.lo) / window * window).or_default().push(a);
    }
    let (mut written, mut bytes, mut insns, mut n_big, mut biggest) = (0usize, 0usize, 0usize, 0usize, (0usize, 0u32));
    let mut names: BTreeSet<String> = BTreeSet::new();
    for (base, addrs) in &parts {
        let mut body = String::new();
        let mut protos: BTreeSet<u32> = BTreeSet::new();
        for a in addrs {
            let f = &funcs[a];
            if f.insns.is_empty() {
                continue;
            }
            protos.insert(*a);
            protos.extend(f.calls.iter().copied().filter(|&t| known(t)));
            let n = f.insns.len();
            insns += n;
            if n > biggest.0 {
                biggest = (n, *a);
            }
            let mut c = translate::emit(f, &known)?;
            if hooks.contains(a) {
                const FLAGS: &str = "    u32 cf = 0, zf = 0, sf = 0, of = 0, pf = 0, af = 0;\n";
                c = c.replacen(FLAGS, &format!("{FLAGS}    if (altair_hook(c, {a:#x}u)) return;\n"), 1);
                hooked += 1;
            }
            if n > big {
                n_big += 1;
                body.push_str("ALTAIR_BIG ");
            }
            body.push_str(&c);
            body.push('\n');
        }
        let mut text = String::with_capacity(body.len() + 64 * protos.len());
        text.push_str("/* Generated by `altair image`: do not edit and do not publish. */\n#include \"altair.h\"\n\n");
        for p in &protos {
            text.push_str(&format!("void {}(Ctx *c);\n", translate::fn_name(*p)));
        }
        text.push('\n');
        text.push_str(&body);
        let name = format!("part_{base:08x}.c");
        bytes += text.len();
        written += write_if_changed(&out.join(&name), &text)? as usize;
        names.insert(name);
    }

    let mut table = String::from("/* Generated by `altair image`: address -> function table, sorted. */\n#include \"altair.h\"\n\n");
    let emitted: Vec<u32> = funcs.iter().filter(|(_, f)| !f.insns.is_empty()).map(|(a, _)| *a).collect();
    for a in &emitted {
        table.push_str(&format!("void {}(Ctx *c);\n", translate::fn_name(*a)));
    }
    table.push_str("\nconst AltairEntry altair_functions[] = {\n");
    for a in &emitted {
        table.push_str(&format!("    {{{a:#x}u, {}}},\n", translate::fn_name(*a)));
    }
    table.push_str(&format!("}};\nconst u32 altair_function_count = {};\n", emitted.len()));
    bytes += table.len();
    written += write_if_changed(&out.join("table.c"), &table)? as usize;
    names.insert("table.c".into());

    // Remove the files of windows that no longer exist.
    let mut removed = 0;
    for e in std::fs::read_dir(&out)? {
        let e = e?;
        let n = e.file_name().to_string_lossy().into_owned();
        if n.ends_with(".c") && !names.contains(&n) {
            std::fs::remove_file(e.path())?;
            removed += 1;
        }
    }

    let runtime = std::fs::canonicalize("runtime").map_err(|_| "run `altair image` from the project root (runtime/ is needed)")?;
    let makefile = format!(
        "# Generated by `altair image`. Usage: make -j$(nproc)   or   make -j$(nproc) CC=gcc TAG=gcc\n\
         CC = clang\n\
         OPT = -O2\n\
         TAG = $(CC)\n\
         RUNTIME := {rt}\n\
         CFLAGS := $(OPT) -fwrapv -fno-strict-aliasing -fno-math-errno -ffp-contract=off -fPIC -w -I$(RUNTIME)\n\
         OBJ := ../../obj/full-$(TAG)\n\
         LIB := ../../libaltair_ac1-$(TAG).so\n\
         SRCS := $(wildcard *.c)\n\
         OBJS := $(SRCS:%.c=$(OBJ)/%.o) $(OBJ)/altair_runtime.o\n\n\
         $(LIB): $(OBJS)\n\t@echo \"link $@\"\n\t@$(CC) -shared $(OBJS) -lm -o $@\n\n\
         $(OBJ)/%.o: %.c $(wildcard $(RUNTIME)/*.h) | $(OBJ)\n\t@$(CC) $(CFLAGS) -c $< -o $@\n\n\
         $(OBJ)/altair_runtime.o: $(RUNTIME)/altair.c $(wildcard $(RUNTIME)/*.h) | $(OBJ)\n\t$(CC) $(CFLAGS) -c $< -o $@\n\n\
         $(OBJ):\n\tmkdir -p $@\n",
        rt = runtime.display()
    );
    write_if_changed(&out.join("Makefile"), &makefile)?;

    println!("functions           {} ({extra} of them added because called but not discovered)", funcs.len());
    println!("instructions        {insns} (counted per function)");
    println!("files               {} of {} KB of addresses each, plus table.c", parts.len(), window / 1024);
    println!("size                {:.1} MB of C", bytes as f64 / 1e6);
    println!("files rewritten     {written}, removed {removed}");
    if !hooks.is_empty() {
        println!("hooks               {hooked} of {} requested", hooks.len());
    }
    println!("functions not optimized (more than {big} instructions): {n_big}; the largest {:#x} with {} instructions", biggest.1, biggest.0);
    println!("time                {:.1} s", t0.elapsed().as_secs_f64());
    Ok(())
}

/// Extracts the executable's first group icon into a `.ico` file. Like the generated C, the
/// result is material derived from the game: it goes in `build/`, never in the repository.
fn icon_cmd(img: &Image, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    let d = &img.data;
    let u16_at = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or("truncated resources");
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).ok_or("truncated resources");
    let pe = u32_at(0x3c)?;
    let rva = u32_at(pe + 24 + 96 + 8 * 2)? as u32;
    if rva == 0 {
        return Err("the executable has no resources".into());
    }
    let root = img.off(img.base + rva).ok_or("resource section not found")?;
    // The entries of a directory: (id or name, offset from the root, is a subdirectory).
    let entries = |dir: usize| -> Result<Vec<(usize, usize, bool)>, &'static str> {
        let n = u16_at(dir + 12)? + u16_at(dir + 14)?;
        (0..n).map(|i| Ok((u32_at(dir + 16 + 8 * i)?, u32_at(dir + 20 + 8 * i)? & 0x7fff_ffff, u32_at(dir + 20 + 8 * i)? >> 31 == 1))).collect()
    };
    // The data of the first language of a resource (type, id); without an id, the first of the type.
    let find = |kind: usize, id: Option<usize>| -> Result<&[u8], Box<dyn std::error::Error>> {
        let types = entries(root)?;
        let t = types.iter().find(|e| e.0 == kind && e.2).ok_or("resource type missing")?;
        let names = entries(root + t.1)?;
        let n = names.iter().find(|e| id.map_or(true, |id| e.0 == id) && e.2).ok_or("resource missing")?;
        let lang = *entries(root + n.1)?.first().ok_or("resource has no languages")?;
        let (data_rva, size) = (u32_at(root + lang.1)? as u32, u32_at(root + lang.1 + 4)?);
        let off = img.off(img.base + data_rva).ok_or("resource data outside the sections")?;
        Ok(d.get(off..off + size).ok_or("resource data truncated")?)
    };
    let group = find(14, None)?; // RT_GROUP_ICON
    let count = u16::from_le_bytes([group[4], group[5]]) as usize;
    let mut ico = Vec::from(&group[..6]);
    let mut images = Vec::new();
    let mut offset = 6 + 16 * count;
    for i in 0..count {
        let e = group.get(6 + 14 * i..20 + 14 * i).ok_or("icon group truncated")?;
        let image = find(3, Some(u16::from_le_bytes([e[12], e[13]]) as usize))?; // RT_ICON
        ico.extend_from_slice(&e[..8]);
        ico.extend_from_slice(&(image.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += image.len();
        images.push(image);
    }
    for image in images {
        ico.extend_from_slice(image);
    }
    if let Some(dir) = std::path::Path::new(out).parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, &ico)?;
    println!("icon                {count} images, {} bytes, in {out}", ico.len());
    Ok(())
}
