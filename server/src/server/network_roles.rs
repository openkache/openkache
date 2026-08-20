//! Network-role placement, startup reporting, and worker ownership.

use std::future::Future;

use crate::ThreadedKvkache;
use crate::channel::Sender;
use crate::network_runtime;

use super::{Result, ServerError};

pub(crate) type NetworkWorkerCompletion = (usize, std::result::Result<(), String>);

pub(crate) struct NetworkWorkerHandle {
    pub(super) stop: Sender<()>,
    pub(super) secondary_stop: Option<Sender<()>>,
    pub(super) thread: Option<std::thread::JoinHandle<()>>,
}

pub(crate) struct NetworkRolePlacement {
    worker_index: usize,
    cpu_id: usize,
    thread_name: String,
    entries: u32,
    event_interval: usize,
    stop: Sender<()>,
    secondary_stop: Option<Sender<()>>,
}

impl NetworkRolePlacement {
    pub(crate) fn new(
        worker_index: usize,
        cpu_id: usize,
        thread_name: String,
        entries: u32,
        event_interval: usize,
        stop: Sender<()>,
    ) -> Self {
        Self {
            worker_index,
            cpu_id,
            thread_name,
            entries,
            event_interval,
            stop,
            secondary_stop: None,
        }
    }

    pub(crate) fn with_secondary_stop(mut self, secondary_stop: Sender<()>) -> Self {
        self.secondary_stop = Some(secondary_stop);
        self
    }
}

pub(crate) struct NetworkWorkerReporter {
    pub(super) worker_id: usize,
    started: Option<Sender<std::result::Result<(), String>>>,
    finished: Option<Sender<NetworkWorkerCompletion>>,
}

impl NetworkWorkerReporter {
    pub(crate) fn new(
        worker_id: usize,
        started: Sender<std::result::Result<(), String>>,
        finished: Sender<(usize, std::result::Result<(), String>)>,
    ) -> Self {
        Self {
            worker_id,
            started: Some(started),
            finished: Some(finished),
        }
    }

    pub(crate) fn startup_failed(mut self, message: String) {
        if let Some(started) = self.started.take() {
            let _ = started.send(Err(message));
        }
    }

    pub(crate) fn started(&mut self) -> bool {
        self.started
            .take()
            .is_some_and(|started| started.send(Ok(())).is_ok())
    }

    fn take_completion_sender(&mut self) -> Sender<NetworkWorkerCompletion> {
        self.finished
            .take()
            .expect("network worker completion sender is available at launch")
    }
}

impl Drop for NetworkWorkerReporter {
    fn drop(&mut self) {
        if let Some(started) = self.started.take() {
            let failure = if std::thread::panicking() {
                format!("network worker {} panicked during startup", self.worker_id)
            } else {
                format!(
                    "network worker {} exited without reporting startup",
                    self.worker_id
                )
            };
            let _ = started.send(Err(failure));
        }
    }
}

pub(crate) struct NetworkTaskReporter {
    worker_id: usize,
    finished: Option<Sender<NetworkWorkerCompletion>>,
}

impl NetworkTaskReporter {
    pub(crate) fn new(worker_id: usize, finished: Sender<NetworkWorkerCompletion>) -> Self {
        Self {
            worker_id,
            finished: Some(finished),
        }
    }

    fn finish(mut self, result: std::result::Result<(), String>) {
        if let Some(finished) = self.finished.take() {
            let _ = finished.send((self.worker_id, result));
        }
    }
}

impl Drop for NetworkTaskReporter {
    fn drop(&mut self) {
        if let Some(finished) = self.finished.take() {
            let failure = if std::thread::panicking() {
                "panicked"
            } else {
                "exited without reporting completion"
            };
            let _ = finished.send((self.worker_id, Err(failure.into())));
        }
    }
}

async fn run_network_role_task<F, Fut>(
    task_reporter: NetworkTaskReporter,
    reporter: NetworkWorkerReporter,
    role: F,
) where
    F: FnOnce(NetworkWorkerReporter) -> Fut,
    Fut: Future<Output = Option<std::result::Result<(), String>>>,
{
    let result = role(reporter).await.unwrap_or(Ok(()));
    task_reporter.finish(result);
}

pub(crate) fn launch_network_role<F, Fut>(
    cache: &ThreadedKvkache,
    placement: NetworkRolePlacement,
    mut reporter: NetworkWorkerReporter,
    role: F,
) -> Result<NetworkWorkerHandle>
where
    F: FnOnce(NetworkWorkerReporter) -> Fut + Send + 'static,
    Fut: Future<Output = Option<std::result::Result<(), String>>> + 'static,
{
    let NetworkRolePlacement {
        worker_index,
        cpu_id,
        thread_name,
        entries,
        event_interval,
        stop,
        secondary_stop,
    } = placement;
    let worker_id = reporter.worker_id;
    let finished = reporter.take_completion_sender();
    if cache.can_run_on_storage_cpu(cpu_id) {
        let attached = cache.run_on_storage_cpu(cpu_id, move || {
            let task_reporter = NetworkTaskReporter::new(worker_id, finished);
            network_runtime::spawn_detached(run_network_role_task(task_reporter, reporter, role));
        })?;
        if !attached {
            return Err(ServerError::NetworkWorker(format!(
                "storage runtime on CPU {cpu_id} rejected its prepared network role"
            )));
        }
        return Ok(NetworkWorkerHandle {
            stop,
            secondary_stop,
            thread: None,
        });
    }

    let thread = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let task_reporter = NetworkTaskReporter::new(worker_id, finished);
            if let Err(error) = network_runtime::run(
                network_runtime::RuntimeConfig {
                    entries,
                    event_interval,
                    cpu_id: Some(cpu_id),
                    worker_index: Some(worker_index),
                },
                run_network_role_task(task_reporter, reporter, role),
            ) {
                eprintln!("network worker {worker_id} runtime failed: {error}");
            }
        })?;
    Ok(NetworkWorkerHandle {
        stop,
        secondary_stop,
        thread: Some(thread),
    })
}
