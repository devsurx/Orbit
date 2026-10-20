//! End-to-end runs against the real engines.
//!
//! The unit tests assert the argv Orbit *builds*; these assert what the actual
//! tools do with it, which is the only way to catch a flag ffmpeg rejects, a
//! muxer that dislikes an extension, or a filter with the wrong arity.
//!
//! Each test skips itself (loudly) when its engine is not installed, so the
//! suite stays useful on a machine without them rather than failing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use orbit_jobs::{Job, JobEvent, JobRequest, JobState, Options, Runner};
use orbit_registry::Kind;
use serde_json::json;

/// Long enough that ffmpeg emits several progress reports.
const TONE_SECONDS: &str = "4";

/// Whether a tool answers on `PATH`.
fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A throwaway directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("orbit-e2e-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A real PNG, made by the tool itself so no binary fixture lives in the repo.
fn make_png(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("magick")
        .args(["-size", "64x48", "gradient:red-blue"])
        .arg(&path)
        .status()
        .expect("run magick to build a fixture");
    assert!(status.success(), "could not build the PNG fixture");
    path
}

/// A real WAV tone, for the same reason.
fn make_wav(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={TONE_SECONDS}"),
        ])
        .arg(&path)
        .status()
        .expect("run ffmpeg to build a fixture");
    assert!(status.success(), "could not build the WAV fixture");
    path
}

/// A shell script that prints ffmpeg progress lines and then "encodes".
///
/// Pairing it with a *real* WAV means the runner's own `ffprobe` duration probe
/// runs for real, so the percentages below are the ones a user would see.
fn progress_stub(dir: &Path) -> PathBuf {
    let path = dir.join("stub-engine.sh");
    fs::write(
        &path,
        "#!/bin/sh\n\
         printf 'out_time_us=1000000\\n'\n\
         printf 'progress=continue\\n'\n\
         sleep 0.4\n\
         printf 'out_time_us=2000000\\n'\n\
         for last; do :; done\n\
         printf 'encoded' > \"$last\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }
    path
}

/// Run one job and collect every event, stopping at the terminal state.
fn run(request: JobRequest) -> Vec<JobEvent> {
    let (tx, rx) = mpsc::channel();
    let mut runner = Runner::new(1, move |event| {
        let _ = tx.send(event);
    });
    assert!(
        runner.submit(Job {
            id: 1,
            label: "Convert".to_owned(),
            request,
        }),
        "the pool accepted the job"
    );

    let mut events = Vec::new();
    while let Ok(event) = rx.recv_timeout(Duration::from_secs(120)) {
        let finished = !matches!(event.state, JobState::Queued | JobState::Running { .. });
        events.push(event);
        if finished {
            break;
        }
    }
    runner.shutdown();
    events
}

fn done_path(events: &[JobEvent]) -> PathBuf {
    match &events.last().expect("at least one event").state {
        JobState::Done { path } => path.clone(),
        other => panic!("expected a finished job, got {other:?}"),
    }
}

/// Options from a JSON literal, the way the shell would hand them over.
fn options(value: serde_json::Value) -> Options {
    match value {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => Options::new(),
    }
}

fn image_request(source: &Path, target_ext: &str, opts: serde_json::Value) -> JobRequest {
    JobRequest {
        engine: "magick".to_owned(),
        program: PathBuf::from("magick"),
        kind: Kind::Image,
        action: "convert".to_owned(),
        target_ext: target_ext.to_owned(),
        tag: "converted".to_owned(),
        sources: vec![source.to_path_buf()],
        options: options(opts),
        output_dir: None,
        steps: Vec::new(),
    }
}

/// A real CSV, written rather than generated: the point is the parsing.
fn make_csv(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, "name,size\norbit,42\n\"with, comma\",7\n").unwrap();
    path
}

/// A real Markdown note.
fn make_markdown(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(
        &path,
        "# Orbit\n\nConverts things, *quickly*.\n\n- one\n- two\n\n[site](https://example.com)\n",
    )
    .unwrap();
    path
}

/// A PDF with the given number of pages, built by `ImageMagick`.
fn make_pdf(dir: &Path, name: &str, pages: usize) -> PathBuf {
    let path = dir.join(name);
    let mut command = Command::new("magick");
    command.arg("-size").arg("200x300");
    for page in 0..pages {
        if page > 0 {
            command.arg("-size").arg("200x300");
        }
        command.arg(if page % 2 == 0 {
            "xc:white"
        } else {
            "xc:black"
        });
    }
    let status = command.arg(&path).status().expect("build a PDF fixture");
    assert!(status.success(), "could not build the PDF fixture");
    path
}

/// A PNG with a word printed on it, for OCR.
fn make_text_png(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("magick")
        .args([
            "-size",
            "480x140",
            "xc:white",
            "-pointsize",
            "64",
            "-fill",
            "black",
        ])
        .args(["-annotate", "+28+92", "ORBIT"])
        .arg(&path)
        .status()
        .expect("run magick to build a text fixture");
    assert!(status.success(), "could not build the text PNG fixture");
    path
}

/// What `pdfinfo` says, as `key: value` lines, or None when it is not installed.
fn pdf_info(path: &Path) -> Option<String> {
    if !have("pdfinfo") {
        return None;
    }
    let output = Command::new("pdfinfo").arg(path).output().ok()?;
    assert!(
        output.status.success(),
        "pdfinfo failed on {}",
        path.display()
    );
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The page count of a PDF, from `pdfinfo`.
fn page_count(path: &Path) -> usize {
    let info = pdf_info(path).expect("pdfinfo is installed (checked by the caller)");
    info.lines()
        .find_map(|line| line.strip_prefix("Pages:"))
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or_else(|| panic!("no page count in:\n{info}"))
}

#[test]
fn magick_really_converts_a_png_to_webp() {
    if !have("magick") {
        eprintln!("skipping: ImageMagick is not installed");
        return;
    }
    let dir = TempDir::new("magick-webp");
    let source = make_png(dir.path(), "holiday.png");

    let events = run(image_request(&source, "webp", json!({ "quality": 80 })));
    let published = done_path(&events);

    assert_eq!(
        published.file_name().unwrap(),
        "holiday (converted).webp",
        "the documented naming rule"
    );
    let bytes = fs::read(&published).unwrap();
    assert!(bytes.len() > 12, "a real image came out");
    // RIFF....WEBP
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");

    // The source is untouched and no scratch file survived.
    assert!(source.exists());
    let leftovers: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".orbit-"))
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

#[test]
fn ffmpeg_really_converts_a_wav_to_mp3_without_overwriting() {
    if !have("ffmpeg") {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let dir = TempDir::new("ffmpeg-mp3");
    let source = make_wav(dir.path(), "tone.wav");

    let request = JobRequest {
        engine: "ffmpeg".to_owned(),
        program: PathBuf::from("ffmpeg"),
        kind: Kind::Audio,
        action: "convert".to_owned(),
        target_ext: "mp3".to_owned(),
        tag: "converted".to_owned(),
        sources: vec![source.clone()],
        options: options(json!({ "audio_bitrate": "192000" })),
        output_dir: None,
        steps: Vec::new(),
    };

    let first = done_path(&run(request.clone()));
    assert_eq!(first.file_name().unwrap(), "tone (converted).mp3");
    let bytes = fs::read(&first).unwrap();
    assert!(bytes.len() > 1000, "a real audio file came out");
    // Either an ID3 tag or a bare MPEG frame sync.
    let looks_like_mp3 =
        bytes.starts_with(b"ID3") || (bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0);
    assert!(looks_like_mp3, "the output is not MP3");

    // Running it again must add a file, never replace the first.
    let second = done_path(&run(request));
    assert_eq!(second.file_name().unwrap(), "tone (converted 2).mp3");
    assert_eq!(
        fs::read(&first).unwrap(),
        bytes,
        "the first result is intact"
    );
}

#[test]
fn conversion_progress_is_reported_as_percentages() {
    if !have("ffmpeg") {
        eprintln!("skipping: ffmpeg is not installed (needed for the duration probe)");
        return;
    }
    let dir = TempDir::new("progress");
    let source = make_wav(dir.path(), "tone.wav");

    let request = JobRequest {
        engine: "ffmpeg".to_owned(),
        program: progress_stub(dir.path()),
        kind: Kind::Audio,
        action: "convert".to_owned(),
        target_ext: "mp3".to_owned(),
        tag: "converted".to_owned(),
        sources: vec![source.clone()],
        options: options(json!({})),
        output_dir: None,
        steps: Vec::new(),
    };

    let events = run(request);
    assert!(
        matches!(events.last().unwrap().state, JobState::Done { .. }),
        "the stub should have finished: {:?}",
        events.last()
    );

    let percents: Vec<f64> = events
        .iter()
        .filter_map(|event| match event.state {
            JobState::Running {
                percent: Some(percent),
            } => Some(percent),
            _ => None,
        })
        .collect();

    // The stub reports 1s and 2s of a four-second file.
    assert!(
        percents.contains(&25.0) && percents.contains(&50.0),
        "expected 25% and 50% among {percents:?}"
    );
    assert!(
        percents.windows(2).all(|pair| pair[1] >= pair[0]),
        "progress must never go backwards: {percents:?}"
    );
    assert!(
        percents
            .iter()
            .all(|percent| (0.0..=100.0).contains(percent))
    );
}

#[test]
fn a_missing_engine_fails_with_a_readable_reason() {
    // No skip here: this must hold on every machine.
    let dir = TempDir::new("no-engine");
    // The content is irrelevant: this request fails while planning, before any
    // engine is started, so no fixture tooling is needed.
    let source = dir.path().join("holiday.png");
    fs::write(&source, b"not really a png").unwrap();

    let events = run(JobRequest {
        // libvips is driven for conversions only: asking it to trim is the
        // shape of request the planner has to refuse rather than guess at.
        engine: "vips".to_owned(),
        program: PathBuf::from("vips"),
        kind: Kind::Video,
        action: "trim".to_owned(),
        target_ext: "mp4".to_owned(),
        tag: "trimmed".to_owned(),
        sources: vec![source],
        options: options(json!({})),
        output_dir: None,
        steps: Vec::new(),
    });

    match &events.last().unwrap().state {
        JobState::Failed { message } => {
            assert!(
                message.contains("M5"),
                "should name the milestone: {message}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    // Nothing was written, not even a scratch file.
    let entries: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, ["holiday.png"], "only the source remains");
}

// --------------------------------------------------------------------------
// M3: the engines and shapes added after the first milestone.
// --------------------------------------------------------------------------

/// A request against a named engine.
///
/// The program is the engine's own name: for every engine these tests drive, the
/// binary and the registry id are spelled the same way.
fn engine_request(
    engine: &str,
    action: &str,
    kind: Kind,
    target_ext: &str,
    tag: &str,
    sources: Vec<PathBuf>,
    opts: serde_json::Value,
) -> JobRequest {
    JobRequest {
        engine: engine.to_owned(),
        program: PathBuf::from(engine),
        kind,
        action: action.to_owned(),
        target_ext: target_ext.to_owned(),
        tag: tag.to_owned(),
        sources,
        options: options(opts),
        output_dir: None,
        steps: Vec::new(),
    }
}

#[test]
fn qpdf_really_merges_pdfs_into_one() {
    if !have("qpdf") || !have("magick") || !have("pdfinfo") {
        eprintln!("skipping: qpdf, ImageMagick and pdfinfo are needed for this one");
        return;
    }
    let dir = TempDir::new("qpdf-merge");
    let first = make_pdf(dir.path(), "one.pdf", 2);
    let second = make_pdf(dir.path(), "two.pdf", 1);

    // The merge is one job with many sources, which is the whole point of it.
    let events = run(engine_request(
        "qpdf",
        "merge",
        Kind::Pdf,
        "pdf",
        "merged",
        vec![first.clone(), second.clone()],
        json!({ "merge_output": "pdf", "merge_order": "as_dragged" }),
    ));
    let published = done_path(&events);

    assert_eq!(
        published.file_name().unwrap(),
        "one (merged).pdf",
        "the result is named after the file it was dragged from first"
    );
    assert_eq!(page_count(&published), 3, "2 + 1 pages");
    assert!(first.exists() && second.exists(), "sources are untouched");
}

#[test]
fn qpdf_really_keeps_only_the_pages_asked_for() {
    if !have("qpdf") || !have("magick") || !have("pdfinfo") {
        eprintln!("skipping: qpdf, ImageMagick and pdfinfo are needed for this one");
        return;
    }
    let dir = TempDir::new("qpdf-pages");
    let source = make_pdf(dir.path(), "report.pdf", 4);

    let events = run(engine_request(
        "qpdf",
        "extract_pages",
        Kind::Pdf,
        "pdf",
        "pages",
        vec![source],
        json!({ "pages": "2-3" }),
    ));
    let published = done_path(&events);

    assert_eq!(published.file_name().unwrap(), "report (pages).pdf");
    assert_eq!(page_count(&published), 2, "the range, and nothing else");
}

#[test]
fn qpdf_really_takes_the_metadata_out() {
    if !have("qpdf") || !have("magick") || !have("pdfinfo") {
        eprintln!("skipping: qpdf, ImageMagick and pdfinfo are needed for this one");
        return;
    }
    let dir = TempDir::new("qpdf-clean");
    let source = make_pdf(dir.path(), "report.pdf", 1);
    let before = pdf_info(&source).expect("readable");
    assert!(
        before.contains("Producer:"),
        "the fixture should carry metadata to remove:\n{before}"
    );

    let events = run(engine_request(
        "qpdf",
        "strip_metadata",
        Kind::Pdf,
        "pdf",
        "clean",
        vec![source],
        json!({}),
    ));
    let published = done_path(&events);

    assert_eq!(published.file_name().unwrap(), "report (clean).pdf");
    let after = pdf_info(&published).expect("readable");
    assert!(
        !after.contains("Producer:"),
        "metadata should be gone:\n{after}"
    );
    assert_eq!(page_count(&published), 1, "the page itself survives");
}

#[test]
fn tesseract_really_reads_a_word_from_a_png() {
    if !have("tesseract") {
        eprintln!("skipping: tesseract is not installed (set PATH/TESSDATA_PREFIX)");
        return;
    }
    if !have("magick") {
        eprintln!("skipping: ImageMagick is needed for the text fixture");
        return;
    }
    let dir = TempDir::new("tesseract-ocr");
    let source = make_text_png(dir.path(), "sign.png");

    let events = run(engine_request(
        "tesseract",
        "ocr",
        Kind::Image,
        "txt",
        "text",
        vec![source],
        json!({ "ocr_output": "txt" }),
    ));
    let published = done_path(&events);

    assert_eq!(published.file_name().unwrap(), "sign (text).txt");
    let text = fs::read_to_string(&published).unwrap();
    assert!(
        text.to_uppercase().contains("ORBIT"),
        "tesseract should have read the word back: {text:?}"
    );
}

#[test]
fn magick_really_lays_a_png_onto_an_a4_page() {
    if !have("magick") || !have("pdfinfo") {
        eprintln!("skipping: ImageMagick and pdfinfo are needed for this one");
        return;
    }
    let dir = TempDir::new("magick-a4");
    let source = make_png(dir.path(), "holiday.png");

    let events = run(engine_request(
        "magick",
        "convert",
        Kind::Image,
        "pdf",
        "converted",
        vec![source],
        json!({ "page": "a4", "margin": "0cm", "orientation": "auto", "quality": 85 }),
    ));
    let published = done_path(&events);

    assert_eq!(published.file_name().unwrap(), "holiday (converted).pdf");
    let info = pdf_info(&published).unwrap();
    // A4 is 595 x 842 points; a page that stretched to the image's 64 x 48
    // pixels would say so here.
    assert!(
        info.contains("595") && info.contains("842"),
        "the page should be A4:\n{info}"
    );
}

#[test]
fn the_native_engine_really_writes_a_jpeg() {
    if !have("magick") {
        eprintln!("skipping: ImageMagick is needed for the BMP fixture");
        return;
    }
    let dir = TempDir::new("native-jpeg");
    // A BMP source, because that is the format the built-in engine had to learn
    // to read and write itself.
    let source = dir.path().join("flat.bmp");
    let status = Command::new("magick")
        .args(["-size", "32x24", "gradient:red-blue"])
        .arg(&source)
        .status()
        .expect("run magick to build the BMP fixture");
    assert!(status.success(), "could not build the BMP fixture");

    let events = run(engine_request(
        "orbit-native",
        "convert",
        Kind::Image,
        "jpg",
        "converted",
        vec![source],
        json!({ "quality": 80 }),
    ));
    let published = done_path(&events);

    assert_eq!(published.file_name().unwrap(), "flat (converted).jpg");
    let bytes = fs::read(&published).unwrap();
    assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "a real JPEG start-of-image");
    assert!(bytes.len() > 100, "the pixels made it into the file");
}

#[test]
fn the_native_engine_really_turns_markdown_into_html() {
    let dir = TempDir::new("native-html");
    let source = make_markdown(dir.path(), "notes.md");

    let events = run(engine_request(
        "orbit-native",
        "convert",
        Kind::Text,
        "html",
        "converted",
        vec![source],
        json!({}),
    ));
    let published = done_path(&events);

    assert_eq!(published.file_name().unwrap(), "notes (converted).html");
    let html = fs::read_to_string(&published).unwrap();
    assert!(html.contains("<h1>Orbit</h1>"), "{html}");
    assert!(html.contains("<li>one</li>"), "{html}");
    assert!(
        html.contains("<a href=\"https://example.com\">site</a>"),
        "{html}"
    );
}

#[test]
fn the_native_engine_really_turns_csv_into_json() {
    let dir = TempDir::new("native-json");
    let source = make_csv(dir.path(), "parts.csv");

    let events = run(engine_request(
        "orbit-native",
        "convert",
        Kind::Data,
        "json",
        "converted",
        vec![source],
        // The resolver fills this from the petal's defaults; a raw request has
        // to say it, which is exactly the contract the runner documents.
        json!({ "delimiter": ",", "header": true }),
    ));
    let published = done_path(&events);

    assert_eq!(published.file_name().unwrap(), "parts (converted).json");
    let text = fs::read_to_string(&published).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON came out");
    let rows = parsed.as_array().expect("an array of rows");
    assert_eq!(rows.len(), 2, "two records, header not counted");
    assert_eq!(rows[0]["name"], "orbit");
    assert_eq!(rows[0]["size"], 42, "a number stays a number");
    assert_eq!(
        rows[1]["name"], "with, comma",
        "quoting survives the round trip"
    );
}
