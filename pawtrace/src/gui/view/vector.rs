//! Rendering the vector preview in one GPU pass with [`vello`], into iced's own
//! wgpu context via the [`shader`](iced::widget::shader) custom-primitive API.
//!
//! Each frame the whole visible scene is encoded into a single [`vello::Scene`]
//! and rasterized by vello's compute pipeline into an offscreen texture on
//! iced's device, then blitted under the widget's clip. There is no tiling and
//! no CPU readback: a pan re-encodes the scene at the new offset and a zoom does
//! the same, both cheap because a per-path bbox cull keeps encoding to the paths
//! the viewport shows. The texture is sized to the widget in physical
//! pixels, so the raster is always at display resolution.

use crate::color::Srgb;
use crate::gui::compute::{Bbox, VectorScene};
use iced::widget::shader::{self, Primitive};
use iced::Rectangle;
use std::sync::Mutex;
use vello::kurbo::{Affine, BezPath, Stroke};
use vello::peniko::{Color, Fill};
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

/// The vector scene to draw and the viewport that places it, in the widget's
/// local logical coordinates. The physical transform is derived at prepare time
/// from the render viewport's scale factor, so the texture matches the display's
/// pixel grid.
#[derive(Debug)]
pub(super) struct VectorPrimitive {
    scene: VectorScene,
    /// The art's top-left on the widget in local logical px, from the resolved
    /// [`Viewport`](super::viewport::Viewport).
    origin: (f32, f32),
    /// Screen logical px per crop px.
    zoom: f32,
}

impl VectorPrimitive {
    pub(super) fn new(scene: VectorScene, origin: (f32, f32), zoom: f32) -> Self {
        Self {
            scene,
            origin,
            zoom,
        }
    }
}

impl Primitive for VectorPrimitive {
    type Pipeline = VectorPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        let sf = viewport.scale_factor();
        let pw = ((bounds.width * sf).round() as u32).max(1);
        let ph = ((bounds.height * sf).round() as u32).max(1);

        pipeline.resize(device, pw, ph);

        // supersample -> physical px: divide by the trace scale to reach crop px,
        // times zoom to reach logical screen px, times the scale factor to reach
        // physical px, then offset by the art's physical origin on the widget.
        let s = (self.zoom * sf / self.scene.scale as f32) as f64;
        let (tx, ty) = (self.origin.0 * sf, self.origin.1 * sf);
        let transform = Affine::new([s, 0.0, 0.0, s, tx as f64, ty as f64]);

        // The texture rect [0, pw] x [0, ph] mapped back to supersample coords:
        // any path whose box misses this is off-screen and is not encoded.
        let lo = ((-tx as f64) / s, (-ty as f64) / s);
        let hi = ((pw as f64 - tx as f64) / s, (ph as f64 - ty as f64) / s);

        let scene = encode_scene(&self.scene, transform, lo, hi);

        // Split field borrows: the target view (immutable) and the renderer
        // (mutable) are distinct fields, so bind each directly rather than
        // through a method that would borrow the whole pipeline.
        let view = &pipeline
            .target
            .as_ref()
            .expect("target is created in resize before every render")
            .view;
        pipeline
            .renderer
            .get_mut()
            .expect("vello renderer mutex is never poisoned")
            .render_to_texture(
                device,
                queue,
                &scene,
                view,
                &RenderParams {
                    // Transparent so the checkerboard shows through the gaps.
                    base_color: Color::from_rgba8(0, 0, 0, 0),
                    width: pw,
                    height: ph,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .expect("vello render to texture");
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        let Some(bind_group) = &pipeline.bind_group else {
            return false;
        };

        render_pass.set_pipeline(&pipeline.blit);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);

        true
    }
}

fn color(c: Srgb) -> Color {
    Color::from_rgba8(c.r(), c.g(), c.b(), 255)
}

/// Encodes `scene` into a single [`vello::Scene`] under `transform`, bottom layer
/// first and runs in paint order, so a later run paints over an earlier one
/// exactly as the SVG export does. A run whose stroke-inflated box misses the
/// supersample rect `[lo, hi]` is dropped, so only the visible viewport is
/// encoded.
fn encode_scene(scene: &VectorScene, transform: Affine, lo: (f64, f64), hi: (f64, f64)) -> Scene {
    let scale = scene.scale as f64;
    let mut out = Scene::new();

    for layer in &scene.layers {
        let stroke = layer
            .stroke
            .as_ref()
            .and_then(|st| Some((color(Srgb::from_hex(&st.hex)?), st.width as f64 * scale)));

        // A source-px stroke straddles the fill edge, so widen the cull box by
        // half its width (in supersample units) to keep a stroke-only-visible
        // run in the viewport its stroke reaches.
        let pad = stroke.map_or(0.0, |(_, w)| w * 0.5);

        for ((hex, paths), boxes) in layer.colors.iter().zip(layer.bboxes.iter()) {
            if paths.is_empty() {
                continue;
            }

            let run = union_box(boxes);

            let run = Bbox {
                min: (run.min.0 - pad, run.min.1 - pad),
                max: (run.max.0 + pad, run.max.1 + pad),
            };

            if !run.overlaps(lo, hi) {
                continue;
            }

            let mut path = BezPath::new();

            for p in paths {
                path.move_to(p.start);

                for &(c1, c2, end) in &p.cubics {
                    path.curve_to(c1, c2, end);
                }

                path.close_path();
            }

            if path.is_empty() {
                continue;
            }

            if let Some(fill) = Srgb::from_hex(hex).map(color) {
                out.fill(Fill::NonZero, transform, fill, None, &path);
            }

            if let Some((c, width)) = stroke {
                out.stroke(&Stroke::new(width), transform, c, None, &path);
            }
        }
    }

    out
}

/// The union of a run's per-path boxes, the box its whole color run occupies.
fn union_box(boxes: &[Bbox]) -> Bbox {
    let mut lo = (f64::INFINITY, f64::INFINITY);
    let mut hi = (f64::NEG_INFINITY, f64::NEG_INFINITY);

    for b in boxes {
        lo.0 = lo.0.min(b.min.0);
        lo.1 = lo.1.min(b.min.1);
        hi.0 = hi.0.max(b.max.0);
        hi.1 = hi.1.max(b.max.1);
    }

    Bbox { min: lo, max: hi }
}

/// The shared GPU state for the vector primitive: the vello renderer, the
/// offscreen target it draws into, and the pipeline that blits that target under
/// the widget's clip. iced creates one per primitive type and hands it back to
/// every [`VectorPrimitive`].
pub(super) struct VectorPipeline {
    /// vello's [`Renderer`] is `Send` but not `Sync` (it holds a `RefCell` for
    /// its CPU-fallback buffers), so a mutex makes the pipeline `Sync` as iced
    /// requires; `prepare` reaches it through `&mut self` with no contention.
    renderer: Mutex<Renderer>,
    target: Option<Target>,
    blit: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    layout: wgpu::BindGroupLayout,
    /// Samples the current [`Target`]; rebuilt whenever the target is resized.
    bind_group: Option<wgpu::BindGroup>,
}

/// The offscreen texture vello rasterizes into, at the widget's physical size.
struct Target {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl VectorPipeline {
    /// Ensures the offscreen target and its bind group match `(width, height)`,
    /// recreating them on the first call and on any size change.
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self
            .target
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height)
        {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pawtrace vello target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // vello requires `Rgba8Unorm` + `STORAGE_BINDING` to render into it;
            // `TEXTURE_BINDING` lets the blit pass sample it.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pawtrace vello blit bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));

        self.target = Some(Target {
            view,
            width,
            height,
        });
    }
}

impl shader::Pipeline for VectorPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let renderer = Renderer::new(
            device,
            RendererOptions {
                use_cpu: false,
                // Area AA is the recommended default and compiles the fewest
                // pipeline permutations.
                antialiasing_support: AaSupport::area_only(),
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("create vello renderer on iced's device");

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pawtrace vello blit layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pawtrace vello blit shader"),
            source: wgpu::ShaderSource::Wgsl(blit_wgsl(format.is_srgb()).into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pawtrace vello blit pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pawtrace vello blit pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // vello writes premultiplied alpha, so composite it over
                    // whatever the widget stacks under it (the checkerboard).
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            renderer: Mutex::new(renderer),
            target: None,
            blit,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("pawtrace vello blit sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            layout,
            bind_group: None,
        }
    }
}

/// The blit shader: a full-viewport triangle sampling the vello target. iced
/// sets the render pass viewport and scissor to the widget's clip, so the
/// triangle covers exactly the widget.
///
/// vello writes sRGB-encoded, premultiplied bytes into an `Rgba8Unorm` target.
/// When the surface is an sRGB format the GPU re-encodes fragment output on
/// store, so `srgb` linearizes the (un-premultiplied) sample to cancel that and
/// keep colors identical to vello's raster.
fn blit_wgsl(srgb: bool) -> String {
    let convert = if srgb {
        // Un-premultiply, sRGB->linear, re-premultiply, so the surface's
        // linear->sRGB store round-trips vello's bytes unchanged.
        "let a = c.a;
        var rgb = c.rgb;
        if (a > 0.0) { rgb = rgb / a; }
        let cutoff = rgb <= vec3<f32>(0.04045);
        let low = rgb / 12.92;
        let high = pow((rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
        rgb = select(high, low, cutoff) * a;
        return vec4<f32>(rgb, a);"
    } else {
        "return c;"
    };

    format!(
        "@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct VsOut {{
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {{
    let corner = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    // Flip Y: clip-space is Y-up, the target's row 0 is its top.
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {{
    let c = textureSample(tex, samp, in.uv);
    {convert}
}}
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::compute::{layer_bboxes, VectorLayer};
    use crate::trace::TracedPath;
    use std::sync::Arc;

    /// A filled axis-aligned rectangle `[x0,x1] x [y0,y1]` as a one-run red
    /// scene in supersample coords at scale 1, so the cull rect is in the same
    /// space as the geometry.
    fn filled_rect(x0: f64, y0: f64, x1: f64, y1: f64) -> VectorScene {
        let seg = |to: (f64, f64)| (to, to, to);
        let path = TracedPath {
            start: (x0, y0),
            cubics: vec![seg((x1, y0)), seg((x1, y1)), seg((x0, y1)), seg((x0, y0))],
        };
        let colors = Arc::new(vec![("#ff0000".to_string(), vec![path])]);
        let bboxes = Arc::new(layer_bboxes(&colors));

        VectorScene {
            dims: (256, 256),
            scale: 1,
            layers: vec![VectorLayer {
                colors,
                bboxes,
                stroke: None,
            }],
        }
    }

    // A run whose box overlaps the cull rect encodes real path segments; a run
    // the rect misses is dropped, leaving the scene empty. This is the per-path
    // viewport cull the single vello pass relies on to touch only visible paths.
    #[test]
    fn a_run_encodes_only_when_it_reaches_the_cull_rect() {
        let scene = filled_rect(10.0, 10.0, 40.0, 40.0);

        let seen = encode_scene(&scene, Affine::IDENTITY, (0.0, 0.0), (256.0, 256.0));
        assert!(
            !seen.encoding().is_empty(),
            "a run inside the viewport encodes its fill"
        );

        let missed = encode_scene(&scene, Affine::IDENTITY, (1000.0, 1000.0), (2000.0, 2000.0));
        assert!(
            missed.encoding().is_empty(),
            "a run outside the viewport is culled, encoding nothing"
        );
    }
}
