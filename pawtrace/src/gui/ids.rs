//! Typed identifiers the UI selects and routes messages about, so a layer
//! identifier can never be confused with an unrelated count or index.

/// A layer's stable identity, minted at import and unchanged as layers are
/// added or removed around it. It is not a position: resolve it to the layer's
/// current paint-order index with `Doc::layer_pos`, which yields `None` once
/// the layer is gone. Per-layer state is keyed by this id, and a background
/// compute result carries the `LayerId` it was computed for, so a result for a
/// layer that no longer exists keys into nothing and is discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayerId(u128);

impl LayerId {
    /// Mints a fresh, unique layer identity from the system RNG.
    pub fn new() -> Self {
        LayerId(random_u128())
    }

    /// A `LayerId` with a fixed raw value, for tests that need to name the same
    /// identity twice.
    #[cfg(test)]
    pub(crate) fn from_raw(raw: u128) -> Self {
        LayerId(raw)
    }
}

/// A stable identity for an open document, minted at open and unchanged while
/// tabs open and close around it. It is not a position: resolve it to the
/// document's current tab-strip index with
/// [`App::doc_pos`](crate::gui::app::App::doc_pos), which yields `None` once
/// the document has closed. A background compute result carries the `DocId` of
/// the document it was computed for, so it lands in that document or, if the
/// document has since closed, in none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocId(u128);

impl DocId {
    /// Mints a fresh, unique document identity from the system RNG. Random
    /// rather than a per-process counter so identities stay distinct once
    /// documents are persisted and reopened across sessions.
    pub fn new() -> Self {
        DocId(random_u128())
    }

    /// A `DocId` with a fixed raw value, for tests that need to name the same
    /// identity twice.
    #[cfg(test)]
    pub(crate) fn from_raw(raw: u128) -> Self {
        DocId(raw)
    }
}

/// A fresh 128-bit value from the system RNG, the raw material for a minted
/// identity. Random so identities stay distinct across persisted sessions.
fn random_u128() -> u128 {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("system randomness is unavailable");
    u128::from_le_bytes(bytes)
}

/// The layers rail's scrollable, shared by the rail widget and the canvas
/// hit-test so a click can scroll the selected row into view.
pub fn layers_scrollable() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("pawtrace-layers-rail")
}
