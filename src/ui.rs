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
    widgets::{Block, Borders, List, ListItem, Paragraph, ListState},
    Frame, Terminal,
};
use tokio::sync::mpsc;
use textwrap;

use crate::agent::Agent;

#[derive(Debug)]
pub enum StreamEvent {
    Chunk(String),
    Thinking(String),
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
    /// Thinking process content
    pub thinking_content: String,
    /// Whether to show the welcome screen
    pub show_welcome: bool,
    /// State for the chat list
    pub list_state: ListState,
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
            thinking_content: String::new(),
            show_welcome: true,
            list_state: ListState::default(),
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
        self.thinking_content.clear();
        self.waiting = true;
    }

    /// Append chunk to the streaming message
    pub fn append_streaming_chunk(&mut self, chunk: &str) {
        self.streaming_content.push_str(chunk);
    }

    /// Update thinking content
    pub fn update_thinking(&mut self, content: &str) {
        self.thinking_content = content.to_string();
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
        self.thinking_content.clear();
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
        terminal.draw(|f| ui(f, &mut app))?;

        if app.should_quit {
            break;
        }

        if let Ok(event) = rx.try_recv() {
            match event {
                StreamEvent::Chunk(chunk) => {
                    app.append_streaming_chunk(&chunk);
                }
                StreamEvent::Thinking(content) => {
                    app.update_thinking(&content);
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
                                    match agent_clone.handle_message_stream(
                                        &user_input, 
                                        |chunk| {
                                            let _ = tx_clone.send(StreamEvent::Chunk(chunk.to_string()));
                                        },
                                        |think| {
                                            let _ = tx_clone.send(StreamEvent::Thinking(think.to_string()));
                                        }
                                    ).await {
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
fn ui(f: &mut Frame, app: &mut App) {
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
            
            // Safe width calculation with text wrapping
            let width = (chunks[0].width as usize).saturating_sub(4).max(1);
            
            let wrapped_lines: Vec<String> = textwrap::wrap(&content, width)
                .into_iter()
                .map(|s| s.to_string())
                .collect();
            
            let text = Text::from(wrapped_lines.join("\n"));
            ListItem::new(text).style(style)
        })
        .collect();

    if app.waiting {
        if !app.thinking_content.is_empty() {
            let width = (chunks[0].width as usize).saturating_sub(4).max(1);
            let wrapped_thinking: Vec<String> = textwrap::wrap(&format!("Thinking: {}", app.thinking_content), width)
                .into_iter()
                .map(|s| s.to_string())
                .collect();
            messages.push(
                ListItem::new(Text::from(wrapped_thinking.join("\n")))
                    .style(Style::default().fg(Color::DarkGray).add_modifier(ratatui::style::Modifier::ITALIC)),
            );
        }
        
        if !app.streaming_content.is_empty() {
            let width = (chunks[0].width as usize).saturating_sub(4).max(1);
            let streaming_text = format!("Rustline: {}▊", app.streaming_content);
            let wrapped_streaming: Vec<String> = textwrap::wrap(&streaming_text, width)
                .into_iter()
                .map(|s| s.to_string())
                .collect();
            messages.push(
                ListItem::new(Text::from(wrapped_streaming.join("\n")))
                    .style(Style::default().fg(Color::Green)),
            );
        }
    }

    // Auto-scroll to the bottom
    if !messages.is_empty() {
        app.list_state.select(Some(messages.len() - 1));
    }

    let messages_list = List::new(messages)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Chat History")
                .style(Style::default().fg(Color::White)),
        );

    f.render_stateful_widget(messages_list, chunks[0], &mut app.list_state);

    // Input area
    let input_text = if app.waiting {
        format!("{} (thinking...)", app.input)
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

    // Create three-section layout matching the main UI
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),  // Top cards
            Constraint::Min(15),    // Main welcome content
            Constraint::Length(3),  // Bottom hint
        ])
        .split(area);

    // Split top into two feature cards
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(main_chunks[0]);

    // Left card - Weather Preview
    let left_card = vec![
        Line::from(vec![
            Span::styled("☀️  Sunny", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("-22°C", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Toronto, ON", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled("(Live weather soon!)", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let left_para = Paragraph::new(left_card)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("🌤️  Weather")
                .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(left_para, top_chunks[0]);

    // Right card - Time & System Status
    let right_card = vec![
        Line::from(vec![
            Span::styled("🕐 ", Style::default().fg(Color::Blue)),
            Span::styled("--:--:--", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("✨ System Ready", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("🔒 100% Private", Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("📡 Fully Offline", Style::default().fg(Color::Yellow)),
        ]),
    ];

    let right_para = Paragraph::new(right_card)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("⏰ Time & Status")
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(right_para, top_chunks[1]);

    // Main welcome content
    let welcome_content = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("      ╔═══════════════════════════════════╗", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("      ║                                   ║", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("      ║  ", Style::default().fg(Color::Cyan)),
            Span::styled("🦀 RUSTLINE AI AGENT 🦀", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled("      ║", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("      ║                                   ║", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("      ╚═══════════════════════════════════╝", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("           Your Personal AI Assistant", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("✨ Features:", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("   🎯 ", Style::default().fg(Color::Blue)),
            Span::styled("Smart Context-Aware Responses", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("   🔧 ", Style::default().fg(Color::Blue)),
            Span::styled("ReAct-Style Tool Execution", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("   💬 ", Style::default().fg(Color::Blue)),
            Span::styled("Natural Conversation Flow", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("   🚀 ", Style::default().fg(Color::Blue)),
            Span::styled("Lightning Fast Performance", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("     ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("          Press ", Style::default().fg(Color::Gray)),
            Span::styled("⏎ Enter", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
            Span::styled(" to start your journey!", Style::default().fg(Color::Gray)),
        ]),
    ];

    let welcome_para = Paragraph::new(welcome_content)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("🌟 Welcome 🌟")
                .title_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
                .title_alignment(Alignment::Center)
                .border_style(Style::default().fg(Color::Green))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(welcome_para, main_chunks[1]);

    // Bottom hint bar
    let hint_text = vec![
        Line::from(vec![
            Span::styled("💡 Tip: ", Style::default().fg(Color::Yellow)),
            Span::styled("Press ", Style::default().fg(Color::Gray)),
            Span::styled("Ctrl+C", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(" or ", Style::default().fg(Color::Gray)),
            Span::styled("Esc", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(" to exit anytime", Style::default().fg(Color::Gray)),
        ]),
    ];

    let hint_para = Paragraph::new(hint_text)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(hint_para, main_chunks[2]);
}
