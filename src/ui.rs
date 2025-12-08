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
use tokio::sync::mpsc;

use crate::agent::Agent;

#[derive(Debug)]
pub enum StreamEvent {
    Chunk(String),
    Done(Result<String, String>),
}

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
    /// Content being streamed (for the current message)
    pub streaming_content: String,
    /// Whether to show the welcome screen
    pub show_welcome: bool,
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
            messages: vec![],  // Start empty, welcome screen shows instead
            should_quit: false,
            waiting: false,
            streaming_content: String::new(),
            show_welcome: true,  // Show welcome screen on startup
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

    /// Start streaming a new message
    pub fn start_streaming(&mut self) {
        self.streaming_content.clear();
        self.waiting = true;
    }

    /// Append chunk to the streaming message
    pub fn append_streaming_chunk(&mut self, chunk: &str) {
        self.streaming_content.push_str(chunk);
    }

    /// Finish streaming and add the complete message to history
    pub fn finish_streaming(&mut self, role: MessageRole) {
        if !self.streaming_content.is_empty() {
            self.messages.push(ChatMessage {
                role,
                content: self.streaming_content.clone(),
            });
            self.streaming_content.clear();
        }
        self.waiting = false;
    }

    /// Dismiss the welcome screen and start the chat
    pub fn dismiss_welcome(&mut self) {
        self.show_welcome = false;
        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: "Welcome to Rustline! Type your message and press Enter.".to_string(),
        });
    }
}

/// Run the TUI application
pub async fn run_tui(agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamEvent>();

    loop {
        terminal.draw(|f| ui(f, &app))?;

        if app.should_quit {
            break;
        }

        if let Ok(event) = rx.try_recv() {
            match event {
                StreamEvent::Chunk(chunk) => {
                    app.append_streaming_chunk(&chunk);
                }
                StreamEvent::Done(Ok(_)) => {
                    app.finish_streaming(MessageRole::Assistant);
                }
                StreamEvent::Done(Err(e)) => {
                    app.finish_streaming(MessageRole::Assistant);
                    app.add_message(MessageRole::System, format!("Error: {}", e));
                }
            }
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
                            if app.show_welcome {
                                app.dismiss_welcome();
                            } else if !app.input.is_empty() && !app.waiting {
                                let user_input = app.input.clone();
                                app.add_message(MessageRole::User, user_input.clone());
                                app.clear_input();
                                app.start_streaming();

                                // Spawn task to handle agent response with streaming
                                let tx_clone = tx.clone();
                                let mut agent_clone = agent.clone();
                                tokio::spawn(async move {
                                    match agent_clone.handle_message_stream(&user_input, |chunk| {
                                        let _ = tx_clone.send(StreamEvent::Chunk(chunk.to_string()));
                                    }).await {
                                        Ok(response) => {
                                            let _ = tx_clone.send(StreamEvent::Done(Ok(response)));
                                        }
                                        Err(e) => {
                                            let _ = tx_clone.send(StreamEvent::Done(Err(e.to_string())));
                                        }
                                    }
                                });
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
    // Show welcome screen if enabled
    if app.show_welcome {
        render_welcome_screen(f);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),      // Chat area
            Constraint::Length(3),   // Input area
        ])
        .split(f.area());

    // Chat history area
    let mut messages: Vec<ListItem> = app
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

    if app.waiting && !app.streaming_content.is_empty() {
        let streaming_text = format!("Rustline: {}▊", app.streaming_content);
        messages.push(
            ListItem::new(Text::from(streaming_text))
                .style(Style::default().fg(Color::Green)),
        );
    }

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

/// Render the welcome screen
fn render_welcome_screen(f: &mut Frame) {
    use ratatui::{
        layout::{Alignment, Constraint, Direction, Layout},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Paragraph},
    };

    let area = f.area();

    // Create a centered layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Min(15),
            Constraint::Percentage(20),
        ])
        .split(area);

    // ASCII art logo
    let logo = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  ____            _   _ _            ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(" |  _ \\ _   _ ___| |_| (_)_ __   ___ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(" | |_) | | | / __| __| | | '_ \\ / _ \\", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(" |  _ <| |_| \\__ \\ |_| | | | | |  __/", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(" |_| \\_\\\\__,_|___/\\__|_|_|_| |_|\\___|", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("     A Rust-Based Local AI Agent CLI", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
    ];

    let logo_para = Paragraph::new(logo)
        .alignment(Alignment::Center)
        .block(Block::default());

    f.render_widget(logo_para, chunks[1]);

    // Information section
    let info = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("✨ Features:", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("Fully offline operation with Ollama"),
        Line::from("Real-time streaming responses"),
        Line::from("ReAct-style reasoning with tool execution"),
        Line::from("Context-aware conversations"),
        Line::from(""),
        Line::from(vec![
            Span::styled("Quick Start:", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("Press Enter to begin chatting"),
        Line::from("Type !tools to see available tools"),
        Line::from("Press Ctrl+C or Esc to quit"),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::Gray)),
            Span::styled("Enter", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(" to start...", Style::default().fg(Color::Gray)),
        ]),
    ];

    let info_para = Paragraph::new(info)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Welcome ")
                .title_alignment(Alignment::Center),
        );

    // Calculate centered position for info box
    let info_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Percentage(70),
            Constraint::Percentage(15),
        ])
        .split(chunks[2]);

    f.render_widget(info_para, info_chunks[1]);
}
