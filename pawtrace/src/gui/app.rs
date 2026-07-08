//! Application state: documents, per-document sessions, selection, and the
//! profile tiers. Edit-target routing lives in `edit_target`, message
//! handling in `update`, compute in `compute`, widgets in `view`.
//!
//! Everything the user sees or selects is per-document and lives in
//! [`DocState`], so a tab switch preserves it. Profiles are shared: one
//! global library plus one project tier per folder, both kept across tab
//! switches so unsaved edits are not dropped.

use super::compute::{DocStats, Img, Memo, StageImages, StageKeys, STAGE_COUNT};
use super::doc::Doc;
use super::ids::LayerId;
use super::msg::{Msg, StripView, Tool, TraceView};
use super::undo::Command;
use crate::config::Config;
use crate::profiles::{Profiles, Scope, StackRef};
use iced::widget::pane_grid;
use iced::Task;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// An in-progress profile rename in the library modal.
pub struct LibraryRename {
    pub scope: Scope,
    /// The profile's current key, before the edit is committed.
    pub key: String,
    pub text: String,
}

/// The transient UI state of the profile controls: which chip drop-down is
/// open and whether the library modal is showing.
#[derive(Default)]
pub struct ProfileUi {
    /// The layer row whose profile chip drop-down is open, if any.
    pub chip_open: Option<LayerId>,
    pub library_open: bool,
    /// The library row being renamed, holding its in-progress text.
    pub rename: Option<LibraryRename>,
}

/// The three fixed panes of the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Layers,
    Center,
    Inspector,
}

/// Per-document session state: selection, view, and every compute result,
/// so switching tabs restores exactly what a document showed.
pub struct DocState {
    /// Primary selected layer, always a member of `selection`.
    pub selected_layer: LayerId,
    /// The full multi-selection.
    pub selection: BTreeSet<LayerId>,
    /// Range anchor: the last plainly clicked layer, for shift-click.
    pub select_anchor: LayerId,
    /// When set, edits write this layer's override instead of its profile.
    pub override_layer: bool,
    /// Profile-mode edit target: the pattern key edits write to, seeded to
    /// the selected layer's matching profile but freely editable.
    pub profile_input: String,
    /// Control values: always the selected layer's fully resolved Config.
    pub cfg: Config,
    /// Raw stroke-color text; cfg only updates on a valid "#rrggbb".
    pub stroke_hex: String,
    pub view: StripView,
    pub trace_view: TraceView,
    /// Expanded inspector section, 1-based stage number.
    pub expanded: usize,
    /// Document view zoom in screen px per document px; `None` = fit.
    pub doc_zoom: Option<f32>,
    /// Document view pan, a screen-px offset from the centered position.
    pub doc_pan: iced::Vector,
    /// Stage views' shared zoom in screen px per source-crop px; `None` = fit.
    /// Held in crop space so the art stays put across stages whose rasters
    /// differ in pixel density.
    pub stage_zoom: Option<f32>,
    /// Stage views' shared pan, a screen-px offset from the centered position.
    pub stage_pan: iced::Vector,

    pub stages: StageImages,
    /// Content-keyed cache of this document's pipeline outputs, shared by the
    /// stage strip and the full render.
    pub memo: Memo,
    /// The stage keys of the in-flight stage run, so its streamed parts merge
    /// into the memo under the keys they were computed for.
    pub stage_keys: StageKeys,
    pub stage_pending: [bool; STAGE_COUNT],
    pub stages_running: bool,
    pub stages_dirty: bool,
    /// Generation of this document's in-flight stage stream. A part whose
    /// generation differs was superseded by a newer spawn and is discarded;
    /// documents run independently, so the check is per document.
    pub stage_gen: u64,
    pub full_preview: Option<Img>,
    pub full_stats: Option<DocStats>,
    /// Per-layer anchor counts from the last full render.
    pub layer_anchors: Vec<usize>,
    pub full_busy: bool,
    pub full_dirty: bool,
    /// Full render requested but deferred until the stage strip finishes:
    /// both share the rayon pool, and letting the full render start first
    /// starves the strip's trace, so the layer being edited updates last.
    pub full_queued: bool,
    /// Generation of this document's in-flight full render, checked like
    /// [`DocState::stage_gen`].
    pub full_gen: u64,

    /// Undoable commands newest-last; a new edit clears `redo`.
    pub undo: Vec<Command>,
    /// Commands undone and available to redo, newest-last.
    pub redo: Vec<Command>,
}

impl Default for DocState {
    fn default() -> Self {
        Self {
            selected_layer: LayerId(0),
            selection: BTreeSet::new(),
            select_anchor: LayerId(0),
            override_layer: false,
            profile_input: String::new(),
            cfg: Config::default(),
            stroke_hex: String::new(),
            view: StripView::default(),
            trace_view: TraceView::default(),
            expanded: 5,
            doc_zoom: None,
            doc_pan: iced::Vector::ZERO,
            stage_zoom: None,
            stage_pan: iced::Vector::ZERO,
            stages: StageImages::default(),
            memo: Memo::default(),
            stage_keys: StageKeys::of(&Config::default()),
            stage_pending: [false; STAGE_COUNT],
            stages_running: false,
            stages_dirty: false,
            stage_gen: 0,
            full_preview: None,
            full_stats: None,
            layer_anchors: Vec::new(),
            full_busy: false,
            full_dirty: false,
            full_queued: false,
            full_gen: 0,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}

impl DocState {
    /// Whether the active view is the whole-document composite, which keeps
    /// its own viewport separate from the shared stage viewport.
    pub fn is_doc_view(&self) -> bool {
        matches!(self.view, StripView::Document)
    }

    /// The active view's zoom: the document viewport on Document, else the
    /// shared stage viewport. `None` fits to the canvas.
    pub fn zoom(&self) -> Option<f32> {
        if self.is_doc_view() { self.doc_zoom } else { self.stage_zoom }
    }

    /// The active view's pan, in screen px from the centered position.
    pub fn pan(&self) -> iced::Vector {
        if self.is_doc_view() { self.doc_pan } else { self.stage_pan }
    }

    /// Writes both zoom and pan to the active view's viewport.
    pub fn set_viewport(&mut self, zoom: Option<f32>, pan: iced::Vector) {
        if self.is_doc_view() {
            self.doc_zoom = zoom;
            self.doc_pan = pan;
        } else {
            self.stage_zoom = zoom;
            self.stage_pan = pan;
        }
    }

    /// Writes the active view's zoom, leaving its pan unchanged.
    pub fn set_zoom(&mut self, zoom: Option<f32>) {
        if self.is_doc_view() {
            self.doc_zoom = zoom;
        } else {
            self.stage_zoom = zoom;
        }
    }
}

pub struct App {
    pub docs: Vec<Doc>,
    pub selected_doc: usize,
    /// The per-user library, shared across every project.
    pub global_profiles: Profiles,
    /// Project tier per document folder, keyed on `doc.path.parent()`.
    pub projects: HashMap<PathBuf, Profiles>,
    pub modifiers: iced::keyboard::Modifiers,
    /// When set (with the override toggle off), profile edits land in the
    /// global library instead of the project file.
    pub edit_global: bool,
    pub tool: Tool,
    pub panes: pane_grid::State<PaneKind>,
    pub status: String,
    pub profile_ui: ProfileUi,
    /// Generation counters stay global: they only need to be unique so a
    /// late result for a superseded run is discarded.
    pub stages_gen: u64,
    pub full_gen: u64,
    /// The latest animation-frame instant, updated only while something is
    /// processing. Every processing animation derives its phase from this.
    pub anim_now: Instant,
    /// Read-only fallback for the project tier of a folder not yet loaded.
    empty: Profiles,
}

impl Default for App {
    fn default() -> Self {
        use pane_grid::{Axis, Configuration};
        let panes = pane_grid::State::with_configuration(Configuration::Split {
            axis: Axis::Vertical,
            ratio: 0.16,
            a: Box::new(Configuration::Pane(PaneKind::Layers)),
            b: Box::new(Configuration::Split {
                axis: Axis::Vertical,
                ratio: 0.76,
                a: Box::new(Configuration::Pane(PaneKind::Center)),
                b: Box::new(Configuration::Pane(PaneKind::Inspector)),
            }),
        });
        Self {
            docs: Vec::new(),
            selected_doc: 0,
            global_profiles: Profiles::default(),
            projects: HashMap::new(),
            modifiers: iced::keyboard::Modifiers::default(),
            edit_global: false,
            tool: Tool::default(),
            panes,
            status: String::new(),
            profile_ui: ProfileUi::default(),
            stages_gen: 0,
            full_gen: 0,
            anim_now: Instant::now(),
            empty: Profiles::default(),
        }
    }
}

/// The folder that keys a document's project tier.
fn folder_of(path: &Path) -> PathBuf {
    path.parent().map(Path::to_path_buf).unwrap_or_default()
}

impl App {
    pub fn doc(&self) -> Option<&Doc> {
        self.docs.get(self.selected_doc)
    }

    pub fn doc_mut(&mut self) -> Option<&mut Doc> {
        self.docs.get_mut(self.selected_doc)
    }

    /// The selected document's session, or `None` with no documents open.
    pub fn session(&self) -> Option<&DocState> {
        self.doc().map(|d| &d.session)
    }

    pub fn session_mut(&mut self) -> Option<&mut DocState> {
        self.doc_mut().map(|d| &mut d.session)
    }

    /// The two-tier profile view for document `i`: the global library and
    /// that document's project tier.
    pub fn stack(&self, i: usize) -> StackRef<'_> {
        let project = self
            .docs
            .get(i)
            .and_then(|d| self.projects.get(&folder_of(&d.path)))
            .unwrap_or(&self.empty);
        StackRef { global: &self.global_profiles, project }
    }

    /// The selected document's profile view.
    pub fn stack_sel(&self) -> StackRef<'_> {
        self.stack(self.selected_doc)
    }

    /// The project tier for document `i`, created empty if this is the first
    /// document from its folder.
    pub(super) fn project_tier_mut(&mut self, i: usize) -> Option<&mut Profiles> {
        let dir = folder_of(&self.docs.get(i)?.path);
        Some(self.projects.entry(dir).or_default())
    }

    /// Loads document `i`'s project tier if its folder is not yet loaded.
    pub(super) fn ensure_project(&mut self, i: usize) {
        let Some(dir) = self.docs.get(i).map(|d| folder_of(&d.path)) else {
            return;
        };
        if !self.projects.contains_key(&dir) {
            let near = Profiles::load_near(&self.docs[i].path);
            self.projects.insert(dir, near);
        }
    }

    /// Selects document `i`. State is per document, so switching only moves
    /// focus; caches, selection, and profiles stay put. A document focused
    /// for the first time is initialized here (its selection is still empty).
    pub(super) fn select_doc(&mut self, i: usize) -> Task<Msg> {
        if i >= self.docs.len() {
            return Task::none();
        }
        self.selected_doc = i;
        if self.docs[i].session.selection.is_empty() {
            return self.init_doc(i);
        }
        // Latches left set while the document was in the background (its
        // completions only settle the running/busy flags) relaunch here.
        let s = &mut self.docs[i].session;
        if s.stages_dirty && !s.stages_running {
            s.stages_dirty = false;
            return self.spawn_stages();
        }
        if s.full_dirty && !s.full_busy {
            s.full_dirty = false;
            return self.spawn_full();
        }
        // A queued full render behind a still-running strip stays latched;
        // the strip's completion path consumes it.
        if s.full_queued && !s.stages_running && !s.full_busy {
            s.full_queued = false;
            return self.spawn_full();
        }
        Task::none()
    }

    /// Initializes a freshly focused document: load its project tier, select
    /// its topmost layer, and kick off both compute passes.
    fn init_doc(&mut self, i: usize) -> Task<Msg> {
        self.ensure_project(i);
        self.selected_doc = i;
        // Storage is bottom-first paint order; the top of the visual stack is
        // the last index.
        let top = LayerId(self.docs[i].layers.len().saturating_sub(1));
        Task::batch([self.select_layer(top), self.spawn_full()])
    }

    /// Clears the stage strip before the primary selection moves to a
    /// different layer. A no-op when `i` is already primary. The document
    /// preview is untouched.
    pub(super) fn clear_stages_on_switch(&mut self, i: LayerId) {
        if self.session().is_none_or(|s| s.selected_layer == i) {
            return;
        }
        // Orphan any in-flight run: its parts are the old layer's and would
        // repaint the cleared slots. Completion bookkeeping still runs on the
        // final part, so the dirty-latch relaunch is unaffected.
        self.stages_gen += 1;
        let generation = self.stages_gen;
        let s = self.session_mut().expect("checked above");
        s.stages = StageImages::default();
        s.stage_pending = [true; STAGE_COUNT];
        s.stage_gen = generation;
    }

    /// Selects layer `i` alone: the multi-selection collapses to it.
    pub(super) fn select_layer(&mut self, i: LayerId) -> Task<Msg> {
        self.clear_stages_on_switch(i);
        let doc_idx = self.selected_doc;
        let name = self.layer_name_of(doc_idx, i);
        let seed = name
            .as_deref()
            .and_then(|l| self.stack(doc_idx).match_name(l))
            .unwrap_or_default();
        let Some(sess) = self.session_mut() else {
            return Task::none();
        };
        sess.selected_layer = i;
        sess.select_anchor = i;
        sess.selection = BTreeSet::from([i]);
        sess.profile_input = seed;
        self.load_layer_into_controls();
        self.spawn_stages()
    }

    /// The name of layer `i` in document `d`.
    pub fn layer_name_of(&self, d: usize, i: LayerId) -> Option<String> {
        self.docs.get(d).and_then(|doc| doc.layers.get(i.index())).map(|l| l.name.clone())
    }

    /// The selected layer's name in the selected document.
    pub fn layer_name(&self) -> Option<String> {
        let d = self.selected_doc;
        self.session().and_then(|s| self.layer_name_of(d, s.selected_layer))
    }

    /// Load the selected layer's fully resolved config into the controls.
    /// This is the only thing the controls ever show, so switching write
    /// mode never moves a slider.
    pub(super) fn load_layer_into_controls(&mut self) {
        let doc_idx = self.selected_doc;
        let cfg = match self.layer_name() {
            Some(l) => self.stack(doc_idx).resolve(&l).0,
            None => Config::default(),
        };
        let c = cfg.stroke_color;
        let hex = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        if let Some(sess) = self.session_mut() {
            sess.cfg = cfg;
            sess.stroke_hex = hex;
        }
    }

    pub(super) fn preview_tasks(&mut self) -> Task<Msg> {
        if let Some(sess) = self.session_mut() {
            sess.full_queued = true;
        }
        self.spawn_stages()
    }

    /// Screen-raster pixels per source-crop pixel for the active view's image.
    /// The stage rasters differ in density (Source is 1×, Flatten through
    /// Regions are ×`cfg.scale`, the Trace renders are ×2); dividing a raster's
    /// size by this factor gives the crop-space dimensions the shared stage
    /// viewport is expressed in. Document is 1× against its own document px.
    pub fn view_density(&self) -> f32 {
        let Some(sess) = self.session() else {
            return 1.0;
        };
        match sess.view {
            StripView::Document | StripView::Stage(0) => 1.0,
            StripView::Stage(1..=3) => sess.cfg.scale as f32,
            StripView::Stage(_) => 2.0,
        }
    }

    /// The image the preview should show for the active strip view, if it
    /// has been rendered yet.
    pub fn active_image(&self) -> Option<&Img> {
        let sess = self.session()?;
        match sess.view {
            StripView::Document => sess.full_preview.as_ref(),
            StripView::Stage(0) => sess.stages.source.as_ref(),
            StripView::Stage(1) => sess.stages.flat.as_ref(),
            StripView::Stage(2) => sess.stages.quant.as_ref(),
            StripView::Stage(3) => sess.stages.regions.as_ref(),
            StripView::Stage(_) => match sess.trace_view {
                TraceView::Smooth => sess.stages.smooth.as_ref(),
                TraceView::Fit => sess.stages.render.as_ref(),
                TraceView::Final => sess.stages.simplified.as_ref(),
            },
        }
    }

    /// Whether any document is still computing, across every open tab. Gates
    /// the frame clock so redraws stop the instant everything is at rest.
    pub fn is_animating(&self) -> bool {
        self.docs.iter().any(|d| {
            let s = &d.session;
            s.stages_running || s.full_busy || s.stage_pending.iter().any(|&p| p)
        })
    }

    /// Whether the shown view is currently being recomputed, for the scan
    /// sweep and the strip chips. Chip 5 covers smooth, fit, and simplify.
    pub fn view_busy(&self, view: StripView) -> bool {
        let Some(sess) = self.session() else {
            return false;
        };
        match view {
            StripView::Document => sess.full_busy,
            StripView::Stage(i @ 0..=3) => sess.stage_pending[i],
            StripView::Stage(_) => sess.stage_pending[4..7].iter().any(|&p| p),
        }
    }
}
