//! `acrek`: strumenti di analisi e ricompilazione per Assassin's Creed (2007), build Steam DX9.

mod discover;
mod rtti;

use acrek_pe::{Image, AC1_DX9_STEAM_SHA1};
use discover::{Discovery, Kind};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "\
uso: acrek <comando> <AssassinsCreed_Dx9.exe> [opzioni]

  info                      hash, sezioni, import, export
  rtti                      conteggi di type descriptor e vtable
  discover [--strict] [--out FILE.json]
                            scoperta delle funzioni; con --out salva l'elenco
                            (contiene solo indirizzi, ma va comunque in private/)

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
