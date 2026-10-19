/// A source split into its leading comment description and the executable body.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Split<'a> {
    pub(super) description: Option<String>,
    pub(super) body: &'a str,
}

/// Splits consecutive leading lines starting with `marker` (`--` or `//!`) off `source`.
///
/// The body keeps every remaining byte, so the runner executes and hashes exactly
/// what follows the header; editing only the description keeps the checksum.
pub(super) fn split<'a>(source: &'a str, marker: &str) -> Split<'a> {
    let mut lines = Vec::new();
    let mut body = source.strip_prefix('\u{feff}').unwrap_or(source);

    while let Some(text) = body.strip_prefix(marker) {
        let (line, rest) = text.split_once('\n').unwrap_or((text, ""));
        let line = line.trim();

        if !line.is_empty() {
            lines.push(line);
        }

        body = rest;
    }

    let description = (!lines.is_empty()).then(|| lines.join(" "));

    Split { description, body }
}
