//! Registry self-check.
//!
//! The registry is data, so the failure mode is a typo rather than a compile
//! error. This module turns those typos into a single readable list, and the
//! test suite runs it on every build.

use std::collections::BTreeSet;

use crate::model::{Id, Kind, OptionKind, Rule, Target};
use crate::registry::Registry;

/// Option-list entry meaning "whatever the resolved rule exposes".
const WILDCARD_OPTIONS: &str = "*";

/// Every problem found, empty when the registry is sound.
#[must_use]
pub fn validate(registry: &Registry) -> Vec<String> {
    let mut problems = Vec::new();
    validate_formats(registry, &mut problems);
    validate_rules(registry, &mut problems);
    validate_actions(registry, &mut problems);
    validate_chains(registry, &mut problems);
    validate_options(registry, &mut problems);
    validate_categories(registry, &mut problems);
    problems
}

fn validate_formats(registry: &Registry, problems: &mut Vec<String>) {
    for format in registry.formats() {
        if format.extensions.is_empty() {
            problems.push(format!("format {} declares no extensions", format.id));
        }
        if format.can.read && format.readers.is_empty() {
            problems.push(format!(
                "format {} is readable but names no readers",
                format.id
            ));
        }
        if format.can.write && format.writers.is_empty() {
            problems.push(format!(
                "format {} is writable but names no writers",
                format.id
            ));
        }
        if !format.can.read && !format.can.write {
            problems.push(format!(
                "format {} is neither readable nor writable",
                format.id
            ));
        }
        for engine in format.readers.iter().chain(format.writers.iter()) {
            if registry.engine(engine).is_none() {
                problems.push(format!(
                    "format {} references unknown engine {engine}",
                    format.id
                ));
            }
        }
    }
}

fn validate_rules(registry: &Registry, problems: &mut Vec<String>) {
    let all_engines = registry.all_engine_ids();
    for rule in registry.rules() {
        if rule.engines.is_empty() {
            problems.push(format!(
                "rule {} names no engines, so it can never run",
                rule.id
            ));
        }
        for engine in &rule.engines {
            if registry.engine(engine).is_none() {
                problems.push(format!(
                    "rule {} references unknown engine {engine}",
                    rule.id
                ));
            }
        }
        check_rule_target(registry, rule, problems);

        // A token that no listed engine declares means the rule is dead on arrival.
        let declared: BTreeSet<&str> = rule
            .engines
            .iter()
            .filter_map(|id| registry.engine(id))
            .flat_map(|engine| engine.provides.iter().map(String::as_str))
            .collect();
        for token in &rule.requires {
            if !declared.contains(token.as_str()) {
                problems.push(format!(
                    "rule {} requires {token}, which none of {:?} declare",
                    rule.id, rule.engines
                ));
            }
        }

        // The real check: does the rule produce at least one runnable pair?
        if !rule_can_fire(registry, rule, &all_engines) {
            problems.push(format!(
                "rule {} can never run: no source and target combination is satisfiable",
                rule.id
            ));
        }

        for key in &rule.options {
            if registry.option(key).is_none() {
                problems.push(format!("rule {} references unknown option {key}", rule.id));
            }
        }
    }
}

/// Check that a rule names a real target, and that every format it restricts
/// itself to is both the right family and writable.
fn check_rule_target(registry: &Registry, rule: &Rule, problems: &mut Vec<String>) {
    match &rule.to {
        Target::Format(id) => match registry.format(id) {
            None => problems.push(format!("rule {} targets unknown format {id}", rule.id)),
            Some(format) if !format.can.write => problems.push(format!(
                "rule {} targets {id}, which is not writable",
                rule.id
            )),
            Some(_) => {}
        },
        Target::Writable(kind) => {
            for id in &rule.only {
                let Some(format) = registry.format(id) else {
                    problems.push(format!("rule {} only lists unknown format {id}", rule.id));
                    continue;
                };
                if format.kind != *kind {
                    problems.push(format!(
                        "rule {} only lists {id}, which is {} not {kind}",
                        rule.id, format.kind
                    ));
                }
                if !format.can.write {
                    problems.push(format!(
                        "rule {} only lists {id}, which is not writable",
                        rule.id
                    ));
                }
            }
        }
    }
}

/// Whether the rule yields at least one satisfiable source/target pair.
fn rule_can_fire(registry: &Registry, rule: &Rule, all_engines: &BTreeSet<Id>) -> bool {
    for format in registry.formats() {
        if !format.can.read {
            continue;
        }
        let probe = crate::probe::Probe::of_format(
            format,
            format.extensions.first().map_or("", String::as_str),
        );
        if !rule.from.matches(&probe) {
            continue;
        }
        for target in registry.targets_for(rule) {
            if target.id == format.id {
                continue;
            }
            if registry
                .choose_engine(
                    &rule.engines,
                    &rule.requires,
                    &probe,
                    Some(target),
                    all_engines,
                )
                .chosen
                .is_some()
            {
                return true;
            }
        }
    }
    false
}

fn validate_actions(registry: &Registry, problems: &mut Vec<String>) {
    for action in registry.actions() {
        if registry.category(&action.category).is_none() {
            problems.push(format!(
                "action {} is in unknown category {}",
                action.id, action.category
            ));
        }
        if action.applies_to.is_empty() && action.resolves_via.is_none() {
            problems.push(format!(
                "action {} applies to nothing and does not resolve via conversions",
                action.id
            ));
        }
        if action.requires_files == 0 {
            problems.push(format!("action {} requires zero files", action.id));
        }
        if let Some(resolver) = &action.resolves_via {
            if resolver != "conversions" {
                problems.push(format!(
                    "action {} resolves via unknown mechanism {resolver}",
                    action.id
                ));
            }
        }
        for key in &action.options {
            // "*" means "whatever the resolved rule exposes" and is only valid
            // for an action that resolves through the conversion rules.
            if key == WILDCARD_OPTIONS && action.resolves_via.is_none() {
                problems.push(format!(
                    "action {} uses the option wildcard but does not resolve via conversions",
                    action.id
                ));
                continue;
            }
            if key != WILDCARD_OPTIONS && registry.option(key).is_none() {
                problems.push(format!(
                    "action {} references unknown option {key}",
                    action.id
                ));
            }
        }
        for (kind, engines) in &action.engines {
            for engine in engines {
                if registry.engine(engine).is_none() {
                    problems.push(format!(
                        "action {} references unknown engine {engine}",
                        action.id
                    ));
                }
            }
            let requires = action.requires_for(*kind);
            if requires.is_empty() {
                continue;
            }
            let served = engines.iter().any(|id| {
                registry
                    .engine(id)
                    .is_some_and(|engine| engine.provides_all(requires))
            });
            if !served {
                problems.push(format!(
                    "action {} requires {:?} for {kind}, which none of {engines:?} declare",
                    action.id, requires
                ));
            }
        }
        for kind in action.requires.keys() {
            if !action.engines.contains_key(kind) {
                problems.push(format!(
                    "action {} requires capabilities for {kind} but names no engine for it",
                    action.id
                ));
            }
        }
    }
}

fn validate_chains(registry: &Registry, problems: &mut Vec<String>) {
    for chain in registry.chains() {
        if registry.category(&chain.category).is_none() {
            problems.push(format!(
                "chain {} is in unknown category {}",
                chain.id, chain.category
            ));
        }
        if chain.steps.is_empty() {
            problems.push(format!("chain {} has no steps", chain.id));
        }
        for step in &chain.steps {
            let Some(action) = registry.action(&step.action) else {
                problems.push(format!(
                    "chain {} step references unknown action {}",
                    chain.id, step.action
                ));
                continue;
            };
            let wildcard = action.options.iter().any(|key| key == WILDCARD_OPTIONS);
            for key in step.options.keys() {
                if registry.option(key).is_none() {
                    problems.push(format!(
                        "chain {} step {} sets unknown option {key}",
                        chain.id, step.action
                    ));
                    continue;
                }
                if !wildcard
                    && !action.options.is_empty()
                    && !action.options.iter().any(|known| known == key)
                {
                    problems.push(format!(
                        "chain {} step {} sets option {key}, which {} does not expose",
                        chain.id, step.action, action.id
                    ));
                }
            }
        }

        // Every declared input family must be able to run every step.
        for selector in &chain.applies_to {
            let Some(probe) = representative_probe(registry, selector) else {
                continue;
            };
            for step in &chain.steps {
                let Some(action) = registry.action(&step.action) else {
                    continue;
                };
                let applies = action.applies_to.is_empty()
                    || action.applies_to.iter().any(|inner| inner.matches(&probe));
                if !applies {
                    problems.push(format!(
                        "chain {} runs {} on {}, which does not apply to it",
                        chain.id, action.id, probe.ext
                    ));
                    continue;
                }
                if action.resolves_via.as_deref() == Some("conversions") {
                    if !registry.has_any_conversion(&probe) {
                        problems.push(format!(
                            "chain {} step {} has no conversion rule for {}",
                            chain.id, action.id, probe.ext
                        ));
                    }
                    continue;
                }
                let choice = registry.choose_engine(
                    action.engines_for(probe.kind),
                    action.requires_for(probe.kind),
                    &probe,
                    None,
                    &registry.all_engine_ids(),
                );
                if choice.chosen.is_none() {
                    problems.push(format!(
                        "chain {} step {} has no engine for {} ({})",
                        chain.id, action.id, probe.ext, probe.kind
                    ));
                }
            }
        }
    }
}

/// A probe from the first readable format that satisfies `selector`.
fn representative_probe(
    registry: &Registry,
    selector: &crate::model::Selector,
) -> Option<crate::probe::Probe> {
    registry
        .formats()
        .iter()
        .filter(|format| format.can.read)
        .find_map(|format| {
            let probe = crate::probe::Probe::of_format(
                format,
                format.extensions.first().map_or("", String::as_str),
            );
            selector.matches(&probe).then_some(probe)
        })
}

fn validate_options(registry: &Registry, problems: &mut Vec<String>) {
    for (key, spec) in registry.options() {
        if matches!(spec.kind, OptionKind::Enum | OptionKind::MultiEnum) && spec.values.is_empty() {
            problems.push(format!("option {key} is an enum with no values"));
        }
        if let (Some(min), Some(max)) = (spec.min, spec.max) {
            if min >= max {
                problems.push(format!("option {key} has min {min} >= max {max}"));
            }
        }
        for label_key in spec.labels.keys() {
            let as_value = serde_json::Value::String(label_key.clone());
            let as_number = label_key
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number);
            if !spec.values.contains(&as_value)
                && !as_number.is_some_and(|n| spec.values.contains(&n))
            {
                problems.push(format!(
                    "option {key} labels {label_key}, which is not one of its values"
                ));
            }
        }
        if let Some(search) = &spec.search {
            if search.params.is_empty() {
                problems.push(format!("option {key} has a size search with no parameters"));
            }
            for param in &search.params {
                if registry.option(param).is_none() {
                    problems.push(format!(
                        "option {key} size search varies unknown option {param}"
                    ));
                }
            }
        }
    }
}

fn validate_categories(registry: &Registry, problems: &mut Vec<String>) {
    let used: BTreeSet<&str> = registry
        .actions()
        .iter()
        .map(|action| action.category.as_str())
        .chain(
            registry
                .chains()
                .iter()
                .map(|chain| chain.category.as_str()),
        )
        .collect();

    for category in registry.categories() {
        if !used.contains(category.id.as_str()) {
            problems.push(format!(
                "category {} has no actions or chains, so it would render empty",
                category.id
            ));
        }
    }

    for kind in Kind::ALL {
        if kind == Kind::Other {
            continue;
        }
        if registry.formats().iter().all(|format| format.kind != kind) {
            problems.push(format!("no format declares kind {kind}"));
        }
    }
}
