use std::sync::Arc;

use crossterm::event::KeyCode;

use crate::model::{CommandLine, ProcessSnapshot, Snapshot};

/// Owns the application state that is independent of terminal rendering.
///
/// Keeping this state separate from `ui` makes navigation and data transforms
/// testable without a terminal.
#[derive(Debug, Default)]
pub struct App {
    should_quit: bool,
    snapshot: Option<Arc<Snapshot>>,
    selected_pid: Option<u32>,
    vertical_offset: usize,
    command_line_offset_cells: u16,
    sort: SortSpec,
    filter: String,
}

/// The supported sort keys for the first milestone's process table.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SortColumn {
    #[default]
    Pid,
    Name,
    CpuPercent,
    Memory,
}

/// The direction in which the active sort column is ordered.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

/// The active sort specification for visible processes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SortSpec {
    pub column: SortColumn,
    pub direction: SortDirection,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn snapshot(&self) -> Option<&Arc<Snapshot>> {
        self.snapshot.as_ref()
    }

    pub fn selected_pid(&self) -> Option<u32> {
        self.selected_pid
    }

    pub fn vertical_offset(&self) -> usize {
        self.vertical_offset
    }

    pub fn set_vertical_offset(&mut self, offset: usize) {
        self.vertical_offset = offset;
    }

    pub fn command_line_offset_cells(&self) -> u16 {
        self.command_line_offset_cells
    }

    pub fn set_command_line_offset_cells(&mut self, offset: u16) {
        self.command_line_offset_cells = offset;
    }

    pub fn sort(&self) -> SortSpec {
        self.sort
    }

    pub fn set_sort(&mut self, sort: SortSpec) {
        let previous_index = self.selected_index();
        self.sort = sort;
        self.reconcile_selection(previous_index);
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) {
        let previous_index = self.selected_index();
        self.filter = filter.into();
        self.reconcile_selection(previous_index);
    }

    /// Replaces the current immutable snapshot and preserves selection by PID
    /// whenever that process still exists in the new visible list.
    pub fn set_snapshot(&mut self, snapshot: Arc<Snapshot>) {
        let previous_index = self.selected_index();
        self.snapshot = Some(snapshot);
        self.reconcile_selection(previous_index);
    }

    /// Selects a visible process by PID. Returns `false` when the PID is not
    /// available under the current snapshot, filter, and sort state.
    pub fn select_pid(&mut self, pid: u32) -> bool {
        let can_select = self
            .visible_processes()
            .iter()
            .any(|process| process.pid == pid);

        if can_select {
            self.selected_pid = Some(pid);
            self.command_line_offset_cells = 0;
            true
        } else {
            false
        }
    }

    /// Returns processes after applying the current filter and sort settings.
    pub fn visible_processes(&self) -> Vec<&ProcessSnapshot> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };

        let filter = self.filter.to_lowercase();
        let mut processes = snapshot
            .processes
            .value
            .iter()
            .filter(|process| process_matches_filter(process, &filter))
            .collect::<Vec<_>>();

        processes.sort_by(|left, right| {
            let comparison = match self.sort.column {
                SortColumn::Pid => left.pid.cmp(&right.pid),
                SortColumn::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
                SortColumn::CpuPercent => left.cpu_percent.total_cmp(&right.cpu_percent),
                SortColumn::Memory => left.memory_bytes.cmp(&right.memory_bytes),
            };

            let comparison = match self.sort.direction {
                SortDirection::Ascending => comparison,
                SortDirection::Descending => comparison.reverse(),
            };

            comparison.then_with(|| left.pid.cmp(&right.pid))
        });

        processes
    }

    pub fn handle_key(&mut self, key: KeyCode) {
        if matches!(key, KeyCode::Char('q') | KeyCode::Esc) {
            self.should_quit = true;
        }
    }

    fn selected_index(&self) -> Option<usize> {
        let selected_pid = self.selected_pid?;
        self.visible_processes()
            .iter()
            .position(|process| process.pid == selected_pid)
    }

    fn reconcile_selection(&mut self, previous_index: Option<usize>) {
        let (next_selected_pid, reset_vertical_offset, reset_command_offset) = {
            let processes = self.visible_processes();

            if processes.is_empty() {
                (None, true, true)
            } else if self
                .selected_pid
                .is_some_and(|pid| processes.iter().any(|process| process.pid == pid))
            {
                (self.selected_pid, false, false)
            } else {
                let replacement_index = previous_index.unwrap_or(0).min(processes.len() - 1);
                (Some(processes[replacement_index].pid), false, true)
            }
        };

        self.selected_pid = next_selected_pid;
        if reset_vertical_offset {
            self.vertical_offset = 0;
        }
        if reset_command_offset {
            self.command_line_offset_cells = 0;
        }
    }
}

fn process_matches_filter(process: &ProcessSnapshot, lowercase_filter: &str) -> bool {
    if lowercase_filter.is_empty() || process.name.to_lowercase().contains(lowercase_filter) {
        return true;
    }

    matches!(
        &process.command_line,
        CommandLine::Present(command_line) if command_line.to_lowercase().contains(lowercase_filter)
    )
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Instant};

    use super::{App, SortColumn, SortDirection, SortSpec};
    use crate::model::{CommandLine, Metric, ProcessSnapshot, Snapshot, SystemSnapshot};
    use crossterm::event::KeyCode;

    fn process(
        pid: u32,
        name: &str,
        command_line: CommandLine,
        cpu_percent: f32,
    ) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            name: name.into(),
            command_line,
            executable_path: None,
            cpu_percent,
            memory_bytes: u64::from(pid) * 1024,
        }
    }

    fn snapshot(processes: Vec<ProcessSnapshot>) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            generation: 1,
            collected_at: Instant::now(),
            system: SystemSnapshot {
                cpu_percent: Metric::fresh(0.0),
                total_memory_bytes: Metric::fresh(0),
                used_memory_bytes: Metric::fresh(0),
                commit_charge_bytes: Metric::fresh(0),
                commit_limit_bytes: Metric::fresh(0),
            },
            processes: Metric::fresh(processes),
        })
    }

    #[test]
    fn quit_keys_request_a_clean_exit() {
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            let mut app = App::new();
            app.handle_key(key);
            assert!(app.should_quit());
        }
    }

    #[test]
    fn filter_matches_process_name_and_present_command_line() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process(1, "services.exe", CommandLine::Unavailable, 1.0),
            process(
                2,
                "worker.exe",
                CommandLine::Present("worker.exe --http-port 8080".into()),
                2.0,
            ),
        ]));

        app.set_filter("SERV");
        assert_eq!(app.visible_processes()[0].pid, 1);

        app.set_filter("HTTP-PORT");
        assert_eq!(app.visible_processes()[0].pid, 2);

        app.set_filter("missing");
        assert!(app.visible_processes().is_empty());
        assert_eq!(app.selected_pid(), None);
    }

    #[test]
    fn sorting_uses_pid_as_a_deterministic_tie_breaker() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process(9, "same.exe", CommandLine::NotRequested, 2.0),
            process(3, "same.exe", CommandLine::NotRequested, 2.0),
            process(7, "other.exe", CommandLine::NotRequested, 4.0),
        ]));
        app.set_sort(SortSpec {
            column: SortColumn::CpuPercent,
            direction: SortDirection::Descending,
        });

        let pids = app
            .visible_processes()
            .into_iter()
            .map(|process| process.pid)
            .collect::<Vec<_>>();
        assert_eq!(pids, vec![7, 3, 9]);
    }

    #[test]
    fn selection_survives_a_refresh_that_reorders_processes() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process(10, "first.exe", CommandLine::NotRequested, 1.0),
            process(20, "second.exe", CommandLine::NotRequested, 2.0),
        ]));
        app.set_sort(SortSpec {
            column: SortColumn::CpuPercent,
            direction: SortDirection::Descending,
        });
        assert!(app.select_pid(20));

        app.set_snapshot(snapshot(vec![
            process(10, "first.exe", CommandLine::NotRequested, 9.0),
            process(20, "second.exe", CommandLine::NotRequested, 1.0),
        ]));

        assert_eq!(app.selected_pid(), Some(20));
    }

    #[test]
    fn selection_moves_to_the_previous_index_when_a_process_exits() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process(10, "first.exe", CommandLine::NotRequested, 1.0),
            process(20, "second.exe", CommandLine::NotRequested, 2.0),
            process(30, "third.exe", CommandLine::NotRequested, 3.0),
        ]));
        app.set_sort(SortSpec {
            column: SortColumn::Pid,
            direction: SortDirection::Ascending,
        });
        app.set_filter("second");
        assert_eq!(app.selected_pid(), Some(20));

        app.set_filter("");
        app.set_snapshot(snapshot(vec![
            process(10, "first.exe", CommandLine::NotRequested, 1.0),
            process(30, "third.exe", CommandLine::NotRequested, 3.0),
        ]));

        assert_eq!(app.selected_pid(), Some(30));
    }
}
