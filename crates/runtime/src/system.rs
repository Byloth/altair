//! Le DLL piccole e le parti di sistema che il gioco tocca appena: COM, registro, file INI,
//! utente, data e ora, rete.

use crate::win32::{Call, Runtime, Shim, Shim::L};

fn zero(_: &mut Runtime, _: &mut Call) -> u32 {
    0
}
fn one(_: &mut Runtime, _: &mut Call) -> u32 {
    1
}

// --- COM ------------------------------------------------------------------------------------
use crate::com::guid;

/// Nessuna classe COM è registrata: il chiamante riceve REGDB_E_CLASSNOTREG.
fn co_create_instance(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("CoCreateInstance: classe {} interfaccia {} → non registrata (da {:#010x})", guid(rt, c.arg(0)), guid(rt, c.arg(3)), c.ret_addr());
    if c.arg(4) != 0 {
        rt.mem.w32(c.arg(4), 0);
    }
    0x8004_0154
}

// --- registro: esiste una sola chiave, quella che l'installatore di Steam scrive. ------------
// `HKLM\SOFTWARE\Ubisoft\Assassin's Creed` con `Language` e `InstallDir`
// (vedi `installscript.vdf` nella cartella del gioco). La lingua si sceglie con ALTAIR_LANG.
const REG_KEY: u32 = 0x5245_4701;
fn reg_open_key_ex_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let name = rt.mem.cstr(c.arg(1));
    let found = name.trim_end_matches('\\').eq_ignore_ascii_case("SOFTWARE\\Ubisoft\\Assassin's Creed");
    if rt.trace {
        eprintln!("    RegOpenKeyExA(\"{}\"): {}", name, if found { "presente" } else { "assente" });
    }
    if !found {
        return 2; // ERROR_FILE_NOT_FOUND
    }
    rt.mem.w32(c.arg(4), REG_KEY);
    0
}
fn reg_query_value_ex_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let name = if c.arg(1) != 0 { rt.mem.cstr(c.arg(1)) } else { String::new() };
    let value = match name.to_ascii_lowercase().as_str() {
        _ if c.arg(0) != REG_KEY => None,
        "language" => Some(std::env::var("ALTAIR_LANG").unwrap_or_else(|_| "Italian".into())),
        "installdir" => Some("C:\\AC1\\".to_string()),
        _ => None,
    };
    if rt.trace {
        eprintln!("    RegQueryValueExA(\"{}\"): {:?}", name, value);
    }
    let Some(value) = value else { return 2 };
    let need = value.len() as u32 + 1;
    if c.arg(3) != 0 {
        rt.mem.w32(c.arg(3), 1); // REG_SZ
    }
    let (data, size_ptr) = (c.arg(4), c.arg(5));
    let mut status = 0;
    if data != 0 && size_ptr != 0 {
        if rt.mem.r32(size_ptr) >= need {
            rt.mem.write_cstr(data, value.as_bytes());
        } else {
            status = 234; // ERROR_MORE_DATA
        }
    }
    if size_ptr != 0 {
        rt.mem.w32(size_ptr, need);
    }
    status
}

// --- utente e macchina ----------------------------------------------------------------------
fn write_name(rt: &mut Runtime, buf: u32, size_ptr: u32, name: &[u8]) -> u32 {
    if rt.mem.r32(size_ptr) <= name.len() as u32 {
        rt.mem.w32(size_ptr, name.len() as u32 + 1);
        return 0;
    }
    rt.mem.write_cstr(buf, name);
    rt.mem.w32(size_ptr, name.len() as u32 + 1);
    1
}
fn get_user_name_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    write_name(rt, c.arg(0), c.arg(1), b"Player")
}
fn get_computer_name_ex_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    write_name(rt, c.arg(1), c.arg(2), b"ALTAIR")
}
fn get_user_default_lcid(_: &mut Runtime, _: &mut Call) -> u32 {
    0x0409 // en-US
}

// --- data e ora -----------------------------------------------------------------------------
/// SYSTEMTIME dal tempo Unix (UTC).
fn write_system_time(rt: &mut Runtime, p: u32, unix_ms: u64) {
    let secs = unix_ms / 1000;
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // Algoritmo civile di Howard Hinnant.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    let fields = [y as u16, m as u16, ((days + 4).rem_euclid(7)) as u16, d as u16, (rem / 3600) as u16, (rem % 3600 / 60) as u16, (rem % 60) as u16, (unix_ms % 1000) as u16];
    for (i, f) in fields.iter().enumerate() {
        rt.mem.w16(p + 2 * i as u32, *f);
    }
}
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
fn get_system_time(rt: &mut Runtime, c: &mut Call) -> u32 {
    write_system_time(rt, c.arg(0), now_ms());
    0
}
const FILETIME_EPOCH: u64 = 116_444_736_000_000_000;
fn file_time_to_system_time(rt: &mut Runtime, c: &mut Call) -> u32 {
    let ft = rt.mem.r32(c.arg(0)) as u64 | (rt.mem.r32(c.arg(0) + 4) as u64) << 32;
    write_system_time(rt, c.arg(1), ft.saturating_sub(FILETIME_EPOCH) / 10_000);
    1
}
/// Il fuso orario è UTC: ora locale e ora di sistema coincidono.
fn copy_file_time(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = rt.mem.bytes(c.arg(0), 8).to_vec();
    rt.mem.write(c.arg(1), &v);
    1
}
fn get_time_zone_information(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.fill(c.arg(0), 172, 0);
    0 // TIME_ZONE_ID_UNKNOWN
}

// --- file INI veri, nel disco scrivibile del gioco (`run/c/…`) ------------------------------
/// Le righe di un file INI guest; vuoto se non esiste.
fn ini_lines(rt: &Runtime, file: u32) -> (Option<std::path::PathBuf>, Vec<String>) {
    let path = if file != 0 { rt.fs.resolve(&rt.mem.cstr(file)).map(|p| p.0) } else { None };
    let text = path.as_ref().and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
    (path, String::from_utf8_lossy(&text).lines().map(|l| l.trim_end_matches('\r').to_string()).collect())
}
fn section_name(line: &str) -> Option<&str> {
    line.trim().strip_prefix('[')?.strip_suffix(']')
}
/// Indici delle righe di una sezione (intestazione esclusa).
fn section_range(lines: &[String], section: &str) -> Option<std::ops::Range<usize>> {
    let start = lines.iter().position(|l| section_name(l).is_some_and(|n| n.eq_ignore_ascii_case(section)))? + 1;
    let end = lines[start..].iter().position(|l| section_name(l).is_some()).map_or(lines.len(), |i| start + i);
    Some(start..end)
}
fn key_value(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once('=')?;
    (!k.trim_start().starts_with(';')).then(|| (k.trim(), v.trim()))
}
fn ini_get(rt: &Runtime, section: u32, key: u32, file: u32) -> Option<String> {
    if section == 0 || key == 0 {
        return None;
    }
    let (_, lines) = ini_lines(rt, file);
    let (section, key) = (rt.mem.cstr(section), rt.mem.cstr(key));
    let value = section_range(&lines, &section).and_then(|r| lines[r].iter().filter_map(|l| key_value(l)).find(|(k, _)| k.eq_ignore_ascii_case(&key)).map(|(_, v)| v.trim_matches('"').to_string()));
    if rt.trace {
        eprintln!("    ini [{section}] {key} in {} → {value:?}", rt.mem.cstr(file));
    }
    value
}
fn get_private_profile_int_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(v) = ini_get(rt, c.arg(0), c.arg(1), c.arg(3)) else { return c.arg(2) };
    // Come Windows: cifre iniziali, con segno; altrimenti zero.
    let digits: String = v.chars().enumerate().take_while(|(i, ch)| ch.is_ascii_digit() || (*i == 0 && *ch == '-')).map(|(_, ch)| ch).collect();
    digits.parse::<i64>().unwrap_or(0) as u32
}
/// Scrive nel buffer una serie di stringhe chiuse da zero, con uno zero finale in più.
fn write_list(rt: &mut Runtime, buf: u32, size: u32, items: &[String]) -> u32 {
    let mut out = Vec::new();
    for item in items {
        out.extend_from_slice(item.as_bytes());
        out.push(0);
    }
    if size < 2 {
        return 0;
    }
    out.truncate(size as usize - 2);
    let n = out.len() as u32;
    out.extend([0, 0]);
    rt.mem.write(buf, &out);
    n
}
fn get_private_profile_string_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (section, key, default, buf, size, file) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5));
    if section != 0 && key == 0 {
        // Senza chiave: l'elenco delle chiavi della sezione.
        let (_, lines) = ini_lines(rt, file);
        let keys: Vec<String> = section_range(&lines, &rt.mem.cstr(section)).map(|r| lines[r].iter().filter_map(|l| key_value(l)).map(|(k, _)| k.to_string()).collect()).unwrap_or_default();
        return write_list(rt, buf, size, &keys);
    }
    let value = match ini_get(rt, section, key, file) {
        Some(v) => v.into_bytes(),
        None if default != 0 => rt.mem.cstr_bytes(default).to_vec(),
        None => Vec::new(),
    };
    let n = value.len().min(size.saturating_sub(1) as usize);
    if size > 0 {
        rt.mem.write_cstr(buf, &value[..n]);
    }
    n as u32
}
/// `GetPrivateProfileSectionA(sezione, buffer, capienza, file)`: le righe `chiave=valore`.
fn get_private_profile_section_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (_, lines) = ini_lines(rt, c.arg(3));
    let section = if c.arg(0) != 0 { rt.mem.cstr(c.arg(0)) } else { String::new() };
    let items: Vec<String> = section_range(&lines, &section).map(|r| lines[r].iter().filter_map(|l| key_value(l)).map(|(k, v)| format!("{k}={v}")).collect()).unwrap_or_default();
    write_list(rt, c.arg(1), c.arg(2), &items)
}
/// `GetPrivateProfileSectionNamesA(buffer, capienza, file)`.
fn get_private_profile_section_names_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (_, lines) = ini_lines(rt, c.arg(2));
    let names: Vec<String> = lines.iter().filter_map(|l| section_name(l)).map(str::to_string).collect();
    write_list(rt, c.arg(0), c.arg(1), &names)
}
/// `WritePrivateProfileStringA(sezione, chiave, valore, file)`: valore nullo cancella la chiave.
fn write_private_profile_string_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (section, key, value, file) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    if section == 0 || key == 0 {
        return 1; // cancellare un'intera sezione non serve al gioco
    }
    let (Some(path), mut lines) = ini_lines(rt, file) else { return 0 };
    if !rt.fs.resolve(&rt.mem.cstr(file)).is_some_and(|p| p.1) {
        return 0; // cartella del gioco: sola lettura
    }
    let (section, key) = (rt.mem.cstr(section), rt.mem.cstr(key));
    let value = (value != 0).then(|| rt.mem.cstr(value));
    if rt.trace {
        eprintln!("    ini scrive [{section}] {key} = {value:?} in {}", path.display());
    }
    let range = match section_range(&lines, &section) {
        Some(r) => r,
        None => {
            lines.push(format!("[{section}]"));
            lines.len()..lines.len()
        }
    };
    let at = range.clone().find(|&i| key_value(&lines[i]).is_some_and(|(k, _)| k.eq_ignore_ascii_case(&key)));
    match (at, value) {
        (Some(i), Some(v)) => lines[i] = format!("{key}={v}"),
        (Some(i), None) => {
            lines.remove(i);
        }
        (None, Some(v)) => lines.insert(range.end, format!("{key}={v}")),
        (None, None) => {}
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&path, lines.join("\r\n") + "\r\n").is_ok() as u32
}

// --- rete: non disponibile ------------------------------------------------------------------
fn wsa_startup(_: &mut Runtime, _: &mut Call) -> u32 {
    10091 // WSASYSNOTREADY
}
fn socket_error(_: &mut Runtime, _: &mut Call) -> u32 {
    0xffff_ffff
}
fn wsa_get_last_error(_: &mut Runtime, _: &mut Call) -> u32 {
    10050 // WSAENETDOWN
}
fn get_adapters_info(_: &mut Runtime, _: &mut Call) -> u32 {
    232 // ERROR_NO_DATA
}

// --- audio: nessun dispositivo (arriva in Fase 3) -------------------------------------------
fn ds_no_driver(_: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("[audio] DirectSound non disponibile (richiesto da {:#010x})", c.ret_addr());
    0x8878_0078 // DSERR_NODRIVER
}
fn bad_device(_: &mut Runtime, _: &mut Call) -> u32 {
    2 // MMSYSERR_BADDEVICEID
}

// --- numeri casuali di sistema --------------------------------------------------------------
fn crypt_acquire_context_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.w32(c.arg(0), 0x0c0c_0001);
    1
}
fn crypt_gen_random(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (n, buf) = (c.arg(1), c.arg(2));
    // SAFETY: il buffer è memoria guest valida per `n` byte.
    let dst = unsafe { std::slice::from_raw_parts_mut(rt.mem.ptr(buf), n as usize) };
    let ok = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, dst)).is_ok();
    ok as u32
}

fn bswap32(_: &mut Runtime, c: &mut Call) -> u32 {
    c.arg(0).swap_bytes()
}
fn bswap16(_: &mut Runtime, c: &mut Call) -> u32 {
    (c.arg(0) as u16).swap_bytes() as u32
}
fn inet_addr(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("[rete] il gioco chiede l'indirizzo \"{}\": rete non disponibile", rt.mem.cstr(c.arg(0)));
    0xffff_ffff // INADDR_NONE
}
fn gethostbyname(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("[rete] il gioco cerca l'host \"{}\": rete non disponibile", rt.mem.cstr(c.arg(0)));
    0
}

pub static SHIMS: &[(&str, u32, Shim)] = &[
    ("CoInitialize", 1, L(zero)),
    ("CoUninitialize", 0, L(zero)),
    ("CoCreateInstance", 5, L(co_create_instance)),
    ("RegOpenKeyExA", 5, L(reg_open_key_ex_a)),
    ("RegQueryValueExA", 6, L(reg_query_value_ex_a)),
    ("RegCloseKey", 1, L(zero)),
    ("GetUserNameA", 2, L(get_user_name_a)),
    ("GetComputerNameExA", 3, L(get_computer_name_ex_a)),
    ("GetUserDefaultLCID", 0, L(get_user_default_lcid)),
    ("GetSystemTime", 1, L(get_system_time)),
    ("GetLocalTime", 1, L(get_system_time)),
    ("FileTimeToSystemTime", 2, L(file_time_to_system_time)),
    ("FileTimeToLocalFileTime", 2, L(copy_file_time)),
    ("LocalFileTimeToFileTime", 2, L(copy_file_time)),
    ("GetTimeZoneInformation", 1, L(get_time_zone_information)),
    ("GetPrivateProfileIntA", 4, L(get_private_profile_int_a)),
    ("GetPrivateProfileStringA", 6, L(get_private_profile_string_a)),
    ("GetPrivateProfileSectionA", 4, L(get_private_profile_section_a)),
    ("GetPrivateProfileSectionNamesA", 3, L(get_private_profile_section_names_a)),
    ("WritePrivateProfileStringA", 4, L(write_private_profile_string_a)),
    ("ws2_32.dll!#115", 2, L(wsa_startup)),
    ("ws2_32.dll!#116", 0, L(zero)),
    ("ws2_32.dll!#111", 0, L(wsa_get_last_error)),
    ("GetAdaptersInfo", 2, L(get_adapters_info)),
    ("InternetGetCookieW", 4, L(zero)),
    ("InternetSetCookieW", 3, L(zero)),
    ("dsound.dll!#2", 2, L(zero)),
    ("eax.dll!#6", 3, L(ds_no_driver)),
    ("waveOutGetDevCapsA", 3, L(bad_device)),
    ("CryptAcquireContextA", 5, L(crypt_acquire_context_a)),
    ("CryptGenRandom", 3, L(crypt_gen_random)),
    ("CryptReleaseContext", 2, L(one)),
    ("ws2_32.dll!#1", 3, L(socket_error)),
    ("ws2_32.dll!#2", 3, L(socket_error)),
    ("ws2_32.dll!#3", 1, L(socket_error)),
    ("ws2_32.dll!#4", 3, L(socket_error)),
    ("ws2_32.dll!#5", 3, L(socket_error)),
    ("ws2_32.dll!#6", 3, L(socket_error)),
    ("ws2_32.dll!#7", 5, L(socket_error)),
    ("ws2_32.dll!#10", 3, L(socket_error)),
    ("ws2_32.dll!#13", 2, L(socket_error)),
    ("ws2_32.dll!#16", 4, L(socket_error)),
    ("ws2_32.dll!#17", 6, L(socket_error)),
    ("ws2_32.dll!#18", 5, L(socket_error)),
    ("ws2_32.dll!#19", 4, L(socket_error)),
    ("ws2_32.dll!#20", 6, L(socket_error)),
    ("ws2_32.dll!#21", 5, L(socket_error)),
    ("ws2_32.dll!#22", 2, L(socket_error)),
    ("ws2_32.dll!#23", 3, L(socket_error)),
    ("ws2_32.dll!#57", 2, L(socket_error)),
    ("ws2_32.dll!#101", 4, L(socket_error)),
    ("ws2_32.dll!#8", 1, L(bswap32)),
    ("ws2_32.dll!#14", 1, L(bswap32)),
    ("ws2_32.dll!#9", 1, L(bswap16)),
    ("ws2_32.dll!#15", 1, L(bswap16)),
    ("ws2_32.dll!#11", 1, L(inet_addr)),
    ("ws2_32.dll!#12", 1, L(zero)),
    ("ws2_32.dll!#51", 3, L(zero)),
    ("ws2_32.dll!#52", 1, L(gethostbyname)),
    ("ws2_32.dll!#151", 2, L(zero)),
];
