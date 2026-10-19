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
use std::time::SystemTime;

use orbit_registry::{
    Action, EngineStatus, Format, Kind, Probe, Registry, Rule, SourceFile, any_selector_matches,
};
use serde_json::Value;

use crate::plan::{JobRequest, Options};

/// The action conversions run under; its tag names their output.
const CONVERT: &str = "convert";

/// The `output_ext` value meaning "whatever the input had".
const KEEP_EXT: &str = "same";

/// The option that says what a merge produces, and its values.
const MERGE_OUTPUT: &str = "merge_output";
const MERGE_ORDER: &str = "merge_order";

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

/// Resolve a pick into the jobs it describes.
///
/// Most picks give one job per dragged file: a conversion writes its result
/// beside its source, so every file needs its own output name, its own progress
/// and its own failure. A *combining* action is the exception — a merge is one
/// job with many sources, because its whole point is that the result is one
/// file.
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

    Ok(if intent.combines() {
        intent.combine(registry, statuses, &available, pick, sources)
    } else {
        intent.each(registry, statuses, &available, pick, sources)
    })
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
        /// How the result's extension is decided.
        ext: ExtRule,
    },
}

/// How an action's output extension is decided.
enum ExtRule {
    /// Fixed by the registry, such as `pages` always producing a PDF.
    Fixed(String),
    /// Whatever the input had.
    Keep,
    /// Chosen by an option and the family, which is what a merge does: the same
    /// action writes a PDF, a GIF, an MP4 or an MP3 depending on its inputs.
    Merge,
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
    /// What the file is, so the engine choice can check readers and writers.
    probe: Probe,
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
                let ext = if action.combines {
                    ExtRule::Merge
                } else {
                    match action.output_ext.as_str() {
                        KEEP_EXT => ExtRule::Keep,
                        concrete => ExtRule::Fixed(concrete.to_owned()),
                    }
                };
                Ok(Self::Edit {
                    action: Box::new(action.clone()),
                    ext,
                })
            }
        }
    }

    /// True when the action folds its inputs into one result.
    fn combines(&self) -> bool {
        match self {
            Self::Edit { action, .. } => action.combines,
            Self::Convert { .. } => false,
        }
    }

    /// One job per file the pick suits.
    fn each(
        &self,
        registry: &Registry,
        statuses: &BTreeMap<String, EngineStatus>,
        available: &BTreeSet<String>,
        pick: &Pick,
        sources: &[PathBuf],
    ) -> Resolution {
        let mut resolution = Resolution::default();
        for path in sources {
            let file = probe_file(registry, path);
            let assembled = self.spec(registry, &file, &pick.defaults).and_then(|spec| {
                assemble(
                    registry,
                    statuses,
                    available,
                    &spec,
                    vec![path.clone()],
                    pick,
                )
            });
            match assembled {
                Ok(request) => resolution.requests.push(request),
                Err(reason) => resolution.skipped.push(Skipped {
                    path: path.clone(),
                    reason,
                }),
            }
        }
        resolution
    }

    /// One job covering every file the pick suits.
    ///
    /// Files of another family are left out rather than forced in: merging a
    /// video into a batch of photographs is not a thing, and saying so is more
    /// useful than a command line that dies inside the engine.
    fn combine(
        &self,
        registry: &Registry,
        statuses: &BTreeMap<String, EngineStatus>,
        available: &BTreeSet<String>,
        pick: &Pick,
        sources: &[PathBuf],
    ) -> Resolution {
        let mut resolution = Resolution::default();
        let mut usable: Vec<(PathBuf, Kind)> = Vec::new();
        for path in sources {
            let file = probe_file(registry, path);
            match self.spec(registry, &file, &pick.defaults) {
                Ok(_) => usable.push((path.clone(), file.probe.kind)),
                Err(reason) => resolution.skipped.push(Skipped {
                    path: path.clone(),
                    reason,
                }),
            }
        }

        let Some(kind) = usable.first().map(|(_, kind)| *kind) else {
            return resolution;
        };
        let mut paths: Vec<PathBuf> = Vec::new();
        for (path, file_kind) in usable {
            if file_kind == kind {
                paths.push(path);
            } else {
                resolution.skipped.push(Skipped {
                    path,
                    reason: format!(
                        "one {} can only combine {} files",
                        action_label(self),
                        family(kind)
                    ),
                });
            }
        }
        order_sources(
            &mut paths,
            option_text(&pick.defaults, MERGE_ORDER).as_deref(),
        );

        let wanted = self.minimum().unwrap_or(2);
        if paths.len() < wanted {
            if let Some(path) = paths.into_iter().next() {
                resolution.skipped.push(Skipped {
                    path,
                    reason: format!("{} needs at least {wanted} files", action_label(self)),
                });
            }
            return resolution;
        }

        let file = probe_file(registry, &paths[0]);
        let assembled = self
            .spec(registry, &file, &pick.defaults)
            .and_then(|spec| assemble(registry, statuses, available, &spec, paths, pick));
        match assembled {
            Ok(request) => resolution.requests.push(request),
            Err(reason) => {
                // Every file was share of the same failure: report the first so
                // the hub has something to say, and leave the rest alone.
                resolution
                    .skipped
                    .extend(sources.iter().map(|path| Skipped {
                        path: path.clone(),
                        reason: reason.clone(),
                    }));
            }
        }
        resolution
    }

    /// How many files the action needs before it can run at all.
    fn minimum(&self) -> Option<usize> {
        match self {
            Self::Edit { action, .. } => Some(action.requires_files as usize),
            Self::Convert { .. } => None,
        }
    }

    /// Work out what this file would need.
    ///
    /// The `Err` string is a reason to leave *this* file alone; the pick may
    /// still be right for the others in the batch.
    fn spec<'a>(
        &'a self,
        registry: &'a Registry,
        file: &SourceFile,
        options: &Options,
    ) -> Result<Spec<'a>, String> {
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
                    probe: probe.clone(),
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
                    ExtRule::Fixed(concrete) => concrete.clone(),
                    ExtRule::Merge => {
                        merge_target(probe.kind, option_text(options, MERGE_OUTPUT).as_deref())
                    }
                    ExtRule::Keep if probe.ext.is_empty() => {
                        return Err(format!("{} has no extension to keep", describe(probe)));
                    }
                    ExtRule::Keep => probe.ext.clone(),
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
                    probe: probe.clone(),
                })
            }
        }
    }
}

/// Pick the engine for a spec and build the request.
///
/// One place for both shapes of job, so a merged request is filtered and
/// reported exactly like a single-file one.
fn assemble(
    registry: &Registry,
    statuses: &BTreeMap<String, EngineStatus>,
    available: &BTreeSet<String>,
    spec: &Spec<'_>,
    sources: Vec<PathBuf>,
    pick: &Pick,
) -> Result<JobRequest, String> {
    let choice = registry.choose_engine(
        &spec.candidates,
        &spec.requires,
        &spec.probe,
        spec.target,
        available,
    );
    let Some(engine) = choice.chosen else {
        return Err(match choice.missing {
            Some(missing) => format!("needs {missing}, which is not installed"),
            None => format!(
                "no installed engine can do that to {}",
                describe(&spec.probe)
            ),
        });
    };
    // Engines with no binary are compiled into Orbit.
    let program = statuses
        .get(&engine)
        .and_then(|status| status.binary.clone())
        .unwrap_or_else(|| PathBuf::from(&engine));

    Ok(JobRequest {
        engine,
        program,
        kind: spec.probe.kind,
        action: spec.action.clone(),
        target_ext: spec.target_ext.clone(),
        tag: spec.tag.clone(),
        sources,
        options: pick.defaults.clone(),
        output_dir: None,
    })
}

/// What a merge produces, given the family of its inputs and the output chosen.
///
/// The option's registry default is a PDF, which means nothing for a batch of
/// audio files. An output that cannot apply to the family therefore falls back
/// to that family's own default instead of failing the run: the panel offers
/// every value for every family, because the registry has one option catalog.
fn merge_target(kind: Kind, chosen: Option<&str>) -> String {
    let ext = match kind {
        Kind::Video => "mp4",
        Kind::Audio => "mp3",
        // A GIF is a still-image slideshow: the only other result a batch of
        // images can sensibly become.
        Kind::Image if chosen == Some("gif") => "gif",
        Kind::Image | Kind::Pdf | Kind::Text | Kind::Data | Kind::Document | Kind::Other => "pdf",
    };
    ext.to_owned()
}

/// Sort a merge's inputs the way the option asks.
///
/// The order is the one thing about a merge the user cannot see coming, so
/// "as dragged" is the default and the other two are explicit.
fn order_sources(paths: &mut [PathBuf], order: Option<&str>) {
    match order {
        Some("name") => paths.sort_by(|a, b| a.file_name().cmp(&b.file_name())),
        Some("date") => paths.sort_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        }),
        _ => {}
    }
}

/// Read an option that holds text.
fn option_text(options: &Options, key: &str) -> Option<String> {
    match options.get(key)? {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// The name of the action a pick refers to, for a message.
fn action_label(intent: &Intent) -> String {
    match intent {
        Intent::Edit { action, .. } => action.label.clone(),
        Intent::Convert { .. } => "convert".to_owned(),
    }
}

/// A family, for "one PDF can only combine PDF files".
fn family(kind: Kind) -> &'static str {
    match kind {
        Kind::Image => "image",
        Kind::Video => "video",
        Kind::Audio => "audio",
        Kind::Pdf => "PDF",
        Kind::Text => "text",
        Kind::Data => "data",
        Kind::Document => "document",
        Kind::Other => "recognised",
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
