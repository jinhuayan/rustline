use std::io;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    text::Text,
    widgets::{Block, Borders, List, ListItem, ListState},
};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::agent::Agent;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;

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
            messages: vec![], // Start empty, welcome screen shows instead
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

    /// Get current time as formatted string in local timezone
    fn get_current_time() -> String {
        use chrono::Local;
        let now = Local::now();
        now.format("%I:%M:%S %p").to_string()
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
            temperature: "...".to_string(),
            condition: "Loading".to_string(),
            location: "Detecting...".to_string(),
            icon: "⏳".to_string(),
        }
    }

    /// Get weather icon from condition text
    fn condition_to_icon(condition: &str) -> &'static str {
        let condition_lower = condition.to_lowercase();
        if condition_lower.contains("clear") || condition_lower.contains("sunny") {
            "☀️"
        } else if condition_lower.contains("cloud") || condition_lower.contains("overcast") {
            "☁️"
        } else if condition_lower.contains("partly") || condition_lower.contains("mix") {
            "⛅"
        } else if condition_lower.contains("rain") && condition_lower.contains("snow") {
            "🌨️"
        } else if condition_lower.contains("rain") || condition_lower.contains("drizzle") {
            "🌧️"
        } else if condition_lower.contains("snow") || condition_lower.contains("flurr") {
            "❄️"
        } else if condition_lower.contains("thunder") || condition_lower.contains("storm") {
            "⛈️"
        } else if condition_lower.contains("fog") || condition_lower.contains("mist") {
            "🌫️"
        } else {
            "🌡️"
        }
    }

    /// Fetch weather data from api with automatic location detection
    pub async fn fetch_weather() -> Result<Self, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();

        let location_data: LocationResponse = client
            .get("http://ip-api.com/json/?fields=status,city,regionName,country,lat,lon,timezone")
            .send()
            .await?
            .json()
            .await?;

        if location_data.status != "success" {
            return Err("Failed to detect location".into());
        }

        let latitude = location_data.lat;
        let longitude = location_data.lon;
        let location_name = format!(
            "{}, {}",
            location_data.city,
            location_data.region_name
        );

        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,weather_code&temperature_unit=celsius&timezone={}",
            latitude, longitude, location_data.timezone
        );

        let weather_data: OpenMeteoResponse = client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;

        let temperature = format!("{}°C", weather_data.current.temperature_2m.round() as i32);
        let condition = Self::weather_code_to_condition(weather_data.current.weather_code);
        let icon = Self::condition_to_icon(&condition);

        Ok(WeatherInfo {
            temperature,
            condition,
            location: location_name,
            icon: icon.to_string(),
        })
    }

    fn weather_code_to_condition(code: i32) -> String {
        match code {
            0 => "Clear Sky".to_string(),
            1 => "Mainly Clear".to_string(),
            2 => "Partly Cloudy".to_string(),
            3 => "Overcast".to_string(),
            45 | 48 => "Foggy".to_string(),
            51 | 53 | 55 => "Drizzle".to_string(),
            56 | 57 => "Freezing Drizzle".to_string(),
            61 | 63 | 65 => "Rain".to_string(),
            66 | 67 => "Freezing Rain".to_string(),
            71 | 73 | 75 => "Snow".to_string(),
            77 => "Snow Grains".to_string(),
            80..=82 => "Rain Showers".to_string(),
            85 | 86 => "Snow Showers".to_string(),
            95 => "Thunderstorm".to_string(),
            96 | 99 => "Thunderstorm with Hail".to_string(),
            _ => "Unknown".to_string(),
        }
    }
}

#[derive(Deserialize)]
struct LocationResponse {
    status: String,
    city: String,
    #[serde(rename = "regionName")]
    region_name: String,
    #[allow(dead_code)]
    country: String,
    lat: f64,
    lon: f64,
    timezone: String,
}

#[derive(Deserialize)]
struct OpenMeteoResponse {
    current: CurrentWeather,
}

#[derive(Deserialize)]
struct CurrentWeather {
    temperature_2m: f64,
    weather_code: i32,
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
    // Share a single Agent instance across all user inputs to preserve pending state
    let shared_agent = Arc::new(AsyncMutex::new(agent));

    let (tx, mut rx) = mpsc::unbounded_channel::<StreamEvent>();

    // Fetch initial weather data
    if let Ok(weather) = WeatherInfo::fetch_weather().await {
        app.weather_info = weather;
    }

    // Track last weather update time
    let mut last_weather_update = std::time::Instant::now();

    loop {
        // Update time before each render
        app.update_time();

        // Update weather every 5 minutes
        if last_weather_update.elapsed().as_secs() >= 300 {
            if let Ok(weather) = WeatherInfo::fetch_weather().await {
                app.weather_info = weather;
            }
            last_weather_update = std::time::Instant::now();
        }

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
        if event::poll(std::time::Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
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
                        let shared_agent_clone = Arc::clone(&shared_agent);
                        tokio::spawn(async move {
                                    let mut agent_guard = shared_agent_clone.lock().await;
                            match agent_guard
                                .handle_message_stream(
                                    &user_input,
                                    |chunk| {
                                        let _ =
                                            tx_clone.send(StreamEvent::Chunk(chunk.to_string()));
                                    },
                                    |think| {
                                        let _ =
                                            tx_clone.send(StreamEvent::Thinking(think.to_string()));
                                    },
                                )
                                .await
                            {
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
            Constraint::Length(5), // Status bar
            Constraint::Min(10),   // Chat history
            Constraint::Length(3), // Input area
        ])
        .split(f.area());

    // Split top status bar into two cards
    let status_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[0]);

    // Left status card - Weather Display
    let weather_lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", app.weather_info.icon),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                &app.weather_info.condition,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            &app.weather_info.temperature,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            &app.weather_info.location,
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    let weather_para = Paragraph::new(weather_lines)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("🌤️  Weather")
                .title_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
                .border_style(
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(weather_para, status_chunks[0]);

    // Right status card - Time Display
    let status_icon = if app.waiting { "⚡" } else { "✨" };
    let status_text = if app.waiting { "Processing" } else { "Ready" };
    let status_color = if app.waiting {
        Color::Yellow
    } else {
        Color::Green
    };

    let time_lines = vec![
        Line::from(vec![
            Span::styled("🕐 ", Style::default().fg(Color::Blue)),
            Span::styled(
                &app.current_time,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("{} ", status_icon),
                Style::default().fg(status_color),
            ),
            Span::styled(
                status_text,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            format!("💬 {} msgs", app.messages.len()),
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    let time_para = Paragraph::new(time_lines)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("⏰ Time & Status")
                .title_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .border_style(
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )
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
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    "👤",
                    "You",
                    "╭─",
                    "─╮",
                ),
                MessageRole::Assistant => (
                    Style::default().fg(Color::Green),
                    "🤖",
                    "Rustline",
                    "┌─",
                    "─┐",
                ),
                MessageRole::System => (
                    Style::default().fg(Color::Yellow),
                    "ℹ️",
                    "System",
                    "┏━",
                    "━┓",
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
                ListItem::new(Text::from(thinking_content.join("\n"))).style(
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::ITALIC | Modifier::DIM),
                ),
            );
        }

        if !app.streaming_content.is_empty() {
            let width = (main_chunks[1].width as usize).saturating_sub(6).max(1);

            let typing_dots = ["   ", ".  ", ".. ", "..."];
            let dot_idx = (app.streaming_content.len() / 10) % typing_dots.len();
            let dots = typing_dots[dot_idx];

            let header = format!("┌─ 🤖 Rustline {} ─┐", dots);
            let wrapped_streaming: Vec<String> =
                textwrap::wrap(&format!("{}▊", app.streaming_content), width)
                    .into_iter()
                    .map(|s| format!("│ {}", s))
                    .collect();

            let mut streaming_msg = vec![header];
            streaming_msg.extend(wrapped_streaming);
            streaming_msg.push("└─────────────────────────".to_string());

            messages.push(
                ListItem::new(Text::from(streaming_msg.join("\n"))).style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
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

    let messages_list = List::new(messages).block(
        Block::default()
            .borders(Borders::ALL)
            .title(chat_title)
            .title_style(
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(if app.waiting {
                Color::Yellow
            } else {
                Color::Green
            }))
            .style(Style::default().bg(Color::Black)),
    );

    f.render_stateful_widget(messages_list, main_chunks[1], &mut app.list_state);

    let (input_display, input_style, input_border_color, input_title) = if app.waiting {
        (
            format!("⏳ {} (AI is processing...)", app.input),
            Style::default().fg(Color::Yellow),
            Color::Yellow,
            "📝 Input [⏸ Waiting for response...]",
        )
    } else if app.input.is_empty() {
        (
            "✨ Start typing your message... (Press Enter to send)".to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
            Color::Cyan,
            "📝 Input [Ready]",
        )
    } else {
        (
            format!("✍️  {} █", app.input),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            Color::Green,
            "📝 Input [Typing...]",
        )
    };

    let awaiting_confirm = pending_confirmation_active(app);
    let input_title = if awaiting_confirm {
        "Input (Awaiting confirmation: yes/no, or !do/!skip)"
    } else {
        "Input (Ctrl+C or Esc to quit)"
    };

    let input = Paragraph::new(input_display).style(input_style).block(
        Block::default()
            .borders(Borders::ALL)
            .title(input_title)
            .title_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(
                Style::default()
                    .fg(input_border_color)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(Color::Black)),
    );

    f.render_widget(input, main_chunks[2]);
}

/// Detect whether the last assistant message is a human-style confirmation prompt.
fn pending_confirmation_active(app: &App) -> bool {
    if let Some(last) = app.messages.iter().rev().find(|m| m.role == MessageRole::Assistant) {
        let lc = last.content.to_lowercase();
        return lc.contains("i am going to create this file") || lc.contains("i am going to open this file");
    }
    false
}

/// Try to pretty-print JSON if the input is JSON; otherwise return the original string.
fn pretty_json_if_possible(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => {
                // Special-case for create_file exists protection: show a clear message.
                if let Some(obj) = v.as_object() {
                    if obj.get("exists").and_then(|b| b.as_bool()).unwrap_or(false) {
                        if let Some(msg) = obj.get("message").and_then(|m| m.as_str()) {
                            let path = obj.get("path").and_then(|p| p.as_str()).unwrap_or("");
                            return format!("{}\nPath: {}", msg, path);
                        }
                    }
                }
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| s.to_string())
            }
            Err(_) => s.to_string(),
        }
    } else {
        s.to_string()
    }
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
            Constraint::Length(7), // Top cards
            Constraint::Min(15),   // Main welcome content
            Constraint::Length(3), // Bottom hint
        ])
        .split(area);

    // Split top into two feature cards
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[0]);

    // Left card - Weather Preview
    let left_card = vec![
        Line::from(vec![Span::styled(
            "⏳  Loading",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "...",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Detecting location...",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(vec![Span::styled(
            "(Auto-detected weather!)",
            Style::default().fg(Color::Green),
        )]),
    ];

    let left_para = Paragraph::new(left_card)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("🌤️  Weather")
                .title_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
                .border_style(
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(left_para, top_chunks[0]);

    // Right card - Time & System Status
    let right_card = vec![
        Line::from(vec![
            Span::styled("🕐 ", Style::default().fg(Color::Blue)),
            Span::styled(
                "--:--:--",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            "✨ System Ready",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "🔒 100% Private",
            Style::default().fg(Color::Green),
        )]),
        Line::from(vec![Span::styled(
            "📡 Fully Offline",
            Style::default().fg(Color::Yellow),
        )]),
    ];

    let right_para = Paragraph::new(right_card)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("⏰ Time & Status")
                .title_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .border_style(
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(right_para, top_chunks[1]);

    // Main welcome content
    let welcome_content = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "      ╔═══════════════════════════════════╗",
            Style::default().fg(Color::Cyan),
        )]),
        Line::from(vec![Span::styled(
            "      ║                                   ║",
            Style::default().fg(Color::Cyan),
        )]),
        Line::from(vec![
            Span::styled("      ║  ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "🦀 RUSTLINE AI AGENT 🦀",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("      ║", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![Span::styled(
            "      ║                                   ║",
            Style::default().fg(Color::Cyan),
        )]),
        Line::from(vec![Span::styled(
            "      ╚═══════════════════════════════════╝",
            Style::default().fg(Color::Cyan),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "           Your Personal AI Assistant",
            Style::default().fg(Color::Yellow),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "✨ Features:",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled("   🎯 ", Style::default().fg(Color::Blue)),
            Span::styled(
                "Smart Context-Aware Responses",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("   🔧 ", Style::default().fg(Color::Blue)),
            Span::styled(
                "ReAct-Style Tool Execution",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("   💬 ", Style::default().fg(Color::Blue)),
            Span::styled(
                "Natural Conversation Flow",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("   🚀 ", Style::default().fg(Color::Blue)),
            Span::styled(
                "Lightning Fast Performance",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "     ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("          Press ", Style::default().fg(Color::Gray)),
            Span::styled(
                "⏎ Enter",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ),
            Span::styled(" to start your journey!", Style::default().fg(Color::Gray)),
        ]),
    ];

    let welcome_para = Paragraph::new(welcome_content)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("🌟 Welcome 🌟")
                .title_style(
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )
                .title_alignment(Alignment::Center)
                .border_style(Style::default().fg(Color::Green))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(welcome_para, main_chunks[1]);

    // Bottom hint bar
    let hint_text = vec![Line::from(vec![
        Span::styled("💡 Tip: ", Style::default().fg(Color::Yellow)),
        Span::styled("Press ", Style::default().fg(Color::Gray)),
        Span::styled(
            "Ctrl+C",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(Color::Gray)),
        Span::styled(
            "Esc",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to exit anytime", Style::default().fg(Color::Gray)),
    ])];

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
