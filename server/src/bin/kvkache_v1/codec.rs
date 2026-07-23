// Checkpoint byte codec, checksums, integer helpers, and the standalone CLI entry.

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.offset.saturating_add(length);
        if end > self.bytes.len() {
            return Err(KvError::Corrupt("truncated index checkpoint".into()));
        }
        let result = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(result)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn checksum64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn get_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn main() -> std::result::Result<(), Box<dyn Error>> {
    let (config, command) = match AppConfig::parse() {
        Ok(parsed) => parsed,
        Err(KvError::Usage(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        Err(error) => return Err(error.into()),
    };
    let mut cache = ThreadedKvkache::start(config)?;
    let operation = (|| -> Result<()> {
        match command {
            Command::Get(key) => match cache.get(key)? {
                Some(value) => println!("{}", String::from_utf8_lossy(&value)),
                None => println!("(nil)"),
            },
            Command::Set(key, value) => println!("{:?}", cache.set(key, value)?),
            Command::Delete(key) => {
                println!(
                    "{}",
                    if cache.delete(key)? {
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
