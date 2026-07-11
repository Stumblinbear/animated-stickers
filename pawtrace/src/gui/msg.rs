//! Message types, one sub-enum per interaction domain, and the small shared
//! enums naming what the preview shows and which tool is active.

use crate::color::Srgb;
use super::compute::{FullResult, StagePart};
use super::fields::Field;
use super::ids::{DocId, LayerId};
use super::phases::SubView;
use super::tools::{Tool, ToolMsg};
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
    Recent(RecentMsg),
    Modifiers(keyboard::Modifiers),
    /// A window frame while something is processing; carries that frame's
    /// instant, which the processing animations read as their clock.
    Tick(Instant),
}

/// The welcome screen's recent-items panel: which category is shown, the
/// search filter, and per-item open/pin.
#[derive(Debug, Clone)]
pub enum RecentMsg {
    Tab(RecentTab),
    Search(String),
    /// Opens the recent entry at this index in the current filtered list.
    Open(usize),
    /// Toggles the pinned state of the recent entry at this index.
    Pin(usize),
}

/// The recent panel's two categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecentTab {
    #[default]
    Files,
    Folders,
}

/// One pipeline phase: the unit the strip switches between and the inspector
/// groups its settings under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Colors,
    Paint,
    Shapes,
    Curves,
}

/// What the preview shows: the whole-document composite, or the selected
/// layer's output for one pipeline phase. The shown sub-view within a phase is
/// held in [`DocState`], not here, so two `Phase` values compare equal whenever
/// the same phase is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StripView {
    #[default]
    Document,
    Phase(Phase),
}

#[derive(Debug, Clone)]
pub enum FileMsg {
    OpenFiles,
    OpenFolder,
    /// Opens each path as a document and records it as a recent file.
    Opened(Vec<PathBuf>),
    /// Records a folder as a recent, then scans and opens the art files in it.
    OpenedFolder(PathBuf),
    /// Focuses the document at this tab-strip position.
    SelectDoc(usize),
    /// Closes a document: the one with this identity, or the selected one when
    /// `None` (the keyboard shortcut and menu, which cannot name a tab).
    CloseDoc(Option<DocId>),
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
    ToggleLock(Srgb),
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
/// `layer` values are layer ids in the selected document.
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
    /// Filters the library modal's template list.
    LibrarySearch(String),
    /// Imports templates into the global library from a file.
    ImportLibrary,
    /// Exports the global library to a file.
    ExportLibrary,
}

#[derive(Debug, Clone)]
pub enum UiMsg {
    View(StripView),
    Tool(Tool),
    /// Selects an intermediate render to show within the active phase.
    SubView(SubView),
    /// Clicks an inspector phase section. Toggles the accordion when the section
    /// is editable, or jumps the view to that phase and unlocks it when it is
    /// locked (downstream of the viewed phase).
    ExpandSection(Phase),
    ZoomIn,
    ZoomOut,
    ZoomFit,
    PaneResized(pane_grid::ResizeEvent),
    /// A parameter edit for the active tool's fly-out.
    ToolMsg(ToolMsg),
    /// Clears the current trace failure and re-runs the pipeline.
    Retry,
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
    /// A Select-tool click on the Document view at document px: hit-tests the
    /// layer stack, or deselects when it lands on empty space.
    SelectAt(Point),
    ToolPress(Point),
    ToolDrag(Point),
    ToolRelease,
}

/// Background compute results, tagged with the [`DocId`] they were computed
/// for so a late result lands in the right document even after the selection
/// moved or other tabs closed and shifted, and drops when that document has
/// itself closed.
#[derive(Debug, Clone)]
pub enum ComputeMsg {
    StagePart(DocId, u64, StagePart),
    FullReady(DocId, u64, Box<FullResult>),
}
