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

// --- registro: non c'è nulla. Il gioco usa i valori predefiniti. ----------------------------
fn reg_open_key_ex_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    if rt.trace {
        eprintln!("    RegOpenKeyExA(\"{}\"): assente", rt.mem.cstr(c.arg(1)));
    }
    2 // ERROR_FILE_NOT_FOUND
}
fn file_not_found(_: &mut Runtime, _: &mut Call) -> u32 {
    2
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

// --- file INI: nessun file, sempre il valore predefinito ------------------------------------
fn get_private_profile_int_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    if rt.trace {
        eprintln!("    ini [{}] {} in {}", rt.mem.cstr(c.arg(0)), rt.mem.cstr(c.arg(1)), rt.mem.cstr(c.arg(3)));
    }
    c.arg(2)
}
fn get_private_profile_string_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (default, buf, size) = (c.arg(2), c.arg(3), c.arg(4));
    if rt.trace {
        eprintln!("    ini [{}] {} in {}", rt.mem.cstr(c.arg(0)), rt.mem.cstr(c.arg(1)), rt.mem.cstr(c.arg(5)));
    }
    let d = if default != 0 { rt.mem.cstr_bytes(default).to_vec() } else { Vec::new() };
    let n = d.len().min(size.saturating_sub(1) as usize);
    if size > 0 {
        rt.mem.write_cstr(buf, &d[..n]);
    }
    n as u32
}
fn empty_section(rt: &mut Runtime, c: &mut Call) -> u32 {
    // GetPrivateProfileSectionA(app, buf, size, file) / SectionNamesA(buf, size, file)
    let (buf, size) = if c.arg(3) != 0 && c.arg(2) < 0x10000 { (c.arg(1), c.arg(2)) } else { (c.arg(0), c.arg(1)) };
    if size >= 2 {
        rt.mem.w16(buf, 0);
    }
    0
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

// --- video Bink: nessun decodificatore, i filmati vengono saltati (Fase 3) -------------------
fn bink_open(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("[video] {} saltato: Bink non disponibile", rt.mem.cstr(c.arg(0)));
    0
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

pub static SHIMS: &[(&str, u32, Shim)] = &[
    ("CoInitialize", 1, L(zero)),
    ("CoUninitialize", 0, L(zero)),
    ("CoCreateInstance", 5, L(co_create_instance)),
    ("RegOpenKeyExA", 5, L(reg_open_key_ex_a)),
    ("RegQueryValueExA", 6, L(file_not_found)),
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
    ("GetPrivateProfileSectionA", 4, L(empty_section)),
    ("GetPrivateProfileSectionNamesA", 3, L(empty_section)),
    ("WritePrivateProfileStringA", 4, L(one)),
    ("ws2_32.dll!#115", 2, L(wsa_startup)),
    ("ws2_32.dll!#116", 0, L(zero)),
    ("ws2_32.dll!#111", 0, L(wsa_get_last_error)),
    ("ws2_32.dll!#23", 3, L(socket_error)),
    ("GetAdaptersInfo", 2, L(get_adapters_info)),
    ("InternetGetCookieW", 4, L(zero)),
    ("InternetSetCookieW", 3, L(zero)),
    ("dsound.dll!#2", 2, L(zero)),
    ("eax.dll!#6", 3, L(ds_no_driver)),
    ("waveOutGetDevCapsA", 3, L(bad_device)),
    ("_BinkSetMemory@8", 2, L(zero)),
    ("_BinkSetSoundSystem@8", 2, L(one)),
    ("_BinkOpenDirectSound@4", 1, L(one)),
    ("_BinkSetSoundTrack@8", 2, L(zero)),
    ("_BinkOpen@8", 2, L(bink_open)),
    ("_BinkClose@4", 1, L(zero)),
    ("_BinkCopyToBuffer@28", 7, L(zero)),
    ("_BinkDoFrame@4", 1, L(zero)),
    ("_BinkNextFrame@4", 1, L(zero)),
    ("_BinkPause@8", 2, L(zero)),
    ("_BinkSetVolume@12", 3, L(zero)),
    ("_BinkShouldSkip@4", 1, L(zero)),
    ("_BinkWait@4", 1, L(zero)),
    ("_BinkGetFrameBuffersInfo@8", 2, L(zero)),
    ("CryptAcquireContextA", 5, L(crypt_acquire_context_a)),
    ("CryptGenRandom", 3, L(crypt_gen_random)),
    ("CryptReleaseContext", 2, L(one)),
];
