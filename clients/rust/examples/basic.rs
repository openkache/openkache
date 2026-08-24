//! A complete Tokio round trip against an OpenKache endpoint.
//!
//! Set `OPENKACHE_ENDPOINT` and `OPENKACHE_DATA_PROTECTION_KEY` before running
//! this example. The endpoint must use a certificate trusted by the local
//! system trust store, or replace the builder's trust configuration with an
//! explicit [`Certificate`](openkache_client::Certificate).

use openkache_client::{Client, DataProtectionKey, DeleteOutcome, GetOutcome, SetOutcome};

#[tokio::main]
async fn main() -> openkache_client::Result<()> {
    let endpoint =
        std::env::var("OPENKACHE_ENDPOINT").unwrap_or_else(|_| "cache.example.com:4433".into());
    let encoded_key = std::env::var("OPENKACHE_DATA_PROTECTION_KEY").expect(
        "set OPENKACHE_DATA_PROTECTION_KEY to a padded or unpadded Base64-encoded 32-byte secret",
    );
    let protection_key = DataProtectionKey::from_base64(&encoded_key)?;
    let client = Client::connect(&endpoint, protection_key).await?;

    client.ping().await?;
    let set = client
        .set(b"example:greeting", b"hello from OpenKache")
        .await?;
    assert!(matches!(set, SetOutcome::Created | SetOutcome::Replaced));

    match client.get(b"example:greeting").await? {
        GetOutcome::Found(value) => {
            println!("{}", String::from_utf8_lossy(&value));
        }
        GetOutcome::NotFound => println!("the example key was not found"),
    }

    if client.delete(b"example:greeting").await? == DeleteOutcome::Deleted {
        println!("deleted example:greeting");
    }
    client.close().await
}
