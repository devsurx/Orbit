//! Turning a pick in the ring into concrete jobs.
//!
//! [`plan`](crate::plan), [`output`](crate::output) and
//! [`runner`](crate::runner) deliberately know nothing about the registry: each
//! is handed a [`JobRequest`] that is already fully resolved. This module is the
//! resolver for the bundled registry — the piece that answers *which engine,
//! which extension, which word in the name, which files* — so the desktop shell
//! (and later the command-line tool) does not have to re-derive it.
//!
//! The rule that matters here is honesty about batches. The ring offers a petal
//! when it suits *at least one* dragged file, so a single pick can legitimately
//! apply to four files out of five. Files the pick cannot serve are reported in
//! [`Resolution::skipped`] with a reason, rather than being silently dropped or
//! forced through a command line that is going to fail.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use orbit_registry::{
    Action, EngineStatus, Format, Probe, Registry, Rule, SourceFile, any_selector_matches,
};

use crate::plan::{JobRequest, Options};

/// The action conversions run under; its tag names their output.
const CONVERT: &str = "convert";

/// The `output_ext` value meaning "whatever the input had".
const KEEP_EXT: &str = "same";

/// Stand-in tag, used only if the registry somehow lacks the convert action.
const FALLBACK_TAG: &str = "converted";

/// Which ring affordance was picked, mirroring `Petal::kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    /// A format petal: convert to the referenced format.
    Conversion,
    /// An editing action.
    Action,
    /// A chain or preset.
    Preset,
    /// A ring control such as Search or More; never runs a job.
    Operator,
}

impl PickKind {
    /// Parse the name the webview sends, which is `Petal::kind` in snake case.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "conversion" => Some(Self::Conversion),
            "action" => Some(Self::Action),
            "preset" => Some(Self::Preset),
            "operator" => Some(Self::Operator),
            _ => None,
        }
    }
}

/// A pick, as the ring described it.
#[derive(Debug, Clone)]
pub struct Pick {
    /// Which affordance was picked.
    pub kind: PickKind,
    /// Format id, action id, chain id or operator name.
    pub reference: String,
    /// Option values the ring had already resolved for a plain drop.
    pub defaults: Options,
}

/// A dragged file the pick does not cover.
#[derive(Debug, Clone)]
pub struct Skipped {
    /// The file that is left alone.
    pub path: PathBuf,
    /// One line for the hub, naming the reason.
    pub reason: String,
}

/// The jobs a pick produced, and what it had to leave out.
#[derive(Debug, Clone, Default)]
pub struct Resolution {
    /// One request per file the pick suits.
    pub requests: Vec<JobRequest>,
    /// Files it does not suit, each with a reason.
    pub skipped: Vec<Skipped>,
}

impl Resolution {
    /// How many dragged files the pick was considered for.
    #[must_use]
    pub fn total(&self) -> usize {
        self.requests.len() + self.skipped.len()
    }

    /// True when the pick applies to nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// The first reason a file was skipped, for a one-line summary.
    #[must_use]
    pub fn first_reason(&self) -> Option<&str> {
        self.skipped.first().map(|skip| skip.reason.as_str())
    }
}

/// Why a pick cannot be turned into jobs at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// The petal named an action the registry does not have.
    #[error("unknown action \"{0}\"")]
    UnknownAction(String),
    /// The petal named a format the registry does not have, or one with no
    /// extension to write.
    #[error("unknown format \"{0}\"")]
    UnknownFormat(String),
    /// A chain or preset, which this milestone does not run yet.
    #[error("{0} need a later milestone")]
    NotYetSupported(String),
    /// A ring control was dispatched as though it were a job.
    #[error("\"{0}\" is a ring control, not a job")]
    NotAJob(String),
}

/// Resolve a pick into one job per dragged file it suits.
///
/// One job per file rather than one for the batch: a conversion writes its
/// result beside its source, so every file needs its own output name, its own
/// progress and its own failure. Actions that genuinely combine their inputs
/// (merge) are not driven yet and say so from [`plan`](crate::plan).
///
/// # Errors
/// Fails when the pick itself makes no sense: an unknown action or format, a
/// chain that is not implemented, a control dispatched as a job. A pick that is
/// merely *partial* over a batch is not an error — check
/// [`Resolution::is_empty`] and [`Resolution::first_reason`] for that case.
pub fn resolve_pick(
    registry: &Registry,
    statuses: &BTreeMap<String, EngineStatus>,
    pick: &Pick,
    sources: &[PathBuf],
) -> Result<Resolution, ResolveError> {
    let available = orbit_registry::available_ids(statuses);
    let intent = Intent::from_pick(registry, pick)?;

    let mut resolution = Resolution::default();
    for path in sources {
        let file = probe_file(registry, path);
        match intent.request(registry, statuses, &available, pick, path, &file) {
            Ok(request) => resolution.requests.push(request),
            Err(reason) => resolution.skipped.push(Skipped {
                path: path.clone(),
                reason,
            }),
        }
    }
    Ok(resolution)
}

/// A pick with everything that can be decided before the files are considered.
enum Intent {
    /// Convert to a concrete format.
    Convert {
        /// Target format id.
        target: String,
        /// Extension the result carries.
        ext: String,
        /// Word used in the output name.
        tag: String,
    },
    /// Run an editing action.
    Edit {
        /// The action itself, copied so the registry borrow ends here.
        action: Box<Action>,
        /// Extension the result carries, or `None` to keep the input's.
        ext: Option<String>,
    },
}

/// The per-file facts a request is built from.
///
/// Candidates and requirements are per file, not per pick: a batch can mix
/// families, and "Compress" means `ImageMagick` for an image but `FFmpeg` for a
/// video.
struct Spec<'a> {
    /// Action id the request runs under.
    action: String,
    /// Word used in the output name.
    tag: String,
    /// Extension the result carries.
    target_ext: String,
    /// Engines that could carry it out, best first.
    candidates: Vec<String>,
    /// Capability tokens the engine must provide.
    requires: Vec<String>,
    /// Format being written, when it is a concrete one.
    target: Option<&'a Format>,
}

impl Intent {
    /// Read the pick, failing on anything structurally wrong with it.
    fn from_pick(registry: &Registry, pick: &Pick) -> Result<Self, ResolveError> {
        match pick.kind {
            PickKind::Operator => Err(ResolveError::NotAJob(pick.reference.clone())),
            PickKind::Preset => Err(ResolveError::NotYetSupported(format!(
                "chains and presets ({})",
                pick.reference
            ))),
            PickKind::Conversion => {
                let target = registry
                    .format(&pick.reference)
                    .ok_or_else(|| ResolveError::UnknownFormat(pick.reference.clone()))?;
                let ext = target.extensions.first().cloned().ok_or_else(|| {
                    ResolveError::UnknownFormat(format!("{} has no extension", pick.reference))
                })?;
                let tag = registry
                    .action(CONVERT)
                    .map_or_else(|| FALLBACK_TAG.to_owned(), |action| action.tag.clone());
                Ok(Self::Convert {
                    target: target.id.clone(),
                    ext,
                    tag,
                })
            }
            PickKind::Action => {
                let action = registry
                    .action(&pick.reference)
                    .ok_or_else(|| ResolveError::UnknownAction(pick.reference.clone()))?;
                let ext = match action.output_ext.as_str() {
                    KEEP_EXT => None,
                    concrete => Some(concrete.to_owned()),
                };
                Ok(Self::Edit {
                    action: Box::new(action.clone()),
                    ext,
                })
            }
        }
    }

    /// Work out what this file would need.
    ///
    /// The `Err` string is a reason to leave *this* file alone; the pick may
    /// still be right for the others in the batch.
    fn spec<'a>(&'a self, registry: &'a Registry, file: &SourceFile) -> Result<Spec<'a>, String> {
        let probe = &file.probe;
        match self {
            Self::Convert { target, ext, tag } => {
                let rule = rule_for(registry, probe, target).ok_or_else(|| {
                    format!(
                        "{} cannot become {}",
                        describe(probe),
                        target.to_uppercase()
                    )
                })?;
                Ok(Spec {
                    action: CONVERT.to_owned(),
                    tag: tag.clone(),
                    target_ext: ext.clone(),
                    candidates: rule.engines.clone(),
                    requires: rule.requires.clone(),
                    target: registry.format(target),
                })
            }
            Self::Edit { action, ext } => {
                if !any_selector_matches(&action.applies_to, probe) {
                    return Err(format!(
                        "{} does not apply to {}",
                        action.label,
                        describe(probe)
                    ));
                }
                let target_ext = match ext {
                    Some(concrete) => concrete.clone(),
                    None if probe.ext.is_empty() => {
                        return Err(format!("{} has no extension to keep", describe(probe)));
                    }
                    None => probe.ext.clone(),
                };
                Ok(Spec {
                    action: action.id.clone(),
                    tag: action.tag.clone(),
                    target_ext,
                    candidates: action.engines_for(probe.kind).to_vec(),
                    requires: action.requires_for(probe.kind).to_vec(),
                    // Actions keep their input's format, so there is no target
                    // format to check writers against — the ring does the same.
                    target: None,
                })
            }
        }
    }

    /// Build the request for one file, or say why it does not apply.
    fn request(
        &self,
        registry: &Registry,
        statuses: &BTreeMap<String, EngineStatus>,
        available: &BTreeSet<String>,
        pick: &Pick,
        path: &Path,
        file: &SourceFile,
    ) -> Result<JobRequest, String> {
        let probe = &file.probe;
        let spec = self.spec(registry, file)?;

        let choice = registry.choose_engine(
            &spec.candidates,
            &spec.requires,
            probe,
            spec.target,
            available,
        );
        let Some(engine) = choice.chosen else {
            return Err(match choice.missing {
                Some(missing) => format!("needs {missing}, which is not installed"),
                None => format!("no installed engine can do that to {}", describe(probe)),
            });
        };
        // Engines with no binary are compiled into Orbit; the ones this
        // milestone can drive all have one.
        let Some(program) = statuses
            .get(&engine)
            .and_then(|status| status.binary.clone())
        else {
            return Err(format!("{engine} cannot be run here yet"));
        };

        Ok(JobRequest {
            engine,
            program,
            kind: probe.kind,
            action: spec.action,
            target_ext: spec.target_ext,
            tag: spec.tag,
            sources: vec![path.to_path_buf()],
            options: pick.defaults.clone(),
            output_dir: None,
        })
    }
}

/// A short human name for a probed file: "PNG", or the format when the
/// extension told us nothing.
fn describe(probe: &Probe) -> String {
    if probe.ext.is_empty() {
        probe
            .format
            .clone()
            .unwrap_or_else(|| "that file".to_owned())
    } else {
        probe.ext.to_uppercase()
    }
}

/// The rule the ring used to offer `target` for this input.
///
/// Mirrors `Registry::pairs_for`: a rule counts when its source selector matches
/// and the target is one of its concrete targets. Ties go to the higher ranking
/// rule, which is the one the score would have floated to the top.
fn rule_for<'a>(registry: &'a Registry, probe: &Probe, target: &str) -> Option<&'a Rule> {
    registry
        .rules()
        .iter()
        .filter(|rule| rule.from.matches(probe))
        .filter(|rule| {
            registry
                .targets_for(rule)
                .iter()
                .any(|format| format.id == target)
        })
        .max_by_key(|rule| rule.rank)
}

/// Classify a path, carrying its size when it is a real file.
fn probe_file(registry: &Registry, path: &Path) -> SourceFile {
    let size = std::fs::metadata(path).map_or(0, |meta| meta.len());
    registry.classify(path, size)
}
