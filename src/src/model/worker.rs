use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

use tokio::sync::{mpsc, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::model::actions::{WorkerCommand, WorkerEvent};
use crate::model::scanner::scan_directory_with_report;
use crate::model::search::search_files;
use crate::model::state::FileEntry;

pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);

pub struct Worker {
    files: Arc<RwLock<Vec<FileEntry>>>,
    search_task: Option<JoinHandle<()>>,
    search_cancel: Option<CancellationToken>,
    scan_task: Option<JoinHandle<()>>,
    search_generation: Arc<AtomicU64>,
    scan_generation: Arc<AtomicU64>,
    extraction_slots: Arc<Semaphore>,
}

impl Default for Worker {
    fn default() -> Self {
        Self {
            files: Arc::new(RwLock::new(Vec::new())),
            search_task: None,
            search_cancel: None,
            scan_task: None,
            search_generation: Arc::new(AtomicU64::new(0)),
            scan_generation: Arc::new(AtomicU64::new(0)),
            extraction_slots: Arc::new(Semaphore::new(1)),
        }
    }
}

impl Worker {
    pub async fn run(
        mut self,
        mut commands: mpsc::Receiver<WorkerCommand>,
        events: mpsc::Sender<WorkerEvent>,
    ) {
        while let Some(command) = commands.recv().await {
            match command {
                WorkerCommand::Scan { root } => self.start_scan(root, &events).await,
                WorkerCommand::QueryChanged { query } => self.start_search(query, &events).await,
                WorkerCommand::Shutdown => break,
            }
        }

        self.cancel_search();
        self.cancel_scan();
    }

    async fn start_scan(&mut self, root: std::path::PathBuf, events: &mpsc::Sender<WorkerEvent>) {
        self.cancel_search();
        self.cancel_scan();
        let generation = self.next_scan_generation();
        let _ = events.send(WorkerEvent::ScanStarted).await;

        let files = Arc::clone(&self.files);
        let current_generation = Arc::clone(&self.scan_generation);
        let events = events.clone();
        self.scan_task = Some(tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || scan_directory_with_report(&root)).await;
            if current_generation.load(Ordering::Acquire) != generation {
                return;
            }

            match result {
                Ok(Ok(report)) => {
                    let mut current_files = files.write().await;
                    if current_generation.load(Ordering::Acquire) != generation {
                        return;
                    }
                    *current_files = report.files.clone();
                    drop(current_files);

                    if current_generation.load(Ordering::Acquire) == generation {
                        let _ = events
                            .send(WorkerEvent::ScanCompleted {
                                files: report.files,
                                oversized_files: report.oversized_files,
                                unreadable_entries: report.unreadable_entries,
                            })
                            .await;
                    }
                }
                Ok(Err(error)) => {
                    if current_generation.load(Ordering::Acquire) == generation {
                        let _ = events.send(WorkerEvent::ScanError(error.to_string())).await;
                    }
                }
                Err(error) => {
                    if current_generation.load(Ordering::Acquire) == generation {
                        let _ = events
                            .send(WorkerEvent::ScanError(format!("Scanner task failed: {error}")))
                            .await;
                    }
                }
            }
        }));
    }

    async fn start_search(&mut self, query: String, events: &mpsc::Sender<WorkerEvent>) {
        self.cancel_search();
        let generation = self.next_generation();

        if query.trim().is_empty() {
            let _ = events
                .send(WorkerEvent::SearchCompleted {
                    query,
                    results: Vec::new(),
                })
                .await;
            return;
        }

        let cancel = CancellationToken::new();
        let child_cancel = cancel.clone();
        let files = Arc::clone(&self.files);
        let events = events.clone();
        let query_for_task = query;
        let current_generation = Arc::clone(&self.search_generation);
        let extraction_slots = Arc::clone(&self.extraction_slots);
        self.search_cancel = Some(cancel);
        self.search_task = Some(tokio::spawn(async move {
            tokio::select! {
                _ = child_cancel.cancelled() => return,
                _ = tokio::time::sleep(SEARCH_DEBOUNCE) => {}
            }

            let query_for_event = query_for_task.clone();
            if !send_search_event(
                &events,
                &child_cancel,
                &current_generation,
                generation,
                WorkerEvent::SearchStarted {
                    query: query_for_event,
                },
            )
            .await
            {
                return;
            }

            let file_list = files.read().await.clone();
            let total = file_list.len();
            let mut results = Vec::new();

            for (index, file) in file_list.into_iter().enumerate() {
                if child_cancel.is_cancelled()
                    || current_generation.load(Ordering::Acquire) != generation
                {
                    return;
                }

                let permit = tokio::select! {
                    biased;
                    _ = child_cancel.cancelled() => return,
                    permit = Arc::clone(&extraction_slots).acquire_owned() => {
                        match permit {
                            Ok(permit) => permit,
                            Err(_) => return,
                        }
                    }
                };
                let query_for_file = query_for_task.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let _permit: OwnedSemaphorePermit = permit;
                    search_files(std::slice::from_ref(&file), &query_for_file)
                })
                .await;

                if child_cancel.is_cancelled()
                    || current_generation.load(Ordering::Acquire) != generation
                {
                    return;
                }

                match result {
                    Ok(Ok(mut matches)) => results.append(&mut matches),
                    Ok(Err(error)) => {
                        let _ = send_search_event(
                            &events,
                            &child_cancel,
                            &current_generation,
                            generation,
                            WorkerEvent::SearchError {
                                query: query_for_task.clone(),
                                error: error.to_string(),
                            },
                        )
                        .await;
                        return;
                    }
                    Err(error) => {
                        let _ = send_search_event(
                            &events,
                            &child_cancel,
                            &current_generation,
                            generation,
                            WorkerEvent::SearchError {
                                query: query_for_task.clone(),
                                error: format!("Search task failed: {error}"),
                            },
                        )
                        .await;
                        return;
                    }
                }

                if !send_search_event(
                    &events,
                    &child_cancel,
                    &current_generation,
                    generation,
                    WorkerEvent::SearchProgress {
                        completed: index + 1,
                        total,
                    },
                )
                .await
                {
                    return;
                }
            }

            let _ = send_search_event(
                &events,
                &child_cancel,
                &current_generation,
                generation,
                WorkerEvent::SearchCompleted {
                    query: query_for_task,
                    results,
                },
            )
            .await;
        }));
    }

    fn next_generation(&self) -> u64 {
        self.search_generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn cancel_search(&mut self) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.cancel();
        }
        if let Some(task) = self.search_task.take() {
            task.abort();
        }
    }

    fn cancel_scan(&mut self) {
        if let Some(task) = self.scan_task.take() {
            task.abort();
        }
    }
}

async fn send_search_event(
    events: &mpsc::Sender<WorkerEvent>,
    cancel: &CancellationToken,
    generation: &AtomicU64,
    expected_generation: u64,
    event: WorkerEvent,
) -> bool {
    if cancel.is_cancelled() || generation.load(Ordering::Acquire) != expected_generation {
        return false;
    }

    tokio::select! {
        biased;
        _ = cancel.cancelled() => false,
        result = events.send(event) => result.is_ok(),
    }
}

pub fn channel(capacity: usize) -> (mpsc::Sender<WorkerCommand>, mpsc::Receiver<WorkerCommand>) {
    mpsc::channel(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::actions::WorkerCommand;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn empty_query_completes_without_searching() {
        let (command_tx, command_rx) = channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let worker = tokio::spawn(Worker::default().run(command_rx, event_tx));

        command_tx
            .send(WorkerCommand::QueryChanged {
                query: "  ".to_owned(),
            })
            .await
            .unwrap();
        assert!(matches!(
            event_rx.recv().await,
            Some(WorkerEvent::SearchCompleted { results, .. }) if results.is_empty()
        ));
        command_tx.send(WorkerCommand::Shutdown).await.unwrap();
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn scan_then_search_returns_results() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("one.txt"), "needle").unwrap();
        let (command_tx, command_rx) = channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(32);
        let worker = tokio::spawn(Worker::default().run(command_rx, event_tx));

        command_tx
            .send(WorkerCommand::Scan {
                root: root.path().to_path_buf(),
            })
            .await
            .unwrap();
        assert!(matches!(event_rx.recv().await, Some(WorkerEvent::ScanStarted)));
        assert!(matches!(
            event_rx.recv().await,
            Some(WorkerEvent::ScanCompleted { .. })
        ));
        command_tx
            .send(WorkerCommand::QueryChanged {
                query: "needle".to_owned(),
            })
            .await
            .unwrap();

        let mut completed = false;
        while let Some(event) = event_rx.recv().await {
            if matches!(event, WorkerEvent::SearchCompleted { .. }) {
                completed = true;
                break;
            }
        }
        assert!(completed);
        command_tx.send(WorkerCommand::Shutdown).await.unwrap();
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn newer_query_supersedes_debounced_query() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("one.txt"), "second").unwrap();
        let (command_tx, command_rx) = channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(32);
        let worker = tokio::spawn(Worker::default().run(command_rx, event_tx));

        command_tx
            .send(WorkerCommand::Scan {
                root: root.path().to_path_buf(),
            })
            .await
            .unwrap();
        assert!(matches!(event_rx.recv().await, Some(WorkerEvent::ScanStarted)));
        assert!(matches!(
            event_rx.recv().await,
            Some(WorkerEvent::ScanCompleted { .. })
        ));

        command_tx
            .send(WorkerCommand::QueryChanged {
                query: "first".to_owned(),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        command_tx
            .send(WorkerCommand::QueryChanged {
                query: "second".to_owned(),
            })
            .await
            .unwrap();

        let completion = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(WorkerEvent::SearchCompleted { query, .. }) = event_rx.recv().await {
                    break query;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(completion, "second");

        command_tx.send(WorkerCommand::Shutdown).await.unwrap();
        worker.await.unwrap();
    }
}
