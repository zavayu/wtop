use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table, Wrap},
};

/// Renders the first-milestone shell. Live values and process rows are added
/// once the collector module exists.
pub fn render(frame: &mut Frame) {
    let area = frame.area();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new("CPU: --%    Memory: -- / --    Commit: -- / --")
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .title(" wtop ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        );
    frame.render_widget(header, sections[0]);

    let table = Table::new(
        std::iter::empty::<Row<'static>>(),
        [
            Constraint::Length(8),
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(["PID", "NAME", "CPU", "MEMORY", "COMMAND LINE"])
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title(" Processes "));
    frame.render_widget(table, sections[1]);

    let footer = Paragraph::new("q / Esc Quit    Ctrl+C Interrupt")
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, sections[2]);
}
