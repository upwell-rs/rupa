use std::path::Path;

use super::Error;
use super::frontmatter::Frontmatter;

/// The key opening a header's YAML frontmatter.
const FRONTMATTER: &str = "rupa:";

/// A source split into its leading comment header and the executable body.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Split<'a> {
    pub(super) description: Option<String>,
    pub(super) frontmatter: Option<String>,
    pub(super) body: &'a str,
}

/// What a migration's header declares besides its body.
#[derive(Debug)]
pub(super) struct Header {
    pub(super) description: Option<String>,
    pub(super) frontmatter: Option<Frontmatter>,
}

impl Split<'_> {
    /// Parses the frontmatter; `path` names the source in errors.
    pub(super) fn header(self, path: &Path) -> Result<Header, Error> {
        let frontmatter = self
            .frontmatter
            .map(|yaml| Frontmatter::parse(&yaml))
            .transpose()
            .map_err(|error| Error::Frontmatter {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;

        Ok(Header {
            description: self.description,
            frontmatter,
        })
    }
}

/// Splits consecutive leading lines starting with `marker` (`--` or `//!`) off `source`.
///
/// Lines before the first one starting with `rupa:` form the description;
/// that line and every later header line form the YAML frontmatter, with the
/// marker and one following space removed so indentation is kept. The body
/// keeps every remaining byte, so the runner executes and hashes exactly what
/// follows the header; editing only the description keeps the checksum.
pub(super) fn split<'a>(source: &'a str, marker: &str) -> Split<'a> {
    let mut lines = Vec::new();
    let mut frontmatter: Option<String> = None;
    let mut body = source.strip_prefix('\u{feff}').unwrap_or(source);

    while let Some(text) = body.strip_prefix(marker) {
        let (line, rest) = text.split_once('\n').unwrap_or((text, ""));
        let line = line.strip_suffix('\r').unwrap_or(line);
        let yaml = line.strip_prefix(' ').unwrap_or(line);

        if let Some(document) = &mut frontmatter {
            document.push('\n');
            document.push_str(yaml);
        } else if line.trim_start().starts_with(FRONTMATTER) {
            frontmatter = Some(yaml.trim_start().to_owned());
        } else if !line.trim().is_empty() {
            lines.push(line.trim());
        }

        body = rest;
    }

    let description = (!lines.is_empty()).then(|| lines.join(" "));

    Split {
        description,
        frontmatter,
        body,
    }
}
