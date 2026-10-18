//! Input probing: what a dragged path looks like to the registry.
//!
//! In M1 a file is classified by extension. M2 replaces the extension lookup
//! with magic-byte sniffing and engine metadata (`ffprobe`, `vipsheader`,
//! `pdfinfo`) while keeping this type, so nothing above it changes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{Format, Id, Kind};

/// What Orbit knows about one input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    /// Matched format id, when the registry recognises the input.
    pub format: Option<Id>,
    /// Family, [`Kind::Other`] when unrecognised.
    pub kind: Kind,
    /// Lower-case extension without the dot; may be empty.
    pub ext: String,
    /// Primary MIME type, when known.
    pub mime: Option<String>,
    /// Format tags, copied from the matched format.
    pub tags: Vec<String>,
}

impl Probe {
    /// A probe for a registry format, as if a file of that format was dragged.
    #[must_use]
    pub fn of_format(format: &Format, ext: &str) -> Self {
        Self {
            format: Some(format.id.clone()),
            kind: format.kind,
            ext: ext.to_owned(),
            mime: format.mimes.first().cloned(),
            tags: format.tags.clone(),
        }
    }

    /// An unrecognised input with the given extension.
    #[must_use]
    pub fn unknown(ext: &str) -> Self {
        Self {
            format: None,
            kind: Kind::Other,
            ext: ext.to_owned(),
            mime: None,
            tags: Vec::new(),
        }
    }

    /// True when the input was matched to a known format.
    #[must_use]
    pub fn is_known(&self) -> bool {
        self.format.is_some()
    }
}

/// A file the user dragged, plus its probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    /// Absolute path, absent for registry-only probes.
    pub path: Option<PathBuf>,
    /// Size in bytes, `0` when unknown.
    pub size_bytes: u64,
    /// Classification.
    pub probe: Probe,
}

impl SourceFile {
    /// A source with no path, used to reason about a format rather than a file.
    #[must_use]
    pub fn virtual_file(probe: Probe) -> Self {
        Self {
            path: None,
            size_bytes: 0,
            probe,
        }
    }

    /// File name without directories, falling back to the extension.
    #[must_use]
    pub fn file_name(&self) -> String {
        self.path.as_deref().and_then(Path::file_name).map_or_else(
            || format!("file.{}", self.probe.ext),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// File stem used for output naming.
    #[must_use]
    pub fn stem(&self) -> String {
        self.path.as_deref().and_then(Path::file_stem).map_or_else(
            || "file".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        )
    }
}

/// Lower-case extension of a path, without the dot.
///
/// A dotfile such as `.bashrc` has no extension and returns an empty string.
#[must_use]
pub fn extension_of(path: &Path) -> String {
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_ascii_lowercase(),
        Some(_) | None => {
            // No dot at all: treat the whole name as the extension, which is how
            // extensionless formats such as `Makefile` were handled before.
            if name.contains('.') {
                String::new()
            } else {
                name.to_ascii_lowercase()
            }
        }
    }
}
