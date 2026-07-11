//! Headless vello rasterization of traced layers for the golden harness.
//!
//! Mirrors the GUI preview's scene encoding (`src/gui/view/vector.rs`): a
//! bottom-first fill per color run under the nonzero rule with holes as closed
//! subpaths, an optional centered stroke per layer, painted in order. The GUI
//! encodes its own `VectorScene` with viewport culling into iced's device; this
//! encodes plain [`SvgLayer`]s with no culling into a private headless device,
//! so the two keep separate encoders rather than share one over inputs that do
//! not match.

use std::sync::{Mutex, OnceLock};

use image::{Rgba, RgbaImage};
use vello::kurbo::{Affine, BezPath, Stroke};
use vello::peniko::{Color, Fill};
use vello::util::RenderContext;
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

use pawtrace::color::Srgb;
use pawtrace::output::SvgLayer;

/// Rasterizes `layers` at the document's native (source-pixel) size `w`x`h`.
/// Paths are in `scale`x supersample space, so they map down to the native
/// grid; a layer stroke stays `width` source px wide.
pub fn rasterize(w: u32, h: u32, scale: u32, layers: &[SvgLayer]) -> RgbaImage {
    render_layers(w, h, Affine::scale(1.0 / scale as f64), scale, layers)
}

/// Like [`rasterize`] but at the `scale`x supersampled density: the paths' own
/// space maps 1:1 to pixels, so the output is `w*scale`x`h*scale`.
pub fn rasterize_scaled(w: u32, h: u32, scale: u32, layers: &[SvgLayer]) -> RgbaImage {
    render_layers(w * scale, h * scale, Affine::IDENTITY, scale, layers)
}

fn render_layers(
    out_w: u32,
    out_h: u32,
    transform: Affine,
    scale: u32,
    layers: &[SvgLayer],
) -> RgbaImage {
    let scene = encode(layers, transform, scale);

    let mut guard = gpu().lock().expect("vello gpu mutex poisoned");
    let Gpu {
        context,
        device_id,
        renderer,
    } = &mut *guard;
    let handle = &context.devices[*device_id];
    let (device, queue) = (&handle.device, &handle.queue);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pawtrace headless vello target"),
        size: wgpu::Extent3d {
            width: out_w,
            height: out_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // vello writes through the storage binding, and COPY_SRC lets the
        // readback copy the result back out.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    renderer
        .render_to_texture(
            device,
            queue,
            &scene,
            &view,
            &RenderParams {
                base_color: Color::from_rgba8(0, 0, 0, 0),
                width: out_w,
                height: out_h,
                antialiasing_method: AaConfig::Area,
            },
        )
        .expect("vello render to texture");

    read_back(device, queue, &texture, out_w, out_h)
}

/// Encodes each layer bottom-first, painting a color run's fill and then its
/// optional stroke before the next run, so a later run covers an earlier one as
/// the SVG export's paint order does.
fn encode(layers: &[SvgLayer], transform: Affine, scale: u32) -> Scene {
    let mut scene = Scene::new();

    for layer in layers {
        let stroke = layer
            .stroke
            .and_then(|st| Some((color(Srgb::from_hex(&st.hex)?), st.width as f64 * scale as f64)));

        for (hex, paths) in layer.colors {
            if paths.is_empty() {
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
                scene.fill(Fill::NonZero, transform, fill, None, &path);
            }

            if let Some((c, width)) = stroke {
                scene.stroke(&Stroke::new(width), transform, c, None, &path);
            }
        }
    }

    scene
}

fn color(c: Srgb) -> Color {
    Color::from_rgba8(c.r(), c.g(), c.b(), 255)
}

/// Copies the rendered texture into a mappable buffer and reads it back as
/// straight-alpha RGBA8. vello writes premultiplied sRGB bytes, so each channel
/// is divided by alpha to match the straight-alpha PNGs the harness composites.
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    w: u32,
    h: u32,
) -> RgbaImage {
    let unpadded = w * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pawtrace vello readback"),
        size: padded as u64 * h as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("pawtrace vello readback encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map vello readback buffer"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll headless device for readback");

    let data = slice.get_mapped_range();
    let mut img = RgbaImage::new(w, h);

    for y in 0..h {
        let row = &data[(y * padded) as usize..][..unpadded as usize];
        for x in 0..w {
            let px = &row[(x * 4) as usize..][..4];
            img.put_pixel(x, y, straight(px[0], px[1], px[2], px[3]));
        }
    }

    drop(data);
    buffer.unmap();
    img
}

/// Un-premultiplies one pixel: divides each channel by alpha so a semi-opaque
/// edge reads at its true color, matching the straight-alpha golden PNGs.
fn straight(r: u8, g: u8, b: u8, a: u8) -> Rgba<u8> {
    if a == 0 {
        return Rgba([0, 0, 0, 0]);
    }
    let un = |c: u8| (((c as u32 * 255 + a as u32 / 2) / a as u32).min(255)) as u8;
    Rgba([un(r), un(g), un(b), a])
}

/// The shared headless GPU state. Creating a device and vello [`Renderer`] is
/// expensive, so one is built on first use and reused across every golden in a
/// test run; the mutex serializes the multi-threaded test harness's renders.
struct Gpu {
    context: RenderContext,
    device_id: usize,
    renderer: Renderer,
}

fn gpu() -> &'static Mutex<Gpu> {
    static GPU: OnceLock<Mutex<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| {
        let mut context = RenderContext::new();
        let device_id = pollster::block_on(context.device(None))
            .expect("no headless wgpu adapter for vello golden rasterization");
        let renderer = Renderer::new(
            &context.devices[device_id].device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::area_only(),
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("create headless vello renderer");

        Mutex::new(Gpu {
            context,
            device_id,
            renderer,
        })
    })
}
