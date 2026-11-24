//! GPU backend for the D3Q19 solver, using wgpu (Vulkan on Linux, so AMD
//! cards work). It runs the same arithmetic as `cfd_lbm::Lbm::advance`; the
//! CPU solver prepares the grid, walls and wheels, then hands them over.

use anyhow::{anyhow, bail, Context, Result};
use cfd_lbm::{inflow_at, mirror_z, opposite, Config, Ground, Lbm, PartForce, StepStats, C, Q, W};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    nx: u32,
    ny: u32,
    nz: u32,
    n: u32,
    gx: u32,
    bits: u32,
    step: u32,
    pad: u32,
    tau0: f32,
    cs2: f32,
    u_ramp: f32,
    ground_u: f32,
}

pub struct GpuLbm {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind: [wgpu::BindGroup; 2],
    f: [wgpu::Buffer; 2],
    force: wgpu::Buffer,
    stats: wgpu::Buffer,
    params: wgpu::Buffer,
    force_read: wgpu::Buffer,
    stats_read: wgpu::Buffer,
    groups: (u32, u32),
    n: usize,
    nx: usize,
    ny: usize,
    nz: usize,
    boundary_parts: Vec<u16>,
    n_parts: usize,
    config: Config,
    moving: bool,
    /// Index of the buffer holding the current populations.
    current: usize,
    pub step: usize,
    pub adapter: String,
}

fn tables() -> String {
    let list = |v: Vec<String>| v.join(", ");
    format!(
        "var<private> CX: array<i32, 19> = array<i32, 19>({});\n\
         var<private> CY: array<i32, 19> = array<i32, 19>({});\n\
         var<private> CZ: array<i32, 19> = array<i32, 19>({});\n\
         var<private> WQ: array<f32, 19> = array<f32, 19>({});\n\
         var<private> OPP: array<u32, 19> = array<u32, 19>({});\n\
         var<private> MZ: array<u32, 19> = array<u32, 19>({});\n",
        list((0..Q).map(|q| C[q][0].to_string()).collect()),
        list((0..Q).map(|q| C[q][1].to_string()).collect()),
        list((0..Q).map(|q| C[q][2].to_string()).collect()),
        list((0..Q).map(|q| format!("{:.9}", W[q])).collect()),
        list((0..Q).map(|q| format!("{}u", opposite(q))).collect()),
        list((0..Q).map(|q| format!("{}u", mirror_z(q))).collect()),
    )
}

pub fn shader_source() -> String {
    include_str!("step.wgsl").replace("//TABLES", &tables())
}

/// Name of the GPU wgpu would pick, if there is one.
pub fn probe() -> Option<String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    let info = adapter.get_info();
    let soft = if info.device_type == wgpu::DeviceType::Cpu { ", software" } else { "" };
    Some(format!("{} ({:?}{soft})", info.name, info.backend))
}

/// Largest grid (cells) the GPU's per-buffer limit allows.
pub fn max_cells() -> Option<usize> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    let l = adapter.limits();
    let bytes = (l.max_storage_buffer_binding_size as u64).min(l.max_buffer_size);
    Some((bytes / (Q as u64 * 4)) as usize)
}

impl GpuLbm {
    /// Copies a prepared CPU solver (grid, wall distances, wheel speeds and
    /// current populations) onto the GPU.
    pub fn new(lbm: &Lbm) -> Result<Self> {
        Self::new_on(lbm, true)
    }

    /// As `new`; with `allow_software` false, a CPU-emulated adapter such as
    /// llvmpipe is refused, because it is slower than the CPU solver.
    pub fn new_on(lbm: &Lbm, allow_software: bool) -> Result<Self> {
        let (wall_index, wall_delta) = lbm.wall_links();
        if wall_index.is_empty() {
            bail!("call set_wall_distances before moving the solver to the GPU");
        }
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok_or_else(|| anyhow!("no GPU found (wgpu found no Vulkan, Metal or DirectX 12 adapter)"))?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?})", info.name, info.backend);
        if !allow_software && info.device_type == wgpu::DeviceType::Cpu {
            bail!("only a software GPU ({adapter_name}) was found, which is slower than the CPU");
        }
        let limits = adapter.limits();
        let n = lbm.cells();
        let f_bytes = (n * Q * 4) as u64;
        let max = (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size);
        if f_bytes > max {
            bail!(
                "this grid needs {:.1} GB per buffer but {adapter_name} allows {:.1} GB; pick a lower quality",
                f_bytes as f64 / 1e9,
                max as f64 / 1e9
            );
        }
        if limits.max_storage_buffers_per_shader_stage < 8 {
            bail!("{adapter_name} allows too few storage buffers per shader");
        }
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("cfd"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .context("could not open the GPU")?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("step"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("step"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        let init = |label: &str, bytes: &[u8]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytes, usage: storage })
        };
        let make_f = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: f_bytes, usage: storage, mapped_at_creation: false })
        };
        let f = [make_f("f0"), make_f("f1")];
        let pops = lbm.populations();
        let mut column = vec![0f32; n];
        for q in 0..Q {
            for (c, v) in column.iter_mut().enumerate() {
                *v = pops[c * Q + q];
            }
            let at = (q * n * 4) as u64;
            queue.write_buffer(&f[0], at, bytemuck::cast_slice(&column));
            queue.write_buffer(&f[1], at, bytemuck::cast_slice(&column));
            queue.submit(None);
        }
        drop(column);
        let flags: Vec<u32> = lbm.flags.iter().map(|&x| x as u32).collect();
        let flags_buf = init("flags", bytemuck::cast_slice(&flags));
        let widx_buf = init("wall_index", bytemuck::cast_slice(wall_index));
        let mut deltas: Vec<f32> = wall_delta.iter().flat_map(|d| d.iter().copied()).collect();
        if deltas.is_empty() {
            deltas = vec![0.5; Q];
        }
        let wdelta_buf = init("wall_delta", bytemuck::cast_slice(&deltas));
        let wu = lbm.wall_velocities();
        let moving = !wu.is_empty();
        let wall_u: Vec<f32> = if moving { wu.iter().flat_map(|v| v.iter().copied()).collect() } else { vec![0.0; 4] };
        let wallu_buf = init("wall_u", bytemuck::cast_slice(&wall_u));
        let nb = wall_delta.len().max(1);
        let force = init("force", &vec![0u8; nb * 16]);
        let stats = init("stats", &[0u8; 8]);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let force_read = read("force_read", (nb * 16) as u64);
        let stats_read = read("stats_read", 8);
        let layout = pipeline.get_bind_group_layout(0);
        fn e(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }
        }
        let bind_for = |src: &wgpu::Buffer, dst: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[
                    e(0, &params),
                    e(1, src),
                    e(2, dst),
                    e(3, &flags_buf),
                    e(4, &widx_buf),
                    e(5, &wdelta_buf),
                    e(6, &wallu_buf),
                    e(7, &force),
                    e(8, &stats),
                ],
            })
        };
        let bind = [bind_for(&f[0], &f[1]), bind_for(&f[1], &f[0])];
        let total = n.div_ceil(64) as u32;
        let gx = total.min(65_535);
        let gy = total.div_ceil(gx);
        Ok(Self {
            device,
            queue,
            pipeline,
            bind,
            f,
            force,
            stats,
            params,
            force_read,
            stats_read,
            groups: (gx, gy),
            n,
            nx: lbm.nx,
            ny: lbm.ny,
            nz: lbm.nz,
            boundary_parts: lbm.boundary_parts(),
            n_parts: lbm.n_parts,
            config: lbm.config,
            moving,
            current: 0,
            step: lbm.step,
            adapter: adapter_name,
        })
    }

    fn params_for(&self, step: usize) -> Params {
        let u_ramp = inflow_at(&self.config, step);
        Params {
            nx: self.nx as u32,
            ny: self.ny as u32,
            nz: self.nz as u32,
            n: self.n as u32,
            gx: self.groups.0,
            bits: u32::from(self.config.ground == Ground::Slip) | (u32::from(self.moving) << 1),
            step: step as u32,
            pad: 0,
            tau0: (3.0 * self.config.nu + 0.5) as f32,
            cs2: (self.config.smagorinsky * self.config.smagorinsky) as f32,
            u_ramp,
            ground_u: if self.config.ground == Ground::Moving { u_ramp } else { 0.0 },
        }
    }

    /// Runs `steps` steps, then reads back the forces of the last one.
    pub fn advance(&mut self, steps: usize) -> StepStats {
        let mut done = 0;
        while done < steps {
            // While the inflow ramps up every step needs its own parameters;
            // after that, many steps share one submission.
            let ramping = self.step < self.config.ramp_steps + 1;
            let batch = if ramping { 1 } else { (steps - done).min(64) };
            self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&self.params_for(self.step)));
            let mut enc = self.device.create_command_encoder(&Default::default());
            for _ in 0..batch {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind[self.current], &[]);
                pass.dispatch_workgroups(self.groups.0, self.groups.1, 1);
                drop(pass);
                self.current = 1 - self.current;
                self.step += 1;
            }
            self.queue.submit(Some(enc.finish()));
            done += batch;
        }
        self.read_stats()
    }

    fn read_stats(&mut self) -> StepStats {
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.force, 0, &self.force_read, 0, self.force.size());
        enc.copy_buffer_to_buffer(&self.stats, 0, &self.stats_read, 0, 8);
        self.queue.submit(Some(enc.finish()));
        let fslice = self.force_read.slice(..);
        let sslice = self.stats_read.slice(..);
        fslice.map_async(wgpu::MapMode::Read, |_| {});
        sslice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let mut parts = vec![PartForce::default(); self.n_parts];
        let mut finite = true;
        {
            let data = fslice.get_mapped_range();
            let v: &[f32] = bytemuck::cast_slice(&data);
            for (k, &p) in self.boundary_parts.iter().enumerate() {
                let a = &mut parts[p as usize];
                let c = &v[k * 4..k * 4 + 4];
                if !c.iter().all(|x| x.is_finite()) {
                    finite = false;
                }
                a.fx += c[0] as f64;
                a.fy += c[1] as f64;
                a.fz += c[2] as f64;
                a.fz_x += c[3] as f64;
            }
        }
        let (max2, bad) = {
            let data = sslice.get_mapped_range();
            let s: &[u32] = bytemuck::cast_slice(&data);
            (f32::from_bits(s[0]), s[1] != 0)
        };
        self.force_read.unmap();
        self.stats_read.unmap();
        self.queue.write_buffer(&self.stats, 0, &[0u8; 8]);
        StepStats {
            step: self.step,
            parts,
            max_speed: (max2 as f64).sqrt(),
            max_speed_cell: 0,
            finite: finite && !bad,
        }
    }

    /// Copies the current populations back into the CPU solver, for slices,
    /// ParaView output and comparisons.
    pub fn download_into(&self, lbm: &mut Lbm) {
        let size = (self.n * Q * 4) as u64;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("download"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.f[self.current], 0, &staging, 0, size);
        self.queue.submit(Some(enc.finish()));
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        {
            let data = slice.get_mapped_range();
            let soa: &[f32] = bytemuck::cast_slice(&data);
            let out = lbm.populations_mut();
            for q in 0..Q {
                for (c, &v) in soa[q * self.n..(q + 1) * self.n].iter().enumerate() {
                    out[c * Q + q] = v;
                }
            }
        }
        staging.unmap();
        lbm.step = self.step;
    }

    pub fn cells(&self) -> usize {
        self.n
    }
}
