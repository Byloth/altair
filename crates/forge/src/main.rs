//! `altair-forge`: ispeziona gli archivi `.forge`. L'output contiene nomi di file del gioco:
//! se lo salvi, mettilo in `private/`.

use altair_forge::{crc32_update, Forge};

const USAGE: &str = "uso: altair-forge list <archivio.forge>      voci: id, dimensione, nome
     altair-forge digest <archivio.forge>    per voce: id, sezioni, byte decompressi, CRC-32";

fn run() -> altair_forge::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [cmd, path] = &args[..] else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let mut forge = Forge::open(path)?;
    let entries = forge.entries.clone();
    let out = std::io::stdout();
    let mut out = std::io::BufWriter::new(out.lock());
    use std::io::Write;
    for e in &entries {
        match cmd.as_str() {
            "list" => {
                let name = forge.name(e)?.unwrap_or_else(|| "None".into());
                writeln!(out, "{:#010x} {:>10} {name}", e.id, e.size)?;
            }
            "digest" => {
                // Stessa definizione di `forge.py digest`: CRC dei dati decompressi di tutte le
                // sezioni, in fila; per una voce senza sezioni, dei dati grezzi.
                let sections = forge.sections(e)?;
                let (mut crc, mut len) = (0u32, 0usize);
                if sections.is_empty() {
                    let raw = forge.raw(e)?;
                    crc = crc32_update(crc, &raw);
                    len = raw.len();
                }
                for s in &sections {
                    let data = forge.decompress(s)?;
                    crc = crc32_update(crc, &data);
                    len += data.len();
                }
                writeln!(out, "{:#010x} {} {len} {crc:08x}", e.id, sections.len())?;
            }
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("errore: {e}");
        std::process::exit(1);
    }
}
