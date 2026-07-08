//! Typed identifiers the UI selects and routes messages about, so a layer
//! identifier can never be confused with an unrelated count or index.

/// A layer within its document, identified by its position in the document's
/// stack. Convert to a raw storage index with [`LayerId::index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayerId(pub usize);

impl LayerId {
    /// The layer's index into its document's layer storage.
    pub fn index(self) -> usize {
        self.0
    }
}
