//! USER32 without a screen: a window that exists only on paper. The game creates it, reads
//! its size and never receives messages. The real window comes with the graphics (Phase 3).

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
    // WNDCLASSEXA: lpfnWndProc at +8, lpszClassName at +40.
    let p = c.arg(0);
    rt.wndproc = rt.mem.r32(p + 8);
    eprintln!("[window] class \"{}\", procedure {:#010x}", rt.mem.cstr(rt.mem.r32(p + 40)), rt.wndproc);
    0xc001
}
fn create_window_ex_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("[window] CreateWindowExA \"{}\" {}x{}", rt.mem.cstr(c.arg(2)), c.arg(6) as i32, c.arg(7) as i32);
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
    let (x, y) = crate::window::state().cursor;
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
    eprintln!("[window] PostQuitMessage({})", c.arg(0));
    0
}

/// `ALTAIR_TEXT="2000:Desmond"`: from frame 2000 it types the text, one character every 12
/// frames, as window messages (`WM_KEYDOWN`, `WM_CHAR`, `WM_KEYUP`). Multiple texts
/// are separated with `|`.
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

/// The game drains the message queue every frame, from the thread that owns the window.
fn peek_message_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    if rt.wndproc == 0 || rt.mem.r32(c.teb() + 0x24) != rt.window_thread {
        return 0;
    }
    let frame = rt.d3d.frames;
    if frame != rt.text_frame {
        rt.text_frame = frame;
        // The script's arrow keys also arrive as messages: the menu lists read them from there.
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
            // No WM_CHAR: the window procedure derives the character by itself, with ToUnicode.
            rt.messages.extend([(0x100, vk, 1), (0x101, vk, 0xc000_0001)]);
            rt.typed.push_back((vk, ch));
        }
    }
    // Real keyboard and mouse, from the window.
    {
        use crate::window::Event;
        let events: Vec<Event> = crate::window::state().events.drain(..).collect();
        for e in events {
            if crate::window::log_enabled() {
                match &e {
                    Event::Key { vk, down, ch, .. } => eprintln!("[input] message: {} VK {vk}, character {ch:?}", if *down { "WM_KEYDOWN" } else { "WM_KEYUP" }),
                    Event::Button { index, down } => eprintln!("[input] message: button {index} {}", if *down { "down" } else { "up" }),
                    Event::MouseMove(..) => {}
                }
            }
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
                    // WM_LBUTTONDOWN 0x201, WM_RBUTTONDOWN 0x204, WM_MBUTTONDOWN 0x207; the release is the next one.
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

/// `ToUnicode(vk, scan, state, buffer, capacity, flags)`: the character of the pressed key. Here
/// keys only come from the script, so the character is the one waiting.
fn to_unicode(rt: &mut Runtime, c: &mut Call) -> u32 {
    let vk = c.arg(0);
    // The character waiting for this key; older ones, never asked for, are discarded.
    let Some(i) = rt.typed.iter().position(|(v, _)| *v == vk) else { return 0 };
    let (_, ch) = rt.typed.drain(..=i).last().unwrap();
    if c.arg(4) < 1 {
        return 0;
    }
    rt.mem.w16(c.arg(3), ch as u16);
    1
}

/// Delivers a message to the window procedure, which is game code.
pub fn dispatch_message_a(c: &mut Call) -> u32 {
    let p = c.arg(0);
    let args = [c.mem.r32(p), c.mem.r32(p + 4), c.mem.r32(p + 8), c.mem.r32(p + 12)];
    let wndproc = crate::win32::with_runtime(|rt| rt.wndproc);
    if wndproc == 0 {
        return 0;
    }
    crate::win32::call_guest(c, wndproc, &args)
}

/// A single monitor, as large as the declared screen. The callback is guest code.
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
