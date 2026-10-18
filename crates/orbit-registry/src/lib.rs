//! Orbit's registry: the data-driven answer to "what can this file become?".
//!
//! Four ideas hold the design together:
//!
//! 1. **Data, not code.** Formats, engines, conversion rules, editing actions,
//!    chains and option specs live in `registry/*.json`. Adding a format or a
//!    conversion is a data edit that flows to the radial menu, the command
//!    palette and the CLI at once.
//! 2. **Families, then pairs.** Rules describe families (`any image -> any
//!    writable image`); [`Registry::usable_pairs`] multiplies them out against
//!    the formats and the engines actually installed.
//! 3. **Truthful capability.** A step runs only when the chosen engine declares
//!    the required capability tokens *and* can read the source and write the
//!    target. When nothing fits, the petal is greyed out with a reason instead
//!    of quietly disappearing.
//! 4. **Two levels, always.** [`Registry::menu`] emits categories and petals,
//!    never a third level.
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use orbit_registry::{MenuRequest, Registry, SourceFile};
//!
//! let registry = Registry::bundled()?;
//! let file = SourceFile::virtual_file(registry.probe_ext("mov"));
//! let menu = registry.menu(&MenuRequest::for_files(vec![file]));
//!
//! let convert = menu.categories.iter().find(|c| c.id == "convert").expect("convert ring");
//! assert!(convert.petals.iter().any(|p| p.id == "mp4"));
//! assert!(!convert.petals.iter().any(|p| p.id == "docx"));
//! # Ok(())
//! # }
//! ```

mod engines;
mod load;
mod menu;
mod model;
mod pairs;
mod probe;
mod registry;
mod selector;
mod validate;
pub use engines::{EngineStatus, available_ids, detect, detect_all, host_platform};
pub use load::{
    DOCUMENTS, OptionCatalog, RegistryDocs, bundled, bundled_docs, load_dir, load_docs_dir,
};
pub use menu::{
    CategoryView, DEFAULT_MAX_PETALS, Disabled, FileSummary, MenuModel, MenuRequest, Petal,
    PetalKind,
};
pub use model::{
    Action, Can, Category, Chain, ChainStep, Distribution, Engine, Format, Id, Kind, OptionKind,
    OptionSpec, Progress, Properties, Rule, SearchSpec, Selector, Target,
};
pub use pairs::Pair;
pub use probe::{Probe, SourceFile, extension_of};
pub use registry::{EngineChoice, Registry, RegistryError};
pub use selector::any_selector_matches;
pub use validate::validate;
