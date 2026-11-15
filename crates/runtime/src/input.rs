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
    eprintln!("[input] dispositivo: {name}");
    0
}

/// Stato immediato: nessun tasto premuto, mouse fermo.
fn dev_get_device_state(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.fill(c.arg(2), c.arg(1), 0);
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
    ("EnumObjects", 4, Some(L(com::ok))),
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
