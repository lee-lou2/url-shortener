//! Link normalization, allow-list validation, 9-field SHA-256 deduplication,
//! and PostgreSQL queries.
//!
//! `create_or_find` runs `SELECT` before `INSERT ... ON CONFLICT DO NOTHING` so
//! duplicate submissions burn zero `urls_id_seq` values while staying race-safe.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use url::Url;

use crate::{
    error::{AppError, AppResult},
    short_key::generate_random_key,
    webhook,
};

#[derive(Clone, Default, Deserialize, Serialize, FromRow)]
#[serde(default, rename_all = "camelCase")]
pub struct LinkFields {
    pub default_fallback_url: Option<String>,
    pub ios_deep_link: Option<String>,
    pub ios_fallback_url: Option<String>,
    pub android_deep_link: Option<String>,
    pub android_fallback_url: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image_url: Option<String>,
    pub webhook_url: Option<String>,
}

impl LinkFields {
    pub fn normalize(mut self) -> AppResult<Self> {
        self.default_fallback_url = Some(
            normalize_url(
                self.default_fallback_url.as_deref(),
                web_url,
                "defaultFallbackUrl",
            )?
            .ok_or_else(|| AppError::bad_request("defaultFallbackUrl is required"))?,
        );
        for (field, name) in [
            (&mut self.ios_fallback_url, "iosFallbackUrl"),
            (&mut self.android_fallback_url, "androidFallbackUrl"),
            (&mut self.og_image_url, "ogImageUrl"),
        ] {
            *field = normalize_url(field.as_deref(), web_url, name)?;
        }
        for (field, name) in [
            (&mut self.ios_deep_link, "iosDeepLink"),
            (&mut self.android_deep_link, "androidDeepLink"),
        ] {
            *field = normalize_url(field.as_deref(), deep_link, name)?;
        }
        self.webhook_url = normalize_url(self.webhook_url.as_deref(), webhook_url, "webhookUrl")?;
        self.og_title = normalize_text(self.og_title.as_deref(), 200, "ogTitle")?;
        self.og_description = normalize_text(self.og_description.as_deref(), 500, "ogDescription")?;
        Ok(self)
    }

    pub fn dedupe_hash(&self) -> AppResult<String> {
        let mut hash = Sha256::new();
        hash.update(b"v2\0");
        for field in [
            &self.default_fallback_url,
            &self.ios_deep_link,
            &self.ios_fallback_url,
            &self.android_deep_link,
            &self.android_fallback_url,
            &self.og_title,
            &self.og_description,
            &self.og_image_url,
            &self.webhook_url,
        ] {
            if let Some(value) = field {
                let length = u32::try_from(value.len())
                    .map_err(|_| AppError::bad_request("Field too large"))?;
                hash.update([1]);
                hash.update(length.to_le_bytes());
                hash.update(value.as_bytes());
            } else {
                hash.update([0]);
            }
        }
        let digest = hash.finalize();
        let mut encoded = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(encoded, "{byte:02x}");
        }
        Ok(encoded)
    }
}

fn parse_url(raw: &str) -> Option<Url> {
    if raw.bytes().any(|byte| byte.is_ascii_control()) {
        return None;
    }
    Url::parse(raw.trim()).ok()
}

pub(crate) fn web_url(raw: &str) -> Option<Url> {
    let url = parse_url(raw)?;
    (matches!(url.scheme(), "http" | "https") && url.host().is_some()).then_some(url)
}

pub(crate) fn deep_link(raw: &str) -> Option<Url> {
    let url = parse_url(raw)?;
    (!matches!(
        url.scheme(),
        "javascript" | "data" | "vbscript" | "file" | "blob" | "about"
    ))
    .then_some(url)
}

pub(crate) fn webhook_url(raw: &str) -> Option<Url> {
    web_url(raw).filter(webhook::allowed_url)
}

fn normalize_url(
    value: Option<&str>,
    parse: fn(&str) -> Option<Url>,
    name: &str,
) -> AppResult<Option<String>> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            parse(value)
                .filter(|url| value.trim().len() <= 2048 && url.as_str().len() <= 2048)
                .map(Into::into)
                .ok_or_else(|| AppError::bad_request(format!("Invalid {name}")))
        })
        .transpose()
}

fn normalize_text(value: Option<&str>, max: usize, name: &str) -> AppResult<Option<String>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.chars().count() > max || value.contains('\0') {
                return Err(AppError::bad_request(format!("Invalid {name}")));
            }
            Ok(value.to_owned())
        })
        .transpose()
}

#[derive(Deserialize, Serialize, FromRow)]
pub(crate) struct LinkRecord {
    pub id: i64,
    pub random_key: String,
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub fields: LinkFields,
}

#[derive(FromRow)]
pub(crate) struct LinkIdentity {
    pub id: i64,
    pub random_key: String,
}

pub(crate) async fn create_or_find(
    db: &PgPool,
    fields: &LinkFields,
) -> AppResult<(LinkIdentity, bool)> {
    let hash = fields.dedupe_hash()?;
    let random_key = generate_random_key();
    for _ in 0..3 {
        if let Some(existing) = sqlx::query_as::<_, LinkIdentity>(
            "SELECT id, random_key FROM urls WHERE hashed_value = $1 AND deleted_at IS NULL AND is_active = true",
        ).bind(&hash).fetch_optional(db).await? {
            return Ok((existing, false));
        }
        if let Some(inserted) = sqlx::query_as::<_, LinkIdentity>(
            "INSERT INTO urls (random_key, hashed_value, default_fallback_url, ios_deep_link,
                ios_fallback_url, android_deep_link, android_fallback_url, og_title,
                og_description, og_image_url, webhook_url)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (hashed_value) WHERE deleted_at IS NULL AND is_active = true DO NOTHING
             RETURNING id, random_key",
        )
        .bind(&random_key)
        .bind(&hash)
        .bind(&fields.default_fallback_url)
        .bind(&fields.ios_deep_link)
        .bind(&fields.ios_fallback_url)
        .bind(&fields.android_deep_link)
        .bind(&fields.android_fallback_url)
        .bind(&fields.og_title)
        .bind(&fields.og_description)
        .bind(&fields.og_image_url)
        .bind(&fields.webhook_url)
        .fetch_optional(db)
        .await?
        {
            return Ok((inserted, true));
        }
    }
    Err(AppError::Internal(
        "Link changed during repeated create attempts",
    ))
}

pub(crate) async fn find_active(db: &PgPool, id: i64) -> AppResult<Option<LinkRecord>> {
    Ok(sqlx::query_as::<_, LinkRecord>(
        "SELECT id, random_key, default_fallback_url, ios_deep_link, ios_fallback_url,
                android_deep_link, android_fallback_url, og_title, og_description, og_image_url, webhook_url
         FROM urls WHERE id = $1 AND deleted_at IS NULL AND is_active = true",
    )
    .bind(id)
    .fetch_optional(db)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> LinkFields {
        serde_json::from_str(r#"{"defaultFallbackUrl":"https://example.com/"}"#).unwrap()
    }

    #[test]
    fn hash_format_is_fixed() {
        assert_eq!(
            example().dedupe_hash().unwrap(),
            "23816ffeebf1a435bbd72fd2323da9724c70cd4f7837d81cf3a9de93f8fe185a"
        );
    }

    #[test]
    fn hash_preserves_field_boundaries_and_metadata() {
        let first = LinkFields {
            ios_deep_link: Some("app:a:https://b".into()),
            ..example()
        };
        let second = LinkFields {
            ios_deep_link: Some("app:a".into()),
            ios_fallback_url: Some("https://b".into()),
            ..example()
        };
        assert_ne!(first.dedupe_hash().unwrap(), second.dedupe_hash().unwrap());
        let baseline = example().dedupe_hash().unwrap();
        for name in [
            "iosDeepLink",
            "iosFallbackUrl",
            "androidDeepLink",
            "androidFallbackUrl",
            "ogTitle",
            "ogDescription",
            "ogImageUrl",
            "webhookUrl",
        ] {
            let mut fields = serde_json::json!({"defaultFallbackUrl": "https://example.com/"});
            fields[name] = serde_json::json!("https://example.net/");
            let fields: LinkFields = serde_json::from_value(fields).unwrap();
            assert_ne!(fields.dedupe_hash().unwrap(), baseline, "{name}");
        }
    }

    #[test]
    fn rejects_controls_and_unsupported_schemes() {
        for value in [
            "javascript:0",
            "data:text/plain,x",
            "vbscript:0",
            "file:///tmp/a",
            "blob:https://example.com/x",
            "about:blank",
            "https://example.com/\r\n",
            "https://example.com/\t",
        ] {
            assert!(web_url(value).is_none(), "{value}");
            assert!(deep_link(value).is_none(), "{value}");
        }
        assert!(web_url("ftp://example.com").is_none());
        assert!(deep_link("myapp:open").is_some());
        let long_url = format!("https://example.com/{}", "x".repeat(2048));
        assert!(normalize_url(Some(&long_url), web_url, "url").is_err());
    }
}
