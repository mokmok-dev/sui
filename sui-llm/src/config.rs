use crate::LlmError;

/// Connection settings for a `LiteLLM` Proxy (`OpenAI`-compatible).
///
/// `base_url` is treated as **trusted operator configuration**. It is not an
/// end-user input surface: feed only known Proxy endpoints. Validation allows
/// `http`/`https`, requires a host, and rejects userinfo; it does not attempt
/// IP blocklists. Cleartext `http` remains allowed for local proxies.
///
/// The API key is stored as a plain [`String`] (not `SecretString`); [`Debug`]
/// redacts non-empty values. Prefer not to log configs at all in production.
#[derive(Clone, PartialEq, Eq)]
pub struct LlmConfig {
    base_url: String,
    api_key: String,
    model: String,
}

impl std::fmt::Debug for LlmConfig {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        f.debug_struct("LlmConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &redact_secret(&self.api_key))
            .field("model", &self.model)
            .finish()
    }
}

impl LlmConfig {
    /// Creates config from explicit values.
    ///
    /// `base_url` may omit the `/v1` suffix; it is normalized for
    /// `async-openai` (which appends `/chat/completions`).
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::InvalidConfig`] when `base_url` or `model` is empty
    /// after trimming, `base_url` is not an `http`/`https` URL with a host and
    /// without userinfo, `base_url` contains ASCII controls, or `api_key` is not
    /// a header-safe token (avoids panics inside `async-openai`).
    pub fn new(
        base_url: impl AsRef<str>,
        api_key: impl Into<String>,
        model: impl AsRef<str>,
    ) -> Result<Self, LlmError> {
        let base_url = normalize_base_url(base_url.as_ref())?;
        let api_key = api_key.into();
        validate_api_key(&api_key)?;
        let model = require_non_empty_model(model.as_ref())?;
        Ok(Self {
            base_url,
            api_key,
            model: model.to_owned(),
        })
    }

    /// Loads `LITELLM_BASE_URL`, optional `LITELLM_API_KEY`, and
    /// `LITELLM_MODEL` from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::MissingEnv`] when a required variable is unset, or
    /// [`LlmError::InvalidConfig`] when values fail validation.
    pub fn from_env() -> Result<Self, LlmError> {
        Self::from_lookup(|key| std::env::var(key))
    }

    /// Normalized `OpenAI`-compatible API base (includes `/v1`).
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Proxy credential (virtual key). May be empty for open local proxies.
    #[must_use]
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Default logical model name for chat calls.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    fn from_lookup<F>(mut lookup: F) -> Result<Self, LlmError>
    where
        F: FnMut(&str) -> Result<String, std::env::VarError>,
    {
        let base_url = require_env_lookup(&mut lookup, "LITELLM_BASE_URL")?;
        let api_key = match lookup("LITELLM_API_KEY") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => String::new(),
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(LlmError::InvalidConfig(
                    "LITELLM_API_KEY must be valid UTF-8".into(),
                ));
            },
        };
        let model = require_env_lookup(&mut lookup, "LITELLM_MODEL")?;
        Self::new(base_url, api_key, model)
    }
}

fn require_env_lookup<F>(
    lookup: &mut F,
    key: &'static str,
) -> Result<String, LlmError>
where
    F: FnMut(&str) -> Result<String, std::env::VarError>,
{
    match lookup(key) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Err(LlmError::MissingEnv(key)),
        Err(std::env::VarError::NotUnicode(_)) => Err(LlmError::InvalidConfig(format!(
            "{key} must be valid UTF-8"
        ))),
    }
}

/// Trims and rejects an empty logical model name.
pub fn require_non_empty_model(model: &str) -> Result<&str, LlmError> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Err(LlmError::InvalidConfig(
            "model must be a non-empty string".into(),
        ));
    }
    Ok(trimmed)
}

fn validate_api_key(api_key: &str) -> Result<(), LlmError> {
    // `async-openai` builds `Authorization: Bearer {api_key}` via
    // `HeaderValue::parse(...).unwrap()`. Reject anything that would panic.
    if !api_key
        .bytes()
        .all(|b| (0x20..=0x7e).contains(&b) && b != b' ')
    {
        return Err(LlmError::InvalidConfig(
            "api_key must be printable ASCII without spaces or controls".into(),
        ));
    }
    Ok(())
}

fn normalize_base_url(raw: &str) -> Result<String, LlmError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(LlmError::InvalidConfig(
            "base_url must be a non-empty string".into(),
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(LlmError::InvalidConfig(
            "base_url must not contain control characters".into(),
        ));
    }
    let without_slash = trimmed.trim_end_matches('/');
    let has_v1_suffix = without_slash
        .get(without_slash.len().saturating_sub(3)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case("/v1"));
    let normalized = if has_v1_suffix {
        without_slash.to_owned()
    } else {
        format!("{without_slash}/v1")
    };
    validate_base_url(&normalized)?;
    Ok(normalized)
}

fn validate_base_url(url: &str) -> Result<(), LlmError> {
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(LlmError::InvalidConfig(
            "base_url must not contain whitespace or control characters".into(),
        ));
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(LlmError::InvalidConfig(
            "base_url must use http or https scheme".into(),
        ));
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(LlmError::InvalidConfig(
            "base_url must use http or https scheme".into(),
        ));
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err(LlmError::InvalidConfig(
            "base_url must include a host".into(),
        ));
    }
    if authority.contains('@') {
        return Err(LlmError::InvalidConfig(
            "base_url must not include userinfo".into(),
        ));
    }
    Ok(())
}

fn redact_secret(secret: &str) -> String {
    if secret.is_empty() {
        String::new()
    } else {
        "***".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base_url_and_rejects_empty() {
        let cfg = LlmConfig::new("http://localhost:4000/", "k", "m").expect("config");
        assert_eq!(cfg.base_url(), "http://localhost:4000/v1");
        assert_eq!(cfg.api_key(), "k");
        assert_eq!(cfg.model(), "m");

        let already = LlmConfig::new("http://localhost:4000/v1", "", "gpt").expect("config");
        assert_eq!(already.base_url(), "http://localhost:4000/v1");

        assert!(matches!(
            LlmConfig::new("  ", "k", "m"),
            Err(LlmError::InvalidConfig(_))
        ));
        assert!(matches!(
            LlmConfig::new("http://x", "k", "  "),
            Err(LlmError::InvalidConfig(_))
        ));
    }

    #[test]
    fn rejects_unsafe_base_url_and_api_key() {
        assert!(matches!(
            LlmConfig::new("ftp://localhost:4000", "k", "m"),
            Err(LlmError::InvalidConfig(_))
        ));
        assert!(matches!(
            LlmConfig::new("http://user:pass@localhost:4000", "k", "m"),
            Err(LlmError::InvalidConfig(_))
        ));
        assert!(matches!(
            LlmConfig::new("http://localhost:4000", "bad\nkey", "m"),
            Err(LlmError::InvalidConfig(_))
        ));
        assert!(matches!(
            LlmConfig::new("http://localhost:4000", "bad\rkey", "m"),
            Err(LlmError::InvalidConfig(_))
        ));
        assert!(matches!(
            LlmConfig::new("http://localhost:4000", "bad\x01key", "m"),
            Err(LlmError::InvalidConfig(_))
        ));
        assert!(matches!(
            LlmConfig::new("http://localhost:4000\r\n/evil", "k", "m"),
            Err(LlmError::InvalidConfig(_))
        ));
        assert!(matches!(
            LlmConfig::new("http:///v1", "k", "m"),
            Err(LlmError::InvalidConfig(_))
        ));
        assert!(LlmConfig::new("HTTPS://localhost:4000", "k", "m").is_ok());
    }

    #[test]
    fn from_env_rejects_non_unicode_api_key() {
        let err = LlmConfig::from_lookup(|key| match key {
            "LITELLM_BASE_URL" => Ok("http://localhost:4000".into()),
            "LITELLM_MODEL" => Ok("proxy-model".into()),
            "LITELLM_API_KEY" => Err(std::env::VarError::NotUnicode("x".into())),
            _ => Err(std::env::VarError::NotPresent),
        })
        .expect_err("non-utf8 api key");
        assert!(matches!(err, LlmError::InvalidConfig(_)));
    }

    #[test]
    fn from_env_rejects_blank_required_values() {
        let err = LlmConfig::from_lookup(|key| match key {
            "LITELLM_BASE_URL" => Ok("  ".into()),
            "LITELLM_MODEL" => Ok("proxy-model".into()),
            _ => Err(std::env::VarError::NotPresent),
        })
        .expect_err("blank base");
        assert!(matches!(err, LlmError::InvalidConfig(_)));
    }

    #[test]
    fn debug_redacts_api_key() {
        let cfg = LlmConfig::new("http://localhost:4000", "super-secret", "m").expect("config");
        let rendered = format!("{cfg:?}");
        assert!(rendered.contains("***"), "{rendered}");
        assert!(!rendered.contains("super-secret"), "{rendered}");
    }

    #[test]
    fn debug_empty_api_key_is_empty_string() {
        let cfg = LlmConfig::new("http://localhost:4000", "", "m").expect("config");
        let rendered = format!("{cfg:?}");
        assert!(rendered.contains("api_key: \"\""), "{rendered}");
        assert!(!rendered.contains("***"), "{rendered}");
    }

    #[test]
    fn from_env_missing_required_vars() {
        let err =
            LlmConfig::from_lookup(|_| Err(std::env::VarError::NotPresent)).expect_err("missing");
        assert!(matches!(err, LlmError::MissingEnv("LITELLM_BASE_URL")));

        let err = LlmConfig::from_lookup(|key| match key {
            "LITELLM_BASE_URL" => Ok("http://localhost:4000".into()),
            _ => Err(std::env::VarError::NotPresent),
        })
        .expect_err("missing model");
        assert!(matches!(err, LlmError::MissingEnv("LITELLM_MODEL")));
    }

    #[test]
    fn from_env_optional_api_key_defaults_empty() {
        let cfg = LlmConfig::from_lookup(|key| match key {
            "LITELLM_BASE_URL" => Ok("http://localhost:4000".into()),
            "LITELLM_MODEL" => Ok("proxy-model".into()),
            _ => Err(std::env::VarError::NotPresent),
        })
        .expect("config");
        assert_eq!(cfg.api_key(), "");
        assert_eq!(cfg.model(), "proxy-model");
        assert_eq!(cfg.base_url(), "http://localhost:4000/v1");
    }
}
