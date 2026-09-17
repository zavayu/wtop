use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

use crate::{
    app::App,
    model::{CommandLine, Freshness, Metric, ProcessSnapshot, Snapshot, SystemSnapshot},
};

const MINIMUM_WIDTH: u16 = 60;
const MINIMUM_HEIGHT: u16 = 10;

/// Renders the first-milestone dashboard from the newest completed snapshot.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if terminal_is_too_small(area.width, area.height) {
        render_minimum_size_message(frame);
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(system_summary(
        app.snapshot().map(|snapshot| &snapshot.system),
    ))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .title(" wtop ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(header, sections[0]);

    let table = Table::new(
        app.visible_processes().into_iter().map(process_row),
        [
            Constraint::Length(7),
            Constraint::Length(16),
            Constraint::Length(7),
            Constraint::Length(10),
            Constraint::Min(8),
        ],
    )
    .header(
        Row::new(["PID", "NAME", "CPU", "MEMORY", "COMMAND LINE"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(Block::default().borders(Borders::ALL).title(" Processes "));
    frame.render_widget(table, sections[1]);

    let footer = Paragraph::new(footer_text(
        app.snapshot().map(|snapshot| snapshot.as_ref()),
    ))
    .alignment(Alignment::Center)
    .wrap(Wrap { trim: true })
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, sections[2]);
}

fn terminal_is_too_small(width: u16, height: u16) -> bool {
    width < MINIMUM_WIDTH || height < MINIMUM_HEIGHT
}

fn render_minimum_size_message(frame: &mut Frame) {
    let message =
        format!("Terminal too small — minimum {MINIMUM_WIDTH} columns × {MINIMUM_HEIGHT} rows");
    let paragraph = Paragraph::new(message)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(Block::default().title(" wtop ").borders(Borders::ALL));
    frame.render_widget(paragraph, frame.area());
}

fn system_summary(system: Option<&SystemSnapshot>) -> String {
    let Some(system) = system else {
        return "CPU: --%  Mem: -- / --  Commit: -- / --".into();
    };

    format!(
        "CPU: {}  Mem: {}/{}  Commit: {}/{}",
        format_percent(&system.cpu_percent),
        format_metric_bytes(&system.used_memory_bytes),
        format_metric_bytes(&system.total_memory_bytes),
        format_metric_bytes(&system.commit_charge_bytes),
        format_metric_bytes(&system.commit_limit_bytes),
    )
}

fn process_row(process: &ProcessSnapshot) -> Row<'static> {
    Row::new(vec![
        Cell::from(process.pid.to_string()),
        Cell::from(process.name.clone()),
        Cell::from(format_percent_value(process.cpu_percent)),
        Cell::from(format_bytes(process.memory_bytes)),
        Cell::from(format_command_line(process)),
    ])
}

fn footer_text(snapshot: Option<&Snapshot>) -> &'static str {
    if snapshot.is_some_and(snapshot_is_stale) {
        "q / Esc Quit    Ctrl+C Interrupt    ~ Stale data"
    } else {
        "q / Esc Quit    Ctrl+C Interrupt"
    }
}

fn snapshot_is_stale(snapshot: &Snapshot) -> bool {
    metric_is_stale(&snapshot.system.cpu_percent)
        || metric_is_stale(&snapshot.system.total_memory_bytes)
        || metric_is_stale(&snapshot.system.used_memory_bytes)
        || metric_is_stale(&snapshot.system.commit_charge_bytes)
        || metric_is_stale(&snapshot.system.commit_limit_bytes)
        || metric_is_stale(&snapshot.processes)
}

fn metric_is_stale<T>(metric: &Metric<T>) -> bool {
    matches!(&metric.freshness, Freshness::Stale { .. })
}

fn format_percent(metric: &Metric<f32>) -> String {
    format!(
        "{}{}",
        freshness_marker(metric),
        format_percent_value(metric.value)
    )
}

fn format_metric_bytes(metric: &Metric<u64>) -> String {
    format!("{}{}", freshness_marker(metric), format_bytes(metric.value))
}

fn freshness_marker<T>(metric: &Metric<T>) -> &'static str {
    if metric_is_stale(metric) { "~" } else { "" }
}

fn format_percent_value(value: f32) -> String {
    format!("{value:.1}%")
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{bytes}B")
    } else {
        format!("{value:.1}{}", UNITS[unit_index])
    }
}

fn format_command_line(process: &ProcessSnapshot) -> String {
    match &process.command_line {
        CommandLine::Present(command_line) if command_line.is_empty() => "<empty>".into(),
        CommandLine::Present(command_line) => command_line.clone(),
        CommandLine::Unavailable => process.executable_path.as_ref().map_or_else(
            || "<unavailable>".into(),
            |path| format!("<unavailable> {}", path.display()),
        ),
        CommandLine::NotRequested => "<pending>".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        MINIMUM_HEIGHT, MINIMUM_WIDTH, format_bytes, format_command_line, format_percent,
        terminal_is_too_small,
    };
    use crate::model::{CommandLine, Freshness, Metric, ProcessSnapshot};

    #[test]
    fn byte_values_use_compact_binary_units() {
        assert_eq!(format_bytes(42), "42B");
        assert_eq!(format_bytes(1_024), "1.0KiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0GiB");
    }

    #[test]
    fn stale_percentages_have_a_compact_marker() {
        let metric = Metric {
            value: 12.5,
            freshness: Freshness::Stale {
                reason: "sample failed".into(),
            },
        };

        assert_eq!(format_percent(&metric), "~12.5%");
    }

    #[test]
    fn unavailable_command_lines_fall_back_to_the_executable_path() {
        let process = ProcessSnapshot {
            pid: 1,
            name: "example.exe".into(),
            command_line: CommandLine::Unavailable,
            executable_path: Some(PathBuf::from(r"C:\Tools\example.exe")),
            cpu_percent: 0.0,
            memory_bytes: 0,
        };

        assert_eq!(
            format_command_line(&process),
            r"<unavailable> C:\Tools\example.exe"
        );
    }

    #[test]
    fn minimum_terminal_size_is_enforced_in_both_dimensions() {
        assert!(terminal_is_too_small(MINIMUM_WIDTH - 1, MINIMUM_HEIGHT));
        assert!(terminal_is_too_small(MINIMUM_WIDTH, MINIMUM_HEIGHT - 1));
        assert!(!terminal_is_too_small(MINIMUM_WIDTH, MINIMUM_HEIGHT));
    }
}
