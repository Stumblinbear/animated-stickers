//! Settings profiles, keyed on a plain glob of the layer name: `*` matches
//! any run of characters, everything else is literal (spaces included, so
//! "Deer" is exact, "Deer *" a word prefix, "* Fill" a suffix, "Deer * Fill"
//! anchors both ends). Exactly one profile applies to a layer: the most
//! specific match (more literal characters win, then a suffix over a prefix,
//! then the longer key). Profiles never stack.
//!
//! A layer override is a separate tweak keyed on an exact layer name,
//! applied on top of that one profile, so a single layer can deviate
//! without altering the class it belongs to.
//!
//! Resolution order: Config::default() > tier [default] sections > the one
//! matching profile > the layer override > CLI flags. Two tiers: a global
//! library (roles shared across projects) and the project pawtrace.toml
//! (characters and layer overrides), the project winning a specificity tie.
//! File: pawtrace.toml, searched next to the input file, then cwd.

use crate::color::Srgb;
use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Every field optional: profiles state only their deviations.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Overrides {
    pub detail: Option<f32>,
    pub max_colors: Option<usize>,
    /// Palette colors seeded unconditionally, as "#rrggbb" hex strings.
    pub locked: Option<Vec<String>>,
    pub shade_split: Option<f32>,
    pub shade_noise: Option<f32>,
    pub scale: Option<u32>,
    pub alpha_threshold: Option<u8>,
    pub alphamax: Option<f64>,
    pub opttolerance: Option<f64>,
    pub seam_slack: Option<f64>,
    pub simplify: Option<f64>,
    pub mode_filter: Option<u32>,
    pub color_cleanup: Option<u32>,
    pub smoothing: Option<f32>,
    pub absorb_dist: Option<f32>,
    pub absorb_aggr: Option<f32>,
    pub stroke_merge_dist: Option<f32>,
    pub stroke_merge_width: Option<f32>,
    pub stroke_width: Option<f32>,
    /// "#rrggbb" hex, like `locked`.
    pub stroke_color: Option<String>,
}

impl Overrides {
    pub fn apply(&self, mut c: Config) -> Config {
        // Destructured with no `..` so a field added to Overrides but not
        // applied here is a compile error, never a silently ignored setting.
        let Overrides {
            detail,
            max_colors,
            locked,
            shade_split,
            shade_noise,
            scale,
            alpha_threshold,
            alphamax,
            opttolerance,
            seam_slack,
            simplify,
            mode_filter,
            color_cleanup,
            smoothing,
            absorb_dist,
            absorb_aggr,
            stroke_merge_dist,
            stroke_merge_width,
            stroke_width,
            stroke_color,
        } = self;
        if let Some(v) = *detail {
            c.detail = v;
        }
        if let Some(v) = *max_colors {
            c.max_colors = v;
        }
        if let Some(v) = locked {
            c.locked = v.iter().filter_map(|s| Srgb::from_hex(s)).collect();
        }
        if let Some(v) = *shade_split {
            c.shade_split = v;
        }
        if let Some(v) = *shade_noise {
            c.shade_noise = v;
        }
        if let Some(v) = *scale {
            c.scale = v;
        }
        if let Some(v) = *alpha_threshold {
            c.alpha_threshold = v;
        }
        if let Some(v) = *alphamax {
            c.alphamax = v;
        }
        if let Some(v) = *opttolerance {
            c.opttolerance = v;
        }
        if let Some(v) = *seam_slack {
            c.seam_slack = v;
        }
        if let Some(v) = *simplify {
            c.simplify = v;
        }
        if let Some(v) = *mode_filter {
            c.mode_filter = v;
        }
        if let Some(v) = *color_cleanup {
            c.color_cleanup = v;
        }
        if let Some(v) = *smoothing {
            c.smoothing = v;
        }
        if let Some(v) = *absorb_dist {
            c.absorb_dist = v;
        }
        if let Some(v) = *absorb_aggr {
            c.absorb_aggr = v;
        }
        if let Some(v) = *stroke_merge_dist {
            c.stroke_merge_dist = v;
        }
        if let Some(v) = *stroke_merge_width {
            c.stroke_merge_width = v;
        }
        if let Some(v) = *stroke_width {
            c.stroke_width = v;
        }
        if let Some(v) = stroke_color.as_deref().and_then(Srgb::from_hex) {
            c.stroke_color = v;
        }
        c
    }
}

/// Whether a profile key matches a layer name, as a plain (case-sensitive)
/// glob anchored to the whole name: `*` matches any run of zero or more
/// characters, every other character must match literally. Spaces are
/// ordinary characters, so `Deer` matches only "Deer" exactly, `Deer *`
/// matches "Deer R Hand" (not "Deerhoof", the space is required), `* Fill`
/// matches "Deer L Hand Fill" (not "Refill"), and `Deer * Fill` anchors both
/// ends.
pub fn key_matches(key: &str, layer_name: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let (pat, s) = (key.as_bytes(), layer_name.as_bytes());
    let (mut pi, mut si) = (0usize, 0usize);
    let mut star_p: Option<usize> = None;
    let mut star_s = 0usize;
    while si < s.len() {
        if pi < pat.len() && pat[pi] != b'*' && pat[pi] == s[si] {
            pi += 1;
            si += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star_p = Some(pi);
            star_s = si;
            pi += 1;
        } else if let Some(sp) = star_p {
            // Mismatch: let the last `*` absorb one more character and retry.
            pi = sp + 1;
            star_s += 1;
            si = star_s;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

/// Ordering key for choosing the one matching profile: more literal (non-`*`)
/// characters rank higher, then a leading-`*` suffix over a prefix, then the
/// longer key.
fn specificity(key: &str) -> (usize, bool, usize) {
    let literals = key.chars().filter(|&c| c != '*').count();
    (literals, key.starts_with('*'), key.len())
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Profiles {
    #[serde(default)]
    pub default: Overrides,
    /// Class key -> overrides. A key matches layer names as a prefix
    /// ("Deer"), or as a suffix when it starts with "*" ("* Fill"). Exactly
    /// one profile applies to a layer (the most specific match); profiles
    /// never stack.
    #[serde(default)]
    pub profiles: BTreeMap<String, Overrides>,
    /// Exact layer name -> tweak, applied on top of the layer's one matching
    /// profile. A layer override adjusts a single layer without touching the
    /// class it belongs to. Project tier only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, Overrides>,
    /// Exact layer name -> profile key. An explicit assignment pins a layer to
    /// a named profile regardless of glob matching, overriding specificity.
    /// Project tier only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assign: BTreeMap<String, String>,
}

impl Profiles {
    pub fn load_near(input: &std::path::Path) -> Self {
        let candidates = [
            input.parent().map(|p| p.join("pawtrace.toml")),
            Some(std::path::PathBuf::from("pawtrace.toml")),
        ];
        for c in candidates.into_iter().flatten() {
            if let Ok(s) = std::fs::read_to_string(&c) {
                match toml::from_str::<Profiles>(&s) {
                    Ok(p) => return p,
                    Err(e) => eprintln!("pawtrace.toml parse error ({}): {e}", c.display()),
                }
            }
        }
        Self::default()
    }

    /// The most specific profile matching `layer_name`, or `None`. Suffix
    /// keys ("* Fill", a role) beat prefix keys (a character) whatever their
    /// length; within a kind the longer key wins.
    pub fn best_profile(&self, layer_name: &str) -> Option<(&str, &Overrides)> {
        self.profiles
            .iter()
            .filter(|(k, _)| key_matches(k, layer_name))
            .max_by_key(|(k, _)| (specificity(k), k.as_str()))
            .map(|(k, ov)| (k.as_str(), ov))
    }
}

/// Which tier of the [`ProfileStack`] an edit or lookup targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The per-user library: traits shared across projects ("* Fill", eye
    /// settings). Lives in the user config directory.
    Global,
    /// pawtrace.toml next to the art: characters and layer overrides.
    Project,
}

/// Two-tier profiles: the global library underneath, the project file on
/// top. Each tier cascades internally by specificity; the project tier wins
/// wherever both set a field.
#[derive(Default, Clone, Debug)]
pub struct ProfileStack {
    pub global: Profiles,
    pub project: Profiles,
}

/// The global library's file, inside the app's [`data_dir`](crate::paths::data_dir).
pub fn global_path() -> Option<std::path::PathBuf> {
    crate::paths::data_dir().map(|d| d.join("pawtrace.toml"))
}

/// A borrowed view over a global tier and one project tier.
#[derive(Clone, Copy)]
pub struct StackRef<'a> {
    pub global: &'a Profiles,
    pub project: &'a Profiles,
}

impl<'a> StackRef<'a> {
    /// Resolve a layer's config: built-in defaults, then each tier's
    /// `[default]`, then the single most specific matching profile, then the
    /// project layer override for this exact layer. The returned name is that
    /// one profile, for display. Profiles never stack.
    pub fn resolve(self, layer_name: &str) -> (Config, Option<String>) {
        let mut c = self.profile_base();
        let matched = self.best_profile(layer_name);
        if let Some((_, ov)) = matched {
            c = ov.apply(c);
        }
        if let Some(ov) = self.project.overrides.get(layer_name) {
            c = ov.apply(c);
        }
        (c, matched.map(|(k, _)| k.to_string()))
    }

    /// The profile governing a layer: an explicit project assignment if one
    /// names an existing profile, otherwise the single most specific matching
    /// profile across both tiers, project preferred on an equal-specificity
    /// tie.
    pub fn best_profile(self, layer_name: &str) -> Option<(&'a str, &'a Overrides)> {
        if let Some(key) = self.project.assign.get(layer_name) {
            if let Some(hit) = self.lookup_key(key) {
                return Some(hit);
            }
        }
        match (
            self.global.best_profile(layer_name),
            self.project.best_profile(layer_name),
        ) {
            (g, None) => g,
            (None, p) => p,
            (Some(g), Some(p)) => {
                Some(if specificity(p.0) >= specificity(g.0) { p } else { g })
            }
        }
    }

    /// A profile by exact key, project tier preferred, for an explicit
    /// assignment whose key an unmatching glob would otherwise miss.
    fn lookup_key(self, key: &str) -> Option<(&'a str, &'a Overrides)> {
        self.project
            .profiles
            .get_key_value(key)
            .or_else(|| self.global.profiles.get_key_value(key))
            .map(|(k, ov)| (k.as_str(), ov))
    }

    /// The config both tiers' `[default]` sections resolve to, which every
    /// profile applies on top of.
    pub fn profile_base(self) -> Config {
        self.project.default.apply(self.global.default.apply(Config::default()))
    }

    /// The base a layer override edits on top of: the full resolution of the
    /// layer minus the override itself (defaults plus the matching profile).
    pub fn override_base(self, layer_name: &str) -> Config {
        let mut c = self.profile_base();
        if let Some((_, ov)) = self.best_profile(layer_name) {
            c = ov.apply(c);
        }
        c
    }

    /// Whether a layer has its own override.
    pub fn has_override(self, layer_name: &str) -> bool {
        self.project.overrides.contains_key(layer_name)
    }

    /// Display tag: the single matching profile's name.
    pub fn match_name(self, layer_name: &str) -> Option<String> {
        self.best_profile(layer_name).map(|(k, _)| k.to_string())
    }

    /// An owned stack cloned from both tiers.
    pub fn to_owned(self) -> ProfileStack {
        ProfileStack {
            global: self.global.clone(),
            project: self.project.clone(),
        }
    }
}

/// Loads the global library, or an empty tier when there is none.
pub fn load_global() -> Profiles {
    global_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persists the global library to its current location, creating the directory
/// if needed.
///
/// # Errors
///
/// Returns an error if no config directory can be located, or if serializing
/// or writing the file fails.
pub fn save_global(tier: &Profiles) -> std::io::Result<()> {
    let path = global_path()
        .ok_or_else(|| std::io::Error::other("no config directory for the global library"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = toml::to_string_pretty(tier)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, s)
}

impl ProfileStack {
    pub fn load_near(input: &std::path::Path) -> Self {
        Self {
            global: load_global(),
            project: Profiles::load_near(input),
        }
    }

    pub fn as_stack_ref(&self) -> StackRef<'_> {
        StackRef { global: &self.global, project: &self.project }
    }

    pub fn tier(&self, scope: Scope) -> &Profiles {
        match scope {
            Scope::Global => &self.global,
            Scope::Project => &self.project,
        }
    }

    pub fn tier_mut(&mut self, scope: Scope) -> &mut Profiles {
        match scope {
            Scope::Global => &mut self.global,
            Scope::Project => &mut self.project,
        }
    }

    /// See [`StackRef::resolve`].
    pub fn resolve(&self, layer_name: &str) -> (Config, Option<String>) {
        self.as_stack_ref().resolve(layer_name)
    }

    /// See [`StackRef::best_profile`].
    pub fn best_profile(&self, layer_name: &str) -> Option<(&str, &Overrides)> {
        self.as_stack_ref().best_profile(layer_name)
    }

    /// See [`StackRef::profile_base`].
    pub fn profile_base(&self) -> Config {
        self.as_stack_ref().profile_base()
    }

    /// The base a `[default]` edit at `scope` layers onto: nothing for the
    /// global default, the global default for the project default.
    pub fn default_base(&self, scope: Scope) -> Config {
        match scope {
            Scope::Global => Config::default(),
            Scope::Project => self.global.default.apply(Config::default()),
        }
    }

    /// See [`StackRef::override_base`].
    pub fn override_base(&self, layer_name: &str) -> Config {
        self.as_stack_ref().override_base(layer_name)
    }

    /// See [`StackRef::has_override`].
    pub fn has_override(&self, layer_name: &str) -> bool {
        self.project.overrides.contains_key(layer_name)
    }

    /// See [`StackRef::match_name`].
    pub fn match_name(&self, layer_name: &str) -> Option<String> {
        self.as_stack_ref().match_name(layer_name)
    }
}

/// Overrides holding only the fields where `cfg` differs from `base`: the
/// minimal delta that reproduces `cfg` when applied on top of `base`.
pub fn diff(base: &Config, cfg: &Config) -> Overrides {
    fn d<T: PartialEq>(b: T, c: T) -> Option<T> {
        (b != c).then_some(c)
    }
    Overrides {
        detail: d(base.detail, cfg.detail),
        max_colors: d(base.max_colors, cfg.max_colors),
        locked: (base.locked != cfg.locked)
            .then(|| cfg.locked.iter().map(|c| c.to_hex()).collect()),
        shade_split: d(base.shade_split, cfg.shade_split),
        shade_noise: d(base.shade_noise, cfg.shade_noise),
        scale: d(base.scale, cfg.scale),
        alpha_threshold: d(base.alpha_threshold, cfg.alpha_threshold),
        alphamax: d(base.alphamax, cfg.alphamax),
        opttolerance: d(base.opttolerance, cfg.opttolerance),
        seam_slack: d(base.seam_slack, cfg.seam_slack),
        simplify: d(base.simplify, cfg.simplify),
        mode_filter: d(base.mode_filter, cfg.mode_filter),
        color_cleanup: d(base.color_cleanup, cfg.color_cleanup),
        smoothing: d(base.smoothing, cfg.smoothing),
        absorb_dist: d(base.absorb_dist, cfg.absorb_dist),
        absorb_aggr: d(base.absorb_aggr, cfg.absorb_aggr),
        stroke_merge_dist: d(base.stroke_merge_dist, cfg.stroke_merge_dist),
        stroke_merge_width: d(base.stroke_merge_width, cfg.stroke_merge_width),
        stroke_width: d(base.stroke_width, cfg.stroke_width),
        stroke_color: (base.stroke_color != cfg.stroke_color)
            .then(|| cfg.stroke_color.to_hex()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_key_is_an_exact_match() {
        assert!(key_matches("Deer", "Deer"));
        assert!(!key_matches("Deer", "Deer R Hand")); // no trailing wildcard
        assert!(!key_matches("Deer", "Deerhoof"));
    }

    #[test]
    fn trailing_star_is_a_word_prefix() {
        assert!(key_matches("Deer *", "Deer R Hand"));
        assert!(!key_matches("Deer *", "Deerhoof")); // the space is required
        assert!(key_matches("Deer*", "Deerhoof")); // no space: raw char prefix
    }

    #[test]
    fn leading_star_is_a_suffix() {
        assert!(key_matches("* Fill", "Deer L Hand Fill"));
        assert!(!key_matches("* Fill", "Refill")); // no " Fill", wrong casing
        assert!(!key_matches("* Fill", "Deer Fill Hand")); // not at the end
        assert!(!key_matches("* Fill", "Fill")); // the leading space is required
    }

    #[test]
    fn glob_matches_interior_and_multiple_wildcards() {
        // Interior wildcard anchors both ends.
        assert!(key_matches("Deer * Fill", "Deer L Hand Fill"));
        assert!(!key_matches("Deer * Fill", "Seff L Hand Fill")); // wrong start
        assert!(!key_matches("Deer * Fill", "Deer L Hand")); // wrong end
        // Multiple wildcards.
        assert!(key_matches("* Hand *", "Deer L Hand Fill"));
        assert!(key_matches("* L * Fill", "Deer L Hand Fill"));
        assert!(!key_matches("* L * Fill", "Deer R Hand Fill")); // no " L "
        // A lone star matches everything; an empty key matches nothing.
        assert!(key_matches("*", "anything at all"));
        assert!(!key_matches("", "Deer"));
    }

    #[test]
    fn more_literal_pattern_beats_broad_suffix() {
        let mut s = ProfileStack::default();
        s.project.profiles.insert(
            "* Fill".into(),
            Overrides { detail: Some(1.0), ..Default::default() },
        );
        s.project.profiles.insert(
            "Deer * Fill".into(),
            Overrides { detail: Some(2.0), ..Default::default() },
        );
        // "Deer * Fill" has more literal characters, so it outranks "* Fill".
        let (cfg, matched) = s.resolve("Deer L Hand Fill");
        assert_eq!(cfg.detail, 2.0);
        assert_eq!(matched.as_deref(), Some("Deer * Fill"));
        // A non-Deer fill still falls to the broad "* Fill".
        let (cfg, _) = s.resolve("Seff Tail Fill");
        assert_eq!(cfg.detail, 1.0);
    }

    #[test]
    fn one_profile_applies_not_a_cascade() {
        let mut s = ProfileStack::default();
        s.project.default.detail = Some(1.0);
        s.project.profiles.insert(
            "Deer *".into(),
            Overrides { detail: Some(2.0), max_colors: Some(9), ..Default::default() },
        );
        s.project.profiles.insert(
            "* Fill".into(),
            Overrides { detail: Some(3.0), ..Default::default() },
        );
        // A Fill layer: the suffix wins and is the ONLY profile applied, so
        // the Deer prefix's max_colors does not leak in.
        let (cfg, matched) = s.resolve("Deer L Hand Fill");
        assert_eq!(cfg.detail, 3.0);
        assert_eq!(cfg.max_colors, Config::default().max_colors);
        assert_eq!(matched.as_deref(), Some("* Fill"));
        // A non-Fill Deer layer gets the Deer profile over the default.
        let (cfg, matched) = s.resolve("Deer R Arm");
        assert_eq!(cfg.detail, 2.0);
        assert_eq!(cfg.max_colors, 9);
        assert_eq!(matched.as_deref(), Some("Deer *"));
    }

    #[test]
    fn suffix_beats_prefix_on_equal_literal_count() {
        let mut s = ProfileStack::default();
        // "Deer *" and "* Fill" have the same literal-character count, so the
        // suffix (role) wins the tie over the prefix (character).
        s.project.profiles.insert(
            "Deer *".into(),
            Overrides { detail: Some(2.0), ..Default::default() },
        );
        s.project.profiles.insert(
            "* Fill".into(),
            Overrides { detail: Some(3.0), ..Default::default() },
        );
        let (cfg, matched) = s.resolve("Deer L Hand Fill");
        assert_eq!(cfg.detail, 3.0);
        assert_eq!(matched.as_deref(), Some("* Fill"));
    }

    #[test]
    fn layer_override_applies_on_top_of_its_profile() {
        let mut s = ProfileStack::default();
        s.project.profiles.insert(
            "Seff *".into(),
            Overrides { detail: Some(2.0), max_colors: Some(9), ..Default::default() },
        );
        s.project.overrides.insert(
            "Seff Body".into(),
            Overrides { detail: Some(5.0), ..Default::default() },
        );
        let (cfg, matched) = s.resolve("Seff Body");
        assert_eq!(cfg.detail, 5.0); // override wins the field it sets
        assert_eq!(cfg.max_colors, 9); // the profile still contributes the rest
        assert_eq!(matched.as_deref(), Some("Seff *")); // tag is the profile, not the override
        assert!(s.has_override("Seff Body"));
        // A sibling layer without an override gets just the profile.
        let (cfg, _) = s.resolve("Seff Head");
        assert_eq!(cfg.detail, 2.0);
    }

    #[test]
    fn project_profile_wins_tie_over_global() {
        let mut s = ProfileStack::default();
        s.global.profiles.insert(
            "* Fill".into(),
            Overrides { detail: Some(3.0), max_colors: Some(2), ..Default::default() },
        );
        s.project.profiles.insert(
            "* Fill".into(),
            Overrides { detail: Some(4.0), ..Default::default() },
        );
        let (cfg, _) = s.resolve("Deer L Hand Fill");
        assert_eq!(cfg.detail, 4.0); // project "* Fill" wins the tie
        // Only one profile applies, so global's max_colors does not leak in.
        assert_eq!(cfg.max_colors, Config::default().max_colors);
    }

    #[test]
    fn global_profile_applies_when_project_has_none() {
        let mut s = ProfileStack::default();
        s.global.profiles.insert(
            "* Fill".into(),
            Overrides { detail: Some(3.0), ..Default::default() },
        );
        let (cfg, matched) = s.resolve("Deer Fill");
        assert_eq!(cfg.detail, 3.0);
        assert_eq!(matched.as_deref(), Some("* Fill"));
    }

    #[test]
    fn assignment_beats_glob_and_restores_on_removal() {
        let mut s = ProfileStack::default();
        s.project.profiles.insert(
            "Deer *".into(),
            Overrides { detail: Some(2.0), ..Default::default() },
        );
        s.project.profiles.insert(
            "Special".into(),
            Overrides { detail: Some(9.0), ..Default::default() },
        );
        // "Deer L Hand" globs to "Deer *"; an explicit assignment to "Special"
        // wins even though "Special" would never match by glob.
        s.project.assign.insert("Deer L Hand".into(), "Special".into());
        let (cfg, matched) = s.resolve("Deer L Hand");
        assert_eq!(cfg.detail, 9.0);
        assert_eq!(matched.as_deref(), Some("Special"));
        // Removing the assignment restores the glob match.
        s.project.assign.remove("Deer L Hand");
        let (cfg, matched) = s.resolve("Deer L Hand");
        assert_eq!(cfg.detail, 2.0);
        assert_eq!(matched.as_deref(), Some("Deer *"));
    }

    #[test]
    fn assignment_to_a_global_tier_key_works() {
        let mut s = ProfileStack::default();
        s.global.profiles.insert(
            "Eye".into(),
            Overrides { detail: Some(7.0), ..Default::default() },
        );
        s.project.assign.insert("Deer L Hand".into(), "Eye".into());
        let (cfg, matched) = s.resolve("Deer L Hand");
        assert_eq!(cfg.detail, 7.0);
        assert_eq!(matched.as_deref(), Some("Eye"));
    }

    #[test]
    fn assignment_round_trips_toml() {
        let mut p = Profiles::default();
        p.assign.insert("Deer L Hand".into(), "Special".into());
        let toml = toml::to_string(&p).unwrap();
        assert!(toml.contains("[assign]"));
        let back: Profiles = toml::from_str(&toml).unwrap();
        assert_eq!(back.assign.get("Deer L Hand").map(String::as_str), Some("Special"));
        // An empty assign map is skipped, leaving no orphan table.
        let empty = toml::to_string(&Profiles::default()).unwrap();
        assert!(!empty.contains("[assign]"));
    }

    #[test]
    fn promoting_a_layer_keeps_its_resolution() {
        let mut s = ProfileStack::default();
        s.project.profiles.insert(
            "Deer *".into(),
            Overrides { detail: Some(2.0), max_colors: Some(9), ..Default::default() },
        );
        // 7.0 differs from Config::default().detail (5.0), which the diff
        // below would otherwise omit.
        s.project.overrides.insert(
            "Deer L Hand".into(),
            Overrides { detail: Some(7.0), ..Default::default() },
        );
        let (before, _) = s.resolve("Deer L Hand");
        // Mirrors the GUI's promote-to-profile: the new profile is the layer's
        // full deviation from the tier defaults, because the assignment
        // replaces both the old glob profile and the override.
        let ov = diff(&s.profile_base(), &before);
        assert_eq!(ov.detail, Some(7.0));
        assert_eq!(ov.max_colors, Some(9)); // the old profile's field comes along
        s.project.profiles.insert("Deer 2 *".into(), ov);
        s.project.assign.insert("Deer L Hand".into(), "Deer 2 *".into());
        s.project.overrides.remove("Deer L Hand");
        let (after, matched) = s.resolve("Deer L Hand");
        assert_eq!(after, before);
        assert_eq!(matched.as_deref(), Some("Deer 2 *"));
    }

    #[test]
    fn diff_records_only_deviations_and_round_trips() {
        let base = Config::default();
        let mut cfg = base.clone();
        cfg.detail = 9.0;
        cfg.locked = vec![Srgb([1, 2, 3])];
        let ov = diff(&base, &cfg);
        assert_eq!(ov.detail, Some(9.0));
        assert_eq!(ov.locked.as_deref(), Some(&["#010203".to_string()][..]));
        assert_eq!(ov.max_colors, None);
        assert_eq!(ov.apply(base), cfg);
    }

    #[test]
    fn edit_bases_layer_at_the_right_level() {
        let mut s = ProfileStack::default();
        s.project.default.detail = Some(1.0);
        s.project.profiles.insert(
            "Seff *".into(),
            Overrides { detail: Some(2.0), ..Default::default() },
        );
        // A profile edits on top of the resolved defaults.
        assert_eq!(s.profile_base().detail, 1.0);
        // A layer override edits on top of defaults plus the matching profile.
        assert_eq!(s.override_base("Seff Body").detail, 2.0);
        // A [default] edit sits on the bare built-in config (no self-apply).
        assert_eq!(s.default_base(Scope::Project).detail, Config::default().detail);
    }
}
