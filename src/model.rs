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
    /// Best-effort account name or SID for the process owner.
    pub user: Option<String>,
    /// Indicates how the displayed owner was obtained.
    pub user_source: UserSource,
    pub command_line: CommandLine,
    pub executable_path: Option<PathBuf>,
    /// Percentage of total logical CPU capacity used by this process (0-100).
    pub cpu_percent: f32,
    /// Number of threads reported by the latest ToolHelp process snapshot.
    pub thread_count: Metric<u64>,
    pub memory_bytes: u64,
}

/// The confidence level of a process owner label.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UserSource {
    /// The process access token supplied the owner SID.
    #[default]
    Token,
    /// The Service Control Manager supplied the configured service account.
    ServiceConfiguration,
    /// Windows did not allow the process token or an SCM fallback to be read.
    Restricted,
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

    pub fn user_display(&self) -> &str {
        // PID 4 is Windows' kernel-owned System process. It has no user token,
        // so calling it unknown hides useful information from the operator.
        if self.pid == 4 {
            "<kernel>"
        } else if self.user_source == UserSource::Restricted {
            "<restricted>"
        } else {
            self.user.as_deref().unwrap_or("<unknown>")
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
    pub network: NetworkSnapshot,
    pub gpu: GpuSnapshot,
}

/// Current telemetry for one hardware GPU adapter.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuAdapterSnapshot {
    /// DXGI adapter LUID packed into a stable 64-bit identifier.
    pub id: u64,
    pub name: String,
    /// Busiest active engine, not a sum of overlapping engine percentages.
    pub utilization_percent: Metric<Option<f32>>,
    pub dedicated_memory_used_bytes: Metric<Option<u64>>,
    pub dedicated_memory_capacity_bytes: Metric<Option<u64>>,
    pub shared_memory_used_bytes: Metric<Option<u64>>,
    pub shared_memory_capacity_bytes: Metric<Option<u64>>,
}

/// Hardware GPU adapters and their capability-aware telemetry.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuSnapshot {
    pub adapters: Metric<Vec<GpuAdapterSnapshot>>,
}

impl Default for GpuSnapshot {
    fn default() -> Self {
        Self {
            adapters: Metric::fresh(Vec::new()),
        }
    }
}

/// Current traffic for one Windows network interface.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkInterfaceSnapshot {
    /// Stable Windows interface LUID. It is an identity, not UI text.
    pub id: u64,
    pub alias: String,
    pub operational: bool,
    /// `None` means that no pair of counter samples is available yet.
    pub transmit_bytes_per_second: Metric<Option<f64>>,
    /// `None` means that no pair of counter samples is available yet.
    pub receive_bytes_per_second: Metric<Option<f64>>,
}

/// Network counters collected independently of process data.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkSnapshot {
    pub interfaces: Metric<Vec<NetworkInterfaceSnapshot>>,
    pub total_transmit_bytes_per_second: Metric<Option<f64>>,
    pub total_receive_bytes_per_second: Metric<Option<f64>>,
}

impl Default for NetworkSnapshot {
    fn default() -> Self {
        Self {
            interfaces: Metric::fresh(Vec::new()),
            total_transmit_bytes_per_second: Metric::fresh(None),
            total_receive_bytes_per_second: Metric::fresh(None),
        }
    }
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
    use super::{
        CommandLine, Freshness, HISTORY_CAPACITY, History, HistorySample, Metric, ProcessSnapshot,
        UserSource,
    };
    use std::path::PathBuf;

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
    fn user_display_marks_a_missing_process_owner() {
        let mut process = ProcessSnapshot {
            pid: 1,
            parent_pid: None,
            name: "example.exe".into(),
            user: None,
            user_source: UserSource::Token,
            command_line: CommandLine::NotRequested,
            executable_path: Some(PathBuf::from("example.exe")),
            cpu_percent: 0.0,
            thread_count: Metric::fresh(1),
            memory_bytes: 0,
        };
        assert_eq!(process.user_display(), "<unknown>");

        process.user = Some("Alice".into());
        assert_eq!(process.user_display(), "Alice");
    }

    #[test]
    fn system_process_displays_as_kernel_not_unknown() {
        let process = ProcessSnapshot {
            pid: 4,
            parent_pid: None,
            name: "System".into(),
            user: None,
            user_source: UserSource::Restricted,
            command_line: CommandLine::NotRequested,
            executable_path: None,
            cpu_percent: 0.0,
            thread_count: Metric::fresh(1),
            memory_bytes: 0,
        };

        assert_eq!(process.user_display(), "<kernel>");
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
