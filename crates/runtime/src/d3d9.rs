//! Direct3D 9 "registratore": accetta le chiamate del gioco, tiene lo stato minimo che il gioco
//! rilegge, conta tutto e non disegna nulla. Serve a sapere quali metodi, formati e shader usa
//! davvero il gioco, prima di scrivere il renderer su wgpu.
//!
//! `ALTAIR_FRAMES=N` ferma il gioco dopo N `Present` e stampa il censimento.

use crate::com::{self, Method};
use crate::user32::SCREEN;
use crate::win32::{Call, Runtime, Shim, Shim::L};
use std::collections::{BTreeMap, HashMap};

const D3D_OK: u32 = 0;
const NOT_AVAILABLE: u32 = 0x8876_086a;
const INVALID_CALL: u32 = 0x8876_086c;

pub enum Res {
    Texture { w: u32, h: u32, depth: u32, levels: u32, format: u32, usage: u32, pool: u32, surfaces: HashMap<u32, u32> },
    Surface { w: u32, h: u32, format: u32, usage: u32, mem: u32 },
    Buffer { mem: u32, size: u32, usage: u32, pool: u32, fvf: u32, index: bool },
    Shader { code: Vec<u8> },
    Decl { elements: Vec<u8> },
    Query { kind: u32 },
    Other,
}

#[derive(Default)]
pub struct D3d {
    d3d: u32,
    device: u32,
    pub backbuffer: u32,
    depth: u32,
    /// superficie -> (texture madre, livello, faccia)
    pub surface_parent: HashMap<u32, (u32, u32, u32)>,
    /// texture -> 1 cubo, 2 volume
    pub tex_kind: HashMap<u32, u8>,
    /// flussi di vertici: (buffer, offset, passo)
    pub streams: [(u32, u32, u32); 4],
    pub indices: u32,
    pub vs_f: Vec<f32>,
    pub ps_f: Vec<f32>,
    pub vs_i: Vec<i32>,
    pub ps_i: Vec<i32>,
    pub vs_b: u32,
    pub ps_b: u32,
    /// (campionatore, tipo di stato) -> valore
    pub sampler_states: HashMap<(u32, u32), u32>,
    pub dirty_buffers: std::cell::RefCell<std::collections::HashSet<u32>>,
    pub res: HashMap<u32, Res>,
    /// Tabelle delle costanti D3DX: oggetto -> (indirizzo guest del CTAB, dimensione).
    ctabs: HashMap<u32, (u32, u32)>,
    pub render_targets: [u32; 4],
    pub depth_target: u32,
    pub viewport: [u32; 6],
    pub render_states: HashMap<u32, u32>,
    pub textures: HashMap<u32, u32>,
    pub vshader: u32,
    pub pshader: u32,
    pub decl: u32,
    pub frames: u64,
    draws: u64,
    draws_at_last_present: u64,
    max_draws_per_frame: u64,
    primitives: u64,
    // Censimento
    texture_formats: BTreeMap<(String, u32), u64>,
    rt_formats: BTreeMap<(String, u32), u64>,
    shader_versions: BTreeMap<String, u64>,
    render_state_ids: BTreeMap<u32, u64>,
    sampler_state_ids: BTreeMap<u32, u64>,
    format_checks: BTreeMap<(u32, u32, u32), u64>,
    decl_usages: BTreeMap<(u8, u8), u64>,
    limit: Option<u64>,
    present: (u32, u32),
}

fn fmt_name(f: u32) -> String {
    let known = match f {
        20 => "R8G8B8",
        21 => "A8R8G8B8",
        22 => "X8R8G8B8",
        23 => "R5G6B5",
        25 => "A1R5G5B5",
        26 => "A4R4G4B4",
        28 => "A8",
        32 => "A8B8G8R8",
        34 => "G16R16",
        36 => "A16B16G16R16",
        50 => "L8",
        51 => "A8L8",
        60 => "V8U8",
        63 => "Q8W8V8U8",
        70 => "D16_LOCKABLE",
        71 => "D32",
        75 => "D24S8",
        77 => "D24X8",
        80 => "D16",
        82 => "D32F_LOCKABLE",
        83 => "D24FS8",
        100 => "VERTEXDATA",
        101 => "INDEX16",
        102 => "INDEX32",
        111 => "R16F",
        112 => "G16R16F",
        113 => "A16B16G16R16F",
        114 => "R32F",
        115 => "G32R32F",
        116 => "A32B32G32R32F",
        0 => "UNKNOWN",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let b = f.to_le_bytes();
    if b.iter().all(|c| c.is_ascii_graphic()) {
        return String::from_utf8_lossy(&b).into_owned();
    }
    format!("{f:#x}")
}

/// (byte per pixel, oppure 0 se a blocchi 4x4; byte per blocco)
fn fmt_layout(f: u32) -> (u32, u32) {
    match &f.to_le_bytes() {
        b"DXT1" | b"ATI1" => (0, 8),
        b"DXT2" | b"DXT3" | b"DXT4" | b"DXT5" | b"ATI2" => (0, 16),
        _ => match f {
            20 => (3, 0),
            23 | 25 | 26 | 51 | 60 | 70 | 80 | 101 | 111 => (2, 0),
            28 | 50 => (1, 0),
            36 | 113 | 115 => (8, 0),
            116 => (16, 0),
            _ => (4, 0),
        },
    }
}

/// (pitch, dimensione in byte) di un livello.
fn level_size(f: u32, w: u32, h: u32) -> (u32, u32) {
    let (bpp, block) = fmt_layout(f);
    if bpp == 0 {
        let (bw, bh) = (w.div_ceil(4).max(1), h.div_ceil(4).max(1));
        (bw * block, bw * bh * block)
    } else {
        (w * bpp, w * h * bpp)
    }
}

/// Esegue `f` con il renderer, se c'è, e lo stato Direct3D in sola lettura.
fn with_gfx(rt: &mut Runtime, f: impl FnOnce(&mut crate::gfx::Gfx, &D3d, crate::machine::Mem)) {
    if let Some(mut g) = rt.gfx.take() {
        f(&mut g, &rt.d3d, rt.mem);
        rt.gfx = Some(g);
    }
}

fn zero(_: &mut Runtime, _: &mut Call) -> u32 {
    0
}
fn one(_: &mut Runtime, _: &mut Call) -> u32 {
    1
}

// --- IDirect3D9 -----------------------------------------------------------------------------

pub fn direct3d_create9(rt: &mut Runtime, c: &mut Call) -> u32 {
    let obj = com::create(rt, "IDirect3D9", DIRECT3D, 0);
    rt.d3d.d3d = obj;
    rt.d3d.limit = std::env::var("ALTAIR_FRAMES").ok().and_then(|v| v.parse().ok());
    eprintln!("[d3d] Direct3DCreate9(SDK {}) → registratore", c.arg(0));
    obj
}

const MODES: [(u32, u32); 6] = [(640, 480), (800, 600), (1024, 768), (1280, 720), (1280, 1024), (1920, 1080)];

fn write_mode(rt: &mut Runtime, p: u32, w: u32, h: u32) {
    rt.mem.w32(p, w);
    rt.mem.w32(p + 4, h);
    rt.mem.w32(p + 8, 60);
    rt.mem.w32(p + 12, 22); // X8R8G8B8
}
fn d3d_get_adapter_identifier(rt: &mut Runtime, c: &mut Call) -> u32 {
    let p = c.arg(3);
    rt.mem.fill(p, 1100, 0);
    rt.mem.write_cstr(p, b"altair.dll");
    rt.mem.write_cstr(p + 512, b"Altair Direct3D 9 (registratore)");
    rt.mem.write_cstr(p + 1024, b"\\\\.\\DISPLAY1");
    rt.mem.w32(p + 1064, 0x10de); // VendorId: NVIDIA
    rt.mem.w32(p + 1068, 0x0191); // DeviceId: GeForce 8800 GTX
    D3D_OK
}
fn d3d_get_adapter_mode_count(_: &mut Runtime, c: &mut Call) -> u32 {
    if matches!(c.arg(2), 21 | 22) { MODES.len() as u32 } else { 0 }
}
fn d3d_enum_adapter_modes(rt: &mut Runtime, c: &mut Call) -> u32 {
    match MODES.get(c.arg(3) as usize) {
        Some(&(w, h)) => {
            write_mode(rt, c.arg(4), w, h);
            D3D_OK
        }
        None => INVALID_CALL,
    }
}
fn d3d_get_adapter_display_mode(rt: &mut Runtime, c: &mut Call) -> u32 {
    write_mode(rt, c.arg(2), 1920, 1080);
    D3D_OK
}
fn d3d_check_device_format(rt: &mut Runtime, c: &mut Call) -> u32 {
    *rt.d3d.format_checks.entry((c.arg(4), c.arg(5), c.arg(6))).or_default() += 1;
    // Tra i formati con codice a quattro lettere si accettano solo i DXT: gli altri sono trucchi
    // di singoli produttori (R2VB, INTZ, NULL…) che il gioco usa solo se li trova.
    let f = c.arg(6);
    if f > 0xffff && !matches!(&f.to_le_bytes(), b"DXT1" | b"DXT2" | b"DXT3" | b"DXT4" | b"DXT5") {
        return NOT_AVAILABLE;
    }
    D3D_OK
}
fn d3d_check_multisample(rt: &mut Runtime, c: &mut Call) -> u32 {
    if c.arg(5) > 1 {
        return NOT_AVAILABLE; // niente MSAA nel registratore
    }
    if c.arg(6) != 0 {
        rt.mem.w32(c.arg(6), 1);
    }
    D3D_OK
}

fn write_caps(rt: &mut Runtime, p: u32) {
    let m = rt.mem;
    m.fill(p, 304, 0);
    let f = |v: f32| v.to_bits();
    let fields: [(u32, u32); 52] = [
        (0, 1), // D3DDEVTYPE_HAL
        (8, 0x0002_0000),
        (12, 0xe002_0000),
        (16, 0x0000_03a0),
        (20, 0x8000_000f),
        (24, 1),
        (28, 0x001b_bef0),
        (32, 0x002f_cef2),
        (36, 0x0773_2191),
        (40, 0xff),
        (44, 0x3fff),
        (48, 0x3fff),
        (52, 0xff),
        (56, 0x0008_4208),
        (60, 0x0001_ecc5),
        (64, 0x0303_0700),
        (68, 0x0303_0700),
        (72, 0x0303_0700),
        (76, 0x3f),
        (80, 0x3f),
        (84, 0x1f),
        (88, 8192),
        (92, 8192),
        (96, 2048),
        (100, 8192),
        (104, 8192),
        (108, 16),
        (112, f(1e10)),
        (116, f(-32768.0)),
        (120, f(-32768.0)),
        (124, f(32768.0)),
        (128, f(32768.0)),
        (136, 0x1ff),
        (140, 0x0018_0008),
        (144, 0x03fe_ffff),
        (148, 8),
        (152, 8),
        (156, 0x17b),
        (160, 8),
        (164, 6),
        (168, 4),
        (176, f(256.0)),
        (180, 0x00ff_ffff),
        (184, 0x00ff_ffff),
        (188, 16),
        (192, 508),
        (196, 0xfffe_0300), // vs_3_0
        (200, 256),
        (204, 0xffff_0300), // ps_3_0
        (208, f(f32::MAX)),
        (212, 0x51),
        (232, 1),
    ];
    for (off, v) in fields {
        m.w32(p + off, v);
    }
    m.w32(p + 236, 0x3ff); // DeclTypes
    m.w32(p + 240, 4); // NumSimultaneousRTs
    m.w32(p + 244, 0x0300_0300);
    for (i, v) in [1u32, 24, 32, 4].iter().enumerate() {
        m.w32(p + 248 + 4 * i as u32, *v);
    }
    for (i, v) in [0x1fu32, 24, 32, 4, 512].iter().enumerate() {
        m.w32(p + 264 + 4 * i as u32, *v);
    }
    m.w32(p + 284, 0x0303_0700);
    m.w32(p + 288, 0xffff_ffff);
    m.w32(p + 292, 0xffff_ffff);
    m.w32(p + 296, 32768);
    m.w32(p + 300, 32768);
}
fn d3d_get_device_caps(rt: &mut Runtime, c: &mut Call) -> u32 {
    write_caps(rt, c.arg(3));
    D3D_OK
}

fn new_surface(rt: &mut Runtime, w: u32, h: u32, format: u32, usage: u32) -> u32 {
    let obj = com::create(rt, "IDirect3DSurface9", SURFACE, 0);
    rt.d3d.res.insert(obj, Res::Surface { w, h, format, usage, mem: 0 });
    obj
}

fn d3d_create_device(rt: &mut Runtime, c: &mut Call) -> u32 {
    let pp = c.arg(5);
    let m = rt.mem;
    let (mut w, mut h) = (m.r32(pp), m.r32(pp + 4));
    if w == 0 || h == 0 {
        (w, h) = SCREEN;
    }
    eprintln!(
        "[d3d] CreateDevice: back buffer {w}x{h} {} ×{}, MSAA {}, swap {}, finestra {}, depth automatico {} ({}), flag comportamento {:#x}, intervallo {:#x}",
        fmt_name(m.r32(pp + 8)),
        m.r32(pp + 12),
        m.r32(pp + 16),
        m.r32(pp + 24),
        if m.r32(pp + 32) != 0 { "sì" } else { "schermo intero" },
        m.r32(pp + 36) != 0,
        fmt_name(m.r32(pp + 40)),
        c.arg(4),
        m.r32(pp + 52)
    );
    let dev = com::create(rt, "IDirect3DDevice9", DEVICE, 0);
    rt.d3d.device = dev;
    rt.d3d.present = (w, h);
    let bb_format = if m.r32(pp + 8) == 0 { 22 } else { m.r32(pp + 8) };
    rt.d3d.backbuffer = new_surface(rt, w, h, bb_format, 1);
    rt.d3d.depth = if m.r32(pp + 36) != 0 { new_surface(rt, w, h, m.r32(pp + 40), 2) } else { 0 };
    rt.d3d.render_targets = [rt.d3d.backbuffer, 0, 0, 0];
    rt.d3d.depth_target = rt.d3d.depth;
    rt.d3d.viewport = [0, 0, w, h, 0f32.to_bits(), 1f32.to_bits()];
    rt.d3d.vs_f = vec![0.0; 1024];
    rt.d3d.ps_f = vec![0.0; 1024];
    rt.d3d.vs_i = vec![0; 64];
    rt.d3d.ps_i = vec![0; 64];
    // ALTAIR_GFX=0 torna al solo registratore, senza disegnare.
    if std::env::var("ALTAIR_GFX").map_or(true, |v| v != "0") {
        rt.gfx = crate::gfx::Gfx::new();
        if rt.gfx.is_none() {
            eprintln!("[gfx] nessun adattatore grafico: si prosegue senza disegnare");
        }
    }
    m.w32(c.arg(6), dev);
    D3D_OK
}

static DIRECT3D: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("RegisterSoftwareDevice", 2, None),
    ("GetAdapterCount", 1, Some(L(one))),
    ("GetAdapterIdentifier", 4, Some(L(d3d_get_adapter_identifier))),
    ("GetAdapterModeCount", 3, Some(L(d3d_get_adapter_mode_count))),
    ("EnumAdapterModes", 5, Some(L(d3d_enum_adapter_modes))),
    ("GetAdapterDisplayMode", 3, Some(L(d3d_get_adapter_display_mode))),
    ("CheckDeviceType", 6, Some(L(zero))),
    ("CheckDeviceFormat", 7, Some(L(d3d_check_device_format))),
    ("CheckDeviceMultiSampleType", 7, Some(L(d3d_check_multisample))),
    ("CheckDepthStencilMatch", 6, Some(L(zero))),
    ("CheckDeviceFormatConversion", 5, Some(L(zero))),
    ("GetDeviceCaps", 4, Some(L(d3d_get_device_caps))),
    ("GetAdapterMonitor", 2, Some(L(one))),
    ("CreateDevice", 7, Some(L(d3d_create_device))),
];

// --- IDirect3DDevice9 -----------------------------------------------------------------------

fn out_obj(rt: &mut Runtime, p: u32, obj: u32) -> u32 {
    if obj != 0 {
        let n = rt.mem.r32(obj + 4) + 1;
        rt.mem.w32(obj + 4, n);
    }
    rt.mem.w32(p, obj);
    if obj == 0 { 0x8876_0866 } else { D3D_OK } // D3DERR_NOTFOUND
}

fn dev_get_direct3d(rt: &mut Runtime, c: &mut Call) -> u32 {
    let o = rt.d3d.d3d;
    out_obj(rt, c.arg(1), o)
}
fn dev_get_device_caps(rt: &mut Runtime, c: &mut Call) -> u32 {
    write_caps(rt, c.arg(1));
    D3D_OK
}
fn dev_get_display_mode(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (w, h) = rt.d3d.present;
    write_mode(rt, c.arg(2), w, h);
    D3D_OK
}
fn dev_available_texture_mem(_: &mut Runtime, _: &mut Call) -> u32 {
    0x4000_0000
}
fn dev_reset(rt: &mut Runtime, c: &mut Call) -> u32 {
    let pp = c.arg(1);
    eprintln!("[d3d] Reset: {}x{} {}", rt.mem.r32(pp), rt.mem.r32(pp + 4), fmt_name(rt.mem.r32(pp + 8)));
    D3D_OK
}

fn dev_present(rt: &mut Runtime, _: &mut Call) -> u32 {
    let d = &mut rt.d3d;
    d.frames += 1;
    let in_frame = d.draws - d.draws_at_last_present;
    d.max_draws_per_frame = d.max_draws_per_frame.max(in_frame);
    d.draws_at_last_present = d.draws;
    if d.frames == 1 || d.frames % 500 == 0 {
        eprintln!("[d3d] Present n. {} ({in_frame} draw in questo fotogramma)", d.frames);
    }
    let frame = d.frames;
    with_gfx(rt, |g, d, _| g.present(d, frame));
    let d = &mut rt.d3d;
    if d.limit.is_some_and(|n| d.frames >= n) {
        report(rt);
        eprintln!("\nlimite di ALTAIR_FRAMES raggiunto: esco");
        std::process::exit(0);
    }
    // Senza vsync il gioco girerebbe a vuoto: un fotogramma dura almeno un millisecondo.
    if rt.gfx.is_none() {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    D3D_OK
}

fn dev_get_back_buffer(rt: &mut Runtime, c: &mut Call) -> u32 {
    let o = rt.d3d.backbuffer;
    out_obj(rt, c.arg(4), o)
}

fn dev_create_texture(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (w, h, levels, usage, format, pool) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5), c.arg(6));
    let levels = if levels == 0 { 32 - w.max(h).max(1).leading_zeros() } else { levels };
    let obj = com::create(rt, "IDirect3DTexture9", TEXTURE, 0);
    let table = if usage & 3 != 0 { &mut rt.d3d.rt_formats } else { &mut rt.d3d.texture_formats };
    *table.entry((fmt_name(format), usage)).or_default() += 1;
    rt.d3d.res.insert(obj, Res::Texture { w, h, depth: 1, levels, format, usage, pool, surfaces: HashMap::new() });
    rt.mem.w32(c.arg(7), obj);
    D3D_OK
}
fn dev_create_volume_texture(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (w, h, depth, levels, usage, format, pool) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5), c.arg(6), c.arg(7));
    let obj = com::create(rt, "IDirect3DVolumeTexture9", VOLUME_TEXTURE, 0);
    rt.d3d.tex_kind.insert(obj, 2);
    *rt.d3d.texture_formats.entry((format!("{} (volume)", fmt_name(format)), usage)).or_default() += 1;
    rt.d3d.res.insert(obj, Res::Texture { w, h, depth, levels: levels.max(1), format, usage, pool, surfaces: HashMap::new() });
    rt.mem.w32(c.arg(8), obj);
    D3D_OK
}
fn dev_create_cube_texture(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (edge, levels, usage, format, pool) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5));
    let obj = com::create(rt, "IDirect3DCubeTexture9", CUBE_TEXTURE, 0);
    rt.d3d.tex_kind.insert(obj, 1);
    *rt.d3d.texture_formats.entry((format!("{} (cubo)", fmt_name(format)), usage)).or_default() += 1;
    rt.d3d.res.insert(obj, Res::Texture { w: edge, h: edge, depth: 1, levels: levels.max(1), format, usage, pool, surfaces: HashMap::new() });
    rt.mem.w32(c.arg(6), obj);
    D3D_OK
}
fn create_buffer(rt: &mut Runtime, c: &mut Call, index: bool) -> u32 {
    let (size, usage, third, pool) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    let mem = rt.halloc(size.max(1), true);
    if mem == 0 {
        return 0x8007_000e; // E_OUTOFMEMORY
    }
    let obj = com::create(rt, if index { "IDirect3DIndexBuffer9" } else { "IDirect3DVertexBuffer9" }, BUFFER, 0);
    rt.d3d.res.insert(obj, Res::Buffer { mem, size, usage, pool, fvf: third, index });
    rt.mem.w32(c.arg(5), obj);
    D3D_OK
}
fn dev_create_vertex_buffer(rt: &mut Runtime, c: &mut Call) -> u32 {
    create_buffer(rt, c, false)
}
fn dev_create_index_buffer(rt: &mut Runtime, c: &mut Call) -> u32 {
    create_buffer(rt, c, true)
}
fn dev_create_render_target(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (w, h, format) = (c.arg(1), c.arg(2), c.arg(3));
    *rt.d3d.rt_formats.entry((format!("{} (superficie)", fmt_name(format)), 1)).or_default() += 1;
    let s = new_surface(rt, w, h, format, 1);
    rt.mem.w32(c.arg(7), s);
    D3D_OK
}
fn dev_create_depth_stencil(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (w, h, format) = (c.arg(1), c.arg(2), c.arg(3));
    *rt.d3d.rt_formats.entry((format!("{} (superficie)", fmt_name(format)), 2)).or_default() += 1;
    let s = new_surface(rt, w, h, format, 2);
    rt.mem.w32(c.arg(7), s);
    D3D_OK
}
fn dev_create_offscreen(rt: &mut Runtime, c: &mut Call) -> u32 {
    let s = new_surface(rt, c.arg(1), c.arg(2), c.arg(3), 0);
    rt.mem.w32(c.arg(5), s);
    D3D_OK
}
fn dev_set_render_target(rt: &mut Runtime, c: &mut Call) -> u32 {
    if let Some(slot) = rt.d3d.render_targets.get_mut(c.arg(1) as usize) {
        *slot = c.arg(2);
    }
    D3D_OK
}
fn dev_get_render_target(rt: &mut Runtime, c: &mut Call) -> u32 {
    let o = rt.d3d.render_targets.get(c.arg(1) as usize).copied().unwrap_or(0);
    out_obj(rt, c.arg(2), o)
}
fn dev_set_depth(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.depth_target = c.arg(1);
    D3D_OK
}
fn dev_get_depth(rt: &mut Runtime, c: &mut Call) -> u32 {
    let o = rt.d3d.depth_target;
    out_obj(rt, c.arg(1), o)
}
fn dev_set_viewport(rt: &mut Runtime, c: &mut Call) -> u32 {
    for i in 0..6 {
        rt.d3d.viewport[i] = rt.mem.r32(c.arg(1) + 4 * i as u32);
    }
    D3D_OK
}
fn dev_get_viewport(rt: &mut Runtime, c: &mut Call) -> u32 {
    for i in 0..6 {
        rt.mem.w32(c.arg(1) + 4 * i as u32, rt.d3d.viewport[i]);
    }
    D3D_OK
}
fn dev_set_render_state(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.render_states.insert(c.arg(1), c.arg(2));
    *rt.d3d.render_state_ids.entry(c.arg(1)).or_default() += 1;
    D3D_OK
}
fn dev_get_render_state(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = rt.d3d.render_states.get(&c.arg(1)).copied().unwrap_or(0);
    rt.mem.w32(c.arg(2), v);
    D3D_OK
}
fn dev_set_sampler_state(rt: &mut Runtime, c: &mut Call) -> u32 {
    *rt.d3d.sampler_state_ids.entry(c.arg(2)).or_default() += 1;
    rt.d3d.sampler_states.insert((c.arg(1), c.arg(2)), c.arg(3));
    D3D_OK
}
fn dev_set_texture(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.textures.insert(c.arg(1), c.arg(2));
    D3D_OK
}
fn dev_get_texture(rt: &mut Runtime, c: &mut Call) -> u32 {
    let o = rt.d3d.textures.get(&c.arg(1)).copied().unwrap_or(0);
    out_obj(rt, c.arg(2), o);
    D3D_OK
}
fn new_state_block(rt: &mut Runtime, out: u32) -> u32 {
    let obj = com::create(rt, "IDirect3DStateBlock9", STATE_BLOCK, 0);
    rt.d3d.res.insert(obj, Res::Other);
    rt.mem.w32(out, obj);
    D3D_OK
}
fn dev_create_state_block(rt: &mut Runtime, c: &mut Call) -> u32 {
    new_state_block(rt, c.arg(2))
}
fn dev_end_state_block(rt: &mut Runtime, c: &mut Call) -> u32 {
    new_state_block(rt, c.arg(1))
}
fn dev_draw(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.draws += 1;
    rt.d3d.primitives += c.arg(3) as u64;
    let (ty, start, count) = (c.arg(1), c.arg(2), c.arg(3));
    with_gfx(rt, |g, d, mem| g.draw(d, mem, ty, start, count, None, None));
    D3D_OK
}
fn dev_draw_indexed(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.draws += 1;
    rt.d3d.primitives += c.arg(6) as u64;
    let (ty, base, start, count) = (c.arg(1), c.arg(2) as i32, c.arg(5), c.arg(6));
    with_gfx(rt, |g, d, mem| g.draw(d, mem, ty, start, count, Some(base), None));
    D3D_OK
}
fn dev_draw_up(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.draws += 1;
    rt.d3d.primitives += c.arg(2) as u64;
    let (ty, count, data, stride) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    with_gfx(rt, |g, d, mem| g.draw(d, mem, ty, 0, count, None, Some((data, stride))));
    D3D_OK
}
fn dev_draw_indexed_up(rt: &mut Runtime, _: &mut Call) -> u32 {
    rt.d3d.draws += 1;
    D3D_OK
}
fn dev_clear(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (flags, color, z, stencil) = (c.arg(3), c.arg(4), f32::from_bits(c.arg(5)), c.arg(6));
    with_gfx(rt, |g, d, _| g.clear(d, flags, color, z, stencil));
    D3D_OK
}
fn dev_stretch_rect(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (src, dst) = (c.arg(1), c.arg(3));
    with_gfx(rt, |g, d, _| g.blit(d, src, dst));
    D3D_OK
}
fn set_consts_f(dst: &mut [f32], rt: &Runtime, c: &Call) {
    let (start, p, n) = (c.arg(1) as usize, c.arg(2), c.arg(3) as usize);
    for k in 0..(4 * n).min(dst.len().saturating_sub(4 * start)) {
        dst[4 * start + k] = f32::from_bits(rt.mem.r32(p + 4 * k as u32));
    }
}
fn set_consts_i(dst: &mut [i32], rt: &Runtime, c: &Call) {
    let (start, p, n) = (c.arg(1) as usize, c.arg(2), c.arg(3) as usize);
    for k in 0..(4 * n).min(dst.len().saturating_sub(4 * start)) {
        dst[4 * start + k] = rt.mem.r32(p + 4 * k as u32) as i32;
    }
}
fn set_consts_b(dst: &mut u32, rt: &Runtime, c: &Call) {
    for k in 0..c.arg(3).min(32) {
        let bit = 1u32 << ((c.arg(1) + k) & 31);
        if rt.mem.r32(c.arg(2) + 4 * k) != 0 { *dst |= bit } else { *dst &= !bit }
    }
}
fn dev_set_vs_f(rt: &mut Runtime, c: &mut Call) -> u32 {
    let mut v = std::mem::take(&mut rt.d3d.vs_f);
    set_consts_f(&mut v, rt, c);
    rt.d3d.vs_f = v;
    D3D_OK
}
fn dev_set_ps_f(rt: &mut Runtime, c: &mut Call) -> u32 {
    let mut v = std::mem::take(&mut rt.d3d.ps_f);
    set_consts_f(&mut v, rt, c);
    rt.d3d.ps_f = v;
    D3D_OK
}
fn dev_set_vs_i(rt: &mut Runtime, c: &mut Call) -> u32 {
    let mut v = std::mem::take(&mut rt.d3d.vs_i);
    set_consts_i(&mut v, rt, c);
    rt.d3d.vs_i = v;
    D3D_OK
}
fn dev_set_ps_i(rt: &mut Runtime, c: &mut Call) -> u32 {
    let mut v = std::mem::take(&mut rt.d3d.ps_i);
    set_consts_i(&mut v, rt, c);
    rt.d3d.ps_i = v;
    D3D_OK
}
fn dev_set_vs_b(rt: &mut Runtime, c: &mut Call) -> u32 {
    let mut v = rt.d3d.vs_b;
    set_consts_b(&mut v, rt, c);
    rt.d3d.vs_b = v;
    D3D_OK
}
fn dev_set_ps_b(rt: &mut Runtime, c: &mut Call) -> u32 {
    let mut v = rt.d3d.ps_b;
    set_consts_b(&mut v, rt, c);
    rt.d3d.ps_b = v;
    D3D_OK
}
fn dev_set_stream_source(rt: &mut Runtime, c: &mut Call) -> u32 {
    if let Some(s) = rt.d3d.streams.get_mut(c.arg(1) as usize) {
        *s = (c.arg(2), c.arg(3), c.arg(4));
    }
    D3D_OK
}
fn dev_set_indices(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.indices = c.arg(1);
    D3D_OK
}

fn dev_create_vertex_declaration(rt: &mut Runtime, c: &mut Call) -> u32 {
    // D3DVERTEXELEMENT9: Stream u16, Offset u16, Type u8, Method u8, Usage u8, UsageIndex u8; fine: Stream 0xFF.
    let mut p = c.arg(1);
    let mut elements = Vec::new();
    while rt.mem.r16(p) != 0xff && elements.len() < 8 * 64 {
        let e = rt.mem.bytes(p, 8);
        *rt.d3d.decl_usages.entry((e[6], e[4])).or_default() += 1;
        elements.extend_from_slice(e);
        p += 8;
    }
    elements.extend_from_slice(rt.mem.bytes(p, 8));
    let obj = com::create(rt, "IDirect3DVertexDeclaration9", DECLARATION, 0);
    rt.d3d.res.insert(obj, Res::Decl { elements });
    rt.mem.w32(c.arg(2), obj);
    D3D_OK
}

/// Lunghezza in byte di uno shader D3D9: token fino a END (0x0000FFFF), saltando i commenti.
pub fn shader_len(rt: &Runtime, p: u32) -> u32 {
    let mut n = 1;
    loop {
        let t = rt.mem.r32(p + 4 * n);
        if t == 0x0000_ffff || n > 1 << 18 {
            return 4 * (n + 1);
        }
        n += if t & 0xffff == 0xfffe { 1 + (t >> 16) } else { 1 };
    }
}
fn create_shader(rt: &mut Runtime, c: &mut Call, iface: &'static str) -> u32 {
    let code = rt.mem.bytes(c.arg(1), shader_len(rt, c.arg(1))).to_vec();
    let v = u32::from_le_bytes(code[..4].try_into().unwrap());
    let name = format!("{}_{}_{}", if v >> 16 == 0xfffe { "vs" } else { "ps" }, (v >> 8) & 0xff, v & 0xff);
    *rt.d3d.shader_versions.entry(name).or_default() += 1;
    // ALTAIR_DUMP_SHADERS=cartella: salva ogni shader distinto (contenuto del gioco: va in private/).
    if let Some(dir) = std::env::var_os("ALTAIR_DUMP_SHADERS") {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in &code {
            h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join(format!("{}_{h:016x}.bin", if v >> 16 == 0xfffe { "vs" } else { "ps" }));
        if !file.exists() {
            let _ = std::fs::write(file, &code);
        }
    }
    let obj = com::create(rt, iface, SHADER, 0);
    rt.d3d.res.insert(obj, Res::Shader { code });
    rt.mem.w32(c.arg(2), obj);
    D3D_OK
}
fn dev_create_vertex_shader(rt: &mut Runtime, c: &mut Call) -> u32 {
    create_shader(rt, c, "IDirect3DVertexShader9")
}
fn dev_create_pixel_shader(rt: &mut Runtime, c: &mut Call) -> u32 {
    create_shader(rt, c, "IDirect3DPixelShader9")
}
fn dev_set_vertex_shader(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.vshader = c.arg(1);
    D3D_OK
}
fn dev_set_pixel_shader(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.pshader = c.arg(1);
    D3D_OK
}
fn dev_get_vertex_shader(rt: &mut Runtime, c: &mut Call) -> u32 {
    let o = rt.d3d.vshader;
    out_obj(rt, c.arg(1), o);
    D3D_OK
}
fn dev_get_pixel_shader(rt: &mut Runtime, c: &mut Call) -> u32 {
    let o = rt.d3d.pshader;
    out_obj(rt, c.arg(1), o);
    D3D_OK
}
fn dev_set_vertex_declaration(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.decl = c.arg(1);
    D3D_OK
}
fn dev_get_vertex_declaration(rt: &mut Runtime, c: &mut Call) -> u32 {
    let o = rt.d3d.decl;
    out_obj(rt, c.arg(1), o);
    D3D_OK
}
fn dev_create_query(rt: &mut Runtime, c: &mut Call) -> u32 {
    if c.arg(2) == 0 {
        return D3D_OK; // solo la domanda "è supportata?"
    }
    let obj = com::create(rt, "IDirect3DQuery9", QUERY, 0);
    rt.d3d.res.insert(obj, Res::Query { kind: c.arg(1) });
    rt.mem.w32(c.arg(2), obj);
    D3D_OK
}

const OK: Option<Shim> = Some(L(zero));

static DEVICE: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("TestCooperativeLevel", 1, OK),
    ("GetAvailableTextureMem", 1, Some(L(dev_available_texture_mem))),
    ("EvictManagedResources", 1, OK),
    ("GetDirect3D", 2, Some(L(dev_get_direct3d))),
    ("GetDeviceCaps", 2, Some(L(dev_get_device_caps))),
    ("GetDisplayMode", 3, Some(L(dev_get_display_mode))),
    ("GetCreationParameters", 2, None),
    ("SetCursorProperties", 4, OK),
    ("SetCursorPosition", 4, OK),
    ("ShowCursor", 2, OK),
    ("CreateAdditionalSwapChain", 3, None),
    ("GetSwapChain", 3, None),
    ("GetNumberOfSwapChains", 1, Some(L(one))),
    ("Reset", 2, Some(L(dev_reset))),
    ("Present", 5, Some(L(dev_present))),
    ("GetBackBuffer", 5, Some(L(dev_get_back_buffer))),
    ("GetRasterStatus", 3, None),
    ("SetDialogBoxMode", 2, OK),
    ("SetGammaRamp", 4, OK),
    ("GetGammaRamp", 3, None),
    ("CreateTexture", 9, Some(L(dev_create_texture))),
    ("CreateVolumeTexture", 10, Some(L(dev_create_volume_texture))),
    ("CreateCubeTexture", 8, Some(L(dev_create_cube_texture))),
    ("CreateVertexBuffer", 7, Some(L(dev_create_vertex_buffer))),
    ("CreateIndexBuffer", 7, Some(L(dev_create_index_buffer))),
    ("CreateRenderTarget", 9, Some(L(dev_create_render_target))),
    ("CreateDepthStencilSurface", 9, Some(L(dev_create_depth_stencil))),
    ("UpdateSurface", 5, OK),
    ("UpdateTexture", 3, OK),
    ("GetRenderTargetData", 3, OK),
    ("GetFrontBufferData", 3, OK),
    ("StretchRect", 6, Some(L(dev_stretch_rect))),
    ("ColorFill", 4, OK),
    ("CreateOffscreenPlainSurface", 7, Some(L(dev_create_offscreen))),
    ("SetRenderTarget", 3, Some(L(dev_set_render_target))),
    ("GetRenderTarget", 3, Some(L(dev_get_render_target))),
    ("SetDepthStencilSurface", 2, Some(L(dev_set_depth))),
    ("GetDepthStencilSurface", 2, Some(L(dev_get_depth))),
    ("BeginScene", 1, OK),
    ("EndScene", 1, OK),
    ("Clear", 7, Some(L(dev_clear))),
    ("SetTransform", 3, OK),
    ("GetTransform", 3, None),
    ("MultiplyTransform", 3, None),
    ("SetViewport", 2, Some(L(dev_set_viewport))),
    ("GetViewport", 2, Some(L(dev_get_viewport))),
    ("SetMaterial", 2, OK),
    ("GetMaterial", 2, None),
    ("SetLight", 3, OK),
    ("GetLight", 3, None),
    ("LightEnable", 3, OK),
    ("GetLightEnable", 3, None),
    ("SetClipPlane", 3, OK),
    ("GetClipPlane", 3, None),
    ("SetRenderState", 3, Some(L(dev_set_render_state))),
    ("GetRenderState", 3, Some(L(dev_get_render_state))),
    ("CreateStateBlock", 3, Some(L(dev_create_state_block))),
    ("BeginStateBlock", 1, OK),
    ("EndStateBlock", 2, Some(L(dev_end_state_block))),
    ("SetClipStatus", 2, OK),
    ("GetClipStatus", 2, None),
    ("GetTexture", 3, Some(L(dev_get_texture))),
    ("SetTexture", 3, Some(L(dev_set_texture))),
    ("GetTextureStageState", 4, None),
    ("SetTextureStageState", 4, OK),
    ("GetSamplerState", 4, None),
    ("SetSamplerState", 4, Some(L(dev_set_sampler_state))),
    ("ValidateDevice", 2, OK),
    ("SetPaletteEntries", 3, OK),
    ("GetPaletteEntries", 3, None),
    ("SetCurrentTexturePalette", 2, OK),
    ("GetCurrentTexturePalette", 2, None),
    ("SetScissorRect", 2, OK),
    ("GetScissorRect", 2, None),
    ("SetSoftwareVertexProcessing", 2, OK),
    ("GetSoftwareVertexProcessing", 1, OK),
    ("SetNPatchMode", 2, OK),
    ("GetNPatchMode", 1, OK),
    ("DrawPrimitive", 4, Some(L(dev_draw))),
    ("DrawIndexedPrimitive", 7, Some(L(dev_draw_indexed))),
    ("DrawPrimitiveUP", 5, Some(L(dev_draw_up))),
    ("DrawIndexedPrimitiveUP", 9, Some(L(dev_draw_indexed_up))),
    ("ProcessVertices", 7, None),
    ("CreateVertexDeclaration", 3, Some(L(dev_create_vertex_declaration))),
    ("SetVertexDeclaration", 2, Some(L(dev_set_vertex_declaration))),
    ("GetVertexDeclaration", 2, Some(L(dev_get_vertex_declaration))),
    ("SetFVF", 2, OK),
    ("GetFVF", 2, None),
    ("CreateVertexShader", 3, Some(L(dev_create_vertex_shader))),
    ("SetVertexShader", 2, Some(L(dev_set_vertex_shader))),
    ("GetVertexShader", 2, Some(L(dev_get_vertex_shader))),
    ("SetVertexShaderConstantF", 4, Some(L(dev_set_vs_f))),
    ("GetVertexShaderConstantF", 4, None),
    ("SetVertexShaderConstantI", 4, Some(L(dev_set_vs_i))),
    ("GetVertexShaderConstantI", 4, None),
    ("SetVertexShaderConstantB", 4, Some(L(dev_set_vs_b))),
    ("GetVertexShaderConstantB", 4, None),
    ("SetStreamSource", 5, Some(L(dev_set_stream_source))),
    ("GetStreamSource", 5, None),
    ("SetStreamSourceFreq", 3, OK),
    ("GetStreamSourceFreq", 3, None),
    ("SetIndices", 2, Some(L(dev_set_indices))),
    ("GetIndices", 2, None),
    ("CreatePixelShader", 3, Some(L(dev_create_pixel_shader))),
    ("SetPixelShader", 2, Some(L(dev_set_pixel_shader))),
    ("GetPixelShader", 2, Some(L(dev_get_pixel_shader))),
    ("SetPixelShaderConstantF", 4, Some(L(dev_set_ps_f))),
    ("GetPixelShaderConstantF", 4, None),
    ("SetPixelShaderConstantI", 4, Some(L(dev_set_ps_i))),
    ("GetPixelShaderConstantI", 4, None),
    ("SetPixelShaderConstantB", 4, Some(L(dev_set_ps_b))),
    ("GetPixelShaderConstantB", 4, None),
    ("DrawRectPatch", 4, None),
    ("DrawTriPatch", 4, None),
    ("DeletePatch", 2, None),
    ("CreateQuery", 3, Some(L(dev_create_query))),
];

// --- risorse --------------------------------------------------------------------------------

fn res_get_device(rt: &mut Runtime, c: &mut Call) -> u32 {
    let o = rt.d3d.device;
    out_obj(rt, c.arg(1), o)
}

macro_rules! resource_head {
    () => {
        [
            ("QueryInterface", 3, Some(L(com::no_interface))),
            ("AddRef", 1, Some(L(com::add_ref))),
            ("Release", 1, Some(L(com::release))),
            ("GetDevice", 2, Some(L(res_get_device))),
            ("SetPrivateData", 5, OK),
            ("GetPrivateData", 4, None),
            ("FreePrivateData", 2, OK),
            ("SetPriority", 2, OK),
            ("GetPriority", 1, OK),
            ("PreLoad", 1, OK),
            ("GetType", 1, None),
        ]
    };
}

fn write_surface_desc(rt: &mut Runtime, p: u32, format: u32, usage: u32, pool: u32, w: u32, h: u32) {
    for (i, v) in [format, 1, usage, pool, 0, 0, w, h].iter().enumerate() {
        rt.mem.w32(p + 4 * i as u32, *v);
    }
}
fn level_dims(w: u32, h: u32, level: u32) -> (u32, u32) {
    ((w >> level).max(1), (h >> level).max(1))
}

fn tex_get_level_count(rt: &mut Runtime, c: &mut Call) -> u32 {
    match rt.d3d.res.get(&c.arg(0)) {
        Some(Res::Texture { levels, .. }) => *levels,
        _ => 1,
    }
}
fn tex_get_level_desc(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(&Res::Texture { w, h, format, usage, pool, .. }) = rt.d3d.res.get(&c.arg(0)) else { return INVALID_CALL };
    let (lw, lh) = level_dims(w, h, c.arg(1));
    write_surface_desc(rt, c.arg(2), format, usage, pool, lw, lh);
    D3D_OK
}
/// La superficie di un livello (o di una faccia di cubo): creata alla prima richiesta.
fn tex_surface(rt: &mut Runtime, tex: u32, key: u32, level: u32) -> u32 {
    let Some(&Res::Texture { w, h, format, usage, .. }) = rt.d3d.res.get(&tex) else { return 0 };
    if let Some(Res::Texture { surfaces, .. }) = rt.d3d.res.get(&tex) {
        if let Some(&s) = surfaces.get(&key) {
            return s;
        }
    }
    let (lw, lh) = level_dims(w, h, level);
    let s = new_surface(rt, lw, lh, format, usage);
    rt.d3d.surface_parent.insert(s, (tex, level, key >> 8));
    if let Some(Res::Texture { surfaces, .. }) = rt.d3d.res.get_mut(&tex) {
        surfaces.insert(key, s);
    }
    s
}
fn tex_get_surface_level(rt: &mut Runtime, c: &mut Call) -> u32 {
    let s = tex_surface(rt, c.arg(0), c.arg(1), c.arg(1));
    out_obj(rt, c.arg(2), s)
}
fn cube_get_surface(rt: &mut Runtime, c: &mut Call) -> u32 {
    let s = tex_surface(rt, c.arg(0), c.arg(1) << 8 | c.arg(2), c.arg(2));
    out_obj(rt, c.arg(3), s)
}
/// Blocca una superficie: la memoria esiste solo tra Lock e Unlock, perché nessuno la legge.
fn lock_surface(rt: &mut Runtime, s: u32, locked: u32, depth: u32) -> u32 {
    let Some(&Res::Surface { w, h, format, mem, .. }) = rt.d3d.res.get(&s) else { return INVALID_CALL };
    let (pitch, size) = level_size(format, w, h);
    let mem = if mem != 0 { mem } else { rt.halloc((size * depth).max(16), false) };
    if mem == 0 {
        return 0x8007_000e;
    }
    if let Some(Res::Surface { mem: m, .. }) = rt.d3d.res.get_mut(&s) {
        *m = mem;
    }
    rt.mem.w32(locked, pitch);
    if depth > 1 {
        rt.mem.w32(locked + 4, size); // D3DLOCKED_BOX: RowPitch, SlicePitch, pBits
        rt.mem.w32(locked + 8, mem);
    } else {
        rt.mem.w32(locked + 4, mem);
    }
    D3D_OK
}
fn unlock_surface(rt: &mut Runtime, s: u32) -> u32 {
    if let Some(&Res::Surface { mem, .. }) = rt.d3d.res.get(&s) {
        if mem != 0 {
            with_gfx(rt, |g, d, m| g.upload(d, m, s, mem));
        }
    }
    if let Some(Res::Surface { mem, .. }) = rt.d3d.res.get_mut(&s) {
        let m = std::mem::take(mem);
        if m != 0 {
            rt.hfree(m);
        }
    }
    D3D_OK
}
fn tex_lock_rect(rt: &mut Runtime, c: &mut Call) -> u32 {
    let s = tex_surface(rt, c.arg(0), c.arg(1), c.arg(1));
    lock_surface(rt, s, c.arg(2), 1)
}
fn tex_unlock_rect(rt: &mut Runtime, c: &mut Call) -> u32 {
    let s = tex_surface(rt, c.arg(0), c.arg(1), c.arg(1));
    unlock_surface(rt, s)
}
fn cube_lock_rect(rt: &mut Runtime, c: &mut Call) -> u32 {
    let s = tex_surface(rt, c.arg(0), c.arg(1) << 8 | c.arg(2), c.arg(2));
    lock_surface(rt, s, c.arg(3), 1)
}
fn cube_unlock_rect(rt: &mut Runtime, c: &mut Call) -> u32 {
    let s = tex_surface(rt, c.arg(0), c.arg(1) << 8 | c.arg(2), c.arg(2));
    unlock_surface(rt, s)
}
fn vol_lock_box(rt: &mut Runtime, c: &mut Call) -> u32 {
    let depth = match rt.d3d.res.get(&c.arg(0)) {
        Some(Res::Texture { depth, .. }) => (*depth >> c.arg(1)).max(1),
        _ => 1,
    };
    let s = tex_surface(rt, c.arg(0), c.arg(1), c.arg(1));
    lock_surface(rt, s, c.arg(2), depth.max(2))
}

macro_rules! concat_methods {
    ($name:ident, $n:expr, [$($tail:expr),* $(,)?]) => {
        static $name: &[Method] = &{
            let head: [Method; 11] = resource_head!();
            let tail = [$($tail),*];
            let mut out: [Method; $n] = [("", 0, None); $n];
            let mut i = 0;
            while i < 11 {
                out[i] = head[i];
                i += 1;
            }
            while i < $n {
                out[i] = tail[i - 11];
                i += 1;
            }
            out
        };
    };
}

concat_methods!(TEXTURE, 22, [
    ("SetLOD", 2, OK),
    ("GetLOD", 1, OK),
    ("GetLevelCount", 1, Some(L(tex_get_level_count))),
    ("SetAutoGenFilterType", 2, OK),
    ("GetAutoGenFilterType", 1, None),
    ("GenerateMipSubLevels", 1, OK),
    ("GetLevelDesc", 3, Some(L(tex_get_level_desc))),
    ("GetSurfaceLevel", 3, Some(L(tex_get_surface_level))),
    ("LockRect", 5, Some(L(tex_lock_rect))),
    ("UnlockRect", 2, Some(L(tex_unlock_rect))),
    ("AddDirtyRect", 2, OK),
]);
concat_methods!(CUBE_TEXTURE, 22, [
    ("SetLOD", 2, OK),
    ("GetLOD", 1, OK),
    ("GetLevelCount", 1, Some(L(tex_get_level_count))),
    ("SetAutoGenFilterType", 2, OK),
    ("GetAutoGenFilterType", 1, None),
    ("GenerateMipSubLevels", 1, OK),
    ("GetLevelDesc", 3, Some(L(tex_get_level_desc))),
    ("GetCubeMapSurface", 4, Some(L(cube_get_surface))),
    ("LockRect", 6, Some(L(cube_lock_rect))),
    ("UnlockRect", 3, Some(L(cube_unlock_rect))),
    ("AddDirtyRect", 3, OK),
]);
concat_methods!(VOLUME_TEXTURE, 22, [
    ("SetLOD", 2, OK),
    ("GetLOD", 1, OK),
    ("GetLevelCount", 1, Some(L(tex_get_level_count))),
    ("SetAutoGenFilterType", 2, OK),
    ("GetAutoGenFilterType", 1, None),
    ("GenerateMipSubLevels", 1, OK),
    ("GetLevelDesc", 3, None),
    ("GetVolumeLevel", 3, None),
    ("LockBox", 5, Some(L(vol_lock_box))),
    ("UnlockBox", 2, Some(L(tex_unlock_rect))),
    ("AddDirtyBox", 2, OK),
]);

fn surf_get_desc(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(&Res::Surface { w, h, format, usage, .. }) = rt.d3d.res.get(&c.arg(0)) else { return INVALID_CALL };
    write_surface_desc(rt, c.arg(1), format, usage, 0, w, h);
    D3D_OK
}
fn surf_lock_rect(rt: &mut Runtime, c: &mut Call) -> u32 {
    lock_surface(rt, c.arg(0), c.arg(1), 1)
}
fn surf_unlock_rect(rt: &mut Runtime, c: &mut Call) -> u32 {
    unlock_surface(rt, c.arg(0))
}
concat_methods!(SURFACE, 17, [
    ("GetContainer", 3, None),
    ("GetDesc", 2, Some(L(surf_get_desc))),
    ("LockRect", 4, Some(L(surf_lock_rect))),
    ("UnlockRect", 1, Some(L(surf_unlock_rect))),
    ("GetDC", 2, None),
    ("ReleaseDC", 2, None),
]);

fn buf_lock(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(&Res::Buffer { mem, .. }) = rt.d3d.res.get(&c.arg(0)) else { return INVALID_CALL };
    rt.mem.w32(c.arg(3), mem + c.arg(1));
    D3D_OK
}
fn buf_get_desc(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(&Res::Buffer { size, usage, pool, fvf, index, .. }) = rt.d3d.res.get(&c.arg(0)) else { return INVALID_CALL };
    let p = c.arg(1);
    let format = if !index { 100 } else if fvf == 102 { 102 } else { 101 };
    for (i, v) in [format, if index { 7 } else { 6 }, usage, pool, size, if index { 0 } else { fvf }].iter().enumerate() {
        if i < 5 || !index {
            rt.mem.w32(p + 4 * i as u32, *v);
        }
    }
    D3D_OK
}
fn buf_unlock(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.d3d.dirty_buffers.borrow_mut().insert(c.arg(0));
    D3D_OK
}
concat_methods!(BUFFER, 14, [
    ("Lock", 5, Some(L(buf_lock))),
    ("Unlock", 1, Some(L(buf_unlock))),
    ("GetDesc", 2, Some(L(buf_get_desc))),
]);

fn decl_get_declaration(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(Res::Decl { elements }) = rt.d3d.res.get(&c.arg(0)) else { return INVALID_CALL };
    let e = elements.clone();
    if c.arg(1) != 0 {
        rt.mem.write(c.arg(1), &e);
    }
    rt.mem.w32(c.arg(2), e.len() as u32 / 8);
    D3D_OK
}
static DECLARATION: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetDevice", 2, Some(L(res_get_device))),
    ("GetDeclaration", 3, Some(L(decl_get_declaration))),
];

fn shader_get_function(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(Res::Shader { code }) = rt.d3d.res.get(&c.arg(0)) else { return INVALID_CALL };
    let code = code.clone();
    if c.arg(1) != 0 {
        rt.mem.write(c.arg(1), &code);
    }
    rt.mem.w32(c.arg(2), code.len() as u32);
    D3D_OK
}
static SHADER: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetDevice", 2, Some(L(res_get_device))),
    ("GetFunction", 3, Some(L(shader_get_function))),
];

/// Le query rispondono subito: "evento concluso", oppure "pixel visibili" per l'occlusione.
fn query_get_data(rt: &mut Runtime, c: &mut Call) -> u32 {
    let kind = match rt.d3d.res.get(&c.arg(0)) {
        Some(Res::Query { kind }) => *kind,
        _ => 0,
    };
    if c.arg(1) != 0 && c.arg(2) >= 4 {
        rt.mem.w32(c.arg(1), if kind == 9 { 10_000 } else { 1 }); // D3DQUERYTYPE_OCCLUSION = 9
    }
    D3D_OK
}
fn query_data_size(_: &mut Runtime, _: &mut Call) -> u32 {
    4
}
static QUERY: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetDevice", 2, Some(L(res_get_device))),
    ("GetType", 1, None),
    ("GetDataSize", 1, Some(L(query_data_size))),
    ("Issue", 2, OK),
    ("GetData", 4, Some(L(query_get_data))),
];
static STATE_BLOCK: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetDevice", 2, Some(L(res_get_device))),
    ("Capture", 1, OK),
    ("Apply", 1, OK),
];

// --- D3DX -----------------------------------------------------------------------------------

/// `D3DXCheckTextureRequirements`: accetta dimensioni e formato così come sono.
fn d3dx_check_texture_requirements(_: &mut Runtime, _: &mut Call) -> u32 {
    D3D_OK
}

/// Cerca la tabella delle costanti (commento `CTAB`) in uno shader. Ritorna l'indirizzo guest
/// della struttura D3DXSHADER_CONSTANTTABLE e la sua dimensione.
fn find_ctab(rt: &Runtime, shader: u32) -> Option<(u32, u32)> {
    let mut n = 1;
    loop {
        let t = rt.mem.r32(shader + 4 * n);
        if t == 0x0000_ffff || n > 1 << 18 {
            return None;
        }
        if t & 0xffff == 0xfffe {
            let words = t >> 16;
            if rt.mem.r32(shader + 4 * n + 4) == u32::from_le_bytes(*b"CTAB") {
                return Some((shader + 4 * n + 8, 4 * words - 4));
            }
            n += 1 + words;
        } else {
            n += 1;
        }
    }
}

/// `D3DXGetShaderConstantTable`: il gioco la usa per sapere in quale registro sta ogni costante
/// di uno shader. I nomi restano dove sono, dentro lo shader in memoria guest.
fn d3dx_get_shader_constant_table(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some((ctab, size)) = find_ctab(rt, c.arg(0)) else {
        rt.mem.w32(c.arg(1), 0);
        return 0x8876_0b59; // D3DXERR_INVALIDDATA
    };
    let obj = com::create(rt, "ID3DXConstantTable", CONSTANT_TABLE, 0);
    rt.d3d.ctabs.insert(obj, (ctab, size));
    rt.mem.w32(c.arg(1), obj);
    D3D_OK
}

const HANDLE_BASE: u32 = 0x0c7a_b000;

fn ctab_of(rt: &Runtime, c: &Call) -> (u32, u32) {
    rt.d3d.ctabs.get(&c.arg(0)).copied().unwrap_or((0, 0))
}
/// Indice della costante indicata da un handle, che può anche essere il suo nome.
fn ctab_index(rt: &Runtime, ctab: u32, handle: u32) -> Option<u32> {
    let count = rt.mem.r32(ctab + 12);
    if (HANDLE_BASE..HANDLE_BASE + count).contains(&handle) {
        return Some(handle - HANDLE_BASE);
    }
    if handle == 0 {
        return None;
    }
    let wanted = rt.mem.cstr_bytes(handle).to_vec();
    let info = ctab + rt.mem.r32(ctab + 16);
    (0..count).find(|i| rt.mem.cstr_bytes(ctab + rt.mem.r32(info + 20 * i)) == wanted)
}
fn ctab_get_buffer_pointer(rt: &mut Runtime, c: &mut Call) -> u32 {
    ctab_of(rt, c).0
}
fn ctab_get_buffer_size(rt: &mut Runtime, c: &mut Call) -> u32 {
    ctab_of(rt, c).1
}
fn ctab_get_desc(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (ctab, _) = ctab_of(rt, c);
    let p = c.arg(1);
    rt.mem.w32(p, ctab + rt.mem.r32(ctab + 4));
    rt.mem.w32(p + 4, rt.mem.r32(ctab + 8));
    rt.mem.w32(p + 8, rt.mem.r32(ctab + 12));
    D3D_OK
}
fn ctab_get_constant(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (ctab, _) = ctab_of(rt, c);
    if c.arg(1) != 0 || c.arg(2) >= rt.mem.r32(ctab + 12) {
        return 0; // i membri delle strutture non sono gestiti
    }
    HANDLE_BASE + c.arg(2)
}
fn ctab_get_constant_by_name(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (ctab, _) = ctab_of(rt, c);
    if c.arg(1) != 0 {
        return 0;
    }
    ctab_index(rt, ctab, c.arg(2)).map_or(0, |i| HANDLE_BASE + i)
}
fn ctab_get_constant_desc(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (ctab, _) = ctab_of(rt, c);
    let Some(i) = ctab_index(rt, ctab, c.arg(1)) else { return INVALID_CALL };
    let m = rt.mem;
    let info = ctab + m.r32(ctab + 16) + 20 * i;
    let ty = ctab + m.r32(info + 12);
    let (rows, cols, elements) = (m.r16(ty + 4) as u32, m.r16(ty + 6) as u32, m.r16(ty + 8) as u32);
    let default = m.r32(info + 16);
    let p = c.arg(2);
    if p != 0 {
        let fields = [
            ctab + m.r32(info),
            m.r16(info + 4) as u32,
            m.r16(info + 6) as u32,
            m.r16(info + 8) as u32,
            m.r16(ty) as u32,
            m.r16(ty + 2) as u32,
            rows,
            cols,
            elements,
            m.r16(ty + 10) as u32,
            4 * rows * cols * elements.max(1),
            if default != 0 { ctab + default } else { 0 },
        ];
        for (k, v) in fields.iter().enumerate() {
            m.w32(p + 4 * k as u32, *v);
        }
    }
    if c.arg(3) != 0 {
        m.w32(c.arg(3), 1);
    }
    D3D_OK
}

static CONSTANT_TABLE: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetBufferPointer", 1, Some(L(ctab_get_buffer_pointer))),
    ("GetBufferSize", 1, Some(L(ctab_get_buffer_size))),
    ("GetDesc", 2, Some(L(ctab_get_desc))),
    ("GetConstantDesc", 4, Some(L(ctab_get_constant_desc))),
    ("GetSamplerIndex", 2, None),
    ("GetConstant", 3, Some(L(ctab_get_constant))),
    ("GetConstantByName", 3, Some(L(ctab_get_constant_by_name))),
    ("GetConstantElement", 3, None),
    ("SetDefaults", 2, None),
    ("SetValue", 5, None),
    ("SetBool", 4, None),
    ("SetBoolArray", 5, None),
    ("SetInt", 4, None),
    ("SetIntArray", 5, None),
    ("SetFloat", 4, None),
    ("SetFloatArray", 5, None),
    ("SetVector", 4, None),
    ("SetVectorArray", 5, None),
    ("SetMatrix", 4, None),
    ("SetMatrixArray", 5, None),
    ("SetMatrixPointerArray", 5, None),
    ("SetMatrixTranspose", 4, None),
    ("SetMatrixTransposeArray", 5, None),
    ("SetMatrixTransposePointerArray", 5, None),
];

pub static SHIMS: &[(&str, u32, Shim)] = &[
    ("Direct3DCreate9", 1, L(direct3d_create9)),
    ("D3DXGetShaderConstantTable", 2, L(d3dx_get_shader_constant_table)),
    ("D3DXCheckTextureRequirements", 7, L(d3dx_check_texture_requirements)),
];

// --- censimento -----------------------------------------------------------------------------

pub fn report(rt: &Runtime) {
    let d = &rt.d3d;
    if d.d3d == 0 {
        return;
    }
    if let Some(g) = &rt.gfx {
        g.report();
    }
    eprintln!("\n===== censimento Direct3D 9 =====");
    eprintln!("fotogrammi (Present): {}; draw: {} (massimo {} in un fotogramma); primitive: {}", d.frames, d.draws, d.max_draws_per_frame, d.primitives);
    let mut calls: Vec<(&String, &u64)> = rt.calls.iter().filter(|(k, _)| k.starts_with("IDirect3D") || k.starts_with("ID3DX") || k.contains("D3DX")).collect();
    calls.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    eprintln!("metodi chiamati ({} distinti):", calls.len());
    for (k, n) in &calls {
        eprintln!("  {n:>9}  {k}");
    }
    let show = |title: &str, t: &BTreeMap<(String, u32), u64>| {
        if !t.is_empty() {
            eprintln!("{title}:");
            for ((f, usage), n) in t {
                eprintln!("  {n:>6}  {f}  (usage {usage:#x})");
            }
        }
    };
    show("texture create, per formato", &d.texture_formats);
    show("render target e depth, per formato", &d.rt_formats);
    eprintln!("shader creati: {:?}", d.shader_versions);
    eprintln!("render state impostati (id → volte): {:?}", d.render_state_ids);
    eprintln!("sampler state impostati (id → volte): {:?}", d.sampler_state_ids);
    eprintln!("elementi di dichiarazione dei vertici (usage, tipo) → volte: {:?}", d.decl_usages);
    eprintln!("CheckDeviceFormat (usage, tipo di risorsa, formato) → volte:");
    for ((usage, rtype, f), n) in &d.format_checks {
        eprintln!("  {n:>4}  usage {usage:#x} risorsa {rtype} {}", fmt_name(*f));
    }
}
