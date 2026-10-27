/// What a redacted secret is replaced with.
const MASK: &str = "***";

/// Query and keyword parameters whose values are credentials.
const PASSWORD_KEYS: [&str; 3] = ["password=", "pass=", "sslpassword="];

/// Masks secrets in rendered command output.
///
/// Every rendered report passes through it, human and JSON alike, so a
/// driver error that echoes a connection string still shows no password.
/// Each secret is matched as written, percent-decoded and JSON-escaped.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    /// Masks nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Also masks `secret`. An empty secret is ignored.
    pub fn secret(mut self, secret: impl Into<String>) -> Self {
        let secret = secret.into();

        if !secret.is_empty() {
            let decoded = percent_decode(&secret);

            for form in [secret, decoded] {
                let escaped = serde_json::to_string(&form).expect("a string serializes");

                self.secrets.push(escaped[1..escaped.len() - 1].to_owned());
                self.secrets.push(form);
            }

            self.secrets
                .sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
            self.secrets.dedup();
        }

        self
    }

    /// Also masks the password of `url`: its userinfo password and any
    /// `password`, `pass` or `sslpassword` parameter, in URL or keyword form.
    pub fn url(self, url: &str) -> Self {
        passwords(url).into_iter().fold(self, Self::secret)
    }

    /// `text` with every secret masked.
    pub fn redact(&self, text: &str) -> String {
        let mut text = text.to_owned();

        for secret in &self.secrets {
            text = text.replace(secret.as_str(), MASK);
        }

        text
    }
}

/// `url` with its passwords masked, for showing which target a command used.
pub fn redact_url(url: &str) -> String {
    Redactor::new().url(url).redact(url)
}

/// The password components of a connection URL or keyword string.
fn passwords(url: &str) -> Vec<&str> {
    let mut found = Vec::new();

    if let Some((_, rest)) = url.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();

        if let Some((userinfo, _)) = authority.rsplit_once('@')
            && let Some((_, password)) = userinfo.split_once(':')
        {
            found.push(password);
        }
    }

    for parameter in url.split(['?', '&', ';', ' ']) {
        let value = PASSWORD_KEYS
            .iter()
            .find_map(|key| parameter.strip_prefix(key))
            .map(|value| value.trim_matches(['\'', '"']));

        found.extend(value);
    }

    found
}

/// `text` with `%XX` escapes decoded, or as written if they are not UTF-8.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());

        match escape {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded).unwrap_or_else(|_| text.to_owned())
}
