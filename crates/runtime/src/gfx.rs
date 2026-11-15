//! Il renderer: esegue su wgpu i draw che il gioco chiede a Direct3D 9.
//!
//! Prima versione, fuori schermo: ogni superficie D3D9 è una texture wgpu; a ogni `Present`
//! il back buffer può essere salvato come PNG (`ALTAIR_SHOT_DIR`, `ALTAIR_SHOT_EVERY`).
//! Lo stato lo tiene `d3d9.rs`; qui lo si traduce in pipeline, bind group e render pass.
//! Ciò che non è ancora gestito viene saltato e contato, non fa fermare il gioco.

use crate::d3d9::{D3d, Res};
use crate::machine::Mem;
use altair_shader as sh;
use std::collections::{BTreeMap, HashMap};

const CONSTS_SIZE: u64 = 256 * 16 + 16 * 16 + 16 + 16;
const CONSTS_SLOT: u64 = 4608; // CONSTS_SIZE arrotondato all'allineamento degli offset (256)
const UBUF_SIZE: u64 = CONSTS_SLOT * 4096;
const SCRATCH_SIZE: u64 = 64 << 20;
/// Dimensione massima di un buffer che, riscritto mentre è in uso, va nel buffer di appoggio.
const SHADOW_MAX: u64 = 1 << 20;

struct Tex {
    texture: wgpu::Texture,
    format: wgpu::TextureFormat,
    w: u32,
    h: u32,
    dim: wgpu::TextureViewDimension,
    /// Come convertire i dati caricati dal gioco.
    conv: Conv,
}

#[derive(Clone, Copy, PartialEq)]
enum Conv {
    None,
    A8,
    L8,
    A8L8,
    X8,
    Unsupported,
}

struct ShaderGpu {
    module: wgpu::ShaderModule,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipeKey {
    vs: u32,
    ps: u32,
    decl: u32,
    strides: [u32; 4],
    opts_vs: sh::Options,
    opts_ps: sh::Options,
    colors: [Option<wgpu::TextureFormat>; 4],
    depth: Option<wgpu::TextureFormat>,
    topology: u32,
    index16: bool,
    states: [u32; 30],
}

/// Una scrittura da fare sulla coda prima dei comandi del fotogramma.
enum Write {
    Buffer(wgpu::Buffer, u64, Vec<u8>),
    Texture { texture: wgpu::Texture, level: u32, origin: wgpu::Origin3d, bytes: Vec<u8>, layout: wgpu::TexelCopyBufferLayout, extent: wgpu::Extent3d },
}

/// Ciò che il thread del gioco consegna al thread di invio: le scritture, i comandi
/// registrati, e l'immagine della finestra da presentare a cose fatte.
struct Batch {
    writes: Vec<Write>,
    commands: Option<wgpu::CommandBuffer>,
    present: Option<wgpu::SurfaceTexture>,
}

enum Job {
    Batch(Batch),
    /// Risponde quando tutto ciò che precede è stato inviato.
    Sync(std::sync::mpsc::Sender<()>),
}

/// Il cancello tra i due thread che parlano con wgpu. Con OpenGL wgpu protegge il contesto con
/// un lock che, se aspetta più di un secondo, manda il programma in errore: un invio lungo
/// (un caricamento) e la creazione di una risorsa dall'altro thread bastano a superarlo. Chi
/// invia e chi crea risorse passano prima da qui, dove si aspetta senza limite di tempo.
static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn gate() -> std::sync::MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Tempo passato dal thread di invio dentro la coda, in nanosecondi dall'ultima lettura.
static SUBMIT_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn run_batch(queue: &wgpu::Queue, batch: Batch) {
    let start = std::time::Instant::now();
    // Tutto ciò che tocca la coda avviene con il cancello in mano: così non si sovrappone mai
    // alla creazione di una risorsa sull'altro thread (i lock interni di wgpu non reggono
    // quell'incrocio: il gioco si fermava in caricamento). Le scritture grosse partono a pezzi,
    // rilasciando il cancello tra l'uno e l'altro, perché chi crea non aspetti troppo.
    const CHUNK: usize = 8 << 20;
    let mut writes = batch.writes.into_iter().peekable();
    loop {
        let _gate = gate();
        let mut staged = 0usize;
        while staged < CHUNK {
            let Some(w) = writes.next() else { break };
            match w {
                Write::Buffer(buffer, offset, data) => {
                    staged += data.len();
                    queue.write_buffer(&buffer, offset, &data)
                }
                Write::Texture { texture, level, origin, bytes, layout, extent } => {
                    staged += bytes.len();
                    queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level, origin, aspect: wgpu::TextureAspect::All }, &bytes, layout, extent)
                }
            }
        }
        if writes.peek().is_some() {
            queue.submit([]);
            continue;
        }
        queue.submit(batch.commands);
        if let Some(frame) = batch.present {
            frame.present();
        }
        break;
    }
    SUBMIT_NS.fetch_add(start.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
}

/// Il thread di invio. Eseguire i comandi (per OpenGL: le chiamate vere al driver) costa quanto
/// tutto il resto del fotogramma: farlo qui lascia il thread del gioco libero di preparare il
/// fotogramma successivo. La coda è corta, così il gioco non corre troppo avanti.
fn spawn_submitter(queue: wgpu::Queue) -> Option<std::sync::mpsc::SyncSender<Job>> {
    if std::env::var("ALTAIR_SUBMIT_THREAD").is_ok_and(|v| v == "0") {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::sync_channel::<Job>(1);
    std::thread::Builder::new()
        .name("invio-gpu".into())
        .spawn(move || {
            for job in rx {
                match job {
                    Job::Batch(batch) => run_batch(&queue, batch),
                    Job::Sync(done) => {
                        let _ = done.send(());
                    }
                }
            }
        })
        .ok()?;
    Some(tx)
}

/// Lo stato già impostato nel pass aperto.
#[derive(Default)]
struct PassState {
    pipeline: Option<wgpu::RenderPipeline>,
    blend: Option<u32>,
    consts: [Option<u32>; 2],
    groups: [Option<wgpu::BindGroup>; 2],
    stencil: Option<u32>,
    scissor: Option<[u32; 4]>,
    viewport: Option<[u32; 6]>,
    vbs: [Option<(wgpu::Buffer, u64)>; 5],
    index: Option<(wgpu::Buffer, u64, bool)>,
}

pub struct Gfx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Verso il thread di invio (`None`: si invia da qui) e le scritture in attesa.
    submitter: Option<std::sync::mpsc::SyncSender<Job>>,
    writes: Vec<Write>,
    blit_sampler: Option<wgpu::Sampler>,
    encoder: Option<wgpu::CommandEncoder>,
    pass: Option<wgpu::RenderPass<'static>>,
    pass_key: ([u32; 4], u32),
    pass_formats: ([Option<wgpu::TextureFormat>; 4], Option<wgpu::TextureFormat>),
    pass_size: (u32, u32),
    pending_clear: HashMap<u32, [f64; 4]>,
    pending_depth_clear: HashMap<u32, (Option<f32>, Option<u32>)>,
    textures: HashMap<u32, Tex>,
    buffers: HashMap<u32, wgpu::Buffer>,
    parsed: HashMap<u32, Option<sh::Shader>>,
    modules: HashMap<(u32, sh::Options), Option<ShaderGpu>>,
    pipelines: HashMap<PipeKey, Option<wgpu::RenderPipeline>>,
    last_pipeline: Option<(PipeKey, wgpu::RenderPipeline)>,
    samplers: HashMap<[u32; 6], wgpu::Sampler>,
    g0_layout: [wgpu::BindGroupLayout; 2],
    g0: [wgpu::BindGroup; 2],
    ubuf: wgpu::Buffer,
    /// Le altre copie di (costanti, bind group, appoggio), in attesa del loro turno.
    ring: std::collections::VecDeque<(wgpu::Buffer, [wgpu::BindGroup; 2], wgpu::Buffer)>,
    ubuf_off: u64,
    /// Contenuto in attesa del buffer uniforme e di quello di appoggio: scritti all'invio.
    ubuf_cpu: Vec<u8>,
    scratch_cpu: Vec<u8>,
    tex_layouts: HashMap<Vec<(u32, u32)>, wgpu::BindGroupLayout>,
    scratch: wgpu::Buffer,
    scratch_off: u64,
    zero_vb: wgpu::Buffer,
    dummies: [wgpu::TextureView; 3],
    blit: Option<(wgpu::ShaderModule, HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>)>,
    /// La superficie della finestra, se c'è (`ALTAIR_WINDOW=1`), con la sua configurazione.
    /// Buffer usati da draw non ancora inviati; versioni di appoggio (offset nel buffer di
    /// appoggio); buffer veri rimasti indietro rispetto alla memoria guest.
    used: std::collections::HashSet<u32>,
    shadow: HashMap<u32, u64>,
    /// Riserva di buffer vuoti per (dimensione, indici o vertici); quelli scartati in questo
    /// invio; quelli scartati negli invii precedenti, in attesa di tornare nella riserva.
    pool: HashMap<(u64, bool), Vec<wgpu::Buffer>>,
    retiring: Vec<((u64, bool), wgpu::Buffer)>,
    retired: std::collections::VecDeque<Vec<((u64, bool), wgpu::Buffer)>>,
    /// Buffer usati dagli ultimi invii (il più recente per primo), e se evitarne la riscrittura.
    recent: [std::collections::HashSet<u32>; 3],
    avoid_busy: bool,
    orphaned: u64,
    /// Intervallo di ogni buffer vero non ancora caricato dalla memoria guest.
    pending: HashMap<u32, (u64, u64)>,
    stale: std::collections::HashSet<u32>,
    flushes: u64,
    uploaded: u64,
    /// Ciò che il pass aperto ha già impostato, per non ripeterlo.
    last: PassState,
    prof: [std::time::Duration; 8],
    prof_end: std::time::Duration,
    prof_finish: std::time::Duration,
    /// Bind group delle texture già creati, per (stadio, texture e stati dei campionatori).
    groups: HashMap<Vec<[u32; 9]>, wgpu::BindGroup>,
    /// Ultimo blocco di costanti scritto per vertex e pixel shader, con il suo offset.
    consts_at: [Option<(usize, usize)>; 2],
    surface: Option<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration, std::sync::Arc<winit::window::Window>)>,
    pub drawn: u64,
    pub skipped: BTreeMap<String, u64>,
    shot_dir: Option<std::path::PathBuf>,
    shot_every: u64,
}

fn d3d_format(f: u32, depth: bool) -> (wgpu::TextureFormat, Conv) {
    use wgpu::TextureFormat as F;
    if depth {
        return (F::Depth24PlusStencil8, Conv::Unsupported);
    }
    match &f.to_le_bytes() {
        b"DXT1" => return (F::Bc1RgbaUnorm, Conv::None),
        b"DXT2" | b"DXT3" => return (F::Bc2RgbaUnorm, Conv::None),
        b"DXT4" | b"DXT5" => return (F::Bc3RgbaUnorm, Conv::None),
        _ => {}
    }
    match f {
        21 => (F::Bgra8Unorm, Conv::None),
        22 => (F::Bgra8Unorm, Conv::X8),
        28 => (F::Bgra8Unorm, Conv::A8),
        50 => (F::Bgra8Unorm, Conv::L8),
        51 => (F::Bgra8Unorm, Conv::A8L8),
        // I target in virgola mobile: a 16 bit, che si possono filtrare su ogni backend.
        111..=116 | 36 => (F::Rgba16Float, Conv::Unsupported),
        70..=83 => (F::Depth24PlusStencil8, Conv::Unsupported),
        _ => (F::Bgra8Unorm, Conv::Unsupported),
    }
}

fn compare(f: u32) -> wgpu::CompareFunction {
    use wgpu::CompareFunction::*;
    match f {
        1 => Never,
        2 => Less,
        3 => Equal,
        4 => LessEqual,
        5 => Greater,
        6 => NotEqual,
        7 => GreaterEqual,
        _ => Always,
    }
}

fn blend_factor(f: u32) -> wgpu::BlendFactor {
    use wgpu::BlendFactor::*;
    match f {
        1 => Zero,
        2 => One,
        3 => Src,
        4 => OneMinusSrc,
        5 => SrcAlpha,
        6 => OneMinusSrcAlpha,
        7 => DstAlpha,
        8 => OneMinusDstAlpha,
        9 => Dst,
        10 => OneMinusDst,
        11 => SrcAlphaSaturated,
        14 => Constant,
        15 => OneMinusConstant,
        _ => One,
    }
}

fn stencil_op(o: u32) -> wgpu::StencilOperation {
    use wgpu::StencilOperation::*;
    match o {
        2 => Zero,
        3 => Replace,
        4 => IncrementClamp,
        5 => DecrementClamp,
        6 => Invert,
        7 => IncrementWrap,
        8 => DecrementWrap,
        _ => Keep,
    }
}

fn blend_op(o: u32) -> wgpu::BlendOperation {
    use wgpu::BlendOperation::*;
    match o {
        2 => Subtract,
        3 => ReverseSubtract,
        4 => Min,
        5 => Max,
        _ => Add,
    }
}

/// Formato wgpu di un elemento di dichiarazione (D3DDECLTYPE) e come scalarlo nello shader.
fn decl_format(ty: u8) -> Option<(wgpu::VertexFormat, u8)> {
    use wgpu::VertexFormat::*;
    Some(match ty {
        0 => (Float32, 0),
        1 => (Float32x2, 0),
        2 => (Float32x3, 0),
        3 => (Float32x4, 0),
        4 => (Unorm8x4Bgra, 0),
        5 => (Unorm8x4, 1),
        6 => (Snorm16x2, 2),
        7 => (Snorm16x4, 2),
        8 => (Unorm8x4, 0),
        9 => (Snorm16x2, 0),
        10 => (Snorm16x4, 0),
        11 => (Unorm16x2, 0),
        12 => (Unorm16x4, 0),
        15 => (Float16x2, 0),
        16 => (Float16x4, 0),
        _ => return None,
    })
}

const BLIT: &str = "
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
struct V { @builtin(position) p: vec4f, @location(0) uv: vec2f }
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    let xy = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return V(vec4f(xy * 2.0 - 1.0, 0.0, 1.0), vec2f(xy.x, 1.0 - xy.y));
}
@fragment fn fs(v: V) -> @location(0) vec4f { return textureSample(t, s, v.uv); }
";

impl Gfx {
    pub fn new() -> Option<Gfx> {
        // Con la finestra, istanza e superficie arrivano dal thread che l'ha creata.
        let window = crate::window::window();
        let (instance, surface) = match window.as_ref().and_then(|_| crate::window::take_graphics()) {
            Some((instance, surface)) => (instance, Some(surface)),
            None => (wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() }), None),
        };
        if window.is_some() && surface.is_none() {
            eprintln!("[gfx] nessuna superficie per la finestra: si disegna fuori schermo");
        }
        let mut adapters = instance.enumerate_adapters(wgpu::Backends::all());
        if let Some(s) = &surface {
            adapters.retain(|a| a.is_surface_supported(s));
        }
        // Prima una GPU vera; a parità, il backend chiesto con ALTAIR_BACKEND (vulkan, dx12,
        // gl), altrimenti nell'ordine Vulkan, Direct3D 12, OpenGL. In WSL c'è solo OpenGL su
        // GPU; su Windows gli altri due parlano al driver senza passaggi intermedi.
        let wanted = std::env::var("ALTAIR_BACKEND").unwrap_or_default().to_ascii_lowercase();
        let rank = |a: &wgpu::Adapter| {
            let info = a.get_info();
            let name = format!("{:?}", info.backend).to_ascii_lowercase();
            let asked = !wanted.is_empty() && name.starts_with(&wanted);
            let order = match info.backend {
                wgpu::Backend::Vulkan => 0,
                wgpu::Backend::Dx12 => 1,
                _ => 2,
            };
            (!asked, info.device_type == wgpu::DeviceType::Cpu, info.device_type != wgpu::DeviceType::DiscreteGpu && info.backend != wgpu::Backend::Gl, order)
        };
        adapters.sort_by_key(rank);
        let adapter = adapters.into_iter().next()?;
        let info = adapter.get_info();
        eprintln!("[gfx] adattatore: {:?} | {} | {:?}", info.backend, info.name, info.device_type);
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::TEXTURE_COMPRESSION_BC,
                required_limits: wgpu::Limits { max_bind_groups: 4, ..Default::default() },
                memory_hints: Default::default(),
            },
            None,
        ))
        .ok()?;
        let surface = surface.zip(window).and_then(|(s, w)| {
            let size = w.inner_size();
            let mut config = s.get_default_config(&adapter, size.width.max(1), size.height.max(1))?;
            // Il gioco scrive colori già pronti per lo schermo: niente conversione sRGB.
            let caps = s.get_capabilities(&adapter);
            if let Some(f) = caps.formats.iter().find(|f| !f.is_srgb()) {
                config.format = *f;
            }
            // Senza attesa del refresh: il gioco disegna da un thread solo, e aspettare lo schermo
            // dentro `Present` gli toglie tempo.
            config.present_mode = wgpu::PresentMode::AutoNoVsync;
            s.configure(&device, &config);
            eprintln!("[gfx] finestra: {}x{} {:?}; modalità di presentazione offerte: {:?}", config.width, config.height, config.format, caps.present_modes);
            Some((s, config, w))
        });
        // Un errore di validazione non deve abbattere il gioco: lo si segnala e si va avanti.
        device.on_uncaptured_error(Box::new(|e| {
            let text = e.to_string();
            eprintln!("[gfx] ERRORE wgpu: {}", text.lines().take(6).collect::<Vec<_>>().join(" | "));
        }));
        let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(CONSTS_SIZE) },
            count: None,
        };
        // Costanti del vertex shader nel gruppo 0, del pixel shader nel gruppo 3.
        let g0_layout = [wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT].map(|stage| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[uniform(0, stage)] }));
        // I buffer riscritti a ogni invio (costanti e dati di appoggio) esistono in più copie,
        // usate a rotazione: riscrivere un buffer che la GPU sta ancora leggendo per il
        // fotogramma precedente obbliga il driver ad aspettarla.
        let frame_set = || {
            let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: UBUF_SIZE, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let binding = |b| wgpu::BindGroupEntry {
                binding: b,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &ubuf, offset: 0, size: wgpu::BufferSize::new(CONSTS_SIZE) }),
            };
            let g0 = [0, 1].map(|i| device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &g0_layout[i], entries: &[binding(0)] }));
            let scratch = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: SCRATCH_SIZE, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            (ubuf, g0, scratch)
        };
        let ring_len = std::env::var("ALTAIR_RING").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(4).max(1);
        let ring: std::collections::VecDeque<_> = (1..ring_len).map(|_| frame_set()).collect();
        let (ubuf, g0, scratch) = frame_set();
        let zero_vb = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 64, usage: wgpu::BufferUsages::VERTEX, mapped_at_creation: false });
        let dummy = |dim, layers, depth, view_dim| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: if dim == wgpu::TextureDimension::D3 { depth } else { layers } },
                mip_level_count: 1,
                sample_count: 1,
                dimension: dim,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(view_dim), ..Default::default() })
        };
        let dummies = [
            dummy(wgpu::TextureDimension::D2, 1, 1, wgpu::TextureViewDimension::D2),
            dummy(wgpu::TextureDimension::D2, 6, 1, wgpu::TextureViewDimension::Cube),
            dummy(wgpu::TextureDimension::D3, 1, 1, wgpu::TextureViewDimension::D3),
        ];
        Some(Gfx {
            device,
            submitter: spawn_submitter(queue.clone()),
            writes: Vec::new(),
            blit_sampler: None,
            queue,
            encoder: None,
            pass: None,
            pass_key: ([0; 4], 0),
            pass_formats: ([None; 4], None),
            pass_size: (0, 0),
            pending_clear: HashMap::new(),
            pending_depth_clear: HashMap::new(),
            textures: HashMap::new(),
            buffers: HashMap::new(),
            parsed: HashMap::new(),
            modules: HashMap::new(),
            pipelines: HashMap::new(),
            last_pipeline: None,
            samplers: HashMap::new(),
            g0_layout,
            g0,
            ubuf,
            ring,
            ubuf_off: 0,
            ubuf_cpu: Vec::new(),
            scratch_cpu: Vec::new(),
            tex_layouts: HashMap::new(),
            scratch,
            scratch_off: 0,
            zero_vb,
            dummies,
            blit: None,
            used: Default::default(),
            shadow: HashMap::new(),
            pool: HashMap::new(),
            retiring: Vec::new(),
            retired: Default::default(),
            recent: Default::default(),
            avoid_busy: std::env::var("ALTAIR_BUSY").is_ok_and(|v| v == "1"),
            orphaned: 0,
            pending: HashMap::new(),
            stale: Default::default(),
            flushes: 0,
            uploaded: 0,
            last: Default::default(),
            prof: Default::default(),
            prof_end: Default::default(),
            prof_finish: Default::default(),
            groups: HashMap::new(),
            consts_at: [None; 2],
            surface,
            drawn: 0,
            skipped: BTreeMap::new(),
            shot_dir: std::env::var_os("ALTAIR_SHOT_DIR").map(Into::into),
            shot_every: std::env::var("ALTAIR_SHOT_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(100),
        })
    }

    fn skip(&mut self, why: &str) {
        *self.skipped.entry(why.to_string()).or_default() += 1;
    }

    /// L'oggetto che possiede la texture di una superficie: la texture madre, o la superficie.
    fn owner(d: &D3d, surface: u32) -> (u32, u32, u32) {
        d.surface_parent.get(&surface).copied().unwrap_or((surface, 0, 0))
    }

    /// Crea, se manca, la texture wgpu di una texture o superficie D3D9.
    fn ensure_texture(&mut self, d: &D3d, obj: u32) -> bool {
        if self.textures.contains_key(&obj) {
            return true;
        }
        let (w, h, depth, levels, format, usage, kind) = match d.res.get(&obj) {
            Some(&Res::Texture { w, h, depth, levels, format, usage, .. }) => (w, h, depth, levels, format, usage, d.tex_kind.get(&obj).copied().unwrap_or(0)),
            Some(&Res::Surface { w, h, format, usage, .. }) => (w, h, 1, 1, format, usage, 0),
            _ => return false,
        };
        let is_depth = usage & 2 != 0;
        let (fmt, conv) = d3d_format(format, is_depth);
        let block = fmt.is_compressed();
        if w == 0 || h == 0 || (block && (w % 4 != 0 || h % 4 != 0)) {
            return false;
        }
        let max_levels = 32 - w.max(h).leading_zeros();
        let mut usage_bits = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC;
        if !block && kind != 2 {
            usage_bits |= wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        let (dimension, layers, dim) = match kind {
            1 => (wgpu::TextureDimension::D2, 6, wgpu::TextureViewDimension::Cube),
            2 => (wgpu::TextureDimension::D3, depth.max(1), wgpu::TextureViewDimension::D3),
            _ => (wgpu::TextureDimension::D2, 1, wgpu::TextureViewDimension::D2),
        };
        let _gate = gate();
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: layers },
            mip_level_count: levels.clamp(1, max_levels),
            sample_count: 1,
            dimension,
            format: fmt,
            usage: usage_bits,
            view_formats: &[],
        });
        self.textures.insert(obj, Tex { texture, format: fmt, w, h, dim, conv });
        true
    }

    /// Carica nella texture i dati che il gioco ha scritto tra `Lock` e `Unlock`.
    pub fn upload(&mut self, d: &D3d, mem: Mem, surface: u32, data: u32) {
        let (obj, level, face) = Self::owner(d, surface);
        if !self.ensure_texture(d, obj) {
            return self.skip("caricamento: texture non creabile");
        }
        let t = &self.textures[&obj];
        if level >= t.texture.mip_level_count() {
            return;
        }
        let (w, h) = ((t.w >> level).max(1), (t.h >> level).max(1));
        let Some(&Res::Surface { format, .. }) = d.res.get(&surface) else { return };
        let depth = if t.dim == wgpu::TextureViewDimension::D3 { (t.texture.depth_or_array_layers() >> level).max(1) } else { 1 };
        let (bytes, row, rows): (Vec<u8>, u32, u32) = if t.format.is_compressed() {
            let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
            let block = if t.format == wgpu::TextureFormat::Bc1RgbaUnorm { 8 } else { 16 };
            (mem.bytes(data, bw * bh * block).to_vec(), bw * block, bh)
        } else {
            let n = w * h * depth;
            let px: Vec<u8> = match t.conv {
                Conv::None => mem.bytes(data, n * 4).to_vec(),
                Conv::X8 => mem.bytes(data, n * 4).chunks(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
                Conv::A8 => mem.bytes(data, n).iter().flat_map(|&a| [0, 0, 0, a]).collect(),
                Conv::L8 => mem.bytes(data, n).iter().flat_map(|&l| [l, l, l, 255]).collect(),
                Conv::A8L8 => mem.bytes(data, n * 2).chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
                Conv::Unsupported => {
                    let _ = format;
                    return self.skip("caricamento: formato non gestito");
                }
            };
            (px, w * 4, h)
        };
        // I livelli più piccoli di un blocco vanno comunque scritti come un blocco intero.
        let extent = if t.format.is_compressed() { wgpu::Extent3d { width: w.div_ceil(4) * 4, height: h.div_ceil(4) * 4, depth_or_array_layers: 1 } } else { wgpu::Extent3d { width: w, height: h, depth_or_array_layers: depth } };
        let extent = wgpu::Extent3d { width: extent.width.min(w.max(4)), height: extent.height.min(h.max(4)), ..extent };
        let write = Write::Texture {
            texture: t.texture.clone(),
            level,
            origin: wgpu::Origin3d { x: 0, y: 0, z: face },
            bytes: bytes.to_vec(),
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rows) },
            extent: if t.format.is_compressed() { wgpu::Extent3d { width: w.max(4).min(extent.width.max(w)), height: h.max(4).min(extent.height.max(h)), depth_or_array_layers: 1 } } else { extent },
        };
        self.writes.push(write);
    }

    fn end_pass(&mut self) {
        self.pass = None;
        self.last = Default::default();
        self.pass_key = ([0; 4], 0);
    }

    fn flush(&mut self) {
        self.flush_and_present(None);
    }

    /// Consegna al thread di invio tutto ciò che è stato registrato finora.
    fn flush_and_present(&mut self, present: Option<wgpu::SurfaceTexture>) {
        let start = std::time::Instant::now();
        self.end_pass();
        if !self.ubuf_cpu.is_empty() {
            let data = std::mem::take(&mut self.ubuf_cpu);
            self.writes.push(Write::Buffer(self.ubuf.clone(), 0, data));
        }
        if !self.scratch_cpu.is_empty() {
            let data = std::mem::take(&mut self.scratch_cpu);
            self.writes.push(Write::Buffer(self.scratch.clone(), 0, data));
        }
        let commands = self.encoder.take().map(|e| e.finish());
        if self.writes.is_empty() && commands.is_none() && present.is_none() {
            return;
        }
        self.flushes += 1;
        let batch = Batch { writes: std::mem::take(&mut self.writes), commands, present };
        match &self.submitter {
            // Se il thread di invio è indietro di due lotti, qui si aspetta: è il freno.
            Some(tx) => {
                let _ = tx.send(Job::Batch(batch));
            }
            None => run_batch(&self.queue, batch),
        }
        self.prof[7] += start.elapsed();
        // I buffer scartati tornano disponibili dopo quattro invii, quando nessun lotto in
        // viaggio li usa più.
        self.retired.push_back(std::mem::take(&mut self.retiring));
        if self.retired.len() > 4 {
            for (key, buffer) in self.retired.pop_front().unwrap() {
                let list = self.pool.entry(key).or_default();
                if list.len() < 32 {
                    list.push(buffer);
                } else {
                    let _gate = gate();
                    drop(buffer);
                }
            }
        }
        // Il prossimo invio scrive in un'altra copia dei buffer per-fotogramma.
        if let Some((ubuf, g0, scratch)) = self.ring.pop_front() {
            let old = (std::mem::replace(&mut self.ubuf, ubuf), std::mem::replace(&mut self.g0, g0), std::mem::replace(&mut self.scratch, scratch));
            self.ring.push_back(old);
        }
        self.ubuf_off = 0;
        self.scratch_off = 0;
        self.consts_at = [None; 2];
        // Le versioni di appoggio non valgono più: i buffer veri vanno riallineati.
        self.recent.rotate_right(1);
        self.recent[0] = std::mem::take(&mut self.used);
        self.stale.extend(self.shadow.drain().map(|(obj, _)| obj));
    }

    /// Dimentica una risorsa D3D9 liberata: il suo indirizzo potrà essere riusato da un'altra.
    /// Va chiamata a fine fotogramma, senza pass aperti.
    pub fn forget(&mut self, obj: u32) {
        // Un buffer rilasciato dal gioco passa per la stessa strada di quelli scartati.
        if let Some(old) = self.buffers.remove(&obj) {
            let usage = old.usage();
            self.retiring.push(((old.size(), usage.contains(wgpu::BufferUsages::INDEX)), old));
        }
        self.shadow.remove(&obj);
        self.pending.remove(&obj);
        self.stale.remove(&obj);
        self.used.remove(&obj);
        let removed = self.textures.remove(&obj);
        if removed.is_some() {
            // Distruggere una texture è una chiamata al driver: si passa dal cancello.
            let _gate = gate();
            drop(removed);
            // I bind group che la nominano tengono viva la texture wgpu e, peggio, verrebbero
            // riusati per la texture che prenderà il suo indirizzo.
            self.groups.retain(|key, _| !key.iter().skip(1).any(|e| e[2] == obj));
        }
        self.pending_clear.remove(&obj);
        self.pending_depth_clear.remove(&obj);
    }

    /// Aspetta che il thread di invio abbia eseguito tutto ciò che gli è stato consegnato.
    fn sync(&mut self) {
        self.flush();
        if let Some(tx) = &self.submitter {
            let (done, wait) = std::sync::mpsc::channel();
            if tx.send(Job::Sync(done)).is_ok() {
                let _ = wait.recv();
            }
        }
    }

    /// Millisecondi passati dal thread di invio dentro la coda dall'ultima lettura.
    pub fn take_submit_ms(&mut self) -> f64 {
        SUBMIT_NS.swap(0, std::sync::atomic::Ordering::Relaxed) as f64 / 1e6
    }

    /// Contatori dall'ultima lettura: (invii alla GPU, byte di buffer caricati).
    pub fn take_stats(&mut self) -> (u64, u64) {
        if std::env::var_os("ALTAIR_STATS").is_some() && self.orphaned > 0 {
            eprintln!("[gfx] buffer sostituiti perché ancora in uso dalla GPU: {}", std::mem::take(&mut self.orphaned));
        }
        (std::mem::take(&mut self.flushes), std::mem::take(&mut self.uploaded))
    }

    /// Tempo passato nelle fasi di un draw dall'ultima lettura, in millisecondi: buffer,
    /// dichiarazione e shader, bind group, apertura del pass, pipeline, costanti, comandi del
    /// pass; per ultimo l'invio alla GPU.
    pub fn take_profile(&mut self) -> [f64; 8] {
        std::mem::take(&mut self.prof).map(|d| d.as_secs_f64() * 1000.0)
    }

    /// Dentro l'invio: chiusura del pass (wgpu valida e traduce i comandi) e `finish`.
    pub fn take_submit_split(&mut self) -> (f64, f64) {
        (std::mem::take(&mut self.prof_end).as_secs_f64() * 1000.0, std::mem::take(&mut self.prof_finish).as_secs_f64() * 1000.0)
    }

    fn view_of(&self, d: &D3d, surface: u32) -> Option<(wgpu::TextureView, wgpu::TextureFormat, u32, u32)> {
        let (obj, level, face) = Self::owner(d, surface);
        let t = self.textures.get(&obj)?;
        let view = t.texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_mip_level: level,
            mip_level_count: Some(1),
            base_array_layer: face,
            array_layer_count: Some(1),
            ..Default::default()
        });
        Some((view, t.format, (t.w >> level).max(1), (t.h >> level).max(1)))
    }

    /// Apre, se serve, il render pass sui target correnti. Ritorna formati dei colori e del depth.
    fn begin_pass(&mut self, d: &D3d) -> Option<([Option<wgpu::TextureFormat>; 4], Option<wgpu::TextureFormat>)> {
        let key = (d.render_targets, d.depth_target);
        // Stessi target del draw precedente, pass ancora aperto: i formati sono quelli di prima.
        if self.pass.is_some() && self.pass_key == key {
            return Some(self.pass_formats);
        }
        for s in d.render_targets.iter().chain([&d.depth_target]) {
            if *s != 0 {
                let (obj, ..) = Self::owner(d, *s);
                if !self.ensure_texture(d, obj) {
                    return None;
                }
            }
        }
        let mut colors = [None; 4];
        let mut views = Vec::new();
        let mut size = (0, 0);
        for (i, &s) in d.render_targets.iter().enumerate() {
            if s == 0 {
                views.push(None);
                continue;
            }
            let (view, format, w, h) = self.view_of(d, s)?;
            if format.is_depth_stencil_format() {
                return None;
            }
            if size == (0, 0) {
                size = (w, h);
            } else if size != (w, h) {
                views.push(None);
                continue;
            }
            colors[i] = Some(format);
            views.push(Some((view, s)));
        }
        if size == (0, 0) {
            return None;
        }
        // wgpu vuole tutti gli allegati della stessa dimensione: un depth diverso viene lasciato fuori.
        let depth = if d.depth_target != 0 { self.view_of(d, d.depth_target).filter(|v| (v.2, v.3) == size && v.1.is_depth_stencil_format()) } else { None };
        let depth_format = depth.as_ref().map(|v| v.1);
        if self.pass.is_some() && self.pass_key == key {
            return Some((colors, depth_format));
        }
        self.end_pass();
        let encoder = self.encoder.get_or_insert_with(|| self.device.create_command_encoder(&Default::default()));
        let attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = views
            .iter()
            .map(|v| {
                v.as_ref().map(|(view, s)| wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: match self.pending_clear.remove(s) {
                            Some(c) => wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: c[3] }),
                            None => wgpu::LoadOp::Load,
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
            .collect();
        let clear = self.pending_depth_clear.remove(&d.depth_target).unwrap_or((None, None));
        let depth_attachment = depth.as_ref().map(|(view, ..)| wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations { load: clear.0.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store }),
            stencil_ops: Some(wgpu::Operations { load: clear.1.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store }),
        });
        let pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &attachments,
                depth_stencil_attachment: depth_attachment,
                timestamp_writes: None,
                occlusion_query_set: None,
            })
            .forget_lifetime();
        self.pass = Some(pass);
        self.pass_key = key;
        self.pass_formats = (colors, depth_format);
        self.pass_size = size;
        Some((colors, depth_format))
    }

    /// `Clear`: ricordato e applicato all'apertura del prossimo pass su quei target.
    pub fn clear(&mut self, d: &D3d, flags: u32, color: u32, z: f32, stencil: u32) {
        self.end_pass();
        if flags & 1 != 0 {
            let c = [(color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff, color >> 24].map(|v| v as f64 / 255.0);
            for &s in d.render_targets.iter().filter(|s| **s != 0) {
                self.pending_clear.insert(s, c);
            }
        }
        if flags & 6 != 0 && d.depth_target != 0 {
            let e = self.pending_depth_clear.entry(d.depth_target).or_insert((None, None));
            if flags & 2 != 0 {
                e.0 = Some(z);
            }
            if flags & 4 != 0 {
                e.1 = Some(stencil);
            }
        }
        // Il pass va aperto subito: se nessun draw seguisse, il clear andrebbe perso.
        self.begin_pass(d);
    }

    /// `ColorFill` su un'intera superficie: un pass che fa solo il clear.
    pub fn fill(&mut self, d: &D3d, surface: u32, color: u32) {
        self.end_pass();
        let (obj, ..) = Self::owner(d, surface);
        if !self.ensure_texture(d, obj) {
            return self.skip("ColorFill: superficie non creabile");
        }
        let Some((view, format, ..)) = self.view_of(d, surface) else { return self.skip("ColorFill: superficie non creabile") };
        if format.is_depth_stencil_format() {
            return self.skip("ColorFill su un depth buffer");
        }
        let c = [(color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff, color >> 24].map(|v| v as f64 / 255.0);
        self.pending_clear.remove(&surface);
        let encoder = self.encoder.get_or_insert_with(|| self.device.create_command_encoder(&Default::default()));
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: c[3] }), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    fn module(&mut self, d: &D3d, obj: u32, opts: sh::Options) -> bool {
        if !self.parsed.contains_key(&obj) {
            let parsed = match d.res.get(&obj) {
                Some(Res::Shader { code }) => sh::parse(code).ok(),
                _ => None,
            };
            self.parsed.insert(obj, parsed);
        }
        if !self.modules.contains_key(&(obj, opts)) {
            let _gate = gate();
            let module = self.parsed[&obj].as_ref().and_then(|p| sh::translate_with(p, &opts).ok()).map(|wgsl| ShaderGpu {
                module: self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(wgsl.into()) }),
            });
            self.modules.insert((obj, opts), module);
        }
        self.modules[&(obj, opts)].is_some()
    }

    fn tex_layout(&mut self, sig: &[(u32, u32)], stage: wgpu::ShaderStages) -> wgpu::BindGroupLayout {
        if let Some(l) = self.tex_layouts.get(sig) {
            return l.clone();
        }
        let mut entries = Vec::new();
        for &(n, kind) in sig {
            let n = n & 0xff;
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2 * n,
                visibility: stage,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: match kind {
                        3 => wgpu::TextureViewDimension::Cube,
                        4 => wgpu::TextureViewDimension::D3,
                        _ => wgpu::TextureViewDimension::D2,
                    },
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry { binding: 2 * n + 1, visibility: stage, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None });
        }
        let l = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries });
        self.tex_layouts.insert(sig.to_vec(), l.clone());
        l
    }

    fn sampler(&mut self, d: &D3d, index: u32) -> wgpu::Sampler {
        let st = |ty: u32, default: u32| d.sampler_states.get(&(index, ty)).copied().unwrap_or(default);
        let key = [st(1, 1), st(2, 1), st(3, 1), st(5, 1), st(6, 1), st(7, 0)];
        if let Some(s) = self.samplers.get(&key) {
            return s.clone();
        }
        let address = |a: u32| match a {
            2 => wgpu::AddressMode::MirrorRepeat,
            3 | 4 | 5 => wgpu::AddressMode::ClampToEdge,
            _ => wgpu::AddressMode::Repeat,
        };
        let filter = |f: u32| if f >= 2 { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
        let _gate = gate();
        let s = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: address(key[0]),
            address_mode_v: address(key[1]),
            address_mode_w: address(key[2]),
            mag_filter: filter(key[3]),
            min_filter: filter(key[4]),
            mipmap_filter: filter(key[5]),
            lod_max_clamp: if key[5] == 0 { 0.0 } else { 32.0 },
            ..Default::default()
        });
        self.samplers.insert(key, s.clone());
        s
    }

    /// Bind group delle texture di uno stadio; None se una texture è anche un target del pass.
    /// I gruppi si riusano: crearne due per draw costava più del draw stesso.
    fn tex_group(&mut self, d: &D3d, sig: &[(u32, u32)], base: u32, stage: wgpu::ShaderStages) -> Option<wgpu::BindGroup> {
        let st = |index: u32, ty: u32, default: u32| d.sampler_states.get(&(index, ty)).copied().unwrap_or(default);
        let targets: Vec<u32> = d.render_targets.iter().chain([&d.depth_target]).filter(|s| **s != 0).map(|s| Self::owner(d, *s).0).collect();
        let mut key = Vec::with_capacity(sig.len() + 1);
        key.push([base, 0, 0, 0, 0, 0, 0, 0, 0]);
        for &(n, kind) in sig {
            let index = base + (n & 0xff);
            let bound = d.textures.get(&index).copied().unwrap_or(0);
            // Una texture non può essere letta e scritta nello stesso pass.
            if bound != 0 && targets.contains(&bound) {
                return None;
            }
            key.push([n, kind, bound, st(index, 1, 1), st(index, 2, 1), st(index, 3, 1), st(index, 5, 1), st(index, 6, 1), st(index, 7, 0)]);
        }
        if let Some(g) = self.groups.get(&key) {
            return Some(g.clone());
        }
        let layout = self.tex_layout(sig, stage);
        let mut views = Vec::new();
        let mut samplers = Vec::new();
        for &(n, kind) in sig {
            let bound = d.textures.get(&(base + (n & 0xff))).copied().unwrap_or(0);
            let want = match kind {
                3 => wgpu::TextureViewDimension::Cube,
                4 => wgpu::TextureViewDimension::D3,
                _ => wgpu::TextureViewDimension::D2,
            };
            let mut view = None;
            if bound != 0 && self.ensure_texture(d, bound) {
                let t = &self.textures[&bound];
                if t.dim == want && !t.format.is_depth_stencil_format() {
                    view = Some(t.texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(want), ..Default::default() }));
                }
            }
            views.push(view.unwrap_or_else(|| self.dummies[(kind.clamp(2, 4) - 2) as usize].clone()));
            samplers.push(self.sampler(d, base + (n & 0xff)));
        }
        let mut entries = Vec::new();
        for (i, &(n, _)) in sig.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: 2 * (n & 0xff), resource: wgpu::BindingResource::TextureView(&views[i]) });
            entries.push(wgpu::BindGroupEntry { binding: 2 * (n & 0xff) + 1, resource: wgpu::BindingResource::Sampler(&samplers[i]) });
        }
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &entries });
        if self.groups.len() > 20_000 {
            self.groups.clear();
        }
        self.groups.insert(key, group.clone());
        Some(group)
    }

    /// Scrive un blocco di costanti nel buffer uniforme e ne ritorna l'offset. Se è identico
    /// all'ultimo scritto per quello stadio (capita spesso tra draw consecutivi) lo riusa.
    fn write_consts(&mut self, stage: usize, f: &[f32], i: &[i32], b: u32, misc: [f32; 4], floats: usize) -> u32 {
        // Solo i registri float che lo shader legge: stanno in fondo al blocco apposta. Il
        // blocco intero pesa 4,4 KB e ne partono due per draw; la gran parte degli shader ne usa
        // una frazione.
        let f = &f[..(4 * floats).min(f.len())];
        // SAFETY: f32 e i32 non hanno byte di riempimento; l'host è little-endian come il formato atteso.
        let (fb, ib) = unsafe { (std::slice::from_raw_parts(f.as_ptr() as *const u8, f.len() * 4), std::slice::from_raw_parts(i.as_ptr() as *const u8, i.len() * 4)) };
        // Il blocco si scrive direttamente in coda al buffer del fotogramma; se è uguale
        // all'ultimo dello stesso stadio (o ne è l'inizio) lo si toglie e si riusa quello.
        let off = self.ubuf_off as usize;
        self.ubuf_cpu.truncate(off);
        self.ubuf_cpu.extend(misc.iter().flat_map(|v| v.to_le_bytes()));
        self.ubuf_cpu.extend([b, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
        self.ubuf_cpu.extend_from_slice(ib);
        self.ubuf_cpu.extend_from_slice(fb);
        let len = self.ubuf_cpu.len() - off;
        if let Some((last_off, last_len)) = self.consts_at[stage] {
            let (head, tail) = self.ubuf_cpu.split_at(off);
            if last_len >= len && head[last_off..last_off + len] == tail[..] {
                self.ubuf_cpu.truncate(off);
                return last_off as u32;
            }
        }
        let slot = (len as u64).next_multiple_of(256);
        self.ubuf_cpu.resize(off + slot as usize, 0);
        self.ubuf_off += slot;
        self.consts_at[stage] = Some((off, len));
        off as u32
    }

    fn scratch_write(&mut self, data: &[u8]) -> u64 {
        let off = self.scratch_off;
        self.scratch_cpu.extend_from_slice(data);
        self.scratch_cpu.resize(self.scratch_cpu.len().next_multiple_of(4), 0);
        self.scratch_off = self.scratch_cpu.len() as u64;
        off
    }

    /// Dà a `obj` un buffer GPU vuoto, preso dalla riserva se ce n'è uno della stessa misura.
    /// Quello di prima non si distrugge: resta ai draw già registrati e, dopo qualche invio,
    /// torna nella riserva. Creare e distruggere buffer a ogni fotogramma costava caro: sono
    /// chiamate al driver, e passano dal cancello condiviso con il thread di invio.
    fn replace_buffer(&mut self, obj: u32, padded: u64, index: bool) {
        let key = (padded, index);
        let fresh = match self.pool.get_mut(&key).and_then(Vec::pop) {
            Some(b) => b,
            None => {
                let usage = if index { wgpu::BufferUsages::INDEX } else { wgpu::BufferUsages::VERTEX } | wgpu::BufferUsages::COPY_DST;
                let _gate = gate();
                self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: padded, usage, mapped_at_creation: false })
            }
        };
        if let Some(old) = self.buffers.insert(obj, fresh) {
            self.retiring.push((key, old));
        }
    }

    /// Il buffer GPU di un vertex o index buffer D3D9, con l'offset a cui stanno i suoi dati.
    ///
    /// Un buffer riscritto dopo essere stato usato da un draw non ancora inviato non si può
    /// aggiornare sul posto (cambierebbe i dati di quel draw). Inviare tutto a ogni riscrittura
    /// costa troppo: i buffer dinamici del gioco vengono riscritti decine o centinaia di volte
    /// per fotogramma. La nuova versione va allora nel buffer di appoggio, e il buffer vero si
    /// riallinea dopo il prossimo invio.
    fn gpu_buffer(&mut self, d: &D3d, mem: Mem, obj: u32, need: (u64, u64)) -> Option<(wgpu::Buffer, u64)> {
        let &Res::Buffer { mem: data, size, index, .. } = d.res.get(&obj)? else { return None };
        if size == 0 {
            return None;
        }
        let padded = (size as u64).next_multiple_of(4);
        let fresh = !self.buffers.contains_key(&obj);
        let dirty = d.dirty_buffers.borrow_mut().remove(&obj);
        let stale = self.stale.remove(&obj);
        // `pending`: la parte del buffer vero non ancora allineata alla memoria guest.
        let widen = |g: &mut Gfx, lo: u64, hi: u64| {
            let e = g.pending.entry(obj).or_insert((lo, hi));
            *e = (e.0.min(lo), e.1.max(hi));
        };
        if fresh {
            self.replace_buffer(obj, padded, index);
            self.pending.insert(obj, (0, padded));
        } else if stale || dirty.is_some() {
            let had_shadow = self.shadow.remove(&obj).is_some();
            // Con una versione di appoggio in giro il buffer vero è indietro: va riallineato tutto.
            let (lo, hi, append, discard) = match dirty {
                Some(x) if !stale && !had_shadow => x,
                _ => (0, size, false, false),
            };
            let (lo, hi) = ((lo & !3) as u64, (hi as u64).next_multiple_of(4).min(padded));
            let in_use = self.used.contains(&obj);
            // Usato da un invio recente: la GPU può starlo ancora leggendo, e scriverci sopra
            // costringe il driver ad aspettarla. Meglio un buffer nuovo, riempito man mano
            // con ciò che i draw leggono.
            let busy = self.avoid_busy && self.recent.iter().any(|set| set.contains(&obj));
            if hi <= lo {
                // Lock senza scrittura utile.
            } else if busy && !in_use {
                self.replace_buffer(obj, padded, index);
                self.pending.insert(obj, (0, padded));
                self.orphaned += 1;
            } else if !in_use || append {
                // Nessun draw in sospeso legge quell'intervallo: si potrà aggiornare sul posto.
                widen(self, lo, hi);
            } else if discard {
                // Il gioco rinuncia al vecchio contenuto: buffer nuovo, il vecchio resta ai draw in sospeso.
                self.replace_buffer(obj, padded, index);
                self.pending.insert(obj, (lo, hi));
            } else if padded <= SHADOW_MAX && self.scratch_off + padded + (4 << 20) <= SCRATCH_SIZE {
                let off = self.scratch_write(mem.bytes(data, padded as u32));
                self.shadow.insert(obj, off);
                self.uploaded += padded;
                return Some((self.scratch.clone(), off));
            } else {
                self.replace_buffer(obj, padded, index);
                self.pending.insert(obj, (0, padded));
            }
        }
        if let Some(&off) = self.shadow.get(&obj) {
            return Some((self.scratch.clone(), off));
        }
        // Si carica ciò che questo draw legge e non è ancora allineato. Se l'intervallo cade in
        // mezzo alla parte in attesa si carica dall'inizio di questa, così ogni byte parte una
        // volta sola.
        if let Some(&(lo, hi)) = self.pending.get(&obj) {
            let (a, b) = (need.0 & !3, need.1.next_multiple_of(4).min(padded));
            if b > lo && a < hi {
                let (from, to, rest) = if a > lo && b >= hi { (a, hi, (lo, a)) } else { (lo, b.min(hi), (b.min(hi), hi)) };
                self.writes.push(Write::Buffer(self.buffers[&obj].clone(), from, mem.bytes(data + from as u32, (to - from) as u32).to_vec()));
                self.uploaded += to - from;
                if rest.1 > rest.0 {
                    self.pending.insert(obj, rest);
                } else {
                    self.pending.remove(&obj);
                }
            }
        }
        self.used.insert(obj);
        self.buffers.get(&obj).cloned().map(|b| (b, 0))
    }

    /// Un draw. `up` porta i vertici passati al volo: (dati, passo); `indexed`: (indice di
    /// partenza, vertice base).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, d: &D3d, mem: Mem, prim_type: u32, start: u32, prim_count: u32, indexed: Option<(i32, u32, u32)>, up: Option<(u32, u32)>) {
        if self.ubuf_off + 2 * CONSTS_SLOT > UBUF_SIZE || self.scratch_off + (4 << 20) > SCRATCH_SIZE {
            self.flush();
        }
        let mut mark = std::time::Instant::now();
        macro_rules! lap {
            ($i:expr) => {{
                let now = std::time::Instant::now();
                self.prof[$i] += now - mark;
                mark = now;
            }};
        }
        let verts = indexed;
        let indexed = indexed.map(|v| v.0);
        let (vs, ps, decl) = (d.vshader, d.pshader, d.decl);
        if vs == 0 || ps == 0 || decl == 0 {
            return self.skip("senza shader o dichiarazione (pipeline fissa)");
        }
        let vcount = match prim_type {
            1 => prim_count,
            2 => prim_count * 2,
            3 => prim_count + 1,
            4 => prim_count * 3,
            5 | 6 => prim_count + 2,
            _ => return self.skip("tipo di primitiva sconosciuto"),
        };
        if prim_count == 0 {
            return;
        }
        if prim_type == 6 && indexed.is_some() {
            return self.skip("ventaglio di triangoli indicizzato");
        }
        // Buffer dei vertici e degli indici, prima di aprire il pass (possono forzare un invio).
        let mut strides = [0u32; 4];
        let mut vbs: [Option<(wgpu::Buffer, u64)>; 4] = [None, None, None, None];
        if let Some((data, stride)) = up {
            let off = self.scratch_write(mem.bytes(data, vcount * stride));
            strides[0] = stride;
            vbs[0] = Some((self.scratch.clone(), off));
        } else {
            for s in 0..4 {
                let (obj, offset, stride) = d.streams[s];
                if obj != 0 {
                    // I vertici letti: da `start` per i draw semplici; per quelli indicizzati
                    // l'intervallo che il gioco dichiara (vertice base + minimo, numero).
                    let (first, count) = match verts {
                        Some((base, min, num)) if num > 0 => ((base as i64 + min as i64).max(0) as u64, num as u64),
                        Some(_) => (0, u64::MAX / (stride.max(1) as u64 * 2)),
                        None => (start as u64, vcount as u64),
                    };
                    let need = (offset as u64 + first * stride as u64, (offset as u64 + (first + count) * stride as u64).max(offset as u64 + 4));
                    let Some((b, base)) = self.gpu_buffer(d, mem, obj, need) else { return self.skip("vertex buffer non disponibile") };
                    strides[s] = stride;
                    vbs[s] = Some((b, base + offset as u64));
                }
            }
        }
        let mut index16 = true;
        let mut ib = None;
        if indexed.is_some() {
            let Some(&Res::Buffer { fvf, .. }) = d.res.get(&d.indices) else { return self.skip("index buffer mancante") };
            index16 = fvf != 102;
            let width = if index16 { 2 } else { 4 };
            ib = self.gpu_buffer(d, mem, d.indices, (start as u64 * width, (start as u64 + vcount as u64) * width));
            if ib.is_none() {
                return self.skip("index buffer non disponibile");
            }
        }
        let fan = if prim_type == 6 {
            let idx: Vec<u8> = (1..=prim_count).flat_map(|i| [0u16, i as u16, i as u16 + 1]).flat_map(|v| v.to_le_bytes()).collect();
            Some(self.scratch_write(&idx))
        } else {
            None
        };

        lap!(0);
        // Dichiarazione dei vertici: attributi e scala per tipo.
        let Some(Res::Decl { elements }) = d.res.get(&decl) else { return self.skip("dichiarazione sconosciuta") };
        let mut attrs: [Vec<wgpu::VertexAttribute>; 4] = Default::default();
        let mut provided = 0u32;
        let (mut scale255, mut scale32767) = (0u32, 0u32);
        for e in elements.chunks(8) {
            let stream = u16::from_le_bytes([e[0], e[1]]);
            if stream == 0xff {
                break;
            }
            let Some(loc) = sh::attribute_location(e[6] as u32, e[7] as u32) else { continue };
            let Some((format, scale)) = decl_format(e[4]) else { return self.skip("tipo di attributo non gestito") };
            if stream as usize >= 4 {
                continue;
            }
            match scale {
                1 => scale255 |= 1 << loc,
                2 => scale32767 |= 1 << loc,
                _ => {}
            }
            provided |= 1 << loc;
            attrs[stream as usize].push(wgpu::VertexAttribute { format, offset: u16::from_le_bytes([e[2], e[3]]) as u64, shader_location: loc });
        }
        let rs = |id: u32, default: u32| d.render_states.get(&id).copied().unwrap_or(default);
        let alpha_func = if rs(15, 0) != 0 { rs(25, 8) } else { 0 };
        let opts_vs = sh::Options { scale255, scale32767, alpha_func: 0 };
        let opts_ps = sh::Options { scale255: 0, scale32767: 0, alpha_func: if alpha_func == 8 { 0 } else { alpha_func } };
        if !self.module(d, vs, opts_vs) || !self.module(d, ps, opts_ps) {
            return self.skip("shader non traducibile");
        }
        let vs_floats = self.parsed[&vs].as_ref().unwrap().float_consts as usize;
        let ps_floats = self.parsed[&ps].as_ref().unwrap().float_consts as usize;
        let (vs_inputs, vs_samplers): (Vec<sh::Decl>, Vec<(u32, u32)>) = {
            let p = self.parsed[&vs].as_ref().unwrap();
            (p.inputs.clone(), p.samplers.iter().map(|(k, v)| (*k | 0x100, *v)).collect())
        };
        let (ps_samplers, ps_outputs): (Vec<(u32, u32)>, Vec<u32>) = {
            let p = self.parsed[&ps].as_ref().unwrap();
            (p.samplers.iter().map(|(k, v)| (*k, *v)).collect(), p.color_outputs())
        };
        // Attributi che lo shader chiede e la dichiarazione non dà: zeri da un buffer a parte.
        let mut missing = Vec::new();
        for i in &vs_inputs {
            if let Some(loc) = sh::attribute_location(i.usage, i.index) {
                if provided & (1 << loc) == 0 {
                    missing.push(wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: loc });
                }
            }
        }
        // Lo shader non deve ricevere attributi che non dichiara.
        let wanted: u32 = vs_inputs.iter().filter_map(|i| sh::attribute_location(i.usage, i.index)).fold(0, |m, l| m | 1 << l);
        for a in attrs.iter_mut() {
            a.retain(|x| wanted & (1 << x.shader_location) != 0);
        }

        if (0..4).any(|s| !attrs[s].is_empty() && vbs[s].is_none()) {
            return self.skip("flusso di vertici senza buffer");
        }
        lap!(1);
        let ps_group = self.tex_group(d, &ps_samplers, 0, wgpu::ShaderStages::FRAGMENT);
        let vs_group = self.tex_group(d, &vs_samplers, 257 - 0x100, wgpu::ShaderStages::VERTEX);
        let (Some(ps_group), Some(vs_group)) = (ps_group, vs_group) else { return self.skip("texture usata anche come render target") };

        lap!(2);
        let Some((colors, depth_format)) = self.begin_pass(d) else { return self.skip("render target non utilizzabile") };
        lap!(3);

        // Da 14 in poi: stencil (52 attivo, 53-56 fronte, 58/59 maschere, 185 due facce,
        // 186-189 retro) e scostamento di profondità (195 costante, 175 pendenza; sono float).
        // Senza stencil attivo i suoi stati valgono zero, per non moltiplicare le pipeline.
        let st = |id: u32, default: u32| if rs(52, 0) != 0 { rs(id, default) } else { 0 };
        let two = st(185, 0) != 0;
        let states = [
            rs(7, 1), rs(14, 1), rs(23, 4), rs(27, 0), rs(19, 2), rs(20, 1), rs(171, 1), rs(206, 0), rs(207, 2), rs(208, 1), rs(209, 1), rs(22, 3), rs(168, 0xf), rs(190, 0xf),
            st(52, 0), st(53, 1), st(54, 1), st(55, 1), st(56, 8), st(58, 0xffff_ffff) & 0xff, st(59, 0xffff_ffff) & 0xff,
            if two { st(186, 1) } else { st(53, 1) }, if two { st(187, 1) } else { st(54, 1) }, if two { st(188, 1) } else { st(55, 1) }, if two { st(189, 8) } else { st(56, 8) },
            rs(195, 0), rs(175, 0), 0, 0, 0,
        ];
        let key = PipeKey { vs, ps, decl, strides, opts_vs, opts_ps, colors, depth: depth_format, topology: prim_type, index16, states };
        // Draw consecutivi usano spesso la stessa pipeline: si confronta con l'ultima prima di
        // cercare nella tabella (la chiave è grossa da ridurre a un hash).
        let cached = self.last_pipeline.as_ref().filter(|(k, _)| *k == key).map(|(_, p)| p.clone());
        if cached.is_none() && !self.pipelines.contains_key(&key) {
            let _gate = gate();
            let ps_layout = self.tex_layout(&ps_samplers, wgpu::ShaderStages::FRAGMENT);
            let vs_layout = self.tex_layout(&vs_samplers, wgpu::ShaderStages::VERTEX);
            let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&self.g0_layout[0], &ps_layout, &vs_layout, &self.g0_layout[1]], push_constant_ranges: &[] });
            // Solo i flussi che portano attributi: uno slot senza attributi verrebbe comunque
            // controllato contro la dimensione del buffer legato.
            let mut buffers: Vec<wgpu::VertexBufferLayout> = (0..4)
                .filter(|s| !attrs[*s].is_empty())
                .map(|s| wgpu::VertexBufferLayout { array_stride: strides[s].max(4) as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs[s] })
                .collect();
            buffers.push(wgpu::VertexBufferLayout { array_stride: 16, step_mode: wgpu::VertexStepMode::Instance, attributes: &missing });
            let blend = (states[3] != 0).then(|| {
                let color = wgpu::BlendComponent { src_factor: blend_factor(states[4]), dst_factor: blend_factor(states[5]), operation: blend_op(states[6]) };
                let alpha = if states[7] != 0 { wgpu::BlendComponent { src_factor: blend_factor(states[8]), dst_factor: blend_factor(states[9]), operation: blend_op(states[10]) } } else { color };
                let fix = |c: wgpu::BlendComponent| if matches!(c.operation, wgpu::BlendOperation::Min | wgpu::BlendOperation::Max) { wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: c.operation } } else { c };
                wgpu::BlendState { color: fix(color), alpha: fix(alpha) }
            });
            let targets: Vec<Option<wgpu::ColorTargetState>> = colors
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    f.map(|format| wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: if ps_outputs.contains(&(i as u32)) { wgpu::ColorWrites::from_bits_truncate(if i == 0 { states[12] } else { states[13] }) } else { wgpu::ColorWrites::empty() },
                    })
                })
                .collect();
            let topology = match prim_type {
                1 => wgpu::PrimitiveTopology::PointList,
                2 => wgpu::PrimitiveTopology::LineList,
                3 => wgpu::PrimitiveTopology::LineStrip,
                5 => wgpu::PrimitiveTopology::TriangleStrip,
                _ => wgpu::PrimitiveTopology::TriangleList,
            };
            let strip = matches!(prim_type, 3 | 5);
            let vm = &self.modules[&(vs, opts_vs)].as_ref().unwrap().module;
            let pm = &self.modules[&(ps, opts_ps)].as_ref().unwrap().module;
            self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: vm, entry_point: Some("main"), buffers: &buffers, compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: pm, entry_point: Some("main"), targets: &targets, compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    strip_index_format: (strip && indexed.is_some()).then_some(if index16 { wgpu::IndexFormat::Uint16 } else { wgpu::IndexFormat::Uint32 }),
                    // In D3D9 il davanti è in senso orario; CULL_CCW (3) scarta il retro, CULL_CW (2) il davanti.
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: match states[11] {
                        2 => Some(wgpu::Face::Front),
                        3 => Some(wgpu::Face::Back),
                        _ => None,
                    },
                    ..Default::default()
                },
                depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
                    format,
                    depth_write_enabled: states[0] != 0 && states[1] != 0,
                    depth_compare: if states[0] != 0 { compare(states[2]) } else { wgpu::CompareFunction::Always },
                    stencil: if states[14] != 0 && format.has_stencil_aspect() {
                        let face = |i: usize| wgpu::StencilFaceState { compare: compare(states[i + 3]), fail_op: stencil_op(states[i]), depth_fail_op: stencil_op(states[i + 1]), pass_op: stencil_op(states[i + 2]) };
                        wgpu::StencilState { front: face(15), back: face(21), read_mask: states[19], write_mask: states[20] }
                    } else {
                        Default::default()
                    },
                    // D3DRS_DEPTHBIAS è in unità di profondità; wgpu vuole multipli del passo
                    // minimo di un buffer a 24 bit. Solo sui triangoli.
                    bias: if matches!(prim_type, 4..=6) {
                        wgpu::DepthBiasState { constant: (f32::from_bits(states[25]) * 16_777_216.0) as i32, slope_scale: f32::from_bits(states[26]), clamp: 0.0 }
                    } else {
                        Default::default()
                    },
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });
            let err = pollster::block_on(self.device.pop_error_scope());
            if let Some(e) = &err {
                let text = e.to_string();
                eprintln!("[gfx] pipeline rifiutata (vs {vs:#x}, ps {ps:#x}): {}", text.lines().take(5).collect::<Vec<_>>().join(" | "));
            }
            self.pipelines.insert(key.clone(), err.is_none().then_some(pipeline));
        }
        let pipeline = match cached {
            Some(p) => p,
            None => {
                let Some(p) = self.pipelines[&key].clone() else { return self.skip("pipeline rifiutata da wgpu") };
                self.last_pipeline = Some((key, p.clone()));
                p
            }
        };

        lap!(4);
        let (w, h) = self.pass_size;
        let half = [1.0 / w as f32, -1.0 / h as f32, rs(24, 0) as f32 / 255.0, 0.0];
        let vs_off = self.write_consts(0, &d.vs_f, &d.vs_i, d.vs_b, half, vs_floats);
        let ps_off = self.write_consts(1, &d.ps_f, &d.ps_i, d.ps_b, half, ps_floats);

        lap!(5);
        // Ogni comando del pass costa all'invio (wgpu lo ripete a OpenGL, Mesa a D3D12): si
        // manda solo ciò che è cambiato dal draw precedente dello stesso pass.
        let pass = self.pass.as_mut().unwrap();
        let last = &mut self.last;
        if last.pipeline.as_ref() != Some(&pipeline) {
            pass.set_pipeline(&pipeline);
            last.pipeline = Some(pipeline);
        }
        // D3DRS_BLENDFACTOR (193), in formato ARGB.
        let bf = rs(193, 0xffff_ffff);
        if last.blend != Some(bf) {
            pass.set_blend_constant(wgpu::Color { r: ((bf >> 16) & 0xff) as f64 / 255.0, g: ((bf >> 8) & 0xff) as f64 / 255.0, b: (bf & 0xff) as f64 / 255.0, a: (bf >> 24) as f64 / 255.0 });
            last.blend = Some(bf);
        }
        if last.consts[0] != Some(vs_off) {
            pass.set_bind_group(0, &self.g0[0], &[vs_off]);
            last.consts[0] = Some(vs_off);
        }
        if last.consts[1] != Some(ps_off) {
            pass.set_bind_group(3, &self.g0[1], &[ps_off]);
            last.consts[1] = Some(ps_off);
        }
        if last.groups[0].as_ref() != Some(&ps_group) {
            pass.set_bind_group(1, &ps_group, &[]);
            last.groups[0] = Some(ps_group);
        }
        if last.groups[1].as_ref() != Some(&vs_group) {
            pass.set_bind_group(2, &vs_group, &[]);
            last.groups[1] = Some(vs_group);
        }
        let stencil = rs(57, 0) & 0xff;
        if last.stencil != Some(stencil) {
            pass.set_stencil_reference(stencil);
            last.stencil = Some(stencil);
        }
        // D3DRS_SCISSORTESTENABLE (174): il rettangolo vale solo quando è acceso.
        let sc = if rs(174, 0) != 0 { d.scissor } else { [0, 0, w, h] };
        if last.scissor != Some(sc) {
            let (sx, sy) = (sc[0].min(w), sc[1].min(h));
            pass.set_scissor_rect(sx, sy, sc[2].clamp(sx, w) - sx, sc[3].clamp(sy, h) - sy);
            last.scissor = Some(sc);
        }
        let vp = d.viewport;
        if last.viewport != Some(vp) {
            let (vx, vy) = (vp[0].min(w.saturating_sub(1)) as f32, vp[1].min(h.saturating_sub(1)) as f32);
            let (vw, vh) = ((vp[2] as f32).min(w as f32 - vx).max(1.0), (vp[3] as f32).min(h as f32 - vy).max(1.0));
            pass.set_viewport(vx, vy, vw, vh, f32::from_bits(vp[4]).clamp(0.0, 1.0), f32::from_bits(vp[5]).clamp(0.0, 1.0));
            last.viewport = Some(vp);
        }
        let mut slot = 0;
        let zero = (self.zero_vb.clone(), 0u64);
        for (s, vb) in vbs.iter().enumerate() {
            if attrs[s].is_empty() {
                continue;
            }
            let want = vb.as_ref().unwrap_or(&zero);
            if last.vbs[slot].as_ref() != Some(want) {
                pass.set_vertex_buffer(slot as u32, want.0.slice(want.1..));
                last.vbs[slot] = Some(want.clone());
            }
            slot += 1;
        }
        if last.vbs[slot].as_ref() != Some(&zero) {
            pass.set_vertex_buffer(slot as u32, zero.0.slice(..));
            last.vbs[slot] = Some(zero);
        }
        match (indexed, fan) {
            (Some(base), _) => {
                let (b, off) = ib.unwrap();
                let want = (b, off, index16);
                if last.index.as_ref() != Some(&want) {
                    pass.set_index_buffer(want.0.slice(off..), if index16 { wgpu::IndexFormat::Uint16 } else { wgpu::IndexFormat::Uint32 });
                    last.index = Some(want);
                }
                pass.draw_indexed(start..start + vcount, base, 0..1);
            }
            (None, Some(off)) => {
                last.index = None;
                pass.set_index_buffer(self.scratch.slice(off..off + prim_count as u64 * 6), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..prim_count * 3, start as i32, 0..1);
            }
            (None, None) => pass.draw(start..start + vcount, 0..1),
        }
        self.drawn += 1;
        lap!(6);
        let _ = mark;
    }

    /// `StretchRect` e simili: copia una superficie su un'altra, scalando.
    pub fn blit(&mut self, d: &D3d, src: u32, dst: u32) {
        self.end_pass();
        let (so, do_) = (Self::owner(d, src).0, Self::owner(d, dst).0);
        if !self.ensure_texture(d, so) || !self.ensure_texture(d, do_) || so == do_ {
            return self.skip("copia tra superfici non possibile");
        }
        let (Some((sv, sf, ..)), Some((dv, df, ..))) = (self.view_of(d, src), self.view_of(d, dst)) else { return };
        if sf.is_depth_stencil_format() || df.is_depth_stencil_format() {
            return self.skip("copia di un depth buffer");
        }
        self.blit_views(&sv, &dv, df);
    }

    /// Disegna una vista su un'altra con un triangolo a tutto schermo.
    fn blit_views(&mut self, sv: &wgpu::TextureView, dv: &wgpu::TextureView, df: wgpu::TextureFormat) {
        let _gate = gate();
        if self.blit.is_none() {
            let m = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(BLIT.into()) });
            self.blit = Some((m, HashMap::new()));
        }
        let (module, cache) = self.blit.as_mut().unwrap();
        let pipeline = cache.entry(df).or_insert_with(|| {
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: wgpu::VertexState { module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module, entry_point: Some("fs"), targets: &[Some(df.into())], compilation_options: Default::default() }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        });
        let sampler = self.blit_sampler.get_or_insert_with(|| self.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() })).clone();
        drop(_gate);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(sv) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
        });
        let encoder = self.encoder.get_or_insert_with(|| self.device.create_command_encoder(&Default::default()));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: dv, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Porta il back buffer sulla finestra, se c'è, adattandolo alla sua dimensione.
    fn show(&mut self, d: &D3d) {
        let Some((surface, mut config, window)) = self.surface.take() else { return };
        self.end_pass();
        let size = window.inner_size();
        // Il cancello si tiene solo per le operazioni sulla superficie: mai mentre si consegna
        // un lotto, o i due thread si aspetterebbero a vicenda.
        let acquired = {
            let _gate = gate();
            if size.width > 0 && size.height > 0 && (size.width, size.height) != (config.width, config.height) {
                config.width = size.width;
                config.height = size.height;
                surface.configure(&self.device, &config);
            }
            surface.get_current_texture()
        };
        match acquired {
            Ok(frame) => {
                let (obj, ..) = Self::owner(d, d.backbuffer);
                if self.ensure_texture(d, obj) {
                    if let Some((sv, ..)) = self.view_of(d, d.backbuffer) {
                        let dv = frame.texture.create_view(&Default::default());
                        self.blit_views(&sv, &dv, config.format);
                    }
                }
                // La presentazione avviene dopo l'invio, sul thread di invio: se aspetta lo
                // schermo, non ferma il gioco.
                self.flush_and_present(Some(frame));
            }
            // Superficie persa o cambiata (ridimensionamento): si riconfigura e si salta un fotogramma.
            Err(_) => {
                let _gate = gate();
                surface.configure(&self.device, &config)
            }
        }
        self.surface = Some((surface, config, window));
    }

    /// Fine del fotogramma: invia i comandi e, ogni tanto, salva il back buffer.
    pub fn present(&mut self, d: &D3d, frame: u64) {
        self.show(d);
        self.flush();
        let Some(dir) = self.shot_dir.clone() else { return };
        if frame % self.shot_every != 0 && frame != 1 {
            return;
        }
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("frame-{frame:06}.png"));
        match self.save(d, d.backbuffer, &path) {
            Some((w, h, lit)) => eprintln!("[gfx] {} ({w}x{h}, {lit} pixel non neri)", path.display()),
            None => eprintln!("[gfx] back buffer non leggibile al fotogramma {frame}"),
        }
    }

    /// Salva una superficie come PNG. Ritorna (larghezza, altezza, pixel non neri).
    pub fn save(&mut self, d: &D3d, surface: u32, path: &std::path::Path) -> Option<(u32, u32, usize)> {
        let (obj, level, _) = Self::owner(d, surface);
        if !self.ensure_texture(d, obj) {
            return None;
        }
        // La lettura passa dalla coda: prima deve essere arrivato tutto il resto.
        self.sync();
        let _gate = gate();
        let t = &self.textures[&obj];
        if t.format != wgpu::TextureFormat::Bgra8Unorm {
            return None;
        }
        let (w, h) = ((t.w >> level).max(1), (t.h >> level).max(1));
        let row = (w * 4).next_multiple_of(256);
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &t.texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let data = buf.slice(..).get_mapped_range();
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for p in data[(y * row) as usize..(y * row + w * 4) as usize].chunks(4) {
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
        }
        let lit = rgba.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).ok()?), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header().ok()?.write_image_data(&rgba).ok()?;
        Some((w, h, lit))
    }

    pub fn report(&self) {
        eprintln!("[gfx] draw eseguiti: {}; pipeline: {}; texture: {}; shader tradotti: {}", self.drawn, self.pipelines.len(), self.textures.len(), self.modules.len());
        for (why, n) in &self.skipped {
            eprintln!("[gfx]   saltati {n:>8}: {why}");
        }
    }
}
