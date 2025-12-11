use std::io;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    text::Text,
    widgets::{Block, Borders, List, ListItem, ListState},
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
    /// Current time display (TODO: update periodically)
    pub current_time: String,
    /// Weather information (TODO: fetch from API)
    pub weather_info: WeatherInfo,
}

#[derive(Clone)]
pub struct WeatherInfo {
    pub temperature: String,
    pub condition: String,
    pub location: String,
    pub icon: String,
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
            current_time: Self::get_current_time(),
            weather_info: WeatherInfo::default(),
        }
    }

    /// Update the current time display
    pub fn update_time(&mut self) {
        self.current_time = Self::get_current_time();
    }

    /// Get current time as formatted string
    fn get_current_time() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // TODO: Use chrono crate for proper timezone formatting
        let hours = (now / 3600) % 24;
        let minutes = (now / 60) % 60;
        let seconds = now % 60;
        
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    }

    /// Update weather information
    #[allow(dead_code)]
    pub fn update_weather(&mut self) {
        // TODO: Implement actual weather API call
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

impl WeatherInfo {
    pub fn default() -> Self {
        WeatherInfo {
            // TODO: Fetch real weather data from api 
            temperature: "-22°C".to_string(),
            condition: "Sunny".to_string(),
            location: "Toronto, ON".to_string(),
            icon: "☀️".to_string(),
        }
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
        // Update time before each render
        app.update_time();
        
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
        
        // TODO: Periodically update weather

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
    use ratatui::{
        layout::{Alignment, Constraint, Direction, Layout},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::Paragraph,
    };

    // Show welcome screen if enabled
    if app.show_welcome {
        render_welcome_screen(f);
        return;
    }

    // Create three-section layout: status bar, chat history, input
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // Status bar
            Constraint::Min(10),    // Chat history
            Constraint::Length(3),  // Input area
        ])
        .split(f.area());

    // Split top status bar into two cards
    let status_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(main_chunks[0]);

    // Left status card - Weather Display
    let weather_lines = vec![
        Line::from(vec![
            Span::styled(format!("{} ", app.weather_info.icon), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(&app.weather_info.condition, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(&app.weather_info.temperature, Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(&app.weather_info.location, Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let weather_para = Paragraph::new(weather_lines)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("🌤️  Weather")
                .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(weather_para, status_chunks[0]);

    // Right status card - Time Display
    let status_icon = if app.waiting { "⚡" } else { "✨" };
    let status_text = if app.waiting { "Processing" } else { "Ready" };
    let status_color = if app.waiting { Color::Yellow } else { Color::Green };
    
    let time_lines = vec![
        Line::from(vec![
            Span::styled("🕐 ", Style::default().fg(Color::Blue)),
            Span::styled(&app.current_time, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(format!("{} ", status_icon), Style::default().fg(status_color)),
            Span::styled(status_text, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(format!("💬 {} msgs", app.messages.len()), Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let time_para = Paragraph::new(time_lines)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("⏰ Time & Status")
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(time_para, status_chunks[1]);

    // Chat history with modern message bubbles
    let mut messages: Vec<ListItem> = app
        .messages
        .iter()
        .enumerate()
        .map(|(idx, msg)| {
            let (style, icon, prefix, decorator_left, decorator_right) = match msg.role {
                MessageRole::User => (
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    "👤",
                    "You",
                    "╭─",
                    "─╮"
                ),
                MessageRole::Assistant => (
                    Style::default().fg(Color::Green),
                    "🤖",
                    "Rustline",
                    "┌─",
                    "─┐"
                ),
                MessageRole::System => (
                    Style::default().fg(Color::Yellow),
                    "ℹ️",
                    "System",
                    "┏━",
                    "━┓"
                ),
            };

            let header = format!("{} {} {} {}", decorator_left, icon, prefix, decorator_right);
            
            // Safe width calculation with text wrapping
            let width = (main_chunks[1].width as usize).saturating_sub(6).max(1);
            
            let wrapped_lines: Vec<String> = textwrap::wrap(&msg.content, width)
                .into_iter()
                .map(|s| format!("│ {}", s))
                .collect();
            
            let footer = match msg.role {
                MessageRole::User => "╰─────────────────────────",
                MessageRole::Assistant => "└─────────────────────────",
                MessageRole::System => "┗━━━━━━━━━━━━━━━━━━━━━━━━━",
            };
            
            let mut full_content = vec![header];
            full_content.extend(wrapped_lines);
            full_content.push(footer.to_string());
            
            if idx < app.messages.len() - 1 {
                full_content.push("".to_string());
            }
            
            let text = Text::from(full_content.join("\n"));
            ListItem::new(text).style(style)
        })
        .collect();

    if app.waiting {
        if !app.thinking_content.is_empty() {
            let width = (main_chunks[1].width as usize).saturating_sub(6).max(1);
            
            let thinking_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let frame_idx = app.messages.len() % thinking_frames.len();
            let spinner = thinking_frames[frame_idx];
            
            let header = format!("┌─ {} Thinking... ─┐", spinner);
            let wrapped_thinking: Vec<String> = textwrap::wrap(&app.thinking_content, width)
                .into_iter()
                .map(|s| format!("│ {}", s))
                .collect();
            
            let mut thinking_content = vec![header];
            thinking_content.extend(wrapped_thinking);
            thinking_content.push("└─────────────────────────".to_string());
            thinking_content.push("".to_string());
            
            messages.push(
                ListItem::new(Text::from(thinking_content.join("\n")))
                    .style(Style::default().fg(Color::Magenta).add_modifier(Modifier::ITALIC | Modifier::DIM)),
            );
        }
        
        if !app.streaming_content.is_empty() {
            let width = (main_chunks[1].width as usize).saturating_sub(6).max(1);
            
            let typing_dots = ["   ", ".  ", ".. ", "..."];
            let dot_idx = (app.streaming_content.len() / 10) % typing_dots.len();
            let dots = typing_dots[dot_idx];
            
            let header = format!("┌─ 🤖 Rustline {} ─┐", dots);
            let wrapped_streaming: Vec<String> = textwrap::wrap(&format!("{}▊", app.streaming_content), width)
                .into_iter()
                .map(|s| format!("│ {}", s))
                .collect();
            
            let mut streaming_msg = vec![header];
            streaming_msg.extend(wrapped_streaming);
            streaming_msg.push("└─────────────────────────".to_string());
            
            messages.push(
                ListItem::new(Text::from(streaming_msg.join("\n")))
                    .style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            );
        }
    }

    // Auto-scroll to the bottom
    if !messages.is_empty() {
        app.list_state.select(Some(messages.len() - 1));
    }

    let chat_title = if app.waiting {
        "💬 Chat History ⚡ (AI is thinking...)"
    } else if app.messages.is_empty() {
        "💬 Chat History ✨ (Start a conversation!)"
    } else {
        "💬 Chat History ✓"
    };
    
    let messages_list = List::new(messages)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(chat_title)
                .title_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(if app.waiting { Color::Yellow } else { Color::Green }))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_stateful_widget(messages_list, main_chunks[1], &mut app.list_state);

    let (input_display, input_style, input_border_color, input_title) = if app.waiting {
        (
            format!("⏳ {} (AI is processing...)", app.input),
            Style::default().fg(Color::Yellow),
            Color::Yellow,
            "📝 Input [⏸ Waiting for response...]"
        )
    } else if app.input.is_empty() {
        (
            "✨ Start typing your message... (Press Enter to send)".to_string(),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            Color::Cyan,
            "📝 Input [Ready]"
        )
    } else {
        (
            format!("✍️  {} █", app.input),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            Color::Green,
            "📝 Input [Typing...]"
        )
    };

    let input = Paragraph::new(input_display)
        .style(input_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(input_title)
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(input_border_color).add_modifier(Modifier::BOLD))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(input, main_chunks[2]);
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
