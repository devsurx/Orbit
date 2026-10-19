//! Value types for the registry documents in `registry/*.json`.
//!
//! Everything here is plain data: no I/O, no behaviour beyond parsing and
//! matching. The files are the source of truth for what Orbit can do, so these
//! types are deliberately close to the on-disk shape.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Identifier used by every registry document (`"webp"`, `"ffmpeg"`,
/// `"video-to-gif"`).
pub type Id = String;

/// Broad family a format belongs to. Used for applicability, engine selection
/// and menu layout, never for equality of concrete formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Raster and vector images.
    Image,
    /// Moving pictures.
    Video,
    /// Sound only.
    Audio,
    /// Office documents that are not PDFs.
    Document,
    /// PDF, which behaves differently enough to deserve its own family.
    Pdf,
    /// Plain text and markup.
    Text,
    /// Structured and tabular data.
    Data,
    /// Unclassified input.
    Other,
}

impl Kind {
    /// Every variant, useful for exhaustive iteration and diagnostics.
    pub const ALL: [Self; 8] = [
        Self::Image,
        Self::Video,
        Self::Audio,
        Self::Document,
        Self::Pdf,
        Self::Text,
        Self::Data,
        Self::Other,
    ];

    /// Lower-case wire name, matching the JSON documents.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Document => "document",
            Self::Pdf => "pdf",
            Self::Text => "text",
            Self::Data => "data",
            Self::Other => "other",
        }
    }

    /// Parse a wire name; unknown names become [`Kind::Other`] rather than an error.
    #[must_use]
    pub fn parse(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(name.trim()))
            .unwrap_or(Self::Other)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a format can be read, written, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Can {
    /// Orbit can decode it.
    #[serde(default)]
    pub read: bool,
    /// Orbit can encode it.
    #[serde(default)]
    pub write: bool,
}

/// Traits of the bitstream that affect which options make sense.
///
/// Eight independent flags on purpose: this mirrors the on-disk schema, and
/// grouping them into an enum or bitset would only obscure `formats.json`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Properties {
    /// Encoding is lossy, so a quality option is meaningful.
    #[serde(default)]
    pub lossy: bool,
    /// Supports transparency.
    #[serde(default)]
    pub alpha: bool,
    /// Can hold more than one frame (GIF, WebP, video).
    #[serde(default)]
    pub animated: bool,
    /// Can hold more than one page.
    #[serde(default)]
    pub multipage: bool,
    /// Vector source, so rasterising needs a scale.
    #[serde(default)]
    pub vector: bool,
    /// Layered source such as PSD or XCF.
    #[serde(default)]
    pub layered: bool,
    /// More than one track (video plus several audio tracks).
    #[serde(default)]
    pub multitrack: bool,
    /// Indirection only, like camera RAW.
    #[serde(default)]
    pub palette: bool,
}

/// One entry of `formats.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Format {
    /// Stable id used by rules and UI.
    pub id: Id,
    /// Human label.
    pub label: String,
    /// Family.
    pub kind: Kind,
    /// Lower-case extensions without the dot.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// MIME types.
    #[serde(default)]
    pub mimes: Vec<String>,
    /// Hex magic-byte prefixes used once sniffing lands in M2.
    #[serde(default)]
    pub signature: Vec<String>,
    /// Read and write support.
    pub can: Can,
    /// Engines that can decode it, best first.
    #[serde(default)]
    pub readers: Vec<Id>,
    /// Engines that can encode it, best first.
    #[serde(default)]
    pub writers: Vec<Id>,
    /// Bitstream properties.
    #[serde(default)]
    pub properties: Properties,
    /// Free-form tags matching `Selector::tag`.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Ranking bias, higher floats closer to the top of the ring.
    #[serde(default)]
    pub rank: i32,
    /// Caveat shown in the UI, for example a licensing or codec note.
    #[serde(default)]
    pub note: Option<String>,
}

/// How an engine is meant to reach the user's machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Distribution {
    /// Ships inside the installer.
    #[default]
    Bundled,
    /// Detected on the system, used when present.
    System,
    /// Detected but never installed for the user.
    Optional,
}

/// How progress is read from an engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Progress {
    /// `-progress pipe:1` style key/value pairs (`FFmpeg`).
    #[serde(rename = "keyvalue")]
    KeyValue,
    /// Progress is derived from lines on stderr.
    Line,
    /// No machine-readable progress: stage-based reporting only.
    #[default]
    None,
}

/// One entry of `engines.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Engine {
    /// Stable id used by rules and actions.
    pub id: Id,
    /// Human label.
    pub label: String,
    /// Bare executable name, absent for engines compiled into Orbit.
    #[serde(default)]
    pub binary: Option<String>,
    /// Other names the same tool may be installed as.
    #[serde(default)]
    pub alternate_binaries: Vec<String>,
    /// Companion tools that ship with this engine (ffprobe, pdftotext).
    #[serde(default)]
    pub related_binaries: Vec<String>,
    /// Optional metadata tool, required for probing.
    #[serde(default)]
    pub probe_binary: Option<String>,
    /// Arguments that print the version.
    #[serde(default)]
    pub version_args: Vec<String>,
    /// Lowest version Orbit is tested against.
    #[serde(default)]
    pub min_version: Option<String>,
    /// SPDX expression or licence name.
    pub license: String,
    /// How it reaches the user.
    #[serde(default)]
    pub distribution: Distribution,
    /// Platforms the engine is available for.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Capability tokens such as `op:trim` or `op:pdf_merge`.
    #[serde(default)]
    pub provides: Vec<String>,
    /// Progress protocol.
    #[serde(default)]
    pub progress: Progress,
    /// Relative cost, used to settle ties between capable engines.
    #[serde(default = "default_cost")]
    pub cost: u8,
    /// Licence or behaviour note surfaced in Settings and the README.
    #[serde(default)]
    pub note: Option<String>,
}

fn default_cost() -> u8 {
    1
}

impl Engine {
    /// Every executable name this engine may answer to, primary first.
    #[must_use]
    pub fn binaries(&self) -> Vec<&str> {
        let mut names = Vec::new();
        if let Some(binary) = &self.binary {
            names.push(binary.as_str());
        }
        names.extend(self.alternate_binaries.iter().map(String::as_str));
        names
    }

    /// True when the engine declares support for `token`.
    #[must_use]
    pub fn provides(&self, token: &str) -> bool {
        self.provides.iter().any(|have| have == token)
    }

    /// True when every token is declared.
    #[must_use]
    pub fn provides_all(&self, tokens: &[String]) -> bool {
        tokens.iter().all(|token| self.provides(token))
    }

    /// True when the engine runs on `platform` (an empty list means "anywhere").
    #[must_use]
    pub fn supports_platform(&self, platform: &str) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|have| have == platform)
    }
}

/// A conversion target: either one format, or every writable format of a family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A concrete format id.
    Format(Id),
    /// Every writable format of this family, written as `@writable:<kind>`.
    Writable(Kind),
}

impl Target {
    /// Prefix for the family-wide form.
    pub const WRITABLE_PREFIX: &'static str = "@writable:";

    /// Parse the on-disk form.
    ///
    /// # Errors
    /// Returns a message when the string is an unknown `@` directive.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if let Some(family) = raw.strip_prefix(Self::WRITABLE_PREFIX) {
            let kind = Kind::parse(family);
            let named_other = family.trim().eq_ignore_ascii_case(Kind::Other.as_str());
            if kind == Kind::Other && !named_other {
                return Err(format!("unknown family in target: {raw}"));
            }
            return Ok(Self::Writable(kind));
        }
        if raw.starts_with('@') {
            return Err(format!("unknown target directive: {raw}"));
        }
        Ok(Self::Format(raw.to_owned()))
    }

    /// On-disk form, used for errors and round-tripping.
    #[must_use]
    pub fn as_written(&self) -> String {
        match self {
            Self::Format(id) => id.clone(),
            Self::Writable(kind) => format!("{}{kind}", Self::WRITABLE_PREFIX),
        }
    }
}

impl Serialize for Target {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_written())
    }
}

impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// One entry of `conversions.json`: every pair from one input family to one or
/// more output formats, with the engines that can do it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Stable id.
    pub id: Id,
    /// Human label, used in the palette and tooltips.
    #[serde(default)]
    pub label: String,
    /// Free-form discriminator; `"conversion"` for everything in this file.
    #[serde(default)]
    pub kind: Option<String>,
    /// Which inputs the rule accepts.
    pub from: Selector,
    /// Where it sends them.
    pub to: Target,
    /// Optional allow-list of concrete targets for family-wide rules.
    #[serde(default)]
    pub only: Vec<Id>,
    /// Engines that can carry out the rule, best first.
    #[serde(default)]
    pub engines: Vec<Id>,
    /// Capability tokens the chosen engine must declare.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Option values applied when the user drops without opening the panel.
    #[serde(default)]
    pub defaults: BTreeMap<String, serde_json::Value>,
    /// Option catalog keys the panel should show.
    #[serde(default)]
    pub options: Vec<Id>,
    /// Ranking bias.
    #[serde(default)]
    pub rank: i32,
    /// Caveat shown as a tooltip.
    #[serde(default)]
    pub note: Option<String>,
}

/// One entry of `actions.json`: an editing tool that keeps the format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    /// Stable id.
    pub id: Id,
    /// Human label.
    pub label: String,
    /// Radial category this action belongs to.
    pub category: Id,
    /// Monoline icon name.
    pub icon: String,
    /// Which inputs it applies to; an empty list means "anything".
    #[serde(default)]
    pub applies_to: Vec<Selector>,
    /// Candidate engines per input family, best first.
    #[serde(default)]
    pub engines: BTreeMap<Kind, Vec<Id>>,
    /// Capability tokens the chosen engine must declare, per input family.
    #[serde(default)]
    pub requires: BTreeMap<Kind, Vec<String>>,
    /// Option catalog keys.
    #[serde(default)]
    pub options: Vec<Id>,
    /// Defaults used on a plain drop.
    #[serde(default)]
    pub defaults: BTreeMap<String, serde_json::Value>,
    /// Ranking bias.
    #[serde(default)]
    pub rank: i32,
    /// Minimum number of dragged files for the action to make sense.
    #[serde(default = "default_requires_files")]
    pub requires_files: u32,
    /// True when the action folds every dragged file into *one* result instead
    /// of producing one per file.
    ///
    /// This is the difference between "convert these four photos" (four files,
    /// each named after its own source) and "merge these four PDFs" (one file).
    /// The two are genuinely different shapes of job, and the resolver has to
    /// know which it is before it can decide how many requests to emit.
    #[serde(default)]
    pub combines: bool,
    /// When set to `conversions`, viability comes from the conversion rules
    /// instead of `engines` (used by the generic Convert action).
    #[serde(default)]
    pub resolves_via: Option<String>,
    /// Whether the action may appear in the radial ring. A false value keeps it
    /// available to the command palette and to chains only, which is how the
    /// generic Convert entry avoids competing with the concrete format petals.
    #[serde(default = "default_true")]
    pub ring: bool,
    /// Caveat shown as a tooltip.
    #[serde(default)]
    pub note: Option<String>,
    /// Word used in the output name, as in `photo (converted).webp`.
    ///
    /// Data rather than code, so the naming rule has one home and a translator
    /// has one place to change it. A format petal uses the generic `convert`
    /// action's tag.
    #[serde(default = "default_tag")]
    pub tag: String,
    /// Extension the result carries: `same` keeps the input's own extension
    /// (`compress` a PNG and it stays a PNG), anything else is used verbatim
    /// (`mp3` for "Extract audio", `png` for "Extract frame").
    #[serde(default = "default_output_ext")]
    pub output_ext: String,
}

fn default_true() -> bool {
    true
}

fn default_requires_files() -> u32 {
    1
}

fn default_tag() -> String {
    "converted".to_owned()
}

fn default_output_ext() -> String {
    "same".to_owned()
}

impl Action {
    /// Engines to try for this input family.
    #[must_use]
    pub fn engines_for(&self, kind: Kind) -> &[Id] {
        self.engines.get(&kind).map_or(&[], Vec::as_slice)
    }

    /// Whether a plain drop of `count` files has enough to work with.
    #[must_use]
    pub fn accepts(&self, count: usize) -> bool {
        count >= usize::try_from(self.requires_files).unwrap_or(1)
    }

    /// Capability tokens to require for this input family.
    #[must_use]
    pub fn requires_for(&self, kind: Kind) -> &[String] {
        self.requires.get(&kind).map_or(&[], Vec::as_slice)
    }
}

/// One step of a chain or preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainStep {
    /// Action id, for example `resize` or the generic `convert`.
    pub action: Id,
    /// Option values for this step.
    #[serde(default)]
    pub options: BTreeMap<String, serde_json::Value>,
}

/// One entry of `chains.json` or `presets.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chain {
    /// Stable id.
    pub id: Id,
    /// Human label.
    pub label: String,
    /// Monoline icon name.
    #[serde(default)]
    pub icon: String,
    /// Radial category this chain appears in.
    #[serde(default = "default_category")]
    pub category: Id,
    /// `builtin`, `seed` or `user`.
    #[serde(default)]
    pub source: Option<String>,
    /// Which inputs the chain applies to; empty means "trust the steps".
    #[serde(default)]
    pub applies_to: Vec<Selector>,
    /// Rank boost from the user pinning it.
    #[serde(default)]
    pub pin: Option<u8>,
    /// Ordered steps.
    pub steps: Vec<ChainStep>,
    /// Caveat shown as a tooltip.
    #[serde(default)]
    pub note: Option<String>,
}

fn default_category() -> Id {
    "convert".to_owned()
}

/// One entry of `categories.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    /// Stable id.
    pub id: Id,
    /// Human label.
    pub label: String,
    /// Monoline icon name.
    pub icon: String,
    /// Sort order around the ring.
    #[serde(default)]
    pub order: i32,
    /// Optional theme accent token.
    #[serde(default)]
    pub accent: Option<String>,
    /// Tooltip text.
    #[serde(default)]
    pub description: Option<String>,
}

/// A condition on a probed file. All present fields must hold; the `any_of`,
/// `all_of` and `not` combinators nest freely.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Selector {
    /// Match one family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    /// Match one format id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Id>,
    /// Match one MIME type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// Match any of these extensions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ext: Vec<String>,
    /// Match one format tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// At least one must match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_of: Vec<Self>,
    /// All must match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all_of: Vec<Self>,
    /// Inverted: matches when the inner selector does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not: Option<Box<Self>>,
}

/// Type of a user-facing option, which decides how the panel renders it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionKind {
    /// Closed set of values.
    Enum,
    /// Closed set of values, more than one selectable.
    MultiEnum,
    /// Slider with a range.
    Range,
    /// Integer field.
    Int,
    /// Floating point field.
    Number,
    /// Checkbox.
    Bool,
    /// Single-line text.
    Text,
    /// Byte count shown as KB/MB/GB.
    Bytes,
    /// Number of seconds, editable as `hh:mm:ss.mmm`.
    Duration,
    /// A point in time, editable as `hh:mm:ss.mmm`.
    Time,
    /// A start and end pair, drawn as a timeline selection.
    TimeRange,
    /// A rectangle, drawn as a crop box.
    Region,
    /// A file or folder path.
    Path,
    /// A format id, populated from the registry at run time.
    Format,
    /// A percentage.
    Percent,
}

/// Bisection settings for size-targeted compression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSpec {
    /// Option keys the search is allowed to vary.
    pub params: Vec<Id>,
    /// How many encode attempts before giving up with the closest result.
    #[serde(default = "default_attempts")]
    pub max_attempts: u8,
}

fn default_attempts() -> u8 {
    6
}

/// One entry of `options.json`, referenced by rules, actions and chains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionSpec {
    /// Key, repeated for self-describing documents.
    pub key: Id,
    /// Human label.
    #[serde(default)]
    pub label: String,
    /// Widget to render.
    #[serde(rename = "type")]
    pub kind: OptionKind,
    /// Allowed values for enum-like kinds.
    #[serde(default)]
    pub values: Vec<serde_json::Value>,
    /// Per-value labels for enums.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Minimum for numeric kinds.
    #[serde(default)]
    pub min: Option<f64>,
    /// Maximum for numeric kinds.
    #[serde(default)]
    pub max: Option<f64>,
    /// Value used when the user drops without opening the panel.
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// Unit suffix, for example `px` or `bit/s`.
    #[serde(default)]
    pub unit: Option<String>,
    /// Only show this option when every listed key already equals the value.
    #[serde(default)]
    pub show_if: Option<BTreeMap<Id, serde_json::Value>>,
    /// Placeholder for text kinds.
    #[serde(default)]
    pub placeholder: Option<String>,
    /// Helper line under the field.
    #[serde(default)]
    pub help: Option<String>,
    /// Size-target search settings.
    #[serde(default)]
    pub search: Option<SearchSpec>,
}

impl OptionSpec {
    /// True when every `show_if` condition matches the current values.
    #[must_use]
    pub fn is_visible(&self, values: &BTreeMap<Id, serde_json::Value>) -> bool {
        self.show_if.as_ref().is_none_or(|conditions| {
            conditions
                .iter()
                .all(|(key, wanted)| values.get(key).is_some_and(|have| have == wanted))
        })
    }
}
