# Pawtrace

Headless Image Trace replacement for sticker art: PSD/PNG → per-layer
region tracing → **Tailmovin JSON** (native AE shape layers via
`host/tailmovin-import.jsx`) / SVG. One-slider iced GUI behind
`--features gui` (unbuilt). Traces a 52-layer 1026² PSD in ~3s.

```
pawtrace art.psd                 # Tailmovin JSON next to the PSD
pawtrace art.psd --format svg    # stacked SVG
PAWTRACE_TIMING=1 pawtrace …     # per-stage timing report
```

## Pipeline

One layer at a time (`src/pipeline.rs` orchestrates; each stage is its own
module):

1. **Crop** to the art's alpha bbox; all coordinates translated back at the
   end (`pipeline.rs`).
2. **Supersample + flatten** (`raster.rs`): premultiplied bilinear ×scale,
   alpha threshold, transparent → magenta KEY.
3. **Palette** (`palette.rs`): bucketed histogram → greedy selection by
   error energy (count × ΔE² to nearest kept color) with OKLab dedup →
   per-pixel remap to nearest palette color.
4. **Regions** (`regions.rs`): segment into connected same-color regions;
   absorb AA transition bands on the region graph (union-find cascade);
   merge sub-speckle fragments into their nearest-color neighbor; each
   region becomes a solid shape (its subtree in a seam-weighted spanning
   tree + non-transparent holes).
5. **Trace** (`trace.rs` + `fit.rs`): pixel boundary walk (visioncortex) →
   corner detection → corner-pinned smoothing → Schneider error-bounded
   cubic fitting.
6. **Output** (`output.rs`): shapes painted as a containment forest,
   parents before children, so the heaviest seams are traced once, by the
   shape on top, and cracks can only expose an ancestor's color; per color,
   one SVG path with hole subpaths (nonzero winding), or Tailmovin JSON
   (`v`/`i`/`o` arrays per contour, source-pixel coordinates).

## Decision table — why each step is what it is (do not "simplify")

| decision | reason |
| --- | --- |
| alpha threshold BEFORE flatten | edge blends fringing into the KEY |
| premultiplied SMOOTH supersample | nearest-neighbor clones each AA blend color scale² times until blends earn palette slots; smooth resampling dilutes them below the floor and gives the tracer 1px boundaries. Bilinear, not cubic: negative lobes make alpha undershoot at the silhouette and unpremultiplying then paints a bright 1px ring |
| scale=3 default | sub-pixel boundary precision through quantization |
| mode filter OFF by default | its job is covered by histogram dilution + OKLab remap; enabled, it voted KEY into edge pixels (bright ring). Kernel is alpha-aware now; knob remains |
| bucketed palette histogram | exact-color counts undercount soft airbrushed features (highlights spread over a continuum where no single value clears the floor) |
| error-energy palette priority | count-first spent slots on gradient mid-steps; count × ΔE² gives a blend family one early representative and always ranks isolated feature colors (linework, eyes) above a blend's third step |
| dominant colors, not k-means | k-means ate white patch + eye colors |
| no KEY guard in palette | every downstream loop gates on the alpha mask and the mode filter is alpha-aware, so KEY cannot reach art pixels; the old guard's only effect was rejecting genuine magenta art within 0.20 OKLab of the key |
| locked palette colors | profile-pinned colors seed the palette unconditionally, so a hand-picked color survives merge/detail changes |
| region-containment shapes, no global stacking | stacked cumulative masks retraced upper regions once per level below them (duplicated geometry); per-region solids trace every boundary once and paint correctly in area order |
| holes with opposite winding, one SVG path per color | separate path elements never cut holes; a ring without its hole paints solid over everything inside (the wrong-color-forearm bug) |
| transition absorption on the region graph | quantized gradients cost shapes; a band that is near a neighbor in color, thin everywhere, two-sided, and colored between its dominant neighbors is an AA artifact. Islands (highlights), extrema, wide spikes (layered fur), and nested families (soft strokes) all fail at least one test and survive |
| shared-stretch seam stitching, no dilation | abutting siblings walk the same pixel seam; each shared stretch is canonicalized once and both shapes splice the identical curve, so a fit wobble cannot open a sub-pixel crack. Dilation was rejected: the speckle floor must apply pre-dilation or dilation resurrects speckles |
| Schneider fitting, not visioncortex splines | splines fit one cubic per ~45° of direction change with no error-driven merging: anchors every few px |
| fit tolerance in scaled px | potrace's opttolerance units (it ran on the supersampled bitmap); source-px tolerance visibly wobbles thin line widths |
| deterministic tie-breaks | HashMap iteration order is randomized per process; boundary/count ties must not change output run to run |

## Calibration (golden harness)

Per-stage contact sheets + anchor metrics for known-difficult layers, every
knob overridable:

```
cargo run --example golden --features preview --release -- \
  fixtures/art.psd out/ "Seff Hair" "Deer R Thigh" merge_dist=0.03 absorb_dist=0
```

Calibrated on Seff Hair / Seff Head / Seff L Ear / Deer L Forearm /
Deer R Thigh (targets from Illustrator Image Trace: ~17 colors, ~193
anchors per art layer):

- `merge_dist` 0.03: 0.10 erased face shading and the eye; 0.04 still
  erased soft fur highlights (ΔE ≈ 0.037 from their base).
- `absorb_dist` 0.08, `opttolerance` 0.4 (scaled px), `alphamax` 1.15.

Other diagnostics: `psddump` (layer PNGs), `psdmeta` (opacity/blend audit),
`psdprobe` (per-point layer coverage), `render` (SVG → PNG via resvg).

## Real-PSD findings

- The `psd` crate misreads the visibility flag (bit 1 is set when HIDDEN);
  import ignores it and drops empty layers instead (`psd_import.rs`).
- The crate returns layers top-first; painting order needs bottom-first,
  so import reverses.
- `layer.rgba()` is a document-sized buffer; art is tight, hence the crop.
- detail normalization MUST use DOCUMENT dimensions, never layer
  dimensions — a 22x7 mouth layer otherwise derives a palette floor larger
  than itself.
- Micro-layers (mouths, rings, eyelids) legitimately collapse to 1 color
  under the default detail floor; a `--detail 2` profile is the remedy.
- Blend modes/opacity: all Normal/255 in the reference PSDs; non-normal
  modes cannot be represented by tracing a layer in isolation — warn,
  don't silently mis-render (not yet implemented).

## Profiles and layer overrides (pawtrace.toml)

A **profile** is settings for a class of layers. Its key is a
case-sensitive glob of the whole layer name: `*` matches any run of
characters, everything else (spaces included) is literal. So `Deer` matches
only "Deer", `Deer *` is a word prefix (matches "Deer R Hand", not
"Deerhoof" — the space is required), `* Fill` is a suffix (matches "Deer L
Hand Fill", not "Refill"), and `Deer * Fill` anchors both ends. **Exactly
one profile applies to a layer**: the most specific match — more literal
(non-`*`) characters win, then a suffix over a prefix, then the longer key.
Profiles never stack.

A **layer override** is a separate tweak keyed on an exact layer name
(`[overrides."Seff Body"]`), applied on top of that one profile. It adjusts
a single layer without touching the class it belongs to, and is always
project-side.

Two tiers share this format. The **global library**
(`%APPDATA%\pawtrace\pawtrace.toml` on Windows, `~/.pawtrace.toml`
elsewhere) holds roles shared across projects; the **project file** lives
next to your PSDs and wins a specificity tie. Full resolution order:
built-in defaults, each tier's `[default]`, the one matching profile, the
layer override, then CLI flags (which override only when explicitly
passed). In the GUI, "Save to global library" routes a profile edit to the
library; "Edit this layer only" writes a layer override (marked `*` in the
layer list).

```toml
[default]
detail = 5.0
max_colors = 24

[profiles."Deer *"]     # a character (prefix; the space is required)
merge_dist = 0.04

[profiles."* Fill"]     # a role: solid mattes need far fewer anchors
opttolerance = 2.0
alphamax = 2.0
stroke_width = 11.0     # the white sticker outline, in source px
stroke_color = "#ffffff"

[overrides."Deer PP"]   # one layer, on top of the Deer profile
detail = 2.0
```

## Perceptual color: OKLab

All color comparisons use OKLab ΔE (`config.rs: color_dist`): palette
dedup, error-energy scoring, the KEY guard, per-pixel remap, and
transition absorption all agree with the eye. Distances live on ~0..1;
see the calibrated defaults above.

## Future: layered ".ai" (PDF + OCG)

A modern .ai is a PDF plus a private Adobe stream, and every consumer
except Illustrator's own editor reads only the PDF half. Emitting a PDF
with one OCG per PSD layer, named `.ai`, would import into AE as a
composition and open in Illustrator with editable paths. GATE: verify both
imports with a 3-named-OCG fixture before implementing; the Tailmovin JSON
import is the working path today.
