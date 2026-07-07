//! Dominant-color palette (NOT k-means: flat sticker art has fills as large
//! histogram spikes and AA blends as thousands of tiny counts; k-means
//! provably ate white patches and eye colors during Python prototyping).
//! Selection is greedy by error energy over bucketed histogram counts, with
//! OKLab dedup.

use image::{RgbImage, GrayImage};
use std::collections::HashMap;
use crate::config::{color_dist, srgb_to_oklab, Config};

pub fn extract_palette(flat: &RgbImage, alpha: &GrayImage, cfg: &Config, dim: u32)
    -> Vec<[u8; 3]>
{
    // Pool counts into 16-level color buckets (count-weighted mean as the
    // representative). An exact-color histogram undercounts soft airbrushed
    // features: smooth resampling spreads a highlight stroke over a
    // continuum where no single value clears the floor, while its bucket
    // does. Flat fills dominate their bucket, so their mean stays exact.
    // The bucket key indexes a flat array; hashing per pixel cost more than
    // the rest of extraction combined. bits is clamped to 3..=6: below 3,
    // unrelated colors pool into meaningless means; above 6, the array
    // outgrows memory (8 bits would be 536 MB).
    let bits = cfg.hist_bits.clamp(3, 6) as usize;
    let shift = 8 - bits;
    let mut hist = vec![(0u64, [0u64; 3]); 1 << (3 * bits)];
    for (x, y, p) in flat.enumerate_pixels() {
        if alpha.get_pixel(x, y)[0] != 0 {
            let c = p.0;
            let key = ((c[0] as usize >> shift) << (2 * bits))
                | ((c[1] as usize >> shift) << bits)
                | (c[2] as usize >> shift);
            let e = &mut hist[key];
            e.0 += 1;
            for (s, v) in e.1.iter_mut().zip(c) {
                *s += v as u64;
            }
        }
    }
    // A color must cover at least one detail-sized feature.
    let min_count = cfg.detail_area_scaled(dim) as u64;
    let mut entries: Vec<([u8; 3], u64)> = hist
        .into_iter()
        .filter(|&(n, _)| n > 0)
        .map(|(n, sums)| {
            let mean = [
                (sums[0] / n) as u8,
                (sums[1] / n) as u8,
                (sums[2] / n) as u8,
            ];
            (mean, n)
        })
        .filter(|&(_, n)| n >= min_count.max(1))
        .collect();
    if entries.is_empty() && cfg.locked.is_empty() {
        return Vec::new();
    }
    // Tie-break by color: entries arrive in randomized HashMap order, and
    // count ties would otherwise seed selection differently run to run.
    entries.sort_by_key(|&(c, n)| (std::cmp::Reverse(n), c));

    // Locked colors are seeded unconditionally: no count floor, no merge
    // test. Everything else merges toward them, so a locked color survives
    // any settings change.
    let mut palette: Vec<[u8; 3]> = Vec::new();
    for &c in &cfg.locked {
        if !palette.contains(&c) {
            palette.push(c);
        }
    }
    if palette.is_empty() {
        palette.push(entries[0].0);
    }

    // Greedy selection by error energy, count * dE^2 to the nearest already
    // kept color, not by raw count. A gradient's mid-steps score near zero
    // once one family member is in (small dE, squared), while a chromatically
    // isolated feature like linework or an eye scores high at any size, so
    // slots go to distinct features before the third step of a blend.
    //
    // Two merge tests, orthogonal geometry: point distance catches
    // near-duplicates, segment distance catches gradient interiors, which
    // sit far from both endpoints but exactly between them (a dark-to-light
    // gradient spans ~0.6 OKLab; no point radius can merge its middle
    // without also merging distinct fills). Deliberate mid-tones survive
    // through energy order: they are picked early, before both endpoints
    // exist to form their segment; gradient steps score low and arrive
    // after it.
    let lab: Vec<[f32; 3]> = entries.iter().map(|&(c, _)| srgb_to_oklab(c)).collect();
    let mut kept: Vec<[f32; 3]> = palette.iter().map(|&c| srgb_to_oklab(c)).collect();
    let mut point_d = vec![f32::MAX; entries.len()];
    let mut seg_d = vec![f32::MAX; entries.len()];
    // seg_d stays MAX when gradient merging is off, keeping its filter
    // inert and skipping the O(candidates * kept^2) segment updates.
    let gradients = cfg.gradient_dist > 0.0;
    for (i, &l) in lab.iter().enumerate() {
        for (j, &k) in kept.iter().enumerate() {
            point_d[i] = point_d[i].min(lab_dist(l, k));
            if gradients {
                for &k2 in &kept[..j] {
                    seg_d[i] = seg_d[i].min(seg_dist(l, k, k2));
                }
            }
        }
    }
    while palette.len() < cfg.max_colors {
        // Candidates inside a merge radius are merged, not scored: a huge
        // near-duplicate must not outrank or terminate the search while
        // distinct small features remain.
        let best = entries
            .iter()
            .enumerate()
            .filter(|(i, _)| point_d[*i] >= cfg.merge_dist && seg_d[*i] >= cfg.gradient_dist)
            .max_by(|(i, (_, n1)), (j, (_, n2))| {
                let s1 = *n1 as f32 * point_d[*i] * point_d[*i];
                let s2 = *n2 as f32 * point_d[*j] * point_d[*j];
                s1.partial_cmp(&s2).unwrap()
            })
            .map(|(i, _)| i);
        let Some(best) = best else { break };
        palette.push(entries[best].0);
        let new = lab[best];
        for i in 0..entries.len() {
            point_d[i] = point_d[i].min(lab_dist(lab[i], new));
            if gradients {
                for &k in &kept {
                    seg_d[i] = seg_d[i].min(seg_dist(lab[i], k, new));
                }
            }
        }
        kept.push(new);
    }
    palette
}

fn lab_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (d0, d1, d2) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    (d0 * d0 + d1 * d1 + d2 * d2).sqrt()
}

/// Distance from `p` to the segment between `a` and `b` in OKLab.
fn seg_dist(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    let t = if len2 > 1e-12 {
        ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    lab_dist(p, [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]])
}

/// Map every art pixel to nearest palette color (OKLab ΔE); pixels outside
/// the alpha keep their meaningless zero fill.
pub fn remap(flat: &RgbImage, alpha: &GrayImage, palette: &[[u8; 3]]) -> RgbImage {
    let mut out = flat.clone();
    let mut cache: HashMap<[u8; 3], [u8; 3]> = HashMap::new();
    // Flat art runs the same color for long spans; checking the previous
    // pixel first skips the hash for the vast majority of pixels.
    let mut last: Option<([u8; 3], [u8; 3])> = None;
    for (x, y, p) in out.enumerate_pixels_mut() {
        if alpha.get_pixel(x, y)[0] == 0 { continue; }
        let c = p.0;
        if let Some((lc, lm)) = last {
            if c == lc {
                p.0 = lm;
                continue;
            }
        }
        let mapped = *cache.entry(c).or_insert_with(|| {
            *palette.iter()
                .min_by(|a, b| color_dist(c, **a).partial_cmp(&color_dist(c, **b)).unwrap())
                .unwrap_or(&c)
        });
        last = Some((c, mapped));
        p.0 = mapped;
    }
    out
}

/// Mode-filters the quantized labels so color boundaries settle where the
/// local majority sits. Nearest-color remap assigns the resize blend band
/// noisily when two palette colors are perceptually close (dark linework
/// against dark fur), pinching thin lines to nothing in places; majority
/// voting reclaims those pixels. Only art pixels vote: nothing outside the
/// alpha can outvote art, so the silhouette cannot erode.
pub fn label_smooth(quant: &RgbImage, alpha: &GrayImage, k: u32) -> RgbImage {
    let (w, h) = quant.dimensions();
    let r = (k / 2) as i64;
    let mut out = quant.clone();
    let mut counts: Vec<([u8; 3], u32)> = Vec::with_capacity((k * k) as usize);
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if alpha.get_pixel(x as u32, y as u32)[0] == 0 {
                continue;
            }
            counts.clear();
            for dy in -r..=r {
                for dx in -r..=r {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64
                        && alpha.get_pixel(nx as u32, ny as u32)[0] != 0
                    {
                        let c = quant.get_pixel(nx as u32, ny as u32).0;
                        match counts.iter_mut().find(|(cc, _)| *cc == c) {
                            Some((_, n)) => *n += 1,
                            None => counts.push((c, 1)),
                        }
                    }
                }
            }
            if let Some(best) = counts.iter().max_by_key(|(_, n)| *n) {
                out.put_pixel(x as u32, y as u32, image::Rgb(best.0));
            }
        }
    }
    out
}
