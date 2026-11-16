//! The playground's axum server (DESIGN 8a): `cargo run -p playground-rust --bin server`, then
//! `curl -H 'Accept-Language: ru' localhost:3000/settings`. Each request runs in a scope of
//! its `Accept-Language` locale; the answer is JSON of `Text` through `Serialize`.

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let i18n = playground_rust::catalog().map_err(std::io::Error::other)?;
    let addr = std::env::var("ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, playground_rust::app(i18n)).await
}
