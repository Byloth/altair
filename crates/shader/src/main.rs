//! `altair-shaderc`: traduce gli shader D3D9 estratti dal gioco e ne misura la copertura.
//!
//!   altair-shaderc <cartella>            traduce e valida tutti i `.bin`, stampa il riepilogo
//!   altair-shaderc <file.bin>            stampa il WGSL di uno shader
//!   altair-shaderc <cartella> --out DIR  salva anche i `.wgsl` (contenuto del gioco: in private/)

use altair_shader::{op_name, parse, translate, validate, Stage};
use std::collections::BTreeMap;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(target) = args.first() else {
        eprintln!("uso: altair-shaderc <cartella | file.bin> [--out DIR]");
        std::process::exit(1);
    };
    let path = Path::new(target);
    if path.is_file() {
        let code = std::fs::read(path).expect("lettura");
        match parse(&code).and_then(|s| translate(&s)) {
            Ok(w) => {
                println!("{w}");
                if let Err(e) = validate(&w) {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            }
            Err(e) => {
                eprintln!("errore: {e}");
                std::process::exit(2);
            }
        }
        return;
    }
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1));
    if let Some(o) = out {
        std::fs::create_dir_all(o).expect("cartella di uscita");
    }
    let mut files: Vec<_> = std::fs::read_dir(path).expect("cartella").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "bin")).collect();
    files.sort();
    let (mut ok, mut total) = ([0usize; 2], [0usize; 2]);
    let mut failures: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut opcodes: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    let mut invalid = 0;
    for f in &files {
        let code = std::fs::read(f).expect("lettura");
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let sh = match parse(&code) {
            Ok(s) => s,
            Err(e) => {
                let entry = failures.entry(format!("[lettura] {e}")).or_insert((0, name.clone()));
                entry.0 += 1;
                continue;
            }
        };
        let k = (sh.stage == Stage::Pixel) as usize;
        total[k] += 1;
        for (op, n) in &sh.opcodes {
            let e = opcodes.entry(*op).or_default();
            e.0 += n;
            e.1 += 1;
        }
        match translate(&sh) {
            Ok(w) => match validate(&w) {
                Ok(()) => {
                    ok[k] += 1;
                    if let Some(o) = out {
                        std::fs::write(Path::new(o).join(name.replace(".bin", ".wgsl")), w).expect("scrittura");
                    }
                }
                Err(e) => {
                    invalid += 1;
                    let first = e.lines().find(|l| l.contains("error")).unwrap_or(e.lines().next().unwrap_or("?")).to_string();
                    let entry = failures.entry(format!("[naga] {}", first.chars().take(110).collect::<String>())).or_insert((0, name.clone()));
                    entry.0 += 1;
                }
            },
            Err(e) => {
                let key = e.split(" (istruzione").next().unwrap_or(&e).to_string();
                let entry = failures.entry(key).or_insert((0, name.clone()));
                entry.0 += 1;
            }
        }
    }
    println!("vertex shader: {} tradotti e validi su {}", ok[0], total[0]);
    println!("pixel shader:  {} tradotti e validi su {}", ok[1], total[1]);
    println!("WGSL prodotto ma rifiutato da naga: {invalid}");
    if !failures.is_empty() {
        println!("motivi dei fallimenti (shader, esempio):");
        let mut f: Vec<_> = failures.into_iter().collect();
        f.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
        for (why, (n, example)) in f {
            println!("  {n:>5}  {why}  [{example}]");
        }
    }
    println!("istruzioni usate (occorrenze, shader che le usano):");
    let mut o: Vec<_> = opcodes.into_iter().collect();
    o.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    println!("  {}", o.iter().map(|(op, (n, s))| format!("{} {n}/{s}", op_name(*op))).collect::<Vec<_>>().join(", "));
}
