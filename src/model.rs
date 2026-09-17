use std::{path::PathBuf, time::Instant};

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
    pub name: String,
    pub command_line: CommandLine,
    pub executable_path: Option<PathBuf>,
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

/// System-wide metrics displayed in the compact header.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemSnapshot {
    pub cpu_percent: Metric<f32>,
    pub total_memory_bytes: Metric<u64>,
    pub used_memory_bytes: Metric<u64>,
    pub commit_charge_bytes: Metric<u64>,
    pub commit_limit_bytes: Metric<u64>,
}

/// A complete, immutable view of process and system state from one refresh.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub generation: u64,
    pub collected_at: Instant,
    pub system: SystemSnapshot,
    pub processes: Metric<Vec<ProcessSnapshot>>,
}

#[cfg(test)]
mod tests {
    use super::{Freshness, Metric};

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
}
