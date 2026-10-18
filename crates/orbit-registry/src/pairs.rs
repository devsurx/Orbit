//! Pair expansion: turning the ~20 rules in `conversions.json` into the full
//! source -> target matrix.
//!
//! This is what satisfies "150+ pairs, generated from data, not hardcoded": the
//! rules describe families, and this module multiplies them out against the
//! formats and the engines that are actually installed.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::model::{Id, Kind};
use crate::probe::{Probe, SourceFile};
use crate::registry::{EngineChoice, Registry};

/// One runnable source -> target conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    /// Rule that generated it.
    pub rule: Id,
    /// Source format id.
    pub from: Id,
    /// Target format id.
    pub to: Id,
    /// Source family.
    pub from_kind: Kind,
    /// Target family.
    pub to_kind: Kind,
    /// Engine that will run it.
    pub choice: EngineChoice,
    /// Ranking score, higher is closer to the top of the ring.
    pub score: i32,
    /// True when the rule exposes options worth opening the panel for.
    pub needs_options: bool,
}

impl Pair {
    /// True when the pair can actually run right now.
    #[must_use]
    pub fn is_runnable(&self) -> bool {
        self.choice.chosen.is_some()
    }
}

impl Registry {
    /// Pairs generated from a concrete input.
    ///
    /// Unavailable engines are included when `include_unavailable` is set, so
    /// the menu can show a greyed-out petal explaining what to install instead
    /// of silently hiding the conversion.
    #[must_use]
    pub fn pairs_for(
        &self,
        source: &SourceFile,
        available: &BTreeSet<Id>,
        include_unavailable: bool,
    ) -> Vec<Pair> {
        let mut pairs = Vec::new();
        for rule in self.rules() {
            if !rule.from.matches(&source.probe) {
                continue;
            }
            for target in self.targets_for(rule) {
                if source.probe.format.as_deref() == Some(target.id.as_str()) {
                    continue;
                }
                let choice = self.choose_engine(
                    &rule.engines,
                    &rule.requires,
                    &source.probe,
                    Some(target),
                    available,
                );
                if choice.chosen.is_none() && !(include_unavailable && choice.missing.is_some()) {
                    continue;
                }
                pairs.push(Pair {
                    rule: rule.id.clone(),
                    from: source.probe.format.clone().unwrap_or_default(),
                    to: target.id.clone(),
                    from_kind: source.probe.kind,
                    to_kind: target.kind,
                    score: if choice.chosen.is_some() {
                        rule.rank + target.rank
                    } else {
                        rule.rank + target.rank - 10_000
                    },
                    needs_options: !rule.options.is_empty(),
                    choice,
                });
            }
        }
        pairs
    }

    /// Every runnable pair in the registry, deduplicated by source and target.
    ///
    /// Used by the registry self-check, the command palette's "can I do this?"
    /// answers and the README's pair count.
    #[must_use]
    pub fn usable_pairs(&self, available: &BTreeSet<Id>) -> Vec<Pair> {
        let mut best: std::collections::BTreeMap<(Id, Id), Pair> =
            std::collections::BTreeMap::new();
        for format in self.formats() {
            if !format.can.read {
                continue;
            }
            let probe =
                Probe::of_format(format, format.extensions.first().map_or("", String::as_str));
            let source = SourceFile::virtual_file(probe);
            for pair in self.pairs_for(&source, available, false) {
                let key = (pair.from.clone(), pair.to.clone());
                match best.get(&key) {
                    Some(existing) if existing.score >= pair.score => {}
                    _ => {
                        best.insert(key, pair);
                    }
                }
            }
        }
        let mut pairs: Vec<Pair> = best.into_values().collect();
        pairs.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.from.cmp(&b.from))
                .then_with(|| a.to.cmp(&b.to))
        });
        pairs
    }

    /// Number of distinct pairs that can run with the given engines installed.
    #[must_use]
    pub fn pair_count(&self, available: &BTreeSet<Id>) -> usize {
        self.usable_pairs(available).len()
    }

    /// Pairs reachable from one format id.
    #[must_use]
    pub fn pairs_from_format(&self, format_id: &str, available: &BTreeSet<Id>) -> Vec<Pair> {
        self.probe_format(format_id).map_or_else(Vec::new, |probe| {
            self.pairs_for(&SourceFile::virtual_file(probe), available, false)
        })
    }
}
