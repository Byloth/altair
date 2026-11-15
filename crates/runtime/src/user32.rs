//! USER32 senza schermo: una finestra che esiste solo sulla carta. Il gioco la crea, ne legge
//! le dimensioni e non riceve mai messaggi. La finestra vera arriva con la grafica (Fase 3).

use crate::win32::{Call, Runtime, Shim, Shim::L};

const HWND: u32 = 0x0001_0100;
const DESKTOP: u32 = 0x0001_0010;
pub const SCREEN: (u32, u32) = (1280, 720);

fn zero(_: &mut Runtime, _: &mut Call) -> u32 {
    0
}
fn one(_: &mut Runtime, _: &mut Call) -> u32 {
    1
}
fn fake_handle(_: &mut Runtime, _: &mut Call) -> u32 {
    0x0005_0000
}
fn get_system_metrics(_: &mut Runtime, c: &mut Call) -> u32 {
    match c.arg(0) {
        0 => SCREEN.0, // SM_CXSCREEN
        1 => SCREEN.1, // SM_CYSCREEN
        _ => 0,
    }
}
fn register_class_ex_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    // WNDCLASSEXA: lpfnWndProc a +8, lpszClassName a +40.
    let p = c.arg(0);
    rt.wndproc = rt.mem.r32(p + 8);
    eprintln!("[finestra] classe \"{}\", procedura {:#010x}", rt.mem.cstr(rt.mem.r32(p + 40)), rt.wndproc);
    0xc001
}
fn create_window_ex_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("[finestra] CreateWindowExA \"{}\" {}x{}", rt.mem.cstr(c.arg(2)), c.arg(6) as i32, c.arg(7) as i32);
    rt.window_thread = rt.mem.r32(c.teb() + 0x24);
    rt.text_script = parse_text();
    HWND
}
fn write_rect(rt: &mut Runtime, p: u32) -> u32 {
    rt.mem.w32(p, 0);
    rt.mem.w32(p + 4, 0);
    rt.mem.w32(p + 8, SCREEN.0);
    rt.mem.w32(p + 12, SCREEN.1);
    1
}
fn get_rect(rt: &mut Runtime, c: &mut Call) -> u32 {
    write_rect(rt, c.arg(1))
}
fn get_cursor_pos(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (x, y) = if crate::window::enabled() { crate::window::state().cursor } else { (SCREEN.0 / 2, SCREEN.1 / 2) };
    rt.mem.w32(c.arg(0), x);
    rt.mem.w32(c.arg(0) + 4, y);
    1
}
fn get_keyboard_state(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.fill(c.arg(0), 256, 0);
    1
}
fn get_desktop_window(_: &mut Runtime, _: &mut Call) -> u32 {
    DESKTOP
}
fn message_box_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("\n[MessageBox] {}\n  {}", rt.mem.cstr(c.arg(2)), rt.mem.cstr(c.arg(1)).replace('\n', "\n  "));
    1 // IDOK
}
fn post_quit_message(_: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("[finestra] PostQuitMessage({})", c.arg(0));
    0
}

/// `ALTAIR_TEXT="2000:Matteo"`: dal fotogramma 2000 batte il testo, un carattere ogni 12
/// fotogrammi, come messaggi della finestra (`WM_KEYDOWN`, `WM_CHAR`, `WM_KEYUP`). Più testi
/// si separano con `|`.
pub fn parse_text() -> Vec<(u64, char)> {
    let Ok(spec) = std::env::var("ALTAIR_TEXT") else { return Vec::new() };
    let mut out = Vec::new();
    for item in spec.split('|') {
        if let Some((frame, text)) = item.split_once(':') {
            if let Ok(f) = frame.trim().parse::<u64>() {
                out.extend(text.chars().enumerate().map(|(i, ch)| (f + 12 * i as u64, ch)));
            }
        }
    }
    out
}

/// Il gioco svuota la coda dei messaggi a ogni fotogramma, dal thread che possiede la finestra.
fn peek_message_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    if rt.wndproc == 0 || rt.mem.r32(c.teb() + 0x24) != rt.window_thread {
        return 0;
    }
    let frame = rt.d3d.frames;
    if frame != rt.text_frame {
        rt.text_frame = frame;
        // Le frecce del copione arrivano anche come messaggi: gli elenchi dei menu le leggono da lì.
        for &(from, to, dik) in &rt.key_script {
            let vk = match dik {
                200 => 38,
                208 => 40,
                203 => 37,
                205 => 39,
                _ => continue,
            };
            if from == frame {
                rt.messages.push_back((0x100, vk, 0x0100_0001));
            } else if to == frame {
                rt.messages.push_back((0x101, vk, 0xc100_0001));
            }
        }
        let due: Vec<char> = rt.text_script.iter().filter(|(f, _)| *f == frame).map(|(_, ch)| *ch).collect();
        for ch in due {
            let vk = ch.to_ascii_uppercase() as u32;
            // Niente WM_CHAR: la procedura della finestra ricava il carattere da sé, con ToUnicode.
            rt.messages.extend([(0x100, vk, 1), (0x101, vk, 0xc000_0001)]);
            rt.typed.push_back((vk, ch));
        }
    }
    // Tastiera e mouse veri, dalla finestra.
    if crate::window::enabled() {
        use crate::window::Event;
        let events: Vec<Event> = crate::window::state().events.drain(..).collect();
        for e in events {
            match e {
                Event::Key { vk, scan, down: true, ch } => {
                    rt.messages.push_back((0x100, vk, 1 | scan << 16));
                    if let Some(ch) = ch {
                        rt.typed.push_back((vk, ch));
                        if rt.typed.len() > 32 {
                            rt.typed.pop_front();
                        }
                    }
                }
                Event::Key { vk, scan, down: false, .. } => rt.messages.push_back((0x101, vk, 0xc000_0001 | scan << 16)),
                Event::MouseMove(x, y) => rt.messages.push_back((0x200, 0, x | y << 16)),
                Event::Button { index, down } => {
                    let (x, y) = crate::window::state().cursor;
                    // WM_LBUTTONDOWN 0x201, WM_RBUTTONDOWN 0x204, WM_MBUTTONDOWN 0x207; il rilascio è il successivo.
                    rt.messages.push_back((0x201 + 3 * index as u32 + !down as u32, 0, x | y << 16));
                }
            }
        }
    }
    let Some((msg, wparam, lparam)) = rt.messages.pop_front() else { return 0 };
    let p = c.arg(0);
    for (i, v) in [HWND, msg, wparam, lparam, 0, 0, 0].iter().enumerate() {
        rt.mem.w32(p + 4 * i as u32, *v);
    }
    1
}

/// `ToUnicode(vk, scan, stato, buffer, capienza, flag)`: il carattere del tasto premuto. Qui i
/// tasti arrivano solo dal copione, quindi il carattere è quello in attesa.
fn to_unicode(rt: &mut Runtime, c: &mut Call) -> u32 {
    let vk = c.arg(0);
    // Il carattere in attesa per questo tasto; quelli più vecchi, mai chiesti, si buttano.
    let Some(i) = rt.typed.iter().position(|(v, _)| *v == vk) else { return 0 };
    let (_, ch) = rt.typed.drain(..=i).last().unwrap();
    if c.arg(4) < 1 {
        return 0;
    }
    rt.mem.w16(c.arg(3), ch as u16);
    1
}

/// Consegna un messaggio alla procedura della finestra, che è codice del gioco.
pub fn dispatch_message_a(c: &mut Call) -> u32 {
    let p = c.arg(0);
    let args = [c.mem.r32(p), c.mem.r32(p + 4), c.mem.r32(p + 8), c.mem.r32(p + 12)];
    let wndproc = crate::win32::with_runtime(|rt| rt.wndproc);
    if wndproc == 0 {
        return 0;
    }
    crate::win32::call_guest(c, wndproc, &args)
}

/// Un solo monitor, grande quanto lo schermo dichiarato. La callback è codice guest.
pub fn enum_display_monitors(c: &mut Call) -> u32 {
    let rect = c.ctx.esp - 0x40;
    for (i, v) in [0, 0, SCREEN.0, SCREEN.1].iter().enumerate() {
        c.mem.w32(rect + 4 * i as u32, *v);
    }
    let (callback, data) = (c.arg(2), c.arg(3));
    crate::win32::call_guest(c, callback, &[0x0001_0001, 0, rect, data]);
    1
}

pub static SHIMS: &[(&str, u32, Shim)] = &[
    ("SetWindowsHookExA", 4, L(fake_handle)),
    ("UnhookWindowsHookEx", 1, L(one)),
    ("CallNextHookEx", 4, L(zero)),
    ("GetSystemMetrics", 1, L(get_system_metrics)),
    ("SystemParametersInfoA", 4, L(one)),
    ("LoadCursorA", 2, L(fake_handle)),
    ("LoadStringA", 4, L(zero)),
    ("RegisterClassExA", 1, L(register_class_ex_a)),
    ("CreateWindowExA", 12, L(create_window_ex_a)),
    ("DestroyWindow", 1, L(one)),
    ("ShowWindow", 2, L(zero)),
    ("UpdateWindow", 1, L(one)),
    ("AdjustWindowRect", 3, L(one)),
    ("GetClientRect", 2, L(get_rect)),
    ("GetWindowRect", 2, L(get_rect)),
    ("PeekMessageA", 5, L(peek_message_a)),
    ("TranslateMessage", 1, L(zero)),
    ("DefWindowProcA", 4, L(zero)),
    ("ShowCursor", 1, L(zero)),
    ("ClipCursor", 1, L(one)),
    ("GetCursorPos", 1, L(get_cursor_pos)),
    ("ScreenToClient", 2, L(one)),
    ("GetDesktopWindow", 0, L(get_desktop_window)),
    ("IsIconic", 1, L(zero)),
    ("IsWindow", 1, L(one)),
    ("IsWindowVisible", 1, L(one)),
    ("GetKeyboardState", 1, L(get_keyboard_state)),
    ("ToUnicode", 6, L(to_unicode)),
    ("MessageBoxA", 4, L(message_box_a)),
    ("PostQuitMessage", 1, L(post_quit_message)),
];
