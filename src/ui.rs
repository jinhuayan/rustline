use std::io;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};

use crate::agent::Agent;

/// Represents the application state for the TUI
pub struct App {
    /// Current user input
    pub input: String,
    /// Chat history to display (user and assistant messages)
    pub messages: Vec<ChatMessage>,
    /// Whether the app should exit
    pub should_quit: bool,
    /// Whether we're currently waiting for a response
    pub waiting: bool,
}

#[derive(Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl App {
    pub fn new() -> Self {
        App {
            input: String::new(),
            messages: vec![ChatMessage {
                role: MessageRole::System,
                content: "Welcome to Rustline! Type your message and press Enter.".to_string(),
            }],
            should_quit: false,
            waiting: false,
        }
    }

    /// Add a character to the input
    pub fn enter_char(&mut self, c: char) {
        self.input.push(c);
    }

    /// Delete the last character from input
    pub fn delete_char(&mut self) {
        self.input.pop();
    }

    /// Clear the current input
    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    /// Add a message to the chat history
    pub fn add_message(&mut self, role: MessageRole, content: String) {
        self.messages.push(ChatMessage { role, content });
    }
}

/// Run the TUI application
pub async fn run_tui(mut agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    loop {
        terminal.draw(|f| ui(f, &app))?;

        if app.should_quit {
            break;
        }

        // Handle input events
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                            app.should_quit = true;
                        }
                        KeyCode::Char('d') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                            app.should_quit = true;
                        }
                        KeyCode::Enter => {
                            if !app.input.is_empty() && !app.waiting {
                                let user_input = app.input.clone();
                                app.add_message(MessageRole::User, user_input.clone());
                                app.clear_input();
                                app.waiting = true;

                                // Get response from agent
                                match agent.handle_message(&user_input).await {
                                    Ok(response) => {
                                        app.add_message(MessageRole::Assistant, response);
                                    }
                                    Err(e) => {
                                        app.add_message(
                                            MessageRole::System,
                                            format!("Error: {}", e),
                                        );
                                    }
                                }

                                app.waiting = false;
                            }
                        }
                        KeyCode::Char(c) => {
                            app.enter_char(c);
                        }
                        KeyCode::Backspace => {
                            app.delete_char();
                        }
                        KeyCode::Esc => {
                            app.should_quit = true;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

/// Render the UI
fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),      // Chat area
            Constraint::Length(3),   // Input area
        ])
        .split(f.area());

    // Chat history area
    let messages: Vec<ListItem> = app
        .messages
        .iter()
        .map(|msg| {
            let style = match msg.role {
                MessageRole::User => Style::default().fg(Color::Cyan),
                MessageRole::Assistant => Style::default().fg(Color::Green),
                MessageRole::System => Style::default().fg(Color::Yellow),
            };

            let prefix = match msg.role {
                MessageRole::User => "You: ",
                MessageRole::Assistant => "Rustline: ",
                MessageRole::System => "System: ",
            };

            let content = format!("{}{}", prefix, msg.content);
            ListItem::new(Text::from(content)).style(style)
        })
        .collect();

    let messages_list = List::new(messages)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Chat History")
                .style(Style::default().fg(Color::White)),
        );

    f.render_widget(messages_list, chunks[0]);

    // Input area
    let input_text = if app.waiting {
        format!("{} (waiting...)", app.input)
    } else {
        app.input.clone()
    };

    let input = Paragraph::new(input_text)
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input (Ctrl+C or Esc to quit)")
                .style(Style::default().fg(Color::White)),
        );

    f.render_widget(input, chunks[1]);
}
