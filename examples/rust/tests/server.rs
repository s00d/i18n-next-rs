//! The axum server on a real ephemeral listener (127.0.0.1:0): each request answers in the
//! locale its `Accept-Language` negotiates, also while requests in other locales run at the
//! same time on other threads; the instance's own locale never changes.

use playground_rust::{catalog, fetch, with_server};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_answer_in_their_own_locale() -> std::io::Result<()> {
    let i18n = catalog().map_err(std::io::Error::other)?;
    let check = i18n.clone();
    with_server(i18n, |addr| async move {
        let (status, body) = fetch(addr, "/settings", "ru-RU,ru;q=0.9,en;q=0.5").await?;
        assert_eq!(status, 200);
        assert_eq!(
            body,
            r#"{"locale":"ru","texts":{"language":"Язык","title":"Настройки"}}"#
        );

        // no header and an unknown language: the default; `ar` has no settings page: fallback
        for accept in ["", "fr"] {
            let (_, body) = fetch(addr, "/settings", accept).await?;
            assert_eq!(body, r#"{"locale":"en","texts":{"language":"Language","title":"Settings"}}"#);
        }
        let (_, body) = fetch(addr, "/settings", "ar").await?;
        assert_eq!(body, r#"{"locale":"ar","texts":{"language":"Language","title":"Settings"}}"#);

        // plurals, a formatted number and linked messages, in parallel requests
        let tasks: Vec<_> = (0..16)
            .map(|i| {
                let accept = ["en", "ru", "ar", "ru;q=0.1, en"][i % 4];
                tokio::spawn(async move { (i % 4, fetch(addr, "/index", accept).await) })
            })
            .collect();
        for task in tasks {
            let (which, reply) = task.await.map_err(std::io::Error::other)?;
            let (status, body) = reply?;
            assert_eq!(status, 200);
            let want = match which {
                1 => r#"{"locale":"ru","texts":{"apples":"3 яблока","cart":"Корзина пуста","demo.price":"Цена: 1 234,5","footer":"Made with i18n-next","greeting":"Привет, Ann!","nav.home":"Главная","shout":"ГЛАВНАЯ!"}}"#,
                2 => r#"{"locale":"ar","texts":{"apples":"3 تفاحات","cart":"Your cart is empty","demo.price":"السعر: ١٬٢٣٤٫٥","footer":"Made with i18n-next","greeting":"مرحبا، Ann!","nav.home":"الرئيسية","shout":"الرئيسية!"}}"#,
                _ => r#"{"locale":"en","texts":{"apples":"3 apples","cart":"Your cart is empty","demo.price":"Price: 1,234.5","footer":"Made with i18n-next","greeting":"Hello, Ann!","nav.home":"Home","shout":"HOME!"}}"#,
            };
            assert_eq!(body.replace('\u{a0}', " "), want, "Accept-Language #{which}");
        }

        let (status, _) = fetch(addr, "/missing-page", "en").await?;
        assert_eq!(status, 404);
        Ok(())
    })
    .await?;
    assert_eq!(*check.locale(), *"en");
    Ok(())
}
