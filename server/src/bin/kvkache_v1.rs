//! Main entry point for the kvkache-v1 cache server.
//! Parses CLI arguments, starts the multi-threaded cache, and dispatches
//! commands (get, set, delete, sync, stats) to the worker threads.

use std::error::Error;

use openkache::{AppConfig, Command, KvError, ThreadedKvkache};
use openkache_protocol::ItemId;
use sha2::{Digest, Sha256};

fn item_id(application_key: &[u8]) -> ItemId {
    ItemId::new(Sha256::digest(application_key).into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let (config, command) = match AppConfig::parse() {
        Ok(parsed) => parsed,
        Err(KvError::Usage(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        Err(error) => return Err(error.into()),
    };
    let mut cache = ThreadedKvkache::start(config)?;
    let operation = (|| -> Result<(), Box<dyn Error>> {
        match command {
            Command::Get(application_key) => match cache.get(item_id(&application_key))? {
                Some(value) => println!("{}", String::from_utf8_lossy(&value)),
                None => println!("(nil)"),
            },
            Command::Set(application_key, value) => {
                println!("{:?}", cache.set(item_id(&application_key), value)?)
            }
            Command::Delete(application_key) => {
                println!(
                    "{}",
                    if cache.delete(item_id(&application_key))? {
                        "Deleted"
                    } else {
                        "NotFound"
                    }
                );
            }
            Command::Sync => {
                cache.sync()?;
                println!("Synced");
            }
            Command::Stats => {
                for stats in cache.stats()? {
                    println!("{stats}");
                }
            }
        }
        Ok(())
    })();
    let shutdown = cache.shutdown();
    operation?;
    shutdown?;
    Ok(())
}
