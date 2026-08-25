//! Complete Gate 0 CRUD example for a local development server.

use openkache::{Client, GetResult, SetOutcome, Value};

#[tokio::main]
async fn main() -> openkache::Result<()> {
    let endpoint = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:4433".to_owned());
    let client = Client::connect(endpoint).await?;

    assert_eq!(
        client.set("greeting", Value::text("hello")).await?,
        SetOutcome::Created
    );
    assert_eq!(
        client.get("greeting").await?,
        GetResult::Found(Value::text("hello"))
    );
    assert!(client.delete("greeting").await?);
    assert_eq!(client.get("greeting").await?, GetResult::Missing);

    client.close().await?;
    println!("Rust OpenKache CRUD smoke test passed");
    Ok(())
}
