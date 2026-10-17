use super::Event;
use crate::{validate, workspace::Graph};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, TrySendError},
        Arc,
    },
    thread,
    time::Duration,
};

pub(super) struct Batch {
    pub revision: u64,
    pub graphs: Vec<Graph>,
}

pub(super) struct ValidatorWorker {
    jobs: Option<mpsc::SyncSender<Batch>>,
    handle: Option<thread::JoinHandle<()>>,
    revision: Arc<AtomicU64>,
}

impl ValidatorWorker {
    pub fn spawn(events: mpsc::SyncSender<Event>, revision: Arc<AtomicU64>) -> Self {
        let worker_revision = revision.clone();
        let (jobs, work) = mpsc::sync_channel::<Batch>(1);
        let handle = thread::spawn(move || {
            while let Ok(batch) = work.recv() {
                let mut results = Vec::new();
                for graph in batch.graphs {
                    if worker_revision.load(Ordering::Relaxed) != batch.revision {
                        break;
                    }
                    results.push(validate::validate_cancellable(&graph, &|| {
                        worker_revision.load(Ordering::Relaxed) != batch.revision
                    }));
                }
                let mut event = Event::Validated(batch.revision, results);
                while worker_revision.load(Ordering::Relaxed) == batch.revision {
                    match events.try_send(event) {
                        Ok(()) => break,
                        Err(TrySendError::Disconnected(_)) => return,
                        Err(TrySendError::Full(value)) => {
                            event = value;
                            thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
            }
        });
        Self {
            jobs: Some(jobs),
            handle: Some(handle),
            revision,
        }
    }

    pub fn try_submit(&self, batch: Batch) -> Result<bool, String> {
        let jobs = self.jobs.as_ref().ok_or("Validator worker stopped")?;
        match jobs.try_send(batch) {
            Ok(()) => Ok(true),
            Err(TrySendError::Full(_)) => Ok(false),
            Err(TrySendError::Disconnected(_)) => Err("Validator worker stopped".into()),
        }
    }
}

impl Drop for ValidatorWorker {
    fn drop(&mut self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.jobs.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
