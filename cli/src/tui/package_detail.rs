use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::api::package::{PackageStatus, PackageSummary};

use super::App;

pub struct PackageDetailWidget {
    pub items: Vec<PackageSummary>,
    pub state: ListState,
}

impl PackageDetailWidget {
    pub fn new() -> Self {
        Self {
            items: vec![],
            state: ListState::default(),
        }
    }

    pub fn set_items(&mut self, items: Vec<PackageSummary>) {
        self.items = items;
        if !self.items.is_empty() {
            self.state.select(Some(0));
        } else {
            self.state.select(None);
        }
    }

    pub fn next(&mut self) {
        let len = self.items.len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().map(|i| (i + 1) % len).unwrap_or(0);
        self.state.select(Some(i));
    }

    pub fn prev(&mut self) {
        let len = self.items.len();
        if len == 0 {
            return;
        }
        let i = self
            .state
            .selected()
            .map(|i| if i == 0 { len - 1 } else { i - 1 })
            .unwrap_or(0);
        self.state.select(Some(i));
    }

    pub fn selected(&self) -> Option<&PackageSummary> {
        self.state.selected().and_then(|i| self.items.get(i))
    }
}

pub fn render(f: &mut Frame, app: &App, registry: &str, name: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let main_area = chunks[0];
    let help_area = chunks[1];

    let items: Vec<ListItem> = app
        .package_detail
        .items
        .iter()
        .map(|p| {
            let (status_str, status_color) = match &p.status {
                PackageStatus::Available => ("✓ available", Color::Green),
                PackageStatus::Blocked { reason } => {
                    let _ = reason;
                    ("✗ blocked  ", Color::Red)
                }
            };
            let reason_suffix = match &p.status {
                PackageStatus::Blocked { reason } => format!("  ({})", reason),
                _ => String::new(),
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<20}", p.version),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(status_str, Style::default().fg(status_color)),
                Span::styled(reason_suffix, Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("   {:>8} dl", p.access_count),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = format!(
        " {registry} > {name} — {} version(s) ",
        app.package_detail.items.len()
    );
    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, main_area, &mut app.package_detail.state.clone());

    let help = Paragraph::new(" Esc:back  ↑↓:navigate  y:yank  u:unyank  ?:help")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, help_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(v: &str, status: PackageStatus) -> PackageSummary {
        PackageSummary {
            registry: "npm-proxy".to_owned(),
            name: "left-pad".to_owned(),
            version: v.to_owned(),
            artifact: None,
            status,
            access_count: 0,
        }
    }

    #[test]
    fn set_items_selects_first_when_non_empty() {
        let mut w = PackageDetailWidget::new();
        w.set_items(vec![
            version("1.0.0", PackageStatus::Available),
            version("1.1.0", PackageStatus::Available),
        ]);
        assert_eq!(w.state.selected(), Some(0));
        assert_eq!(w.selected().unwrap().version, "1.0.0");
    }

    #[test]
    fn set_items_empty_clears_selection() {
        let mut w = PackageDetailWidget::new();
        w.set_items(vec![version("1.0.0", PackageStatus::Available)]);
        assert_eq!(w.state.selected(), Some(0));

        w.set_items(vec![]);
        assert_eq!(w.state.selected(), None);
        assert!(w.selected().is_none());
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let mut w = PackageDetailWidget::new();
        w.set_items(vec![
            version("1.0.0", PackageStatus::Available),
            version(
                "1.1.0",
                PackageStatus::Blocked {
                    reason: "cve".into(),
                },
            ),
        ]);

        w.next();
        assert_eq!(w.state.selected(), Some(1));
        w.next();
        assert_eq!(w.state.selected(), Some(0));

        w.prev();
        assert_eq!(w.state.selected(), Some(1));
    }

    #[test]
    fn next_and_prev_on_empty_list_are_noops() {
        let mut w = PackageDetailWidget::new();
        w.next();
        w.prev();
        assert_eq!(w.state.selected(), None);
    }
}
