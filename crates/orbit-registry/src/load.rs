//! Loading the registry from disk or from the copy embedded in the binary.
//!
//! The embedded copy is what ships: the app never depends on the `registry/`
//! folder being next to the executable, but development can point at it so
//! edits take effect without a rebuild.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::model::{Action, Category, Chain, Engine, Format, Id, OptionSpec, Rule};
use crate::registry::{Registry, RegistryError};

/// The shared option catalog, keyed by option key.
pub type OptionCatalog = BTreeMap<Id, OptionSpec>;

/// The parsed contents of `registry/*.json`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegistryDocs {
    /// `formats.json`
    #[serde(default)]
    pub formats: Vec<Format>,
    /// `engines.json`
    #[serde(default)]
    pub engines: Vec<Engine>,
    /// `options.json`
    #[serde(default)]
    pub options: OptionCatalog,
    /// `conversions.json`
    #[serde(default)]
    pub conversions: Vec<Rule>,
    /// `actions.json`
    #[serde(default)]
    pub actions: Vec<Action>,
    /// `categories.json`
    #[serde(default)]
    pub categories: Vec<Category>,
    /// `chains.json`
    #[serde(default)]
    pub chains: Vec<Chain>,
    /// `presets.json`: seeded user presets, merged into `chains`.
    #[serde(default)]
    pub presets: Vec<Chain>,
}

impl RegistryDocs {
    /// Fold seeded presets into the chain list so everything downstream sees one
    /// collection. Presets are chains that the user can edit or delete.
    pub fn merge_presets(&mut self) {
        let presets = std::mem::take(&mut self.presets);
        self.chains.extend(presets);
    }
}

/// Document file names, in the order they are read.
pub const DOCUMENTS: [&str; 8] = [
    "formats.json",
    "engines.json",
    "options.json",
    "conversions.json",
    "actions.json",
    "categories.json",
    "chains.json",
    "presets.json",
];

/// Load and validate the registry from a directory.
///
/// # Errors
/// Reports unreadable files, parse errors, and every validation problem.
pub fn load_dir(dir: &Path) -> Result<Registry, RegistryError> {
    let mut docs = load_docs_dir(dir)?;
    docs.merge_presets();
    Registry::new(docs)
}

/// Load the documents without validating them.
///
/// # Errors
/// Reports unreadable files and parse errors.
pub fn load_docs_dir(dir: &Path) -> Result<RegistryDocs, RegistryError> {
    macro_rules! read {
        ($name:literal) => {
            read_doc(&dir.join($name), $name, embedded($name))
        };
    }
    // Development convenience: a missing file falls back to the embedded copy,
    // so a partially populated folder still produces a working registry.
    Ok(RegistryDocs {
        formats: read!("formats.json")?,
        engines: read!("engines.json")?,
        options: read!("options.json")?,
        conversions: read!("conversions.json")?,
        actions: read!("actions.json")?,
        categories: read!("categories.json")?,
        chains: read!("chains.json")?,
        presets: read!("presets.json")?,
    })
}

/// The registry compiled into Orbit.
///
/// # Errors
/// Only fails if the embedded documents are broken, which the test suite
/// prevents.
pub fn bundled() -> Result<Registry, RegistryError> {
    let mut docs = bundled_docs()?;
    docs.merge_presets();
    Registry::new(docs)
}

/// The embedded documents, unvalidated.
///
/// # Errors
/// Only fails if the embedded documents are broken.
pub fn bundled_docs() -> Result<RegistryDocs, RegistryError> {
    Ok(RegistryDocs {
        formats: parse("formats.json", embedded("formats.json"))?,
        engines: parse("engines.json", embedded("engines.json"))?,
        options: parse("options.json", embedded("options.json"))?,
        conversions: parse("conversions.json", embedded("conversions.json"))?,
        actions: parse("actions.json", embedded("actions.json"))?,
        categories: parse("categories.json", embedded("categories.json"))?,
        chains: parse("chains.json", embedded("chains.json"))?,
        presets: parse("presets.json", embedded("presets.json"))?,
    })
}

fn embedded(name: &str) -> &'static str {
    match name {
        "formats.json" => include_str!("../../../registry/formats.json"),
        "engines.json" => include_str!("../../../registry/engines.json"),
        "options.json" => include_str!("../../../registry/options.json"),
        "conversions.json" => include_str!("../../../registry/conversions.json"),
        "actions.json" => include_str!("../../../registry/actions.json"),
        "categories.json" => include_str!("../../../registry/categories.json"),
        "chains.json" => include_str!("../../../registry/chains.json"),
        "presets.json" => include_str!("../../../registry/presets.json"),
        other => panic!("no embedded registry document named {other}"),
    }
}

fn read_doc<T: DeserializeOwned>(
    path: &Path,
    name: &str,
    fallback: &'static str,
) -> Result<T, RegistryError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fallback.to_owned(),
        Err(source) => {
            return Err(RegistryError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    parse(name, &text)
}

fn parse<T: DeserializeOwned>(name: &str, text: &str) -> Result<T, RegistryError> {
    serde_json::from_str(text).map_err(|source| RegistryError::Parse {
        path: std::path::PathBuf::from(format!("registry/{name}")),
        source,
    })
}
