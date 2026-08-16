//! Durable, append-only membership metadata for the namespace registry.
//!
//! Namespace lifecycle metadata remains in the existing v2 snapshot.  Item and
//! worker membership changes use this journal so a SET/DELETE does not rewrite
//! and synchronise the complete namespace snapshot.  A dedicated writer thread
//! batches records and acknowledges them only after the journal has been
//! synchronised.  This keeps the conservative "record before storage mutation"
//! invariant without making the network worker perform filesystem sync.

use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crc32fast::hash;
use openkache_protocol::ITEM_ID_BYTES;

const JOURNAL_MAGIC: &[u8; 8] = b"OKNJNL01";
const JOURNAL_VERSION: u32 = 1;
const JOURNAL_HEADER_BYTES: usize = JOURNAL_MAGIC.len() + std::mem::size_of::<u32>();
const JOURNAL_RECORD_BYTES: usize = 56;
const JOURNAL_BATCH_WINDOW: Duration = Duration::from_micros(50);
static NEXT_COMPACTION_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalEvent {
    ReserveItem {
        namespace_id: u64,
        item_id: [u8; ITEM_ID_BYTES],
        route: u64,
        inserted_item: bool,
        inserted_worker: bool,
    },
    RollbackItem {
        namespace_id: u64,
        item_id: [u8; ITEM_ID_BYTES],
        route: u64,
        remove_item: bool,
        remove_worker: bool,
    },
    ReserveWorker {
        namespace_id: u64,
        route: u64,
    },
    MarkWorkersClean {
        namespace_id: u64,
    },
    MarkDelete {
        namespace_id: u64,
        item_id: [u8; ITEM_ID_BYTES],
    },
    PruneItem {
        namespace_id: u64,
        item_id: [u8; ITEM_ID_BYTES],
    },
}

enum Command {
    Append {
        event: JournalEvent,
        ack: SyncSender<io::Result<()>>,
    },
    Compact {
        snapshot: Vec<u8>,
        ack: SyncSender<io::Result<()>>,
    },
    Shutdown,
}

/// A journal writer with a bounded command queue and one dedicated filesystem
/// worker.  The queue is intentionally bounded so metadata pressure cannot
/// grow without limit under a stalled storage device.
pub(crate) struct NamespaceJournal {
    sender: SyncSender<Command>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl NamespaceJournal {
    pub(crate) fn load_events(path: &Path) -> io::Result<Vec<JournalEvent>> {
        let mut file = match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let (events, valid_len) = decode_events(&bytes)?;
        if valid_len != bytes.len() {
            // A process can stop after a record write but before the next
            // record is complete.  Remove only that incomplete tail before
            // appending new events; a complete record with a bad checksum
            // remains a hard recovery error.
            file.set_len(valid_len as u64)?;
            file.sync_all()?;
        }
        Ok(events)
    }

    pub(crate) fn start(path: &Path) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)?;
        if file.metadata()?.len() == 0 {
            write_header(&mut file)?;
            file.sync_all()?;
        } else {
            validate_header(&mut file)?;
        }
        let (sender, receiver) = mpsc::sync_channel(256);
        let path = path.to_owned();
        let thread = thread::Builder::new()
            .name("openkache-namespace-journal".to_owned())
            .spawn(move || run_writer(file, path, receiver))
            .map_err(|error| io::Error::other(format!("namespace journal thread: {error}")))?;
        Ok(Self {
            sender,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub(crate) fn append(&self, event: JournalEvent) -> io::Result<()> {
        let (ack, result) = mpsc::sync_channel(1);
        self.sender
            .send(Command::Append { event, ack })
            .map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "namespace journal stopped"))?;
        result
            .recv()
            .map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "namespace journal stopped"))?
    }

    pub(crate) fn compact(&self, snapshot: Vec<u8>) -> io::Result<()> {
        let (ack, result) = mpsc::sync_channel(1);
        self.sender
            .send(Command::Compact { snapshot, ack })
            .map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "namespace journal stopped"))?;
        result
            .recv()
            .map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "namespace journal stopped"))?
    }
}

impl Drop for NamespaceJournal {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Shutdown);
        let Ok(mut thread) = self.thread.lock() else {
            return;
        };
        if let Some(thread) = thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_writer(mut file: File, path: PathBuf, receiver: Receiver<Command>) {
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Append { event, ack } => {
                let mut pending = vec![(event, ack)];
                let mut control = None;
                loop {
                    match receiver.recv_timeout(JOURNAL_BATCH_WINDOW) {
                        Ok(Command::Append { event, ack }) => pending.push((event, ack)),
                        Ok(command) => {
                            control = Some(command);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                let result = append_batch(&mut file, &pending);
                for (_, ack) in pending {
                    let ack_result = result
                        .as_ref()
                        .map(|()| ())
                        .map_err(|error| clone_error(error));
                    let _ = ack.send(ack_result);
                }
                if result.is_err() {
                    if let Some(command) = control {
                        reject_command(command);
                    }
                    break;
                }
                if let Some(command) = control
                    && !handle_control(&mut file, &path, command)
                {
                    break;
                }
            }
            command => {
                if !handle_control(&mut file, &path, command) {
                    break;
                }
            }
        }
    }
}

fn append_batch(
    file: &mut File,
    pending: &[(JournalEvent, SyncSender<io::Result<()>>)],
) -> io::Result<()> {
    for (event, _) in pending {
        let record = encode_event(*event);
        file.write_all(&record)?;
    }
    file.sync_all()
}

fn handle_control(file: &mut File, path: &Path, command: Command) -> bool {
    match command {
        Command::Compact { snapshot, ack } => {
            let result = compact_snapshot(file, path, &snapshot);
            let keep_running = result.is_ok();
            let _ = ack.send(result);
            keep_running
        }
        Command::Shutdown => false,
        Command::Append { .. } => unreachable!("append is handled by the batch loop"),
    }
}

fn reject_command(command: Command) {
    let error = || io::Error::new(ErrorKind::BrokenPipe, "namespace journal stopped");
    match command {
        Command::Append { ack, .. } | Command::Compact { ack, .. } => {
            let _ = ack.send(Err(error()));
        }
        Command::Shutdown => {}
    }
}

fn compact_snapshot(file: &mut File, path: &Path, snapshot: &[u8]) -> io::Result<()> {
    let sequence = NEXT_COMPACTION_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary_path =
        path.with_extension(format!("snapshot-tmp-{}-{sequence}", std::process::id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut temporary = options.open(&temporary_path)?;
        temporary.write_all(snapshot)?;
        temporary.sync_all()?;
        drop(temporary);
        let snapshot_path = path.with_extension("");
        std::fs::rename(&temporary_path, snapshot_path)?;

        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        write_header(file)?;
        file.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    result
}

fn write_header(file: &mut File) -> io::Result<()> {
    file.write_all(JOURNAL_MAGIC)?;
    file.write_all(&JOURNAL_VERSION.to_be_bytes())
}

fn validate_header(file: &mut File) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0; JOURNAL_HEADER_BYTES];
    file.read_exact(&mut header)?;
    if &header[..JOURNAL_MAGIC.len()] != JOURNAL_MAGIC
        || u32::from_be_bytes(
            header[JOURNAL_MAGIC.len()..]
                .try_into()
                .expect("journal version width is fixed"),
        ) != JOURNAL_VERSION
    {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "namespace journal header is invalid",
        ));
    }
    file.seek(SeekFrom::End(0))?;
    Ok(())
}

fn decode_events(bytes: &[u8]) -> io::Result<(Vec<JournalEvent>, usize)> {
    if bytes.is_empty() {
        return Ok((Vec::new(), 0));
    }
    if bytes.len() < JOURNAL_HEADER_BYTES
        || &bytes[..JOURNAL_MAGIC.len()] != JOURNAL_MAGIC
        || u32::from_be_bytes(
            bytes[JOURNAL_MAGIC.len()..JOURNAL_HEADER_BYTES]
                .try_into()
                .expect("journal version width is fixed"),
        ) != JOURNAL_VERSION
    {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "namespace journal header is invalid",
        ));
    }
    let records = &bytes[JOURNAL_HEADER_BYTES..];
    let complete_len = records.len() / JOURNAL_RECORD_BYTES * JOURNAL_RECORD_BYTES;
    let mut events = Vec::with_capacity(complete_len / JOURNAL_RECORD_BYTES);
    for record in records[..complete_len].chunks_exact(JOURNAL_RECORD_BYTES) {
        if hash(&record[..JOURNAL_RECORD_BYTES - 4])
            != u32::from_be_bytes(
                record[JOURNAL_RECORD_BYTES - 4..]
                    .try_into()
                    .expect("journal checksum width is fixed"),
            )
        {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "namespace journal record checksum is invalid",
            ));
        }
        events.push(decode_event(record)?);
    }
    Ok((events, JOURNAL_HEADER_BYTES + complete_len))
}

fn encode_event(event: JournalEvent) -> [u8; JOURNAL_RECORD_BYTES] {
    let mut record = [0; JOURNAL_RECORD_BYTES];
    let (tag, flags, namespace_id, item_id, route) = match event {
        JournalEvent::ReserveItem {
            namespace_id,
            item_id,
            route,
            inserted_item,
            inserted_worker,
        } => (
            0,
            u8::from(inserted_item) | (u8::from(inserted_worker) << 1),
            namespace_id,
            item_id,
            route,
        ),
        JournalEvent::RollbackItem {
            namespace_id,
            item_id,
            route,
            remove_item,
            remove_worker,
        } => (
            1,
            u8::from(remove_item) | (u8::from(remove_worker) << 1),
            namespace_id,
            item_id,
            route,
        ),
        JournalEvent::ReserveWorker {
            namespace_id,
            route,
        } => (2, 0, namespace_id, [0; ITEM_ID_BYTES], route),
        JournalEvent::MarkWorkersClean { namespace_id } => {
            (3, 0, namespace_id, [0; ITEM_ID_BYTES], 0)
        }
        JournalEvent::MarkDelete {
            namespace_id,
            item_id,
        } => (4, 0, namespace_id, item_id, 0),
        JournalEvent::PruneItem {
            namespace_id,
            item_id,
        } => (5, 0, namespace_id, item_id, 0),
    };
    record[0] = tag;
    record[1] = flags;
    record[4..12].copy_from_slice(&namespace_id.to_be_bytes());
    record[12..12 + ITEM_ID_BYTES].copy_from_slice(&item_id);
    record[44..52].copy_from_slice(&route.to_be_bytes());
    let checksum = hash(&record[..52]).to_be_bytes();
    record[52..56].copy_from_slice(&checksum);
    record
}

fn decode_event(record: &[u8]) -> io::Result<JournalEvent> {
    if record.len() != JOURNAL_RECORD_BYTES || record[2..4] != [0, 0] {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "namespace journal record is malformed",
        ));
    }
    let namespace_id = u64::from_be_bytes(record[4..12].try_into().expect("u64 width is fixed"));
    let item_id = record[12..12 + ITEM_ID_BYTES]
        .try_into()
        .expect("item ID width is fixed");
    let route = u64::from_be_bytes(record[44..52].try_into().expect("u64 width is fixed"));
    let flags = record[1];
    match record[0] {
        0 => Ok(JournalEvent::ReserveItem {
            namespace_id,
            item_id,
            route,
            inserted_item: flags & 1 != 0,
            inserted_worker: flags & 2 != 0,
        }),
        1 => Ok(JournalEvent::RollbackItem {
            namespace_id,
            item_id,
            route,
            remove_item: flags & 1 != 0,
            remove_worker: flags & 2 != 0,
        }),
        2 if flags == 0 => Ok(JournalEvent::ReserveWorker {
            namespace_id,
            route,
        }),
        3 if flags == 0 && route == 0 && item_id == [0; ITEM_ID_BYTES] => {
            Ok(JournalEvent::MarkWorkersClean { namespace_id })
        }
        4 if flags == 0 && route == 0 => Ok(JournalEvent::MarkDelete {
            namespace_id,
            item_id,
        }),
        5 if flags == 0 && route == 0 => Ok(JournalEvent::PruneItem {
            namespace_id,
            item_id,
        }),
        _ => Err(io::Error::new(
            ErrorKind::InvalidData,
            "namespace journal event is unknown",
        )),
    }
}

fn clone_error(error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), error.to_string())
}
