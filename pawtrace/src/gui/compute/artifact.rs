//! Content-hashed pipeline artifacts: the primitive the stage caches key on.
//!
//! An [`Artifact`] wraps an `Arc<T>` alongside an xxh3-128 hash of its content,
//! computed once at production. It compares by that hash, so a downstream key
//! that embeds an upstream artifact compares that output by content without
//! touching `T`: two runs reaching identical content produce equal artifacts and
//! a downstream hit, and any content change moves the key.

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use xxhash_rust::xxh3::Xxh3;

/// An `Arc<T>` tagged with an xxh3-128 content hash of its value, computed once
/// at production. Equality and [`Hash`](std::hash::Hash) read the tag alone: two
/// artifacts are equal exactly when their contents hashed equal, so a key that
/// embeds an artifact compares upstream outputs by content without touching `T`.
/// [`Deref`](std::ops::Deref) hands back the value, so consumers read it as a
/// `T`. A producer builds one with [`Artifact::hashed`] or
/// [`Artifact::hashed_with`], which hash the value's content at production.
#[derive(Debug)]
pub(in crate::gui) struct Artifact<T> {
    value: Arc<T>,
    hash: u128,
}

impl<T: Hash> Artifact<T> {
    /// Wraps `value`, hashing its complete content via its `Hash` impl.
    pub fn new(value: Arc<T>) -> Self {
        let mut h = Xxh3::new();
        value.hash(&mut h);
        Self {
            value,
            hash: h.digest128(),
        }
    }
}

impl<T> Artifact<T> {
    /// Wraps `value` for a `T` that doesn't implement `Hash`: `feed` must write
    /// the value's complete content into the hasher. An omission makes distinct
    /// values collide and serve a stale downstream result.
    pub fn new_with(value: Arc<T>, feed: impl FnOnce(&T, &mut Xxh3)) -> Self {
        let mut h = Xxh3::new();
        feed(&value, &mut h);
        Self {
            value,
            hash: h.digest128(),
        }
    }

}

// The manual impls below deliberately carry no bound on `T`: identity is the
// content hash, so nothing about the payload's own traits is needed.
impl<T> Clone for Artifact<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            hash: self.hash,
        }
    }
}

impl<T> PartialEq for Artifact<T> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl<T> Eq for Artifact<T> {}

impl<T> std::hash::Hash for Artifact<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl<T> std::ops::Deref for Artifact<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

/// Feeds a raster's dimensions and every byte of its pixel buffer into `h`.
pub(super) fn write_raster<P>(h: &mut Xxh3, img: &image::ImageBuffer<P, Vec<u8>>)
where
    P: image::Pixel<Subpixel = u8>,
{
    let (w, ht) = img.dimensions();
    h.write_u32(w);
    h.write_u32(ht);
    h.write(img.as_raw());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regions::Region;

    // An artifact's identity is the content hash the producer computes, not its
    // allocation: two artifacts built from equal content compare equal (the
    // downstream cutoff), and from different content compare unequal.
    #[test]
    fn artifacts_compare_by_content_not_allocation() {
        let region = |c: [u8; 3]| Region {
            color: c,
            x0: 0,
            y0: 0,
            x1: 0,
            y1: 0,
            pixels: vec![(0, 0)],
        };
        let regs = vec![region([1, 2, 3])];
        let (ra, rb) = (Arc::new(regs.clone()), Arc::new(regs.clone()));
        let a = Artifact::new(ra.clone());
        let b = Artifact::new(rb.clone());
        assert!(!Arc::ptr_eq(&ra, &rb), "distinct allocations");
        assert_eq!(a, b, "equal content compares equal");

        let other = vec![region([9, 9, 9])];
        let c = Artifact::new(Arc::new(other.clone()));
        assert_ne!(a, c, "different content compares unequal");
    }
}
