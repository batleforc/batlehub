use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::api::setup::ProjectDetection;

use super::App;

pub struct SetupWizardWidget {
    pub items: Vec<ProjectDetection>,
    pub state: ListState,
    pub cwd: String,
}

impl SetupWizardWidget {
    pub fn new() -> Self {
        Self {
            items: vec![],
            state: ListState::default(),
            cwd: String::new(),
        }
    }

    pub fn set_items(&mut self, items: Vec<ProjectDetection>, cwd: String) {
        self.items = items;
        self.cwd = cwd;
        if !self.items.is_empty() {
            self.state.select(Some(0));
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

    pub fn selected(&self) -> Option<&ProjectDetection> {
        self.state.selected().and_then(|i| self.items.get(i))
    }
}

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let main_area = chunks[0];
    let footer_area = chunks[1];

    if app.setup_wizard.items.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No known project manifest files found in:",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                app.setup_wizard.cwd.as_str(),
                Style::default().fg(Color::Cyan),
            )),
            Line::from(""),
            Line::from("Supported: Cargo.toml, go.mod, package.json, pyproject.toml,"),
            Line::from("           pom.xml, composer.json, *.gemspec, *.nuspec, *.csproj,"),
            Line::from("           *.tf, environment.yml"),
        ])
        .block(
            Block::default()
                .title(format!(" Setup Wizard — {} ", app.setup_wizard.cwd))
                .borders(Borders::ALL),
        );
        f.render_widget(msg, main_area);
    } else {
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(0)])
            .split(main_area);

        let list_area = h_chunks[0];
        let detail_area = h_chunks[1];

        // Left: project list
        let items: Vec<ListItem> = app
            .setup_wizard
            .items
            .iter()
            .map(|d| {
                let base = if let Some(ref name) = d.package_name {
                    format!("{} ({})", d.registry_type, name)
                } else {
                    d.registry_type.to_string()
                };
                let label = if d.relative_path.is_empty() {
                    base
                } else {
                    format!("{base} [{}]", d.relative_path)
                };
                ListItem::new(Line::from(Span::styled(
                    label,
                    Style::default().fg(Color::White),
                )))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().title(" Detected ").borders(Borders::ALL))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        f.render_stateful_widget(list, list_area, &mut app.setup_wizard.state.clone());

        // Right: instructions for selected project
        let detail_text = if let Some(det) = app.setup_wizard.selected() {
            let lines: Vec<Line> = det
                .instructions
                .lines()
                .map(|l| Line::from(Span::raw(l)))
                .collect();
            lines
        } else {
            vec![Line::from(Span::styled(
                "Select a project on the left",
                Style::default().fg(Color::DarkGray),
            ))]
        };

        let detail = Paragraph::new(detail_text)
            .block(
                Block::default()
                    .title(" Setup instructions ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(detail, detail_area);
    }

    let footer = Paragraph::new(" ↑↓: select  Esc: back  ?: help")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, footer_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(registry_type: &'static str, name: Option<&str>) -> ProjectDetection {
        ProjectDetection {
            registry_type,
            package_name: name.map(|s| s.to_owned()),
            instructions: format!("setup {registry_type}"),
            relative_path: String::new(),
        }
    }

    #[test]
    fn set_items_selects_first_and_stores_cwd() {
        let mut w = SetupWizardWidget::new();
        w.set_items(
            vec![detection("npm", Some("left-pad")), detection("cargo", None)],
            "/home/user/project".to_owned(),
        );
        assert_eq!(w.state.selected(), Some(0));
        assert_eq!(w.cwd, "/home/user/project");
        assert_eq!(w.selected().unwrap().registry_type, "npm");
    }

    #[test]
    fn set_items_empty_leaves_selection_unset() {
        let mut w = SetupWizardWidget::new();
        w.set_items(vec![], "/tmp".to_owned());
        assert_eq!(w.state.selected(), None);
        assert!(w.selected().is_none());
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let mut w = SetupWizardWidget::new();
        w.set_items(
            vec![
                detection("npm", None),
                detection("cargo", None),
                detection("maven", None),
            ],
            "/tmp".to_owned(),
        );

        w.next();
        assert_eq!(w.state.selected(), Some(1));
        w.next();
        assert_eq!(w.state.selected(), Some(2));
        w.next();
        assert_eq!(w.state.selected(), Some(0));

        w.prev();
        assert_eq!(w.state.selected(), Some(2));
    }

    #[test]
    fn next_and_prev_on_empty_list_are_noops() {
        let mut w = SetupWizardWidget::new();
        w.next();
        w.prev();
        assert_eq!(w.state.selected(), None);
    }
}
