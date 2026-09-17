use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    model::{CommandLine, ProcessSnapshot, Snapshot},
    text::maximum_scroll_offset,
};

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
    viewport_rows: usize,
    command_line_offset_cells: u16,
    command_line_viewport_cells: u16,
    view_mode: ProcessViewMode,
    cpu_display_mode: CpuDisplayMode,
    collapsed_pids: HashSet<u32>,
    sort: SortSpec,
    filter: String,
    filter_before_edit: Option<String>,
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

/// Chooses whether the process table is a sorted list or a parent/child tree.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProcessViewMode {
    #[default]
    Flat,
    Tree,
}

/// Chooses the aggregate CPU history or current logical-CPU meters in the header.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CpuDisplayMode {
    #[default]
    Summary,
    LogicalCpus,
}

/// One rendered process row, enriched with tree-only presentation state.
#[derive(Clone, Debug)]
pub struct ProcessRow<'a> {
    pub process: &'a ProcessSnapshot,
    /// For every ancestor, whether another sibling follows it and therefore
    /// needs a vertical continuation guide in the rendered tree.
    pub ancestor_has_next_siblings: Vec<bool>,
    /// Whether this row is the final sibling at its level.
    pub is_last_sibling: bool,
    pub has_children: bool,
    pub is_expanded: bool,
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
        self.ensure_selected_row_is_visible();
    }

    pub fn set_viewport_rows(&mut self, viewport_rows: usize) {
        self.viewport_rows = viewport_rows;
        self.ensure_selected_row_is_visible();
    }

    pub fn command_line_offset_cells(&self) -> u16 {
        self.command_line_offset_cells
    }

    pub fn view_mode(&self) -> ProcessViewMode {
        self.view_mode
    }

    pub fn cpu_display_mode(&self) -> CpuDisplayMode {
        self.cpu_display_mode
    }

    pub fn toggle_cpu_display_mode(&mut self) {
        self.cpu_display_mode = match self.cpu_display_mode {
            CpuDisplayMode::Summary => CpuDisplayMode::LogicalCpus,
            CpuDisplayMode::LogicalCpus => CpuDisplayMode::Summary,
        };
    }

    pub fn toggle_view_mode(&mut self) {
        let previous_index = self.selected_index();
        self.view_mode = match self.view_mode {
            ProcessViewMode::Flat => ProcessViewMode::Tree,
            ProcessViewMode::Tree => ProcessViewMode::Flat,
        };
        self.reconcile_selection(previous_index);
    }

    pub fn set_command_line_offset_cells(&mut self, offset: u16) {
        self.command_line_offset_cells = offset;
        self.clamp_command_line_offset();
    }

    pub fn set_command_line_viewport_cells(&mut self, viewport_cells: u16) {
        self.command_line_viewport_cells = viewport_cells;
        self.clamp_command_line_offset();
    }

    pub fn sort(&self) -> SortSpec {
        self.sort
    }

    pub fn set_sort(&mut self, sort: SortSpec) {
        let previous_index = self.selected_index();
        self.sort = sort;
        self.reconcile_selection(previous_index);
    }

    pub fn cycle_sort_column(&mut self) {
        let next_column = match self.sort.column {
            SortColumn::Pid => SortColumn::Name,
            SortColumn::Name => SortColumn::CpuPercent,
            SortColumn::CpuPercent => SortColumn::Memory,
            SortColumn::Memory => SortColumn::Pid,
        };
        self.set_sort(SortSpec::for_column(next_column));
    }

    pub fn reverse_sort_direction(&mut self) {
        let direction = match self.sort.direction {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        };
        self.set_sort(SortSpec {
            column: self.sort.column,
            direction,
        });
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn is_filter_editing(&self) -> bool {
        self.filter_before_edit.is_some()
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.apply_filter(filter.into());
    }

    pub fn handle_key(&mut self, key: KeyCode) {
        self.handle_key_with_modifiers(key, KeyModifiers::NONE);
    }

    pub fn handle_key_with_modifiers(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        if self.is_filter_editing() {
            self.handle_filter_key(key, modifiers);
            return;
        }

        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('s') => self.cycle_sort_column(),
            KeyCode::Char('S') => self.reverse_sort_direction(),
            KeyCode::Char('t') => self.toggle_view_mode(),
            KeyCode::Char('c') => self.toggle_cpu_display_mode(),
            KeyCode::Char('/') => self.begin_filter_edit(),
            KeyCode::Up => self.move_selection_by(-1),
            KeyCode::Down => self.move_selection_by(1),
            KeyCode::PageUp => self.move_selection_by(-(self.page_size() as isize)),
            KeyCode::PageDown => self.move_selection_by(self.page_size() as isize),
            KeyCode::Home => self.move_selection_to(0),
            KeyCode::End => {
                let last_index = self.visible_processes().len().saturating_sub(1);
                self.move_selection_to(last_index);
            }
            KeyCode::Left => self.move_command_line_left(),
            KeyCode::Right => self.move_command_line_right(),
            KeyCode::Enter | KeyCode::Char(' ') => self.toggle_selected_expansion(),
            _ => {}
        }
    }

    fn apply_filter(&mut self, filter: String) {
        let previous_index = self.selected_index();
        self.filter = filter;
        self.reconcile_selection(previous_index);
    }

    fn begin_filter_edit(&mut self) {
        self.filter_before_edit = Some(self.filter.clone());
    }

    fn handle_filter_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        match key {
            KeyCode::Enter => self.filter_before_edit = None,
            KeyCode::Esc => {
                if let Some(previous_filter) = self.filter_before_edit.take() {
                    self.apply_filter(previous_filter);
                }
            }
            KeyCode::Backspace => {
                let mut filter = self.filter.clone();
                filter.pop();
                self.apply_filter(filter);
            }
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.apply_filter(String::new());
            }
            KeyCode::Char(character) if !modifiers.contains(KeyModifiers::CONTROL) => {
                let mut filter = self.filter.clone();
                filter.push(character);
                self.apply_filter(filter);
            }
            _ => {}
        }
    }

    /// Replaces the current immutable snapshot and preserves selection by PID
    /// whenever that process still exists in the new visible list.
    pub fn set_snapshot(&mut self, snapshot: Arc<Snapshot>) {
        let previous_index = self.selected_index();
        self.collapsed_pids.retain(|pid| {
            snapshot
                .processes
                .value
                .iter()
                .any(|process| process.pid == *pid)
        });
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

    /// Returns process rows after applying the current view, filter, and sort
    /// settings. Tree rows preserve ancestry while flat rows have depth zero.
    pub fn visible_rows(&self) -> Vec<ProcessRow<'_>> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };

        let filter = self.filter.to_lowercase();
        match self.view_mode {
            ProcessViewMode::Flat => {
                let mut processes = matching_processes(&snapshot.processes.value, &filter);
                sort_processes(&mut processes, self.sort);
                processes
                    .into_iter()
                    .map(|process| ProcessRow {
                        process,
                        ancestor_has_next_siblings: Vec::new(),
                        is_last_sibling: true,
                        has_children: false,
                        is_expanded: false,
                    })
                    .collect()
            }
            ProcessViewMode::Tree => tree_rows(
                &snapshot.processes.value,
                &filter,
                self.sort,
                &self.collapsed_pids,
            ),
        }
    }

    /// Returns visible processes without tree presentation data.
    pub fn visible_processes(&self) -> Vec<&ProcessSnapshot> {
        self.visible_rows()
            .into_iter()
            .map(|row| row.process)
            .collect()
    }

    /// Returns just the process rows that fit in the current table viewport.
    pub fn viewport_process_rows(&self) -> Vec<ProcessRow<'_>> {
        self.visible_rows()
            .into_iter()
            .skip(self.vertical_offset)
            .take(self.viewport_rows)
            .collect()
    }

    /// Returns the visible viewport without tree presentation data.
    pub fn viewport_processes(&self) -> Vec<&ProcessSnapshot> {
        self.viewport_process_rows()
            .into_iter()
            .map(|row| row.process)
            .collect()
    }

    /// Returns the selected process row relative to the rendered viewport.
    pub fn selected_viewport_index(&self) -> Option<usize> {
        self.selected_index()
            .and_then(|index| index.checked_sub(self.vertical_offset))
            .filter(|index| *index < self.viewport_rows)
    }

    fn page_size(&self) -> usize {
        self.viewport_rows.max(1)
    }

    fn move_selection_by(&mut self, amount: isize) {
        let Some(current_index) = self.selected_index() else {
            return;
        };

        let process_count = self.visible_processes().len();
        let destination = if amount.is_negative() {
            current_index.saturating_sub(amount.unsigned_abs())
        } else {
            current_index
                .saturating_add(amount as usize)
                .min(process_count.saturating_sub(1))
        };
        self.move_selection_to(destination);
    }

    fn move_selection_to(&mut self, index: usize) {
        let next_selected_pid = self
            .visible_processes()
            .get(index)
            .map(|process| process.pid);
        if let Some(next_selected_pid) = next_selected_pid {
            if self.selected_pid != Some(next_selected_pid) {
                self.selected_pid = Some(next_selected_pid);
                self.command_line_offset_cells = 0;
            }
            self.ensure_selected_row_is_visible();
        }
    }

    fn toggle_selected_expansion(&mut self) {
        if self.view_mode != ProcessViewMode::Tree {
            return;
        }

        let Some(selected_pid) = self.selected_pid else {
            return;
        };
        let has_children = self
            .visible_rows()
            .into_iter()
            .find(|row| row.process.pid == selected_pid)
            .is_some_and(|row| row.has_children);
        if !has_children {
            return;
        }

        if !self.collapsed_pids.insert(selected_pid) {
            self.collapsed_pids.remove(&selected_pid);
        }
        self.ensure_selected_row_is_visible();
    }

    fn move_command_line_left(&mut self) {
        self.command_line_offset_cells = self.command_line_offset_cells.saturating_sub(1);
    }

    fn move_command_line_right(&mut self) {
        self.command_line_offset_cells = self.command_line_offset_cells.saturating_add(1);
        self.clamp_command_line_offset();
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
        self.ensure_selected_row_is_visible();
        self.clamp_command_line_offset();
    }

    fn ensure_selected_row_is_visible(&mut self) {
        let process_count = self.visible_processes().len();
        if process_count == 0 || self.viewport_rows == 0 {
            self.vertical_offset = 0;
            return;
        }

        let maximum_offset = process_count.saturating_sub(self.viewport_rows);
        self.vertical_offset = self.vertical_offset.min(maximum_offset);

        if let Some(selected_index) = self.selected_index() {
            if selected_index < self.vertical_offset {
                self.vertical_offset = selected_index;
            } else if selected_index >= self.vertical_offset + self.viewport_rows {
                self.vertical_offset = selected_index + 1 - self.viewport_rows;
            }
        }
    }

    fn clamp_command_line_offset(&mut self) {
        let maximum_offset = self.selected_command_line_maximum_offset();
        self.command_line_offset_cells = self.command_line_offset_cells.min(maximum_offset);
    }

    fn selected_command_line_maximum_offset(&self) -> u16 {
        let Some(selected_pid) = self.selected_pid else {
            return 0;
        };

        self.visible_processes()
            .into_iter()
            .find(|process| process.pid == selected_pid)
            .map(|process| {
                maximum_scroll_offset(
                    &process.command_line_display(),
                    self.command_line_viewport_cells,
                )
            })
            .unwrap_or(0)
    }
}

impl SortSpec {
    fn for_column(column: SortColumn) -> Self {
        let direction = match column {
            SortColumn::Pid | SortColumn::Name => SortDirection::Ascending,
            SortColumn::CpuPercent | SortColumn::Memory => SortDirection::Descending,
        };
        Self { column, direction }
    }
}

fn matching_processes<'a>(
    processes: &'a [ProcessSnapshot],
    lowercase_filter: &str,
) -> Vec<&'a ProcessSnapshot> {
    processes
        .iter()
        .filter(|process| process_matches_filter(process, lowercase_filter))
        .collect()
}

fn tree_rows<'a>(
    processes: &'a [ProcessSnapshot],
    lowercase_filter: &str,
    sort: SortSpec,
    collapsed_pids: &HashSet<u32>,
) -> Vec<ProcessRow<'a>> {
    let processes_by_pid = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();
    let included_pids = tree_filter_pids(&processes_by_pid, lowercase_filter);
    let (mut root_pids, mut child_pids) = tree_relationships(&processes_by_pid);
    let no_collapsed_pids = HashSet::new();
    let effective_collapsed_pids = if lowercase_filter.is_empty() {
        collapsed_pids
    } else {
        &no_collapsed_pids
    };

    sort_process_ids(&mut root_pids, &processes_by_pid, sort);
    for children in child_pids.values_mut() {
        sort_process_ids(children, &processes_by_pid, sort);
    }

    let mut rows = Vec::with_capacity(included_pids.len());
    let mut visited_pids = HashSet::new();
    let mut root_connected_pids = HashSet::new();
    for pid in &root_pids {
        mark_tree_component(*pid, &child_pids, &mut root_connected_pids);
    }
    // A parent cycle has no natural root. Traversing every remaining PID makes
    // such a component visible without risking recursion loops.
    let mut remaining_pids = processes_by_pid.keys().copied().collect::<Vec<_>>();
    sort_process_ids(&mut remaining_pids, &processes_by_pid, sort);
    let mut top_level_pids = root_pids
        .into_iter()
        .filter(|pid| included_pids.contains(pid))
        .collect::<Vec<_>>();
    top_level_pids.extend(
        remaining_pids
            .into_iter()
            .filter(|pid| !root_connected_pids.contains(pid) && included_pids.contains(pid)),
    );

    let top_level_count = top_level_pids.len();
    for (index, pid) in top_level_pids.into_iter().enumerate() {
        append_tree_rows(
            pid,
            &[],
            index + 1 == top_level_count,
            &processes_by_pid,
            &child_pids,
            &included_pids,
            effective_collapsed_pids,
            &mut visited_pids,
            &mut rows,
        );
    }

    rows
}

fn mark_tree_component(
    pid: u32,
    child_pids: &HashMap<u32, Vec<u32>>,
    marked_pids: &mut HashSet<u32>,
) {
    if !marked_pids.insert(pid) {
        return;
    }

    if let Some(children) = child_pids.get(&pid) {
        for child_pid in children {
            mark_tree_component(*child_pid, child_pids, marked_pids);
        }
    }
}

fn tree_filter_pids(
    processes_by_pid: &HashMap<u32, &ProcessSnapshot>,
    lowercase_filter: &str,
) -> HashSet<u32> {
    if lowercase_filter.is_empty() {
        return processes_by_pid.keys().copied().collect();
    }

    let mut included_pids = HashSet::new();
    for process in processes_by_pid.values().copied() {
        if !process_matches_filter(process, lowercase_filter) {
            continue;
        }

        let mut ancestor_pid = Some(process.pid);
        let mut ancestors_seen = HashSet::new();
        while let Some(pid) = ancestor_pid {
            if !ancestors_seen.insert(pid) {
                break;
            }

            let Some(ancestor) = processes_by_pid.get(&pid) else {
                break;
            };
            included_pids.insert(pid);
            ancestor_pid = ancestor.parent_pid;
        }
    }

    included_pids
}

fn tree_relationships(
    processes_by_pid: &HashMap<u32, &ProcessSnapshot>,
) -> (Vec<u32>, HashMap<u32, Vec<u32>>) {
    let mut root_pids = Vec::new();
    let mut child_pids = HashMap::<u32, Vec<u32>>::new();

    for process in processes_by_pid.values().copied() {
        let parent_pid = process.parent_pid.filter(|parent_pid| {
            *parent_pid != process.pid && processes_by_pid.contains_key(parent_pid)
        });
        if let Some(parent_pid) = parent_pid {
            child_pids.entry(parent_pid).or_default().push(process.pid);
        } else {
            root_pids.push(process.pid);
        }
    }

    (root_pids, child_pids)
}

#[allow(clippy::too_many_arguments)]
fn append_tree_rows<'a>(
    pid: u32,
    ancestor_has_next_siblings: &[bool],
    is_last_sibling: bool,
    processes_by_pid: &HashMap<u32, &'a ProcessSnapshot>,
    child_pids: &HashMap<u32, Vec<u32>>,
    included_pids: &HashSet<u32>,
    collapsed_pids: &HashSet<u32>,
    visited_pids: &mut HashSet<u32>,
    rows: &mut Vec<ProcessRow<'a>>,
) {
    if !included_pids.contains(&pid) || !visited_pids.insert(pid) {
        return;
    }

    let Some(process) = processes_by_pid.get(&pid).copied() else {
        return;
    };
    let visible_children = child_pids
        .get(&pid)
        .map(|children| {
            children
                .iter()
                .copied()
                .filter(|child_pid| included_pids.contains(child_pid))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let has_children = !visible_children.is_empty();
    let is_expanded = has_children && !collapsed_pids.contains(&pid);
    rows.push(ProcessRow {
        process,
        ancestor_has_next_siblings: ancestor_has_next_siblings.to_vec(),
        is_last_sibling,
        has_children,
        is_expanded,
    });

    if is_expanded {
        let mut child_ancestor_has_next_siblings = ancestor_has_next_siblings.to_vec();
        child_ancestor_has_next_siblings.push(!is_last_sibling);
        let child_count = visible_children.len();
        for (index, child_pid) in visible_children.into_iter().enumerate() {
            append_tree_rows(
                child_pid,
                &child_ancestor_has_next_siblings,
                index + 1 == child_count,
                processes_by_pid,
                child_pids,
                included_pids,
                collapsed_pids,
                visited_pids,
                rows,
            );
        }
    }
}

fn sort_process_ids(
    process_ids: &mut [u32],
    processes_by_pid: &HashMap<u32, &ProcessSnapshot>,
    sort: SortSpec,
) {
    process_ids.sort_by(|left_pid, right_pid| {
        compare_processes(
            processes_by_pid
                .get(left_pid)
                .expect("tree references a known process"),
            processes_by_pid
                .get(right_pid)
                .expect("tree references a known process"),
            sort,
        )
    });
}

fn sort_processes(processes: &mut [&ProcessSnapshot], sort: SortSpec) {
    processes.sort_by(|left, right| compare_processes(left, right, sort));
}

fn compare_processes(
    left: &ProcessSnapshot,
    right: &ProcessSnapshot,
    sort: SortSpec,
) -> std::cmp::Ordering {
    let comparison = match sort.column {
        SortColumn::Pid => left.pid.cmp(&right.pid),
        SortColumn::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        SortColumn::CpuPercent => left.cpu_percent.total_cmp(&right.cpu_percent),
        SortColumn::Memory => left.memory_bytes.cmp(&right.memory_bytes),
    };

    let comparison = match sort.direction {
        SortDirection::Ascending => comparison,
        SortDirection::Descending => comparison.reverse(),
    };

    comparison.then_with(|| left.pid.cmp(&right.pid))
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

    use super::{App, CpuDisplayMode, ProcessViewMode, SortColumn, SortDirection, SortSpec};
    use crate::model::{CommandLine, Metric, ProcessSnapshot, Snapshot, SystemSnapshot};
    use crossterm::event::{KeyCode, KeyModifiers};

    fn process(
        pid: u32,
        name: &str,
        command_line: CommandLine,
        cpu_percent: f32,
    ) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: None,
            name: name.into(),
            command_line,
            executable_path: None,
            cpu_percent,
            memory_bytes: u64::from(pid) * 1024,
        }
    }

    fn process_with_parent(pid: u32, parent_pid: Option<u32>, name: &str) -> ProcessSnapshot {
        let mut process = process(pid, name, CommandLine::NotRequested, 0.0);
        process.parent_pid = parent_pid;
        process
    }

    fn snapshot(processes: Vec<ProcessSnapshot>) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            generation: 1,
            collected_at: Instant::now(),
            system: SystemSnapshot {
                cpu_percent: Metric::fresh(0.0),
                logical_cpu_percentages: Metric::fresh(Vec::new()),
                total_memory_bytes: Metric::fresh(0),
                used_memory_bytes: Metric::fresh(0),
                commit_charge_bytes: Metric::fresh(0),
                commit_limit_bytes: Metric::fresh(0),
            },
            processes: Metric::fresh(processes),
            history: Default::default(),
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
    fn c_toggles_between_summary_and_logical_cpu_header_modes() {
        let mut app = App::new();
        assert_eq!(app.cpu_display_mode(), CpuDisplayMode::Summary);

        app.handle_key(KeyCode::Char('c'));
        assert_eq!(app.cpu_display_mode(), CpuDisplayMode::LogicalCpus);

        app.handle_key(KeyCode::Char('c'));
        assert_eq!(app.cpu_display_mode(), CpuDisplayMode::Summary);
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

    #[test]
    fn down_navigation_scrolls_the_viewport_to_keep_selection_visible() {
        let mut app = App::new();
        app.set_viewport_rows(3);
        app.set_snapshot(snapshot(
            (1..=6)
                .map(|pid| process(pid, "worker.exe", CommandLine::NotRequested, 0.0))
                .collect(),
        ));

        for _ in 0..3 {
            app.handle_key(KeyCode::Down);
        }

        assert_eq!(app.selected_pid(), Some(4));
        assert_eq!(app.vertical_offset(), 1);
        assert_eq!(app.selected_viewport_index(), Some(2));
        assert_eq!(
            app.viewport_processes()
                .into_iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }

    #[test]
    fn page_home_and_end_navigation_clamp_to_the_process_list() {
        let mut app = App::new();
        app.set_viewport_rows(3);
        app.set_snapshot(snapshot(
            (1..=8)
                .map(|pid| process(pid, "worker.exe", CommandLine::NotRequested, 0.0))
                .collect(),
        ));

        app.handle_key(KeyCode::End);
        assert_eq!(app.selected_pid(), Some(8));
        assert_eq!(app.vertical_offset(), 5);

        app.handle_key(KeyCode::PageUp);
        assert_eq!(app.selected_pid(), Some(5));
        assert_eq!(app.vertical_offset(), 4);

        app.handle_key(KeyCode::Home);
        assert_eq!(app.selected_pid(), Some(1));
        assert_eq!(app.vertical_offset(), 0);

        app.handle_key(KeyCode::PageDown);
        assert_eq!(app.selected_pid(), Some(4));
        assert_eq!(app.vertical_offset(), 1);

        app.handle_key(KeyCode::Up);
        assert_eq!(app.selected_pid(), Some(3));
        assert_eq!(app.vertical_offset(), 1);
    }

    #[test]
    fn navigation_on_an_empty_process_list_keeps_the_viewport_safe() {
        let mut app = App::new();
        app.set_viewport_rows(3);
        app.set_snapshot(snapshot(Vec::new()));

        for key in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
        ] {
            app.handle_key(key);
        }

        assert_eq!(app.selected_pid(), None);
        assert_eq!(app.vertical_offset(), 0);
        assert_eq!(app.selected_viewport_index(), None);
    }

    #[test]
    fn resizing_the_viewport_keeps_the_selected_process_visible() {
        let mut app = App::new();
        app.set_viewport_rows(4);
        app.set_snapshot(snapshot(
            (1..=8)
                .map(|pid| process(pid, "worker.exe", CommandLine::NotRequested, 0.0))
                .collect(),
        ));
        app.handle_key(KeyCode::End);

        app.set_viewport_rows(2);

        assert_eq!(app.selected_pid(), Some(8));
        assert_eq!(app.vertical_offset(), 6);
        assert_eq!(app.selected_viewport_index(), Some(1));
    }

    #[test]
    fn command_line_navigation_clamps_and_resets_for_a_new_selection() {
        let mut app = App::new();
        app.set_viewport_rows(2);
        app.set_command_line_viewport_cells(4);
        app.set_snapshot(snapshot(vec![
            process(1, "first.exe", CommandLine::Present("abcdef".into()), 0.0),
            process(2, "second.exe", CommandLine::Present("uvwxyz".into()), 0.0),
        ]));

        for _ in 0..10 {
            app.handle_key(KeyCode::Right);
        }
        assert_eq!(app.command_line_offset_cells(), 3);

        app.handle_key(KeyCode::Left);
        assert_eq!(app.command_line_offset_cells(), 2);

        app.handle_key(KeyCode::Down);
        assert_eq!(app.selected_pid(), Some(2));
        assert_eq!(app.command_line_offset_cells(), 0);
    }

    #[test]
    fn widening_the_command_line_viewport_clamps_the_offset() {
        let mut app = App::new();
        app.set_command_line_viewport_cells(4);
        app.set_snapshot(snapshot(vec![process(
            1,
            "worker.exe",
            CommandLine::Present("abcdef".into()),
            0.0,
        )]));

        app.set_command_line_offset_cells(3);
        app.set_command_line_viewport_cells(6);

        assert_eq!(app.command_line_offset_cells(), 0);
    }

    #[test]
    fn sort_shortcuts_cycle_columns_with_their_default_directions() {
        let mut app = App::new();

        app.handle_key(KeyCode::Char('s'));
        assert_eq!(
            app.sort(),
            SortSpec {
                column: SortColumn::Name,
                direction: SortDirection::Ascending,
            }
        );

        app.handle_key(KeyCode::Char('s'));
        assert_eq!(
            app.sort(),
            SortSpec {
                column: SortColumn::CpuPercent,
                direction: SortDirection::Descending,
            }
        );

        app.handle_key(KeyCode::Char('s'));
        assert_eq!(
            app.sort(),
            SortSpec {
                column: SortColumn::Memory,
                direction: SortDirection::Descending,
            }
        );

        app.handle_key(KeyCode::Char('s'));
        assert_eq!(
            app.sort(),
            SortSpec {
                column: SortColumn::Pid,
                direction: SortDirection::Ascending,
            }
        );
    }

    #[test]
    fn uppercase_s_reverses_the_active_sort_direction() {
        let mut app = App::new();
        app.handle_key(KeyCode::Char('s'));
        app.handle_key(KeyCode::Char('S'));

        assert_eq!(
            app.sort(),
            SortSpec {
                column: SortColumn::Name,
                direction: SortDirection::Descending,
            }
        );
    }

    #[test]
    fn filter_edits_apply_immediately_and_can_be_accepted_or_cancelled() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process(1, "alpha.exe", CommandLine::NotRequested, 0.0),
            process(
                2,
                "worker.exe",
                CommandLine::Present("worker.exe --http-port 8080".into()),
                0.0,
            ),
        ]));

        app.handle_key(KeyCode::Char('/'));
        assert!(app.is_filter_editing());
        for character in "http".chars() {
            app.handle_key(KeyCode::Char(character));
        }
        assert_eq!(app.filter(), "http");
        assert_eq!(app.visible_processes()[0].pid, 2);

        app.handle_key(KeyCode::Enter);
        assert!(!app.is_filter_editing());

        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('x'));
        assert!(app.visible_processes().is_empty());
        app.handle_key(KeyCode::Esc);

        assert!(!app.is_filter_editing());
        assert_eq!(app.filter(), "http");
        assert_eq!(app.visible_processes()[0].pid, 2);
        assert!(!app.should_quit());
    }

    #[test]
    fn control_u_clears_a_filter_while_editing() {
        let mut app = App::new();
        app.set_filter("worker");
        app.handle_key(KeyCode::Char('/'));
        app.handle_key_with_modifiers(KeyCode::Char('u'), KeyModifiers::CONTROL);

        assert!(app.is_filter_editing());
        assert_eq!(app.filter(), "");
    }

    #[test]
    fn filter_matching_handles_unicode_case() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![process(
            1,
            "FÖÖ.exe",
            CommandLine::NotRequested,
            0.0,
        )]));

        app.set_filter("föö");

        assert_eq!(app.visible_processes()[0].pid, 1);
    }

    #[test]
    fn tree_mode_orders_children_beneath_their_parent() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process_with_parent(1, None, "root.exe"),
            process_with_parent(2, Some(1), "parent.exe"),
            process_with_parent(3, Some(1), "sibling.exe"),
            process_with_parent(4, Some(2), "grandchild.exe"),
            process_with_parent(5, Some(99), "orphan.exe"),
        ]));

        app.toggle_view_mode();

        assert_eq!(app.view_mode(), ProcessViewMode::Tree);
        assert_eq!(
            app.visible_rows()
                .into_iter()
                .map(|row| (row.process.pid, row.ancestor_has_next_siblings.len()))
                .collect::<Vec<_>>(),
            vec![(1, 0), (2, 1), (4, 2), (3, 1), (5, 0)]
        );
    }

    #[test]
    fn tree_mode_sorts_siblings_without_reordering_ancestry() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process_with_parent(1, None, "root.exe"),
            process_with_parent(2, Some(1), "zebra.exe"),
            process_with_parent(3, Some(1), "alpha.exe"),
        ]));
        app.set_sort(SortSpec {
            column: SortColumn::Name,
            direction: SortDirection::Ascending,
        });

        app.toggle_view_mode();

        assert_eq!(
            app.visible_processes()
                .into_iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
    }

    #[test]
    fn tree_filter_keeps_matching_process_ancestors() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process_with_parent(1, None, "root.exe"),
            process_with_parent(2, Some(1), "worker.exe"),
            process_with_parent(3, Some(1), "other.exe"),
        ]));
        app.toggle_view_mode();
        app.set_filter("worker");

        assert_eq!(
            app.visible_processes()
                .into_iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn tree_rows_expand_and_collapse_with_the_selected_parent() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process_with_parent(1, None, "root.exe"),
            process_with_parent(2, Some(1), "child.exe"),
        ]));
        app.toggle_view_mode();

        app.handle_key(KeyCode::Enter);
        assert_eq!(
            app.visible_processes()
                .into_iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![1]
        );

        app.set_filter("child");
        assert_eq!(
            app.visible_processes()
                .into_iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        app.set_filter("");

        app.handle_key(KeyCode::Char(' '));
        assert_eq!(
            app.visible_processes()
                .into_iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn tree_mode_tolerates_cycles_and_missing_parents() {
        let mut app = App::new();
        app.set_snapshot(snapshot(vec![
            process_with_parent(1, Some(2), "cycle-a.exe"),
            process_with_parent(2, Some(1), "cycle-b.exe"),
            process_with_parent(3, Some(99), "orphan.exe"),
        ]));

        app.toggle_view_mode();

        let pids = app
            .visible_processes()
            .into_iter()
            .map(|process| process.pid)
            .collect::<Vec<_>>();
        assert_eq!(pids.len(), 3);
        assert!(pids.contains(&1));
        assert!(pids.contains(&2));
        assert!(pids.contains(&3));
    }
}
