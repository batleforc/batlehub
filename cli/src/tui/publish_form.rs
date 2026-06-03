use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use tui_input::Input;

use super::App;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PublishField {
    FilePath,
    Registry,
    Name,
    Version,
}

pub struct PublishFormWidget {
    pub fields: [Input; 4],
    pub active_field: PublishField,
    pub submitted: bool,
    pub error: Option<String>,
}

impl PublishFormWidget {
    pub fn new() -> Self {
        Self {
            fields: Default::default(),
            active_field: PublishField::FilePath,
            submitted: false,
            error: None,
        }
    }

    fn field_idx(f: PublishField) -> usize {
        match f {
            PublishField::FilePath => 0,
            PublishField::Registry => 1,
            PublishField::Name => 2,
            PublishField::Version => 3,
        }
    }

    pub fn active_input(&mut self) -> &mut Input {
        &mut self.fields[Self::field_idx(self.active_field)]
    }

    pub fn next_field(&mut self) {
        self.active_field = match self.active_field {
            PublishField::FilePath => PublishField::Registry,
            PublishField::Registry => PublishField::Name,
            PublishField::Name => PublishField::Version,
            PublishField::Version => PublishField::FilePath,
        };
    }

    pub fn prev_field(&mut self) {
        self.active_field = match self.active_field {
            PublishField::FilePath => PublishField::Version,
            PublishField::Registry => PublishField::FilePath,
            PublishField::Name => PublishField::Registry,
            PublishField::Version => PublishField::Name,
        };
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => self.next_field(),
            KeyCode::BackTab => self.prev_field(),
            KeyCode::Enter => {
                if self.active_field == PublishField::Version {
                    self.submitted = true;
                } else {
                    self.next_field();
                }
            }
            _ => {
                use tui_input::backend::crossterm::EventHandler;
                self.active_input()
                    .handle_event(&crossterm::event::Event::Key(key));
            }
        }
    }

    pub fn value(&self, f: PublishField) -> &str {
        self.fields[Self::field_idx(f)].value()
    }
}

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let block = Block::default()
        .title(" Publish Artifact ")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(inner);

    let fields = [
        (PublishField::FilePath, "File Path"),
        (PublishField::Registry, "Registry"),
        (PublishField::Name, "Package Name (auto-detected)"),
        (PublishField::Version, "Version (auto-detected)"),
    ];

    for (i, (field, label)) in fields.iter().enumerate() {
        let is_active = app.publish_form.active_field == *field;
        let border_style = if is_active {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let input_block = Block::default()
            .title(*label)
            .borders(Borders::ALL)
            .border_style(border_style);
        let value = app.publish_form.value(*field);
        let para = Paragraph::new(value).block(input_block);
        f.render_widget(para, chunks[i]);
    }

    let hint_text = if let Some(err) = &app.publish_form.error {
        format!("Error: {err}")
    } else {
        " Tab:next field  Enter:submit  Esc:cancel".to_string()
    };
    let hint_style = if app.publish_form.error.is_some() {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let hint = Paragraph::new(hint_text).style(hint_style);
    f.render_widget(hint, chunks[4]);
}
