//! Minimal Gate 0 client example.

use openkache::{Client, GetResult, SetOutcome, Value};

#[tokio::main]
async fn main() -> openkache::Result<()> {
    let client = Client::connect("127.0.0.1:4433").await?;
    let outcome = client.set("greeting", Value::text("hello")).await?;
    assert_eq!(outcome, SetOutcome::Created);
    assert_eq!(
        client.get("greeting").await?,
        GetResult::Found(Value::text("hello"))
    );
    client.close().await
}
