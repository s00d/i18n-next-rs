//! The `i18n-next://` protocol (DESIGN 6.1): the data channel. A pure function from a
//! request to a response, so tests and the model drive it without a WebView.
//!
//! - `GET /v{version}/{locale}/{page}.json`: one chunk (`index` is the root), the nested
//!   tree as JS `addRouteTranslations` takes it, `{}` when the pair has no data;
//! - `GET /v{version}/bundle?locale=…&page=…`: the payload of the locale's chain, roots and
//!   (with `page`) page chunks, as `load` sends it but without the meta fields.
//!
//! The front end builds URLs with `convertFileSrc(path, 'i18n-next')`
//! (`i18n-next://localhost/…`, or `http://i18n-next.localhost/…` on Windows and Android).
//! A response is `application/json` with `ETag` and `Cache-Control: immutable`: the version
//! is in the path, and every change makes a new one ([`crate::EVENT_CHANGED`]). A request
//! for another version than the current one gets `410 Gone` with the current version in
//! `X-I18n-Version`, never stale data under a new URL or new data under an old one.
//!
//! Custom protocols have no ACL, so the handler takes only `GET`, only names the catalog
//! knows (8b.2: `404` with `{kind, message}` otherwise) and only the app's own origins
//! (`403` for any other `Origin`; a request without one, like a plain navigation, passes).

use crate::commands::{known_locale, known_page};
use i18n_next::{I18n, payload};
use tauri::http::{Method, Request, Response, StatusCode, header};

/// The header that carries the current catalog version on `410 Gone`.
pub const VERSION_HEADER: &str = "x-i18n-version";

/// Answers one protocol request. `origins` are the origins allowed to read (the app's own:
/// `tauri://localhost`, `http://tauri.localhost`, the dev server).
///
/// ```
/// use tauri::http::Request;
///
/// let i18n = i18n_next::I18n::builder().messages("en", None, r#"{"a": "A"}"#).build()?;
/// let origins = ["tauri://localhost".to_owned()];
/// let get = |uri: &str| {
///     let req = Request::get(uri).header("origin", "tauri://localhost").body(()).unwrap();
///     tauri_plugin_i18n_next::protocol::handle(&i18n, &origins, &req)
/// };
/// let ok = get("i18n-next://localhost/v0/en/index.json");
/// assert_eq!((ok.status().as_u16(), ok.body().as_slice()), (200, &br#"{"a":"A"}"#[..]));
/// assert_eq!(get("i18n-next://localhost/v0/xx/index.json").status().as_u16(), 404);
/// assert_eq!(get("i18n-next://localhost/v9/en/index.json").status().as_u16(), 410);
/// # Ok::<(), i18n_next::Error>(())
/// ```
pub fn handle<B>(i18n: &I18n, origins: &[String], request: &Request<B>) -> Response<Vec<u8>> {
    let origin = match request.headers().get(header::ORIGIN) {
        None => None,
        Some(o) => match o.to_str().ok().filter(|o| origins.iter().any(|a| a == o)) {
            Some(o) => Some(o),
            None => {
                return error(
                    StatusCode::FORBIDDEN,
                    "Forbidden",
                    "origin not allowed",
                    None,
                );
            }
        },
    };
    if request.method() != Method::GET {
        let mut r = error(
            StatusCode::METHOD_NOT_ALLOWED,
            "MethodNotAllowed",
            "only GET",
            origin,
        );
        r.headers_mut()
            .insert(header::ALLOW, header::HeaderValue::from_static("GET"));
        return r;
    }
    let not_found = || {
        error(
            StatusCode::NOT_FOUND,
            "NotFound",
            "no such resource",
            origin,
        )
    };
    let uri = request.uri();
    let Some((version, rest)) = uri
        .path()
        .strip_prefix("/v")
        .and_then(|p| p.split_once('/'))
        .and_then(|(v, rest)| Some((v.parse::<u64>().ok()?, rest)))
    else {
        return not_found();
    };
    let named = if rest == "bundle" {
        let mut locale = None;
        let mut page = None;
        for pair in uri.query().unwrap_or("").split('&') {
            match pair.split_once('=') {
                Some(("locale", v)) => locale = decode(v),
                Some(("page", v)) => page = decode(v),
                _ => {}
            }
        }
        let Some(locale) = locale else {
            return not_found();
        };
        check(i18n, &locale, page.as_deref())
            .map(|()| payload::encode(i18n, Some(&locale), page.as_deref(), false))
    } else {
        let Some((locale, page)) = rest
            .strip_suffix(".json")
            .and_then(|r| r.split_once('/'))
            .and_then(|(l, p)| Some((decode(l)?, decode(p)?)))
        else {
            return not_found();
        };
        check(i18n, &locale, Some(&page)).map(|()| payload::chunk(i18n, &locale, Some(&page)))
    };
    let (current, body) = match named {
        Ok(found) => found,
        Err(e) => {
            let body = serde_json::to_vec(&e).unwrap_or_default();
            return respond(StatusCode::NOT_FOUND, body, origin, None);
        }
    };
    if current != version {
        let mut r = error(
            StatusCode::GONE,
            "Gone",
            "the catalog has a newer version",
            origin,
        );
        r.headers_mut().insert(VERSION_HEADER, current.into());
        return r;
    }
    let etag = format!("\"{current}\"");
    let fresh = request
        .headers()
        .get(header::IF_NONE_MATCH)
        .is_some_and(|v| v.as_bytes() == etag.as_bytes());
    if fresh {
        return respond(StatusCode::NOT_MODIFIED, Vec::new(), origin, Some(&etag));
    }
    respond(StatusCode::OK, body, origin, Some(&etag))
}

/// `503` while the plugin is not set up yet.
pub(crate) fn unavailable() -> Response<Vec<u8>> {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "Unavailable",
        "i18n-next is not set up",
        None,
    )
}

fn check(i18n: &I18n, locale: &str, page: Option<&str>) -> crate::Result<()> {
    known_locale(i18n, locale)?;
    page.map_or(Ok(()), |p| known_page(i18n, p))
}

fn error(status: StatusCode, kind: &str, message: &str, origin: Option<&str>) -> Response<Vec<u8>> {
    let body = serde_json::json!({ "kind": kind, "message": message }).to_string();
    respond(status, body.into_bytes(), origin, None)
}

/// A JSON response; cacheable forever with an `etag` (a versioned URL), never without.
fn respond(
    status: StatusCode,
    body: Vec<u8>,
    origin: Option<&str>,
    etag: Option<&str>,
) -> Response<Vec<u8>> {
    let mut b = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::VARY, "Origin");
    b = match etag {
        Some(tag) => b
            .header(header::ETAG, tag)
            .header(header::CACHE_CONTROL, "max-age=31536000, immutable"),
        None => b.header(header::CACHE_CONTROL, "no-store"),
    };
    if let Some(o) = origin {
        b = b.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
    }
    // every header value above is ASCII the caller checked (`origin` came from a header)
    b.body(body).unwrap_or_default()
}

/// `%XX`-decodes a path segment or query value; `None` for a broken escape or non-UTF-8.
fn decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = |c: Option<u8>| (c? as char).to_digit(16);
            let hi = hex(bytes.next())?;
            let lo = hex(bytes.next())?;
            out.push((hi * 16 + lo) as u8);
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).ok()
}
