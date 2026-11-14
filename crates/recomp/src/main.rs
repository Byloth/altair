//! `acrek`: strumenti di analisi e ricompilazione per Assassin's Creed (2007), build Steam DX9.

mod discover;
mod rtti;
mod translate;

use acrek_pe::{Image, AC1_DX9_STEAM_SHA1};
use discover::{Discovery, Kind};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "\
uso: acrek <comando> <AssassinsCreed_Dx9.exe> [opzioni]

  info                      hash, sezioni, import, export
  rtti                      conteggi di type descriptor e vtable
  discover [--strict] [--out FILE.json]
                            scoperta delle funzioni; con --out salva l'elenco
                            (contiene solo indirizzi, ma va comunque in private/)

  translate --out FILE.c [--roots FILE] ADDR...
                            traduce in C le funzioni indicate e quelle che chiamano
  candidates [--max N]      funzioni adatte al test differenziale: tutta la chiusura è
                            traducibile, senza import né chiamate indirette
  coverage                  quante funzioni si traducono oggi, e cosa blocca le altre
  census                    quante volte compare ogni istruzione nel codice raggiunto,
                            e quante di quelle il traduttore sa già emettere

Opzione comune: --any-exe   accetta un eseguibile con hash diverso da quello della build Steam";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("errore: {e}");
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
            "SHA-1 {sha}: non è AssassinsCreed_Dx9.exe della build Steam ({AC1_DX9_STEAM_SHA1}).\n\
             Usa --any-exe per analizzarlo comunque."
        )
        .into());
    }
    match cmd.as_str() {
        "info" => info(&img, &sha),
        "rtti" => rtti_stats(&img),
        "discover" => discover_cmd(&img, flag(args, "--strict"), value(args, "--out"))?,
        "translate" => translate_cmd(&img, args)?,
        "coverage" => coverage_cmd(&img),
        "census" => census_cmd(&img),
        "candidates" => candidates_cmd(&img, value(args, "--max").and_then(|v| v.parse().ok()).unwrap_or(500)),
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn info(img: &Image, sha: &str) {
    println!("sha1        {sha}");
    println!("dimensione  {} byte", img.data.len());
    println!("image base  {:#x}   entry {:#x}   timestamp {}", img.base, img.entry, img.timestamp);
    println!("sezioni:");
    for s in &img.sections {
        println!("  {:<8} va={:#010x} vsize={:#09x} rsize={:#09x}", s.name, s.va, s.vsize, s.rsize);
    }
    let mut dlls: BTreeMap<&str, usize> = BTreeMap::new();
    for i in &img.imports {
        *dlls.entry(&i.dll).or_default() += 1;
    }
    println!("import      {} funzioni da {} DLL", img.imports.len(), dlls.len());
    for (dll, n) in dlls {
        println!("  {n:4}  {dll}");
    }
    println!("export      {}", img.exports.len());
    println!("callback TLS {}", img.tls_callbacks.len());
}

fn rtti_stats(img: &Image) {
    let r = rtti::analyze(img);
    let slots: Vec<u32> = r.vtables.iter().flat_map(|v| v.slots.iter().copied()).collect();
    let distinct: HashSet<u32> = slots.iter().copied().collect();
    let classes: HashSet<&str> = r.vtables.iter().map(|v| v.class.as_str()).collect();
    println!("type_info::vftable  {:#x}", r.type_info_vftable);
    println!("type descriptor     {}", r.types.len());
    println!("vtable              {}", r.vtables.len());
    println!("  slot totali       {}", slots.len());
    println!("  funzioni distinte {}", distinct.len());
    println!("  classi con vtable {}", classes.len());
    println!("  secondarie        {}", r.vtables.iter().filter(|v| v.offset != 0).count());
    if let Some(v) = r.vtables.iter().max_by_key(|v| v.slots.len()) {
        println!("  la più grande     {} ({} slot)", rtti::undecorate(&v.class), v.slots.len());
    }
}

fn discover_cmd(img: &Image, strict: bool, out: Option<&str>) -> std::io::Result<()> {
    let t0 = std::time::Instant::now();
    let (d, r) = Discovery::run(img, strict, true);
    let (code, table, int3, unknown) = d.text_breakdown();
    println!("funzioni            {}", d.starts.len());
    println!("istruzioni          {}", d.n_insn);
    println!("jump table          {} (di cui {} con limite dal cmp)", d.tables, d.bounded_tables);
    println!("per origine:");
    let kinds = [Kind::Entry, Kind::Export, Kind::Tls, Kind::Vtable, Kind::Immediate, Kind::DataWord];
    for k in kinds {
        let direct = d.starts.values().filter(|o| o.kind == k && !o.via_call).count();
        let called = d.starts.values().filter(|o| o.kind == k && o.via_call).count();
        if direct + called > 0 {
            println!("  {:<16} {direct:>6}  (+{called} raggiunte per call)", k.label());
        }
    }
    let total = d.own.len() as f64;
    println!(".text ({} byte):", d.own.len());
    for (what, n) in [("codice", code), ("jump table", table), ("int3 non raggiunti", int3), ("non classificato", unknown)] {
        println!("  {what:<20} {n:>9}  {:5.1}%", 100.0 * n as f64 / total);
    }
    println!("scarti:");
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
    println!("slot di vtable non accettati: {missing}");
    println!("tempo               {:.2} s", t0.elapsed().as_secs_f64());

    if let Some(path) = out {
        let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(f, "{{\"functions\": [")?;
        let n = d.starts.len();
        for (i, (addr, o)) in d.starts.iter().enumerate() {
            let label = if o.via_call { format!("{} → call", o.kind.label()) } else { o.kind.label().to_string() };
            writeln!(f, "[{addr}, \"{label}\"]{}", if i + 1 < n { "," } else { "" })?;
        }
        writeln!(f, "]}}")?;
        eprintln!("elenco scritto in {path}");
    }
    Ok(())
}

/// Le partenze certe: un salto verso una di queste è una tail call, non codice interno.
fn hard_starts(d: &Discovery) -> HashSet<u32> {
    d.starts
        .iter()
        .filter(|(_, o)| o.via_call || matches!(o.kind, Kind::Entry | Kind::Export | Kind::Tls | Kind::Vtable))
        .map(|(&a, _)| a)
        .collect()
}

fn translate_cmd(img: &Image, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let out = value(args, "--out").ok_or("manca --out FILE.c")?;
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
        return Err("nessun indirizzo da tradurre".into());
    }
    let (d, _) = Discovery::run(img, false, false);
    let mut hard = hard_starts(&d);
    hard.extend(&roots);
    let is_hard = |a: u32| hard.contains(&a);

    // Chiusura sulle chiamate dirette; le funzioni non traducibili restano fuori.
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
    let mut c = String::from("/* Generato da `acrek translate`: non modificare e non pubblicare. */\n#include \"acrek.h\"\n\n");
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
    c += "const AcrekEntry acrek_functions[] = {\n";
    for a in funcs.keys() {
        c += &format!("    {{{a:#x}u, {}}},\n", translate::fn_name(*a));
    }
    c += &format!("}};\nconst u32 acrek_function_count = {};\n", funcs.len());
    std::fs::write(out, c)?;
    eprintln!("{} funzioni, {insns} istruzioni tradotte in {out}", funcs.len());
    for (a, e) in &failed {
        eprintln!("  non tradotta {a:#010x}: {e}");
    }
    for r in &roots {
        if failed.contains_key(r) {
            return Err(format!("la funzione richiesta {r:#x} non è traducibile").into());
        }
    }
    Ok(())
}

fn coverage_cmd(img: &Image) {
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
                    Err(e) => *blockers.entry(e.split(' ').next().unwrap_or("?").to_string()).or_default() += 1,
                }
            }
            Err(e) => *blockers.entry(format!("[decodifica] {}", e.split(" (").next().unwrap_or("?"))).or_default() += 1,
        }
    }
    let n = d.starts.len();
    println!("funzioni tradotte   {ok} su {n} ({:.1}%)", 100.0 * ok as f64 / n as f64);
    println!("istruzioni          {ok_insns} su {total_insns} ({:.1}%), contate per funzione", 100.0 * ok_insns as f64 / total_insns as f64);
    println!("istruzioni distinte {}", seen.len());
    println!("primo ostacolo delle funzioni non tradotte:");
    let mut b: Vec<_> = blockers.into_iter().collect();
    b.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (why, n) in b.iter().take(40) {
        println!("  {n:>6}  {why}");
    }
    if b.len() > 40 {
        println!("  ... e altri {} motivi", b.len() - 40);
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
    // Chiusura sulle chiamate dirette: piccola, tutta traducibile, senza destinazioni ignote.
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
                // `rdtsc` e `cpuid` danno risultati diversi dall'emulatore: non confrontabili.
                Some(i) if !i.opaque && !i.mnemonics.contains(&Mnemonic::Rdtsc) && !i.mnemonics.contains(&Mnemonic::Cpuid) => {
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
    // Prima qualche funzione per ogni tipo di istruzione, partendo dalle più rare; poi le
    // altre in ordine pseudo-casuale ma ripetibile.
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
    eprintln!("{} funzioni adatte su {} traducibili; scelte {}", good.len(), info.len(), chosen.len());
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
    // (totale, non traducibili) per mnemonico, sulle istruzioni distinte.
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
    println!("istruzioni distinte {total}, di cui non ancora traducibili {missing} ({:.1}%)", 100.0 * missing as f64 / total as f64);
    println!("{:>9} {:>9}  istruzione", "totale", "mancanti");
    let mut rows: Vec<_> = count.into_iter().collect();
    rows.sort_by_key(|(_, c)| std::cmp::Reverse((c.1, c.0)));
    for (m, (n, miss)) in rows {
        println!("{n:>9} {miss:>9}  {m}");
    }
}
