//! The wgpu compositor: draws the same [`FramePlan`] as [`crate::cpu`] with WGSL shaders.
//!
//! What a layer shows is decided once, by [`crate::cpu::resolve`] (pictures fetched, text laid out
//! and drawn into a sprite on the CPU), so both compositors agree on placeholders and errors. Each
//! layer is then one quad over its bounding box; the fragment shader maps a target pixel back into
//! the picture, samples it bilinearly, applies the effects and opacity, and blends. The working
//! picture is `Rgba16Float`, linear and premultiplied, like the CPU's `f32` image.
//!
//! The compositor borrows a device: the editor hands it the one egui draws with, so a preview
//! frame never leaves the GPU ([`GpuCompositor::display`]); export makes its own
//! ([`GpuCompositor::headless`]) and reads frames back already converted to the encoder's Y'CbCr
//! ([`GpuCompositor::read_yuv420`]), so the per-pixel sRGB and 4:2:0 maths run on the GPU too.
//!
//! Results match the CPU reference to within one 8-bit code value: the GPU works in `f16` and its
//! `pow` and rounding differ in the last bit.

use std::collections::HashMap;
use std::sync::Arc;

use reelforge_renderspec::{Blend, Fit, MaskShape, RenderSpec, effects};
use reelforge_time::Tick;

use crate::cpu::{Resolved, resolve, resolve_lut};
use crate::plan::{Affine, FramePlan, PlannedLayer, invert, plan_frame};
use crate::{FrameReport, Image, Lut, MediaSource, Quality, RenderError, unsupported_features};

/// The working picture's format.
const WORK: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Pictures are uploaded as they are held: `f32`, linear, premultiplied.
const PICTURE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// The display picture: straight sRGB, 8 bits.
const DISPLAY: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Effects one layer can carry on the GPU (the uniform's array length).
const MAX_FX: usize = 16;
/// The `Layer` uniform's length in 32-bit words.
const UNIFORM: usize = 40 + MAX_FX * 4;

/// Why the GPU could not be used.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GpuError {
    /// No adapter, or it refused a device.
    #[error("no usable GPU: {0}")]
    Unavailable(String),
    /// Reading a frame back failed.
    #[error("GPU readback failed: {0}")]
    Readback(String),
}

struct Frame {
    width: u32,
    height: u32,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    /// A copy of the target, read by multiply (a pass cannot read what it draws to).
    under: wgpu::Texture,
    under_view: wgpu::TextureView,
    display: wgpu::Texture,
    display_view: wgpu::TextureView,
}

struct Cached {
    /// Held so the pointer key stays unique while the texture lives.
    _image: Arc<Image>,
    view: wgpu::TextureView,
    used: u64,
}

/// Draws frame plans on a wgpu device.
pub struct GpuCompositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: String,
    layer_layout: wgpu::BindGroupLayout,
    /// One per blend mode, in `Blend` order: normal, add, multiply, screen.
    layer_pipelines: [wgpu::RenderPipeline; 4],
    /// Straight sRGB (files, readback) and premultiplied sRGB (what egui draws).
    display_pipelines: [wgpu::RenderPipeline; 2],
    /// Y'CbCr readback; the browser has no blocking readback, so it never reads them.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    luma_pipeline: Option<wgpu::ComputePipeline>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    chroma_pipeline: Option<wgpu::ComputePipeline>,
    no_picture: wgpu::TextureView,
    no_under: wgpu::TextureView,
    no_lut: wgpu::TextureView,
    frame: Option<Frame>,
    /// Uploaded pictures by `Arc` address, kept while frames keep using them (stills, a paused
    /// video).
    pictures: HashMap<usize, Cached>,
    /// Uploaded lookup tables by `Arc` address, kept the same way.
    luts: HashMap<usize, CachedLut>,
    count: u64,
}

struct CachedLut {
    _lut: Arc<Lut>,
    view: wgpu::TextureView,
    used: u64,
}

/// The pipeline a blend mode draws with: fixed-function blending for normal, add and screen; the
/// rest read the picture underneath and blend in the shader.
fn blend_index(b: Blend) -> usize {
    match b {
        Blend::Normal => 0,
        Blend::Add => 1,
        Blend::Screen => 3,
        _ => 2,
    }
}

/// The blend mode as `composite.wgsl` numbers it.
fn blend_code(b: Blend) -> u32 {
    match b {
        Blend::Normal => 0,
        Blend::Add => 1,
        Blend::Multiply => 2,
        Blend::Screen => 3,
        Blend::Overlay => 4,
        Blend::Darken => 5,
        Blend::Lighten => 6,
        Blend::SoftLight => 7,
        Blend::HardLight => 8,
        Blend::Difference => 9,
        Blend::ColorDodge => 10,
        Blend::ColorBurn => 11,
    }
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// A frame on its way back from the GPU as 4:2:0 ([`GpuCompositor::start_yuv420`]).
pub struct YuvReadback {
    buffer: wgpu::Buffer,
    done: std::sync::Arc<std::sync::Mutex<Option<Result<(), String>>>>,
    size: [u32; 2],
    strides: [u32; 2],
    sizes: [u64; 2],
}

impl YuvReadback {
    /// The frame's size in pixels.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Whether the planes have arrived (natively, only after the device was polled).
    pub fn ready(&self) -> bool {
        self.done.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// The planes, once [`ready`](Self::ready): as [`GpuCompositor::read_yuv420`] gives them.
    pub fn finish(self, y: &mut Vec<u8>, u: &mut Vec<u8>, v: &mut Vec<u8>) -> Result<(), GpuError> {
        match self.done.lock().unwrap_or_else(|e| e.into_inner()).take() {
            Some(Ok(())) => {}
            Some(Err(e)) => return Err(GpuError::Readback(e)),
            None => return Err(GpuError::Readback("the frame is not back yet".into())),
        }
        let [w, h] = self.size;
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let [ls, cs] = self.strides;
        let (l, c) = (self.sizes[0] as usize, self.sizes[1] as usize);
        {
            let data = self.buffer.get_mapped_range(..).map_err(|e| GpuError::Readback(e.to_string()))?;
            let rows = |out: &mut Vec<u8>, bytes: &[u8], stride: u32, width: u32, height: u32| {
                out.clear();
                for r in bytes.chunks((stride * 4) as usize).take(height as usize) {
                    out.extend_from_slice(&r[..width as usize]);
                }
            };
            rows(y, &data[..l], ls, w, h);
            rows(u, &data[l..l + c], cs, cw, ch);
            rows(v, &data[l + c..], cs, cw, ch);
        }
        self.buffer.unmap();
        Ok(())
    }
}

impl GpuCompositor {
    /// A compositor on `device`, which it shares (the editor's, say). Compute shaders, used only to
    /// read frames back as Y'CbCr, are set up where the device has them.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, adapter: &wgpu::AdapterInfo) -> GpuCompositor {
        let name = format!("{} ({:?}, {:?})", adapter.name, adapter.device_type, adapter.backend);
        GpuCompositor::named(device, queue, name)
    }

    /// Another compositor on the same device and queue, with its own working picture: an export
    /// beside the editor's preview.
    pub fn sibling(&self) -> GpuCompositor {
        GpuCompositor::named(self.device.clone(), self.queue.clone(), self.adapter.clone())
    }

    fn named(device: wgpu::Device, queue: wgpu::Queue, adapter: String) -> GpuCompositor {
        let composite = device.create_shader_module(wgpu::include_wgsl!("composite.wgsl"));
        let convert = device.create_shader_module(wgpu::include_wgsl!("convert.wgsl"));
        let unfilterable = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let layer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: unfilterable,
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: unfilterable,
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("layer"),
            bind_group_layouts: &[Some(&layer_layout)],
            immediate_size: 0,
        });
        let premul = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let blends = [
            Some(wgpu::BlendState { color: premul, alpha: premul }),
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: premul,
            }),
            // multiply reads the picture underneath and blends in the shader
            None,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::OneMinusDst,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: premul,
            }),
        ];
        let layer_pipelines = blends.map(|blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("layer"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &composite,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &composite,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: WORK,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let display_pipelines = ["fs_srgb", "fs_srgb_premultiplied"].map(|entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &convert,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &convert,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: DISPLAY,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let compute = adapter_has_compute(&device);
        let compute_pipeline = |entry: &str| {
            compute.then(|| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: None,
                    module: &convert,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
        };
        let (luma_pipeline, chroma_pipeline) = (compute_pipeline("luma"), compute_pipeline("chroma"));
        let usage = wgpu::TextureUsages::TEXTURE_BINDING;
        let no_picture = texture(&device, "no picture", 1, 1, PICTURE, usage).create_view(&Default::default());
        let no_under = texture(&device, "no under", 1, 1, WORK, usage).create_view(&Default::default());
        let no_lut = lut_texture(&device, "no lut", 1, usage).create_view(&Default::default());
        GpuCompositor {
            device,
            queue,
            adapter,
            layer_layout,
            layer_pipelines,
            display_pipelines,
            luma_pipeline,
            chroma_pipeline,
            no_picture,
            no_under,
            no_lut,
            frame: None,
            pictures: HashMap::new(),
            luts: HashMap::new(),
            count: 0,
        }
    }

    /// A compositor on a device of its own: the first adapter wgpu finds, a GPU before a software
    /// one. For export and render services.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless() -> Result<GpuCompositor, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|e| GpuError::Unavailable(e.to_string()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("reelforge"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| GpuError::Unavailable(e.to_string()))?;
        Ok(GpuCompositor::new(device, queue, &adapter.get_info()))
    }

    /// Whether frames can be read back as 4:2:0 ([`start_yuv420`](Self::start_yuv420)): the
    /// device has compute shaders.
    pub fn converts_yuv(&self) -> bool {
        self.luma_pipeline.is_some() && self.chroma_pipeline.is_some()
    }

    /// The adapter in use, for logs and reports.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Renders the frame at `t`, `scale` times the output size, into the compositor's working
    /// picture. Read it with [`display`](Self::display), [`read_srgba8`](Self::read_srgba8) or
    /// [`read_yuv420`](Self::read_yuv420). Same contract as [`crate::render_frame`].
    pub fn render(
        &mut self,
        spec: &RenderSpec,
        t: Tick,
        scale: f32,
        media: &dyn MediaSource,
        quality: Quality,
    ) -> Result<FrameReport, RenderError> {
        if quality == Quality::Final {
            let missing = unsupported_features(spec);
            if !missing.is_empty() {
                return Err(RenderError::Unsupported(missing));
            }
        }
        let plan = plan_frame(spec, t, scale);
        self.composite(spec, &plan, media, quality)
    }

    /// Draws `plan` into the working picture.
    pub fn composite(
        &mut self,
        spec: &RenderSpec,
        plan: &FramePlan,
        media: &dyn MediaSource,
        quality: Quality,
    ) -> Result<FrameReport, RenderError> {
        self.count += 1;
        let mut report = FrameReport::default();
        let mut resolved = Vec::with_capacity(plan.layers.len());
        for layer in &plan.layers {
            let what = resolve(spec, plan, layer, media, quality, &mut report)?;
            resolved.push((what, resolve_lut(spec, layer, media, quality, &mut report)?));
        }
        self.ensure_frame(plan.width, plan.height);
        let target = [plan.width as f32, plan.height as f32];

        // every layer's uniforms and textures, before any pass borrows them
        let mut draws = Vec::new();
        for (layer, (what, lut)) in plan.layers.iter().zip(resolved) {
            // the map from the visible part, its size, the whole picture's size, the size a mask is
            // shaped on
            let (m, size, whole, shaped, mode, color, picture) = match what {
                Resolved::Nothing => continue,
                Resolved::Flat(color, None) => {
                    let mut l = layer.clone();
                    l.fit = Fit::Stretch;
                    (l.matrix(1.0, 1.0, target), [1.0, 1.0], [1.0, 1.0], target, 1, color, None)
                }
                Resolved::Flat(color, Some((w, h))) => {
                    let (cw, ch) = layer.visible(w as f32, h as f32);
                    (layer.matrix(cw, ch, target), [cw, ch], [w as f32, h as f32], [cw, ch], 2, color, None)
                }
                Resolved::Picture(pic) => {
                    let (w, h) = (pic.width as f32, pic.height as f32);
                    let (cw, ch) = layer.visible(w, h);
                    let view = self.picture(&pic);
                    (layer.matrix(cw, ch, target), [cw, ch], [w, h], [cw, ch], 0, [0.0; 4], Some(view))
                }
                Resolved::Sprite(sp) => {
                    let (w, h) = (sp.image.width as f32, sp.image.height as f32);
                    let view = self.picture(&sp.image);
                    ([1.0, 0.0, sp.x as f32, 0.0, 1.0, sp.y as f32], [w, h], [w, h], [w, h], 0, [0.0; 4], Some(view))
                }
            };
            let sizes = Sizes { visible: size, whole, shaped };
            let Some(uniform) = layer_uniform(layer, m, sizes, mode, color, target, lut.as_deref()) else { continue };
            let lut_view = lut.as_ref().map(|l| self.lut(l));
            let buffer = self.uniform("layer", bytemuck::cast_slice(&uniform));
            let frame = self.frame.as_ref().expect("sized above");
            let multiply = blend_index(layer.blend) == 2;
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("layer"),
                layout: &self.layer_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(picture.as_ref().unwrap_or(&self.no_picture)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(if multiply {
                            &frame.under_view
                        } else {
                            &self.no_under
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(lut_view.as_ref().unwrap_or(&self.no_lut)),
                    },
                ],
            });
            draws.push((blend_index(layer.blend), bind));
        }

        let frame = self.frame.as_ref().expect("sized above");
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let bg = plan.background.map(f64::from);
        let clear = wgpu::LoadOp::Clear(wgpu::Color { r: bg[0], g: bg[1], b: bg[2], a: bg[3] });
        let begin = |enc: &mut wgpu::CommandEncoder, load| {
            enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("layers"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            })
            .forget_lifetime()
        };
        let mut pass = begin(&mut enc, clear);
        for (blend, bind) in &draws {
            if *blend == 2 {
                // a pass cannot read what it draws to: copy what is underneath, then go on
                drop(pass);
                enc.copy_texture_to_texture(
                    frame.target.as_image_copy(),
                    frame.under.as_image_copy(),
                    wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 },
                );
                pass = begin(&mut enc, wgpu::LoadOp::Load);
            }
            pass.set_pipeline(&self.layer_pipelines[*blend]);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..6, 0..1);
        }
        drop(pass);
        self.queue.submit([enc.finish()]);
        let count = self.count;
        self.pictures.retain(|_, c| c.used == count);
        self.luts.retain(|_, c| c.used == count);
        Ok(report)
    }

    /// The last frame as `Rgba8Unorm` sRGB with premultiplied alpha, on the GPU, sized like the
    /// frame: what egui draws when it is registered as a native texture.
    pub fn display(&self) -> Option<&wgpu::Texture> {
        self.convert(1)
    }

    /// Converts the working picture into the display texture with `display_pipelines[which]`.
    fn convert(&self, which: usize) -> Option<&wgpu::Texture> {
        let pipeline = &self.display_pipelines[which];
        let frame = self.frame.as_ref()?;
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("display"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&frame.target_view),
            }],
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("display") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("display"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.display_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
        Some(&frame.display)
    }

    /// The last frame as straight sRGB 8-bit RGBA, like [`Image::to_srgba8`]: `(width, height,
    /// bytes)`. Blocks until the GPU is done.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_srgba8(&self) -> Result<(u32, u32, Vec<u8>), GpuError> {
        let texture = self.convert(0).ok_or_else(|| GpuError::Readback("nothing rendered yet".into()))?;
        let (w, h) = (texture.width(), texture.height());
        let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("srgba8"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("read") });
        enc.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let data = self.map(&buffer)?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for r in data.chunks(row as usize) {
            out.extend_from_slice(&r[..(w * 4) as usize]);
        }
        Ok((w, h, out))
    }

    /// The last frame as 8-bit limited-range BT.709 4:2:0, flattened onto black: the encoder's
    /// input. `y` gets `width × height` samples, `u` and `v` half that each way (rounded up).
    /// Blocks until the GPU is done.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_yuv420(&self, y: &mut Vec<u8>, u: &mut Vec<u8>, v: &mut Vec<u8>) -> Result<(), GpuError> {
        let read = self.start_yuv420()?;
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| GpuError::Readback(e.to_string()))?;
        read.finish(y, u, v)
    }

    /// Starts reading the last frame back as 4:2:0 ([`read_yuv420`](Self::read_yuv420) without
    /// the wait): the conversion is queued and the copy mapped, and the result is there once
    /// [`YuvReadback::ready`] says so. The browser, which cannot block, waits for that.
    pub fn start_yuv420(&self) -> Result<YuvReadback, GpuError> {
        let frame = self.frame.as_ref().ok_or_else(|| GpuError::Readback("nothing rendered yet".into()))?;
        let (Some(luma), Some(chroma)) = (&self.luma_pipeline, &self.chroma_pipeline) else {
            return Err(GpuError::Readback("this device has no compute shaders".into()));
        };
        let (w, h) = (frame.width, frame.height);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let (ls, cs) = (w.div_ceil(4), cw.div_ceil(4));
        let (lsize, csize) = ((ls * h * 4) as u64, (cs * ch * 4) as u64);
        let dims = self.uniform("dims", bytemuck::cast_slice(&[w, h, ls, cs]));
        let planes = |label, size| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let (yb, ub, vb) = (planes("y", lsize), planes("cb", csize), planes("cr", csize));
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv"),
            size: lsize + 2 * csize,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("yuv") });
        {
            let mut pass =
                enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("yuv"), timestamp_writes: None });
            let bind = |pipeline: &wgpu::ComputePipeline, entries: &[(u32, &wgpu::Buffer)]| {
                let mut all = vec![
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&frame.target_view),
                    },
                    wgpu::BindGroupEntry { binding: 1, resource: dims.as_entire_binding() },
                ];
                all.extend(
                    entries
                        .iter()
                        .map(|(b, buf)| wgpu::BindGroupEntry { binding: *b, resource: buf.as_entire_binding() }),
                );
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("yuv"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &all,
                })
            };
            pass.set_pipeline(luma);
            pass.set_bind_group(0, &bind(luma, &[(2, &yb)]), &[]);
            pass.dispatch_workgroups(ls.div_ceil(16), h.div_ceil(4), 1);
            pass.set_pipeline(chroma);
            pass.set_bind_group(0, &bind(chroma, &[(3, &ub), (4, &vb)]), &[]);
            pass.dispatch_workgroups(cs.div_ceil(16), ch.div_ceil(4), 1);
        }
        enc.copy_buffer_to_buffer(&yb, 0, &read, 0, lsize);
        enc.copy_buffer_to_buffer(&ub, 0, &read, lsize, csize);
        enc.copy_buffer_to_buffer(&vb, 0, &read, lsize + csize, csize);
        self.queue.submit([enc.finish()]);
        let done: std::sync::Arc<std::sync::Mutex<Option<Result<(), String>>>> = Default::default();
        let flag = done.clone();
        read.map_async(wgpu::MapMode::Read, .., move |r| {
            *flag.lock().unwrap_or_else(|e| e.into_inner()) = Some(r.map_err(|e| e.to_string()));
        });
        Ok(YuvReadback { buffer: read, done, size: [w, h], strides: [ls, cs], sizes: [lsize, csize] })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn map(&self, buffer: &wgpu::Buffer) -> Result<Vec<u8>, GpuError> {
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| GpuError::Readback(e.to_string()))?;
        rx.recv().map_err(|e| GpuError::Readback(e.to_string()))?.map_err(|e| GpuError::Readback(e.to_string()))?;
        let data = buffer.get_mapped_range(..).map_err(|e| GpuError::Readback(e.to_string()))?.to_vec();
        buffer.unmap();
        Ok(data)
    }

    /// A uniform buffer holding `bytes`, filled through the queue: Chrome's WebGPU can refuse
    /// buffers mapped at creation (a `RangeError` even for a few hundred bytes), which would
    /// panic inside wgpu.
    fn uniform(&self, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (bytes.len() as u64).next_multiple_of(16),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, bytes);
        buffer
    }

    fn ensure_frame(&mut self, width: u32, height: u32) {
        if self.frame.as_ref().is_some_and(|f| (f.width, f.height) == (width, height)) {
            return;
        }
        use wgpu::TextureUsages as U;
        let d = &self.device;
        let target = texture(d, "frame", width, height, WORK, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC);
        let under = texture(d, "under", width, height, WORK, U::TEXTURE_BINDING | U::COPY_DST);
        let display =
            texture(d, "display", width, height, DISPLAY, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC);
        self.frame = Some(Frame {
            width,
            height,
            target_view: target.create_view(&Default::default()),
            under_view: under.create_view(&Default::default()),
            display_view: display.create_view(&Default::default()),
            target,
            under,
            display,
        });
    }

    /// The texture of a picture, uploaded once while frames keep using it.
    fn picture(&mut self, pic: &Arc<Image>) -> wgpu::TextureView {
        let key = Arc::as_ptr(pic) as usize;
        if let Some(c) = self.pictures.get_mut(&key) {
            c.used = self.count;
            return c.view.clone();
        }
        let view = self.upload(pic, "picture");
        self.pictures.insert(key, Cached { _image: pic.clone(), view: view.clone(), used: self.count });
        view
    }

    /// The 3-D texture of a lookup table, uploaded once while frames keep using it.
    fn lut(&mut self, lut: &Arc<Lut>) -> wgpu::TextureView {
        let key = Arc::as_ptr(lut) as usize;
        if let Some(c) = self.luts.get_mut(&key) {
            c.used = self.count;
            return c.view.clone();
        }
        let n = lut.size as u32;
        let tex =
            lut_texture(&self.device, "lut", n, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        let texels: Vec<[f32; 4]> = lut.data.iter().map(|c| [c[0], c[1], c[2], 1.0]).collect();
        self.queue.write_texture(
            tex.as_image_copy(),
            bytemuck::cast_slice(texels.as_flattened()),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n * 16), rows_per_image: Some(n) },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
        );
        let view = tex.create_view(&Default::default());
        self.luts.insert(key, CachedLut { _lut: lut.clone(), view: view.clone(), used: self.count });
        view
    }

    fn upload(&self, img: &Image, label: &str) -> wgpu::TextureView {
        let (w, h) = (img.width.max(1), img.height.max(1));
        let tex = texture(
            &self.device,
            label,
            w,
            h,
            PICTURE,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        if !img.pixels.is_empty() {
            self.queue.write_texture(
                tex.as_image_copy(),
                bytemuck::cast_slice(img.pixels.as_flattened()),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 16), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        tex.create_view(&Default::default())
    }
}

fn lut_texture(device: &wgpu::Device, label: &str, n: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: PICTURE,
        usage,
        view_formats: &[],
    })
}

fn adapter_has_compute(device: &wgpu::Device) -> bool {
    device.limits().max_compute_workgroups_per_dimension > 0
}

/// A layer's picture sizes, in its own pixels.
struct Sizes {
    /// What shows, after the crop: the box `m` maps.
    visible: [f32; 2],
    /// The whole picture.
    whole: [f32; 2],
    /// The box a mask is shaped on.
    shaped: [f32; 2],
}

/// The `Layer` uniform of `composite.wgsl`, or `None` when the layer covers no pixel. `m` maps
/// the visible part of the picture to target pixels.
fn layer_uniform(
    layer: &PlannedLayer,
    m: Affine,
    sizes: Sizes,
    mode: u32,
    color: [f32; 4],
    target: [f32; 2],
    lut: Option<&Lut>,
) -> Option<[u32; UNIFORM]> {
    let size = sizes.visible;
    let inv = invert(m)?;
    let corners = [(0.0, 0.0), (size[0], 0.0), (0.0, size[1]), (size[0], size[1])]
        .map(|(x, y)| (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]));
    let lo = |f: fn(f32, f32) -> f32, pick: fn(&(f32, f32)) -> f32, init| corners.iter().map(pick).fold(init, f);
    // a pixel's margin all round: the shader discards what lies outside the picture
    let x0 = (lo(f32::min, |c| c.0, f32::INFINITY).floor() - 1.0).max(0.0);
    let y0 = (lo(f32::min, |c| c.1, f32::INFINITY).floor() - 1.0).max(0.0);
    let x1 = (lo(f32::max, |c| c.0, f32::NEG_INFINITY).ceil() + 1.0).min(target[0]);
    let y1 = (lo(f32::max, |c| c.1, f32::NEG_INFINITY).ceil() + 1.0).min(target[1]);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut fx = [[0.0f32; 4]; MAX_FX];
    let mut n = 0;
    for (id, params) in &layer.fx {
        let get = |name: &str, default: f32| params.iter().find(|(k, _)| k == name).map_or(default, |(_, v)| *v);
        let entries = match id.as_str() {
            effects::BRIGHTNESS => vec![[1.0, get("amount", 0.0), 0.0, 0.0]],
            effects::SATURATION => vec![[2.0, get("amount", 1.0), 0.0, 0.0]],
            effects::GRADE => {
                let g = effects::GRADE_PARAMS.map(|n| get(n, 0.0));
                vec![[3.0, g[0], g[1], g[2]], [g[3], g[4], g[5], g[6]], [g[7], g[8], 0.0, 0.0]]
            }
            effects::LUT => match lut {
                Some(l) => vec![
                    [6.0, get("amount", 1.0), l.size as f32, 0.0],
                    [l.min[0], l.min[1], l.min[2], 0.0],
                    [l.max[0], l.max[1], l.max[2], 0.0],
                ],
                None => continue,
            },
            _ => continue,
        };
        if n + entries.len() <= MAX_FX {
            for e in entries {
                fx[n] = e;
                n += 1;
            }
        }
    }
    let f = |v: f32| v.to_bits();
    let mut u = [0u32; UNIFORM];
    u[..16].copy_from_slice(
        &[
            inv[0], inv[1], inv[2], inv[3], inv[4], inv[5], size[0], size[1], x0, y0, x1, y1, color[0], color[1],
            color[2], color[3],
        ]
        .map(f),
    );
    u[16..20].copy_from_slice(&[f(target[0]), f(target[1]), f(layer.opacity), mode]);
    u[20..24].copy_from_slice(&[blend_code(layer.blend), n as u32, 0, 0]);
    let [w, h] = sizes.whole;
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    let (kind, mask) = match layer.shape {
        None => (0.0, [0.0; 7]),
        Some(sm) => (
            match sm.shape {
                MaskShape::Linear => 1.0,
                MaskShape::Mirror => 2.0,
                MaskShape::Circle => 3.0,
                MaskShape::Rectangle => 4.0,
            },
            [sm.center[0], sm.center[1], sm.feather, sm.size[0], sm.size[1], sm.rotation, flag(sm.invert)],
        ),
    };
    u[24..40].copy_from_slice(
        &[
            w,
            h,
            sizes.shaped[0],
            sizes.shaped[1],
            layer.crop[0] * w,
            layer.crop[1] * h,
            flag(layer.flip[0]),
            flag(layer.flip[1]),
            kind,
            mask[0],
            mask[1],
            mask[2],
            mask[3],
            mask[4],
            mask[5],
            mask[6],
        ]
        .map(f),
    );
    for (slot, v) in u[40..].iter_mut().zip(fx.as_flattened()) {
        *slot = f(*v);
    }
    Some(u)
}

#[cfg(test)]
mod tests;
