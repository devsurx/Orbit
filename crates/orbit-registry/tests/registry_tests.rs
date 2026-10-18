//! Registry and menu-model tests.
//!
//! These are the tests that guard the core claim of the design: the menu is
//! generated from data, it is context-aware, it never lies about what is
//! installed, and it stays two levels deep.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use orbit_registry::{
    DEFAULT_MAX_PETALS, Engine, Kind, MenuRequest, PetalKind, Probe, Registry, RegistryDocs,
    Selector, SourceFile, bundled_docs, extension_of, host_platform,
};

fn registry() -> Registry {
    Registry::bundled().expect("the bundled registry must be valid")
}

fn all_engines(registry: &Registry) -> BTreeSet<String> {
    registry.all_engine_ids()
}

fn menu_for(registry: &Registry, ext: &str) -> orbit_registry::MenuModel {
    let file = SourceFile::virtual_file(registry.probe_ext(ext));
    registry.menu(&MenuRequest::for_files(vec![file]))
}

fn petal_ids(menu: &orbit_registry::MenuModel, category: &str) -> Vec<String> {
    menu.categories
        .iter()
        .find(|view| view.id == category)
        .map(|view| view.petals.iter().map(|petal| petal.id.clone()).collect())
        .unwrap_or_default()
}

fn find_petal<'a>(
    menu: &'a orbit_registry::MenuModel,
    id: &str,
) -> Option<&'a orbit_registry::Petal> {
    menu.categories
        .iter()
        .flat_map(|category| category.petals.iter())
        .find(|petal| petal.id == id)
}

#[test]
fn bundled_registry_loads_and_validates() {
    let registry = registry();
    assert!(
        registry.formats().len() >= 60,
        "expected a broad format list"
    );
    assert!(registry.engines().len() >= 8);
    assert!(registry.rules().len() >= 15);
    assert!(registry.actions().len() >= 10);
    assert!(registry.categories().len() >= 7);
    // Validation is part of construction, so reaching here means it passed.
    assert_eq!(orbit_registry::validate(&registry), Vec::<String>::new());
}

#[test]
fn pair_matrix_exceeds_the_150_target() {
    let registry = registry();
    let available = all_engines(&registry);
    let pairs = registry.usable_pairs(&available);
    assert!(
        pairs.len() >= 150,
        "expected at least 150 generated pairs, got {}",
        pairs.len()
    );

    // A sample of the shapes the spec calls out, all generated from data.
    let has = |from: &str, to: &str| pairs.iter().any(|pair| pair.from == from && pair.to == to);
    assert!(has("mov", "mp4"));
    assert!(has("png", "webp"));
    assert!(has("png", "avif"));
    assert!(has("heic", "jpg"));
    assert!(has("mov", "gif"));
    assert!(has("mov", "mp3"));
    assert!(has("pdf", "png"));
    assert!(has("docx", "pdf"));
    assert!(has("md", "html"));
    assert!(has("xlsx", "csv"));
    assert!(has("csv", "json"));
    assert!(has("wav", "flac"));
    assert!(has("aiff", "opus"));
    // Read-only inputs must not become targets.
    assert!(
        !pairs
            .iter()
            .any(|pair| pair.to == "raw" || pair.to == "heic")
    );
}

#[test]
fn install_missing_engines_prunes_the_matrix() {
    let registry = registry();
    let with_everything = registry.pair_count(&all_engines(&registry));

    // Only the engines this machine is likely to have, minus the ones that do
    // the heavy lifting.
    let just_native: BTreeSet<String> = ["orbit-native".to_owned()].into_iter().collect();
    let bare = registry.pair_count(&just_native);
    assert!(
        bare < with_everything,
        "fewer engines must mean fewer pairs ({bare} vs {with_everything})"
    );
    // The built-in engine alone still offers something useful.
    assert!(bare > 0);
}

#[test]
fn video_menu_offers_video_not_documents() {
    let registry = registry();
    let menu = menu_for(&registry, "mov");
    let convert = petal_ids(&menu, "convert");

    assert!(convert.contains(&"mp4".to_owned()));
    assert!(convert.contains(&"gif".to_owned()));
    assert!(convert.contains(&"webm".to_owned()));
    assert!(convert.contains(&"mp3".to_owned()));
    assert!(convert.contains(&"png".to_owned()), "still frames");

    for forbidden in ["docx", "xlsx", "md", "html", "ico"] {
        assert!(
            !convert.contains(&forbidden.to_owned()),
            "a video should not offer {forbidden}"
        );
    }
    assert_eq!(menu.shared_kind, Some(Kind::Video));
}

#[test]
fn image_menu_offers_images_and_knows_about_absent_engines() {
    let registry = registry();
    let menu = menu_for(&registry, "png");
    let convert = petal_ids(&menu, "convert");

    assert!(convert.contains(&"webp".to_owned()));
    assert!(convert.contains(&"avif".to_owned()));
    assert!(convert.contains(&"jpg".to_owned()));
    assert!(
        !convert.contains(&"mkv".to_owned()),
        "no silent video from one image"
    );

    // HEIC is a source, never a target.
    assert!(!convert.contains(&"heic".to_owned()));

    // With no engines installed at all, everything is greyed out but still
    // visible, with a reason.
    let mut request =
        MenuRequest::for_files(vec![SourceFile::virtual_file(registry.probe_ext("heic"))]);
    request.available = Some(BTreeSet::new());
    let bare = registry.menu(&request);
    let jpg = find_petal(&bare, "jpg").expect("heic -> jpg should still be offered");
    let disabled = jpg.disabled.as_ref().expect("and it should be disabled");
    assert_eq!(disabled.code, "engine_missing");
    assert!(
        disabled.reason.contains("not installed"),
        "reason should name the problem: {}",
        disabled.reason
    );

    // Give it libheif and the same petal becomes usable.
    let mut request =
        MenuRequest::for_files(vec![SourceFile::virtual_file(registry.probe_ext("heic"))]);
    request.available = Some(["libheif".to_owned()].into_iter().collect());
    let heif = registry.menu(&request);
    let jpg = find_petal(&heif, "jpg").expect("heic -> jpg");
    assert!(jpg.is_usable());
    assert_eq!(
        jpg.engine.as_deref(),
        Some("libheif"),
        "preferred engine wins"
    );
}

#[test]
fn menu_is_two_levels_and_capped() {
    let registry = registry();
    for ext in ["png", "mov", "wav", "pdf", "docx", "csv", "svg", "heic"] {
        let menu = menu_for(&registry, ext);
        assert!(!menu.categories.is_empty(), "{ext} produced an empty menu");
        for category in &menu.categories {
            assert!(
                category.petals.len() <= DEFAULT_MAX_PETALS + 2,
                "{ext}/{} has {} petals, which breaks the two-level rule",
                category.id,
                category.petals.len()
            );
            if category.hidden > 0 {
                assert!(
                    category.petals.iter().any(|petal| petal.id == "op:more"),
                    "a truncated category must offer More"
                );
            }
        }
    }
}

#[test]
fn merge_needs_two_files() {
    let registry = registry();
    let one = menu_for(&registry, "pdf");
    assert!(
        find_petal(&one, "merge").is_none(),
        "merge is meaningless with a single file"
    );

    let two_files = vec![
        SourceFile::virtual_file(registry.probe_ext("pdf")),
        SourceFile::virtual_file(registry.probe_ext("pdf")),
    ];
    let two = registry.menu(&MenuRequest::for_files(two_files));
    let merge = find_petal(&two, "merge").expect("merge should appear for two PDFs");
    assert_eq!(merge.kind, PetalKind::Action);
    assert_eq!(merge.coverage, 2);
    assert_eq!(merge.total, 2);
    assert!(merge.disabled.is_none());
}

#[test]
fn mixed_drag_marks_partial_coverage() {
    let registry = registry();
    let files = vec![
        SourceFile::virtual_file(registry.probe_ext("png")),
        SourceFile::virtual_file(registry.probe_ext("mp4")),
    ];
    let menu = registry.menu(&MenuRequest::for_files(files));
    assert_eq!(menu.shared_kind, None, "mixed input has no shared family");

    let webp = find_petal(&menu, "png");
    if let Some(webp) = webp {
        assert_eq!(webp.coverage, 1);
        assert_eq!(webp.badge.as_deref(), Some("1 of 2 files"));
    }
    let compress = find_petal(&menu, "compress").expect("compression covers both");
    assert_eq!(compress.coverage, 2);
    assert!(compress.badge.is_none());
    // The total candidate count is reported for the "n of m shown" hint.
    assert!(menu.candidates > 0);
}

#[test]
fn nothing_known_still_offers_a_way_out() {
    let registry = registry();
    let menu = menu_for(&registry, "xyzzy");
    assert_eq!(menu.candidates, 0);
    assert_eq!(menu.categories.len(), 1);
    assert_eq!(menu.categories[0].id, "more");
    assert_eq!(menu.categories[0].petals[0].id, "op:search");
}

#[test]
fn pins_and_usage_reorder_petals() {
    let registry = registry();
    let file = SourceFile::virtual_file(registry.probe_ext("png"));

    let plain = registry.menu(&MenuRequest::for_files(vec![file.clone()]));
    let first_plain = petal_ids(&plain, "convert").first().cloned();

    let mut request = MenuRequest::for_files(vec![file.clone()]);
    request.pins = vec!["bmp".to_owned()];
    let pinned = registry.menu(&request);
    assert_eq!(
        petal_ids(&pinned, "convert").first().cloned(),
        Some("bmp".to_owned()),
        "a pinned petal must lead its category"
    );
    assert_ne!(first_plain, Some("bmp".to_owned()));

    let mut request = MenuRequest::for_files(vec![file]);
    request.usage = BTreeMap::from([("tiff".to_owned(), 50)]);
    let used = registry.menu(&request);
    let ids = petal_ids(&used, "convert");
    let tiff = ids
        .iter()
        .position(|id| id == "tiff")
        .expect("tiff present");
    assert!(
        tiff < 2,
        "frecency should lift tiff near the top, got {ids:?}"
    );
}

#[test]
fn chains_appear_as_petals_with_disabled_reasons() {
    let registry = registry();
    let menu = menu_for(&registry, "png");
    let web_ready = find_petal(&menu, "web-ready").expect("seeded chain should show");
    assert_eq!(web_ready.kind, PetalKind::Preset);
    assert_eq!(web_ready.steps.len(), 3);
    assert!(web_ready.badge.as_deref().unwrap().starts_with("pinned"));
    assert!(web_ready.is_usable());

    // Without any engine the chain stays visible but explains itself.
    let mut request =
        MenuRequest::for_files(vec![SourceFile::virtual_file(registry.probe_ext("png"))]);
    request.available = Some(BTreeSet::new());
    let bare = registry.menu(&request);
    let web_ready = find_petal(&bare, "web-ready").expect("still listed");
    assert!(!web_ready.is_usable());
    assert!(
        web_ready
            .disabled
            .as_ref()
            .unwrap()
            .reason
            .contains("not installed")
    );
}

#[test]
fn selectors_support_nesting() {
    let image = Probe {
        format: Some("png".to_owned()),
        kind: Kind::Image,
        ext: "png".to_owned(),
        mime: Some("image/png".to_owned()),
        tags: vec!["lossless".to_owned(), "web".to_owned()],
    };

    let by_ext: Selector = serde_json::from_str(r#"{"ext":["PNG","jpg"]}"#).unwrap();
    assert!(by_ext.matches(&image));

    let by_tag: Selector = serde_json::from_str(r#"{"tag":"web"}"#).unwrap();
    assert!(by_tag.matches(&image));

    let inverted: Selector =
        serde_json::from_str(r#"{"kind":"image","not":{"format":"svg"}}"#).unwrap();
    assert!(inverted.matches(&image));
    let svg = Probe {
        format: Some("svg".to_owned()),
        kind: Kind::Image,
        ext: "svg".to_owned(),
        mime: None,
        tags: vec![],
    };
    assert!(!inverted.matches(&svg));

    let nested: Selector = serde_json::from_str(
        r#"{"any_of":[{"kind":"video"},{"all_of":[{"kind":"image"},{"ext":["heic"]}]}]}"#,
    )
    .unwrap();
    assert!(!nested.matches(&image));
    assert!(!nested.matches(&svg));

    let empty = Selector::default();
    assert!(empty.is_empty());
    assert!(empty.matches(&image), "an empty selector matches anything");
}

#[test]
fn classification_uses_extensions_and_is_case_insensitive() {
    let registry = registry();

    let png = registry.classify(Path::new("/tmp/Holiday Photo.PNG"), 1234);
    assert_eq!(png.probe.format.as_deref(), Some("png"));
    assert_eq!(png.probe.kind, Kind::Image);
    assert_eq!(png.probe.ext, "png");
    assert_eq!(png.file_name(), "Holiday Photo.PNG");

    let heic = registry.classify(Path::new("/tmp/IMG_1234.HEIC"), 1);
    assert_eq!(heic.probe.format.as_deref(), Some("heic"));

    let unknown = registry.classify(Path::new("/tmp/notes.xyzzy"), 1);
    assert_eq!(unknown.probe.kind, Kind::Other);
    assert!(!unknown.probe.is_known());

    // A dotfile has no extension; an extensionless name is its own token.
    assert_eq!(extension_of(Path::new("/home/someone/.bashrc")), "");
    assert_eq!(extension_of(Path::new("/tmp/Makefile")), "makefile");
    assert_eq!(extension_of(Path::new("/tmp/archive.tar.gz")), "gz");
    assert_eq!(extension_of(Path::new("/tmp/weird.")), "");

    // Unicode paths survive intact.
    let unicode = registry.classify(Path::new("/tmp/vidéo 🎬.mov"), 1);
    assert_eq!(unicode.probe.format.as_deref(), Some("mov"));
    assert_eq!(unicode.stem(), "vidéo 🎬");
}

#[test]
fn engine_detection_reports_platform_and_missing_tools() {
    let engine = Engine {
        id: "fake".to_owned(),
        label: "Fake tool".to_owned(),
        binary: Some("orbit-fake-tool".to_owned()),
        alternate_binaries: vec![],
        related_binaries: vec![],
        probe_binary: None,
        version_args: vec!["--version".to_owned()],
        min_version: None,
        license: "MIT".to_owned(),
        distribution: orbit_registry::Distribution::Bundled,
        platforms: vec!["linux-x64".to_owned(), "macos-arm64".to_owned()],
        provides: vec!["op:test".to_owned()],
        progress: orbit_registry::Progress::None,
        cost: 1,
        note: None,
    };

    // Wrong platform: explain the platform, do not claim it is missing.
    let wrong = orbit_registry::detect(&engine, "windows-x64", Some("/nonexistent"));
    assert!(!wrong.available);
    assert!(wrong.reason.as_deref().unwrap().contains("windows-x64"));

    // Right platform, empty PATH: report the binary by name.
    let dir = scratch_dir("missing");
    let absent = orbit_registry::detect(&engine, "linux-x64", Some(dir.to_str().unwrap()));
    assert!(!absent.available);
    assert!(
        absent
            .reason
            .as_deref()
            .unwrap()
            .contains("orbit-fake-tool is not installed")
    );

    // Present on PATH: detected, with a path.
    let dir = scratch_dir("present");
    let tool = dir.join("orbit-fake-tool");
    std::fs::write(&tool, b"#!/bin/sh\nexit 0\n").unwrap();
    let found = orbit_registry::detect(&engine, "linux-x64", Some(dir.to_str().unwrap()));
    assert!(found.available);
    assert_eq!(found.binary.as_deref(), Some(tool.as_path()));
    assert!(found.status_line("Fake tool").contains("found at"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn option_visibility_follows_show_if() {
    let registry = registry();
    let target_size = registry.option("target_size").expect("catalog entry");
    let mode = registry.option("compress_mode").expect("catalog entry");

    let mut values = BTreeMap::new();
    values.insert("compress_mode".to_owned(), serde_json::json!("quality"));
    assert!(
        !target_size.is_visible(&values),
        "hidden while compressing by quality"
    );

    values.insert("compress_mode".to_owned(), serde_json::json!("target_size"));
    assert!(target_size.is_visible(&values));
    assert!(
        mode.is_visible(&values),
        "no conditions means always visible"
    );

    // The size search names real options.
    let search = target_size.search.as_ref().expect("size search");
    for param in &search.params {
        assert!(registry.option(param).is_some(), "{param} must exist");
    }
}

#[test]
fn menu_model_serialises_for_the_ui() {
    let registry = registry();
    let menu = menu_for(&registry, "mov");
    let json = serde_json::to_value(&menu).expect("menu must be serialisable");
    let categories = json["categories"].as_array().expect("categories array");
    let convert = categories
        .iter()
        .find(|category| category["id"] == "convert")
        .expect("convert category");
    let petals = convert["petals"].as_array().expect("petals array");
    let mp4 = petals
        .iter()
        .find(|petal| petal["id"] == "mp4")
        .expect("mp4 petal");
    assert_eq!(mp4["kind"], "conversion");
    assert_eq!(mp4["reference"], "mp4");
    assert!(mp4["options"].is_array());
    assert!(mp4["needs_options"].as_bool().unwrap());
    assert!(json["files"][0]["name"].is_string());
}

#[test]
fn a_broken_registry_explains_itself() {
    let mut docs: RegistryDocs = bundled_docs().expect("embedded documents");
    docs.merge_presets();

    // Rename an engine that rules depend on: every reference must be reported.
    docs.engines.retain(|engine| engine.id != "ffmpeg");
    let error = Registry::new(docs).expect_err("a missing engine must fail validation");
    let message = error.to_string();
    assert!(
        message.contains("ffmpeg"),
        "message should name ffmpeg: {message}"
    );
    assert!(
        message.contains("unknown engine") || message.contains("can never run"),
        "message should explain the problem: {message}"
    );

    // A duplicate id is caught too.
    let mut docs = bundled_docs().expect("embedded documents");
    let duplicate = docs.formats[0].clone();
    docs.formats.push(duplicate);
    let error = Registry::new(docs).expect_err("duplicate ids must fail");
    assert!(error.to_string().contains("duplicate format id"));
}

#[test]
fn build_time_platform_matches_the_engine_list() {
    let registry = registry();
    let platform = host_platform();
    assert_ne!(
        platform, "unknown",
        "the test host must be a supported platform"
    );
    assert!(
        registry
            .engines()
            .iter()
            .any(|engine| engine.supports_platform(platform)),
        "at least one engine must support {platform}"
    );
}

/// A unique scratch directory for a test.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orbit-registry-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}
