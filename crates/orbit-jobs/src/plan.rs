//! Turning a request into a command line.
//!
//! Everything here is pure: given a [`JobRequest`] this module returns the exact
//! arguments an engine should be run with, and creates nothing but the output
//! plan's scratch file. That keeps the interesting part — *what Orbit asks
//! `ffmpeg`, `qpdf` or `LibreOffice` to do* — testable with plain assertions
//! instead of process spawning, which is where command-building bugs usually
//! hide.
//!
//! Three engines refuse to be a single argument list, and the model here bends
//! to fit them rather than pretending otherwise:
//!
//! * **`LibreOffice` names its own output** (`--outdir` plus the source's stem),
//!   so a job can ask for the file to be [collected](Collected) afterwards.
//! * **ffmpeg's concat demuxer reads a list file**, so a job can declare
//!   [auxiliary files](AuxFile) to write before it starts.
//! * **`orbit-native` is not a process at all**, so a plan is a
//!   [`Backend`] — a command line to spawn or a [`NativeOp`] to run in-process.
//!
//! Actions that need an engine Orbit does not drive yet return
//! [`PlanError::NotYetSupported`] rather than guessing a command line. A wrong
//! flag is worse than an honest "not yet".

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use orbit_registry::Kind;
use serde_json::Value;

use crate::output::OutputPlan;

/// Option values, as they arrive from the registry and the UI.
pub type Options = BTreeMap<String, Value>;

/// How the runner should read progress out of the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    /// No machine-readable progress; the job is running or it is not.
    None,
    /// ffmpeg's `-progress pipe:1`: `key=value` lines on stdout.
    FfmpegKeyValue,
}

/// A file an engine names itself rather than one Orbit hands over.
///
/// `LibreOffice` always writes `<stem>.<ext>` into its `--outdir`, so a name of
/// Orbit's choosing can only be had by collecting the file afterwards. The
/// directory is created before the run and removed after it — which also keeps
/// `LibreOffice` from overwriting anything that was already in the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collected {
    /// Directory the engine writes into.
    pub dir: PathBuf,
    /// File inside `dir` to move onto the scratch path on success.
    pub name: String,
}

/// A file the harness writes before the engine starts.
///
/// ffmpeg's concat demuxer takes a list *file*, not a list of arguments, so a
/// multi-input merge needs one. Describing it here rather than writing it keeps
/// this module free of side effects, so the exact contents stay assertable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuxFile {
    /// Where to write it.
    pub path: PathBuf,
    /// What to put in it.
    pub contents: String,
}

/// A resolved command line.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// Executable to run.
    pub program: OsString,
    /// Arguments, in order.
    pub args: Vec<OsString>,
    /// How to read progress from this process.
    pub progress: ProgressMode,
    /// The file whose duration progress is measured against, for ffmpeg.
    pub media_input: Option<PathBuf>,
    /// Files to write before the run.
    pub aux_files: Vec<AuxFile>,
    /// Directories to create before the run and remove after it, however it
    /// ends. `LibreOffice` needs one for its output and one for its profile.
    pub work_dirs: Vec<PathBuf>,
    /// Output the engine names itself, to collect onto the scratch path.
    pub collected: Option<Collected>,
}

impl CommandSpec {
    /// A plain command line: nothing to write first, nothing to collect after.
    fn plain(program: PathBuf, args: Vec<OsString>, progress: ProgressMode) -> Self {
        Self {
            program: program.into(),
            args,
            progress,
            media_input: None,
            aux_files: Vec::new(),
            work_dirs: Vec::new(),
            collected: None,
        }
    }

    /// The command as a readable line, for logs and tests.
    #[must_use]
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.to_string_lossy().into_owned()];
        parts.extend(
            self.args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned()),
        );
        parts.join(" ")
    }
}

/// How `orbit-native` should treat an image on its way to the encoder.
///
/// One transform per job, because the ring runs one action at a time; chains
/// arrive in M5 and will compose these.
#[derive(Debug, Clone, PartialEq)]
pub enum Transform {
    /// Re-encode as-is. This is also what "remove metadata" means for a built-in
    /// engine: decoding and re-encoding drops every tag there is.
    None,
    /// Scale, by percentage, longest edge or named box.
    Resize(Resize),
    /// Cut a rectangle out of the image.
    Crop {
        /// Left edge.
        x: u32,
        /// Top edge.
        y: u32,
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// Turn a quarter or half turn.
    Rotate {
        /// Degrees clockwise; 90, 180 or 270.
        degrees: u16,
    },
    /// Mirror across an axis.
    Flip {
        /// True for a top-to-bottom mirror.
        vertical: bool,
    },
    /// Drop colour.
    Grayscale,
}

/// A resize expressed the way the panel asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resize {
    /// Percentage of the original size.
    Percent(u32),
    /// Fit inside a square: the longest edge becomes this many pixels.
    Fit(u32),
    /// Fit inside an explicit box, preserving the aspect ratio.
    Box {
        /// Box width.
        width: u32,
        /// Box height.
        height: u32,
    },
}

/// Work the built-in engine does in-process.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeOp {
    /// Decode an image, transform it and write it again.
    Image {
        /// File to read.
        source: PathBuf,
        /// Extension to write, without the dot.
        target_ext: String,
        /// What to do on the way through.
        transform: Transform,
        /// Encoder quality, for formats that have one.
        quality: Option<u8>,
        /// Colour to composite transparency onto, for formats without alpha.
        background: Option<String>,
    },
    /// Render Markdown or plain text as HTML.
    Markup {
        /// File to read.
        source: PathBuf,
    },
    /// Read a delimiter-separated file and write CSV/TSV/JSON.
    Tabular {
        /// File to read.
        source: PathBuf,
        /// Extension to write: `csv`, `tsv` or `json`.
        target_ext: String,
        /// Field separator in the source.
        delimiter: char,
        /// Whether the first row names the columns.
        header: bool,
    },
}

/// What actually carries a job out.
#[derive(Debug, Clone)]
pub enum Backend {
    /// Spawn a process.
    Process(CommandSpec),
    /// Run in-process, in Rust.
    Native(NativeOp),
    /// Run several backends in order, each one reading the previous result.
    Chain(Vec<PlannedStep>),
}

/// One link of a chain: what to run and where its result belongs.
#[derive(Debug, Clone)]
pub struct PlannedStep {
    /// The process or native operation.
    pub backend: Backend,
    /// File this step must write; the next step reads it.
    pub output: PathBuf,
}

/// A run of files written by one engine invocation.
///
/// The engine writes `<prefix>-1.<ext>`, `<prefix>-2.<ext>`, …; the runner
/// renames them to the user-facing series (`scan (converted 001).png`) only
/// once the engine has exited successfully.
#[derive(Debug, Clone)]
pub struct SeriesPlan {
    /// Path before the engine's number and extension.
    pub prefix: PathBuf,
    /// Extension the run carries, without the dot.
    pub ext: String,
    /// Word used in the published names.
    pub tag: String,
}

/// One step of a chain or preset.
///
/// A step is everything a [`JobRequest`] is except the files: the resolver works
/// out each step's engine and extension before the planner builds the command
/// lines, because step *n+1* reads what step *n* writes.
#[derive(Debug, Clone)]
pub struct StepRequest {
    /// Engine id, which decides *how* the command is built.
    pub engine: String,
    /// Executable to run.
    pub program: PathBuf,
    /// Family of the step's input.
    pub kind: Kind,
    /// Action id, or `convert`.
    pub action: String,
    /// Extension this step writes.
    pub target_ext: String,
    /// Word used in the output name; only the last step's matters.
    pub tag: String,
    /// Values for this step.
    pub options: Options,
}

/// Everything the planner needs, already resolved against the registry.
#[derive(Debug, Clone)]
pub struct JobRequest {
    /// Engine id, which decides *how* the command is built.
    pub engine: String,
    /// Executable to run: the engine's primary binary or a found alternate.
    pub program: PathBuf,
    /// Family of the first source file.
    pub kind: Kind,
    /// `convert`, an action id, or `chain` for a preset.
    pub action: String,
    /// Extension the result should carry, without the dot.
    pub target_ext: String,
    /// Word used in the output name: `converted`, `compressed`, `trimmed`.
    pub tag: String,
    /// Inputs, in drag order. The first one names the output.
    pub sources: Vec<PathBuf>,
    /// Values used when the petal is dropped without opening the options panel.
    pub options: Options,
    /// Where to write, when the user has chosen a folder.
    pub output_dir: Option<PathBuf>,
    /// Steps of a chain, in order. Empty for a single-action job.
    pub steps: Vec<StepRequest>,
}

/// A request plus the plan it produced.
#[derive(Debug)]
pub struct Plan {
    /// What carries the job out: a process, or Rust code in this process.
    pub backend: Backend,
    /// The reserved destination and scratch file.
    pub output: OutputPlan,
    /// Set when the backend writes a numbered run instead of one file.
    pub series: Option<SeriesPlan>,
    /// Intermediate files a chain writes; removed however the job ends.
    pub temps: Vec<PathBuf>,
}

/// Why a request could not become a command line.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    /// The action needs a value that a plain drop does not supply.
    #[error("{action} needs “{option}” before it can run")]
    NeedsOption {
        /// Action that asked for it.
        action: String,
        /// Option key the caller has to fill in.
        option: String,
    },
    /// The action needs more files than were given.
    #[error("{action} needs at least {wanted} files, but got {got}")]
    NotEnoughFiles {
        /// Action id.
        action: String,
        /// How many it needs.
        wanted: usize,
        /// How many were supplied.
        got: usize,
    },
    /// Orbit does not drive this engine for this action yet.
    #[error("{action} on {engine} lands in {milestone}")]
    NotYetSupported {
        /// Action id.
        action: String,
        /// Engine id.
        engine: String,
        /// The milestone that will add it.
        milestone: &'static str,
    },
    /// An option carries a value this engine cannot express.
    ///
    /// Better than passing it through and letting the engine fail with a
    /// message about its own syntax: the user picked a page list, and the
    /// useful answer names the limit.
    #[error("{action} cannot take “{value}” for {option} on {engine}")]
    ValueUnsupported {
        /// Action id.
        action: String,
        /// Engine id.
        engine: &'static str,
        /// Option key.
        option: String,
        /// The value that cannot be expressed.
        value: String,
    },
    /// Called with no files at all.
    #[error("no input files")]
    NoInput,
    /// A merge was asked for an output its inputs cannot become.
    #[error("{action} cannot turn {kind} files into {value}")]
    MergeOutput {
        /// Action id.
        action: String,
        /// Family of the inputs.
        kind: &'static str,
        /// Requested output.
        value: String,
    },
    /// The destination could not be reserved.
    #[error(transparent)]
    Output(#[from] std::io::Error),
}

/// Build the command for `request` and reserve its destination.
///
/// # Errors
///
/// Fails when the request has no inputs, when the action needs an option or a
/// file count that a plain drop did not supply, when the engine is not driven
/// for that action yet, or when the output name cannot be reserved.
pub fn plan(request: &JobRequest) -> Result<Plan, PlanError> {
    if !request.steps.is_empty() {
        return plan_chain(request);
    }
    let Some(first) = request.sources.first() else {
        return Err(PlanError::NoInput);
    };

    let output = OutputPlan::reserve(
        first,
        &request.tag,
        &request.target_ext,
        request.output_dir.as_deref(),
    )?;

    let series = series_for(request, output.scratch_path())?;
    let backend = backend(request, output.scratch_path())?;
    Ok(Plan {
        backend,
        output,
        series,
        temps: Vec::new(),
    })
}

/// Build the command lines for a chain, feeding each step's output to the next.
///
/// Only the final step writes to the reserved destination; the earlier ones
/// write hidden siblings that [`Plan::temps`] has removed however the job ends.
fn plan_chain(request: &JobRequest) -> Result<Plan, PlanError> {
    let Some(first) = request.sources.first() else {
        return Err(PlanError::NoInput);
    };
    let output = OutputPlan::reserve(
        first,
        &request.tag,
        &request.target_ext,
        request.output_dir.as_deref(),
    )?;

    let last = request.steps.len().saturating_sub(1);
    let mut backends = Vec::with_capacity(request.steps.len());
    let mut temps = Vec::new();
    let mut input = first.clone();

    for (index, step) in request.steps.iter().enumerate() {
        let output_path = if index == last {
            output.scratch_path().to_path_buf()
        } else {
            let path = output
                .scratch_sibling(&format!("step{index}"))
                .with_extension(&step.target_ext);
            temps.push(path.clone());
            path
        };
        let step_request = JobRequest {
            engine: step.engine.clone(),
            program: step.program.clone(),
            kind: step.kind,
            action: step.action.clone(),
            target_ext: step.target_ext.clone(),
            tag: step.tag.clone(),
            sources: vec![input],
            options: step.options.clone(),
            output_dir: None,
            steps: Vec::new(),
        };
        backends.push(PlannedStep {
            backend: backend(&step_request, &output_path)?,
            output: output_path.clone(),
        });
        input = output_path;
    }

    Ok(Plan {
        backend: Backend::Chain(backends),
        output,
        series: None,
        temps,
    })
}

/// Whether this request writes a numbered run, and where.
fn series_for(request: &JobRequest, scratch: &Path) -> Result<Option<SeriesPlan>, PlanError> {
    let prefix = without_extension(scratch);
    if request.action == "extract_frames" {
        return Ok(Some(SeriesPlan {
            prefix,
            ext: request.target_ext.clone(),
            tag: request.tag.clone(),
        }));
    }
    if request.action == "convert"
        && request.kind == Kind::Pdf
        && matches!(
            request.target_ext.as_str(),
            "png" | "jpg" | "tiff" | "webp" | "avif"
        )
        && pdf_range(request)?.is_some()
    {
        return Ok(Some(SeriesPlan {
            prefix,
            ext: request.target_ext.clone(),
            tag: request.tag.clone(),
        }));
    }
    Ok(None)
}

/// The page range a PDF render should cover, when it covers more than one page.
///
/// `Ok(None)` means one page: either no range was chosen or it names a single
/// page. A list such as `1-3,7` is refused, because the renderers cannot express
/// it in one pass and silently rendering less would be worse.
fn pdf_range(request: &JobRequest) -> Result<Option<(u32, Option<u32>)>, PlanError> {
    let Some(text) = opt_str(&request.options, "pages").filter(|text| !text.trim().is_empty())
    else {
        return Ok(None);
    };
    let Some((first, last)) = contiguous_range(&text) else {
        return Err(PlanError::ValueUnsupported {
            action: request.action.clone(),
            engine: if request.engine == "poppler" {
                "poppler"
            } else if request.engine == "magick" {
                "magick"
            } else {
                "ghostscript"
            },
            option: "pages".to_owned(),
            value: text,
        });
    };
    Ok((last != Some(first)).then_some((first, last)))
}

/// Dispatch to the engine that was chosen for this job.
fn backend(request: &JobRequest, scratch: &Path) -> Result<Backend, PlanError> {
    if request.engine == "orbit-native" {
        return Ok(Backend::Native(native(request)?));
    }
    Ok(Backend::Process(build(request, scratch)?))
}

/// Dispatch to the engine that was chosen for this job.
fn build(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    match request.engine.as_str() {
        "ffmpeg" => build_ffmpeg(request, scratch),
        "magick" => build_magick(request, scratch),
        "vips" => build_vips(request, scratch),
        "poppler" => build_poppler(request, scratch),
        "ghostscript" => build_ghostscript(request, scratch),
        "qpdf" => build_qpdf(request, scratch),
        "libreoffice" => Ok(build_libreoffice(request, scratch)),
        "libheif" => build_libheif(request, scratch),
        "tesseract" => build_tesseract(request, scratch),
        engine => Err(unsupported(&request.action, engine, "M5")),
    }
}

/// A path beside the scratch file, for the side files an engine needs.
///
/// Same rule as [`OutputPlan::scratch_sibling`], on the raw path, because the
/// builders are handed the scratch path rather than the plan.
fn sibling(scratch: &Path, label: &str) -> PathBuf {
    crate::output::scratch_sibling(scratch, label)
}

/// The scratch path without its extension.
///
/// Two engines take a *prefix* and append the extension themselves: `pdftoppm
/// -singlefile` and `tesseract`. Handing them the scratch path verbatim would
/// produce `.orbit-1.png.png`.
fn without_extension(scratch: &Path) -> PathBuf {
    if scratch.extension().is_some() {
        scratch.with_extension("")
    } else {
        scratch.to_path_buf()
    }
}

/// ffmpeg's image2 muxer takes a pattern, not a path: `%04d` is replaced with
/// the frame number.
fn frame_pattern(scratch: &Path) -> OsString {
    let ext = scratch
        .extension()
        .map_or_else(String::new, |ext| ext.to_string_lossy().into_owned());
    format!("{}-%04d.{ext}", without_extension(scratch).display()).into()
}

fn unsupported(action: &str, engine: &str, milestone: &'static str) -> PlanError {
    PlanError::NotYetSupported {
        action: action.to_owned(),
        engine: engine.to_owned(),
        milestone,
    }
}

// ---------------------------------------------------------------- ImageMagick

/// `ImageMagick` takes every job as a pipeline: settings, inputs, operators,
/// output. Three jobs need a setting *before* the input to have any effect —
/// raster density, PDF page density and the page canvas — so they get their own
/// builders instead of pretending every action is a suffix.
fn build_magick(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    match (request.action.as_str(), request.kind) {
        ("merge", _) => return Ok(magick_merge(request, scratch)),
        // One image as a document is laid out, not just re-encoded.
        ("convert", Kind::Image) if request.target_ext == "pdf" => {
            return Ok(magick_image_to_pdf(request, scratch));
        }
        // ImageMagick addresses PDF pages with a bracketed index before the
        // file name, and decodes at the density in force when it reads.
        ("convert", Kind::Pdf) => return magick_pdf_render(request, scratch),
        ("convert", Kind::Image) if is_svg(&request.sources[0]) => {
            return Ok(magick_vector(request, scratch));
        }
        _ => {}
    }

    let options = &request.options;
    let action = request.action.as_str();
    let mut args: Vec<OsString> = request.sources.iter().map(OsString::from).collect();

    match action {
        "convert" | "compress" => {
            push_quality(&mut args, options);
            if opt_bool(options, "strip_metadata") {
                args.push("-strip".into());
            }
        }
        "strip_metadata" => args.push("-strip".into()),
        "grayscale" => {
            args.push("-colorspace".into());
            args.push("Gray".into());
        }
        "rotate" => {
            let degrees = opt_str(options, "rotate").unwrap_or_else(|| "90".to_owned());
            args.push("-rotate".into());
            args.push(degrees.into());
        }
        "flip" => {
            // ImageMagick's `-flop` is the horizontal one; the names flip over.
            let vertical = opt_str(options, "flip_axis").as_deref() == Some("vertical");
            args.push(if vertical { "-flip" } else { "-flop" }.into());
        }
        "resize" => {
            if let Some(geometry) = magick_geometry(options) {
                args.push("-resize".into());
                args.push(geometry.into());
            }
        }
        "crop" => {
            let region = region_option(options, action)?;
            args.push("-crop".into());
            args.push(magick_crop(&region).into());
            args.push("+repage".into());
        }
        other => return Err(unsupported(other, "magick", "M5")),
    }

    args.push(scratch.into());
    Ok(CommandSpec::plain(
        request.program.clone(),
        args,
        ProgressMode::None,
    ))
}

/// Several files folded into one document.
///
/// `ImageMagick` writes a page per image in a PDF and a frame per image in a
/// GIF. `-delay` is in hundredths of a second and has to come before the frames
/// it applies to, so it leads the argument list.
fn magick_merge(request: &JobRequest, scratch: &Path) -> CommandSpec {
    let mut args: Vec<OsString> = Vec::new();
    if request.target_ext == "gif" {
        push_flag(&mut args, "-delay", GIF_DELAY);
    }
    args.extend(request.sources.iter().map(OsString::from));
    args.push(scratch.into());
    CommandSpec::plain(request.program.clone(), args, ProgressMode::None)
}

/// A single image as a page.
///
/// `magick in.png out.pdf` makes a page exactly the image's size, and `-page
/// a4` *stretches* the image to fill A4. Neither is what the page, margin and
/// orientation options promise, so the page is built first and the image
/// composited onto it — the only construction that honours all three.
fn magick_image_to_pdf(request: &JobRequest, scratch: &Path) -> CommandSpec {
    let options = &request.options;
    let page = opt_str(options, "page").unwrap_or_else(|| "source".to_owned());
    let landscape = opt_str(options, "orientation").as_deref() == Some("landscape");
    let canvas = (page != "source")
        .then(|| page_box(&page, landscape))
        .flatten();

    let mut args: Vec<OsString> = Vec::new();
    if let Some((width, height)) = canvas {
        push_flag(&mut args, "-size", &format!("{width}x{height}"));
        args.push("xc:white".into());
        push_flag(&mut args, "-density", &PDF_DPI.to_string());
        args.push("-units".into());
        args.push("PixelsPerInch".into());
        // Parenthesised so `-resize` touches the image and not the page.
        args.push("(".into());
    }
    args.push(request.sources[0].clone().into_os_string());
    if let Some((width, height)) = canvas {
        let margin = margin_pixels(opt_str(options, "margin").as_deref()).unwrap_or(0);
        let box_width = width.saturating_sub(margin.saturating_mul(2)).max(1);
        let box_height = height.saturating_sub(margin.saturating_mul(2)).max(1);
        push_flag(&mut args, "-resize", &format!("{box_width}x{box_height}"));
        args.push(")".into());
        args.push("-gravity".into());
        args.push("center".into());
        args.push("-composite".into());
    }
    push_quality(&mut args, options);
    args.push(scratch.into());
    CommandSpec::plain(request.program.clone(), args, ProgressMode::None)
}

/// One page of a PDF, at the requested resolution.
fn magick_pdf_render(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    let options = &request.options;
    let dpi = opt_str(options, "dpi").unwrap_or_else(|| "150".to_owned());
    let input = request.sources[0].display();

    let mut args: Vec<OsString> = Vec::new();
    push_flag(&mut args, "-density", &dpi);
    push_quality(&mut args, options);

    if let Some((first, last)) = pdf_range(request)? {
        // Zero-based, and `first-` means "through the last page".
        let range = match last {
            Some(last) => format!("{}-{}", first - 1, last - 1),
            None => format!("{}-", first - 1),
        };
        args.push(format!("{input}[{range}]").into());
        let ext = scratch
            .extension()
            .map_or_else(String::new, |ext| ext.to_string_lossy().into_owned());
        args.push(format!("{}-%03d.{ext}", without_extension(scratch).display()).into());
    } else {
        // Zero-based: the panel counts pages from one, ImageMagick from zero.
        let page = single_page(request);
        args.push(format!("{input}[{}]", page - 1).into());
        args.push(scratch.into());
    }

    Ok(CommandSpec::plain(
        request.program.clone(),
        args,
        ProgressMode::None,
    ))
}

/// A vector source rasterised at the requested scale.
///
/// SVG has no pixels of its own, so the density *is* the size; setting it
/// explicitly makes `scale` mean the same thing on every machine.
fn magick_vector(request: &JobRequest, scratch: &Path) -> CommandSpec {
    let options = &request.options;
    let scale = opt_f64(options, "scale").unwrap_or(1.0).max(0.01);
    let dpi = (VECTOR_DPI * scale).round().max(1.0);

    let mut args: Vec<OsString> = Vec::new();
    push_flag(&mut args, "-density", &format!("{dpi:.0}"));
    args.push(request.sources[0].clone().into_os_string());
    if let Some(background) = opt_str(options, "background") {
        push_flag(&mut args, "-background", &background);
    }
    push_quality(&mut args, options);
    args.push(scratch.into());
    CommandSpec::plain(request.program.clone(), args, ProgressMode::None)
}

/// How long each frame of a merged GIF is shown, in hundredths of a second.
const GIF_DELAY: &str = "20";

/// Resolution `ImageMagick` rasterises vectors at when `scale` is 1.
const VECTOR_DPI: f64 = 96.0;

/// `ImageMagick`'s default resolution, which fixes the page size in points.
const PDF_DPI: i64 = 72;

/// Page sizes in points at [`PDF_DPI`], portrait.
fn page_box(name: &str, landscape: bool) -> Option<(u32, u32)> {
    let (width, height) = match name {
        "a4" => (595, 842),
        "a3" => (842, 1191),
        "letter" => (612, 792),
        "legal" => (612, 1008),
        _ => return None,
    };
    Some(if landscape {
        (height, width)
    } else {
        (width, height)
    })
}

/// A margin as pixels on the [`PDF_DPI`] canvas.
///
/// `ImageMagick` understands units in geometry strings but `-resize` takes plain
/// pixels, so the conversion happens here rather than in the argument.
fn margin_pixels(text: Option<&str>) -> Option<u32> {
    let text = text?.trim();
    let split = text
        .find(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let value: f64 = number.trim().parse().ok()?;
    let per_unit = match unit.trim().to_ascii_lowercase().as_str() {
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        "in" => 72.0,
        "" | "pt" | "px" => 1.0,
        _ => return None,
    };
    // A margin is a whole number of pixels, and a page is far smaller than any
    // value the clamp could reject.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pixels = (value * per_unit).round().clamp(0.0, f64::from(u32::MAX)) as u32;
    Some(pixels)
}

/// A crop region in the shape the panel documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Region {
    /// Left edge.
    x: u32,
    /// Top edge.
    y: u32,
    /// Width.
    width: u32,
    /// Height.
    height: u32,
}

/// The `region` option, which accepts both spellings.
///
/// The option help says `left,top,width,height`; `ImageMagick`'s own
/// `1024x768+10+20` is accepted too, because that is what earlier builds
/// advertised and a saved preset may still carry one.
fn region_option(options: &Options, action: &str) -> Result<Region, PlanError> {
    let missing = || PlanError::NeedsOption {
        action: action.to_owned(),
        option: "region".to_owned(),
    };
    opt_str(options, "region")
        .and_then(|region| parse_region(&region))
        .ok_or_else(missing)
}

/// Parse `left,top,width,height` or `WxH+X+Y`.
fn parse_region(text: &str) -> Option<Region> {
    let text = text.trim();
    if text.contains(['x', '+']) {
        let parts: Vec<&str> = text
            .split(['x', '+'])
            .filter(|part| !part.is_empty())
            .collect();
        if let [width, height, x, y] = parts.as_slice() {
            return Some(Region {
                x: x.parse().ok()?,
                y: y.parse().ok()?,
                width: width.parse().ok()?,
                height: height.parse().ok()?,
            });
        }
    }
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [x, y, width, height] => Some(Region {
            x: x.parse().ok()?,
            y: y.parse().ok()?,
            width: width.parse().ok()?,
            height: height.parse().ok()?,
        }),
        _ => None,
    }
}

/// `ImageMagick`'s `WxH+X+Y`.
fn magick_crop(region: &Region) -> String {
    format!(
        "{}x{}+{}+{}",
        region.width, region.height, region.x, region.y
    )
}

fn is_svg(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}

fn push_quality(args: &mut Vec<OsString>, options: &Options) {
    if let Some(quality) = opt_i64(options, "quality") {
        args.push("-quality".into());
        args.push(quality.to_string().into());
    }
}

/// The `-resize` geometry for the chosen mode, or `None` when resizing is off.
fn magick_geometry(options: &Options) -> Option<String> {
    let mode = opt_str(options, "resize_mode").unwrap_or_else(|| "none".to_owned());
    let geometry = match mode.as_str() {
        "percent" => {
            let percent = opt_i64(options, "percent").unwrap_or(50);
            format!("{percent}%")
        }
        "max_dim" => {
            let side = opt_i64(options, "max_dim").unwrap_or(1920);
            format!("{side}x{side}>")
        }
        "preset" => {
            let preset = opt_str(options, "resize_preset").unwrap_or_else(|| "1080p".to_owned());
            format!("{}>", target_box(&preset).unwrap_or("1920x1080"))
        }
        _ => return None,
    };
    Some(geometry)
}

/// Pixel box for a named target size, shared by the image `resize` presets and
/// the video `resolution` option so the two can never drift apart.
///
/// `None` means the name is unknown, which callers read as "leave the size
/// alone" rather than guessing one.
fn target_box(name: &str) -> Option<&'static str> {
    Some(match name {
        "4k" => "3840x2160",
        "1440p" => "2560x1440",
        "1080p" => "1920x1080",
        "720p" => "1280x720",
        "480p" => "854x480",
        "360p" => "640x360",
        "square_1080" => "1080x1080",
        "story_1080x1920" => "1080x1920",
        "thumbnail" => "256x256",
        _ => return None,
    })
}

// ----------------------------------------------------------------------- vips

/// libvips is deliberately minimal here: `vips copy` is the one form Orbit is
/// sure of, and everything else waits until it can be tested against a real
/// install rather than shipped on faith.
fn build_vips(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    if request.action != "convert" {
        return Err(unsupported(&request.action, "vips", "M5"));
    }
    Ok(CommandSpec::plain(
        request.program.clone(),
        vec![
            "copy".into(),
            request.sources[0].clone().into_os_string(),
            scratch.into(),
        ],
        ProgressMode::None,
    ))
}

// --------------------------------------------------------------------- ffmpeg

fn build_ffmpeg(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    // Four jobs are not a filter on one input, and would be wrong if they were
    // squeezed into the general path below.
    match (request.action.as_str(), request.kind) {
        ("merge", _) => return Ok(ffmpeg_merge(request, scratch)),
        ("convert", Kind::Video) if request.target_ext == "gif" => {
            return Ok(ffmpeg_gif(request, scratch));
        }
        ("convert", Kind::Image) if is_video_ext(&request.target_ext) => {
            return Ok(ffmpeg_slideshow(request, scratch));
        }
        ("convert", Kind::Audio) if is_video_ext(&request.target_ext) => {
            return Ok(ffmpeg_visualiser(request, scratch));
        }
        _ => {}
    }

    let options = &request.options;
    let action = request.action.as_str();
    let input = &request.sources[0];
    let audio_only = matches!(request.kind, Kind::Audio);

    let mut args: Vec<OsString> = vec!["-hide_banner".into(), "-nostdin".into(), "-y".into()];
    let mut filters: Vec<String> = Vec::new();

    // Seeking before `-i` decodes from the nearest keyframe instead of decoding
    // and discarding everything up to the point, which matters on long files.
    let seek = if action == "extract_frames" {
        opt_str(options, "time")
    } else {
        opt_str(options, "start")
    };
    if let Some(at) = seek {
        args.push("-ss".into());
        args.push(at.into());
    }

    if audio_only || action == "extract_audio" {
        args.push("-vn".into());
    }

    args.push("-i".into());
    args.push(input.into());

    if let Some(end) = opt_str(options, "end") {
        args.push("-to".into());
        args.push(end.into());
    }

    push_ffmpeg_action(&mut args, &mut filters, options, action, audio_only)?;

    if opt_bool(options, "strip_metadata") && action != "strip_metadata" {
        args.push("-map_metadata".into());
        args.push("-1".into());
    }

    if let Some(chain) = join_filters(&filters) {
        args.push("-vf".into());
        args.push(chain.into());
    }

    if action == "extract_frames" {
        let limit = opt_i64(options, "frame_limit")
            .unwrap_or(100)
            .clamp(1, 100_000);
        push_flag(&mut args, "-frames:v", &limit.to_string());
    }

    push_progress(&mut args);
    if action == "extract_frames" {
        args.push(frame_pattern(scratch));
    } else {
        args.push(scratch.into());
    }

    let mut spec = CommandSpec::plain(request.program.clone(), args, ProgressMode::FfmpegKeyValue);
    spec.media_input = Some(input.clone());
    Ok(spec)
}

/// Joining files without re-encoding.
///
/// ffmpeg's concat demuxer reads a list *file*, not a list of arguments, which
/// is why the plan carries an aux file. `-safe 0` lifts the demuxer's refusal
/// to read absolute paths.
///
/// Audio is re-encoded: stream-copying parts that came from separate encodes
/// leaves each part's own timestamps, and the muxer complains about the joins.
/// Video is copied, because re-encoding a long clip to concatenate it is a
/// surprising amount of work for a merge.
fn ffmpeg_merge(request: &JobRequest, scratch: &Path) -> CommandSpec {
    let list = sibling(scratch, CONCAT_LIST);
    let mut contents = String::new();
    for source in &request.sources {
        contents.push_str("file '");
        // The demuxer's own escaping: a quote inside the path closes and
        // reopens the string.
        contents.push_str(&source.to_string_lossy().replace('\'', r"'\''"));
        contents.push_str("'\n");
    }

    let mut args: Vec<OsString> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list.clone().into_os_string(),
    ];
    if request.kind == Kind::Audio {
        push_audio_encoding(&mut args, &request.options);
    } else {
        push_flag(&mut args, "-c", "copy");
    }
    push_progress(&mut args);
    args.push(scratch.into());

    let mut spec = CommandSpec::plain(request.program.clone(), args, ProgressMode::FfmpegKeyValue);
    // No single input has the joined duration, so no percentage is measured.
    spec.media_input = None;
    spec.aux_files.push(AuxFile {
        path: list,
        contents,
    });
    spec
}

/// Video to GIF, palette and all.
///
/// A GIF holds 256 colours, so ffmpeg needs a palette built from the clip
/// itself: `palettegen` reads the frames once, `paletteuse` maps them through
/// the result. Splitting the stream runs both passes in one command.
fn ffmpeg_gif(request: &JobRequest, scratch: &Path) -> CommandSpec {
    let options = &request.options;
    let fps = opt_i64(options, "fps").unwrap_or(15);
    let width = opt_i64(options, "width").unwrap_or(480);

    let mut args: Vec<OsString> = vec!["-hide_banner".into(), "-nostdin".into(), "-y".into()];
    if let Some(start) = opt_str(options, "start") {
        push_flag(&mut args, "-ss", &start);
    }
    args.push("-i".into());
    args.push(request.sources[0].clone().into_os_string());
    if let Some(duration) = opt_str(options, "duration") {
        push_flag(&mut args, "-t", &duration);
    }
    // `fps` first, so the palette is built from the frames that survive.
    push_flag(
        &mut args,
        "-filter_complex",
        &format!(
            "fps={fps},scale={width}:-2:flags=lanczos,split[a][b];\
             [a]palettegen[p];[b][p]paletteuse"
        ),
    );
    // `-loop 0` is forever and `-1` plays once; the names are ffmpeg's.
    let loop_once = opt_str(options, "loop").as_deref() == Some("once");
    push_flag(&mut args, "-loop", if loop_once { "-1" } else { "0" });
    push_progress(&mut args);
    args.push(scratch.into());
    CommandSpec::plain(request.program.clone(), args, ProgressMode::FfmpegKeyValue)
}

/// One still image as a clip.
///
/// `-loop 1` makes the single frame last, which is the difference between a
/// five-second clip and a five-hundredth-of-a-second one.
fn ffmpeg_slideshow(request: &JobRequest, scratch: &Path) -> CommandSpec {
    let options = &request.options;
    let duration = opt_str(options, "duration").unwrap_or_else(|| "5".to_owned());
    let fps = opt_i64(options, "fps").unwrap_or(30);
    let codec = opt_str(options, "codec").unwrap_or_else(|| "h264".to_owned());

    let mut args: Vec<OsString> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-loop".into(),
        "1".into(),
        "-i".into(),
        request.sources[0].clone().into_os_string(),
        "-t".into(),
        duration.into(),
        "-r".into(),
        fps.to_string().into(),
    ];
    let mut filters = vec!["scale=trunc(iw/2)*2:trunc(ih/2)*2".to_owned()];
    if let Some(geometry) = opt_str(options, "resolution")
        .filter(|resolution| resolution != "source")
        .as_deref()
        .and_then(target_box)
    {
        filters.push(fit_within(geometry));
    }
    push_flag(&mut args, "-c:v", &video_encoder(&codec));
    if let Some(crf) = opt_i64(options, "crf") {
        push_flag(&mut args, "-crf", &crf.to_string());
    }
    // yuv420p plays in browsers; h264's own default does not.
    push_flag(&mut args, "-pix_fmt", "yuv420p");
    if let Some(chain) = join_filters(&filters) {
        push_flag(&mut args, "-vf", &chain);
    }
    push_progress(&mut args);
    args.push(scratch.into());
    CommandSpec::plain(request.program.clone(), args, ProgressMode::FfmpegKeyValue)
}

/// A track with a picture, so audio can be uploaded somewhere that wants video.
fn ffmpeg_visualiser(request: &JobRequest, scratch: &Path) -> CommandSpec {
    let options = &request.options;
    let size = opt_str(options, "size").unwrap_or_else(|| "1920x1080".to_owned());
    let background = opt_str(options, "background").unwrap_or_else(|| "#101014".to_owned());
    // "Cover art" needs artwork the drop has not given us, so it draws a
    // spectrum instead of failing: a visualiser that refuses to visualise is
    // worse than one that picks the nearest thing it can do.
    let visual = match opt_str(options, "visualize_mode").as_deref() {
        Some("spectrum") => {
            format!("showspectrum=s={size}:mode=combined:color=intensity:scale=cbrt")
        }
        _ => format!("showwaves=s={size}:mode=cline:rate=25"),
    };
    // The visualisers draw on transparent, so the colour is a separate source
    // underneath rather than an option they do not have.
    let graph = format!("color=c={background}:s={size}[bg];[0:a]{visual}[w];[bg][w]overlay[out]");

    let args: Vec<OsString> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-i".into(),
        request.sources[0].clone().into_os_string(),
        "-filter_complex".into(),
        graph.into(),
        "-map".into(),
        "[out]".into(),
        "-map".into(),
        "0:a".into(),
        "-c:v".into(),
        "libx264".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-shortest".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        scratch.into(),
    ];
    let mut spec = CommandSpec::plain(request.program.clone(), args, ProgressMode::FfmpegKeyValue);
    spec.media_input = Some(request.sources[0].clone());
    spec
}

/// `-progress pipe:1 -nostats`, which every ffmpeg job here asks for.
fn push_progress(args: &mut Vec<OsString>) {
    args.push("-progress".into());
    args.push("pipe:1".into());
    args.push("-nostats".into());
}

/// True for containers this module can write a video into.
fn is_video_ext(ext: &str) -> bool {
    matches!(ext, "mp4" | "webm" | "mov" | "mkv")
}

/// The name of the aux file the concat demuxer reads.
const CONCAT_LIST: &str = "concat";

/// The per-action output flags for ffmpeg.
///
/// Filters are collected into `filters` rather than emitted here, because
/// ffmpeg accepts a single `-vf` per output and they have to be joined.
fn push_ffmpeg_action(
    args: &mut Vec<OsString>,
    filters: &mut Vec<String>,
    options: &Options,
    action: &str,
    audio_only: bool,
) -> Result<(), PlanError> {
    match action {
        "convert" => {
            if audio_only {
                push_audio_encoding(args, options);
            } else {
                push_video_encoding(args, options, filters);
            }
        }
        "compress" => {
            if audio_only {
                let bitrate = opt_str(options, "audio_bitrate").unwrap_or_else(|| "192000".into());
                push_flag(args, "-b:a", &bitrate);
            } else {
                let crf = opt_i64(options, "crf").unwrap_or(23).to_string();
                push_flag(args, "-c:v", "libx264");
                push_flag(args, "-crf", &crf);
                push_flag(args, "-preset", "medium");
            }
        }
        "strip_metadata" => {
            push_flag(args, "-map_metadata", "-1");
            push_flag(args, "-c", "copy");
        }
        "trim" => {
            // Stream copy: lossless and instant, at the cost of cutting on
            // keyframes. Re-encoding every trim would be the wrong default.
            push_flag(args, "-c", "copy");
        }
        "extract_audio" => push_audio_encoding(args, options),
        "extract_frames" => {
            // A run of stills: the fps filter thins the stream, and the builder
            // caps the count so a two-hour film cannot fill a disk by accident.
            let step = opt_f64(options, "frame_step").unwrap_or(1.0).max(0.01);
            filters.push(format!("fps={}", (1.0 / step).max(0.001)));
            let scale = opt_f64(options, "scale").unwrap_or(1.0);
            if (scale - 1.0).abs() > f64::EPSILON {
                filters.push(format!("scale=iw*{scale}:ih*{scale}"));
            }
        }
        "grayscale" => filters.push("format=gray".into()),
        "rotate" => {
            // 90° clockwise is ffmpeg's `transpose=1`. An unrecognised value
            // falls back to it rather than silently doing nothing.
            let degrees = opt_str(options, "rotate").unwrap_or_else(|| "90".to_owned());
            let rotate = match degrees.as_str() {
                "180" => "hflip,vflip",
                "270" => "transpose=2",
                _ => "transpose=1",
            };
            filters.push(rotate.to_owned());
        }
        "flip" => {
            let vertical = opt_str(options, "flip_axis").as_deref() == Some("vertical");
            filters.push(if vertical { "vflip" } else { "hflip" }.to_owned());
        }
        "resize" => {
            if let Some(scale) = ffmpeg_scale_filter(options) {
                filters.push(scale);
            }
        }
        "crop" => {
            let region = region_option(options, action)?;
            filters.push(format!(
                "crop={}:{}:{}:{}",
                region.width, region.height, region.x, region.y
            ));
        }
        other => return Err(unsupported(other, "ffmpeg", "M5")),
    }
    Ok(())
}

/// `-flag value`, the shape nearly every ffmpeg output option takes.
fn push_flag(args: &mut Vec<OsString>, flag: &str, value: &str) {
    args.push(flag.into());
    args.push(value.into());
}

/// Audio encoding flags, shared by conversions and `Audio from video`.
fn push_audio_encoding(args: &mut Vec<OsString>, options: &Options) {
    let codec = opt_str(options, "audio_codec");
    if let Some(codec) = &codec {
        args.push("-c:a".into());
        args.push(codec.clone().into());
    }
    if codec.as_deref() != Some("copy") {
        let bitrate = opt_str(options, "audio_bitrate").unwrap_or_else(|| "192000".into());
        args.push("-b:a".into());
        args.push(bitrate.into());
    }
    if let Some(rate) = opt_str(options, "sample_rate").filter(|rate| rate != "source") {
        args.push("-ar".into());
        args.push(rate.into());
    }
    match opt_str(options, "channels").as_deref() {
        Some("mono") => {
            args.push("-ac".into());
            args.push("1".into());
        }
        Some("stereo") => {
            args.push("-ac".into());
            args.push("2".into());
        }
        _ => {}
    }
    if opt_bool(options, "normalize") {
        args.push("-af".into());
        args.push("loudnorm".into());
    }
}

/// Video encoding flags. Filters are collected rather than emitted, because
/// ffmpeg accepts one `-vf` per output and they have to be comma-joined.
fn push_video_encoding(args: &mut Vec<OsString>, options: &Options, filters: &mut Vec<String>) {
    let codec = opt_str(options, "codec");
    if let Some(codec) = &codec {
        let encoder = video_encoder(codec);
        args.push("-c:v".into());
        args.push(encoder.into());
        if codec == "copy" {
            args.push("-c:a".into());
            args.push("copy".into());
            return;
        }
        if codec == "vp9" {
            // VP9 requires an explicit zero bitrate for CRF mode to engage.
            args.push("-b:v".into());
            args.push("0".into());
        }
    }
    if let Some(crf) = opt_i64(options, "crf") {
        args.push("-crf".into());
        args.push(crf.to_string().into());
    }
    if let Some(geometry) = opt_str(options, "resolution")
        .filter(|resolution| resolution != "source")
        .as_deref()
        .and_then(target_box)
    {
        filters.push(fit_within(geometry));
    }
    if let Some(fps) = opt_i64(options, "fps") {
        args.push("-r".into());
        args.push(fps.to_string().into());
    }
    push_audio_encoding(args, options);
}

/// The libav encoder name for a codec id from the registry.
///
/// Unknown ids pass through untouched, so a value the panel offers can name an
/// encoder directly without a code change here.
fn video_encoder(codec: &str) -> String {
    match codec {
        "h264" => "libx264",
        "h265" => "libx265",
        "vp9" => "libvpx-vp9",
        "av1" => "libsvtav1",
        "prores" => "prores_ks",
        other => other,
    }
    .to_owned()
}

/// A `scale` filter for the chosen resize mode, or `None` when resizing is off.
fn ffmpeg_scale_filter(options: &Options) -> Option<String> {
    let mode = opt_str(options, "resize_mode").unwrap_or_else(|| "none".to_owned());
    match mode.as_str() {
        "percent" => {
            let percent = opt_i64(options, "percent").unwrap_or(50);
            Some(format!("scale=iw*{percent}/100:-2"))
        }
        "max_dim" => {
            let side = opt_i64(options, "max_dim").unwrap_or(1920);
            Some(fit_within(&format!("{side}:{side}")))
        }
        "preset" => {
            let preset = opt_str(options, "resize_preset").unwrap_or_else(|| "1080p".to_owned());
            Some(fit_within(target_box(&preset).unwrap_or("1920x1080")))
        }
        _ => None,
    }
}

/// ffmpeg takes exactly one `-vf` per output, so a chain of filters has to be
/// one comma-joined argument. `None` when there is nothing to do.
fn join_filters(filters: &[String]) -> Option<String> {
    (!filters.is_empty()).then(|| filters.join(","))
}

/// `1920x1080` in ffmpeg's `W:H` dialect, shrunk to fit and kept even.
fn fit_within(box_geometry: &str) -> String {
    let (width, height) = box_geometry.split_once('x').unwrap_or(("1920", "1080"));
    format!("scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2")
}

// -------------------------------------------------------------------- poppler

/// poppler is a family of small tools rather than one program, so the command
/// follows from what the job needs: `pdftoppm` renders a page, `pdftotext`
/// reads one.
///
/// Neither takes an output *path*. Both take a prefix and append an extension
/// of their own choosing, so the scratch path goes in without one.
fn build_poppler(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    match request.target_ext.as_str() {
        "txt" => poppler_text(request, scratch),
        ext @ ("png" | "jpg" | "tiff") => poppler_render(request, scratch, ext),
        other => Err(PlanError::ValueUnsupported {
            action: request.action.clone(),
            engine: "poppler",
            option: "to".to_owned(),
            value: other.to_owned(),
        }),
    }
}

/// `pdftotext`, for the text layer a PDF already has.
///
/// This reads the document's own text. A scan without one comes out empty,
/// which is honest — OCR is a separate action, on images.
fn poppler_text(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    let options = &request.options;
    let mut args: Vec<OsString> = Vec::new();
    if opt_bool(options, "layout") {
        args.push("-layout".into());
    }
    if let Some(pages) = opt_str(options, "pages").filter(|pages| !pages.trim().is_empty()) {
        let (first, last) =
            contiguous_range(&pages).ok_or_else(|| PlanError::ValueUnsupported {
                action: request.action.clone(),
                engine: "poppler",
                option: "pages".to_owned(),
                value: pages.clone(),
            })?;
        push_flag(&mut args, "-f", &first.to_string());
        if let Some(last) = last {
            push_flag(&mut args, "-l", &last.to_string());
        }
    }
    args.push(request.sources[0].clone().into_os_string());
    args.push(scratch.into());
    Ok(CommandSpec::plain(
        request.program.clone(),
        args,
        ProgressMode::None,
    ))
}

/// One page of a PDF as an image, or a numbered run when a range is chosen.
fn poppler_render(
    request: &JobRequest,
    scratch: &Path,
    ext: &str,
) -> Result<CommandSpec, PlanError> {
    let options = &request.options;
    let mut args: Vec<OsString> = Vec::new();
    args.push(
        match ext {
            "png" => "-png",
            "jpg" => "-jpeg",
            _ => "-tiff",
        }
        .into(),
    );
    if let Some(dpi) = opt_str(options, "dpi") {
        push_flag(&mut args, "-r", &dpi);
    }
    if ext == "jpg" {
        if let Some(quality) = opt_i64(options, "quality") {
            push_flag(&mut args, "-jpegopt", &format!("quality={quality}"));
        }
    }
    if let Some((first, last)) = pdf_range(request)? {
        push_flag(&mut args, "-f", &first.to_string());
        if let Some(last) = last {
            push_flag(&mut args, "-l", &last.to_string());
        }
        // No `-singlefile`: poppler appends `-1.png`, `-2.png`, … to the
        // prefix, and the runner publishes the run under Orbit's names.
        args.push(request.sources[0].clone().into_os_string());
        args.push(without_extension(scratch).into_os_string());
    } else {
        let page = single_page(request);
        push_flag(&mut args, "-f", &page.to_string());
        push_flag(&mut args, "-l", &page.to_string());
        args.push("-singlefile".into());
        args.push(request.sources[0].clone().into_os_string());
        args.push(without_extension(scratch).into_os_string());
    }
    Ok(CommandSpec::plain(
        request.program.clone(),
        args,
        ProgressMode::None,
    ))
}

/// The one page a single-page render should use.
fn single_page(request: &JobRequest) -> u32 {
    let named = opt_str(&request.options, "pages")
        .and_then(|text| contiguous_range(&text))
        .and_then(|(first, last)| (last == Some(first)).then_some(first));
    named
        .or_else(|| {
            opt_i64(&request.options, "pdf_page").and_then(|page| u32::try_from(page.max(1)).ok())
        })
        .unwrap_or(1)
}

/// A *contiguous* page range, which is all poppler can express: `1-3`, `4`,
/// `2-`, `-5`. A list such as `1-3,7` is refused rather than quietly truncated.
fn contiguous_range(text: &str) -> Option<(u32, Option<u32>)> {
    let text = text.trim();
    match text.split_once('-') {
        None => page_number(text).map(|page| (page, Some(page))),
        Some((first, last)) => {
            let first = if first.trim().is_empty() {
                1
            } else {
                page_number(first)?
            };
            let last = last.trim();
            let last = if last.is_empty() {
                None
            } else {
                Some(page_number(last)?)
            };
            Some((first, last))
        }
    }
}

fn page_number(text: &str) -> Option<u32> {
    text.trim().parse().ok().filter(|page| *page > 0)
}

// ---------------------------------------------------------------- ghostscript

/// Ghostscript's strength is rewriting a PDF: it can rasterise a page and it can
/// re-encode a document's streams in one pass.
fn build_ghostscript(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    let options = &request.options;
    let mut args: Vec<OsString> = vec![
        "-dSAFER".into(),
        "-dBATCH".into(),
        "-dNOPAUSE".into(),
        "-dQUIET".into(),
    ];

    match (request.action.as_str(), request.kind) {
        ("merge", _) => {
            // Inputs in order are already a concatenation: one device, one file.
            args.push("-sDEVICE=pdfwrite".into());
            args.push(output_flag(scratch));
            args.extend(request.sources.iter().map(OsString::from));
        }
        ("compress", Kind::Pdf) => {
            args.push(format!("-dPDFSETTINGS=/{}", pdf_preset(options)).into());
            args.push("-sDEVICE=pdfwrite".into());
            args.push(output_flag(scratch));
            args.push(request.sources[0].clone().into_os_string());
        }
        ("convert", Kind::Pdf) => {
            let device = match request.target_ext.as_str() {
                "png" => "png16m",
                "jpg" => "jpeg",
                "tiff" => "tiff24nc",
                other => {
                    return Err(PlanError::ValueUnsupported {
                        action: request.action.clone(),
                        engine: "ghostscript",
                        option: "to".to_owned(),
                        value: other.to_owned(),
                    });
                }
            };
            args.push(format!("-sDEVICE={device}").into());
            if let Some(dpi) = opt_str(options, "dpi") {
                push_flag(&mut args, "-r", &dpi);
            }
            if request.target_ext == "jpg" {
                if let Some(quality) = opt_i64(options, "quality") {
                    push_flag(&mut args, "-dJPEGQ", &quality.clamp(1, 100).to_string());
                }
            }
            let page = single_page(request);
            if let Some((first, last)) = pdf_range(request)? {
                push_flag(&mut args, "-dFirstPage", &first.to_string());
                if let Some(last) = last {
                    push_flag(&mut args, "-dLastPage", &last.to_string());
                }
                let ext = scratch
                    .extension()
                    .map_or_else(String::new, |ext| ext.to_string_lossy().into_owned());
                args.push(
                    format!(
                        "-sOutputFile={}-%03d.{ext}",
                        without_extension(scratch).display()
                    )
                    .into(),
                );
            } else {
                push_flag(&mut args, "-dFirstPage", &page.to_string());
                push_flag(&mut args, "-dLastPage", &page.to_string());
                args.push(output_flag(scratch));
            }
            args.push(request.sources[0].clone().into_os_string());
        }
        (action, _) => return Err(unsupported(action, "ghostscript", "M5")),
    }

    Ok(CommandSpec::plain(
        request.program.clone(),
        args,
        ProgressMode::None,
    ))
}

/// `-sOutputFile=<path>`, which ghostscript names exactly — even a dotfile.
fn output_flag(path: &Path) -> OsString {
    let mut flag = OsString::from("-sOutputFile=");
    flag.push(path);
    flag
}

/// Ghostscript's preset for the chosen compression.
///
/// `prepress` keeps image resolution and passes JPEGs through, so it is the
/// "quality" setting; `screen` downsamples to 72 dpi. Ghostscript cannot aim at
/// a byte count, which is what the other mode asks for, so an exact size waits
/// for the measure-and-retry loop in M5.
fn pdf_preset(options: &Options) -> &'static str {
    match opt_str(options, "compress_mode").as_deref() {
        Some("target_size") => "screen",
        _ => "prepress",
    }
}

// ----------------------------------------------------------------------- qpdf

/// qpdf does structure, not pixels: it can rewrite a PDF losslessly, cut pages
/// out and glue documents together, all without re-encoding anything.
fn build_qpdf(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    let options = &request.options;
    let input = request.sources[0].clone();
    let mut args: Vec<OsString> = Vec::new();

    match request.action.as_str() {
        "compress" => {
            // Recompress every stream and put the objects in object streams,
            // both lossless. Level 9 is zlib at its most thorough.
            args.push("--object-streams=generate".into());
            args.push("--recompress-flate".into());
            args.push("--compression-level=9".into());
            args.push("--compress-streams=y".into());
            args.push(input.into_os_string());
            args.push(scratch.into());
        }
        "strip_metadata" => {
            args.push("--remove-metadata".into());
            args.push(input.into_os_string());
            args.push(scratch.into());
        }
        "rotate" => {
            let degrees = opt_str(options, "rotate").unwrap_or_else(|| "90".to_owned());
            args.push(format!("--rotate={degrees}").into());
            args.push(input.into_os_string());
            args.push(scratch.into());
        }
        "extract_pages" => {
            // qpdf's page syntax is the same dialect the option documents, so
            // `1-3,7` is expressed exactly rather than approximated.
            let pages = opt_str(options, "pages")
                .filter(|pages| !pages.trim().is_empty())
                .unwrap_or_else(|| "1".to_owned());
            args.push(input.into_os_string());
            args.push("--pages".into());
            args.push(".".into());
            args.push(pages.into());
            args.push("--".into());
            args.push(scratch.into());
        }
        "merge" => {
            args.push("--empty".into());
            args.push("--pages".into());
            args.extend(request.sources.iter().map(OsString::from));
            args.push("--".into());
            args.push(scratch.into());
        }
        action => return Err(unsupported(action, "qpdf", "M5")),
    }

    Ok(CommandSpec::plain(
        request.program.clone(),
        args,
        ProgressMode::None,
    ))
}

// --------------------------------------------------------------- LibreOffice

/// `LibreOffice` converts by writing `<stem>.<ext>` into its `--outdir`, and no
/// flag makes it choose another name.
///
/// So the file is converted into a private directory and collected afterwards.
/// The same directory holds a throwaway profile, which also means two jobs can
/// run at once without fighting over one profile lock.
fn build_libreoffice(request: &JobRequest, scratch: &Path) -> CommandSpec {
    let source = request.sources[0].clone();
    let out_dir = sibling(scratch, "lo-out");
    let profile = sibling(scratch, "lo-profile");

    let stem = source.file_stem().map_or_else(
        || "orbit".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let name = format!("{stem}.{}", request.target_ext);

    let args: Vec<OsString> = vec![
        "--headless".into(),
        "--norestore".into(),
        "--nolockcheck".into(),
        "--nodefault".into(),
        format!("-env:UserInstallation={}", file_url(&profile)).into(),
        "--convert-to".into(),
        request.target_ext.as_str().into(),
        "--outdir".into(),
        out_dir.clone().into_os_string(),
        source.into_os_string(),
    ];

    let mut spec = CommandSpec::plain(request.program.clone(), args, ProgressMode::None);
    spec.work_dirs = vec![profile, out_dir.clone()];
    spec.collected = Some(Collected { dir: out_dir, name });
    spec
}

/// A `file://` URL for a local path, which is what `-env:UserInstallation`
/// takes. Spaces have to be escaped, or `LibreOffice` stops reading at the first
/// one; the extra slash gives Windows the `file:///C:/…` shape it wants.
fn file_url(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let text = absolute
        .to_string_lossy()
        .replace('\\', "/")
        .replace(' ', "%20");
    let prefix = if text.starts_with('/') {
        "file://"
    } else {
        "file:///"
    };
    format!("{prefix}{text}")
}

// -------------------------------------------------------------------- libheif

/// libheif is one small tool for one job: HEIC in, PNG or JPEG out. It takes
/// the output path verbatim, unlike the other prefix-based tools here.
fn build_libheif(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    if request.action != "convert" {
        return Err(unsupported(&request.action, "libheif", "M5"));
    }
    Ok(CommandSpec::plain(
        request.program.clone(),
        vec![request.sources[0].clone().into_os_string(), scratch.into()],
        ProgressMode::None,
    ))
}

// ------------------------------------------------------------------ tesseract

/// Tesseract takes a prefix like poppler does, and its output *config* decides
/// the extension: plain text by default, a searchable PDF when asked.
fn build_tesseract(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    if request.action != "ocr" {
        return Err(unsupported(&request.action, "tesseract", "M5"));
    }
    let options = &request.options;
    let mut args: Vec<OsString> = Vec::new();
    let language = opt_str(options, "ocr_lang").unwrap_or_else(|| "auto".to_owned());
    if language != "auto" && !language.is_empty() {
        push_flag(&mut args, "-l", &language);
    }
    args.push(request.sources[0].clone().into_os_string());
    args.push(without_extension(scratch).into_os_string());
    if opt_str(options, "ocr_output").as_deref() == Some("searchable_pdf") {
        args.push("pdf".into());
    }
    Ok(CommandSpec::plain(
        request.program.clone(),
        args,
        ProgressMode::None,
    ))
}

// ------------------------------------------------------------- orbit-native

/// The work Orbit does in Rust, with no sidecar at all.
///
/// This engine may only claim what [`crate::native`] actually implements, and
/// the registry says the same thing in `engines.json`. Anything else is a
/// milestone away, not a guess.
fn native(request: &JobRequest) -> Result<NativeOp, PlanError> {
    let source = request.sources[0].clone();
    let options = &request.options;
    let action = request.action.as_str();

    match (request.kind, action) {
        (Kind::Image, "convert" | "compress" | "strip_metadata") => {
            Ok(image_op(request, source, Transform::None))
        }
        (Kind::Image, "resize") => Ok(image_op(request, source, resize_transform(options))),
        (Kind::Image, "grayscale") => Ok(image_op(request, source, Transform::Grayscale)),
        (Kind::Image, "crop") => {
            let region = region_option(options, action)?;
            Ok(image_op(
                request,
                source,
                Transform::Crop {
                    x: region.x,
                    y: region.y,
                    width: region.width,
                    height: region.height,
                },
            ))
        }
        (Kind::Image, "rotate") => Ok(image_op(request, source, rotate_transform(options))),
        (Kind::Image, "flip") => Ok(image_op(request, source, flip_transform(options))),
        (Kind::Text, "convert") if request.target_ext == "html" => Ok(NativeOp::Markup { source }),
        (Kind::Data, "convert") if matches!(request.target_ext.as_str(), "csv" | "json") => {
            Ok(NativeOp::Tabular {
                source: source.clone(),
                target_ext: request.target_ext.clone(),
                delimiter: table_delimiter(&source, options),
                header: opt_bool(options, "header"),
            })
        }
        (_, action) => Err(unsupported(action, "orbit-native", "M5")),
    }
}

/// An image operation with the quality the options ask for.
fn image_op(request: &JobRequest, source: PathBuf, transform: Transform) -> NativeOp {
    NativeOp::Image {
        source,
        target_ext: request.target_ext.clone(),
        transform,
        quality: opt_i64(&request.options, "quality")
            .and_then(|quality| u8::try_from(quality.clamp(1, 100)).ok()),
        background: opt_str(&request.options, "background"),
    }
}

fn resize_transform(options: &Options) -> Transform {
    let mode = opt_str(options, "resize_mode").unwrap_or_else(|| "none".to_owned());
    match mode.as_str() {
        "percent" => Transform::Resize(Resize::Percent(
            u32::try_from(opt_i64(options, "percent").unwrap_or(50).clamp(1, 1000)).unwrap_or(50),
        )),
        "max_dim" => Transform::Resize(Resize::Fit(
            u32::try_from(
                opt_i64(options, "max_dim")
                    .unwrap_or(1920)
                    .clamp(1, 100_000),
            )
            .unwrap_or(1920),
        )),
        "preset" => {
            let preset = opt_str(options, "resize_preset").unwrap_or_else(|| "1080p".to_owned());
            let (width, height) = parse_box(target_box(&preset).unwrap_or("1920x1080"));
            Transform::Resize(Resize::Box { width, height })
        }
        _ => Transform::None,
    }
}

fn rotate_transform(options: &Options) -> Transform {
    let degrees = opt_i64(options, "rotate").unwrap_or(90);
    Transform::Rotate {
        degrees: match degrees {
            270 | -90 => 270,
            180 => 180,
            _ => 90,
        },
    }
}

fn flip_transform(options: &Options) -> Transform {
    Transform::Flip {
        vertical: opt_str(options, "flip_axis").as_deref() == Some("vertical"),
    }
}

/// The separator of a delimiter-separated source.
///
/// The extension wins over the option, because `csv`, `tsv` and `psv` share one
/// format in the registry: the option's comma default would read a tab-separated
/// file as a single column.
fn table_delimiter(source: &Path, options: &Options) -> char {
    match source
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("tsv") => '\t',
        Some("psv") => '|',
        _ => match opt_str(options, "delimiter").as_deref() {
            Some("tab") => '\t',
            Some(";") => ';',
            Some("|") => '|',
            _ => ',',
        },
    }
}

/// `1920x1080` as numbers.
fn parse_box(geometry: &str) -> (u32, u32) {
    let (width, height) = geometry.split_once('x').unwrap_or(("1920", "1080"));
    (
        width.parse().unwrap_or(1920),
        height.parse().unwrap_or(1080),
    )
}

// ------------------------------------------------------------------ option read

/// Read an option as text. Numbers and flags stringify, `null` reads as absent
/// so a cleared field behaves the same as one that was never set.
fn opt_str(options: &Options, key: &str) -> Option<String> {
    match options.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn opt_i64(options: &Options, key: &str) -> Option<i64> {
    match options.get(key)? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn opt_f64(options: &Options, key: &str) -> Option<f64> {
    match options.get(key)? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn opt_bool(options: &Options, key: &str) -> bool {
    matches!(options.get(key), Some(Value::Bool(true)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(engine: &str, action: &str, kind: Kind, options: Value) -> JobRequest {
        JobRequest {
            engine: engine.to_owned(),
            program: PathBuf::from("/usr/bin/engine"),
            kind,
            action: action.to_owned(),
            target_ext: "webp".to_owned(),
            tag: "converted".to_owned(),
            sources: vec![PathBuf::from("/tmp/orbit-test/source.png")],
            options: match options {
                Value::Object(map) => map.into_iter().collect(),
                _ => Options::new(),
            },
            output_dir: None,
            steps: Vec::new(),
        }
    }

    fn args_of(built: &CommandSpec) -> Vec<String> {
        built
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn magick_conversion_is_input_operators_output() {
        let built = build(
            &request("magick", "convert", Kind::Image, json!({ "quality": 82 })),
            Path::new("/tmp/orbit-test/out.webp"),
        )
        .unwrap();
        assert_eq!(
            args_of(&built),
            [
                "/tmp/orbit-test/source.png",
                "-quality",
                "82",
                "/tmp/orbit-test/out.webp"
            ]
        );
    }

    #[test]
    fn magick_resize_modes_map_to_geometry() {
        let max_dim = build(
            &request(
                "magick",
                "resize",
                Kind::Image,
                json!({ "resize_mode": "max_dim", "max_dim": 800 }),
            ),
            Path::new("/o.png"),
        )
        .unwrap();
        assert!(args_of(&max_dim).contains(&"800x800>".to_owned()));

        let percent = build(
            &request(
                "magick",
                "resize",
                Kind::Image,
                json!({ "resize_mode": "percent", "percent": 40 }),
            ),
            Path::new("/o.png"),
        )
        .unwrap();
        assert!(args_of(&percent).contains(&"40%".to_owned()));

        let preset = build(
            &request(
                "magick",
                "resize",
                Kind::Image,
                json!({ "resize_mode": "preset", "resize_preset": "720p" }),
            ),
            Path::new("/o.png"),
        )
        .unwrap();
        assert!(args_of(&preset).contains(&"1280x720>".to_owned()));
    }

    #[test]
    fn resizing_turned_off_adds_no_flags() {
        let built = build(
            &request(
                "magick",
                "resize",
                Kind::Image,
                json!({ "resize_mode": "none" }),
            ),
            Path::new("/o.png"),
        )
        .unwrap();
        assert_eq!(args_of(&built), ["/tmp/orbit-test/source.png", "/o.png"]);
    }

    #[test]
    fn crop_without_a_region_asks_for_one() {
        let error = build(
            &request("magick", "crop", Kind::Image, json!({})),
            Path::new("/o.png"),
        )
        .expect_err("region is required");
        assert!(matches!(
            error,
            PlanError::NeedsOption { ref option, .. } if option == "region"
        ));
    }

    #[test]
    fn ffmpeg_audio_conversion_drops_video_and_sets_the_bitrate() {
        let built = build(
            &request(
                "ffmpeg",
                "convert",
                Kind::Audio,
                json!({ "audio_bitrate": "320000", "channels": "mono" }),
            ),
            Path::new("/o.mp3"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.contains(&"-vn".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-b:a", "320000"]));
        assert!(args.windows(2).any(|pair| pair == ["-ac", "1"]));
        // Progress is requested machine-readably, and the output comes last.
        assert!(args.windows(2).any(|pair| pair == ["-progress", "pipe:1"]));
        assert_eq!(args.last().unwrap(), "/o.mp3");
    }

    #[test]
    fn ffmpeg_copy_codec_does_not_force_a_bitrate() {
        let built = build(
            &request(
                "ffmpeg",
                "convert",
                Kind::Audio,
                json!({ "audio_codec": "copy" }),
            ),
            Path::new("/o.mka"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.windows(2).any(|pair| pair == ["-c:a", "copy"]));
        assert!(!args.contains(&"-b:a".to_owned()), "copy has no bitrate");
    }

    #[test]
    fn ffmpeg_resolution_keeps_the_aspect_ratio() {
        let built = build(
            &request(
                "ffmpeg",
                "convert",
                Kind::Video,
                json!({ "codec": "h264", "crf": 20, "resolution": "720p" }),
            ),
            Path::new("/o.mp4"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.windows(2).any(|pair| pair == ["-c:v", "libx264"]));
        assert!(args.windows(2).any(|pair| pair == ["-crf", "20"]));
        let filter = args
            .iter()
            .position(|arg| arg == "-vf")
            .map(|i| &args[i + 1])
            .unwrap();
        assert!(filter.contains("scale=1280:720:force_original_aspect_ratio=decrease"));
    }

    #[test]
    fn ffmpeg_vp9_gets_the_zero_bitrate_that_crf_mode_needs() {
        let built = build(
            &request("ffmpeg", "convert", Kind::Video, json!({ "codec": "vp9" })),
            Path::new("/o.webm"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.windows(2).any(|pair| pair == ["-b:v", "0"]));
    }

    #[test]
    fn ffmpeg_trim_copies_streams_and_seeks_before_the_input() {
        let built = build(
            &request(
                "ffmpeg",
                "trim",
                Kind::Video,
                json!({ "start": "00:00:05", "end": "00:00:12" }),
            ),
            Path::new("/o.mp4"),
        )
        .unwrap();
        let args = args_of(&built);
        let seek = args.iter().position(|arg| arg == "-ss").unwrap();
        let input = args.iter().position(|arg| arg == "-i").unwrap();
        assert!(seek < input, "seeking before -i is the fast path");
        assert!(args.windows(2).any(|pair| pair == ["-c", "copy"]));
        assert!(args.windows(2).any(|pair| pair == ["-to", "00:00:12"]));
    }

    #[test]
    fn filters_are_comma_joined_into_a_single_vf_flag() {
        // ffmpeg accepts one `-vf` per output, so a chain must arrive as a
        // single comma-joined argument rather than a repeated flag.
        assert_eq!(join_filters(&[]), None);
        assert_eq!(
            join_filters(&["format=gray".to_owned()]),
            Some("format=gray".to_owned())
        );
        assert_eq!(
            join_filters(&["format=gray".to_owned(), "scale=854:480".to_owned()]),
            Some("format=gray,scale=854:480".to_owned())
        );
    }

    #[test]
    fn grayscale_emits_exactly_one_vf_flag() {
        let built = build(
            &request("ffmpeg", "grayscale", Kind::Video, json!({})),
            Path::new("/o.mp4"),
        )
        .unwrap();
        let args = args_of(&built);
        assert_eq!(args.iter().filter(|arg| *arg == "-vf").count(), 1);
        assert!(args.contains(&"format=gray".to_owned()));
    }

    #[test]
    fn strip_metadata_copies_streams_without_re_encoding() {
        let built = build(
            &request("ffmpeg", "strip_metadata", Kind::Video, json!({})),
            Path::new("/o.mp4"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.windows(2).any(|pair| pair == ["-map_metadata", "-1"]));
        assert!(args.windows(2).any(|pair| pair == ["-c", "copy"]));
    }

    #[test]
    fn extract_audio_defaults_to_a_sane_bitrate() {
        let built = build(
            &request("ffmpeg", "extract_audio", Kind::Video, json!({})),
            Path::new("/o.mp3"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.contains(&"-vn".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-b:a", "192000"]));
    }

    #[test]
    fn extract_frames_takes_one_frame_at_the_requested_time() {
        let built = build(
            &request(
                "ffmpeg",
                "extract_frames",
                Kind::Video,
                json!({ "time": 3 }),
            ),
            Path::new("/o.png"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.windows(2).any(|pair| pair == ["-ss", "3"]));
        // A run, not one still: the fps filter thins the stream, the cap keeps
        // a feature film from writing thousands of PNGs, and the output is a
        // pattern the image2 muxer numbers.
        assert!(args.iter().any(|arg| arg.starts_with("fps=")), "{args:?}");
        assert!(args.windows(2).any(|pair| pair == ["-frames:v", "100"]));
        assert_eq!(args.last().unwrap(), "/o-%04d.png");
    }

    #[test]
    fn vips_is_supported_for_conversions_only() {
        let copy = build(
            &request("vips", "convert", Kind::Image, json!({})),
            Path::new("/o.webp"),
        )
        .unwrap();
        assert_eq!(
            args_of(&copy),
            ["copy", "/tmp/orbit-test/source.png", "/o.webp"]
        );

        let error = build(
            &request("vips", "compress", Kind::Image, json!({})),
            Path::new("/o.webp"),
        )
        .expect_err("not driven yet");
        assert!(matches!(
            error,
            PlanError::NotYetSupported {
                milestone: "M5",
                ..
            }
        ));
    }

    #[test]
    fn undriven_engines_say_so_instead_of_guessing() {
        let error = build(
            &request("vips", "trim", Kind::Video, json!({})),
            Path::new("/o.mp4"),
        )
        .expect_err("must not invent a command line");
        assert!(matches!(error, PlanError::NotYetSupported { .. }));

        let native = backend(
            &request("orbit-native", "merge", Kind::Pdf, json!({})),
            Path::new("/o.pdf"),
        )
        .expect_err("the built-in engine does not merge PDFs");
        assert!(matches!(native, PlanError::NotYetSupported { .. }));
    }

    /// The same request, writing a different extension.
    fn targeting(mut request: JobRequest, ext: &str) -> JobRequest {
        request.target_ext = ext.to_owned();
        request
    }

    #[test]
    fn poppler_renders_one_page_and_lets_the_engine_name_the_file() {
        let built = build(
            &targeting(
                request(
                    "poppler",
                    "convert",
                    Kind::Pdf,
                    json!({ "dpi": "150", "pdf_page": 3, "quality": 80 }),
                ),
                "jpg",
            ),
            Path::new("/o/.orbit-1-1.jpg"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.contains(&"-jpeg".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-r", "150"]));
        assert!(args.windows(2).any(|pair| pair == ["-f", "3"]));
        // `-singlefile` plus a prefix without an extension is how the scratch
        // name survives: pdftoppm appends `.jpg` itself.
        assert!(args.contains(&"-singlefile".to_owned()));
        assert_eq!(args.last().unwrap(), "/o/.orbit-1-1");
        assert!(args.contains(&"quality=80".to_owned()));
    }

    #[test]
    fn poppler_refuses_a_page_list_it_cannot_expand() {
        let error = build(
            &targeting(
                request("poppler", "convert", Kind::Pdf, json!({ "pages": "1-3,7" })),
                "txt",
            ),
            Path::new("/o.txt"),
        )
        .expect_err("poppler takes contiguous ranges only");
        assert!(matches!(
            error,
            PlanError::ValueUnsupported { ref option, ref value, .. }
                if option == "pages" && value == "1-3,7"
        ));
    }

    #[test]
    fn poppler_reads_a_contiguous_range_in_every_spelling() {
        assert_eq!(contiguous_range("1-3"), Some((1, Some(3))));
        assert_eq!(contiguous_range("4"), Some((4, Some(4))));
        assert_eq!(contiguous_range("2-"), Some((2, None)));
        assert_eq!(contiguous_range("-5"), Some((1, Some(5))));
        assert_eq!(contiguous_range("1-3,7"), None);
    }

    #[test]
    fn qpdf_extracts_pages_with_the_full_page_syntax_intact() {
        let built = build(
            &request(
                "qpdf",
                "extract_pages",
                Kind::Pdf,
                json!({ "pages": "1-3,7" }),
            ),
            Path::new("/o.pdf"),
        )
        .unwrap();
        assert_eq!(
            args_of(&built),
            [
                "/tmp/orbit-test/source.png",
                "--pages",
                ".",
                "1-3,7",
                "--",
                "/o.pdf"
            ]
        );
    }

    #[test]
    fn qpdf_merges_by_listing_every_input() {
        let mut request = request("qpdf", "merge", Kind::Pdf, json!({}));
        request.sources = vec![PathBuf::from("/a.pdf"), PathBuf::from("/b.pdf")];
        let built = build(&request, Path::new("/o.pdf")).unwrap();
        assert_eq!(
            args_of(&built),
            ["--empty", "--pages", "/a.pdf", "/b.pdf", "--", "/o.pdf"]
        );
    }

    #[test]
    fn ghostscript_aims_at_a_size_only_when_that_is_what_was_asked_for() {
        let quality = build(
            &request(
                "ghostscript",
                "compress",
                Kind::Pdf,
                json!({ "compress_mode": "quality" }),
            ),
            Path::new("/o.pdf"),
        )
        .unwrap();
        assert!(args_of(&quality).contains(&"-dPDFSETTINGS=/prepress".to_owned()));

        let smaller = build(
            &request(
                "ghostscript",
                "compress",
                Kind::Pdf,
                json!({ "compress_mode": "target_size" }),
            ),
            Path::new("/o.pdf"),
        )
        .unwrap();
        assert!(args_of(&smaller).contains(&"-dPDFSETTINGS=/screen".to_owned()));
    }

    #[test]
    fn ghostscript_names_its_output_exactly() {
        let built = build(
            &targeting(
                request(
                    "ghostscript",
                    "convert",
                    Kind::Pdf,
                    json!({ "dpi": "72", "pdf_page": 2 }),
                ),
                "png",
            ),
            Path::new("/o/.orbit-9.png"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.contains(&"-sDEVICE=png16m".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-dFirstPage", "2"]));
        assert!(args.windows(2).any(|pair| pair == ["-dLastPage", "2"]));
        assert!(args.contains(&"-sOutputFile=/o/.orbit-9.png".to_owned()));
    }

    #[test]
    fn libreoffice_writes_into_a_private_directory_and_is_collected_after() {
        let built = build(
            &targeting(
                request("libreoffice", "convert", Kind::Pdf, json!({})),
                "docx",
            ),
            Path::new("/tmp/orbit-test/.orbit-3-1.docx"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.windows(2).any(|pair| pair == ["--convert-to", "docx"]));
        assert!(args.contains(&"--outdir".to_owned()));
        assert!(
            args.iter()
                .any(|arg| arg.contains("lo-out") && !arg.contains("lo-profile")),
            "the output directory is private: {args:?}"
        );
        assert!(
            args.iter()
                .any(|arg| arg.starts_with("-env:UserInstallation=file://")),
            "each job gets its own profile: {args:?}"
        );

        let collected = built.collected.expect("collected afterwards");
        assert_eq!(collected.name, "source.docx");
        assert_eq!(
            collected.dir,
            Path::new("/tmp/orbit-test/.orbit-3-1.lo-out")
        );
        assert_eq!(built.work_dirs.len(), 2, "profile and output directories");
    }

    #[test]
    fn a_space_in_a_path_is_escaped_for_libreoffices_profile_url() {
        assert_eq!(file_url(Path::new("/home/a b/x")), "file:///home/a%20b/x");
    }

    #[test]
    fn tesseract_takes_a_prefix_and_asks_for_the_config_it_needs() {
        let text = build(
            &request(
                "tesseract",
                "ocr",
                Kind::Image,
                json!({ "ocr_lang": "auto" }),
            ),
            Path::new("/o/.orbit-2-1.txt"),
        )
        .unwrap();
        let args = args_of(&text);
        assert!(
            !args.contains(&"-l".to_owned()),
            "auto means no language flag"
        );
        assert_eq!(args.last().unwrap(), "/o/.orbit-2-1");

        let searchable = build(
            &targeting(
                request(
                    "tesseract",
                    "ocr",
                    Kind::Image,
                    json!({ "ocr_lang": "eng", "ocr_output": "searchable_pdf" }),
                ),
                "pdf",
            ),
            Path::new("/o/.orbit-2-2.pdf"),
        )
        .unwrap();
        let args = args_of(&searchable);
        assert!(args.windows(2).any(|pair| pair == ["-l", "eng"]));
        assert_eq!(args.last().unwrap(), "pdf");
    }

    #[test]
    fn magick_addresses_pdf_pages_from_zero() {
        let built = build(
            &targeting(
                request(
                    "magick",
                    "convert",
                    Kind::Pdf,
                    json!({ "dpi": "300", "pdf_page": 4 }),
                ),
                "png",
            ),
            Path::new("/o.png"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.windows(2).any(|pair| pair == ["-density", "300"]));
        assert!(
            args.contains(&"/tmp/orbit-test/source.png[3]".to_owned()),
            "page four is index three: {args:?}"
        );
    }

    #[test]
    fn image_to_pdf_builds_a_page_before_it_places_the_image() {
        let built = build(
            &targeting(
                request(
                    "magick",
                    "convert",
                    Kind::Image,
                    json!({ "page": "a4", "margin": "2cm", "orientation": "auto" }),
                ),
                "pdf",
            ),
            Path::new("/o.pdf"),
        )
        .unwrap();
        let args = args_of(&built);
        // A4 is 595x842 points; a 2cm margin takes 57 from each side.
        assert!(args.windows(2).any(|pair| pair == ["-size", "595x842"]));
        assert!(args.contains(&"(".to_owned()) && args.contains(&")".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-resize", "481x728"]));
        assert!(args.windows(2).any(|pair| pair == ["-gravity", "center"]));
        assert!(args.contains(&"-composite".to_owned()));
    }

    #[test]
    fn page_boxes_answer_the_names_the_option_offers() {
        assert_eq!(page_box("a4", false), Some((595, 842)));
        assert_eq!(page_box("a4", true), Some((842, 595)));
        assert_eq!(page_box("letter", false), Some((612, 792)));
        assert_eq!(page_box("source", false), None);
        assert_eq!(margin_pixels(Some("2cm")), Some(57));
        assert_eq!(margin_pixels(Some("0cm")), Some(0));
        assert_eq!(margin_pixels(Some("1in")), Some(72));
        assert_eq!(margin_pixels(Some("nonsense")), None);
    }

    #[test]
    fn a_crop_region_is_read_in_both_spellings() {
        let documented = parse_region("10, 20, 100, 50").unwrap();
        let magick = parse_region("100x50+10+20").unwrap();
        assert_eq!(documented, magick);
        assert_eq!(magick_crop(&documented), "100x50+10+20");
        assert_eq!(parse_region("nonsense"), None);

        let built = build(
            &request(
                "magick",
                "crop",
                Kind::Image,
                json!({ "region": "10,20,100,50" }),
            ),
            Path::new("/o.png"),
        )
        .unwrap();
        assert!(args_of(&built).contains(&"100x50+10+20".to_owned()));
    }

    #[test]
    fn ffmpeg_merge_writes_the_list_the_demuxer_reads() {
        let mut request = request("ffmpeg", "merge", Kind::Video, json!({}));
        request.sources = vec![PathBuf::from("/a's.mp4"), PathBuf::from("/b.mp4")];
        let built = build(&request, Path::new("/o.mp4")).unwrap();

        let aux = built.aux_files.first().expect("a concat list");
        assert_eq!(aux.path, Path::new("/o.concat"));
        assert_eq!(aux.contents, "file '/a'\\''s.mp4'\nfile '/b.mp4'\n");
        assert!(
            args_of(&built)
                .windows(2)
                .any(|pair| pair == ["-c", "copy"])
        );
        assert!(
            args_of(&built)
                .windows(2)
                .any(|pair| pair == ["-safe", "0"])
        );
    }

    #[test]
    fn ffmpeg_builds_a_palette_for_a_gif() {
        let built = build(
            &targeting(
                request(
                    "ffmpeg",
                    "convert",
                    Kind::Video,
                    json!({ "fps": 12, "width": 320 }),
                ),
                "gif",
            ),
            Path::new("/o.gif"),
        )
        .unwrap();
        let args = args_of(&built);
        let graph = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| args[index + 1].clone())
            .expect("a filter graph");
        assert!(graph.contains("palettegen"), "got {graph}");
        assert!(graph.contains("paletteuse"));
        assert!(graph.contains("fps=12,scale=320:-2"));
        assert!(args.windows(2).any(|pair| pair == ["-loop", "0"]));
    }

    #[test]
    fn ffmpeg_loops_a_still_image_for_the_requested_length() {
        let built = build(
            &targeting(
                request(
                    "ffmpeg",
                    "convert",
                    Kind::Image,
                    json!({ "duration": 3, "fps": 24 }),
                ),
                "mp4",
            ),
            Path::new("/o.mp4"),
        )
        .unwrap();
        let args = args_of(&built);
        assert!(args.windows(2).any(|pair| pair == ["-loop", "1"]));
        assert!(args.windows(2).any(|pair| pair == ["-t", "3"]));
        assert!(args.windows(2).any(|pair| pair == ["-c:v", "libx264"]));
    }

    #[test]
    fn ffmpeg_draws_a_track_instead_of_refusing_to_make_a_video() {
        let built = build(
            &targeting(
                request(
                    "ffmpeg",
                    "convert",
                    Kind::Audio,
                    json!({ "visualize_mode": "spectrum", "size": "1280x720" }),
                ),
                "mp4",
            ),
            Path::new("/o.mp4"),
        )
        .unwrap();
        let args = args_of(&built);
        let graph = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| args[index + 1].clone())
            .expect("a filter graph");
        assert!(graph.contains("showspectrum=s=1280x720"), "got {graph}");
        assert!(graph.contains("overlay"));
        assert!(args.windows(2).any(|pair| pair == ["-map", "[out]"]));
        assert!(args.contains(&"-shortest".to_owned()));
    }

    #[test]
    fn the_built_in_engine_is_asked_for_what_it_declares() {
        let resize = backend(
            &request(
                "orbit-native",
                "resize",
                Kind::Image,
                json!({ "resize_mode": "percent", "percent": 25, "quality": 90 }),
            ),
            Path::new("/o.png"),
        )
        .unwrap();
        let Backend::Native(op) = resize else {
            panic!("expected the built-in engine, got {resize:?}");
        };
        assert_eq!(
            op,
            NativeOp::Image {
                source: PathBuf::from("/tmp/orbit-test/source.png"),
                target_ext: "webp".to_owned(),
                transform: Transform::Resize(Resize::Percent(25)),
                quality: Some(90),
                background: None,
            }
        );

        let markup = backend(
            &targeting(
                request("orbit-native", "convert", Kind::Text, json!({})),
                "html",
            ),
            Path::new("/o.html"),
        )
        .unwrap();
        let Backend::Native(op) = markup else {
            panic!("expected the built-in engine, got {markup:?}");
        };
        assert_eq!(
            op,
            NativeOp::Markup {
                source: PathBuf::from("/tmp/orbit-test/source.png")
            }
        );
    }

    #[test]
    fn a_tab_separated_file_keeps_its_own_separator() {
        let options: Options = json!({ "delimiter": "," })
            .as_object()
            .cloned()
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(table_delimiter(Path::new("/a/b.tsv"), &options), '\t');
        assert_eq!(table_delimiter(Path::new("/a/b.psv"), &options), '|');
        assert_eq!(table_delimiter(Path::new("/a/b.csv"), &options), ',');
    }

    #[test]
    fn a_scratch_sibling_stays_hidden_and_belongs_to_its_job() {
        assert_eq!(
            sibling(Path::new("/o/.orbit-7-2.png"), "concat"),
            Path::new("/o/.orbit-7-2.concat")
        );
        assert_eq!(
            without_extension(Path::new("/o/.orbit-7-2.png")),
            Path::new("/o/.orbit-7-2")
        );
    }

    #[test]
    fn a_request_with_no_files_is_rejected() {
        let mut request = request("magick", "convert", Kind::Image, json!({}));
        request.sources.clear();
        assert!(matches!(plan(&request), Err(PlanError::NoInput)));
    }

    #[test]
    fn null_options_read_as_absent() {
        // `end: null` is what the options panel sends for "run to the end".
        let options: Options = json!({ "end": null, "start": 0 })
            .as_object()
            .cloned()
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(opt_str(&options, "end"), None);
        assert_eq!(opt_str(&options, "start"), Some("0".to_owned()));
        assert!(!opt_bool(&options, "end"));
    }

    #[test]
    fn plan_reserves_the_output_and_builds_the_command() {
        let dir = std::env::temp_dir().join(format!("orbit-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("holiday.png"), b"x").unwrap();

        let mut request = request("magick", "convert", Kind::Image, json!({ "quality": 70 }));
        request.sources = vec![dir.join("holiday.png")];
        request.target_ext = "jpg".to_owned();

        let planned = plan(&request).unwrap();
        assert_eq!(
            planned.output.final_path().file_name().unwrap(),
            "holiday (converted).jpg"
        );
        assert!(planned.output.scratch_path().exists());
        assert!(matches!(
            planned.backend,
            Backend::Process(ref spec) if spec.progress == ProgressMode::None
        ));

        drop(planned);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
