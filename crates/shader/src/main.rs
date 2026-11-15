//! `altair-shaderc`: translates the D3D9 shaders extracted from the game and measures their coverage.
//!
//!   altair-shaderc <folder>              translates and validates all the `.bin` files, prints the summary
//!   altair-shaderc <file.bin>            prints the WGSL of one shader
//!   altair-shaderc <folder> --out DIR    also saves the `.wgsl` files (game content: in private/)

use altair_shader::{op_name, parse, translate, validate, Stage};
use std::collections::BTreeMap;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(target) = args.first() else {
        eprintln!("usage: altair-shaderc <folder | file.bin> [--out DIR]");
        std::process::exit(1);
    };
    let path = Path::new(target);
    if path.is_file() {
        let code = std::fs::read(path).expect("read");
        match parse(&code).and_then(|s| translate(&s)) {
            Ok(w) => {
                println!("{w}");
                if let Err(e) = validate(&w) {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(2);
            }
        }
        return;
    }
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1));
    if let Some(o) = out {
        std::fs::create_dir_all(o).expect("output folder");
    }
    let mut files: Vec<_> = std::fs::read_dir(path).expect("folder").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "bin")).collect();
    files.sort();
    let (mut ok, mut total) = ([0usize; 2], [0usize; 2]);
    let mut failures: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut opcodes: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    let mut invalid = 0;
    for f in &files {
        let code = std::fs::read(f).expect("read");
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let sh = match parse(&code) {
            Ok(s) => s,
            Err(e) => {
                let entry = failures.entry(format!("[read] {e}")).or_insert((0, name.clone()));
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
                        std::fs::write(Path::new(o).join(name.replace(".bin", ".wgsl")), w).expect("write");
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
                let key = e.split(" (instruction").next().unwrap_or(&e).to_string();
                let entry = failures.entry(key).or_insert((0, name.clone()));
                entry.0 += 1;
            }
        }
    }
    println!("vertex shaders: {} translated and valid out of {}", ok[0], total[0]);
    println!("pixel shaders:  {} translated and valid out of {}", ok[1], total[1]);
    println!("WGSL produced but rejected by naga: {invalid}");
    if !failures.is_empty() {
        println!("failure reasons (shaders, example):");
        let mut f: Vec<_> = failures.into_iter().collect();
        f.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
        for (why, (n, example)) in f {
            println!("  {n:>5}  {why}  [{example}]");
        }
    }
    println!("instructions used (occurrences, shaders that use them):");
    let mut o: Vec<_> = opcodes.into_iter().collect();
    o.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    println!("  {}", o.iter().map(|(op, (n, s))| format!("{} {n}/{s}", op_name(*op))).collect::<Vec<_>>().join(", "));
}
