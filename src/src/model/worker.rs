use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

use tokio::sync::{mpsc, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::model::actions::{WorkerCommand, WorkerEvent};
use crate::model::scanner::scan_directory_with_report;
use crate::model::search::search_files;
use crate::model::state::FileEntry;

pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);

fn default_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4)
        .clamp(2, 8)
}

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
            extraction_slots: Arc::new(Semaphore::new(default_parallelism())),
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
            let result =
                tokio::task::spawn_blocking(move || scan_directory_with_report(&root)).await;
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
                            .send(WorkerEvent::ScanError(format!(
                                "Scanner task failed: {error}"
                            )))
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
            let progress_step = (total / 40).max(1);
            let mut join_set = JoinSet::new();

            for file in file_list {
                if child_cancel.is_cancelled()
                    || current_generation.load(Ordering::Acquire) != generation
                {
                    join_set.abort_all();
                    return;
                }

                let slots = Arc::clone(&extraction_slots);
                let cancel_token = child_cancel.clone();
                let query_for_file = query_for_task.clone();

                join_set.spawn(async move {
                    let permit: OwnedSemaphorePermit = tokio::select! {
                        biased;
                        _ = cancel_token.cancelled() => return Ok(Vec::new()),
                        permit = slots.acquire_owned() => {
                            match permit {
                                Ok(permit) => permit,
                                Err(_) => return Ok(Vec::new()),
                            }
                        }
                    };

                    if cancel_token.is_cancelled() {
                        return Ok(Vec::new());
                    }

                    let relative_path = file.relative_path.clone();
                    let blocking_res = tokio::task::spawn_blocking(move || {
                        let _permit = permit;
                        search_files(std::slice::from_ref(&file), &query_for_file)
                    })
                    .await;

                    match blocking_res {
                        Ok(Ok(matches)) => Ok(matches),
                        Ok(Err(error)) => {
                            tracing::warn!(
                                path = %relative_path,
                                %error,
                                "Skipping file due to extraction/search error"
                            );
                            Ok(Vec::new())
                        }
                        Err(error) => {
                            tracing::warn!(
                                path = %relative_path,
                                %error,
                                "Skipping file because extraction task panicked"
                            );
                            Ok::<_, ()>(Vec::new())
                        }
                    }
                });
            }

            let mut results = Vec::new();
            let mut completed = 0usize;

            while let Some(joined) = tokio::select! {
                biased;
                _ = child_cancel.cancelled() => {
                    join_set.abort_all();
                    return;
                }
                next = join_set.join_next() => next,
            } {
                if child_cancel.is_cancelled()
                    || current_generation.load(Ordering::Acquire) != generation
                {
                    join_set.abort_all();
                    return;
                }

                if let Ok(Ok(mut file_matches)) = joined {
                    results.append(&mut file_matches);
                }

                completed += 1;
                if (completed == total || completed.is_multiple_of(progress_step))
                    && !send_search_event(
                        &events,
                        &child_cancel,
                        &current_generation,
                        generation,
                        WorkerEvent::SearchProgress { completed, total },
                    )
                    .await
                {
                    join_set.abort_all();
                    return;
                }
            }

            results.sort_by(|left, right| left.file_relative_path.cmp(&right.file_relative_path));

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

    fn next_scan_generation(&self) -> u64 {
        self.scan_generation.fetch_add(1, Ordering::AcqRel) + 1
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
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use tempfile::tempdir;
    use zip::write::FileOptions;
    use zip::ZipWriter;

    fn write_test_docx(path: &Path, text: &str) {
        let file = File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("word/document.xml", FileOptions::default())
            .unwrap();
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body>
            </w:document>"#
        );
        zip.write_all(xml.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

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
        write_test_docx(&root.path().join("one.docx"), "needle");
        let (command_tx, command_rx) = channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(32);
        let worker = tokio::spawn(Worker::default().run(command_rx, event_tx));

        command_tx
            .send(WorkerCommand::Scan {
                root: root.path().to_path_buf(),
            })
            .await
            .unwrap();
        assert!(matches!(
            event_rx.recv().await,
            Some(WorkerEvent::ScanStarted)
        ));
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
        write_test_docx(&root.path().join("one.docx"), "second");
        let (command_tx, command_rx) = channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(32);
        let worker = tokio::spawn(Worker::default().run(command_rx, event_tx));

        command_tx
            .send(WorkerCommand::Scan {
                root: root.path().to_path_buf(),
            })
            .await
            .unwrap();
        assert!(matches!(
            event_rx.recv().await,
            Some(WorkerEvent::ScanStarted)
        ));
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
