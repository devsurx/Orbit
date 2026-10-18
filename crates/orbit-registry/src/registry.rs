//! The loaded, indexed and validated registry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::engines::{EngineStatus, available_ids, detect_all};
use crate::load::RegistryDocs;
use crate::model::{Action, Category, Chain, Engine, Format, Id, Kind, OptionSpec, Rule, Target};
use crate::probe::{Probe, SourceFile, extension_of};
use crate::validate;

/// Something went wrong while loading or validating the registry.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// A document could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// File that failed.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// A document could not be parsed.
    #[error("cannot parse {path}: {source}")]
    Parse {
        /// File that failed.
        path: PathBuf,
        /// Underlying error.
        source: serde_json::Error,
    },
    /// The documents parsed but contradict each other.
    #[error("registry is not usable:\n  - {}", .0.join("\n  - "))]
    Invalid(Vec<String>),
}

/// Which engine should run a step, or which one is missing.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct EngineChoice {
    /// First candidate that both fits and is installed.
    pub chosen: Option<Id>,
    /// First candidate that fits but is not installed, for the "why grey" tooltip.
    pub missing: Option<Id>,
}

/// Everything Orbit can do, loaded from `registry/*.json`.
#[derive(Debug, Clone)]
pub struct Registry {
    formats: Vec<Format>,
    engines: Vec<Engine>,
    rules: Vec<Rule>,
    actions: Vec<Action>,
    chains: Vec<Chain>,
    categories: Vec<Category>,
    options: BTreeMap<Id, OptionSpec>,
    format_index: BTreeMap<Id, usize>,
    engine_index: BTreeMap<Id, usize>,
    action_index: BTreeMap<Id, usize>,
    chain_index: BTreeMap<Id, usize>,
    category_index: BTreeMap<Id, usize>,
    ext_index: BTreeMap<String, usize>,
}

impl Registry {
    /// The registry compiled into Orbit: the copy the app ships with.
    ///
    /// # Errors
    /// Only fails if the embedded documents are broken, which the test suite
    /// prevents.
    pub fn bundled() -> Result<Self, RegistryError> {
        crate::load::bundled()
    }

    /// Load and validate the registry from a directory.
    ///
    /// # Errors
    /// Reports unreadable files, parse errors and every validation problem.
    pub fn from_dir(dir: &Path) -> Result<Self, RegistryError> {
        crate::load::load_dir(dir)
    }

    /// Build a registry from parsed documents.
    ///
    /// # Errors
    /// Returns [`RegistryError::Invalid`] listing every contradiction found, so
    /// a broken registry edit is one clear message rather than a bad menu.
    pub fn new(docs: RegistryDocs) -> Result<Self, RegistryError> {
        let (registry, mut problems) = Self::build(docs);
        problems.extend(validate::validate(&registry));
        if problems.is_empty() {
            Ok(registry)
        } else {
            Err(RegistryError::Invalid(problems))
        }
    }

    /// Index the documents without validating, collecting structural problems.
    fn build(docs: RegistryDocs) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut format_index = BTreeMap::new();
        let mut ext_index = BTreeMap::new();

        for (position, format) in docs.formats.iter().enumerate() {
            if format_index.insert(format.id.clone(), position).is_some() {
                problems.push(format!("duplicate format id: {}", format.id));
                continue;
            }
            for ext in &format.extensions {
                let ext = ext.to_ascii_lowercase();
                if let Some(previous) = ext_index.insert(ext.clone(), position) {
                    problems.push(format!(
                        "extension .{ext} is claimed by both {} and {}",
                        docs.formats[previous].id, format.id
                    ));
                }
            }
        }

        let mut engine_index = BTreeMap::new();
        for (position, engine) in docs.engines.iter().enumerate() {
            if engine_index.insert(engine.id.clone(), position).is_some() {
                problems.push(format!("duplicate engine id: {}", engine.id));
            }
        }

        let mut action_index = BTreeMap::new();
        for (position, action) in docs.actions.iter().enumerate() {
            if action_index.insert(action.id.clone(), position).is_some() {
                problems.push(format!("duplicate action id: {}", action.id));
            }
        }

        let mut chain_index = BTreeMap::new();
        for (position, chain) in docs.chains.iter().enumerate() {
            if chain_index.insert(chain.id.clone(), position).is_some() {
                problems.push(format!("duplicate chain id: {}", chain.id));
            }
        }

        let mut category_index = BTreeMap::new();
        for (position, category) in docs.categories.iter().enumerate() {
            if category_index
                .insert(category.id.clone(), position)
                .is_some()
            {
                problems.push(format!("duplicate category id: {}", category.id));
            }
        }

        let mut options = BTreeMap::new();
        for (key, spec) in docs.options {
            if spec.key != key {
                problems.push(format!(
                    "option {key} declares key {}; the catalog key is what rules reference",
                    spec.key
                ));
            }
            options.insert(key, spec);
        }

        let mut categories = docs.categories;
        categories.sort_by_key(|category| category.order);

        let registry = Self {
            formats: docs.formats,
            engines: docs.engines,
            rules: docs.conversions,
            actions: docs.actions,
            chains: docs.chains,
            categories,
            options,
            format_index,
            engine_index,
            action_index,
            chain_index,
            category_index,
            ext_index,
        };
        (registry, problems)
    }

    /// Every format, in document order.
    #[must_use]
    pub fn formats(&self) -> &[Format] {
        &self.formats
    }

    /// Every engine, in document order.
    #[must_use]
    pub fn engines(&self) -> &[Engine] {
        &self.engines
    }

    /// Every conversion rule.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Every editing action.
    #[must_use]
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// Every chain and preset.
    #[must_use]
    pub fn chains(&self) -> &[Chain] {
        &self.chains
    }

    /// Every radial category, in ring order.
    #[must_use]
    pub fn categories(&self) -> &[Category] {
        &self.categories
    }

    /// Look up a format.
    #[must_use]
    pub fn format(&self, id: &str) -> Option<&Format> {
        self.format_index.get(id).and_then(|i| self.formats.get(*i))
    }

    /// Look up an engine.
    #[must_use]
    pub fn engine(&self, id: &str) -> Option<&Engine> {
        self.engine_index.get(id).and_then(|i| self.engines.get(*i))
    }

    /// Look up an action.
    #[must_use]
    pub fn action(&self, id: &str) -> Option<&Action> {
        self.action_index.get(id).and_then(|i| self.actions.get(*i))
    }

    /// Look up a chain or preset.
    #[must_use]
    pub fn chain(&self, id: &str) -> Option<&Chain> {
        self.chain_index.get(id).and_then(|i| self.chains.get(*i))
    }

    /// Look up a radial category.
    #[must_use]
    pub fn category(&self, id: &str) -> Option<&Category> {
        self.category_index
            .get(id)
            .and_then(|i| self.categories.get(*i))
    }

    /// Look up an option spec from the shared catalog.
    #[must_use]
    pub fn option(&self, key: &str) -> Option<&OptionSpec> {
        self.options.get(key)
    }

    /// Every option spec, for the settings and palette UI.
    #[must_use]
    pub fn options(&self) -> &BTreeMap<Id, OptionSpec> {
        &self.options
    }

    /// Writable formats of one family, best-ranked first.
    #[must_use]
    pub fn writable_targets(&self, kind: Kind) -> Vec<&Format> {
        let mut targets: Vec<&Format> = self
            .formats
            .iter()
            .filter(|format| format.can.write && format.kind == kind)
            .collect();
        targets.sort_by(|a, b| b.rank.cmp(&a.rank).then_with(|| a.id.cmp(&b.id)));
        targets
    }

    /// Resolve a rule's target list, honouring its `only` allow-list.
    #[must_use]
    pub fn targets_for(&self, rule: &Rule) -> Vec<&Format> {
        let candidates: Vec<&Format> = match &rule.to {
            Target::Format(id) => self.format(id).into_iter().collect(),
            Target::Writable(kind) => self.writable_targets(*kind),
        };
        candidates
            .into_iter()
            .filter(|format| {
                format.can.write
                    && (rule.only.is_empty() || rule.only.iter().any(|id| id == &format.id))
            })
            .collect()
    }

    /// Probe a path: extension first, magic bytes once M2 lands.
    #[must_use]
    pub fn classify(&self, path: &Path, size_bytes: u64) -> SourceFile {
        let ext = extension_of(path);
        let probe = self
            .ext_index
            .get(&ext)
            .and_then(|index| self.formats.get(*index))
            .map_or_else(
                || Probe::unknown(&ext),
                |format| Probe::of_format(format, &ext),
            );
        SourceFile {
            path: Some(path.to_path_buf()),
            size_bytes,
            probe,
        }
    }

    /// Probe a bare extension, as if a file with that extension was dragged.
    #[must_use]
    pub fn probe_ext(&self, ext: &str) -> Probe {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        self.ext_index
            .get(&ext)
            .and_then(|index| self.formats.get(*index))
            .map_or_else(
                || Probe::unknown(&ext),
                |format| Probe::of_format(format, &ext),
            )
    }

    /// Probe a known format id.
    #[must_use]
    pub fn probe_format(&self, id: &str) -> Option<Probe> {
        self.format(id).map(|format| {
            let ext = format.extensions.first().cloned().unwrap_or_default();
            Probe::of_format(format, &ext)
        })
    }

    /// Detect which engines are usable on this machine.
    #[must_use]
    pub fn detect_engines(
        &self,
        platform: &str,
        path_var: Option<&str>,
    ) -> BTreeMap<Id, EngineStatus> {
        detect_all(&self.engines, platform, path_var)
    }

    /// Engine ids that are installed, from a status map.
    #[must_use]
    pub fn available_engines(statuses: &BTreeMap<Id, EngineStatus>) -> BTreeSet<Id> {
        available_ids(statuses)
    }

    /// Every engine id, used when availability should be ignored (tests,
    /// previews and the registry self-check).
    #[must_use]
    pub fn all_engine_ids(&self) -> BTreeSet<Id> {
        self.engines
            .iter()
            .map(|engine| engine.id.clone())
            .collect()
    }

    /// Can this engine run `requires` on `source`, producing `target`?
    ///
    /// A candidate must declare every capability token, be able to decode the
    /// source format and, when a target is given, be able to encode it.
    #[must_use]
    pub fn engine_can_serve(
        &self,
        engine: &Engine,
        requires: &[String],
        source: &Probe,
        target: Option<&Format>,
    ) -> bool {
        if !engine.provides_all(requires) {
            return false;
        }
        if let Some(source_format) = source.format.as_deref().and_then(|id| self.format(id)) {
            if !source_format.readers.is_empty()
                && !source_format.readers.iter().any(|id| id == &engine.id)
            {
                return false;
            }
        }
        if let Some(target_format) = target {
            if !target_format.writers.iter().any(|id| id == &engine.id) {
                return false;
            }
        }
        true
    }

    /// Pick the best engine from `candidates`, reporting the first capable but
    /// uninstalled one so the UI can explain the gap.
    #[must_use]
    pub fn choose_engine(
        &self,
        candidates: &[Id],
        requires: &[String],
        source: &Probe,
        target: Option<&Format>,
        available: &BTreeSet<Id>,
    ) -> EngineChoice {
        let mut choice = EngineChoice::default();
        for candidate in candidates {
            let Some(engine) = self.engine(candidate) else {
                continue;
            };
            if !self.engine_can_serve(engine, requires, source, target) {
                continue;
            }
            if available.contains(candidate) {
                choice.chosen = Some(candidate.clone());
                return choice;
            }
            if choice.missing.is_none() {
                choice.missing = Some(candidate.clone());
            }
        }
        choice
    }

    /// True when any conversion rule accepts this input, used by the generic
    /// Convert action to decide whether it is applicable at all.
    #[must_use]
    pub fn has_any_conversion(&self, source: &Probe) -> bool {
        self.rules
            .iter()
            .any(|rule| rule.from.matches(source) && !self.targets_for(rule).is_empty())
    }
}
