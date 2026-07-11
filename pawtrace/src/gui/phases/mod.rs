//! The four pipeline phases and the vocabulary shared across them: the
//! sub-views a phase steps through, the concrete pipeline stage each sub-view
//! displays, and the per-phase and per-stage indexing helpers. Each phase's own
//! data (its sub-view list, default sub-view, and inspector section) lives in
//! its module here; this file holds only what every phase shares and the
//! delegation into the phase modules.

mod colors;
mod curves;
mod paint;
mod shapes;

use super::app::App;
use super::compute::{Img, StageImages};
use super::msg::{Msg, Phase};
use iced::Element;
use std::ops::{Index, IndexMut};

/// One intermediate render within a phase. Each names the phase it belongs to,
/// its breadcrumb label, and the stage image it shows (or `None` when that
/// render is not produced yet, so the strip offers it disabled).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubView {
    Source,
    Features,
    Merged,
    Palette,
    Flatten,
    Remap,
    Regions,
    Fates,
    Stack,
    Contours,
    Fit,
    Simplify,
}

impl SubView {
    /// The phase this sub-view belongs to.
    pub fn phase(self) -> Phase {
        match self {
            SubView::Source | SubView::Features | SubView::Merged | SubView::Palette => {
                Phase::Colors
            }
            SubView::Flatten | SubView::Remap => Phase::Paint,
            SubView::Regions | SubView::Fates | SubView::Stack => Phase::Shapes,
            SubView::Contours | SubView::Fit | SubView::Simplify => Phase::Curves,
        }
    }

    /// The breadcrumb label shown in the sub-view panel.
    pub fn label(self) -> &'static str {
        match self {
            SubView::Source => "Source",
            SubView::Features => "Features",
            SubView::Merged => "Merged",
            SubView::Palette => "Palette",
            SubView::Flatten => "Flatten",
            SubView::Remap => "Remap",
            SubView::Regions => "Regions",
            SubView::Fates => "Fates",
            SubView::Stack => "Stack",
            SubView::Contours => "Contours",
            SubView::Fit => "Fit",
            SubView::Simplify => "Simplify",
        }
    }

    /// The pipeline stage this sub-view shows, or `None` while its render is not
    /// yet produced (the feature and merged partition visualizations, the
    /// region fates tint, and the paint stack walk).
    pub fn stage(self) -> Option<Stage> {
        match self {
            SubView::Source => Some(Stage::Source),
            SubView::Palette | SubView::Remap => Some(Stage::Remap),
            SubView::Flatten => Some(Stage::Flatten),
            SubView::Regions => Some(Stage::Regions),
            SubView::Contours => Some(Stage::Contours),
            SubView::Fit => Some(Stage::Fit),
            SubView::Simplify => Some(Stage::Simplify),
            SubView::Features | SubView::Merged | SubView::Fates | SubView::Stack => None,
        }
    }
}

/// A pipeline stage as compute produces it: the display image the preview can
/// show, in [`Stage::ALL`] order. Each carries the raster density it renders at
/// relative to source-crop px and the [`StageImages`] field it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Source,
    Flatten,
    Remap,
    Regions,
    Contours,
    Fit,
    Simplify,
}

impl Stage {
    /// Every stage in pipeline order. The one canonical ordering: positions,
    /// counts, and [`PerStage`] indexing all derive from it.
    pub const ALL: [Stage; 7] = [
        Stage::Source,
        Stage::Flatten,
        Stage::Remap,
        Stage::Regions,
        Stage::Contours,
        Stage::Fit,
        Stage::Simplify,
    ];

    /// This stage's position in [`Stage::ALL`], the index of its [`PerStage`] slot.
    pub fn ordinal(self) -> usize {
        Self::ALL.iter().position(|&s| s == self).expect("ALL contains every stage")
    }

    /// Screen-raster px per source-crop px for this stage's raster. Source is
    /// 1x, Flatten through Regions are the supersample `scale`, the Curves
    /// renders are 2x.
    pub fn density(self, scale: u32) -> f32 {
        match self {
            Stage::Source => 1.0,
            Stage::Flatten | Stage::Remap | Stage::Regions => scale as f32,
            Stage::Contours | Stage::Fit | Stage::Simplify => 2.0,
        }
    }

    /// This stage's rendered image, if it has been computed for the shown layer.
    pub fn image(self, stages: &StageImages) -> Option<&Img> {
        match self {
            Stage::Source => stages.source.as_ref(),
            Stage::Flatten => stages.flat.as_ref(),
            Stage::Remap => stages.remap.as_ref(),
            Stage::Regions => stages.regions.as_ref(),
            Stage::Contours => stages.contours.as_ref(),
            Stage::Fit => stages.render.as_ref(),
            Stage::Simplify => stages.simplified.as_ref(),
        }
    }

}

/// The phases in strip order.
pub const PHASES: [Phase; 4] = [Phase::Colors, Phase::Paint, Phase::Shapes, Phase::Curves];

impl Phase {
    /// This phase's position in strip order, 0 for Colors through 3 for Curves.
    pub fn index(self) -> usize {
        match self {
            Phase::Colors => 0,
            Phase::Paint => 1,
            Phase::Shapes => 2,
            Phase::Curves => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Phase::Colors => "Colors",
            Phase::Paint => "Paint",
            Phase::Shapes => "Shapes",
            Phase::Curves => "Curves",
        }
    }

    pub fn subviews(self) -> &'static [SubView] {
        match self {
            Phase::Colors => colors::SUBVIEWS,
            Phase::Paint => paint::SUBVIEWS,
            Phase::Shapes => shapes::SUBVIEWS,
            Phase::Curves => curves::SUBVIEWS,
        }
    }

    /// The sub-view the phase opens on: the render users already know.
    pub fn default_subview(self) -> SubView {
        match self {
            Phase::Colors => colors::DEFAULT_SUBVIEW,
            Phase::Paint => paint::DEFAULT_SUBVIEW,
            Phase::Shapes => shapes::DEFAULT_SUBVIEW,
            Phase::Curves => curves::DEFAULT_SUBVIEW,
        }
    }

    /// This phase's inspector section: its settings and readouts, the body the
    /// accordion shows while the phase is expanded.
    pub fn inspector(self, app: &App) -> Element<'_, Msg> {
        match self {
            Phase::Colors => colors::inspector(app),
            Phase::Paint => paint::inspector(app),
            Phase::Shapes => shapes::inspector(app),
            Phase::Curves => curves::inspector(app),
        }
    }
}

/// A value stored once per phase, indexed by [`Phase`] rather than a bare index.
#[derive(Debug, Clone, Copy)]
pub struct PerPhase<T>([T; 4]);

impl<T> PerPhase<T> {
    /// A `PerPhase` whose entry for each phase is `f(phase)`.
    pub fn from_fn(mut f: impl FnMut(Phase) -> T) -> Self {
        PerPhase([f(Phase::Colors), f(Phase::Paint), f(Phase::Shapes), f(Phase::Curves)])
    }
}

impl<T> Index<Phase> for PerPhase<T> {
    type Output = T;
    fn index(&self, phase: Phase) -> &T {
        &self.0[phase.index()]
    }
}

impl<T> IndexMut<Phase> for PerPhase<T> {
    fn index_mut(&mut self, phase: Phase) -> &mut T {
        &mut self.0[phase.index()]
    }
}

/// A value stored once per pipeline [`Stage`], indexed by `Stage` rather than a
/// bare ordinal.
#[derive(Debug, Clone, Copy)]
pub struct PerStage<T>([T; Stage::ALL.len()]);

impl<T> PerStage<T> {
    /// A `PerStage` whose entry for each stage is `f(stage)`.
    pub fn from_fn(f: impl FnMut(Stage) -> T) -> Self {
        PerStage(Stage::ALL.map(f))
    }

    /// The entries in [`Stage::ALL`] order.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.0.iter()
    }
}

impl<T> Index<Stage> for PerStage<T> {
    type Output = T;
    fn index(&self, stage: Stage) -> &T {
        &self.0[stage.ordinal()]
    }
}

impl<T> IndexMut<Stage> for PerStage<T> {
    fn index_mut(&mut self, stage: Stage) -> &mut T {
        &mut self.0[stage.ordinal()]
    }
}
