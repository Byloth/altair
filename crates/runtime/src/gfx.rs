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
const SCRATCH_SIZE: u64 = 32 << 20;

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
    states: [u32; 14],
}

pub struct Gfx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    encoder: Option<wgpu::CommandEncoder>,
    pass: Option<wgpu::RenderPass<'static>>,
    pass_key: ([u32; 4], u32),
    pass_size: (u32, u32),
    pending_clear: HashMap<u32, [f64; 4]>,
    pending_depth_clear: HashMap<u32, (Option<f32>, Option<u32>)>,
    textures: HashMap<u32, Tex>,
    buffers: HashMap<u32, wgpu::Buffer>,
    parsed: HashMap<u32, Option<sh::Shader>>,
    modules: HashMap<(u32, sh::Options), Option<ShaderGpu>>,
    pipelines: HashMap<PipeKey, Option<wgpu::RenderPipeline>>,
    samplers: HashMap<[u32; 6], wgpu::Sampler>,
    g0_layout: wgpu::BindGroupLayout,
    g0: wgpu::BindGroup,
    ubuf: wgpu::Buffer,
    ubuf_off: u64,
    tex_layouts: HashMap<Vec<(u32, u32)>, wgpu::BindGroupLayout>,
    scratch: wgpu::Buffer,
    scratch_off: u64,
    zero_vb: wgpu::Buffer,
    dummies: [wgpu::TextureView; 3],
    blit: Option<(wgpu::ShaderModule, HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>)>,
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
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() });
        let mut adapters = instance.enumerate_adapters(wgpu::Backends::all());
        adapters.sort_by_key(|a| a.get_info().device_type == wgpu::DeviceType::Cpu);
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
        let g0_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[uniform(0, wgpu::ShaderStages::VERTEX), uniform(1, wgpu::ShaderStages::FRAGMENT)],
        });
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: UBUF_SIZE, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let binding = |b| wgpu::BindGroupEntry {
            binding: b,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &ubuf, offset: 0, size: wgpu::BufferSize::new(CONSTS_SIZE) }),
        };
        let g0 = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &g0_layout, entries: &[binding(0), binding(1)] });
        let scratch = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: SCRATCH_SIZE, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
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
            queue,
            encoder: None,
            pass: None,
            pass_key: ([0; 4], 0),
            pass_size: (0, 0),
            pending_clear: HashMap::new(),
            pending_depth_clear: HashMap::new(),
            textures: HashMap::new(),
            buffers: HashMap::new(),
            parsed: HashMap::new(),
            modules: HashMap::new(),
            pipelines: HashMap::new(),
            samplers: HashMap::new(),
            g0_layout,
            g0,
            ubuf,
            ubuf_off: 0,
            tex_layouts: HashMap::new(),
            scratch,
            scratch_off: 0,
            zero_vb,
            dummies,
            blit: None,
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
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &t.texture, mip_level: level, origin: wgpu::Origin3d { x: 0, y: 0, z: face }, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rows) },
            if t.format.is_compressed() { wgpu::Extent3d { width: w.max(4).min(extent.width.max(w)), height: h.max(4).min(extent.height.max(h)), depth_or_array_layers: 1 } } else { extent },
        );
    }

    fn end_pass(&mut self) {
        self.pass = None;
        self.pass_key = ([0; 4], 0);
    }

    fn flush(&mut self) {
        self.end_pass();
        if let Some(e) = self.encoder.take() {
            self.queue.submit([e.finish()]);
        }
        self.ubuf_off = 0;
        self.scratch_off = 0;
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

    fn module(&mut self, d: &D3d, obj: u32, opts: sh::Options) -> bool {
        if !self.parsed.contains_key(&obj) {
            let parsed = match d.res.get(&obj) {
                Some(Res::Shader { code }) => sh::parse(code).ok(),
                _ => None,
            };
            self.parsed.insert(obj, parsed);
        }
        if !self.modules.contains_key(&(obj, opts)) {
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
    fn tex_group(&mut self, d: &D3d, sig: &[(u32, u32)], base: u32, stage: wgpu::ShaderStages) -> Option<wgpu::BindGroup> {
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
                // Una texture non può essere letta e scritta nello stesso pass.
                let targets = d.render_targets.iter().chain([&d.depth_target]).map(|s| Self::owner(d, *s).0);
                if targets.into_iter().any(|o| o == bound) {
                    return None;
                }
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
        Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &entries }))
    }

    fn write_consts(&mut self, f: &[f32], i: &[i32], b: u32, misc: [f32; 4]) -> u32 {
        let mut bytes = Vec::with_capacity(CONSTS_SIZE as usize);
        bytes.extend(f.iter().flat_map(|v| v.to_le_bytes()));
        bytes.extend(i.iter().flat_map(|v| v.to_le_bytes()));
        bytes.extend([b, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
        bytes.extend(misc.iter().flat_map(|v| v.to_le_bytes()));
        let off = self.ubuf_off;
        self.queue.write_buffer(&self.ubuf, off, &bytes);
        self.ubuf_off += CONSTS_SLOT;
        off as u32
    }

    fn scratch_write(&mut self, data: &[u8]) -> u64 {
        let off = self.scratch_off;
        let mut padded;
        let data = if data.len() % 4 != 0 {
            padded = data.to_vec();
            padded.resize(data.len().next_multiple_of(4), 0);
            &padded[..]
        } else {
            data
        };
        self.queue.write_buffer(&self.scratch, off, data);
        self.scratch_off += data.len() as u64;
        off
    }

    fn gpu_buffer(&mut self, d: &D3d, mem: Mem, obj: u32) -> Option<wgpu::Buffer> {
        let &Res::Buffer { mem: data, size, index, .. } = d.res.get(&obj)? else { return None };
        if size == 0 {
            return None;
        }
        let padded = (size as u64).next_multiple_of(4);
        let fresh = !self.buffers.contains_key(&obj);
        if fresh {
            let usage = if index { wgpu::BufferUsages::INDEX } else { wgpu::BufferUsages::VERTEX } | wgpu::BufferUsages::COPY_DST;
            self.buffers.insert(obj, self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: padded, usage, mapped_at_creation: false }));
        }
        if fresh || d.dirty_buffers.borrow_mut().remove(&obj) {
            // Scrivere in un buffer usato da un draw già registrato ne cambierebbe i dati.
            self.flush_if_used(obj);
            self.queue.write_buffer(&self.buffers[&obj], 0, mem.bytes(data, padded as u32));
        }
        self.buffers.get(&obj).cloned()
    }

    fn flush_if_used(&mut self, _obj: u32) {
        // Semplice e prudente: prima di riscrivere un buffer si inviano i comandi in sospeso.
        if self.encoder.is_some() {
            let key = self.pass_key;
            self.end_pass();
            if let Some(e) = self.encoder.take() {
                self.queue.submit([e.finish()]);
            }
            self.ubuf_off = 0;
            self.scratch_off = 0;
            let _ = key;
        }
    }

    /// Un draw. `up` porta i vertici passati al volo: (dati, passo); `indexed`: (indice di
    /// partenza, vertice base).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, d: &D3d, mem: Mem, prim_type: u32, start: u32, prim_count: u32, indexed: Option<i32>, up: Option<(u32, u32)>) {
        if self.ubuf_off + 2 * CONSTS_SLOT > UBUF_SIZE || self.scratch_off + (4 << 20) > SCRATCH_SIZE {
            self.flush();
        }
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
                    let Some(b) = self.gpu_buffer(d, mem, obj) else { return self.skip("vertex buffer non disponibile") };
                    strides[s] = stride;
                    vbs[s] = Some((b, offset as u64));
                }
            }
        }
        let mut index16 = true;
        let mut ib = None;
        if indexed.is_some() {
            let Some(&Res::Buffer { fvf, .. }) = d.res.get(&d.indices) else { return self.skip("index buffer mancante") };
            index16 = fvf != 102;
            ib = self.gpu_buffer(d, mem, d.indices);
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
        let ps_group = self.tex_group(d, &ps_samplers, 0, wgpu::ShaderStages::FRAGMENT);
        let vs_group = self.tex_group(d, &vs_samplers, 257 - 0x100, wgpu::ShaderStages::VERTEX);
        let (Some(ps_group), Some(vs_group)) = (ps_group, vs_group) else { return self.skip("texture usata anche come render target") };

        let Some((colors, depth_format)) = self.begin_pass(d) else { return self.skip("render target non utilizzabile") };

        let states = [rs(7, 1), rs(14, 1), rs(23, 4), rs(27, 0), rs(19, 2), rs(20, 1), rs(171, 1), rs(206, 0), rs(207, 2), rs(208, 1), rs(209, 1), rs(22, 3), rs(168, 0xf), rs(190, 0xf)];
        let key = PipeKey { vs, ps, decl, strides, opts_vs, opts_ps, colors, depth: depth_format, topology: prim_type, index16, states };
        if !self.pipelines.contains_key(&key) {
            let ps_layout = self.tex_layout(&ps_samplers, wgpu::ShaderStages::FRAGMENT);
            let vs_layout = self.tex_layout(&vs_samplers, wgpu::ShaderStages::VERTEX);
            let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&self.g0_layout, &ps_layout, &vs_layout], push_constant_ranges: &[] });
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
                    stencil: Default::default(),
                    bias: Default::default(),
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
        let Some(pipeline) = self.pipelines[&key].clone() else { return self.skip("pipeline rifiutata da wgpu") };

        let (w, h) = self.pass_size;
        let half = [1.0 / w as f32, -1.0 / h as f32, rs(24, 0) as f32 / 255.0, 0.0];
        let vs_off = self.write_consts(&d.vs_f, &d.vs_i, d.vs_b, half);
        let ps_off = self.write_consts(&d.ps_f, &d.ps_i, d.ps_b, half);

        let pass = self.pass.as_mut().unwrap();
        pass.set_pipeline(&pipeline);
        // D3DRS_BLENDFACTOR (193), in formato ARGB.
        let bf = rs(193, 0xffff_ffff);
        pass.set_blend_constant(wgpu::Color { r: ((bf >> 16) & 0xff) as f64 / 255.0, g: ((bf >> 8) & 0xff) as f64 / 255.0, b: (bf & 0xff) as f64 / 255.0, a: (bf >> 24) as f64 / 255.0 });
        pass.set_bind_group(0, &self.g0, &[vs_off, ps_off]);
        pass.set_bind_group(1, &ps_group, &[]);
        pass.set_bind_group(2, &vs_group, &[]);
        let vp = d.viewport;
        let (vx, vy) = (vp[0].min(w.saturating_sub(1)) as f32, vp[1].min(h.saturating_sub(1)) as f32);
        let (vw, vh) = ((vp[2] as f32).min(w as f32 - vx).max(1.0), (vp[3] as f32).min(h as f32 - vy).max(1.0));
        pass.set_viewport(vx, vy, vw, vh, f32::from_bits(vp[4]).clamp(0.0, 1.0), f32::from_bits(vp[5]).clamp(0.0, 1.0));
        let mut slot = 0;
        for (s, vb) in vbs.iter().enumerate() {
            if attrs[s].is_empty() {
                continue;
            }
            match vb {
                Some((b, off)) => pass.set_vertex_buffer(slot, b.slice(*off..)),
                None => pass.set_vertex_buffer(slot, self.zero_vb.slice(..)),
            }
            slot += 1;
        }
        pass.set_vertex_buffer(slot, self.zero_vb.slice(..));
        match (indexed, fan) {
            (Some(base), _) => {
                pass.set_index_buffer(ib.as_ref().unwrap().slice(..), if index16 { wgpu::IndexFormat::Uint16 } else { wgpu::IndexFormat::Uint32 });
                pass.draw_indexed(start..start + vcount, base, 0..1);
            }
            (None, Some(off)) => {
                pass.set_index_buffer(self.scratch.slice(off..off + prim_count as u64 * 6), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..prim_count * 3, start as i32, 0..1);
            }
            (None, None) => pass.draw(start..start + vcount, 0..1),
        }
        self.drawn += 1;
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
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&sv) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
        });
        let encoder = self.encoder.get_or_insert_with(|| self.device.create_command_encoder(&Default::default()));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &dv, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Fine del fotogramma: invia i comandi e, ogni tanto, salva il back buffer.
    pub fn present(&mut self, d: &D3d, frame: u64) {
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
