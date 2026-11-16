//! The parts of the playground shared by the tour (`main.rs`), the server (`bin/server.rs`),
//! its test and the Nuxt site's check (`--texts`): the catalog, the texts of each page, the
//! axum app (DESIGN 8a).

use axum::extract::{Extension, Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use i18n_next::{Arg, I18n, Text, scope};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};

/// `examples/`: `i18n.json` and `locales/`.
pub fn examples() -> PathBuf {
    FsPath::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The shared catalog as every playground reads it: `i18n.json`, `locales/` with pages,
/// linked messages.
///
/// # Errors
/// A file that does not load.
pub fn catalog() -> Result<I18n, i18n_next::Error> {
    I18n::builder()
        .config(examples().join("i18n.json"))
        .dir(examples().join("locales"))
        .linked_messages(true)
        .build()
}

/// The pages of `tauri-app` and `nuxt-site`.
pub const PAGES: [&str; 3] = ["index", "settings", "about"];

/// What a page shows, as `examples/nuxt-site/app/pages/*.vue` writes it: `None` for an
/// unknown page. Each `Text` resolves in the scope's locale when shown or serialized.
pub fn page_texts(i18n: &I18n, page: &str) -> Option<Vec<(&'static str, Text)>> {
    let on = |key: &'static str| i18n.text(key).page(page.to_owned());
    Some(match page {
        "index" => vec![
            ("nav.home", on("nav.home")),
            ("greeting", on("greeting").arg("name", "Ann")),
            ("apples", on("apples").count(3)),
            ("cart", on("cart").count(0)),
            (
                "demo.price",
                on("demo.price").arg(
                    "price",
                    Arg::Number {
                        value: 1234.5.into(),
                        format: None,
                    },
                ),
            ),
            ("footer", on("footer")),
            ("shout", on("shout")),
        ],
        "settings" => vec![("title", on("title")), ("language", on("language"))],
        "about" => vec![
            ("title", on("title")),
            ("body", on("body").arg("brand", "i18n-next")),
            ("source", on("source")),
        ],
        _ => return None,
    })
}

/// The server: `GET /{page}` answers `{"locale", "texts": {key: text}}` in the locale
/// `Accept-Language` asks for (else the default), with `Content-Language`.
pub fn app(i18n: I18n) -> Router {
    Router::new()
        .route("/{page}", get(page))
        .route_layer(middleware::from_fn_with_state(i18n.clone(), negotiate))
        .with_state(i18n)
}

/// The request's locale, from `Accept-Language`, for the request's future: handlers and the
/// serialization of their `Text` (inside `next.run`) resolve in it (`scope().wrap`).
async fn negotiate(State(i18n): State<I18n>, mut req: Request, next: Next) -> Response {
    let accept = req
        .headers()
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let codes: Vec<String> = i18n.locales().into_iter().map(|l| l.code).collect();
    let code = i18n_next::negotiate(accept, &codes).unwrap_or_else(|| i18n.locale().to_string());
    req.extensions_mut().insert(Locale(code.clone()));
    let mut res = scope().locale(&code).wrap(next.run(req)).await;
    if let Ok(v) = code.parse() {
        res.headers_mut().insert(header::CONTENT_LANGUAGE, v);
    }
    res
}

/// The negotiated locale, for the handler's answer.
#[derive(Clone)]
struct Locale(String);

#[derive(serde::Serialize)]
struct PageBody {
    locale: String,
    texts: BTreeMap<&'static str, Text>,
}

async fn page(
    State(i18n): State<I18n>,
    Extension(Locale(locale)): Extension<Locale>,
    Path(name): Path<String>,
) -> Response {
    let Some(texts) = page_texts(&i18n, &name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Json(PageBody {
        locale,
        texts: texts.into_iter().collect(),
    })
    .into_response()
}

/// A plain HTTP/1.1 `GET` (one request, `Connection: close`): the status and the body.
///
/// # Errors
/// The connection or a reply that is not HTTP.
pub async fn fetch(
    addr: SocketAddr,
    path: &str,
    accept_language: &str,
) -> std::io::Result<(u16, String)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nAccept-Language: {accept_language}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await?;
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not an HTTP reply");
    let status = reply
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad)?;
    let (_, body) = reply.split_once("\r\n\r\n").ok_or_else(bad)?;
    Ok((status, body.to_owned()))
}

/// `app(i18n)` on `127.0.0.1:0`, `f(address)` against it, then a graceful shutdown.
///
/// # Errors
/// What binding or `f` returns.
pub async fn with_server<F, Fut, T>(i18n: I18n, f: F) -> std::io::Result<T>
where
    F: FnOnce(SocketAddr) -> Fut,
    Fut: Future<Output = std::io::Result<T>>,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app(i18n))
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let out = f(addr).await;
    let _ = stop.send(());
    server.await.map_err(std::io::Error::other)??;
    out
}
