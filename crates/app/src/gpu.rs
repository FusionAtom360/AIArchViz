//! The 3D viewer's wgpu renderer (D-14): the scene is drawn into a texture
//! of its own, with its own depth buffer, and shown in egui as an image.
//!
//! Vertices arrive relative to the eye in meters (camera.rs), so the GPU
//! never sees a large coordinate. Depth is reversed (near 1, infinity 0)
//! in a 32-bit float buffer, which keeps depth precise from 1/16 in to miles.
//! Faces are double-sided, since a section plane shows their backs.

use eframe::egui;
use eframe::egui_wgpu::{RenderState, wgpu};

/// One vertex: position relative to the eye (m), face normal (zero for a
/// line) and color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}

const VERTEX_SIZE: u64 = 40;
const UNIFORM_SIZE: u64 = 80;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const SHADER: &str = r"
struct U { r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, r3: vec4<f32>, light: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;

struct In {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

fn project(p: vec3<f32>) -> vec4<f32> {
    let q = vec4<f32>(p, 1.0);
    return vec4<f32>(dot(u.r0, q), dot(u.r1, q), dot(u.r2, q), dot(u.r3, q));
}

@vertex fn vs_fill(v: In) -> Out {
    var o: Out;
    o.clip = project(v.pos);
    let shade = 0.45 + 0.55 * abs(dot(normalize(v.normal), u.light.xyz));
    o.color = vec4<f32>(v.color.rgb * shade, v.color.a);
    return o;
}

// Lines sit a hair nearer than the faces they outline.
@vertex fn vs_line(v: In) -> Out {
    var o: Out;
    o.clip = project(v.pos);
    o.clip.z = o.clip.z * 1.002;
    o.color = v.color;
    return o;
}

@fragment fn fs(i: Out) -> @location(0) vec4<f32> {
    return i.color;
}
";

struct Target {
    size: [u32; 2],
    #[cfg_attr(not(test), allow(dead_code))]
    texture: wgpu::Texture,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    id: egui::TextureId,
}

pub struct Gpu {
    fill: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    target: Option<Target>,
    vertices: Option<(wgpu::Buffer, u64)>,
}

fn bytes(vertices: &[Vertex]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vertices.len() * VERTEX_SIZE as usize);
    for v in vertices {
        for f in v.pos.iter().chain(&v.normal).chain(&v.color) {
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
    out
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: VERTEX_SIZE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology,
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Gpu {
    pub fn new(rs: &RenderState) -> Gpu {
        let device = &rs.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewer"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let group = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewer"),
            bind_group_layouts: &[Some(&group)],
            immediate_size: 0,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer uniform"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewer"),
            layout: &group,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Gpu {
            fill: pipeline(
                device,
                &layout,
                &module,
                "vs_fill",
                wgpu::PrimitiveTopology::TriangleList,
            ),
            line: pipeline(
                device,
                &layout,
                &module,
                "vs_line",
                wgpu::PrimitiveTopology::LineList,
            ),
            uniform,
            bind_group,
            target: None,
            vertices: None,
        }
    }

    fn target(&mut self, rs: &RenderState, size: [u32; 2]) -> &Target {
        let stale = self.target.as_ref().is_none_or(|t| t.size != size);
        if stale {
            let make = |format, usage| {
                rs.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("viewer"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let texture = make(
                COLOR_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
            );
            let color = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let depth = make(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT)
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut renderer = rs.renderer.write();
            let id = match self.target.take() {
                Some(old) => {
                    renderer.update_egui_texture_from_wgpu_texture(
                        &rs.device,
                        &color,
                        wgpu::FilterMode::Nearest,
                        old.id,
                    );
                    old.id
                }
                None => {
                    renderer.register_native_texture(&rs.device, &color, wgpu::FilterMode::Nearest)
                }
            };
            self.target = Some(Target {
                size,
                texture,
                color,
                depth,
                id,
            });
        }
        match &self.target {
            Some(t) => t,
            None => unreachable!("set above"),
        }
    }

    /// Draws `fills` (triangles) and `lines` with the camera rows and light
    /// into a texture `size` px, and returns its egui id.
    pub fn render(
        &mut self,
        rs: &RenderState,
        size: [u32; 2],
        rows: [[f32; 4]; 4],
        light: [f32; 3],
        fills: &[Vertex],
        lines: &[Vertex],
    ) -> egui::TextureId {
        let size = [size[0].max(1), size[1].max(1)];
        let mut uniform = Vec::with_capacity(UNIFORM_SIZE as usize);
        for f in rows.iter().flatten().chain(&light).chain(&[0.0f32]) {
            uniform.extend_from_slice(&f.to_le_bytes());
        }
        rs.queue.write_buffer(&self.uniform, 0, &uniform);
        let data = bytes(&[fills, lines].concat());
        let needed = (data.len() as u64).max(VERTEX_SIZE);
        if self.vertices.as_ref().is_none_or(|(_, cap)| *cap < needed) {
            let buffer = rs.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewer vertices"),
                size: needed.next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.vertices = Some((buffer, needed.next_power_of_two()));
        }
        if let Some((buffer, _)) = &self.vertices
            && !data.is_empty()
        {
            rs.queue.write_buffer(buffer, 0, &data);
        }
        let (fill, line, bind_group) = (
            self.fill.clone(),
            self.line.clone(),
            self.bind_group.clone(),
        );
        let vertices = self.vertices.as_ref().map(|(b, _)| b.clone());
        let target = self.target(rs, size);
        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("viewer"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewer"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.86,
                            g: 0.88,
                            b: 0.9,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(buffer) = &vertices {
                pass.set_bind_group(0, &bind_group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                let (nf, nl) = (fills.len() as u32, lines.len() as u32);
                if nf > 0 {
                    pass.set_pipeline(&fill);
                    pass.draw(0..nf, 0..1);
                }
                if nl > 0 {
                    pass.set_pipeline(&line);
                    pass.draw(nf..nf + nl, 0..1);
                }
            }
        }
        rs.queue.submit([encoder.finish()]);
        target.id
    }
}

#[cfg(test)]
impl Gpu {
    /// The last frame's pixels, RGBA rows top down, and its size.
    pub(crate) fn read(&self, rs: &RenderState) -> Option<(Vec<u8>, [u32; 2])> {
        let t = self.target.as_ref()?;
        let [w, h] = t.size;
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer readback"),
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        rs.queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |_| {});
        rs.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .ok()?;
        let mapped = buffer.get_mapped_range(..).ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            out.extend_from_slice(mapped.get(y * row as usize..y * row as usize + w as usize * 4)?);
        }
        Some((out, [w, h]))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::yield_now();
        }
    }

    /// A renderer on this machine's default adapter with no window, or None
    /// where there is no adapter (the caller then skips).
    pub(crate) fn headless() -> Option<RenderState> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let (device, queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let target_format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let renderer = eframe::egui_wgpu::Renderer::new(&device, target_format, Default::default());
        Some(RenderState {
            available_adapters: vec![adapter.clone()],
            adapter,
            instance,
            device,
            queue,
            target_format,
            renderer: std::sync::Arc::new(egui::mutex::RwLock::new(renderer)),
        })
    }

    #[test]
    fn a_vertex_is_ten_little_endian_floats() {
        let v = Vertex {
            pos: [1.0, 2.0, 3.0],
            normal: [0.0, 0.0, 1.0],
            color: [0.5, 0.25, 0.125, 1.0],
        };
        let b = bytes(&[v, v]);
        assert_eq!(b.len() as u64, 2 * VERTEX_SIZE);
        assert_eq!(b[0..4], 1.0f32.to_le_bytes());
        assert_eq!(b[36..40], 1.0f32.to_le_bytes());
        assert_eq!(b[40..44], 1.0f32.to_le_bytes());
    }
}
