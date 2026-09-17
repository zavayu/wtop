use std::{
    sync::{
        Arc, RwLock,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{collector::Collector, model::Snapshot};

/// Shares the newest completed snapshot with the UI without accumulating a
/// queue of stale refresh work.
#[derive(Clone, Default)]
pub struct SnapshotStore {
    latest: Arc<RwLock<Option<Arc<Snapshot>>>>,
}

impl SnapshotStore {
    pub fn latest(&self) -> Option<Arc<Snapshot>> {
        self.latest
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn publish(&self, snapshot: Arc<Snapshot>) {
        *self
            .latest
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(snapshot);
    }
}

/// Owns the collector thread and joins it during application shutdown.
pub struct CollectorWorker {
    shutdown: Option<Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
}

impl CollectorWorker {
    pub fn start(snapshot_store: SnapshotStore, refresh_interval: Duration) -> Self {
        let (shutdown, shutdown_receiver) = mpsc::channel();
        let join_handle = thread::spawn(move || {
            collect_until_shutdown(snapshot_store, refresh_interval, shutdown_receiver);
        });

        Self {
            shutdown: Some(shutdown),
            join_handle: Some(join_handle),
        }
    }

    /// Signals shutdown and waits for the collector to finish.
    pub fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl Drop for CollectorWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn collect_until_shutdown(
    snapshot_store: SnapshotStore,
    refresh_interval: Duration,
    shutdown_receiver: Receiver<()>,
) {
    let mut collector = Collector::new();
    // CPU is a delta between refreshes. Collect once to establish sysinfo's
    // timing baseline, but do not publish that unrepresentative first sample.
    let baseline = Arc::new(collector.collect(None));
    let mut previous = Some(baseline);
    let mut cadence = RefreshCadence::new(refresh_interval, Instant::now());
    cadence.record_refresh_completed(Instant::now());

    loop {
        match shutdown_receiver.recv_timeout(cadence.wait_duration(Instant::now())) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                let snapshot = Arc::new(collector.collect(previous.as_deref()));
                previous = Some(Arc::clone(&snapshot));
                snapshot_store.publish(snapshot);
                cadence.record_refresh_completed(Instant::now());
            }
        }
    }
}

/// Schedules non-overlapping refreshes. A slow collection delays the next
/// refresh instead of attempting to catch up with extra work.
#[derive(Debug)]
struct RefreshCadence {
    interval: Duration,
    next_refresh: Instant,
}

impl RefreshCadence {
    fn new(interval: Duration, now: Instant) -> Self {
        Self {
            interval,
            next_refresh: now,
        }
    }

    fn wait_duration(&self, now: Instant) -> Duration {
        self.next_refresh.saturating_duration_since(now)
    }

    fn record_refresh_completed(&mut self, completed_at: Instant) {
        self.next_refresh = completed_at + self.interval;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use super::{RefreshCadence, SnapshotStore};
    use crate::model::{History, Metric, Snapshot, SystemSnapshot};

    fn snapshot(generation: u64) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            generation,
            collected_at: Instant::now(),
            system: SystemSnapshot {
                cpu_percent: Metric::fresh(0.0),
                logical_cpu_percentages: Metric::fresh(Vec::new()),
                total_memory_bytes: Metric::fresh(0),
                used_memory_bytes: Metric::fresh(0),
                commit_charge_bytes: Metric::fresh(0),
                commit_limit_bytes: Metric::fresh(0),
            },
            processes: Metric::fresh(Vec::new()),
            history: History::default(),
        })
    }

    #[test]
    fn snapshot_store_replaces_old_data_with_the_latest_generation() {
        let store = SnapshotStore::default();
        store.publish(snapshot(1));
        store.publish(snapshot(2));

        assert_eq!(store.latest().map(|snapshot| snapshot.generation), Some(2));
    }

    #[test]
    fn cadence_runs_immediately_then_waits_after_collection_completes() {
        let started_at = Instant::now();
        let mut cadence = RefreshCadence::new(Duration::from_secs(1), started_at);

        assert_eq!(cadence.wait_duration(started_at), Duration::ZERO);

        let collection_finished_at = started_at + Duration::from_millis(250);
        cadence.record_refresh_completed(collection_finished_at);

        assert_eq!(
            cadence.wait_duration(collection_finished_at + Duration::from_millis(400)),
            Duration::from_millis(600)
        );
    }

    #[test]
    fn cadence_delays_the_first_published_refresh_after_cpu_warmup() {
        let started_at = Instant::now();
        let mut cadence = RefreshCadence::new(Duration::from_secs(1), started_at);

        cadence.record_refresh_completed(started_at);

        assert_eq!(cadence.wait_duration(started_at), Duration::from_secs(1));
    }
}
