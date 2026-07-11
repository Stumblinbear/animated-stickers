//! Output backends. PRIMARY: layered PDF named ".ai" — one file per PSD,
//! one OCG (PDF layer) per art layer. Rationale: standalone importability.
//! AE imports it as Composition/Retain Layers (VERIFY with the ai-layer-test
//! fixture before building!) and Illustrator opens it with editable paths
//! for manual artifact repair, then re-saves as true .ai. No proprietary
//! Adobe stream is needed: every consumer reads only the PDF half.
//! Implement with the `pdf-writer` crate: one OCG per layer in
//! /OCProperties, content wrapped in BDC/EMC marked-content with /OC refs;
//! cubics map directly to PDF `c` operators, fills to `rg`+`f`.
//! SECONDARY: SVG (external consumers), Tailmovin JSON (future direct
//! shape-layer import via host/tailmovin-import.jsx — optional, not the
//! required path).

use serde::Serialize;
use crate::config::Config;
use crate::trace::TracedPath;

#[derive(Serialize)]
pub struct Doc { pub width: u32, pub height: u32, pub layers: Vec<Layer> }
#[derive(Serialize)]
pub struct Layer {
    pub name: String,
    /// Centered stroke on every path of the layer; absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    pub colors: Vec<ColorGroup>,
}

/// A layer-wide stroke: "#rrggbb" color and width in source px.
#[derive(Serialize, Clone, Debug)]
pub struct Stroke { pub hex: String, pub width: f32 }

/// The layer's stroke as configured, `None` when the width is 0.
pub fn stroke_of(cfg: &Config) -> Option<Stroke> {
    (cfg.stroke_width > 0.0).then(|| Stroke {
        hex: cfg.stroke_color.to_hex(),
        width: cfg.stroke_width,
    })
}
#[derive(Serialize)]
pub struct ColorGroup { pub hex: String, pub paths: Vec<JsonPath> }
#[derive(Serialize)]
pub struct JsonPath {
    /// vertices, in/out tangents RELATIVE to vertex (AE Shape convention),
    /// closed. Cubics -> anchors: each segment end is an anchor; tangents
    /// from the adjoining control points.
    pub v: Vec<[f64; 2]>, pub i: Vec<[f64; 2]>, pub o: Vec<[f64; 2]>,
}

/// One layer's traced content: hex color groups of closed cubic paths, in
/// paint order.
pub type LayerColors = Vec<(String, Vec<TracedPath>)>;

/// Positions a layer-local trace in document space: scales from the layer's
/// `layer_scale` supersample space into the document's `doc_scale`, then
/// translates to the layer's position `offset` (source px).
pub fn place(
    pre: &[(String, Vec<TracedPath>)],
    layer_scale: u32,
    doc_scale: u32,
    offset: (u32, u32),
) -> LayerColors {
    let mut colors = pre.to_vec();
    let ratio = doc_scale as f64 / layer_scale as f64;
    let (dx, dy) = ((offset.0 * doc_scale) as f64, (offset.1 * doc_scale) as f64);

    for (_, paths) in &mut colors {
        for p in paths {
            if ratio != 1.0 {
                p.scale(ratio);
            }

            p.translate(dx, dy);
        }
    }

    colors
}

/// Assembles the Tailmovin JSON document from placed per-layer traces, with
/// path coordinates converted back to source px via `scale`.
pub fn doc(
    width: u32,
    height: u32,
    scale: u32,
    layers: Vec<(String, Option<Stroke>, LayerColors)>,
) -> Doc {
    let s = scale as f64;
    Doc {
        width,
        height,
        layers: layers
            .into_iter()
            .map(|(name, stroke, colors)| Layer {
                name,
                stroke,
                colors: colors
                    .into_iter()
                    .map(|(hex, paths)| ColorGroup {
                        hex,
                        paths: paths.iter().map(|p| to_json_path(p, s)).collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn to_json_path(p: &TracedPath, scale: f64) -> JsonPath {
    let s = 1.0 / scale; // back to source pixel space
    let mut v = vec![[p.start.0 * s, p.start.1 * s]];
    let mut o = Vec::new();
    let mut i = vec![[0.0, 0.0]];
    for (c1, c2, end) in &p.cubics {
        let last = *v.last().unwrap();
        o.push([c1.0 * s - last[0], c1.1 * s - last[1]]);
        v.push([end.0 * s, end.1 * s]);
        i.push([c2.0 * s - end.0 * s, c2.1 * s - end.1 * s]);
    }
    // Closed path: last vertex duplicates first — drop it, fold tangents.
    if v.len() > 1 && (v[0][0] - v[v.len()-1][0]).abs() < 1e-6
                   && (v[0][1] - v[v.len()-1][1]).abs() < 1e-6 {
        v.pop(); let last_i = i.pop().unwrap(); i[0] = last_i;
    } else { o.push([0.0, 0.0]); }
    JsonPath { v, i, o }
}

/// One layer's traced content, borrowed for SVG assembly.
pub struct SvgLayer<'a> {
    pub name: &'a str,
    pub stroke: Option<&'a Stroke>,
    pub colors: &'a [(String, Vec<TracedPath>)],
}

/// `pad` widens the viewBox on every side, in scaled px: pass the stroke
/// overhang (width/2) so a centered stroke on paths touching the bounds
/// isn't clipped. 0 keeps the exact document bounds.
pub fn svg(doc_w: u32, doc_h: u32, scale: u32, pad: f32, layers: &[SvgLayer]) -> String {
    let (vw, vh) = (
        (doc_w * scale) as f32 + 2.0 * pad,
        (doc_h * scale) as f32 + 2.0 * pad,
    );
    // Negating 0.0 gives -0.0, which formats as "-0"; keep the unpadded
    // origin a plain 0.
    let origin = if pad == 0.0 { 0.0 } else { -pad };
    let mut s = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{origin} {origin} {vw} {vh}">"#,
        vw / scale as f32, vh / scale as f32);
    for layer in layers {
        s += &format!(r#"<g id="{}">"#, layer.name);
        // Path coordinates are in scaled space, so the stroke width scales
        // with them to stay `width` source px.
        let stroke_attrs = layer.stroke.map_or(String::new(), |st| {
            format!(
                r#" stroke="{}" stroke-width="{}""#,
                st.hex,
                st.width * scale as f32
            )
        });
        for (hex, paths) in layer.colors {
            // One path element per color: holes are separate contours with
            // opposite winding, and only subpaths of the same element cut
            // under the nonzero fill rule. A hole emitted as its own element
            // would paint solid over the shape beneath it.
            let mut d = String::new();
            for p in paths {
                d += &format!("M {} {} ", p.start.0, p.start.1);
                for (c1, c2, e) in &p.cubics {
                    d += &format!("C {} {} {} {} {} {} ", c1.0, c1.1, c2.0, c2.1, e.0, e.1);
                }
                d += "Z ";
            }
            s += &format!(r#"<path fill="{hex}"{stroke_attrs} d="{}"/>"#, d.trim_end());
        }
        s += "</g>";
    }
    s += "</svg>";
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_pad_widens_the_viewbox_symmetrically() {
        let s = svg(10, 10, 3, 5.0, &[]);
        assert!(s.contains(r#"viewBox="-5 -5 40 40""#), "{s}");
        let s = svg(10, 10, 3, 0.0, &[]);
        assert!(s.contains(r#"viewBox="0 0 30 30""#), "{s}");
    }

    #[test]
    fn svg_applies_the_layer_stroke_to_every_color_path() {
        use crate::trace::TracedPath;
        let colors = vec![(
            "#102030".to_string(),
            vec![TracedPath { start: (0.0, 0.0), cubics: vec![] }],
        )];
        let stroke = Stroke { hex: "#ffffff".into(), width: 11.0 };
        let s = svg(
            10,
            10,
            3,
            0.0,
            &[SvgLayer { name: "Fill", stroke: Some(&stroke), colors: &colors }],
        );
        assert!(s.contains(r##"stroke="#ffffff" stroke-width="33""##), "{s}");
    }
}
