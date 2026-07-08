//! Message types, one sub-enum per interaction domain, and the small shared
//! enums naming what the preview shows and which tool is active.

use super::compute::{FullResult, StagePart};
use super::fields::Field;
use super::ids::LayerId;
use crate::profiles::Scope;
use iced::widget::pane_grid;
use iced::{keyboard, Point, Vector};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Clone)]
pub enum Msg {
    File(FileMsg),
    Layer(LayerMsg),
    Edit(EditMsg),
    Profile(ProfileMsg),
    Ui(UiMsg),
    Canvas(CanvasMsg),
    Compute(ComputeMsg),
    Modifiers(keyboard::Modifiers),
    /// A window frame while something is processing; carries that frame's
    /// instant, which the processing animations read as their clock.
    Tick(Instant),
}

/// What the preview shows: the whole-document composite, or the selected
/// layer's output after one pipeline stage (0-based, `0` = Source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StripView {
    #[default]
    Document,
    Stage(usize),
}

/// The active canvas tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Pin,
}

/// Which render the Trace stage's preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TraceView {
    Smooth,
    Fit,
    #[default]
    Final,
}

#[derive(Debug, Clone)]
pub enum FileMsg {
    OpenFiles,
    OpenFolder,
    Opened(Vec<PathBuf>),
    SelectDoc(usize),
    CloseDoc(usize),
    SaveProfiles,
    ExportAll,
}

#[derive(Debug, Clone)]
pub enum LayerMsg {
    /// A click on a layer row; ctrl/shift in the tracked modifiers turn it
    /// into a multi-select edit.
    Click(LayerId),
    ToggleVisible(LayerId),
    ToggleEnabled(LayerId),
    /// Sets preview visibility across the whole selection.
    BulkVisible(bool),
    /// Sets export inclusion across the whole selection.
    BulkEnabled(bool),
    ClearSelection,
}

#[derive(Debug, Clone)]
pub enum EditMsg {
    Set(Field, f64),
    ResetField(Field),
    StrokeHex(String),
    ToggleLock([u8; 3]),
    /// Flips the edit target between the layer's own override (true) and its
    /// governing profile (false).
    OverrideLayer(bool),
    EditGlobal(bool),
    ProfileInput(String),
    ResetLayer,
    Undo,
    Redo,
    /// Ends the in-progress undo gesture (a slider or tool release), so the
    /// next same-kind edit starts a new undo step.
    Seal,
}

/// Profile assignment and library management. `key` values are profile keys;
/// `layer` values are layer indices in the selected document.
#[derive(Debug, Clone)]
pub enum ProfileMsg {
    /// Toggles the chip drop-down for a layer row open or shut.
    ToggleChip(LayerId),
    CloseChip,
    /// Pins one layer to a named profile.
    Assign(LayerId, String),
    /// Pins the whole selection to a named profile.
    AssignSelection(String),
    /// Promotes one layer's deviations into a fresh project profile.
    NewFromLayer(LayerId),
    /// Promotes the primary layer's deviations into a fresh project profile,
    /// pinning every selected layer to it.
    GroupNew,
    OpenLibrary,
    CloseLibrary,
    /// Begins renaming a library profile: opens an inline text field.
    RenameStart(Scope, String),
    RenameInput(String),
    RenameCommit,
    Duplicate(Scope, String),
    Delete(Scope, String),
}

#[derive(Debug, Clone)]
pub enum UiMsg {
    View(StripView),
    Tool(Tool),
    TraceView(TraceView),
    ExpandStage(usize),
    ZoomIn,
    ZoomOut,
    ZoomFit,
    PaneResized(pane_grid::ResizeEvent),
}

/// Interactions published by the preview canvas. Tool points are in the shown
/// view's coordinates: document px on Document, source-crop px on a stage view.
#[derive(Debug, Clone)]
pub enum CanvasMsg {
    /// A pan or zoom gesture, already resolved against the canvas bounds.
    SetViewport {
        zoom: f32,
        pan: Vector,
    },
    ToolPress(Point),
    ToolDrag(Point),
    ToolRelease,
}

/// Background compute results, tagged with the document index they belong to
/// so a late result lands in the right tab even after the selection moved.
#[derive(Debug, Clone)]
pub enum ComputeMsg {
    StagePart(usize, u64, StagePart),
    FullReady(usize, u64, Result<Box<FullResult>, String>),
}
