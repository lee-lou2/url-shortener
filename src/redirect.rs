use askama::Template;
use axum::{
    body::Bytes,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    error::{AppError, AppResult},
    links::{deep_link, web_url, webhook_url, LinkRecord},
};

pub(crate) const INDEX_HTML: &str = include_str!("../views/index.html");
const REDIRECT_HTML: &str = include_str!("../views/redirect.html");

#[derive(Template)]
#[template(path = "redirect.html")]
pub struct RedirectPage<'a> {
    pub default_url: &'a str,
    pub ios_deep_link: Option<&'a str>,
    pub ios_fallback_url: Option<&'a str>,
    pub android_deep_link: Option<&'a str>,
    pub android_fallback_url: Option<&'a str>,
    pub og_title: Option<&'a str>,
    pub og_description: Option<&'a str>,
    pub og_image_url: Option<&'a str>,
}

pub(crate) struct CachedLink {
    pub random_key: [u8; 4],
    pub webhook_url: Option<Url>,
    payload: RedirectPayload,
}

enum RedirectPayload {
    Direct(HeaderValue),
    Html(Bytes),
}

impl CachedLink {
    pub fn compile(record: &LinkRecord) -> AppResult<Option<Self>> {
        let Ok(random_key) = <[u8; 4]>::try_from(record.random_key.as_bytes()) else {
            return Ok(None);
        };
        let fields = &record.fields;
        let Some(default_url) = fields.default_fallback_url.as_deref().and_then(web_url) else {
            return Ok(None);
        };
        let ios_deep = fields.ios_deep_link.as_deref().and_then(deep_link);
        let ios_fallback = fields.ios_fallback_url.as_deref().and_then(web_url);
        let android_deep = fields.android_deep_link.as_deref().and_then(deep_link);
        let android_fallback = fields.android_fallback_url.as_deref().and_then(web_url);
        let image = fields.og_image_url.as_deref().and_then(web_url);
        let page = RedirectPage {
            default_url: default_url.as_str(),
            ios_deep_link: ios_deep.as_ref().map(Url::as_str),
            ios_fallback_url: ios_fallback.as_ref().map(Url::as_str),
            android_deep_link: android_deep.as_ref().map(Url::as_str),
            android_fallback_url: android_fallback.as_ref().map(Url::as_str),
            og_title: fields
                .og_title
                .as_deref()
                .filter(|value| !value.trim().is_empty()),
            og_description: fields
                .og_description
                .as_deref()
                .filter(|value| !value.trim().is_empty()),
            og_image_url: image.as_ref().map(Url::as_str),
        };
        let payload = if [
            page.ios_deep_link,
            page.ios_fallback_url,
            page.android_deep_link,
            page.android_fallback_url,
            page.og_title,
            page.og_description,
            page.og_image_url,
        ]
        .into_iter()
        .all(|field| field.is_none())
        {
            RedirectPayload::Direct(
                HeaderValue::from_str(page.default_url)
                    .map_err(|_| AppError::Internal("Invalid Location header"))?,
            )
        } else {
            RedirectPayload::Html(Bytes::from(page.render()?))
        };
        Ok(Some(Self {
            random_key,
            payload,
            webhook_url: fields.webhook_url.as_deref().and_then(webhook_url),
        }))
    }

    pub fn weight(&self) -> u32 {
        let payload = match &self.payload {
            RedirectPayload::Direct(location) => location.as_bytes().len(),
            RedirectPayload::Html(html) => html.len(),
        };
        let webhook = self
            .webhook_url
            .as_ref()
            .map_or(0, |url| url.as_str().len());
        u32::try_from(payload + webhook).unwrap_or(u32::MAX - 256)
    }

    pub fn response(&self, csp: &PagePolicies) -> Response {
        let mut response = match &self.payload {
            RedirectPayload::Direct(location) => {
                (StatusCode::FOUND, [(header::LOCATION, location.clone())]).into_response()
            }
            RedirectPayload::Html(html) => (
                [
                    (
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("text/html; charset=utf-8"),
                    ),
                    (header::CONTENT_SECURITY_POLICY, csp.redirect.clone()),
                ],
                html.clone(),
            )
                .into_response(),
        };
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-cache"),
        );
        response
    }
}

pub(crate) struct PagePolicies {
    pub index: HeaderValue,
    redirect: HeaderValue,
}

impl PagePolicies {
    pub fn new() -> AppResult<Self> {
        Ok(Self {
            index: page_policy(INDEX_HTML, " img-src https: http: data:; connect-src 'self'; base-uri 'none'; form-action 'self'")?,
            redirect: page_policy(REDIRECT_HTML, " base-uri 'none'")?,
        })
    }
}

fn page_policy(html: &str, directives: &str) -> AppResult<HeaderValue> {
    let hash =
        extract_inline_script_hash(html).ok_or(AppError::Internal("Inline script is missing"))?;
    HeaderValue::from_str(&format!(
        "default-src 'none'; script-src 'sha256-{hash}'; style-src 'unsafe-inline';{directives}"
    ))
    .map_err(|_| AppError::Internal("Invalid Content-Security-Policy"))
}

fn extract_inline_script_hash(html: &str) -> Option<String> {
    let (_, script) = html.rsplit_once("<script>")?;
    let (script, _) = script.split_once("</script>")?;
    let normalized = script.replace("\r\n", "\n").replace('\r', "\n");
    Some(STANDARD.encode(Sha256::digest(normalized)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_hashes_last_bare_script_with_html_newline_normalization() {
        let page =
            "<script src=\"example\"></script><script>a()</script><script>some()\r\n</script>";
        let expected = STANDARD.encode(Sha256::digest(b"some()\n"));
        assert_eq!(extract_inline_script_hash(page), Some(expected));
    }
}
