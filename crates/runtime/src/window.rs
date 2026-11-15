//! The real window (`ALTAIR_WINDOW=1`): winit on the main thread, which otherwise only
//! waits. The renderer presents the back buffer onto it; keyboard and mouse reach the game
//! through its two routes, the DirectInput state and the window messages.
//!
//! F10 grabs or releases the mouse pointer; F8 shows or hides the diagnostics panel
//! (frames per second, as an overlay: see `overlay.rs`).
//!
//! The runtime is not touched here: events pile up in `State`, and the shims read them
//! when the game asks for them.

use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::user32::SCREEN;

pub enum Event {
    /// Key: Windows virtual-key code, scancode, pressed or released, character produced.
    Key { vk: u32, scan: u32, down: bool, ch: Option<char> },
    /// Pointer, in the game's screen coordinates.
    MouseMove(u32, u32),
    /// Mouse button: 0 left, 1 right, 2 middle.
    Button { index: usize, down: bool },
}

pub struct State {
    /// Pressed keys, indexed by DirectInput DIK codes (0x80 = pressed).
    pub keys: [u8; 256],
    pub events: VecDeque<Event>,
    pub cursor: (u32, u32),
    /// Motion accumulated since the last read: x, y, wheel.
    pub motion: (i32, i32, i32),
    pub buttons: [bool; 8],
    raw_motion: bool,
    grabbed: bool,
    last_cursor: Option<(f64, f64)>,
}

const TITLE: &str = "AC1 Recompiled - Altaïr";
static WINDOW: OnceLock<Arc<Window>> = OnceLock::new();
/// wgpu instance and window surface, created by the window thread and waiting for the
/// renderer to take them.
static GRAPHICS: Mutex<Option<(wgpu::Instance, wgpu::Surface<'static>)>> = Mutex::new(None);

/// Hands the instance and the window surface over to the renderer (once only).
pub fn take_graphics() -> Option<(wgpu::Instance, wgpu::Surface<'static>)> {
    GRAPHICS.lock().unwrap_or_else(|e| e.into_inner()).take()
}
static FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    Mutex::new(State { keys: [0; 256], events: VecDeque::new(), cursor: (SCREEN.0 / 2, SCREEN.1 / 2), motion: (0, 0, 0), buttons: [false; 8], raw_motion: false, grabbed: false, last_cursor: None })
});

pub fn enabled() -> bool {
    std::env::var("ALTAIR_WINDOW").is_ok_and(|v| v == "1")
}

/// `ALTAIR_INPUT_LOG=1`: one line for every keyboard and mouse event, at every stage
/// (window → state → DirectInput or messages), to find out where one gets lost.
pub fn log_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("ALTAIR_INPUT_LOG").is_ok_and(|v| v == "1"))
}

pub fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The window, waiting up to ten seconds for the main thread to have created it.
pub fn window() -> Option<Arc<Window>> {
    if !enabled() {
        return None;
    }
    for _ in 0..1000 {
        if let Some(w) = WINDOW.get() {
            return Some(w.clone());
        }
        if FAILED.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    eprintln!("[window] the window did not appear: continuing off-screen");
    None
}

/// Physical key → (DIK code, Windows virtual-key code).
fn map(code: KeyCode) -> Option<(u8, u32)> {
    use KeyCode::*;
    Some(match code {
        KeyA => (30, 65), KeyB => (48, 66), KeyC => (46, 67), KeyD => (32, 68), KeyE => (18, 69), KeyF => (33, 70),
        KeyG => (34, 71), KeyH => (35, 72), KeyI => (23, 73), KeyJ => (36, 74), KeyK => (37, 75), KeyL => (38, 76),
        KeyM => (50, 77), KeyN => (49, 78), KeyO => (24, 79), KeyP => (25, 80), KeyQ => (16, 81), KeyR => (19, 82),
        KeyS => (31, 83), KeyT => (20, 84), KeyU => (22, 85), KeyV => (47, 86), KeyW => (17, 87), KeyX => (45, 88),
        KeyY => (21, 89), KeyZ => (44, 90),
        Digit1 => (2, 49), Digit2 => (3, 50), Digit3 => (4, 51), Digit4 => (5, 52), Digit5 => (6, 53),
        Digit6 => (7, 54), Digit7 => (8, 55), Digit8 => (9, 56), Digit9 => (10, 57), Digit0 => (11, 48),
        Enter => (28, 13), Escape => (1, 27), Space => (57, 32), Backspace => (14, 8), Tab => (15, 9),
        ShiftLeft => (42, 16), ShiftRight => (54, 16), ControlLeft => (29, 17), ControlRight => (157, 17),
        AltLeft => (56, 18), AltRight => (184, 18), CapsLock => (58, 20),
        ArrowUp => (200, 38), ArrowDown => (208, 40), ArrowLeft => (203, 37), ArrowRight => (205, 39),
        Insert => (210, 45), Delete => (211, 46), Home => (199, 36), End => (207, 35), PageUp => (201, 33), PageDown => (209, 34),
        F1 => (59, 112), F2 => (60, 113), F3 => (61, 114), F4 => (62, 115), F5 => (63, 116), F6 => (64, 117),
        F7 => (65, 118), F8 => (66, 119), F9 => (67, 120), F10 => (68, 121), F11 => (87, 122), F12 => (88, 123),
        Minus => (12, 189), Equal => (13, 187), Comma => (51, 188), Period => (52, 190), Slash => (53, 191),
        Semicolon => (39, 186), Quote => (40, 222), BracketLeft => (26, 219), BracketRight => (27, 221),
        Backslash => (43, 220), Backquote => (41, 192),
        Numpad0 => (82, 96), Numpad1 => (79, 97), Numpad2 => (80, 98), Numpad3 => (81, 99), Numpad4 => (75, 100),
        Numpad5 => (76, 101), Numpad6 => (77, 102), Numpad7 => (71, 103), Numpad8 => (72, 104), Numpad9 => (73, 105),
        NumpadEnter => (156, 13),
        _ => return None,
    })
}

struct App;

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if WINDOW.get().is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title(TITLE).with_inner_size(winit::dpi::PhysicalSize::new(SCREEN.0, SCREEN.1));
        // On Windows the icon is resource no. 1 of the executable: `tools/build-win.sh` extracts
        // it from the game's original executable and links it in (it is not in the repository).
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::{IconExtWindows, WindowAttributesExtWindows};
            match winit::window::Icon::from_resource(1, None) {
                Ok(icon) => attrs.with_window_icon(Some(icon.clone())).with_taskbar_icon(Some(icon)),
                Err(_) => attrs,
            }
        };
        match el.create_window(attrs) {
            Ok(w) => {
                eprintln!("[window] window opened, {}x{}", SCREEN.0, SCREEN.1);
                let w = Arc::new(w);
                // The drawing surface is created here, on the thread that owns the window: on
                // Windows the handle is not available from another thread. The renderer, which is
                // born on the game's thread, receives the wgpu instance and the surface ready-made.
                let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() });
                match instance.create_surface(w.clone()) {
                    Ok(surface) => *GRAPHICS.lock().unwrap_or_else(|e| e.into_inner()) = Some((instance, surface)),
                    Err(e) => eprintln!("[window] cannot create the surface ({e}): the game will draw off-screen"),
                }
                let _ = WINDOW.set(w);
            }
            Err(e) => eprintln!("[window] cannot open the window: {e}"),
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let mut s = state();
        match event {
            WindowEvent::CloseRequested => {
                eprintln!("[window] closed by the user: exiting");
                // Abrupt exit: tearing down wgpu and the window from here, with the game running, is pointless.
                crate::host::exit_now(0)
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                // F10 does not go to the game: it grabs or releases the pointer. Grabbed, the mouse moves
                // the view without stopping at the window edges; released, it is used for the menus.
                if code == KeyCode::F10 {
                    if event.state == ElementState::Pressed && !event.repeat {
                        s.grabbed = !s.grabbed;
                        if let Some(w) = WINDOW.get() {
                            use winit::window::CursorGrabMode::{Confined, Locked, None};
                            let ok = if s.grabbed { w.set_cursor_grab(Locked).or_else(|_| w.set_cursor_grab(Confined)).is_ok() } else { w.set_cursor_grab(None).is_ok() };
                            w.set_cursor_visible(!s.grabbed);
                            eprintln!("[window] pointer {}{}", if s.grabbed { "grabbed (F10 to release it)" } else { "released" }, if ok { "" } else { " (the compositor refused the grab)" });
                        }
                    }
                    return;
                }
                // F8 does not go to the game: it turns the diagnostics panel on or off (overlay.rs).
                if code == KeyCode::F8 {
                    if event.state == ElementState::Pressed && !event.repeat {
                        crate::overlay::toggle();
                    }
                    return;
                }
                let Some((dik, vk)) = map(code) else { return };
                let down = event.state == ElementState::Pressed;
                s.keys[dik as usize] = if down { 0x80 } else { 0 };
                let ch = if down { event.text.as_ref().and_then(|t| t.chars().next()).filter(|c| !c.is_control()) } else { None };
                if log_enabled() && !event.repeat {
                    eprintln!("[input] window: key {code:?} {} → DIK {dik}, VK {vk}, character {ch:?}", if down { "down" } else { "up" });
                }
                s.events.push_back(Event::Key { vk, scan: (dik & 0x7f) as u32, down, ch });
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some((px, py)) = s.last_cursor {
                    if !s.raw_motion {
                        s.motion.0 += (position.x - px) as i32;
                        s.motion.1 += (position.y - py) as i32;
                    }
                }
                s.last_cursor = Some((position.x, position.y));
                // The window can be resized: the game always reasons in terms of its own screen.
                let size = WINDOW.get().map(|w| w.inner_size()).filter(|z| z.width > 0 && z.height > 0);
                let (w, h) = size.map_or((SCREEN.0 as f64, SCREEN.1 as f64), |z| (z.width as f64, z.height as f64));
                let x = (position.x * SCREEN.0 as f64 / w).clamp(0.0, SCREEN.0 as f64 - 1.0) as u32;
                let y = (position.y * SCREEN.1 as f64 / h).clamp(0.0, SCREEN.1 as f64 - 1.0) as u32;
                s.cursor = (x, y);
                s.events.push_back(Event::MouseMove(x, y));
            }
            WindowEvent::MouseInput { state: st, button, .. } => {
                let index = match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    MouseButton::Back => 3,
                    MouseButton::Forward => 4,
                    MouseButton::Other(_) => return,
                };
                let down = st == ElementState::Pressed;
                if log_enabled() {
                    eprintln!("[input] window: button {index} {} at {:?}", if down { "down" } else { "up" }, s.cursor);
                }
                s.buttons[index] = down;
                if index < 3 {
                    s.events.push_back(Event::Button { index, down });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                s.motion.2 += match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 120.0) as i32,
                    MouseScrollDelta::PixelDelta(p) => p.y as i32,
                };
            }
            WindowEvent::Focused(false) => {
                // Without focus the releases do not arrive: no key stays pressed.
                s.keys = [0; 256];
                s.buttons = [false; 8];
            }
            _ => {}
        }
        if s.events.len() > 4096 {
            s.events.drain(..2048); // the game is not reading (loading): do not pile up
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            let mut s = state();
            s.raw_motion = true;
            s.motion.0 += delta.0 as i32;
            s.motion.1 += delta.1 as i32;
        }
    }
}

/// The event loop: it occupies the main thread until the end of the process. If the
/// display server is missing or winit cannot use it (on X11 it needs `libxkbcommon-x11`,
/// which may be missing in WSL; on Wayland it does not), the game continues off-screen.
pub fn run() -> ! {
    let outcome = std::panic::catch_unwind(|| match EventLoop::new() {
        Ok(el) => el.run_app(&mut App).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    });
    match outcome {
        Ok(Ok(())) => std::process::exit(0),
        Ok(Err(e)) => eprintln!("[window] no window ({e}): continuing off-screen"),
        Err(_) => eprintln!("[window] winit cannot open the window: continuing off-screen"),
    }
    FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
    loop {
        std::thread::park();
    }
}
