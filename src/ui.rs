use std::io;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    widgets::{Block, Borders, List, ListItem, ListState, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::agent::Agent;
use crate::PersistenceState;
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
    /// Whether user is manually scrolling chat history
    pub manual_scroll: bool,
    /// Last known message count (for detecting when new messages arrive)
    pub last_message_count: usize,
    /// Number of rendered list items (one per visual line) for scrolling bounds
    pub rendered_items_count: usize,
    /// Current time display (TODO: update periodically)
    pub current_time: String,
    /// Weather information (TODO: fetch from API)
    pub weather_info: WeatherInfo,
    /// Current session information
    pub current_session_id: Option<String>,
    /// Session management mode (for UI state)
    pub session_mode: SessionMode,
    /// Available sessions for session management UI
    pub available_sessions: Vec<crate::persistence::memory_store::SessionInfo>,
    /// Selected session index in session management mode
    pub selected_session_index: usize,
    /// Current persistence state for display
    pub persistence_state: crate::PersistenceState,
}

#[derive(Clone, PartialEq)]
pub enum SessionMode {
    Normal,
    SessionList,
    SessionCreate,
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
            manual_scroll: false,
            last_message_count: 0,
            rendered_items_count: 0,
            current_time: Self::get_current_time(),
            weather_info: WeatherInfo::default(),
            current_session_id: None,
            session_mode: SessionMode::Normal,
            available_sessions: Vec::new(),
            selected_session_index: 0,
            persistence_state: PersistenceState::Enabled, // Default, will be updated
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
        // Reset auto-scroll to bottom on new assistant message
        self.manual_scroll = false;
    }

    /// Dismiss the welcome screen and start the chat
    pub fn dismiss_welcome(&mut self) {
        self.show_welcome = false;
        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: "Welcome to Rustline! Type your message and press Enter. Press F2 for session management.".to_string(),
        });
    }

    /// Enter session management mode
    pub fn enter_session_mode(&mut self) {
        self.session_mode = SessionMode::SessionList;
        self.selected_session_index = 0;
    }

    /// Exit session management mode
    pub fn exit_session_mode(&mut self) {
        self.session_mode = SessionMode::Normal;
    }

    /// Update the current session ID
    pub fn set_current_session(&mut self, session_id: Option<String>) {
        self.current_session_id = session_id;
    }

    /// Update available sessions list
    pub fn update_sessions(&mut self, sessions: Vec<crate::persistence::memory_store::SessionInfo>) {
        self.available_sessions = sessions;
        // Reset selection if it's out of bounds
        if self.selected_session_index >= self.available_sessions.len() && !self.available_sessions.is_empty() {
            self.selected_session_index = 0;
        }
    }

    /// Move selection up in session list
    pub fn select_previous_session(&mut self) {
        if !self.available_sessions.is_empty() {
            self.selected_session_index = if self.selected_session_index == 0 {
                self.available_sessions.len() - 1
            } else {
                self.selected_session_index - 1
            };
        }
    }

    /// Move selection down in session list
    pub fn select_next_session(&mut self) {
        if !self.available_sessions.is_empty() {
            self.selected_session_index = (self.selected_session_index + 1) % self.available_sessions.len();
        }
    }

    /// Get the currently selected session
    pub fn get_selected_session(&self) -> Option<&crate::persistence::memory_store::SessionInfo> {
        self.available_sessions.get(self.selected_session_index)
    }

    /// Load messages from agent history
    pub fn load_messages_from_agent(&mut self, agent_messages: &[crate::ollama::Message]) {
        self.messages.clear();
        for msg in agent_messages {
            let role = match msg.role.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                _ => MessageRole::System,
            };
            self.messages.push(ChatMessage {
                role,
                content: msg.content.clone(),
            });
        }
    }

    /// Set the persistence state for display
    pub fn set_persistence_state(&mut self, state: PersistenceState) {
        self.persistence_state = state;
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

/// Run the TUI application (backward compatibility)
pub async fn run_tui(agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
    run_tui_with_persistence_state(agent, PersistenceState::Enabled).await
}

/// Run the TUI application with persistence state information
pub async fn run_tui_with_persistence_state(mut agent: Agent, persistence_state: PersistenceState) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.set_persistence_state(persistence_state.clone());
    
    // Add system message for persistence state if needed
    match &persistence_state {
        PersistenceState::FailedFallback(error) => {
            app.add_message(MessageRole::System, format!(
                "⚠️ Persistence initialization failed: {}. Running in non-persistent mode - your session will not be saved.",
                error
            ));
        }
        PersistenceState::Disabled => {
            app.add_message(MessageRole::System, 
                "ℹ️ Persistence is disabled. Your session will not be saved.".to_string()
            );
        }
        PersistenceState::Enabled => {
            // Load current session and update app state
            if let Err(e) = agent.load_session(None) {
                eprintln!("Warning: Failed to load session: {}. Starting with empty session.", e);
            } else {
                app.set_current_session(agent.get_current_session_id());
                app.load_messages_from_agent(agent.get_history());
            }
        }
    }
    
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
        if event::poll(std::time::Duration::from_millis(100))? {
            let evt = event::read()?;
            match evt {
                Event::Key(key) if key.kind == KeyEventKind::Press => match (&app.session_mode, key.code) {
                    // Global quit commands
                (_, KeyCode::Char('c')) if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        app.should_quit = true;
                    }
                    (_, KeyCode::Char('d')) if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        app.should_quit = true;
                    }
                (_, KeyCode::Esc) => {
                    if app.session_mode != SessionMode::Normal {
                        app.exit_session_mode();
                    } else {
                        app.should_quit = true;
                    }
                }
                
                // Session management mode
                (SessionMode::SessionList, KeyCode::Up) => {
                    app.select_previous_session();
                }
                (SessionMode::SessionList, KeyCode::Down) => {
                    app.select_next_session();
                }
                (SessionMode::SessionList, KeyCode::Enter) => {
                    if let Some(selected_session) = app.get_selected_session() {
                        let session_id = selected_session.id.clone();
                        let mut agent_guard = shared_agent.lock().await;
                        match agent_guard.switch_session(&session_id) {
                            Ok(()) => {
                                app.set_current_session(Some(session_id));
                                app.load_messages_from_agent(agent_guard.get_history());
                                app.exit_session_mode();
                            }
                            Err(e) => {
                                app.add_message(MessageRole::System, format!("Failed to switch session: {}", e));
                            }
                        }
                    }
                }
                (SessionMode::SessionList, KeyCode::Char('n')) => {
                    // Create new session
                    let mut agent_guard = shared_agent.lock().await;
                    match agent_guard.create_new_session(Some("New Session".to_string())) {
                        Ok(session_id) => {
                            app.set_current_session(Some(session_id));
                            app.messages.clear();
                            app.exit_session_mode();
                        }
                        Err(e) => {
                            app.add_message(MessageRole::System, format!("Failed to create session: {}", e));
                        }
                    }
                }
                (SessionMode::SessionList, KeyCode::Char('d')) => {
                    // Delete selected session
                    if let Some(selected_session) = app.get_selected_session() {
                        let session_id = selected_session.id.clone();
                        let mut agent_guard = shared_agent.lock().await;
                        match agent_guard.delete_session(&session_id) {
                            Ok(()) => {
                                // Update sessions list
                                if let Ok(sessions) = agent_guard.list_sessions() {
                                    app.update_sessions(sessions);
                                }
                                // If we deleted the current session, clear messages
                                if app.current_session_id.as_ref() == Some(&session_id) {
                                    app.set_current_session(None);
                                    app.messages.clear();
                                }
                            }
                            Err(e) => {
                                app.add_message(MessageRole::System, format!("Failed to delete session: {}", e));
                            }
                        }
                    }
                }
                
                // Normal mode
                (SessionMode::Normal, KeyCode::F(2)) => {
                    // Enter session management mode
                    let mut agent_guard = shared_agent.lock().await;
                    if let Ok(sessions) = agent_guard.list_sessions() {
                        app.update_sessions(sessions);
                        app.enter_session_mode();
                    }
                }
                (SessionMode::Normal, KeyCode::Enter) => {
                    if app.show_welcome {
                        app.dismiss_welcome();
                    } else if !app.input.is_empty() && !app.waiting {
                        let user_input = app.input.clone();
                        app.add_message(MessageRole::User, user_input.clone());
                        app.clear_input();
                        app.start_streaming();
                        app.manual_scroll = false;

                        // Spawn task to handle agent response with streaming
                        let tx_clone = tx.clone();
                        let shared_agent_clone = Arc::clone(&shared_agent);
                        tokio::spawn(async move {
                            let mut agent_guard = shared_agent_clone.lock().await;
                            match agent_guard
                                .handle_message_stream(
                                    &user_input,
                                    |chunk| {
                                        let _ = tx_clone.send(StreamEvent::Chunk(chunk.to_string()));
                                    },
                                    |think| {
                                        let _ = tx_clone.send(StreamEvent::Thinking(think.to_string()));
                                    },
                                )
                                .await
                            {
                                Ok(response) => { let _ = tx_clone.send(StreamEvent::Done(Ok(response))); }
                                Err(e) => { let _ = tx_clone.send(StreamEvent::Done(Err(e.to_string()))); }
                            }
                        });
                    }
                }
                (SessionMode::Normal, KeyCode::Char(c)) => {
                    app.enter_char(c);
                }
                (SessionMode::Normal, KeyCode::Backspace) => {
                    app.delete_char();
                }
                // Scroll controls for chat history
                (SessionMode::Normal, KeyCode::Up) => {
                    if app.rendered_items_count > 0 {
                        let cur = app.list_state.selected().unwrap_or(app.rendered_items_count - 1);
                        let next = cur.saturating_sub(1);
                        app.list_state.select(Some(next));
                        app.manual_scroll = true;
                    }
                }
                (SessionMode::Normal, KeyCode::Down) => {
                    if app.rendered_items_count > 0 {
                        let cur = app.list_state.selected().unwrap_or(app.rendered_items_count - 1);
                        let next = (cur + 1).min(app.rendered_items_count.saturating_sub(1));
                        app.list_state.select(Some(next));
                        app.manual_scroll = true;
                    }
                }
                (SessionMode::Normal, KeyCode::PageUp) => {
                    if app.rendered_items_count > 0 {
                        let cur = app.list_state.selected().unwrap_or(app.rendered_items_count - 1);
                        let next = cur.saturating_sub(5);
                        app.list_state.select(Some(next));
                        app.manual_scroll = true;
                    }
                }
                (SessionMode::Normal, KeyCode::PageDown) => {
                    if app.rendered_items_count > 0 {
                        let cur = app.list_state.selected().unwrap_or(app.rendered_items_count - 1);
                        let next = (cur + 5).min(app.rendered_items_count.saturating_sub(1));
                        app.list_state.select(Some(next));
                        app.manual_scroll = true;
                    }
                }
                (SessionMode::Normal, KeyCode::Home) => {
                    if app.rendered_items_count > 0 {
                        app.list_state.select(Some(0));
                        app.manual_scroll = true;
                    }
                }
                (SessionMode::Normal, KeyCode::End) => {
                    if app.rendered_items_count > 0 {
                        app.list_state.select(Some(app.rendered_items_count - 1));
                        app.manual_scroll = true;
                    }
                }
                _ => {}
                }
                Event::Mouse(mouse) => {
                    use crossterm::event::MouseEventKind;
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            if app.rendered_items_count > 0 {
                                let cur = app.list_state.selected().unwrap_or(app.rendered_items_count - 1);
                                let next = cur.saturating_sub(3);
                                app.list_state.select(Some(next));
                                app.manual_scroll = true;
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            if app.rendered_items_count > 0 {
                                let cur = app.list_state.selected().unwrap_or(app.rendered_items_count - 1);
                                let next = (cur + 3).min(app.rendered_items_count.saturating_sub(1));
                                app.list_state.select(Some(next));
                                app.manual_scroll = true;
                            }
                        }
                        _ => {}
                    }
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

    // Show session management screen if in session mode
    if app.session_mode == SessionMode::SessionList {
        render_session_management(f, app);
        return;
    }

    // Create three-section layout: status bar, chat history, input
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Status bar (increased for persistence state)
            Constraint::Min(3),    // Chat history - reduced to allow small terminals
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
    // Render weather paragraph
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

    let session_display = if let Some(ref session_id) = app.current_session_id {
        format!("📁 {}", &session_id[..8.min(session_id.len())])
    } else {
        "📁 No session".to_string()
    };

    let (persistence_icon, persistence_text, persistence_color) = match &app.persistence_state {
        PersistenceState::Enabled => ("💾", "Persistent", Color::Green),
        PersistenceState::Disabled => ("🚫", "Non-persistent", Color::Yellow),
        PersistenceState::FailedFallback(_) => ("⚠️", "Fallback mode", Color::Red),
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
            &session_display,
            Style::default().fg(Color::Magenta),
        )]),
        Line::from(vec![
            Span::styled(
                format!("{} ", persistence_icon),
                Style::default().fg(persistence_color),
            ),
            Span::styled(
                persistence_text,
                Style::default().fg(persistence_color),
            ),
        ]),
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
    // Build list items - one per visual line for proper scrolling
    let mut messages: Vec<ListItem> = Vec::new();
    
    for (idx, msg) in app.messages.iter().enumerate() {
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
        let width = compute_wrap_width(main_chunks[1].width, 6, 20);

        // For assistant responses, process content
        let content_to_wrap = match msg.role {
            MessageRole::Assistant => {
                let cleaned = strip_tool_prefixes(&msg.content);
                let processed = pretty_json_if_possible(&cleaned);
                truncate_for_tui(&processed)
            }
            _ => msg.content.clone(),
        };
        
        // Wrap text appropriately
        let wrapped_lines = wrap_with_prefix(&content_to_wrap, width, "│ ");

        let footer = match msg.role {
            MessageRole::User => "╰─────────────────────────",
            MessageRole::Assistant => "└─────────────────────────",
            MessageRole::System => "┗━━━━━━━━━━━━━━━━━━━━━━━━━",
        };

        // Add header line
        messages.push(ListItem::new(header).style(style));
        
        // Add content lines
        for line in wrapped_lines {
            messages.push(ListItem::new(line).style(style));
        }
        
        // Add footer line
        messages.push(ListItem::new(footer).style(style));
        
        // Add blank line between messages (but not after last message)
        if idx < app.messages.len() - 1 {
            messages.push(ListItem::new("".to_string()));
        }
    }

    if app.waiting {
        if !app.thinking_content.is_empty() {
            let width = compute_wrap_width(main_chunks[1].width, 6, 20);

            let thinking_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let frame_idx = app.messages.len() % thinking_frames.len();
            let spinner = thinking_frames[frame_idx];

            let header = format!("┌─ {} Thinking... ─┐", spinner);
            let wrapped_thinking = wrap_with_prefix(&app.thinking_content, width, "│ ");

            // Add thinking lines as individual ListItems
            messages.push(
                ListItem::new(header).style(
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::ITALIC | Modifier::DIM),
                ),
            );
            for line in wrapped_thinking {
                messages.push(
                    ListItem::new(line).style(
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::ITALIC | Modifier::DIM),
                    ),
                );
            }
            messages.push(
                ListItem::new("└─────────────────────────").style(
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::ITALIC | Modifier::DIM),
                ),
            );
        }

        if !app.streaming_content.is_empty() {
            let width = compute_wrap_width(main_chunks[1].width, 6, 20);

            let typing_dots = ["   ", ".  ", ".. ", "..."];
            let dot_idx = (app.streaming_content.len() / 10) % typing_dots.len();
            let dots = typing_dots[dot_idx];

            let header = format!("┌─ 🤖 Rustline {} ─┐", dots);
            let wrapped_streaming = wrap_with_prefix(&format!("{}▊", app.streaming_content), width, "│ ");

            // Add streaming lines as individual ListItems
            messages.push(
                ListItem::new(header).style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            for line in wrapped_streaming {
                messages.push(
                    ListItem::new(line).style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
            }
            messages.push(
                ListItem::new("└─────────────────────────").style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            );
        }
    }

    // Save message count for scrollbar before moving messages
    let messages_count = messages.len();
    // Expose rendered item count for scroll handling in event loop
    app.rendered_items_count = messages_count;
    
    // Check if new chat messages arrived (by comparing actual message count, not rendered items)
    let new_messages_arrived = app.messages.len() > app.last_message_count;

    // Auto-scroll to the bottom unless user is manually scrolling
    // BUT: if new messages arrived, auto-scroll back to bottom
    if !messages.is_empty() {
        if app.list_state.selected().is_none() || !app.manual_scroll || new_messages_arrived {
            app.list_state.select(Some(messages_count - 1));
            app.manual_scroll = false; // Reset when auto-scrolling
        } else {
            let sel = app.list_state.selected().unwrap_or(0);
            let clamped = sel.min(messages_count - 1);
            app.list_state.select(Some(clamped));
        }
    }
    
    // Remember actual message count for next render (not rendered item count)
    app.last_message_count = app.messages.len();

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

    // Render scrollbar
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"));
    let mut scrollbar_state = ScrollbarState::new(messages_count)
        .position(app.list_state.selected().unwrap_or(0));
    f.render_stateful_widget(
        scrollbar,
        main_chunks[1].inner(ratatui::layout::Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );

    let (input_display, input_style, input_border_color, _input_title) = if app.waiting {
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
        "Input (F2: Sessions, Ctrl+C/Esc: Quit)"
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

/// Compute a wrap width given area width, padding, and minimum desired width.
/// Returns usize::MAX to signal "do not wrap" when the area is too narrow.
fn compute_wrap_width(area_width: u16, padding: usize, min_width: usize) -> usize {
    let aw = area_width as usize;
    if aw <= padding { return usize::MAX; }
    let effective = aw.saturating_sub(padding);
    if effective < min_width { usize::MAX } else { effective }
}

/// Wrap text with a prefix; if width is usize::MAX, only split on existing newlines.
fn wrap_with_prefix(s: &str, width: usize, prefix: &str) -> Vec<String> {
    if width == usize::MAX {
        return s.lines().map(|l| format!("{}{}", prefix, l)).collect();
    }
    let safe_width = width.max(10);
    textwrap::wrap(s, safe_width)
        .into_iter()
        .map(|w| format!("{}{}", prefix, w))
        .collect()
}

/// Try to pretty-print JSON if the input is JSON; otherwise return the original string.
fn pretty_json_if_possible(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => {
                // Special-case for create_file exists protection: show a clear message.
                if let Some(obj) = v.as_object() {
                    // If this is a web_summary result, show only the summary content.
                    if let Some(summary) = obj.get("summary").and_then(|m| m.as_str()) {
                        return summary.to_string();
                    }
                    // If this is a web_fetch result, show title + URL (no raw content) and summary when available
                    if let Some(title) = obj.get("title").and_then(|t| t.as_str()) {
                        let url = obj.get("url").and_then(|u| u.as_str()).unwrap_or("");
                        if let Some(summary) = obj.get("summary").and_then(|s| s.as_str()) {
                            return format!("{}\n[{}]\n\nSummary:\n{}", title, url, summary);
                        }
                        return format!("{}\n[{}]", title, url);
                    }
                    // Fallback: just show text if available
                    if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                        if !text.is_empty() {
                            return text.to_string();
                        }
                    }
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

/// Strip TUI-specific prefixes like "Rustline: " and tool markers, leaving only response content.
fn strip_tool_prefixes(s: &str) -> String {
    let trimmed = s.trim_start();
    let without_label = if trimmed.starts_with("Rustline: ") {
        trimmed.trim_start_matches("Rustline: ").to_string()
    } else {
        trimmed.to_string()
    };

    // If the first line is a tool marker like "[tool:web_fetch]", drop it.
    let mut lines = without_label.lines();
    if let Some(first) = lines.next() {
        if first.starts_with("[tool:") || first.starts_with("tool:") {
            return lines.collect::<Vec<_>>().join("\n");
        }
    }
    without_label
}

/// Truncate overly long assistant content to keep it manageable.
/// Note: ratatui's List widget handles scrolling, so we only cap by character count.
fn truncate_for_tui(s: &str) -> String {
    // Cap by characters only; let scrolling handle the rest.
    const MAX_CHARS: usize = 5000;
    if s.is_empty() { return String::new(); }
    if s.len() > MAX_CHARS {
        return format!("{}…", &s[..MAX_CHARS]);
    }
    s.to_string()
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

/// Render the session management screen
fn render_session_management(f: &mut Frame, app: &mut App) {
    use ratatui::{
        layout::{Alignment, Constraint, Direction, Layout},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, List, ListItem, Paragraph},
    };

    let area = f.area();

    // Create layout for session management
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(10),   // Session list
            Constraint::Length(5), // Help text
        ])
        .split(area);

    // Title
    let title = Paragraph::new("Session Management")
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("📁 Sessions")
                .title_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .border_style(Style::default().fg(Color::Cyan))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(title, main_chunks[0]);

    // Session list
    let sessions: Vec<ListItem> = app
        .available_sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let is_current = app.current_session_id.as_ref() == Some(&session.id);
            let is_selected = i == app.selected_session_index;
            
            let current_marker = if is_current { " (current)" } else { "" };
            let session_text = format!(
                "{} - {} msgs, created: {}{}",
                &session.id[..8.min(session.id.len())],
                session.message_count,
                session.created_at.format("%Y-%m-%d %H:%M"),
                current_marker
            );

            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(session_text).style(style)
        })
        .collect();

    let sessions_list = List::new(sessions).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Available Sessions")
            .title_style(
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(Color::Green))
            .style(Style::default().bg(Color::Black)),
    );

    f.render_widget(sessions_list, main_chunks[1]);

    // Help text
    let help_lines = vec![
        Line::from(vec![
            Span::styled("↑/↓", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(": Navigate  ", Style::default().fg(Color::Gray)),
            Span::styled("Enter", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(": Switch  ", Style::default().fg(Color::Gray)),
            Span::styled("N", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(": New Session", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("D", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(": Delete  ", Style::default().fg(Color::Gray)),
            Span::styled("Esc", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(": Back to Chat", Style::default().fg(Color::Gray)),
        ]),
    ];

    let help_para = Paragraph::new(help_lines)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Controls")
                .title_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
                .border_style(Style::default().fg(Color::Yellow))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(help_para, main_chunks[2]);
}

