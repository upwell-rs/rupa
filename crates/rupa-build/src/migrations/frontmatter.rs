use rupa_migrate::Policy;
use serde::Deserialize;

/// Settings a migration's `rupa:` frontmatter declares.
///
/// Each field is one YAML key: omitted keys keep their defaults and unknown
/// keys are refused, so a new setting is a new field.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Frontmatter {
    /// `transactional` (the default) or `nontransactional`.
    pub(super) policy: Policy,
}

/// The YAML document a header's frontmatter forms, rooted at its `rupa:` key.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    rupa: Option<Frontmatter>,
}

impl Frontmatter {
    /// Parses frontmatter YAML starting at its `rupa:` line; an empty `rupa:`
    /// keeps every default.
    pub(super) fn parse(yaml: &str) -> Result<Self, serde_saphyr::Error> {
        Ok(serde_saphyr::from_str::<Document>(yaml)?
            .rupa
            .unwrap_or_default())
    }
}
