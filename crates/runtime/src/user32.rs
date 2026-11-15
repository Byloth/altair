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
    eprintln!("[finestra] CreateWindowExA \"{}\" {}x{} (senza schermo)", rt.mem.cstr(c.arg(2)), c.arg(6) as i32, c.arg(7) as i32);
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
    rt.mem.w32(c.arg(0), SCREEN.0 / 2);
    rt.mem.w32(c.arg(0) + 4, SCREEN.1 / 2);
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
    ("PeekMessageA", 5, L(zero)),
    ("TranslateMessage", 1, L(zero)),
    ("DispatchMessageA", 1, L(zero)),
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
    ("ToUnicode", 6, L(zero)),
    ("MessageBoxA", 4, L(message_box_a)),
    ("PostQuitMessage", 1, L(post_quit_message)),
];
