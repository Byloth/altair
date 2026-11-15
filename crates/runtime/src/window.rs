//! La finestra vera (`ALTAIR_WINDOW=1`): winit sul thread principale, che altrimenti aspetta
//! soltanto. Il renderer ci presenta sopra il back buffer; tastiera e mouse arrivano al gioco
//! per le sue due strade, lo stato di DirectInput e i messaggi della finestra.
//!
//! F10 cattura o libera il puntatore del mouse; F8 mostra o nasconde i fotogrammi al secondo
//! nel titolo della finestra.
//!
//! Qui non si tocca il runtime: gli eventi si accumulano in `State`, e gli shim li leggono
//! quando il gioco li chiede.

use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::user32::SCREEN;

pub enum Event {
    /// Tasto: codice virtuale di Windows, scancode, premuto o rilasciato, carattere prodotto.
    Key { vk: u32, scan: u32, down: bool, ch: Option<char> },
    /// Puntatore, in coordinate dello schermo del gioco.
    MouseMove(u32, u32),
    /// Pulsante del mouse: 0 sinistro, 1 destro, 2 centrale.
    Button { index: usize, down: bool },
}

pub struct State {
    /// Tasti premuti, indicizzati dai codici DIK di DirectInput (0x80 = premuto).
    pub keys: [u8; 256],
    pub events: VecDeque<Event>,
    pub cursor: (u32, u32),
    /// Movimento accumulato dall'ultima lettura: x, y, rotella.
    pub motion: (i32, i32, i32),
    pub buttons: [bool; 8],
    raw_motion: bool,
    grabbed: bool,
    last_cursor: Option<(f64, f64)>,
}

const TITLE: &str = "AC1 Recompiled - Altaïr";
static SHOW_FPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Chiamata dal renderer un paio di volte al secondo: se F8 li ha accesi, scrive nel titolo
/// della finestra i fotogrammi al secondo e i draw per fotogramma.
pub fn show_fps(fps: f64, draws: u64) {
    if SHOW_FPS.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(w) = WINDOW.get() {
            w.set_title(&format!("{TITLE} · {fps:.0} fps · {draws} draw"));
        }
    }
}

static WINDOW: OnceLock<Arc<Window>> = OnceLock::new();
/// Istanza wgpu e superficie della finestra, create dal thread della finestra e in attesa che
/// il renderer le prenda.
static GRAPHICS: Mutex<Option<(wgpu::Instance, wgpu::Surface<'static>)>> = Mutex::new(None);

/// Consegna al renderer l'istanza e la superficie della finestra (una volta sola).
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

/// `ALTAIR_INPUT_LOG=1`: una riga per ogni evento di tastiera e mouse, a ogni passaggio
/// (finestra → stato → DirectInput o messaggi), per capire dove se ne perde uno.
pub fn log_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("ALTAIR_INPUT_LOG").is_ok_and(|v| v == "1"))
}

pub fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// La finestra, aspettando fino a dieci secondi che il thread principale l'abbia creata.
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
    eprintln!("[finestra] la finestra non è comparsa: si prosegue fuori schermo");
    None
}

/// Tasto fisico → (codice DIK, codice virtuale di Windows).
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
        match el.create_window(attrs) {
            Ok(w) => {
                eprintln!("[finestra] finestra aperta, {}x{}", SCREEN.0, SCREEN.1);
                let w = Arc::new(w);
                // La superficie di disegno si crea qui, sul thread che possiede la finestra: su
                // Windows da un altro thread l'handle non è disponibile. Il renderer, che nasce
                // sul thread del gioco, riceve l'istanza wgpu e la superficie già pronte.
                let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() });
                match instance.create_surface(w.clone()) {
                    Ok(surface) => *GRAPHICS.lock().unwrap_or_else(|e| e.into_inner()) = Some((instance, surface)),
                    Err(e) => eprintln!("[finestra] superficie non creabile ({e}): il gioco disegnerà fuori schermo"),
                }
                let _ = WINDOW.set(w);
            }
            Err(e) => eprintln!("[finestra] impossibile aprire la finestra: {e}"),
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let mut s = state();
        match event {
            WindowEvent::CloseRequested => {
                eprintln!("[finestra] chiusa dall'utente: esco");
                // Uscita secca: smontare wgpu e la finestra da qui, con il gioco che gira, non serve.
                crate::host::exit_now(0)
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                // F10 non va al gioco: cattura o libera il puntatore. Catturato, il mouse muove la
                // visuale senza fermarsi ai bordi della finestra; libero, serve per i menu.
                if code == KeyCode::F10 {
                    if event.state == ElementState::Pressed && !event.repeat {
                        s.grabbed = !s.grabbed;
                        if let Some(w) = WINDOW.get() {
                            use winit::window::CursorGrabMode::{Confined, Locked, None};
                            let ok = if s.grabbed { w.set_cursor_grab(Locked).or_else(|_| w.set_cursor_grab(Confined)).is_ok() } else { w.set_cursor_grab(None).is_ok() };
                            w.set_cursor_visible(!s.grabbed);
                            eprintln!("[finestra] puntatore {}{}", if s.grabbed { "catturato (F10 per liberarlo)" } else { "libero" }, if ok { "" } else { " (il compositore ha rifiutato la cattura)" });
                        }
                    }
                    return;
                }
                // F8 non va al gioco: mostra o nasconde i fotogrammi al secondo nel titolo.
                if code == KeyCode::F8 {
                    if event.state == ElementState::Pressed && !event.repeat {
                        let on = !SHOW_FPS.fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
                        if !on {
                            if let Some(w) = WINDOW.get() {
                                w.set_title(TITLE);
                            }
                        }
                    }
                    return;
                }
                let Some((dik, vk)) = map(code) else { return };
                let down = event.state == ElementState::Pressed;
                s.keys[dik as usize] = if down { 0x80 } else { 0 };
                let ch = if down { event.text.as_ref().and_then(|t| t.chars().next()).filter(|c| !c.is_control()) } else { None };
                if log_enabled() && !event.repeat {
                    eprintln!("[input] finestra: tasto {code:?} {} → DIK {dik}, VK {vk}, carattere {ch:?}", if down { "giù" } else { "su" });
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
                // La finestra può essere ridimensionata: il gioco ragiona sempre sul suo schermo.
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
                    eprintln!("[input] finestra: pulsante {index} {} in {:?}", if down { "giù" } else { "su" }, s.cursor);
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
                // Senza fuoco i rilasci non arrivano: nessun tasto resta premuto.
                s.keys = [0; 256];
                s.buttons = [false; 8];
            }
            _ => {}
        }
        if s.events.len() > 4096 {
            s.events.drain(..2048); // il gioco non sta leggendo (caricamento): non si accumula
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

/// Il ciclo degli eventi: occupa il thread principale fino alla fine del processo. Se il
/// server grafico non c'è o winit non riesce a usarlo (su X11 gli serve `libxkbcommon-x11`,
/// che in WSL può mancare; su Wayland no), il gioco prosegue fuori schermo.
pub fn run() -> ! {
    let outcome = std::panic::catch_unwind(|| match EventLoop::new() {
        Ok(el) => el.run_app(&mut App).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    });
    match outcome {
        Ok(Ok(())) => std::process::exit(0),
        Ok(Err(e)) => eprintln!("[finestra] nessuna finestra ({e}): si prosegue fuori schermo"),
        Err(_) => eprintln!("[finestra] winit non riesce ad aprire la finestra: si prosegue fuori schermo"),
    }
    FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
    loop {
        std::thread::park();
    }
}
