//! Matching logic for [`Selector`].
//!
//! The type lives in [`crate::model`] because it is part of the on-disk schema;
//! the behaviour lives here so the schema module stays pure data.

use crate::model::Selector;
use crate::probe::Probe;

impl Selector {
    /// True when the selector constrains nothing, which means "any input".
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.format.is_none()
            && self.mime.is_none()
            && self.ext.is_empty()
            && self.tag.is_none()
            && self.any_of.is_empty()
            && self.all_of.is_empty()
            && self.not.is_none()
    }

    /// True when a probed input satisfies every constraint.
    #[must_use]
    pub fn matches(&self, probe: &Probe) -> bool {
        if self.not.as_ref().is_some_and(|inner| inner.matches(probe)) {
            return false;
        }
        if let Some(kind) = self.kind {
            // An unrecognised input only matches an explicit `other` selector.
            if probe.kind != kind {
                return false;
            }
        }
        if let Some(format) = &self.format {
            if probe.format.as_deref() != Some(format.as_str()) {
                return false;
            }
        }
        if let Some(mime) = &self.mime {
            if probe.mime.as_deref() != Some(mime.as_str()) {
                return false;
            }
        }
        if !self.ext.is_empty()
            && !self
                .ext
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(&probe.ext))
        {
            return false;
        }
        if let Some(tag) = &self.tag {
            if !probe.tags.iter().any(|have| have == tag) {
                return false;
            }
        }
        if !self.any_of.is_empty() && !self.any_of.iter().any(|inner| inner.matches(probe)) {
            return false;
        }
        if !self.all_of.is_empty() && !self.all_of.iter().all(|inner| inner.matches(probe)) {
            return false;
        }
        true
    }

    /// True when the selector mentions `format_id` anywhere inside it.
    #[must_use]
    pub fn mentions_format(&self, format_id: &str) -> bool {
        if self.format.as_deref() == Some(format_id) {
            return true;
        }
        self.any_of
            .iter()
            .any(|inner| inner.mentions_format(format_id))
            || self
                .all_of
                .iter()
                .any(|inner| inner.mentions_format(format_id))
            || self
                .not
                .as_ref()
                .is_some_and(|inner| inner.mentions_format(format_id))
    }
}

/// True when every selector in `selectors` matches, with an empty list meaning
/// "applies to anything".
#[must_use]
pub fn any_selector_matches(selectors: &[Selector], probe: &Probe) -> bool {
    selectors.is_empty() || selectors.iter().any(|selector| selector.matches(probe))
}
