use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::App;

pub fn render(f: &mut Frame, _app: &App) {
    let area = f.area();

    // Centre a box
    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Min(0),
            Constraint::Percentage(10),
        ])
        .split(area);
    let hchunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Min(0),
            Constraint::Percentage(15),
        ])
        .split(vchunks[1]);
    let box_area = hchunks[1];

    let lines = vec![
        Line::from(Span::styled(
            "Keyboard shortcuts",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Global",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  q / Ctrl-C   Quit"),
        Line::from("  ?            This help"),
        Line::from("  Esc          Go back"),
        Line::from(""),
        Line::from(Span::styled(
            "Registry list",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑ / k        Move up"),
        Line::from("  ↓ / j        Move down"),
        Line::from("  Enter        Open registry / browse packages"),
        Line::from("  p            Open publish wizard"),
        Line::from(""),
        Line::from(Span::styled(
            "Package list",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  Enter        View version details"),
        Line::from("  /            Toggle search filter"),
        Line::from(""),
        Line::from(Span::styled(
            "Package detail (versions)",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  y            Yank selected version"),
        Line::from("  u            Unyank selected version"),
        Line::from(""),
        Line::from(Span::styled(
            "Publish wizard",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  Tab          Next field"),
        Line::from("  Shift-Tab    Previous field"),
        Line::from("  Enter        Submit"),
        Line::from("  Esc          Cancel"),
        Line::from(""),
        Line::from(Span::styled(
            "Press Esc or ? to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::White)),
        )
        .alignment(Alignment::Left);

    f.render_widget(para, box_area);
}
