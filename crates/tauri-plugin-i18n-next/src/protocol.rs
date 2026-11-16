//! The `i18n-next://` protocol (DESIGN 6.1): the data channel. A pure function from a
//! request to a response, so tests and the model drive it without a WebView.
//!
//! - `GET /{tag}/{locale}/{page}.json`: one chunk (`index` is the root),
//!   `{"data":{…},"linked":{…}}` as `i18n_next::payload::chunk` writes it (`{"data":{}}`
//!   when the pair has no data);
//! - `GET /{tag}/bundle?locale=…&page=…`: the chunks of the locale's chain, roots and (with
//!   `page`) page chunks, as `load` sends them but without the meta fields.
//!
//! `{tag}` is the content tag of what the URL serves (16 hex digits): the chunk's own, or
//! for a bundle a hash of its chunks' tags. The `meta` command lists the current tags. A
//! change that leaves a chunk as it was keeps its URL, so switching `en → de → en` reads
//! `en` from the cache; one that changes it gives it a new URL. A response is
//! `application/json` with `ETag: "{tag}"` and `Cache-Control: immutable`. A request with
//! another tag than the current one gets `410 Gone` with the current tag and version in
//! the body (`{kind, message, tag, version}`) and in `X-I18n-Tag` / `X-I18n-Version`
//! (exposed to cross-origin `fetch`), never stale data under a new URL or new data under
//! an old one.
//!
//! The front end builds URLs with `convertFileSrc(path, 'i18n-next')`, which percent-encodes
//! the whole path, `?` and `=` included (`i18n-next://localhost/<tag>%2Fde%2Findex.json`, or
//! `http://i18n-next.localhost/…` on Windows and Android): the handler decodes the whole
//! path once before it splits it, so `path` holds plain names
//! (`` `${tag}/bundle?locale=${locale}&page=${page}` ``).
//!
//! Custom protocols have no ACL, so the handler takes only `GET` / `HEAD` (and answers the
//! CORS preflight `OPTIONS`), only names the catalog knows (8b.2: `404` with
//! `{kind, message}` otherwise) and only the app's own origins (`403` for any other
//! `Origin`; a request without one, like a plain navigation, passes).

use crate::commands::{known_locale, known_page};
use i18n_next::{I18n, payload};
use tauri::http::header::HeaderValue;
use tauri::http::{Method, Request, Response, StatusCode, header};

/// The header that carries the current catalog version on `410 Gone`.
pub const VERSION_HEADER: &str = "x-i18n-version";

/// The header that carries the current tag of the resource on `410 Gone`.
pub const TAG_HEADER: &str = "x-i18n-tag";

/// Headers a cross-origin `fetch` may read.
const EXPOSE: &str = "etag, x-i18n-tag, x-i18n-version";

/// Answers one protocol request. `origins` are the origins allowed to read (the app's own:
/// `tauri://localhost`, `http://tauri.localhost`, the dev server, `Builder::origins`).
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
/// let (tag, _) = i18n_next::payload::chunk(&i18n, "en", None);
/// let ok = get(&format!("i18n-next://localhost/{tag:016x}%2Fen%2Findex.json"));
/// assert_eq!((ok.status().as_u16(), ok.body().as_slice()), (200, &br#"{"data":{"a":"A"}}"#[..]));
/// assert_eq!(get(&format!("i18n-next://localhost/{tag:016x}/xx/index.json")).status().as_u16(), 404);
/// assert_eq!(get("i18n-next://localhost/0000000000000000/en/index.json").status().as_u16(), 410);
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
    let method = request.method();
    if method == Method::OPTIONS {
        return preflight(request, origin);
    }
    if method != Method::GET && method != Method::HEAD {
        let mut r = error(
            StatusCode::METHOD_NOT_ALLOWED,
            "MethodNotAllowed",
            "only GET, HEAD and OPTIONS",
            origin,
        );
        r.headers_mut().insert(
            header::ALLOW,
            HeaderValue::from_static("GET, HEAD, OPTIONS"),
        );
        return r;
    }
    let mut r = get(i18n, request, origin);
    if method == Method::HEAD {
        r.body_mut().clear();
    }
    r
}

/// `GET` (and `HEAD`, whose body the caller drops).
fn get<B>(i18n: &I18n, request: &Request<B>, origin: Option<&str>) -> Response<Vec<u8>> {
    let not_found = || {
        error(
            StatusCode::NOT_FOUND,
            "NotFound",
            "no such resource",
            origin,
        )
    };
    let uri = request.uri();
    // `convertFileSrc` encodes the whole path, `?` included: decode once, then split
    let decoded = match uri.query() {
        Some(q) => decode(&[uri.path(), "?", q].concat()),
        None => decode(uri.path()),
    };
    let Some(decoded) = decoded else {
        return not_found();
    };
    let (path, query) = match decoded.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (&*decoded, None),
    };
    let Some((tag, rest)) = path
        .strip_prefix('/')
        .and_then(|p| p.split_once('/'))
        .and_then(|(t, rest)| Some((parse_tag(t)?, rest)))
    else {
        return not_found();
    };
    let found = if rest == "bundle" {
        let mut locale = None;
        let mut page = None;
        for pair in query.unwrap_or("").split('&') {
            match pair.split_once('=') {
                Some(("locale", v)) => locale = Some(v),
                Some(("page", v)) => page = Some(v),
                _ => {}
            }
        }
        let Some(locale) = locale else {
            return not_found();
        };
        check(i18n, locale, page).map(|()| payload::bundle(i18n, locale, page, Some(tag)))
    } else {
        let Some((locale, page)) = rest.strip_suffix(".json").and_then(|r| r.split_once('/'))
        else {
            return not_found();
        };
        check(i18n, locale, Some(page)).map(|()| {
            // the tag first: a stale URL copies nothing
            let (current, json) = payload::chunk(i18n, locale, Some(page));
            (current, (current == tag).then(|| json.as_bytes().to_vec()))
        })
    };
    let (current, body) = match found {
        Ok(found) => found,
        Err(e) => {
            let body = serde_json::to_vec(&e).unwrap_or_default();
            return respond(StatusCode::NOT_FOUND, body, origin, None);
        }
    };
    let etag = format!("\"{current:016x}\"");
    let Some(body) = body else {
        let version = i18n.version();
        let body = format!(
            r#"{{"kind":"Gone","message":"the resource changed: read it under its current tag","tag":"{current:016x}","version":{version}}}"#
        );
        let mut r = respond(StatusCode::GONE, body.into_bytes(), origin, None);
        let h = r.headers_mut();
        h.insert(VERSION_HEADER, version.into());
        if let Ok(v) = HeaderValue::from_str(&etag[1..17]) {
            h.insert(TAG_HEADER, v);
        }
        return r;
    };
    // `If-None-Match` is a list of tags or `*`
    let fresh = request
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .map(|t| t.trim().trim_start_matches("W/"))
                .any(|t| t == etag || t == "*")
        });
    if fresh {
        return respond(StatusCode::NOT_MODIFIED, Vec::new(), origin, Some(&etag));
    }
    respond(StatusCode::OK, body, origin, Some(&etag))
}

/// The CORS preflight: `GET` / `HEAD` with whatever headers the page asks for (the data is
/// read-only), cached for a day. The origin was checked by the caller.
fn preflight<B>(request: &Request<B>, origin: Option<&str>) -> Response<Vec<u8>> {
    let mut r = respond(StatusCode::NO_CONTENT, Vec::new(), origin, None);
    let h = r.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, HEAD, OPTIONS"),
    );
    if let Some(asked) = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
    {
        h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, asked.clone());
    }
    h.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
    r
}

/// 16 hex digits.
fn parse_tag(s: &str) -> Option<u64> {
    (s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u64::from_str_radix(s, 16).ok())
        .flatten()
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

/// `{kind, message}` of a fixed `kind` and `message` (ASCII without quotes or escapes).
fn error(
    status: StatusCode,
    kind: &'static str,
    message: &'static str,
    origin: Option<&str>,
) -> Response<Vec<u8>> {
    let body = format!(r#"{{"kind":"{kind}","message":"{message}"}}"#);
    respond(status, body.into_bytes(), origin, None)
}

/// A JSON response; cacheable forever with an `etag` (a content-tagged URL), never without.
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
        b = b
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, o)
            .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, EXPOSE);
    }
    // every header value above is ASCII the caller checked (`origin` came from a header)
    b.body(body).unwrap_or_default()
}

/// `%XX`-decodes a path (with its query); `None` for a broken escape or non-UTF-8.
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

#[cfg(test)]
mod tests {
    /// Before the plugin is set up the scheme answers `503` with a JSON error body.
    #[test]
    fn unavailable_is_503_with_its_kind() {
        let r = super::unavailable();
        assert_eq!(r.status(), 503);
        let body: serde_json::Value = serde_json::from_slice(r.body()).unwrap();
        assert_eq!(body["kind"], "Unavailable");
    }
}
