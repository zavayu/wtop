use std::{collections::VecDeque, path::PathBuf, time::Instant};

/// The command-line information wtop can obtain for a process.
///
/// Windows restricts memory access for some processes, so an unavailable
/// command line must remain distinct from one that is genuinely empty.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CommandLine {
    #[default]
    NotRequested,
    Present(String),
    Unavailable,
}

/// Indicates whether a value was collected in the current refresh.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Freshness {
    #[default]
    Fresh,
    Stale {
        reason: String,
    },
}

/// A collected value together with its refresh status.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric<T> {
    pub value: T,
    pub freshness: Freshness,
}

impl<T> Metric<T> {
    pub fn fresh(value: T) -> Self {
        Self {
            value,
            freshness: Freshness::Fresh,
        }
    }

    pub fn stale(value: T, reason: impl Into<String>) -> Self {
        Self {
            value,
            freshness: Freshness::Stale {
                reason: reason.into(),
            },
        }
    }
}

/// The fields wtop needs to render one process in the first milestone.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub command_line: CommandLine,
    pub executable_path: Option<PathBuf>,
    /// Percentage of total logical CPU capacity used by this process (0-100).
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

impl ProcessSnapshot {
    /// Returns the concise command-line representation shown in the process
    /// table, including the executable fallback for inaccessible processes.
    pub fn command_line_display(&self) -> String {
        match &self.command_line {
            CommandLine::Present(command_line) if command_line.is_empty() => "<empty>".into(),
            CommandLine::Present(command_line) => command_line.clone(),
            CommandLine::Unavailable => self.executable_path.as_ref().map_or_else(
                || "<unavailable>".into(),
                |path| format!("<unavailable> {}", path.display()),
            ),
            CommandLine::NotRequested => "<pending>".into(),
        }
    }
}

/// The number of recent refresh samples retained for the header history.
pub const HISTORY_CAPACITY: usize = 60;

/// One collected CPU sample for the header sparkline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HistorySample {
    pub cpu_percent: f32,
}

/// A bounded, newest-last CPU history.
#[derive(Clone, Debug, PartialEq)]
pub struct History {
    samples: VecDeque<HistorySample>,
    capacity: usize,
}

impl History {
    pub fn new() -> Self {
        Self::with_capacity(HISTORY_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0, "history capacity must be greater than zero");
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Appends a sample, evicting the oldest once the capacity is reached.
    pub fn push(&mut self, sample: HistorySample) {
        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn samples(&self) -> impl DoubleEndedIterator<Item = &HistorySample> {
        self.samples.iter()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

/// System-wide metrics displayed in the compact header.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemSnapshot {
    pub cpu_percent: Metric<f32>,
    /// Current usage for each logical CPU, in operating-system order.
    pub logical_cpu_percentages: Metric<Vec<f32>>,
    pub total_memory_bytes: Metric<u64>,
    pub used_memory_bytes: Metric<u64>,
    pub commit_charge_bytes: Metric<u64>,
    pub commit_limit_bytes: Metric<u64>,
}

/// A complete, immutable view of process, system, and header-history state from
/// one refresh.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub generation: u64,
    pub collected_at: Instant,
    pub system: SystemSnapshot,
    pub processes: Metric<Vec<ProcessSnapshot>>,
    pub history: History,
}

#[cfg(test)]
mod tests {
    use super::{Freshness, HISTORY_CAPACITY, History, HistorySample, Metric};

    #[test]
    fn stale_metrics_keep_the_last_value_and_reason() {
        let metric = Metric::stale(42_u64, "system query failed");

        assert_eq!(metric.value, 42);
        assert_eq!(
            metric.freshness,
            Freshness::Stale {
                reason: "system query failed".into(),
            }
        );
    }

    #[test]
    fn history_evicts_the_oldest_sample_at_its_capacity() {
        let mut history = History::with_capacity(2);
        history.push(sample(1.0));
        history.push(sample(2.0));
        history.push(sample(3.0));

        assert_eq!(history.len(), 2);
        assert_eq!(history.samples().next().unwrap().cpu_percent, 2.0);
        assert_eq!(history.samples().next_back().unwrap().cpu_percent, 3.0);
    }

    #[test]
    fn default_history_retains_the_full_window_newest_last() {
        let mut history = History::default();
        for index in 0..(HISTORY_CAPACITY + 3) {
            history.push(sample(index as f32));
        }

        assert_eq!(history.len(), HISTORY_CAPACITY);
        assert_eq!(history.samples().next().unwrap().cpu_percent, 3.0);
        assert_eq!(
            history.samples().next_back().unwrap().cpu_percent,
            (HISTORY_CAPACITY + 2) as f32
        );
    }

    #[test]
    #[should_panic(expected = "history capacity must be greater than zero")]
    fn history_rejects_a_zero_capacity() {
        let _ = History::with_capacity(0);
    }

    fn sample(cpu_percent: f32) -> HistorySample {
        HistorySample { cpu_percent }
    }
}
