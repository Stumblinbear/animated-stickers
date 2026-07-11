//! The one-entry memo cell: the per-stage cache primitive.

/// A one-entry cache holding the last `(key, value)`. The value is served on a
/// key hit and recomputed on a miss; only the most recent input is retained.
#[derive(Clone, Debug)]
pub(in crate::gui) struct Memo<K, V>(Option<(K, V)>);

impl<K, V> Default for Memo<K, V> {
    fn default() -> Self {
        Memo(None)
    }
}

impl<K: PartialEq, V: Clone> Memo<K, V> {
    /// The cached value when `key` matches the stored key, without recomputing.
    pub fn get(&self, key: &K) -> Option<V> {
        match &self.0 {
            Some((k, v)) if k == key => Some(v.clone()),
            _ => None,
        }
    }

    /// The stored value regardless of its key, for a consumer that wants
    /// whatever the memo currently holds (the pin hit test reads the regions
    /// the strip last produced).
    pub fn current(&self) -> Option<V> {
        self.0.as_ref().map(|(_, v)| v.clone())
    }

    /// Stores `(key, value)` as the sole entry, replacing whatever was held. A
    /// later [`get`](Self::get) of an equal key serves this `value`, so a value
    /// computed elsewhere can be published without rerunning the compute.
    pub fn install(&mut self, key: K, value: V) {
        self.0 = Some((key, value));
    }

    /// The cached value for `key`, or `f(&key, ctx)` computed, stored, and
    /// returned on a miss. `ctx` carries the layer-fixed inputs a compute needs
    /// that are not part of the key.
    pub fn get_or<C>(&mut self, key: K, ctx: C, f: fn(&K, C) -> V) -> V {
        if let Some(v) = self.get(&key) {
            return v;
        }

        // `f` is a fn pointer, not a closure: it can't capture, so a compute
        // reads only its key and `ctx`, never a config field the key omits.
        let value = f(&key, ctx);

        self.0 = Some((key, value.clone()));

        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // A key hit serves the cached value without calling the compute again, and
    // returns the very same Arc.
    #[test]
    fn a_memo_hit_skips_compute_and_returns_the_same_arc() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);

        fn build(_k: &u32, _c: ()) -> Arc<RgbImage> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            Arc::new(RgbImage::new(1, 1))
        }

        let mut memo: Memo<u32, Arc<RgbImage>> = Memo::default();

        let first = memo.get_or(7, (), build);
        let second = memo.get_or(7, (), build);

        assert_eq!(CALLS.load(Ordering::SeqCst), 1, "compute ran once");
        assert!(
            Arc::ptr_eq(&first, &second),
            "the hit returns the cached Arc"
        );
    }
}
