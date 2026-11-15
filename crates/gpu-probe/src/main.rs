//! Tests of the graphics stack on this machine (see docs/analysis/d3d9.md).
//!
//!   altair-gpu-probe                 lists the adapters wgpu sees
//!   altair-gpu-probe triangle F.png  draws a triangle off-screen and saves it
//!   altair-gpu-probe window [sec]    opens a window with the triangle for a few seconds
//!
//! To use the GPU from WSL: GALLIUM_DRIVER=d3d12 LD_LIBRARY_PATH=/usr/lib/wsl/lib

use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const SHADER: &str = "
struct Out { @builtin(position) pos: vec4f, @location(0) color: vec3f }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    var p = array<vec2f, 3>(vec2f(0.0, 0.7), vec2f(-0.7, -0.6), vec2f(0.7, -0.6));
    var c = array<vec3f, 3>(vec3f(1.0, 0.2, 0.2), vec3f(0.2, 1.0, 0.2), vec3f(0.2, 0.4, 1.0));
    return Out(vec4f(p[i], 0.0, 1.0), c[i]);
}
@fragment fn fs(v: Out) -> @location(0) vec4f { return vec4f(v.color, 1.0); }
";

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() })
}

fn describe(a: &wgpu::Adapter) -> String {
    let i = a.get_info();
    format!("{:?} | {} | type {:?} | {}", i.backend, i.name, i.device_type, i.driver_info)
}

/// The best adapter: a real GPU first, then whatever is there.
fn pick(instance: &wgpu::Instance, surface: Option<&wgpu::Surface>) -> wgpu::Adapter {
    let mut all = instance.enumerate_adapters(wgpu::Backends::all());
    if let Some(s) = surface {
        all.retain(|a| a.is_surface_supported(s));
    }
    all.sort_by_key(|a| a.get_info().device_type == wgpu::DeviceType::Cpu);
    all.into_iter().next().expect("no usable graphics adapter")
}

fn pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), targets: &[Some(format.into())], compilation_options: Default::default() }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

fn draw(device: &wgpu::Device, queue: &wgpu::Queue, pipeline: &wgpu::RenderPipeline, view: &wgpu::TextureView) -> wgpu::CommandEncoder {
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.05, b: 0.08, a: 1.0 }), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.draw(0..3, 0..1);
    }
    let _ = queue;
    enc
}

fn triangle(path: &str) {
    let instance = instance();
    let adapter = pick(&instance, None);
    println!("adapter: {}", describe(&adapter));
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None)).expect("device");
    let (w, h) = (640u32, 360u32);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let pipe = pipeline(&device, format);
    let mut enc = draw(&device, &queue, &pipe, &view);
    let row = (w * 4).next_multiple_of(256);
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("buffer read"));
    device.poll(wgpu::Maintain::Wait);
    let data = buf.slice(..).get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).expect("output file")), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.write_header().unwrap().write_image_data(&pixels).unwrap();
    let lit = pixels.chunks(4).filter(|p| p[0] > 40 || p[1] > 40 || p[2] > 60).count();
    println!("wrote {path}: {w}x{h}, {lit} pixels colored by the triangle");
}

struct App {
    seconds: f32,
    start: std::time::Instant,
    state: Option<(Arc<Window>, wgpu::Surface<'static>, wgpu::Device, wgpu::Queue, wgpu::RenderPipeline)>,
    frames: u32,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = Arc::new(el.create_window(Window::default_attributes().with_title("Altair: graphics test").with_inner_size(winit::dpi::LogicalSize::new(640, 360))).expect("window"));
        let instance = instance();
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = pick(&instance, Some(&surface));
        println!("adapter: {}", describe(&adapter));
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None)).expect("device");
        let size = window.inner_size();
        let config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface configuration");
        println!("surface: {}x{} {:?}", config.width, config.height, config.format);
        surface.configure(&device, &config);
        let pipe = pipeline(&device, config.format);
        self.start = std::time::Instant::now();
        self.state = Some((window, surface, device, queue, pipe));
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some((window, surface, device, queue, pipe)) = &self.state else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => {
                if let Ok(frame) = surface.get_current_texture() {
                    let view = frame.texture.create_view(&Default::default());
                    let enc = draw(device, queue, pipe, &view);
                    queue.submit([enc.finish()]);
                    frame.present();
                    self.frames += 1;
                }
                if self.start.elapsed().as_secs_f32() >= self.seconds {
                    println!("{} frames presented in {:.1} s", self.frames, self.start.elapsed().as_secs_f32());
                    el.exit();
                } else {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some((window, ..)) = &self.state {
            window.request_redraw();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("triangle") => triangle(args.get(1).map_or("triangle.png", String::as_str)),
        Some("window") => {
            let seconds = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
            let el = EventLoop::new().expect("event loop");
            let mut app = App { seconds, start: std::time::Instant::now(), state: None, frames: 0 };
            el.run_app(&mut app).expect("event loop run");
        }
        _ => {
            let adapters = instance().enumerate_adapters(wgpu::Backends::all());
            if adapters.is_empty() {
                println!("no adapter found");
            }
            for a in adapters {
                let (l, f) = (a.limits(), a.features());
                println!("{} | max 2D texture {} | render targets {}", describe(&a), l.max_texture_dimension_2d, l.max_color_attachments);
                println!("    BC (DXT) compression: {} | filterable float32: {}", f.contains(wgpu::Features::TEXTURE_COMPRESSION_BC), f.contains(wgpu::Features::FLOAT32_FILTERABLE));
            }
        }
    }
}
