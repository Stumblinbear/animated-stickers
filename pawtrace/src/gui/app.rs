//! Application state: documents, per-document sessions, selection, and the
//! profile tiers. Edit-target routing lives in `edit_target`, message
//! handling in `update`, compute in `compute`, widgets in `view`.
//!
//! Everything the user sees or selects is per-document and lives in
//! [`DocState`], so a tab switch preserves it. Profiles are shared: one
//! global library plus one project tier per folder, both kept across tab
//! switches so unsaved edits are not dropped.

use super::compute::{DocStats, Img, Memo, StageImages, StageKeys};
use super::doc::{Doc, LayerOutputs};
use super::fields::Field;
use super::ids::{DocId, LayerId};
use super::msg::{Msg, Phase, StripView};
use super::phases::{PerPhase, PerStage, Stage, SubView};
use super::recents::RecentEntry;
use super::tools::{Tool, Tools};
use super::undo::Command;
use crate::config::Config;
use crate::profiles::{Profiles, Scope, StackRef};
use iced::widget::pane_grid;
use iced::Task;
use rustc_hash::FxHashMap;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// A layer whose trace failed, driving the coordinated red failure treatment
/// shown across the UI.
#[derive(Clone)]
pub struct LayerError {
    pub layer: LayerId,
    /// The phase whose chip and inspector header turn red.
    pub phase: Phase,
    /// The human-readable cause shown large in the placeholder.
    pub human: String,
    /// The raw pipeline message, shown in a monospace box.
    pub raw: String,
    /// An optional one-click setting change offered beside Retry.
    pub fix: Option<ErrorFix>,
}

/// A suggested one-click fix for a failed trace: a labeled button that sets one
/// field, which re-runs the trace and clears the error if it resolves.
#[derive(Clone)]
pub struct ErrorFix {
    pub label: String,
    pub field: Field,
    pub value: f64,
}

/// The welcome screen's recent-items panel state: the entries, the search
/// filter, and the shown category.
#[derive(Default)]
pub struct WelcomeUi {
    /// Recent files and folders, newest-first.
    pub recents: Vec<RecentEntry>,
    pub search: String,
    pub tab: super::msg::RecentTab,
}

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
    /// The library modal's template-search filter.
    pub library_search: String,
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
    /// The sub-view each phase last showed, so switching phases restores what
    /// that phase was last displaying.
    pub phase_sub: PerPhase<SubView>,
    /// The expanded inspector phase section, or `None` when the accordion is
    /// fully collapsed.
    pub expanded: Option<Phase>,
    /// The selected layer's trace failure, if any.
    pub trace_error: Option<LayerError>,
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
    pub stage_pending: PerStage<bool>,
    pub stages_running: bool,
    pub stages_dirty: bool,
    /// Generation of this document's in-flight stage stream. A part whose
    /// generation differs was superseded by a newer spawn and is discarded;
    /// documents run independently, so the check is per document.
    pub stage_gen: u64,
    pub full_preview: Option<Img>,
    pub full_stats: Option<DocStats>,
    /// Per-layer derived render outputs from the last full render, keyed by
    /// layer id. The second per-layer map (the first, artist inputs, lives on
    /// the document); both grow by lifetime, never by feature.
    pub layer_outputs: FxHashMap<LayerId, LayerOutputs>,
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

    /// Whether this document has been focused once. An empty selection is a
    /// legal state after a deselect, so the first-focus auto-select keys off
    /// this rather than on the selection being empty.
    pub initialized: bool,
}

impl Default for DocState {
    fn default() -> Self {
        Self {
            // Placeholder identities that resolve to no layer until the first
            // selection lands; an empty selection is a legal resting state.
            selected_layer: LayerId::new(),
            selection: BTreeSet::new(),
            select_anchor: LayerId::new(),
            override_layer: true,
            profile_input: String::new(),
            cfg: Config::default(),
            stroke_hex: String::new(),
            view: StripView::default(),
            phase_sub: PerPhase::from_fn(Phase::default_subview),
            expanded: Some(Phase::Colors),
            trace_error: None,
            doc_zoom: None,
            doc_pan: iced::Vector::ZERO,
            stage_zoom: None,
            stage_pan: iced::Vector::ZERO,
            stages: StageImages::default(),
            memo: Memo::default(),
            stage_keys: StageKeys::of(&Config::default(), &[]),
            stage_pending: PerStage::from_fn(|_| false),
            stages_running: false,
            stages_dirty: false,
            stage_gen: 0,
            full_preview: None,
            full_stats: None,
            layer_outputs: FxHashMap::default(),
            full_busy: false,
            full_dirty: false,
            full_queued: false,
            full_gen: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            initialized: false,
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
    pub selected_doc: DocId,
    /// The per-user library, shared across every project.
    pub global_profiles: Profiles,
    /// Project tier per document folder, keyed on `doc.path.parent()`.
    pub projects: HashMap<PathBuf, Profiles>,
    pub modifiers: iced::keyboard::Modifiers,
    /// When set (with the override toggle off), profile edits land in the
    /// global library instead of the project file.
    pub edit_global: bool,
    pub tools: Tools,
    /// The welcome screen's recents panel state.
    pub welcome: WelcomeUi,
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
            // Names no open document until one is selected; every read resolves
            // through `doc_pos`, which yields `None` for an unmatched identity.
            selected_doc: DocId::new(),
            global_profiles: Profiles::default(),
            projects: HashMap::new(),
            modifiers: iced::keyboard::Modifiers::default(),
            edit_global: false,
            tools: Tools::default(),
            welcome: WelcomeUi::default(),
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
    /// Builds an app with `path` opened as its only document, focused with its
    /// top layer selected, for headless snapshotting. Runs only the synchronous
    /// focus steps; the compute pipeline never starts, so the stage and preview
    /// images stay empty and the panels show their loading state.
    pub(super) fn with_document(path: &Path) -> anyhow::Result<Self> {
        let doc = super::doc::load_doc(path)?;
        let mut app = Self::default();
        app.docs.push(doc);
        app.ensure_project(0);
        app.selected_doc = app.docs[0].id;
        app.docs[0].session.initialized = true;
        // Only the state mutations matter here; the returned compute Task is
        // dropped unpolled, which the iced runtime would otherwise drive.
        if let Some(top) = app.docs[0].top_layer() {
            let _ = app.select_layer(top);
        }
        Ok(app)
    }

    /// The recent entries shown for the current tab and search filter,
    /// newest-first. Indices into this list are what the welcome rows carry.
    pub fn filtered_recents(&self) -> Vec<&super::recents::RecentEntry> {
        let want_folder = self.welcome.tab == super::msg::RecentTab::Folders;
        let q = self.welcome.search.trim().to_lowercase();
        self.welcome
            .recents
            .iter()
            .filter(|e| e.folder == want_folder)
            .filter(|e| q.is_empty() || e.path.to_string_lossy().to_lowercase().contains(&q))
            .collect()
    }

    /// The path of the recent entry at `i` in the current filtered list, if any.
    pub(super) fn recent_path(&self, i: usize) -> Option<PathBuf> {
        self.filtered_recents().get(i).map(|e| e.path.clone())
    }

    /// Toggles the pinned flag of the recent at `i` in the filtered list and
    /// persists the change.
    pub(super) fn toggle_recent_pin(&mut self, i: usize) {
        let Some(path) = self.recent_path(i) else {
            return;
        };
        if let Some(e) = self.welcome.recents.iter_mut().find(|e| e.path == path) {
            e.pinned = !e.pinned;
        }
        self.welcome
            .recents
            .sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.opened.cmp(&a.opened)));
        super::recents::save(&self.welcome.recents);
    }

    /// Records each of `paths` as just opened and persists the recents once.
    pub(super) fn remember_recents(&mut self, paths: &[PathBuf], folder: bool) {
        if paths.is_empty() {
            return;
        }
        for p in paths {
            super::recents::touch(&mut self.welcome.recents, p, folder);
        }
        super::recents::save(&self.welcome.recents);
    }

    /// The tab-strip position of the document identified by `id`, or `None`
    /// when no open document has that identity (it was never opened, or has
    /// since closed).
    pub fn doc_pos(&self, id: DocId) -> Option<usize> {
        self.docs.iter().position(|d| d.id == id)
    }

    /// The tab-strip position of the selected document. Returns a past-the-end
    /// index when no open document is selected, so the position readers
    /// (`docs.get`, [`stack`](Self::stack)) resolve to nothing rather than to
    /// the wrong document.
    pub fn selected_pos(&self) -> usize {
        self.doc_pos(self.selected_doc).unwrap_or(self.docs.len())
    }

    pub fn doc(&self) -> Option<&Doc> {
        self.docs.get(self.selected_pos())
    }

    pub fn doc_mut(&mut self) -> Option<&mut Doc> {
        let pos = self.selected_pos();
        self.docs.get_mut(pos)
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
        self.stack(self.selected_pos())
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
    /// for the first time is initialized here.
    pub(super) fn select_doc(&mut self, i: usize) -> Task<Msg> {
        if i >= self.docs.len() {
            return Task::none();
        }
        self.selected_doc = self.docs[i].id;
        if !self.docs[i].session.initialized {
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
        self.selected_doc = self.docs[i].id;
        self.docs[i].session.initialized = true;
        let Some(top) = self.docs[i].top_layer() else {
            return self.spawn_full();
        };
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
        s.stage_pending = PerStage::from_fn(|_| true);
        s.stage_gen = generation;
    }

    /// Selects layer `i` alone: the multi-selection collapses to it.
    pub(super) fn select_layer(&mut self, i: LayerId) -> Task<Msg> {
        self.clear_stages_on_switch(i);
        let doc_idx = self.selected_pos();
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
        // The failure treatment is per primary layer, so a new selection starts clean.
        sess.trace_error = None;
        self.load_layer_into_controls();
        self.spawn_stages()
    }

    /// The name of layer `i` in document `d`.
    pub fn layer_name_of(&self, d: usize, i: LayerId) -> Option<String> {
        self.docs.get(d).and_then(|doc| doc.layer(i)).map(|l| l.name.clone())
    }

    /// The selected layer's name in the selected document.
    pub fn layer_name(&self) -> Option<String> {
        let d = self.selected_pos();
        self.session().and_then(|s| self.layer_name_of(d, s.selected_layer))
    }

    /// Load the selected layer's fully resolved config into the controls.
    /// This is the only thing the controls ever show, so switching write
    /// mode never moves a slider.
    pub(super) fn load_layer_into_controls(&mut self) {
        let doc_idx = self.selected_pos();
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

    /// The phase the strip is showing, or `None` on the Document view.
    pub fn active_phase(&self) -> Option<Phase> {
        match self.session()?.view {
            StripView::Document => None,
            StripView::Phase(p) => Some(p),
        }
    }

    /// The remembered sub-view of the active phase, or `None` on Document.
    pub fn active_subview(&self) -> Option<SubView> {
        let sess = self.session()?;
        self.active_phase().map(|p| sess.phase_sub[p])
    }

    /// The pipeline stage the active phase's current sub-view resolves to, or
    /// `None` on Document or a sub-view whose render is not produced yet.
    pub fn active_stage(&self) -> Option<Stage> {
        self.active_subview()?.stage()
    }

    /// Screen-raster pixels per source-crop pixel for the active view's image.
    /// The stage rasters differ in density; dividing a raster's size by this
    /// factor gives the crop-space dimensions the shared stage viewport is
    /// expressed in. Document is 1x against its own document px.
    pub fn view_density(&self) -> f32 {
        let scale = self.session().map(|s| s.cfg.scale).unwrap_or(1);
        self.active_stage().map(|s| s.density(scale)).unwrap_or(1.0)
    }

    /// The image the preview should show for the active view, if it has been
    /// rendered yet.
    pub fn active_image(&self) -> Option<&Img> {
        let sess = self.session()?;
        match sess.view {
            StripView::Document => sess.full_preview.as_ref(),
            StripView::Phase(_) => self.active_stage()?.image(&sess.stages),
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
    /// sweep and the strip chips. A phase is busy while any of its sub-views'
    /// stages compute.
    pub fn view_busy(&self, view: StripView) -> bool {
        let Some(sess) = self.session() else {
            return false;
        };
        match view {
            StripView::Document => sess.full_busy,
            StripView::Phase(p) => self.phase_busy(p),
        }
    }

    /// Whether any of phase `p`'s sub-view stages are recomputing.
    pub fn phase_busy(&self, p: Phase) -> bool {
        let Some(sess) = self.session() else {
            return false;
        };
        p.subviews()
            .iter()
            .filter_map(|sv| sv.stage())
            .any(|stage| sess.stage_pending[stage])
    }

    /// Whether `tool` is offered on the current view. `false` with no document
    /// open, so no tool stays active over an empty preview.
    pub fn tool_applicable(&self, tool: Tool) -> bool {
        match self.session() {
            Some(s) => tool.applies(s.view, self.active_subview()),
            None => false,
        }
    }

    /// Falls the active tool back to Select when the current view no longer
    /// offers it, so a hidden tool is never left active (spec: tool set is a
    /// function of the view).
    pub(super) fn reconcile_tool(&mut self) {
        if !self.tool_applicable(self.tools.active) {
            self.tools.active = Tool::Select;
        }
    }

    /// Whether inspector phase section `phase` is locked because it is
    /// downstream of the viewed phase, so its effect can't be shown. The
    /// Document view depends on the whole pipeline, so nothing is locked there.
    pub fn section_locked(&self, phase: Phase) -> bool {
        self.active_phase().is_some_and(|p| phase.index() > p.index())
    }
}
