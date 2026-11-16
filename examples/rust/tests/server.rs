//! The axum server on a real ephemeral listener (127.0.0.1:0): each request answers in the
//! locale its `Accept-Language` negotiates, also while requests in other locales run at the
//! same time on other threads; the instance's own locale never changes.

use axum::Json;
use axum::extract::State;
use axum::routing::get;
use i18n_next::I18n;
use playground_rust::{app, app_with, catalog, fetch, with_server};
use std::sync::Arc;
use tokio::sync::Barrier;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_answer_in_their_own_locale() -> std::io::Result<()> {
    let i18n = catalog().map_err(std::io::Error::other)?;
    let check = i18n.clone();
    with_server(app(i18n), |addr| async move {
        let reply = fetch(addr, "/settings", "ru-RU,ru;q=0.9,en;q=0.5").await?;
        assert_eq!(reply.status, 200);
        assert_eq!(reply.content_language.as_deref(), Some("ru"));
        assert_eq!(
            reply.body,
            r#"{"locale":"ru","texts":{"language":"Язык","title":"Настройки"}}"#
        );

        // no header and an unknown language: the default; `ar` has no settings page: fallback
        for accept in ["", "fr"] {
            let reply = fetch(addr, "/settings", accept).await?;
            assert_eq!(reply.content_language.as_deref(), Some("en"));
            assert_eq!(reply.body, r#"{"locale":"en","texts":{"language":"Language","title":"Settings"}}"#);
        }
        let reply = fetch(addr, "/settings", "ar").await?;
        assert_eq!(reply.content_language.as_deref(), Some("ar"));
        assert_eq!(reply.body, r#"{"locale":"ar","texts":{"language":"Language","title":"Settings"}}"#);

        // plurals, a formatted number and linked messages, in parallel requests
        let tasks: Vec<_> = (0..16)
            .map(|i| {
                let accept = ["en", "ru", "ar", "ru;q=0.1, en"][i % 4];
                tokio::spawn(async move { (i % 4, fetch(addr, "/index", accept).await) })
            })
            .collect();
        for task in tasks {
            let (which, reply) = task.await.map_err(std::io::Error::other)?;
            let reply = reply?;
            assert_eq!(reply.status, 200);
            let body = reply.body;
            let want = match which {
                1 => r#"{"locale":"ru","texts":{"apples":"3 яблока","cart":"Корзина пуста","demo.price":"Цена: 1 234,5","footer":"Made with i18n-next","greeting":"Привет, Ann!","nav.home":"Главная","shout":"ГЛАВНАЯ!"}}"#,
                2 => r#"{"locale":"ar","texts":{"apples":"3 تفاحات","cart":"Your cart is empty","demo.price":"السعر: ١٬٢٣٤٫٥","footer":"Made with i18n-next","greeting":"مرحبا، Ann!","nav.home":"الرئيسية","shout":"الرئيسية!"}}"#,
                _ => r#"{"locale":"en","texts":{"apples":"3 apples","cart":"Your cart is empty","demo.price":"Price: 1,234.5","footer":"Made with i18n-next","greeting":"Hello, Ann!","nav.home":"Home","shout":"HOME!"}}"#,
            };
            assert_eq!(body.replace('\u{a0}', " "), want, "Accept-Language #{which}");
        }

        let reply = fetch(addr, "/missing-page", "en").await?;
        assert_eq!(reply.status, 404);
        Ok(())
    })
    .await?;
    assert_eq!(*check.locale(), *"en");
    Ok(())
}

/// Given a route that waits on a barrier after the middleware entered its scope, when a
/// `ru` and an `en` request are both parked there on one thread, then each resolves in its
/// own locale after the wait, and a lookup outside any scope on that thread gets the
/// default: a scope that stayed on the stack across a suspension (or after the request)
/// fails here.
#[tokio::test(flavor = "current_thread")]
async fn a_scope_survives_awaits_and_leaves_nothing_behind() -> std::io::Result<()> {
    let i18n = catalog().map_err(std::io::Error::other)?;
    let check = i18n.clone();
    let barrier = Arc::new(Barrier::new(2));
    let parked = get(move |State(i18n): State<I18n>| async move {
        barrier.wait().await;
        Json(i18n.text("title").page("settings"))
    });
    let router = app_with(i18n, axum::Router::new().route("/parked", parked));
    with_server(router, |addr| async move {
        let (ru, en) = tokio::join!(fetch(addr, "/parked", "ru"), fetch(addr, "/parked", "en"));
        let (ru, en) = (ru?, en?);
        assert_eq!((ru.status, ru.body.as_str()), (200, r#""Настройки""#));
        assert_eq!((en.status, en.body.as_str()), (200, r#""Settings""#));
        assert_eq!(ru.content_language.as_deref(), Some("ru"));
        Ok(())
    })
    .await?;
    assert_eq!(check.text("title").page("settings").to_string(), "Settings");
    Ok(())
}
