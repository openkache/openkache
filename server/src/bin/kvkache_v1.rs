//! Main entry point for the kvkache-v1 cache server.
//! Parses CLI arguments, starts the multi-threaded cache, and dispatches
//! commands (get, set, delete, sync, stats) to the worker threads.

use std::error::Error;

use openkache::{AppConfig, Command, ItemId, KvError, StorageKey, ThreadedKvkache};
use sha2::{Digest, Sha256};

fn item_id(application_key: &[u8]) -> ItemId {
    ItemId::new(Sha256::digest(application_key).into())
}

fn storage_key(cache: &ThreadedKvkache, application_key: &[u8]) -> StorageKey {
    cache.storage_key_for_item_id(item_id(application_key))
}

fn main() -> Result<(), Box<dyn Error>> {
    let runtime = compio::runtime::Runtime::new()?;
    runtime.block_on(run())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let (config, command) = match AppConfig::parse() {
        Ok(parsed) => parsed,
        Err(KvError::Usage(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        Err(error) => return Err(error.into()),
    };
    let mut cache = ThreadedKvkache::start(config)?;
    let operation: Result<(), Box<dyn Error>> = async {
        match command {
            Command::Get(application_key) => {
                match cache.get(storage_key(&cache, &application_key)).await? {
                    Some(value) => println!("{}", String::from_utf8_lossy(&value)),
                    None => println!("(nil)"),
                }
            }
            Command::Set(application_key, value) => {
                println!(
                    "{:?}",
                    cache
                        .set(storage_key(&cache, &application_key), value)
                        .await?
                )
            }
            Command::Delete(application_key) => {
                println!(
                    "{}",
                    if cache.delete(storage_key(&cache, &application_key)).await? {
                        "Deleted"
                    } else {
                        "NotFound"
                    }
                );
            }
            Command::Sync => {
                cache.sync().await?;
                println!("Synced");
            }
            Command::Stats => {
                for stats in cache.stats().await? {
                    println!("{stats}");
                }
            }
        }
        Ok(())
    }
    .await;
    let shutdown = cache.shutdown();
    operation?;
    shutdown?;
    Ok(())
}
