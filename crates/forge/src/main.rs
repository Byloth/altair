//! `altair-forge`: inspects `.forge` archives. The output contains names of game files:
//! if you save it, put it in `private/`.

use altair_forge::{crc32_update, Forge};

const USAGE: &str = "usage: altair-forge list <archive.forge>      entries: id, size, name
       altair-forge digest <archive.forge>    per entry: id, sections, decompressed bytes, CRC-32";

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
                // Same definition as `forge.py digest`: CRC of the decompressed data of all the
                // sections, one after another; for an entry without sections, of the raw data.
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
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
