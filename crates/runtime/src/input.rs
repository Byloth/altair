//! DirectInput 8 e XInput senza dispositivi veri: una tastiera e un mouse che non vengono mai
//! toccati, nessun joystick, nessun controller. L'input vero arriva in Fase 3.

use crate::com::{self, Method};
use crate::win32::{Call, Runtime, Shim, Shim::L};

pub fn direct_input8_create(rt: &mut Runtime, c: &mut Call) -> u32 {
    let obj = com::create(rt, "IDirectInput8", DINPUT, 0);
    rt.mem.w32(c.arg(3), obj);
    eprintln!("[input] DirectInput creato (interfaccia {})", com::guid(rt, c.arg(2)));
    0
}

fn di_create_device(rt: &mut Runtime, c: &mut Call) -> u32 {
    let which = com::guid(rt, c.arg(1));
    let name = match which.as_str() {
        "6F1D2B61-D5A0-11CF-BFC7-444553540000" => "tastiera",
        "6F1D2B60-D5A0-11CF-BFC7-444553540000" => "mouse",
        _ => {
            eprintln!("[input] CreateDevice di un dispositivo sconosciuto {which}: rifiutato");
            return 0x8004_0154; // DIERR_DEVICENOTREG
        }
    };
    let obj = com::create(rt, "IDirectInputDevice8", DEVICE, 0);
    rt.mem.w32(c.arg(2), obj);
    if name == "tastiera" {
        rt.keyboard = obj;
        rt.key_script = parse_keys();
    }
    if name == "mouse" {
        rt.mouse = obj;
    }
    eprintln!("[input] dispositivo: {name}");
    0
}

/// `ALTAIR_KEYS="900:28,1200:30:5"`: al fotogramma 900 preme il tasto 28 (codici DIK di
/// DirectInput: 28 Invio, 1 Esc, 57 Spazio, 200/208/203/205 frecce) per 10 fotogrammi, o per
/// quanti indicati dopo il secondo `:`.
fn parse_keys() -> Vec<(u64, u64, u8)> {
    let Ok(spec) = std::env::var("ALTAIR_KEYS") else { return Vec::new() };
    spec.split(',')
        .filter_map(|item| {
            let mut p = item.split(':').map(|v| v.trim().parse::<u64>().ok());
            let (frame, key) = (p.next()??, p.next()??);
            let hold = p.next().flatten().unwrap_or(10);
            Some((frame, frame + hold, key as u8))
        })
        .collect()
}

/// Stato immediato: i tasti del copione per la tastiera, altrimenti nulla; mouse fermo.
fn dev_get_device_state(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.fill(c.arg(2), c.arg(1), 0);
    if c.arg(0) == rt.keyboard && c.arg(1) >= 256 {
        let frame = rt.d3d.frames;
        for &(from, to, key) in &rt.key_script {
            if (from..to).contains(&frame) {
                rt.mem.w8(c.arg(2) + key as u32, 0x80);
            }
        }
        let now = std::time::Instant::now();
        rt.key_timed.retain(|(until, _)| now < *until);
        for &(_, key) in &rt.key_timed {
            rt.mem.w8(c.arg(2) + key as u32, 0x80);
        }
        // Tasti veri (finestra) o simulati dal canale di comando: stessa tabella.
        let keys = crate::window::state().keys;
        for (i, k) in keys.iter().enumerate().filter(|(_, k)| **k != 0) {
            rt.mem.w8(c.arg(2) + i as u32, *k);
        }
        if crate::window::log_enabled() {
            let down: Vec<u32> = (0..256).filter(|i| rt.mem.r8(c.arg(2) + i) != 0).collect();
            if down != rt.input_seen.0 {
                eprintln!("[input] DirectInput tastiera: premuti {down:?}");
                rt.input_seen.0 = down;
            }
        }
    }
    // DIMOUSESTATE (16 byte) o DIMOUSESTATE2 (20): spostamento dall'ultima lettura e pulsanti.
    if c.arg(0) == rt.mouse && c.arg(1) >= 16 {
        let mut s = crate::window::state();
        let (dx, dy, dz) = std::mem::take(&mut s.motion);
        for (i, v) in [dx, dy, dz].iter().enumerate() {
            rt.mem.w32(c.arg(2) + 4 * i as u32, *v as u32);
        }
        for i in 0..(c.arg(1) - 12).min(8) {
            rt.mem.w8(c.arg(2) + 12 + i, if s.buttons[i as usize] { 0x80 } else { 0 });
        }
        if crate::window::log_enabled() {
            let now = (s.buttons, (dx, dy, dz) != (0, 0, 0));
            if now != rt.input_seen.1 {
                eprintln!("[input] DirectInput mouse: spostamento ({dx}, {dy}, {dz}), pulsanti {:?}", s.buttons);
                rt.input_seen.1 = now;
            }
        }
    }
    0
}
/// Nome di un tasto per codice DIK, come lo mostrerebbe una tastiera italiana.
fn key_name(dik: u32) -> Option<String> {
    const LETTERS: [(u32, &str); 4] = [(16, "QWERTYUIOP"), (30, "ASDFGHJKL"), (44, "ZXCVBNM"), (2, "1234567890")];
    for (first, row) in LETTERS {
        if (first..first + row.len() as u32).contains(&dik) {
            return Some(row[(dik - first) as usize..][..1].to_string());
        }
    }
    let name = match dik {
        1 => "Esc", 14 => "Backspace", 15 => "Tab", 28 => "Invio", 29 => "Ctrl sinistro", 42 => "Maiusc sinistro",
        54 => "Maiusc destro", 56 => "Alt", 57 => "Spazio", 58 => "Bloc Maiusc", 157 => "Ctrl destro", 184 => "Alt Gr",
        200 => "Freccia su", 208 => "Freccia giù", 203 => "Freccia sinistra", 205 => "Freccia destra",
        199 => "Home", 207 => "Fine", 201 => "Pag su", 209 => "Pag giù", 210 => "Ins", 211 => "Canc",
        12 => "'", 13 => "ì", 26 => "è", 27 => "+", 39 => "ò", 40 => "à", 41 => "\\", 43 => "ù", 51 => ",", 52 => ".", 53 => "-",
        59..=68 => return Some(format!("F{}", dik - 58)),
        87 => "F11", 88 => "F12",
        71..=73 => return Some(format!("Tn {}", dik - 64)),
        75..=77 => return Some(format!("Tn {}", dik - 71)),
        79..=81 => return Some(format!("Tn {}", dik - 78)),
        82 => "Tn 0", 156 => "Tn Invio",
        _ => return None,
    };
    Some(name.to_string())
}

/// `EnumObjects(callback, contesto, flag)`: per il mouse tre assi relativi e otto pulsanti
/// (come `c_dfDIMouse2`), per la tastiera i tasti con il loro nome. La callback è codice del
/// gioco (`0x009091C0`): copia il nome di ogni pulsante e di ogni tasto in una tabella, da cui
/// prende ciò che scrive nei messaggi ("Premi senza rilasciare (Mouse destro)").
///
/// Il gioco crea `IDirectInput8W`: la struttura è `DIDEVICEOBJECTINSTANCEW`, con il nome in
/// UTF-16.
fn dev_enum_objects(c: &mut Call) -> u32 {
    let (dev, callback, context) = (c.arg(0), c.arg(1), c.arg(2));
    let (mouse, keyboard) = crate::win32::with_runtime(|rt| (dev == rt.mouse, dev == rt.keyboard));
    if callback == 0 || !(mouse || keyboard) {
        return 0;
    }
    const AXIS: [u8; 16] = [0xe0, 0x02, 0x6d, 0xa3, 0xf3, 0xc9, 0xcf, 0x11, 0xbf, 0xc7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00];
    const KEY: [u8; 16] = [0x20, 0x82, 0x72, 0x55, 0x3c, 0xd3, 0xcf, 0x11, 0xbf, 0xc7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00];
    let mut objects: Vec<([u8; 16], u32, u32, String)> = Vec::new();
    if mouse {
        for (i, name) in ["Asse X", "Asse Y", "Rotella"].iter().enumerate() {
            let mut guid = AXIS;
            guid[0] += i as u8; // GUID_XAxis, GUID_YAxis, GUID_ZAxis
            objects.push((guid, 4 * i as u32, 0x01 | (i as u32) << 8, name.to_string())); // DIDFT_RELAXIS
        }
        for i in 0..8u32 {
            let mut guid = AXIS;
            guid[0] = 0xf0; // GUID_Button
            let name = match i {
                0 => "Mouse sinistro".to_string(),
                1 => "Mouse destro".to_string(),
                2 => "Mouse centrale".to_string(),
                _ => format!("Mouse {}", i + 1),
            };
            objects.push((guid, 12 + i, 0x04 | (3 + i) << 8, name)); // DIDFT_PSHBUTTON
        }
    } else {
        objects.extend((0..256).filter_map(|dik| key_name(dik).map(|name| (KEY, dik, 0x04 | dik << 8, name))));
    }
    const SIZE: u32 = 0x240; // DIDEVICEOBJECTINSTANCEW
    let item = crate::win32::with_runtime(|rt| rt.halloc(SIZE, true));
    for (guid, offset, kind, name) in objects {
        // dwSize, guidType, dwOfs, dwType, dwFlags, tszName[260] in UTF-16, …
        c.mem.fill(item, SIZE, 0);
        c.mem.w32(item, SIZE);
        c.mem.write(item + 4, &guid);
        c.mem.w32(item + 20, offset);
        c.mem.w32(item + 24, kind);
        for (i, unit) in name.encode_utf16().take(259).enumerate() {
            c.mem.w16(item + 32 + 2 * i as u32, unit);
        }
        if crate::win32::call_guest(c, callback, &[item, context]) == 0 {
            break; // DIENUM_STOP
        }
    }
    crate::win32::with_runtime(|rt| rt.hfree(item));
    0
}

/// Dati bufferizzati: nessun evento.
fn dev_get_device_data(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.w32(c.arg(3), 0);
    0
}
fn dev_get_capabilities(rt: &mut Runtime, c: &mut Call) -> u32 {
    let p = c.arg(1);
    let size = rt.mem.r32(p).min(44);
    rt.mem.fill(p + 4, size.saturating_sub(4), 0);
    rt.mem.w32(p + 4, 1 | 4); // DIDC_ATTACHED | DIDC_EMULATED
    0
}
fn unsupported(_: &mut Runtime, _: &mut Call) -> u32 {
    0x8000_4001 // E_NOTIMPL: per esempio gli effetti di vibrazione
}

static DINPUT: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("CreateDevice", 4, Some(L(di_create_device))),
    ("EnumDevices", 5, Some(L(com::ok))), // nessun dispositivo da elencare
    ("GetDeviceStatus", 2, Some(L(com::ok))),
    ("RunControlPanel", 3, None),
    ("Initialize", 3, Some(L(com::ok))),
    ("FindDevice", 4, None),
    ("EnumDevicesBySemantics", 6, None),
    ("ConfigureDevices", 5, None),
];

static DEVICE: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetCapabilities", 2, Some(L(dev_get_capabilities))),
    ("EnumObjects", 4, Some(Shim::F(dev_enum_objects))),
    ("GetProperty", 3, None),
    ("SetProperty", 3, Some(L(com::ok))),
    ("Acquire", 1, Some(L(com::ok))),
    ("Unacquire", 1, Some(L(com::ok))),
    ("GetDeviceState", 3, Some(L(dev_get_device_state))),
    ("GetDeviceData", 5, Some(L(dev_get_device_data))),
    ("SetDataFormat", 2, Some(L(com::ok))),
    ("SetEventNotification", 2, Some(L(com::ok))),
    ("SetCooperativeLevel", 3, Some(L(com::ok))),
    ("GetObjectInfo", 4, None),
    ("GetDeviceInfo", 2, None),
    ("RunControlPanel", 3, None),
    ("Initialize", 4, Some(L(com::ok))),
    ("CreateEffect", 5, Some(L(unsupported))),
    ("EnumEffects", 4, Some(L(com::ok))),
    ("GetEffectInfo", 3, None),
    ("GetForceFeedbackState", 2, None),
    ("SendForceFeedbackCommand", 2, Some(L(com::ok))),
    ("EnumCreatedEffectObjects", 4, Some(L(com::ok))),
    ("Escape", 2, None),
    ("Poll", 1, Some(L(com::ok))),
    ("SendDeviceData", 5, None),
    ("EnumEffectsInFile", 5, None),
    ("WriteEffectToFile", 5, None),
    ("BuildActionMap", 4, None),
    ("SetActionMap", 4, None),
    ("GetImageInfo", 2, None),
];

fn not_connected(_: &mut Runtime, _: &mut Call) -> u32 {
    1167 // ERROR_DEVICE_NOT_CONNECTED
}

pub static SHIMS: &[(&str, u32, Shim)] = &[
    ("DirectInput8Create", 5, L(direct_input8_create)),
    ("xinput1_3.dll!#2", 2, L(not_connected)),
    ("xinput1_3.dll!#3", 2, L(not_connected)),
    ("xinput1_3.dll!#4", 3, L(not_connected)),
];
