//! Complete CRUD example for a local development server.

use openkache::{Client, SetOutcome, Value};

#[tokio::main]
async fn main() -> openkache::Result<()> {
    let endpoint = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:4433".to_owned());
    let client = Client::connect(endpoint).await?;

    assert_eq!(client.set("greeting", "hello").await?, SetOutcome::Created);
    assert_eq!(client.get("greeting").await?.unwrap(), Value::text("hello"));

    assert!(client.delete("greeting").await?);
    assert_eq!(client.get("greeting").await?, None);

    client.close().await?;
    println!("Rust OpenKache CRUD smoke test passed");
    Ok(())
}
