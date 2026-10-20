//! Resolver tests against the bundled registry.
//!
//! The engine map is built by hand rather than detected, so these assert the
//! *decision* (which engine, which extension, which word) and do not change
//! meaning on a machine with a different set of tools installed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use orbit_jobs::{Options, Pick, PickKind, ResolveError, resolve_pick};
use orbit_registry::{EngineStatus, Progress, Registry};

/// A status map containing exactly the engines named, each with a binary.
fn engines(named: &[(&str, &str)]) -> BTreeMap<String, EngineStatus> {
    named
        .iter()
        .map(|(id, binary)| {
            (
                (*id).to_owned(),
                EngineStatus {
                    id: (*id).to_owned(),
                    available: true,
                    binary: Some(PathBuf::from(binary)),
                    reason: None,
                    progress: Progress::None,
                },
            )
        })
        .collect()
}

/// The usual case: `ImageMagick` and `FFmpeg` are installed.
fn installed() -> BTreeMap<String, EngineStatus> {
    engines(&[("magick", "/usr/bin/magick"), ("ffmpeg", "/usr/bin/ffmpeg")])
}

fn convert(to: &str) -> Pick {
    Pick {
        kind: PickKind::Conversion,
        reference: to.to_owned(),
        defaults: Options::default(),
    }
}

fn action(id: &str) -> Pick {
    Pick {
        kind: PickKind::Action,
        reference: id.to_owned(),
        defaults: Options::default(),
    }
}

fn registry() -> Registry {
    Registry::bundled().expect("the bundled registry loads")
}

/// Resolve one file against a set of installed engines.
fn run(pick: &Pick, path: &str, available: &BTreeMap<String, EngineStatus>) -> Vec<String> {
    let sources = vec![PathBuf::from(path)];
    let resolution = resolve_pick(&registry(), available, pick, &sources).expect("resolves");
    resolution
        .requests
        .iter()
        .map(|request| {
            format!(
                "{}|{}|{}|{}|{}",
                request.engine,
                request.action,
                request.target_ext,
                request.tag,
                request.program.display()
            )
        })
        .collect()
}

#[test]
fn a_format_petal_becomes_a_conversion_on_the_chosen_engine() {
    let resolved = run(&convert("webp"), "/tmp/orbit/holiday.png", &installed());
    assert_eq!(
        resolved,
        ["magick|convert|webp|converted|/usr/bin/magick"],
        "the tag comes from the convert action, the word and extension from the target"
    );
}

#[test]
fn a_conversion_uses_the_targets_canonical_extension() {
    // The JPEG format lists `jpg` first, and that is what a user expects in
    // the name — not `jpeg`, even though both are accepted on input.
    let resolved = run(&convert("jpg"), "/tmp/orbit/holiday.png", &installed());
    assert!(
        resolved[0].contains("|convert|jpg|converted|"),
        "{resolved:?}"
    );
}

#[test]
fn a_missing_engine_is_named_rather_than_guessed() {
    let resolution = resolve_pick(
        &registry(),
        &BTreeMap::new(),
        &convert("webp"),
        &[PathBuf::from("/tmp/orbit/holiday.png")],
    )
    .expect("resolves");

    assert!(resolution.is_empty());
    assert_eq!(resolution.total(), 1);
    let reason = resolution.first_reason().expect("a reason");
    assert!(reason.contains("not installed"), "{reason}");
    assert!(
        reason.contains("magick") || reason.contains("vips"),
        "{reason}"
    );
}

#[test]
fn an_action_keeps_the_inputs_extension() {
    let resolved = run(&action("compress"), "/tmp/orbit/holiday.png", &installed());
    assert_eq!(resolved, ["magick|compress|png|compressed|/usr/bin/magick"]);
}

#[test]
fn an_action_that_changes_the_format_says_which() {
    // "Audio from video" writes an audio file even though the input is video.
    let audio = run(
        &action("extract_audio"),
        "/tmp/orbit/clip.mp4",
        &installed(),
    );
    assert_eq!(audio, ["ffmpeg|extract_audio|mp3|audio|/usr/bin/ffmpeg"]);

    // And a still frame comes out as a PNG.
    let frame = run(
        &action("extract_frames"),
        "/tmp/orbit/clip.mp4",
        &installed(),
    );
    assert_eq!(frame, ["ffmpeg|extract_frames|png|frame|/usr/bin/ffmpeg"]);
}

#[test]
fn a_video_action_picks_ffmpeg_even_when_imagemagick_is_present() {
    let resolved = run(&action("grayscale"), "/tmp/orbit/clip.mp4", &installed());
    assert!(
        resolved[0].starts_with("ffmpeg|grayscale|mp4|grayscale|"),
        "{resolved:?}"
    );
}

#[test]
fn an_action_that_does_not_apply_to_the_file_is_skipped_with_a_reason() {
    // "Audio from video" on a still image: the ring would never offer it, but a
    // stale window could still ask.
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &action("extract_audio"),
        &[PathBuf::from("/tmp/orbit/holiday.png")],
    )
    .expect("resolves");

    assert!(resolution.is_empty());
    let reason = resolution.first_reason().expect("a reason");
    assert!(reason.contains("PNG"), "{reason}");
}

#[test]
fn a_batch_converts_the_files_it_can_and_reports_the_rest() {
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &convert("webp"),
        &[
            PathBuf::from("/tmp/orbit/holiday.png"),
            PathBuf::from("/tmp/orbit/logo.svg"),
            PathBuf::from("/tmp/orbit/tone.wav"),
        ],
    )
    .expect("resolves");

    // A WAV cannot become a WEBP, so it is left alone; the image is converted.
    assert_eq!(resolution.total(), 3);
    let names: Vec<String> = resolution
        .skipped
        .iter()
        .map(|skip| {
            skip.path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(names.contains(&"tone.wav".to_owned()), "{names:?}");
    assert!(
        resolution
            .requests
            .iter()
            .any(|request| request.sources[0].ends_with("holiday.png"))
    );
}

#[test]
fn every_request_names_one_source() {
    // One job per file is what makes "output beside the source, never
    // overwriting" true for a batch.
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &convert("jpg"),
        &[
            PathBuf::from("/tmp/orbit/one.png"),
            PathBuf::from("/tmp/orbit/two.png"),
        ],
    )
    .expect("resolves");

    assert_eq!(resolution.requests.len(), 2);
    assert!(resolution.requests.iter().all(|r| r.sources.len() == 1));
    assert!(resolution.skipped.is_empty());
}

#[test]
fn a_merge_becomes_one_job_holding_every_source() {
    // The whole point of a merge is one result, so it is a single job with many
    // sources rather than one job per file.
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &action("merge"),
        &[
            PathBuf::from("/tmp/orbit/a.png"),
            PathBuf::from("/tmp/orbit/b.png"),
        ],
    )
    .expect("resolves");

    assert_eq!(resolution.requests.len(), 1);
    assert_eq!(resolution.requests[0].sources.len(), 2);
    assert_eq!(resolution.requests[0].target_ext, "pdf");
    assert_eq!(resolution.requests[0].tag, "merged");
    assert!(resolution.skipped.is_empty(), "{:?}", resolution.skipped);
}

#[test]
fn a_merged_result_can_be_a_gif_instead() {
    let pick = Pick {
        kind: PickKind::Action,
        reference: "merge".to_owned(),
        defaults: Options::from_iter([("merge_output".to_owned(), serde_json::json!("gif"))]),
    };
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &pick,
        &[
            PathBuf::from("/tmp/orbit/a.png"),
            PathBuf::from("/tmp/orbit/b.png"),
        ],
    )
    .expect("resolves");

    assert_eq!(resolution.requests[0].target_ext, "gif");
}

#[test]
fn a_result_the_family_cannot_use_falls_back_to_its_own() {
    // One option catalog serves every family, so the panel offers "audio" for a
    // batch of pictures as well. Choosing it means the family default, not a
    // run that dies inside the engine.
    let pick = Pick {
        kind: PickKind::Action,
        reference: "merge".to_owned(),
        defaults: Options::from_iter([("merge_output".to_owned(), serde_json::json!("audio"))]),
    };
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &pick,
        &[
            PathBuf::from("/tmp/orbit/a.png"),
            PathBuf::from("/tmp/orbit/b.png"),
        ],
    )
    .expect("resolves");

    assert_eq!(resolution.requests[0].target_ext, "pdf");
}

#[test]
fn a_merge_leaves_a_file_of_another_family_alone() {
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &action("merge"),
        &[
            PathBuf::from("/tmp/orbit/a.png"),
            PathBuf::from("/tmp/orbit/b.png"),
            PathBuf::from("/tmp/orbit/tone.wav"),
        ],
    )
    .expect("resolves");

    assert_eq!(resolution.requests.len(), 1);
    assert_eq!(resolution.requests[0].sources.len(), 2);
    assert_eq!(resolution.skipped.len(), 1);
    assert!(
        resolution.skipped[0].reason.contains("image"),
        "{}",
        resolution.skipped[0].reason
    );
}

#[test]
fn a_merge_of_one_file_is_refused_with_a_reason() {
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &action("merge"),
        &[PathBuf::from("/tmp/orbit/a.png")],
    )
    .expect("resolves");

    assert!(resolution.is_empty());
    assert!(
        resolution.first_reason().unwrap().contains("at least 2"),
        "{:?}",
        resolution.skipped
    );
}

#[test]
fn merge_order_by_name_ignores_the_drag_order() {
    let pick = Pick {
        kind: PickKind::Action,
        reference: "merge".to_owned(),
        defaults: Options::from_iter([("merge_order".to_owned(), serde_json::json!("name"))]),
    };
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &pick,
        &[
            PathBuf::from("/tmp/orbit/b.png"),
            PathBuf::from("/tmp/orbit/a.png"),
        ],
    )
    .expect("resolves");

    let first = resolution.requests[0].sources[0].file_name().unwrap();
    assert_eq!(first, "a.png");
}

#[test]
fn a_merge_names_an_engine_it_needs_rather_than_using_the_wrong_one() {
    // Only `ImageMagick` is installed, and it cannot merge PDFs: the job is
    // reported as needing qpdf or Ghostscript, not handed to magick to fail.
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &action("merge"),
        &[
            PathBuf::from("/tmp/orbit/one.pdf"),
            PathBuf::from("/tmp/orbit/two.pdf"),
        ],
    )
    .expect("resolves");

    assert!(resolution.is_empty());
    let reason = resolution.first_reason().expect("a reason");
    assert!(reason.contains("not installed"), "{reason}");
}

#[test]
fn a_pdf_merge_uses_qpdf_when_it_is_there() {
    let resolution = resolve_pick(
        &registry(),
        &engines(&[("qpdf", "/usr/bin/qpdf")]),
        &action("merge"),
        &[
            PathBuf::from("/tmp/orbit/one.pdf"),
            PathBuf::from("/tmp/orbit/two.pdf"),
        ],
    )
    .expect("resolves");

    assert_eq!(resolution.requests[0].engine, "qpdf");
    assert_eq!(resolution.requests[0].target_ext, "pdf");
}

#[test]
fn an_unknown_action_is_refused() {
    let error = resolve_pick(&registry(), &installed(), &action("time-travel"), &[]).unwrap_err();
    assert_eq!(error, ResolveError::UnknownAction("time-travel".to_owned()));
}

#[test]
fn an_unknown_target_format_is_refused() {
    let error = resolve_pick(&registry(), &installed(), &convert("xyzzy"), &[]).unwrap_err();
    assert_eq!(error, ResolveError::UnknownFormat("xyzzy".to_owned()));
}

#[test]
fn a_ring_control_is_not_a_job() {
    let pick = Pick {
        kind: PickKind::Operator,
        reference: "search".to_owned(),
        defaults: Options::default(),
    };
    let error = resolve_pick(&registry(), &installed(), &pick, &[]).unwrap_err();
    assert_eq!(error, ResolveError::NotAJob("search".to_owned()));
}

#[test]
fn a_preset_is_a_chain_of_resolved_steps() {
    let pick = Pick {
        kind: PickKind::Preset,
        reference: "web-ready".to_owned(),
        defaults: Options::default(),
    };
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &pick,
        &[PathBuf::from("/tmp/orbit/holiday.png")],
    )
    .expect("resolves");
    assert_eq!(resolution.requests.len(), 1);
    assert_eq!(resolution.requests[0].steps.len(), 3);
}

#[test]
fn the_pick_kind_parses_what_the_webview_sends() {
    assert_eq!(
        PickKind::from_wire("conversion"),
        Some(PickKind::Conversion)
    );
    assert_eq!(PickKind::from_wire("action"), Some(PickKind::Action));
    assert_eq!(PickKind::from_wire("preset"), Some(PickKind::Preset));
    assert_eq!(PickKind::from_wire("operator"), Some(PickKind::Operator));
    assert_eq!(PickKind::from_wire("Conversion"), None);
}

#[test]
fn the_options_the_ring_resolved_travel_with_the_job() {
    // Values already chosen for a plain drop must reach the command line.
    let pick = Pick {
        kind: PickKind::Conversion,
        reference: "webp".to_owned(),
        defaults: Options::from_iter([("quality".to_owned(), serde_json::json!(55))]),
    };
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &pick,
        &[PathBuf::from("/tmp/orbit/holiday.png")],
    )
    .expect("resolves");

    let quality = resolution.requests[0].options.get("quality");
    assert_eq!(quality, Some(&serde_json::json!(55)));
}

#[test]
fn a_preset_becomes_one_job_with_every_step_resolved() {
    let pick = Pick {
        kind: PickKind::Preset,
        reference: "web-ready".to_owned(),
        defaults: Options::default(),
    };
    let resolution = resolve_pick(
        &registry(),
        &installed(),
        &pick,
        &[PathBuf::from("/tmp/orbit/holiday.png")],
    )
    .expect("resolves");

    assert_eq!(resolution.requests.len(), 1, "one request per dragged file");
    let request = &resolution.requests[0];
    assert_eq!(request.action, "chain");
    assert_eq!(
        request.target_ext, "webp",
        "the last step decides the result"
    );
    assert_eq!(request.tag, "web-ready", "the preset names the output");
    assert_eq!(request.steps.len(), 3);
    assert_eq!(request.steps[0].action, "resize");
    assert_eq!(request.steps[1].action, "convert");
    assert_eq!(request.steps[1].target_ext, "webp");
    assert_eq!(request.steps[2].action, "strip_metadata");
    assert!(
        request.steps.iter().all(|step| step.engine == "magick"),
        "the one installed engine carries every step"
    );
    // The intermediate steps keep the format, so the last one is the only
    // place the extension changes.
    assert_eq!(request.steps[0].target_ext, "png");
}

#[test]
fn a_preset_that_needs_a_missing_engine_is_refused_with_a_reason() {
    let pick = Pick {
        kind: PickKind::Preset,
        reference: "web-ready".to_owned(),
        defaults: Options::default(),
    };
    let resolution = resolve_pick(
        &registry(),
        &engines(&[("ffmpeg", "/usr/bin/ffmpeg")]),
        &pick,
        &[PathBuf::from("/tmp/orbit/holiday.png")],
    )
    .expect("the pick itself is valid");

    assert!(resolution.is_empty());
    let reason = resolution.first_reason().unwrap_or_default();
    assert!(
        reason.contains("Web ready"),
        "the reason names the preset: {reason}"
    );
}
