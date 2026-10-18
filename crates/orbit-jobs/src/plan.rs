//! Turning a request into a command line.
//!
//! Everything here is pure: given a [`JobRequest`] this module returns the exact
//! arguments an engine should be run with, and creates nothing but the output
//! plan's scratch file. That keeps the interesting part — *what Orbit asks
//! `ffmpeg` or `ImageMagick` to do* — testable with plain assertions instead of
//! process spawning, which is where command-building bugs usually hide.
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
}

impl CommandSpec {
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

/// Everything the planner needs, already resolved against the registry.
#[derive(Debug, Clone)]
pub struct JobRequest {
    /// Engine id, which decides *how* the command is built.
    pub engine: String,
    /// Executable to run: the engine's primary binary or a found alternate.
    pub program: PathBuf,
    /// Family of the first source file.
    pub kind: Kind,
    /// `convert`, or an action id from `registry/actions.json`.
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
}

/// A request plus the plan it produced.
#[derive(Debug)]
pub struct Plan {
    /// The command to run.
    pub command: CommandSpec,
    /// The reserved destination and scratch file.
    pub output: OutputPlan,
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
    /// Called with no files at all.
    #[error("no input files")]
    NoInput,
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
    let Some(first) = request.sources.first() else {
        return Err(PlanError::NoInput);
    };

    let output = OutputPlan::reserve(
        first,
        &request.tag,
        &request.target_ext,
        request.output_dir.as_deref(),
    )?;

    let command = build(request, output.scratch_path())?;
    Ok(Plan { command, output })
}

/// Dispatch to the engine that was chosen for this job.
fn build(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
    match request.engine.as_str() {
        "ffmpeg" => build_ffmpeg(request, scratch),
        "magick" => build_magick(request, scratch),
        "vips" => build_vips(request, scratch),
        engine => Err(unsupported(&request.action, engine, "M3")),
    }
}

fn unsupported(action: &str, engine: &str, milestone: &'static str) -> PlanError {
    PlanError::NotYetSupported {
        action: action.to_owned(),
        engine: engine.to_owned(),
        milestone,
    }
}

// ---------------------------------------------------------------- ImageMagick

/// `ImageMagick` takes every job as a pipeline: inputs, operators, output.
fn build_magick(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
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
            let region = opt_str(options, "region").ok_or_else(|| PlanError::NeedsOption {
                action: action.to_owned(),
                option: "region".to_owned(),
            })?;
            args.push("-crop".into());
            args.push(region.into());
            args.push("+repage".into());
        }
        "merge" => {
            // Several inputs already queued: ImageMagick writes one document.
            args.push("-strip".into());
        }
        other => return Err(unsupported(other, "magick", "M3")),
    }

    args.push(scratch.into());
    Ok(CommandSpec {
        program: request.program.clone().into(),
        args,
        progress: ProgressMode::None,
        media_input: None,
    })
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
        return Err(unsupported(&request.action, "vips", "M3"));
    }
    Ok(CommandSpec {
        program: request.program.clone().into(),
        args: vec![
            "copy".into(),
            request.sources[0].clone().into_os_string(),
            scratch.into(),
        ],
        progress: ProgressMode::None,
        media_input: None,
    })
}

// --------------------------------------------------------------------- ffmpeg

fn build_ffmpeg(request: &JobRequest, scratch: &Path) -> Result<CommandSpec, PlanError> {
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

    args.push("-progress".into());
    args.push("pipe:1".into());
    args.push("-nostats".into());
    args.push(scratch.into());

    Ok(CommandSpec {
        program: request.program.clone().into(),
        args,
        progress: ProgressMode::FfmpegKeyValue,
        media_input: Some(input.clone()),
    })
}

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
            push_flag(args, "-frames:v", "1");
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
            let missing = || PlanError::NeedsOption {
                action: action.to_owned(),
                option: "region".to_owned(),
            };
            let region = opt_str(options, "region").ok_or_else(missing)?;
            filters.push(crop_filter(&region).ok_or_else(missing)?);
        }
        other => return Err(unsupported(other, "ffmpeg", "M3")),
    }
    Ok(())
}

/// `-flag value`, the shape nearly every ffmpeg output option takes.
fn push_flag(args: &mut Vec<OsString>, flag: &str, value: &str) {
    args.push(flag.into());
    args.push(value.into());
}

/// The panel speaks `ImageMagick`'s `WxH+X+Y`; ffmpeg wants `crop=W:H:X:Y`.
fn crop_filter(region: &str) -> Option<String> {
    let parts: Vec<&str> = region
        .split(['x', '+'])
        .filter(|part| !part.is_empty())
        .collect();
    match parts.as_slice() {
        [width, height, x, y] => Some(format!("crop={width}:{height}:{x}:{y}")),
        _ => None,
    }
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
        let encoder = match codec.as_str() {
            "h264" => "libx264",
            "h265" => "libx265",
            "vp9" => "libvpx-vp9",
            "av1" => "libsvtav1",
            "prores" => "prores_ks",
            other => other,
        };
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
        assert!(args.windows(2).any(|pair| pair == ["-frames:v", "1"]));
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
                milestone: "M3",
                ..
            }
        ));
    }

    #[test]
    fn undriven_engines_say_so_instead_of_guessing() {
        for engine in [
            "qpdf",
            "poppler",
            "ghostscript",
            "libreoffice",
            "orbit-native",
        ] {
            let error = build(
                &request(engine, "convert", Kind::Pdf, json!({})),
                Path::new("/o.pdf"),
            )
            .expect_err("must not invent a command line");
            assert!(
                matches!(error, PlanError::NotYetSupported { .. }),
                "{engine}"
            );
        }
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
        assert_eq!(planned.command.progress, ProgressMode::None);

        drop(planned);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
