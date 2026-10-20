use std::path::Path;

use super::Error;
use super::directive::{Directive, DirectiveError, Settings};

/// Prefix of a header line that configures the migration instead of describing it.
const DIRECTIVE: &str = "rupa:";

/// A source split into its leading comment header and the executable body.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Split<'a> {
    pub(super) description: Option<String>,
    pub(super) directives: Vec<&'a str>,
    pub(super) body: &'a str,
}

/// What a migration's header declares besides its body.
#[derive(Debug)]
pub(super) struct Header {
    pub(super) description: Option<String>,
    pub(super) directives: Vec<Directive>,
    pub(super) settings: Settings,
}

impl Split<'_> {
    /// Parses the directives into settings; `path` names the source in errors.
    pub(super) fn header(self, path: &Path) -> Result<Header, Error> {
        let mut directives = Vec::with_capacity(self.directives.len());
        let refuse = |text: &str, reason| Error::Directive {
            path: path.to_path_buf(),
            directive: text.to_owned(),
            reason,
        };

        for text in self.directives {
            let directive = Directive::parse(text).map_err(|reason| refuse(text, reason))?;

            if directives.contains(&directive) {
                return Err(refuse(text, DirectiveError::Duplicate));
            }

            directives.push(directive);
        }

        let settings = Settings::from_directives(&directives);

        Ok(Header {
            description: self.description,
            directives,
            settings,
        })
    }
}

/// Splits consecutive leading lines starting with `marker` (`--` or `//!`) off `source`.
///
/// Lines starting with `rupa:` are directives; the others form the description.
/// The body keeps every remaining byte, so the runner executes and hashes exactly
/// what follows the header; editing only the description keeps the checksum.
pub(super) fn split<'a>(source: &'a str, marker: &str) -> Split<'a> {
    let mut lines = Vec::new();
    let mut directives = Vec::new();
    let mut body = source.strip_prefix('\u{feff}').unwrap_or(source);

    while let Some(text) = body.strip_prefix(marker) {
        let (line, rest) = text.split_once('\n').unwrap_or((text, ""));
        let line = line.trim();

        if let Some(directive) = line.strip_prefix(DIRECTIVE) {
            directives.push(directive.trim());
        } else if !line.is_empty() {
            lines.push(line);
        }

        body = rest;
    }

    let description = (!lines.is_empty()).then(|| lines.join(" "));

    Split {
        description,
        directives,
        body,
    }
}
