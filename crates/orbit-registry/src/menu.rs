//! Building the radial menu model.
//!
//! The UI renders exactly what this module returns, so "context-aware" lives
//! here: petals are generated from the dragged files, engine availability, pins
//! and recent use, then grouped into the ring's categories. The model is two
//! levels deep by construction, never more.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{Chain, ChainStep, Id, Kind};
use crate::probe::SourceFile;
use crate::registry::{EngineChoice, Registry};
use crate::selector::any_selector_matches;

/// How many petals a category shows before rolling the rest into "More".
pub const DEFAULT_MAX_PETALS: usize = 8;

/// Score added for every extra dragged file a petal covers.
const COVERAGE_BONUS: i32 = 40;
/// Score added to the first pinned petal, decreasing for each later pin.
const PIN_BONUS: i32 = 1_000;
/// Score subtracted from a petal whose engine is missing.
const DISABLED_PENALTY: i32 = 5_000;
/// Ceiling on the frecency bonus.
const USAGE_CEILING: u32 = 200;

/// What the caller wants a menu for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuRequest {
    /// Dragged files; the first file drives the labels, the rest constrain which
    /// petals survive.
    pub files: Vec<SourceFile>,
    /// Engine ids known to be installed. `None` means "assume everything", which
    /// is what a preview or the registry self-check wants.
    #[serde(default)]
    pub available: Option<BTreeSet<Id>>,
    /// Petal ids the user pinned, most important first.
    #[serde(default)]
    pub pins: Vec<Id>,
    /// How often each petal has been used, for frecency ordering.
    #[serde(default)]
    pub usage: BTreeMap<Id, u32>,
    /// Override for the per-category petal cap.
    #[serde(default)]
    pub max_petals: Option<usize>,
    /// Restrict the ring to these category ids, in this order.
    #[serde(default)]
    pub categories: Option<Vec<Id>>,
    /// Show petals whose engine is missing, greyed out with a reason.
    #[serde(default)]
    pub include_unavailable: Option<bool>,
}

impl MenuRequest {
    /// A request for a set of files, everything else defaulted.
    #[must_use]
    pub fn for_files(files: Vec<SourceFile>) -> Self {
        Self {
            files,
            available: None,
            pins: Vec::new(),
            usage: BTreeMap::new(),
            max_petals: None,
            categories: None,
            include_unavailable: None,
        }
    }

    /// Whether missing engines should show up as disabled petals.
    #[must_use]
    pub fn shows_unavailable(&self) -> bool {
        self.include_unavailable.unwrap_or(true)
    }

    /// Petal cap per category.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.max_petals.unwrap_or(DEFAULT_MAX_PETALS).max(1)
    }
}

/// One input, as the UI should describe it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSummary {
    /// File name with extension.
    pub name: String,
    /// Absolute path, when the input came from disk.
    pub path: Option<String>,
    /// Matched format id.
    pub format: Option<Id>,
    /// Human format label.
    pub format_label: Option<String>,
    /// Family.
    pub kind: Kind,
    /// Lower-case extension.
    pub ext: String,
    /// Size in bytes.
    pub size_bytes: u64,
}

/// What a petal does when it is dropped on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetalKind {
    /// Convert to the referenced format.
    Conversion,
    /// Run the referenced editing action.
    Action,
    /// Run the referenced chain or preset.
    Preset,
    /// UI affordance: search, more, options.
    Operator,
}

/// Why a petal cannot be used.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Disabled {
    /// Machine-readable reason, for example `engine_missing`.
    pub code: String,
    /// Human sentence.
    pub reason: String,
    /// What the user could do about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// One slice of the outer ring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Petal {
    /// Stable id, unique inside the menu.
    pub id: Id,
    /// Label shown on hover.
    pub label: String,
    /// Monoline icon name, or a format id for conversions.
    pub icon: String,
    /// What dropping does.
    pub kind: PetalKind,
    /// Format id, action id, chain id or operator name.
    pub reference: Id,
    /// Engine that would run it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<Id>,
    /// Option catalog keys the panel should show.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Id>,
    /// Values used on a plain drop.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<Id, serde_json::Value>,
    /// Steps, for chain petals.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<ChainStep>,
    /// Short marker such as "2 of 3 files" or "pinned 1".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    /// Set when the petal is visible but not usable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<Disabled>,
    /// Ranking score, exposed for tests and for animating order.
    pub score: i32,
    /// How many of the dragged files this petal covers.
    pub coverage: usize,
    /// Total files in the request.
    pub total: usize,
    /// True when Alt or a hold should open the options panel.
    pub needs_options: bool,
}

impl Petal {
    /// True when the petal can be dropped on right now.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.disabled.is_none()
    }
}

/// One category of the inner ring, with its petals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryView {
    /// Category id.
    pub id: Id,
    /// Human label.
    pub label: String,
    /// Monoline icon name.
    pub icon: String,
    /// Theme accent token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Ring position.
    pub order: i32,
    /// Petals, best first.
    pub petals: Vec<Petal>,
    /// How many petals were rolled into "More".
    pub hidden: usize,
}

/// The complete menu, ready to render.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuModel {
    /// The inputs, for the header and badges.
    pub files: Vec<FileSummary>,
    /// Categories with at least one petal.
    pub categories: Vec<CategoryView>,
    /// Petals generated before capping, so the UI can say "8 of 340 shown".
    pub candidates: usize,
    /// Shared family when every file agrees, useful for a title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_kind: Option<Kind>,
}

struct Candidate {
    category: Id,
    petal: Petal,
}

/// Result of asking whether a chain can run on one input.
enum ChainOutcome {
    /// Every step applies and every engine is installed.
    Applies,
    /// Every step applies but an engine is missing.
    Missing(Id),
    /// The chain does not describe this input at all.
    Never,
}

impl Registry {
    /// Build the radial menu for a set of dragged files.
    #[must_use]
    pub fn menu(&self, request: &MenuRequest) -> MenuModel {
        let available = request
            .available
            .clone()
            .unwrap_or_else(|| self.all_engine_ids());
        let total = request.files.len();
        let mut candidates = self.collect_candidates(request, &available, total);

        // Coverage is only known once every file has been considered, so the
        // per-file bonus and the "2 of 3 files" badge are applied here.
        for candidate in candidates.values_mut() {
            let coverage = candidate.petal.coverage;
            candidate.petal.score +=
                COVERAGE_BONUS * i32::try_from(coverage.saturating_sub(1)).unwrap_or(i32::MAX);
            if coverage < total {
                candidate.petal.badge = Some(format!("{coverage} of {total} files"));
            }
        }

        let candidate_count = candidates.len();
        let mut by_category: BTreeMap<Id, Vec<Petal>> = BTreeMap::new();
        for candidate in candidates.into_values() {
            by_category
                .entry(candidate.category)
                .or_default()
                .push(candidate.petal);
        }

        let categories = self.build_categories(request, by_category, total);

        let shared_kind = request
            .files
            .first()
            .map(|file| file.probe.kind)
            .filter(|kind| request.files.iter().all(|file| file.probe.kind == *kind));

        MenuModel {
            files: request
                .files
                .iter()
                .map(|file| self.summary(file))
                .collect(),
            categories,
            candidates: candidate_count,
            shared_kind,
        }
    }

    /// Gather one candidate per operation, folding duplicates within a file
    /// first (so two rules offering "heic -> png" count as one file) and then
    /// across files (so coverage reflects how many dragged files a petal suits).
    fn collect_candidates(
        &self,
        request: &MenuRequest,
        available: &BTreeSet<Id>,
        total: usize,
    ) -> BTreeMap<Id, Candidate> {
        let include_unavailable = request.shows_unavailable();
        let mut candidates: BTreeMap<Id, Candidate> = BTreeMap::new();
        for file in &request.files {
            let mut from_file: BTreeMap<Id, (Id, Petal)> = BTreeMap::new();
            let mut collected = Vec::new();
            self.conversion_petals(
                file,
                request,
                available,
                include_unavailable,
                total,
                &mut collected,
            );
            self.action_petals(file, request, available, total, &mut collected);
            self.chain_petals(file, request, available, total, &mut collected);

            for (key, category, mut petal) in collected {
                petal.total = total;
                petal.coverage = 1;
                match from_file.get(&key) {
                    Some((_, existing)) if existing.score >= petal.score => {}
                    _ => {
                        from_file.insert(key, (category, petal));
                    }
                }
            }

            for (key, (category, petal)) in from_file {
                match candidates.get_mut(&key) {
                    Some(existing) => {
                        let coverage = existing.petal.coverage + 1;
                        if petal.score > existing.petal.score {
                            existing.petal = petal;
                        }
                        existing.petal.coverage = coverage;
                    }
                    None => {
                        candidates.insert(key, Candidate { category, petal });
                    }
                }
            }
        }
        candidates
    }

    /// Group ranked petals into ring categories, applying the per-ring cap and
    /// appending the "More"/"Search" operator petals. A menu that matched
    /// nothing still falls back to "Search", so the user is never stuck.
    fn build_categories(
        &self,
        request: &MenuRequest,
        mut by_category: BTreeMap<Id, Vec<Petal>>,
        total: usize,
    ) -> Vec<CategoryView> {
        let allowed = request.categories.as_deref();
        let mut categories = Vec::new();
        for category in self.categories() {
            if allowed.is_some_and(|ids| !ids.contains(&category.id)) {
                continue;
            }
            let mut petals = by_category.remove(&category.id).unwrap_or_default();
            if petals.is_empty() && category.id != "more" {
                continue;
            }
            petals.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.label.cmp(&b.label)));

            let limit = request.limit();
            let hidden = petals.len().saturating_sub(limit);
            if hidden > 0 {
                petals.truncate(limit);
                petals.push(operator_petal(
                    "op:more",
                    &format!("More… ({hidden} hidden)"),
                    "op:search",
                    500,
                    total,
                ));
            }
            if category.id == "more" {
                petals.push(operator_petal(
                    "op:search",
                    "Search all…",
                    "op:search",
                    400,
                    total,
                ));
            }

            categories.push(CategoryView {
                id: category.id.clone(),
                label: category.label.clone(),
                icon: category.icon.clone(),
                accent: category.accent.clone(),
                order: category.order,
                petals,
                hidden,
            });
        }

        if categories.is_empty() && !request.files.is_empty() {
            categories.push(CategoryView {
                id: "more".to_owned(),
                label: "More".to_owned(),
                icon: "dots".to_owned(),
                accent: Some("muted".to_owned()),
                order: 0,
                petals: vec![operator_petal(
                    "op:search",
                    "Search all…",
                    "op:search",
                    400,
                    total,
                )],
                hidden: 0,
            });
        }

        categories
    }

    fn summary(&self, file: &SourceFile) -> FileSummary {
        FileSummary {
            name: file.file_name(),
            path: file
                .path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            format: file.probe.format.clone(),
            format_label: file
                .probe
                .format
                .as_deref()
                .and_then(|id| self.format(id))
                .map(|format| format.label.clone()),
            kind: file.probe.kind,
            ext: file.probe.ext.clone(),
            size_bytes: file.size_bytes,
        }
    }

    fn conversion_petals(
        &self,
        file: &SourceFile,
        request: &MenuRequest,
        available: &BTreeSet<Id>,
        include_unavailable: bool,
        total: usize,
        out: &mut Vec<(Id, Id, Petal)>,
    ) {
        for pair in self.pairs_for(file, available, include_unavailable) {
            let Some(target) = self.format(&pair.to) else {
                continue;
            };
            let rule = self.rules().iter().find(|rule| rule.id == pair.rule);
            let runnable = pair.choice.chosen.is_some();
            out.push((
                format!("conv:{}", pair.to),
                "convert".to_owned(),
                Petal {
                    id: pair.to.clone(),
                    label: target.label.clone(),
                    icon: target.id.clone(),
                    kind: PetalKind::Conversion,
                    reference: target.id.clone(),
                    engine: pair.choice.chosen.clone(),
                    options: rule.map(|rule| rule.options.clone()).unwrap_or_default(),
                    defaults: rule.map(|rule| rule.defaults.clone()).unwrap_or_default(),
                    steps: Vec::new(),
                    badge: None,
                    disabled: self.disabled_from_choice(&pair.choice),
                    score: Self::score(pair.score, &pair.to, request, runnable),
                    coverage: 1,
                    total,
                    needs_options: pair.needs_options,
                },
            ));
        }
    }

    fn action_petals(
        &self,
        file: &SourceFile,
        request: &MenuRequest,
        available: &BTreeSet<Id>,
        total: usize,
        out: &mut Vec<(Id, Id, Petal)>,
    ) {
        for action in self.actions() {
            if !action.ring {
                // Palette-only and chain-only actions stay out of the ring.
                continue;
            }
            let applies = any_selector_matches(&action.applies_to, &file.probe);
            if !applies {
                continue;
            }
            if usize::try_from(action.requires_files).unwrap_or(1) > total {
                // "Merge" with one file selected is not a thing; stay quiet.
                continue;
            }

            let kind = file.probe.kind;
            let resolvable = action.resolves_via.as_deref() == Some("conversions");
            let choice = if resolvable {
                if !self.has_any_conversion(&file.probe) {
                    continue;
                }
                EngineChoice::default()
            } else {
                let choice = self.choose_engine(
                    action.engines_for(kind),
                    action.requires_for(kind),
                    &file.probe,
                    None,
                    available,
                );
                if choice.chosen.is_none() && choice.missing.is_none() {
                    continue;
                }
                choice
            };

            let runnable = choice.chosen.is_some() || resolvable;
            out.push((
                format!("act:{}", action.id),
                action.category.clone(),
                Petal {
                    id: action.id.clone(),
                    label: action.label.clone(),
                    icon: action.icon.clone(),
                    kind: PetalKind::Action,
                    reference: action.id.clone(),
                    engine: choice.chosen.clone(),
                    options: action.options.clone(),
                    defaults: action.defaults.clone(),
                    steps: Vec::new(),
                    badge: None,
                    disabled: self.disabled_from_choice(&choice),
                    score: Self::score(action.rank, &action.id, request, runnable),
                    coverage: 1,
                    total,
                    needs_options: !action.options.is_empty(),
                },
            ));
        }
    }

    fn chain_petals(
        &self,
        file: &SourceFile,
        request: &MenuRequest,
        available: &BTreeSet<Id>,
        total: usize,
        out: &mut Vec<(Id, Id, Petal)>,
    ) {
        for chain in self.chains() {
            let disabled = match self.chain_outcome(chain, file, available) {
                ChainOutcome::Never => continue,
                ChainOutcome::Applies => None,
                ChainOutcome::Missing(engine) => Some(self.disabled_for_engine(&engine)),
            };
            // A pinned chain outranks everything that is not pinned.
            let base = chain.pin.map_or(80, |pin| 120 - i32::from(pin));
            let runnable = disabled.is_none();
            out.push((
                format!("chain:{}", chain.id),
                chain.category.clone(),
                Petal {
                    id: chain.id.clone(),
                    label: chain.label.clone(),
                    icon: if chain.icon.is_empty() {
                        "sparkle".to_owned()
                    } else {
                        chain.icon.clone()
                    },
                    kind: PetalKind::Preset,
                    reference: chain.id.clone(),
                    engine: None,
                    options: Vec::new(),
                    defaults: BTreeMap::new(),
                    steps: chain.steps.clone(),
                    badge: chain
                        .pin
                        .map(|pin| format!("pinned {}", u16::from(pin) + 1)),
                    disabled,
                    score: Self::score(base, &chain.id, request, runnable),
                    coverage: 1,
                    total,
                    needs_options: false,
                },
            ));
        }
    }

    /// Whether a chain applies to this input, and if it does but cannot run,
    /// which engine is missing.
    fn chain_outcome(
        &self,
        chain: &Chain,
        file: &SourceFile,
        available: &BTreeSet<Id>,
    ) -> ChainOutcome {
        if !chain.applies_to.is_empty()
            && !chain
                .applies_to
                .iter()
                .any(|selector| selector.matches(&file.probe))
        {
            return ChainOutcome::Never;
        }

        let everything = self.all_engine_ids();
        let mut missing = None;
        for step in &chain.steps {
            let Some(action) = self.action(&step.action) else {
                return ChainOutcome::Never;
            };
            let applies = any_selector_matches(&action.applies_to, &file.probe);
            if !applies {
                return ChainOutcome::Never;
            }
            if action.resolves_via.as_deref() == Some("conversions") {
                if !self.has_any_conversion(&file.probe) {
                    return ChainOutcome::Never;
                }
                continue;
            }
            let candidates = action.engines_for(file.probe.kind);
            let requires = action.requires_for(file.probe.kind);
            let structural =
                self.choose_engine(candidates, requires, &file.probe, None, &everything);
            let Some(possible) = structural.chosen else {
                return ChainOutcome::Never;
            };
            if missing.is_none() {
                let live = self.choose_engine(candidates, requires, &file.probe, None, available);
                if live.chosen.is_none() {
                    missing = Some(live.missing.unwrap_or(possible));
                }
            }
        }

        match missing {
            Some(engine) => ChainOutcome::Missing(engine),
            None => ChainOutcome::Applies,
        }
    }

    fn disabled_from_choice(&self, choice: &EngineChoice) -> Option<Disabled> {
        if choice.chosen.is_some() {
            return None;
        }
        choice
            .missing
            .as_deref()
            .map(|missing| self.disabled_for_engine(missing))
    }

    fn disabled_for_engine(&self, missing: &str) -> Disabled {
        let label = self
            .engine(missing)
            .map_or_else(|| missing.to_owned(), |engine| engine.label.clone());
        Disabled {
            code: "engine_missing".to_owned(),
            reason: format!("{label} is not installed"),
            hint: Some("Install it, or see Settings › Engines.".to_owned()),
        }
    }

    /// Ranking: pinned first, then recently used, then the registry's own bias.
    /// Coverage is added afterwards, once the whole selection is known.
    fn score(base: i32, id: &str, request: &MenuRequest, runnable: bool) -> i32 {
        let pin = request
            .pins
            .iter()
            .position(|pinned| pinned == id)
            .map_or_else(
                || 0,
                |index| {
                    let index = i32::try_from(index).unwrap_or(i32::MAX);
                    PIN_BONUS.saturating_sub(index.saturating_mul(10))
                },
            );
        let usage = i32::try_from(
            request
                .usage
                .get(id)
                .copied()
                .unwrap_or(0)
                .min(USAGE_CEILING),
        )
        .unwrap_or(i32::MAX);
        base + pin + usage - if runnable { 0 } else { DISABLED_PENALTY }
    }
}

fn operator_petal(id: &str, label: &str, reference: &str, score: i32, total: usize) -> Petal {
    Petal {
        id: id.to_owned(),
        label: label.to_owned(),
        icon: "dots".to_owned(),
        kind: PetalKind::Operator,
        reference: reference.to_owned(),
        engine: None,
        options: Vec::new(),
        defaults: BTreeMap::new(),
        steps: Vec::new(),
        badge: None,
        disabled: None,
        score,
        coverage: total,
        total,
        needs_options: false,
    }
}
