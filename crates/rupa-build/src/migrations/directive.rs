use std::fmt;

use rupa_migrate::Policy;

/// A recognized `rupa:` header directive.
///
/// Each line is a keyword, optionally followed by a value for directives that
/// take one. Adding a directive means adding a variant, its row in
/// [`KEYWORDS`](Self::KEYWORDS) and its effect on [`Settings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Directive {
    /// Runs the body without an artifact transaction.
    NonTransactional,
}

impl Directive {
    /// Every directive by keyword, in the order diagnostics list them.
    const KEYWORDS: [(&'static str, Self); 1] = [("nontransactional", Self::NonTransactional)];

    /// Parses the text after `rupa:`.
    pub(super) fn parse(text: &str) -> Result<Self, DirectiveError> {
        let end = text
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(text.len());
        let (keyword, value) = (&text[..end], text[end..].trim());
        let directive = Self::KEYWORDS
            .iter()
            .find(|(known, _)| *known == keyword)
            .map(|(_, directive)| *directive)
            .ok_or(DirectiveError::Unknown)?;

        if !value.is_empty() {
            return Err(DirectiveError::UnexpectedValue);
        }

        Ok(directive)
    }

    fn apply(self, settings: &mut Settings) {
        match self {
            Self::NonTransactional => settings.policy = Policy::NonTransactional,
        }
    }
}

/// What a migration's directives configure, starting from the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Settings {
    pub(super) policy: Policy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            policy: Policy::TransactionalRequired,
        }
    }
}

impl Settings {
    /// The defaults with each directive applied in order.
    pub(super) fn from_directives(directives: &[Directive]) -> Self {
        let mut settings = Self::default();

        for directive in directives {
            directive.apply(&mut settings);
        }

        settings
    }
}

/// Why a `rupa:` header line was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DirectiveError {
    /// The keyword names no directive.
    #[error("unknown directive; known directives: {}", Known)]
    Unknown,
    /// A directive that takes no value was given one.
    #[error("this directive takes no value")]
    UnexpectedValue,
    /// The same directive appears twice in one header.
    #[error("this directive is declared more than once")]
    Duplicate,
}

/// The known keywords, comma separated.
struct Known;

impl fmt::Display for Known {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (keyword, _)) in Directive::KEYWORDS.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }

            f.write_str(keyword)?;
        }

        Ok(())
    }
}
