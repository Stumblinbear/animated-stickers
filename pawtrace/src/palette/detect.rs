//! Fine feature detection: color-uniform component growing over the opaque
//! pixels of the 1x source crop, guided by its RTV-smoothed copy.

use super::common::{Lab, UnionFind};
use super::{Feature, FeatureId, FeatureLabels, Partition};
use crate::config::Config;
use image::RgbaImage;

/// Growing cap for detection (OKLab ΔE): a pixel joins a component only while
/// the component's mean color stays within this of it. Fine on purpose, so a
/// smooth ramp over-segments into thin bands that feature-level merging then
/// judges band by band. A coarse cap lets a component's mean drift through the
/// anti-alias blend far enough to swallow an adjacent soft highlight (measured
/// ΔE ~0.037 from base fur), which no later stage can then recover.
const DETECT_TOL: f32 = 0.015;

/// Accumulator for one growing component.
struct Comp {
    count: u32,
    // f64: a component can span millions of pixels, and f32 stops absorbing
    // per-pixel increments around 4e6.
    sum_lab: [f64; 3],
    sum_rgb: [u64; 3],
    bbox: (u32, u32, u32, u32),
}

impl Comp {
    fn mean_lab(&self) -> Lab {
        let n = self.count as f64;
        Lab([
            (self.sum_lab[0] / n) as f32,
            (self.sum_lab[1] / n) as f32,
            (self.sum_lab[2] / n) as f32,
        ])
    }

    fn add(&mut self, lab: Lab, rgb: [u8; 3], x: u32, y: u32) {
        self.count += 1;
        for (s, v) in self.sum_lab.iter_mut().zip(lab.0) {
            *s += v as f64;
        }
        for (s, v) in self.sum_rgb.iter_mut().zip(rgb) {
            *s += v as u64;
        }
        self.bbox.0 = self.bbox.0.min(x);
        self.bbox.1 = self.bbox.1.min(y);
        self.bbox.2 = self.bbox.2.max(x);
        self.bbox.3 = self.bbox.3.max(y);
    }
}

/// Grows the color-uniform components of [`Partition::detect`] over `src`'s
/// opaque pixels, taking `smooth` (same dimensions as `src`) as the guide
/// whose OKLab drives growing and each component's mean cap, while every
/// feature's reported color is read from the real `src` pixels.
/// [`Partition::detect`] passes the [`super::rtv_smooth`] output.
pub(super) fn grow_features(src: &RgbaImage, smooth: &RgbaImage, cfg: &Config) -> Partition {
    // Squared distances: the tolerance tests run up to three times per pixel,
    // and squaring the threshold once saves the sqrt in each.
    let tol2 = DETECT_TOL * DETECT_TOL;
    let (w, h) = src.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let raw = src.as_raw();
    let sraw = smooth.as_raw();
    let mut label: Vec<u32> = vec![u32::MAX; wu * hu];
    let mut comps: Vec<Comp> = Vec::new();
    let mut uf = UnionFind::new(0);
    for y in 0..hu {
        for x in 0..wu {
            let i = y * wu + x;
            if raw[i * 4 + 3] < cfg.alpha_threshold {
                continue;
            }
            // rgb is the real source pixel that fixes the feature's reported
            // color; lab comes from the smoothed pixel so growing and the mean
            // it caps against ignore edge speckle.
            let rgb = [raw[i * 4], raw[i * 4 + 1], raw[i * 4 + 2]];
            let lab = Lab::of([sraw[i * 4], sraw[i * 4 + 1], sraw[i * 4 + 2]]);
            let mut joined: Option<u32> = None;
            for ni in [x.checked_sub(1).map(|x| y * wu + x), y.checked_sub(1).map(|y| y * wu + x)]
                .into_iter()
                .flatten()
            {
                let nl = label[ni];
                if nl == u32::MAX {
                    continue;
                }
                let root = uf.find(nl);
                match joined {
                    None => {
                        // The cap tests the component mean against the pixel,
                        // not neighbor against neighbor: pairwise linkage
                        // would chain a smooth gradient dark to light, while
                        // the mean drifts out of tolerance after ~2 tolerances
                        // of gradient and cuts a new band.
                        if comps[root as usize].mean_lab().dist2(lab) <= tol2 {
                            comps[root as usize].add(lab, rgb, x as u32, y as u32);
                            label[i] = root;
                            joined = Some(root);
                        }
                    }
                    Some(j) if root != j => {
                        // Both components accepted this pixel, but they only
                        // fuse when their means also agree: one boundary pixel
                        // must not bridge two adjacent gradient bands.
                        if comps[root as usize].mean_lab().dist2(lab) <= tol2
                            && comps[root as usize]
                                .mean_lab()
                                .dist2(comps[j as usize].mean_lab())
                                <= tol2
                        {
                            // The smaller id stays root, keeping component
                            // order first-encounter.
                            let (lo, hi) = (j.min(root), j.max(root));
                            uf.union(lo, hi);
                            let (count, sum_lab, sum_rgb, bbox) = {
                                let c = &comps[hi as usize];
                                (c.count, c.sum_lab, c.sum_rgb, c.bbox)
                            };
                            let t = &mut comps[lo as usize];
                            t.count += count;
                            for (s, v) in t.sum_lab.iter_mut().zip(sum_lab) {
                                *s += v;
                            }
                            for (s, v) in t.sum_rgb.iter_mut().zip(sum_rgb) {
                                *s += v;
                            }
                            t.bbox.0 = t.bbox.0.min(bbox.0);
                            t.bbox.1 = t.bbox.1.min(bbox.1);
                            t.bbox.2 = t.bbox.2.max(bbox.2);
                            t.bbox.3 = t.bbox.3.max(bbox.3);
                            joined = Some(lo);
                        }
                    }
                    Some(_) => {}
                }
            }
            if joined.is_none() {
                let id = uf.push();
                let (x, y) = (x as u32, y as u32);
                comps.push(Comp {
                    count: 1,
                    sum_lab: [lab.0[0] as f64, lab.0[1] as f64, lab.0[2] as f64],
                    sum_rgb: [rgb[0] as u64, rgb[1] as u64, rgb[2] as u64],
                    bbox: (x, y, x, y),
                });
                label[i] = id;
            }
        }
    }
    let mut root_feat = vec![u32::MAX; comps.len()];
    let mut features = Vec::new();
    for id in 0..comps.len() as u32 {
        if !uf.is_root(id) {
            continue;
        }
        root_feat[id as usize] = features.len() as u32;
        let c = &comps[id as usize];
        let n = c.count as u64;
        features.push(Feature {
            mean: [
                (c.sum_rgb[0] / n) as u8,
                (c.sum_rgb[1] / n) as u8,
                (c.sum_rgb[2] / n) as u8,
            ],
            area: c.count,
            bbox: c.bbox,
        });
    }
    let mut at = vec![FeatureId::NONE; wu * hu];
    for (i, slot) in at.iter_mut().enumerate() {
        if label[i] != u32::MAX {
            let root = uf.find(label[i]);
            *slot = FeatureId(root_feat[root as usize]);
        }
    }
    Partition { features, labels: FeatureLabels { w, h, at } }
}
