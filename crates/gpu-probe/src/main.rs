//! Elenca gli adattatori grafici che wgpu vede su questa macchina, per ogni backend.
//! Serve a decidere dove può girare il renderer (vedi docs/analisi/d3d9.md).

fn main() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() });
    let adapters = instance.enumerate_adapters(wgpu::Backends::all());
    if adapters.is_empty() {
        println!("nessun adattatore trovato");
    }
    for a in adapters {
        let i = a.get_info();
        let l = a.limits();
        println!(
            "{:?} | {} | tipo {:?} | driver {} {} | texture 2D max {} | render target {}",
            i.backend, i.name, i.device_type, i.driver, i.driver_info, l.max_texture_dimension_2d, l.max_color_attachments
        );
        let f = a.features();
        println!(
            "    compressione BC (DXT): {} | float32 filtrabile: {} | query di occlusione: sempre",
            f.contains(wgpu::Features::TEXTURE_COMPRESSION_BC),
            f.contains(wgpu::Features::FLOAT32_FILTERABLE)
        );
    }
}
