use reqwest::Client;
use tokio::time::{sleep, Duration};
use std::time::Instant;

use crate::config::Config;
use crate::ollama;
use crate::ollama::Message;
use crate::tools::{self, DynTool};
use crate::persistence::{
    session_manager::SessionManager,
    preference_manager::PreferenceManager,
    PersistenceError,
};
use serde_json::Value;

const REACT_PROMPT_TEMPLATE: &str = r#"
You are Rustline, a helpful assistant that solves problems by reasoning step-by-step
and using tools when needed.

You have access to the following tools:

{tool_descriptions}

Each tool is described as: name | description

When asked to locate or find files, you MUST call the `locate` tool first. Only after `locate` returns candidate paths should you call `read_file` on a specific path. Do not invent file locations or contents.

Search roots available on this system (the agent may try these when locating files):
{search_paths}

You MUST use the following format to reason and act:

Question: the input question you must answer
Thought: you should always think about what to do
Action: the action to take, must be one of [{tool_names}] or 'finish'
Action Input: the input to the action (as plain text arguments)
Observation: the result of the action
... (this Thought/Action/Action Input/Observation can repeat zero or more times)
Thought: I now know the final answer
Final Answer: the final answer to the original input question

IMPORTANT:
- If you need to use a tool, set Action to a tool name (e.g. time, echo) and provide Action Input.
- If you are ready to answer the question, set Action to 'finish' and write the final answer in 'Final Answer'.
- Do NOT invent tools not in the list.
- Always strictly follow the format above.

Previous steps (if any):
{scratchpad}

Now begin!

Question: {question}
"#;

/// One ReAct TAO step stored by the executor.
struct AgentStep {
    pub action: String,
    pub action_input: String,
    pub observation: String,
}

/// What the model decided for this iteration.
struct PlannedAction {
    pub thought: Option<String>,
    pub tool: String,
    pub input: String,
}

enum PlanOutput {
    Action(PlannedAction),
    FinalAnswer {
        thought: Option<String>,
        answer: String,
    },
}

/// Core “brain” of Rustline.
/// Keeps config, ReAct tools, and some lightweight history.
/// Enhanced with persistence capabilities for session and preference management.
pub struct Agent {
    http: Client,
    pub history: Vec<Message>,
    pub config: Config,
    tools: Vec<DynTool>,
    pending_input: Option<String>,
    pending_action: Option<(String, String)>, // (tool, input)
    last_tool_invoked: Option<(String, String)>, // (tool, input)
    // Persistence managers
    pub session_manager: Option<SessionManager>,
    pub preference_manager: Option<PreferenceManager>,
}

impl Clone for Agent {
    fn clone(&self) -> Self {
        Agent {
            http: Client::new(),
            history: self.history.clone(),
            config: self.config.clone(),
            tools: tools::default_tools(), // Recreate tools instead of cloning
            pending_input: self.pending_input.clone(),
            pending_action: self.pending_action.clone(),
            last_tool_invoked: self.last_tool_invoked.clone(),
            // Note: Persistence managers are not cloned to avoid shared state issues
            session_manager: None,
            preference_manager: None,
        }
    }
}

impl Agent {
    /// Create a new agent with given config (without persistence).
    pub fn new(config: Config) -> Self {
        Agent {
            http: Client::new(),
            history: Vec::new(),
            config,
            tools: tools::default_tools(),
            pending_input: None,
            pending_action: None,
            last_tool_invoked: None,
            session_manager: None,
            preference_manager: None,
        }
    }

    /// Create a new agent with persistence capabilities.
    pub fn new_with_persistence(
        config: Config, 
        session_manager: SessionManager, 
        preference_manager: PreferenceManager
    ) -> Self {
        Agent {
            http: Client::new(),
            history: Vec::new(),
            config,
            tools: tools::default_tools(),
            pending_input: None,
            pending_action: None,
            last_tool_invoked: None,
            session_manager: Some(session_manager),
            preference_manager: Some(preference_manager),
        }
    }

    /// Clear conversation state (local history and persistent storage if available).
    pub fn reset(&mut self) {
        self.history.clear();
        
        // If we have a session manager, clear the current session's persistent storage
        if let Some(session_manager) = &mut self.session_manager {
            if let Some(current_session) = session_manager.get_current_session() {
                let session_id = current_session.to_string();
                // Delete and recreate the session to clear all data
                if let Err(e) = session_manager.delete_session(&session_id) {
                    log::warn!("Failed to clear session during reset: {}", e);
                } else {
                    // Create a new session with the same name if possible
                    if let Ok(new_session_id) = session_manager.create_session(Some("Default Session".to_string())) {
                        if let Err(e) = session_manager.switch_session(&new_session_id) {
                            log::warn!("Failed to switch to new session after reset: {}", e);
                        }
                    }
                }
            }
        }
    }

    /// Change the model name at runtime.
    pub fn set_model(&mut self, model: String) {
        self.config.model = model;
    }

    /// Load conversation history from a specific session
    pub fn load_session(&mut self, session_id: Option<&str>) -> Result<(), PersistenceError> {
        if let Some(session_manager) = &mut self.session_manager {
            match session_id {
                Some(id) => {
                    // Switch to the specified session
                    session_manager.switch_session(id)?;
                    // Load the session history
                    self.history = session_manager.load_current_session_history()?;
                }
                None => {
                    // Load the current session or create a default one
                    let _ = session_manager.get_or_create_current_session()?;
                    self.history = session_manager.load_current_session_history()?;
                }
            }
        }
        Ok(())
    }

    /// Save current conversation state to persistent storage
    pub fn save_current_state(&self) -> Result<(), PersistenceError> {
        // State is automatically saved as messages are added, so this is mostly a no-op
        // but could be used for explicit save operations in the future
        Ok(())
    }

    /// Create a new session and optionally switch to it
    pub fn create_new_session(&mut self, name: Option<String>) -> Result<String, PersistenceError> {
        if let Some(session_manager) = &mut self.session_manager {
            let session_id = session_manager.create_session(name)?;
            // Switch to the new session
            session_manager.switch_session(&session_id)?;
            // Clear current history since we're starting fresh
            self.history.clear();
            Ok(session_id)
        } else {
            Err(PersistenceError::InvalidSessionId {
                session_id: "No session manager available".to_string(),
            })
        }
    }

    /// Switch to an existing session
    pub fn switch_session(&mut self, session_id: &str) -> Result<(), PersistenceError> {
        if let Some(session_manager) = &mut self.session_manager {
            session_manager.switch_session(session_id)?;
            // Load the session history
            self.history = session_manager.load_session_history(session_id)?;
            Ok(())
        } else {
            Err(PersistenceError::InvalidSessionId {
                session_id: "No session manager available".to_string(),
            })
        }
    }

    /// Get the current session ID
    pub fn get_current_session_id(&self) -> Option<String> {
        self.session_manager
            .as_ref()
            .and_then(|sm| sm.get_current_session().map(|s| s.to_string()))
    }

    /// Get a reference to the conversation history
    pub fn get_history(&self) -> &[Message] {
        &self.history
    }

    /// List all available sessions
    pub fn list_sessions(&mut self) -> Result<Vec<crate::persistence::memory_store::SessionInfo>, PersistenceError> {
        if let Some(session_manager) = &mut self.session_manager {
            session_manager.list_sessions()
        } else {
            Ok(Vec::new())
        }
    }

    /// Delete a session
    pub fn delete_session(&mut self, session_id: &str) -> Result<(), PersistenceError> {
        if let Some(session_manager) = &mut self.session_manager {
            session_manager.delete_session(session_id)?;
            // If we deleted the current session, clear history
            if self.get_current_session_id().as_deref() == Some(session_id) {
                self.history.clear();
            }
            Ok(())
        } else {
            Err(PersistenceError::InvalidSessionId {
                session_id: "No session manager available".to_string(),
            })
        }
    }

    /// Export all user data to a file
    pub fn export_data(&mut self, export_path: &std::path::Path) -> Result<(), PersistenceError> {
        use crate::persistence::{ExportData, SessionExport};
        use std::fs::File;
        use std::io::Write;
        
        // Ensure we have persistence managers
        let session_manager = self.session_manager.as_mut().ok_or_else(|| {
            PersistenceError::InvalidSessionId {
                session_id: "No session manager available for export".to_string(),
            }
        })?;
        
        let preference_manager = self.preference_manager.as_ref().ok_or_else(|| {
            PersistenceError::InvalidSessionId {
                session_id: "No preference manager available for export".to_string(),
            }
        })?;
        
        // Get all sessions
        let sessions = session_manager.list_sessions()?;
        let mut session_exports = Vec::new();
        
        // Export each session with its messages
        for session_info in sessions {
            let messages = session_manager.load_session_history(&session_info.id)?;
            session_exports.push(SessionExport {
                metadata: session_info,
                messages,
            });
        }
        
        // Get user preferences
        let preferences = preference_manager.get_preferences().clone();
        
        // Create export data structure
        let export_data = ExportData {
            version: "1.0.0".to_string(),
            exported_at: chrono::Utc::now(),
            sessions: session_exports,
            preferences,
        };
        
        // Serialize to JSON
        let json_data = serde_json::to_string_pretty(&export_data)
            .map_err(|e| PersistenceError::Serialization(e))?;
        
        // Write to file atomically
        let temp_path = export_path.with_extension("tmp");
        {
            let mut file = File::create(&temp_path)
                .map_err(|e| PersistenceError::Io(e))?;
            file.write_all(json_data.as_bytes())
                .map_err(|e| PersistenceError::Io(e))?;
            file.sync_all()
                .map_err(|e| PersistenceError::Io(e))?;
        }
        
        // Atomically rename to final location
        std::fs::rename(&temp_path, export_path)
            .map_err(|e| PersistenceError::Io(e))?;
        
        log::info!("Successfully exported data to: {}", export_path.display());
        Ok(())
    }

    /// Import user data from a file with validation and conflict resolution
    pub fn import_data(&mut self, import_path: &std::path::Path) -> Result<(), PersistenceError> {
        use crate::persistence::ExportData;
        use std::fs::File;
        use std::io::Read;
        use std::collections::HashSet;
        
        // Ensure we have persistence managers
        let session_manager = self.session_manager.as_mut().ok_or_else(|| {
            PersistenceError::InvalidSessionId {
                session_id: "No session manager available for import".to_string(),
            }
        })?;
        
        let preference_manager = self.preference_manager.as_mut().ok_or_else(|| {
            PersistenceError::InvalidSessionId {
                session_id: "No preference manager available for import".to_string(),
            }
        })?;
        
        // Read and parse the import file
        let mut file = File::open(import_path)
            .map_err(|e| PersistenceError::Io(e))?;
        
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| PersistenceError::Io(e))?;
        
        let import_data: ExportData = serde_json::from_str(&contents)
            .map_err(|e| PersistenceError::ImportValidation {
                reason: format!("Invalid JSON format: {}", e),
            })?;
        
        // Validate import data version
        if import_data.version != "1.0.0" {
            return Err(PersistenceError::ImportValidation {
                reason: format!("Unsupported export version: {}", import_data.version),
            });
        }
        
        // Validate import data structure
        if import_data.sessions.is_empty() {
            log::warn!("Import file contains no sessions");
        }
        
        // Validate each session before importing
        for session_export in &import_data.sessions {
            if session_export.metadata.id.is_empty() {
                return Err(PersistenceError::ImportValidation {
                    reason: "Session with empty ID found in import data".to_string(),
                });
            }
            
            // Validate message structure
            for (i, message) in session_export.messages.iter().enumerate() {
                if message.role.is_empty() {
                    return Err(PersistenceError::ImportValidation {
                        reason: format!("Message {} in session {} has empty role", i, session_export.metadata.id),
                    });
                }
                if message.message_id.is_empty() {
                    return Err(PersistenceError::ImportValidation {
                        reason: format!("Message {} in session {} has empty message_id", i, session_export.metadata.id),
                    });
                }
            }
        }
        
        // Get existing session IDs to detect conflicts
        let existing_sessions = session_manager.list_sessions()?;
        let existing_session_ids: HashSet<String> = existing_sessions
            .into_iter()
            .map(|s| s.id)
            .collect();
        
        // Track imported sessions for rollback capability
        let mut imported_session_ids = Vec::new();
        
        // Import sessions with conflict resolution and rollback on error
        let import_result = (|| -> Result<(), PersistenceError> {
            for session_export in import_data.sessions {
                let mut session_id = session_export.metadata.id.clone();
                
                // Resolve session ID conflicts by generating new unique IDs
                if existing_session_ids.contains(&session_id) {
                    // Generate a new unique session ID
                    let original_id = session_id.clone();
                    session_id = format!("imported_{}_{}", 
                        chrono::Utc::now().timestamp(), 
                        uuid::Uuid::new_v4().simple()
                    );
                    
                    log::info!("Resolved session ID conflict: {} -> {}", original_id, session_id);
                }
                
                // Create the session with the resolved ID
                session_manager.create_session_with_id(&session_id, session_export.metadata.name.clone())?;
                imported_session_ids.push(session_id.clone());
                
                // Import all messages for this session
                for message in &session_export.messages {
                    session_manager.save_message(&session_id, message)?;
                }
                
                log::info!("Imported session: {} ({} messages)", 
                    session_id, session_export.messages.len());
            }
            Ok(())
        })();
        
        // If import failed, rollback by deleting imported sessions
        if let Err(e) = import_result {
            log::error!("Import failed, rolling back imported sessions: {}", e);
            for session_id in imported_session_ids {
                if let Err(rollback_err) = session_manager.delete_session(&session_id) {
                    log::error!("Failed to rollback session {}: {}", session_id, rollback_err);
                }
            }
            return Err(e);
        }
        
        // Import preferences (merge with existing preferences)
        // Store original preferences for rollback
        let original_preferences = preference_manager.get_preferences().clone();
        
        let preference_result = (|| -> Result<(), PersistenceError> {
            preference_manager.update_model_preference(import_data.preferences.default_model.clone())?;
            preference_manager.update_confirmation_preference(import_data.preferences.confirm_before_tools)?;
            preference_manager.save_preferences()?;
            Ok(())
        })();
        
        // If preference import failed, restore original preferences
        if let Err(e) = preference_result {
            log::error!("Preference import failed, restoring original preferences: {}", e);
            if let Err(restore_err) = preference_manager.update_model_preference(original_preferences.default_model) {
                log::error!("Failed to restore model preference: {}", restore_err);
            }
            if let Err(restore_err) = preference_manager.update_confirmation_preference(original_preferences.confirm_before_tools) {
                log::error!("Failed to restore confirmation preference: {}", restore_err);
            }
            // Don't fail the entire import if only preferences failed
            log::warn!("Preferences import failed but sessions were imported successfully");
        }
        
        log::info!("Successfully imported data from: {} ({} sessions)", 
            import_path.display(), imported_session_ids.len());
        Ok(())
    }

    /// Manual tool invocation via `!` commands in the REPL.
    /// Returns Ok(Some(reply)) if handled as a tool command,
    /// Ok(None) if not a tool command.
    fn try_run_tool(&mut self, input: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if !input.starts_with('!') {
            return Ok(None);
        }

        let rest = &input[1..];
        let mut parts = rest.splitn(2, ' ');
        let name_part = parts.next().unwrap_or("").to_lowercase();
        let args = parts.next().unwrap_or("").trim();

        // special: change precheck mode (not a tool, but a control command)
        if name_part == "mode" {
            let mode = args.to_lowercase();
            if mode == "strict" || mode == "assisted" {
                self.config.precheck_mode = mode.clone();
                return Ok(Some(format!("Precheck mode set to '{}'.", mode)));
            } else {
                return Ok(Some("Usage: !mode <strict|assisted>".to_string()));
            }
        }

        if name_part == "confirm" {
            let val = args.to_lowercase();
            match val.as_str() {
                "on" => { self.config.confirm_before_tools = true; return Ok(Some("Confirm-before-tools: ON".to_string())); }
                "off" => { self.config.confirm_before_tools = false; return Ok(Some("Confirm-before-tools: OFF".to_string())); }
                _ => return Ok(Some("Usage: !confirm <on|off>".to_string())),
            }
        }

        if name_part == "do" {
            if let Some((tool, inp)) = self.pending_action.take() {
                // Execute the previously planned action
                if let Some(tool_impl) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case(&tool)) {
                    match tool_impl.invoke(&inp) {
                        Ok(res) => return Ok(Some(res)),
                        Err(e) => return Ok(Some(format!("Tool `{}` error: {}", tool, e))),
                    }
                } else {
                    return Ok(Some(format!("Unknown tool `{}`.", tool)));
                }
            }

            if let Some(pending) = self.pending_input.take() {
                // Temporarily disable confirmation to execute the pending precheck actions
                let was_confirm = self.config.confirm_before_tools;
                self.config.confirm_before_tools = false;
                let res = match self.strict_precheck_response(&pending)? {
                    Some(r) => r,
                    None => "Nothing to run.".to_string(),
                };
                self.config.confirm_before_tools = was_confirm;
                return Ok(Some(res));
            } else {
                return Ok(Some("No pending action to run.".to_string()));
            }
        }

        if name_part == "skip" {
            self.pending_input = None;
            self.pending_action = None;
            return Ok(Some("Pending action cleared.".to_string()));
        }

        if name_part == "status" {
            let mut s = String::new();
            if let Some((t, i)) = &self.pending_action { s.push_str(&format!("Pending action: {} | {}\n", t, i)); } else { s.push_str("Pending action: none\n"); }
            if let Some(p) = &self.pending_input { s.push_str(&format!("Pending input: {}\n", p)); } else { s.push_str("Pending input: none\n"); }
            if let Some((t, i)) = &self.last_tool_invoked { s.push_str(&format!("Last tool invoked: {} | {}\n", t, i)); } else { s.push_str("Last tool invoked: none\n"); }
            return Ok(Some(s));
        }

        // special: list tools
        if name_part == "tools" || name_part == "help" {
            let mut out = String::from("Available tools:\n");
            for t in &self.tools {
                out.push_str(&format!("  !{} - {}\n", t.name(), t.description()));
            }
            return Ok(Some(out));
        }

        if name_part.is_empty() {
            return Ok(Some(
                "Usage: !<tool> [args]. Try !tools to list tools.".to_string(),
            ));
        }

        if let Some(tool) = self
            .tools
            .iter()
            .find(|t| t.name().eq_ignore_ascii_case(&name_part))
        {
            let result = tool.invoke(args)?;
            Ok(Some(format!("[tool:{}]\n{}", name_part, result)))
        } else {
            Ok(Some(format!(
                "Unknown tool: {name}\nUse !tools to list available tools.",
                name = name_part
            )))
        }
    }

    /// Plan a single ReAct step given the current steps (TAO history).
    async fn plan_once(
        &self,
        question: &str,
        steps: &[AgentStep],
    ) -> Result<PlanOutput, Box<dyn std::error::Error>> {
        let mut scratchpad = String::new();
        for step in steps {
            scratchpad.push_str(&format!(
                "Thought: I should use a tool.\nAction: {action}\nAction Input: {input}\nObservation: {obs}\n",
                action = step.action,
                input = step.action_input,
                obs = step.observation,
            ));
        }

        let (tool_descs, tool_names) = build_tool_descriptions(&self.tools);

        // compute search roots to show the model (cwd, project root, src, HOME, etc)
        let mut search_roots: Vec<String> = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            search_roots.push(cwd.to_string_lossy().to_string());
            search_roots.push(cwd.join("src").to_string_lossy().to_string());
            if let Some(root) = crate::tools::find_project_root(&cwd) {
                search_roots.push(root.to_string_lossy().to_string());
                search_roots.push(root.join("src").to_string_lossy().to_string());
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            search_roots.push(home);
        }

        let search_paths = format!("[{}]", search_roots.iter().map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(", "));

        let prompt = REACT_PROMPT_TEMPLATE
            .replace("{tool_descriptions}", &tool_descs)
            .replace("{tool_names}", &tool_names)
            .replace("{search_paths}", &search_paths)
            .replace("{scratchpad}", &scratchpad)
            .replace("{question}", question);

        let reply = ollama::chat_single_turn(
            &self.http,
            &self.config.ollama_base_url,
            &self.config.model,
            &prompt,
        )
        .await?;

        Ok(parse_react_reply(&reply))
    }

    /// Handle a single user message using a ReAct loop,
    /// printing Thought / Action / Observation to the CLI.
    pub async fn handle_message(
        &mut self,
        input: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if input.is_empty() {
            return Ok("You didn't type anything 🤔".to_string());
        }

        // If there's a pending destructive action, interpret natural-language confirmation.
        if self.pending_action.is_some() {
            match interpret_confirmation(input) {
                Some(true) => { return Ok(self.run_pending_action().unwrap_or_else(|e| e)); }
                Some(false) => { self.pending_action = None; return Ok("Pending action cleared.".to_string()); }
                None => { /* carry on */ }
            }
        }

        // Manual `!` tools (bypass LLM & ReAct).
        if let Some(tool_reply) = self.try_run_tool(input)? {
            println!("[ReAct] User invoked manual tool command.");
            self.log_history("user", input.to_string());
            self.log_history("assistant", tool_reply.clone());
            return Ok(tool_reply);
        }

        // Enhanced STRICT auto-precheck:
        // - If the user used file-related verbs (locate/find/where/read/open) or the input
        //   contains a filename-like token, we will ALWAYS call `locate` first.
        // - If `locate` returns matches:
        //     * For locate/where/find queries: return the locate JSON array immediately.
        //     * For read/open queries: call `read_file` on the first matched path and return its result.
        // - If `locate` returns no matches, return a clear "No file found" message and DO NOT call the LLM.
        if let Some(precheck) = self.strict_precheck_response(input)? {
            return Ok(precheck);
        }

        let question = input.to_string();
        let mut steps: Vec<AgentStep> = Vec::new();
        let max_iterations = self.config.react_max_iterations;
        let mut total_thinking: std::time::Duration = std::time::Duration::from_secs(0);

        println!("\n[ReAct] Starting reasoning loop for question: {question}");

        for iter in 0..max_iterations {
            println!("[ReAct] Iteration {}", iter + 1);
            // time the planning call and print an elapsed message every 10s while waiting
            let start = Instant::now();

            // Spawn a background task that prints elapsed time every 10 seconds.
            // It will be aborted as soon as planning completes or errors.
            let progress_handle = tokio::task::spawn({
                async move {
                    // initial delay is 10s so we don't spam short operations
                    loop {
                        sleep(Duration::from_secs(10)).await;
                        let e = start.elapsed();
                        println!("[Timing][elapsed] {:.3} seconds", e.as_secs_f64());
                    }
                }
            });

            // Await the planning call. If it errors, abort the progress task and return the error.
            let plan = match self.plan_once(&question, &steps).await {
                Ok(p) => p,
                Err(e) => {
                    progress_handle.abort();
                    return Err(e);
                }
            };

            // Stop the progress printer and record elapsed time.
            progress_handle.abort();
            let elapsed = start.elapsed();
            total_thinking += elapsed;
            println!("[Timing] Planning took {:.3} seconds", elapsed.as_secs_f64());

            match plan {
                PlanOutput::FinalAnswer { thought, answer } => {
                    if let Some(t) = thought {
                        println!("[Thought] {}", t);
                    }
                    println!("[Final Answer] {}", answer);

                    // Log early termination event
                    println!("[ReAct] Early termination: Final answer received after {} iterations (max: {})", 
                        iter + 1, max_iterations);
                    log::info!("ReAct loop terminated early after {} iterations due to final answer", iter + 1);

                    // If confirmation mode is ON and the input implies a toolable intent (e.g., create),
                    // but the LLM did not actually call a tool, offer to run the appropriate tool.
                    if self.config.confirm_before_tools {
                        let lowq = question.to_lowercase();
                        let is_create_verb = lowq.contains("create file") || lowq.starts_with("create ") || lowq.contains("create a file");
                        let is_negated_create = contains_negation_for_create(&lowq);
                        if is_create_verb && !is_negated_create && self.pending_action.is_none() {
                            if let Some(filename) = build_create_filename(&question) {
                                let msg = self.preview_destructive(&question, "create_file", &filename);
                                return Ok(msg);
                            }
                        }
                    }

                    // store as simple Q/A history
                    self.log_history("user", question.clone());
                    self.log_history("assistant", answer.clone());

                    return Ok(answer);
                }
                PlanOutput::Action(planned) => {
                    if let Some(t) = planned.thought {
                        println!("[Thought] {}", t);
                    }

                    let tool_name = planned.tool.trim().to_lowercase();

                    // If confirmation mode is enabled, preview and defer execution for destructive tools.
                    if self.config.confirm_before_tools && (tool_name == "create_file" || tool_name == "open_file" || tool_name == "delete_file") {
                        let msg = self.preview_destructive(&question, &tool_name, &planned.input);
                        return Ok(msg);
                    }

                    println!(
                        "[Action] Using tool '{}' with input: {}",
                        tool_name, planned.input
                    );

                    let maybe_tool = self
                        .tools
                        .iter()
                        .find(|t| t.name().eq_ignore_ascii_case(&tool_name));

                    let observation = if let Some(tool_impl) = maybe_tool {
                        match tool_impl.invoke(&planned.input) {
                            Ok(res) => { 
                                self.last_tool_invoked = Some((tool_name.clone(), planned.input.clone()));
                                // Persist tool invocation as part of conversation history
                                self.persist_message_with_tool("assistant", &format!("Used tool: {}", tool_name), &tool_name, &planned.input, &res, true);
                                res 
                            },
                            Err(e) => {
                                let error_msg = format!("Tool `{}` error: {}", tool_name, e);
                                // Persist failed tool invocation
                                self.persist_message_with_tool("assistant", &format!("Tool failed: {}", tool_name), &tool_name, &planned.input, &error_msg, false);
                                error_msg
                            },
                        }
                    } else {
                        format!(
                            "Unknown tool `{}`. Available tools: {}",
                            tool_name,
                            self.tools
                                .iter()
                                .map(|t| t.name())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };

                    // Pretty-print observations for known tools where output is JSON.
                    let short_obs = if tool_name == "locate" {
                        match serde_json::from_str::<Value>(&observation) {
                            Ok(Value::Array(arr)) => format_locate_results(&arr),
                            _ => observation.clone(),
                        }
                    } else if tool_name == "read_file" {
                        match serde_json::from_str::<Value>(&observation) {
                            Ok(Value::Object(map)) => format_read_output(&map),
                            _ => observation.clone(),
                        }
                    } else if tool_name == "web_fetch" {
                        match serde_json::from_str::<Value>(&observation) {
                            Ok(Value::Object(map)) => format_web_fetch_output(&map),
                            _ => observation.clone(),
                        }
                    } else if tool_name == "web_summary" {
                        match serde_json::from_str::<Value>(&observation) {
                            Ok(Value::Object(map)) => {
                                if let Some(summary) = map.get("summary").and_then(|s| s.as_str()) {
                                    summary.to_string()
                                } else {
                                    observation.clone()
                                }
                            }
                            _ => observation.clone(),
                        }
                    } else if observation.len() > 200 {
                        format!("{}...", &observation[..200])
                    } else {
                        observation.clone()
                    };
                    println!("[Observation] {}", short_obs);

                    steps.push(AgentStep {
                        action: tool_name,
                        action_input: planned.input,
                        observation,
                    });
                }
            }
        }

        // Provide a more meaningful response when iteration limit is reached
        let response = format!(
            "I've reached the maximum number of reasoning steps ({}) while working on your request. \
            Based on my analysis so far, I may need more specific information or a different approach to complete this task. \
            Please try rephrasing your question or breaking it into smaller parts.",
            max_iterations
        );
        println!("[ReAct] Reached iteration limit of {} steps", max_iterations);
        Ok(response)
    }

    /// Handle a single user message with streaming support.
    /// The callback receives each token as it's generated.
    pub async fn handle_message_stream<F, G>(
        &mut self,
        input: &str,
        mut on_chunk: F,
        mut on_think: G,
    ) -> Result<String, Box<dyn std::error::Error>>
    where
        F: FnMut(&str),
        G: FnMut(&str),
    {
        if input.is_empty() {
            return Ok("You didn't type anything 🤔".to_string());
        }

        // Natural-language confirmation for pending actions (streaming path).
        if self.pending_action.is_some() {
            match interpret_confirmation(input) {
                Some(true) => {
                    let res = self.run_pending_action().unwrap_or_else(|e| e);
                    on_chunk(&res);
                    return Ok(res);
                }
                Some(false) => {
                    self.pending_action = None;
                    let msg = "Pending action cleared.".to_string();
                    on_chunk(&msg);
                    return Ok(msg);
                }
                None => { /* continue normal flow */ }
            }
        }

        // Manual `!` tools (bypass LLM & ReAct).
        if let Some(tool_reply) = self.try_run_tool(input)? {
            on_chunk(&tool_reply);
            self.log_history("user", input.to_string());
            self.log_history("assistant", tool_reply.clone());
            return Ok(tool_reply);
        }

        if let Some(precheck) = self.strict_precheck_response(input)? {
            on_chunk(&precheck);
            return Ok(precheck);
        }

        let question = input.to_string();
        let mut steps: Vec<AgentStep> = Vec::new();
        let max_iterations = self.config.react_max_iterations;

        for iter in 0..max_iterations {
            let plan = self.plan_once(&question, &steps).await?;

            match plan {
                PlanOutput::FinalAnswer { thought, answer } => {
                    if let Some(t) = thought {
                        on_think(&format!("Thought: {}", t));
                    }
                    
                    // Log early termination event
                    on_think(&format!("Early termination: Final answer received after {} iterations (max: {})", 
                        iter + 1, max_iterations));
                    log::info!("ReAct loop terminated early after {} iterations due to final answer", iter + 1);
                    
                    // If confirmation mode is ON and input implies a toolable intent (e.g., create) but no tool was planned,
                    // offer to run the appropriate tool instead of finalizing immediately.
                    if self.config.confirm_before_tools {
                        let lowq = question.to_lowercase();
                        let is_create_verb = lowq.contains("create file") || lowq.starts_with("create ") || lowq.contains("create a file");
                        let is_negated_create = contains_negation_for_create(&lowq);
                        if is_create_verb && !is_negated_create && self.pending_action.is_none() {
                            if let Some(filename) = build_create_filename(&question) {
                                let msg = self.preview_destructive(&question, "create_file", &filename);
                                on_chunk(&msg);
                                return Ok(msg);
                            }
                        }
                    }

                    // Stream the final answer
                    for chunk in answer.chars().collect::<Vec<_>>().chunks(5) {
                        let chunk_str: String = chunk.iter().collect();
                        on_chunk(&chunk_str);
                        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
                    }

                    // store as simple Q/A history
                    self.log_history("user", question.clone());
                    self.log_history("assistant", answer.clone());

                    return Ok(answer);
                }
                PlanOutput::Action(planned) => {
                    if let Some(t) = planned.thought {
                        on_think(&format!("Thought: {}", t));
                    }

                    let tool_name = planned.tool.trim().to_lowercase();

                    // If confirmation mode is enabled, preview and defer execution for destructive tools.
                    if self.config.confirm_before_tools && (tool_name == "create_file" || tool_name == "open_file" || tool_name == "delete_file") {
                        let msg = self.preview_destructive(&question, &tool_name, &planned.input);
                        on_chunk(&msg);
                        return Ok(msg);
                    }

                    on_think(&format!("Action: Using tool '{}' with input: {}", tool_name, planned.input));

                    let maybe_tool = self
                        .tools
                        .iter()
                        .find(|t| t.name().eq_ignore_ascii_case(&tool_name));

                    let observation = if let Some(tool_impl) = maybe_tool {
                        match tool_impl.invoke(&planned.input) {
                            Ok(res) => { 
                                self.last_tool_invoked = Some((tool_name.clone(), planned.input.clone()));
                                // Persist tool invocation as part of conversation history
                                self.persist_message_with_tool("assistant", &format!("Used tool: {}", tool_name), &tool_name, &planned.input, &res, true);
                                res 
                            },
                            Err(e) => {
                                let error_msg = format!("Tool `{}` error: {}", tool_name, e);
                                // Persist failed tool invocation
                                self.persist_message_with_tool("assistant", &format!("Tool failed: {}", tool_name), &tool_name, &planned.input, &error_msg, false);
                                error_msg
                            },
                        }
                    } else {
                        format!(
                            "Unknown tool `{}`. Available tools: {}",
                            tool_name,
                            self.tools
                                .iter()
                                .map(|t| t.name())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };

                    let short_obs = if tool_name == "locate" {
                        match serde_json::from_str::<Value>(&observation) {
                            Ok(Value::Array(arr)) => format_locate_results(&arr),
                            _ => observation.clone(),
                        }
                    } else if tool_name == "read_file" {
                        match serde_json::from_str::<Value>(&observation) {
                            Ok(Value::Object(map)) => format_read_output(&map),
                            _ => observation.clone(),
                        }
                    } else if observation.len() > 200 {
                        format!("{}...", &observation[..200])
                    } else {
                        observation.clone()
                    };
                    on_think(&format!("Observation: {}", short_obs));

                    steps.push(AgentStep {
                        action: tool_name,
                        action_input: planned.input,
                        observation,
                    });
                }
            }
        }

        let msg = format!(
            "I've reached the maximum number of reasoning steps ({}) while working on your request. \
            Based on my analysis so far, I may need more specific information or a different approach to complete this task. \
            Please try rephrasing your question or breaking it into smaller parts.",
            max_iterations
        );
        on_think(&msg);
        Ok(msg)
    }
}

// ===== helper functions for ReAct =====

fn build_tool_descriptions(tools: &[DynTool]) -> (String, String) {
    let mut descs = String::new();
    let mut names = Vec::new();

    for t in tools {
        names.push(t.name().to_string());
        descs.push_str(&format!("{} | {}\n", t.name(), t.description()));
    }

    (descs, names.join(", "))
}

fn parse_react_reply(reply: &str) -> PlanOutput {
    let mut last_thought: Option<String> = None;
    let mut last_action: Option<(String, String)> = None;
    let mut final_answer: Option<String> = None;

    let lines: Vec<&str> = reply.lines().map(|l| l.trim()).collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        if let Some(stripped) = line.strip_prefix("Thought:") {
            let thought = stripped.trim().to_string();
            if !thought.is_empty() {
                last_thought = Some(thought);
            }
        } else if let Some(stripped) = line.strip_prefix("Action:") {
            let action_name = stripped.trim().to_string();

            let mut action_input = String::new();
            if i + 1 < lines.len() {
                if let Some(next) = lines[i + 1].strip_prefix("Action Input:") {
                    action_input = next.trim().to_string();
                    i += 1;
                }
            }

            last_action = Some((action_name, action_input));
        } else if let Some(stripped) = line.strip_prefix("Final Answer:") {
            let ans = stripped.trim().to_string();
            if !ans.is_empty() {
                final_answer = Some(ans);
            }
        }

        i += 1;
    }

    if let Some(ans) = final_answer {
        return PlanOutput::FinalAnswer {
            thought: last_thought,
            answer: ans,
        };
    }

    if let Some((action_name, action_input)) = &last_action {
        if action_name.eq_ignore_ascii_case("finish") {
            return PlanOutput::FinalAnswer {
                thought: last_thought,
                answer: action_input.clone(),
            };
        }
    }

    if let Some((action_name, action_input)) = last_action {
        return PlanOutput::Action(PlannedAction {
            thought: last_thought,
            tool: action_name,
            input: action_input,
        });
    }

    PlanOutput::FinalAnswer {
        thought: last_thought,
        answer: reply.trim().to_string(),
    }
}

// Helper: check if any whole word from the list appears in text.
// Whole word means surrounded by whitespace or start/end of string.
fn has_word_in(text: &str, words: &[&str]) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    for word in words {
        if tokens.contains(word) {
            return true;
        }
    }
    false
}

// Heuristic to pick a filename-like token from user input.
fn extract_file_candidate(input: &str) -> Option<String> {
    let low = input.to_lowercase();

    // If the user used an explicit file-related verb, prefer tokens containing a dot.
    if low.contains("locate") || low.contains("find") || low.contains("where is") || low.contains("read file") || low.contains("open file") {
        for tok in input.split_whitespace() {
            let t = tok.trim_matches(&[',', '.', '!', '?', '"', '\''][..]);
            if t.contains('.') {
                return Some(t.to_string());
            }
        }
    }

    // Otherwise accept any token that looks like a filename (has an extension)
    for tok in input.split_whitespace() {
        let t = tok.trim_matches(&[',', '.', '!', '?', '"', '\''][..]);
        if t.contains('.') {
            return Some(t.to_string());
        }
    }

    None
}

// Detect common negation phrases indicating the user does NOT want to create a file.
fn contains_negation_for_create(low: &str) -> bool {
    // Specific patterns directly referencing create
    let specific = [
        "do not create",
        "don't create",
        "do n't create",
        "no need to create",
        "no need create",
        "not to create",
        "won't create",
        "will not create",
    ];
    if specific.iter().any(|p| low.contains(p)) {
        return true;
    }

    // General negatives combined with "create" somewhere in the sentence
    let general_negatives = [
        "do not",
        "don't",
        " no ",
        " not ",
        "without",
        "avoid ",
        "stop ",
        "rather not",
        "won't",
        "will not",
    ];
    if general_negatives.iter().any(|p| low.contains(p)) && low.contains("create") {
        return true;
    }

    false
}

// Shared strict precheck to avoid duplication between streaming and non-streaming paths.
impl Agent {
    fn strict_precheck_response(&mut self, input: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        // If configured for assisted mode, disable auto-precheck and let the agent plan.
        if !self.config.precheck_mode.eq_ignore_ascii_case("strict") {
            return Ok(None);
        }
        // Tighten rule: in strict mode, reads and locates always run deterministically,
        // regardless of confirmation, to avoid LLM-only answers.
        let low = input.to_lowercase();
        let is_locate_verb = has_word_in(&low, &["locate", "find"]) || low.contains("where is");
        let is_read_verb = has_word_in(&low, &["read"]) || low.contains("what is inside") || low.contains("what's inside") || low.contains("contents of");
        let is_open_verb = has_word_in(&low, &["open"]);
        let is_create_verb = low.contains("create file") || has_word_in(&low, &["create"]) || low.contains("create a file");
        let is_delete_verb = low.contains("delete file") || has_word_in(&low, &["delete"]) || low.contains("remove file") || has_word_in(&low, &["remove"]);
        let is_negated_create = contains_negation_for_create(&low);
        // Web intents
        let has_url = low.contains("http://") || low.contains("https://");
        let wants_summary = low.contains("summarize") || low.contains("summary") || low.contains("brief");
        let wants_fetch = low.contains("fetch") || low.contains("get ") || low.contains("download");

        let has_filename_token = extract_file_candidate(input).is_some();

        if !(is_locate_verb || is_read_verb || is_open_verb || is_create_verb || has_filename_token || has_url) {
            return Ok(None);
        }
        // Handle web fetch/summary deterministically in strict mode if a URL is present
        if has_url {
            // Extract the first URL token
            let mut url = String::new();
            for tok in input.split_whitespace() {
                let t = tok.trim_matches(&[',', '"', '\'', '(', ')'][..]);
                if t.starts_with("http://") || t.starts_with("https://") { url = t.to_string(); break; }
            }
            if !url.is_empty() {
                if wants_summary {
                    if let Some(tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("web_summary")) {
                        match tool.invoke(&url) {
                            Ok(res) => {
                                // Format the web_summary result before returning
                                if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&res) {
                                    if let Some(summary) = map.get("summary").and_then(|s| s.as_str()) {
                                        return Ok(Some(summary.to_string()));
                                    }
                                }
                                return Ok(Some(res));
                            },
                            Err(e) => return Ok(Some(format!("Error summarizing '{}': {}", url, e))),
                        }
                    } else {
                        return Ok(Some("Web summary tool is not available.".to_string()));
                    }
                } else if wants_fetch || !wants_summary {
                    if let Some(tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("web_fetch")) {
                        match tool.invoke(&url) {
                            Ok(res) => {
                                // Format the web_fetch result before returning
                                if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&res) {
                                    return Ok(Some(format_web_fetch_output(&map)));
                                }
                                return Ok(Some(res));
                            },
                            Err(e) => return Ok(Some(format!("Error fetching '{}': {}", url, e))),
                        }
                    } else {
                        return Ok(Some("Web fetch tool is not available.".to_string()));
                    }
                }
            }
        }

        // Confirmation summaries are handled in planning stage; proceed with strict precheck.

        // If explicitly asked to create a file, handle via create_file tool and bypass locate.
        // Only proceed if intent is not negated. In confirm mode, preview and defer.
        if is_create_verb && !is_negated_create {
            if self.config.confirm_before_tools {
                if let Some(filename) = build_create_filename(input) {
                    let msg = self.preview_destructive(input, "create_file", &filename);
                    return Ok(Some(msg));
                } else {
                    return Ok(None);
                }
            }

            if let Some(filename) = build_create_filename(input) {
                if let Some(create_tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("create_file")) {
                    match create_tool.invoke(&filename) {
                        Ok(res) => return Ok(Some(res)),
                        Err(e) => return Ok(Some(format!("Error creating file '{}': {}", filename, e))),
                    }
                } else {
                    return Ok(Some("Create tool is not available.".to_string()));
                }
            } else {
                // No filename found; do not auto-create. Defer to LLM.
                return Ok(None);
            }
        }

        // Handle delete requests similarly: preview under confirmation, otherwise run delete_file on first located match.
        if is_delete_verb {
            // Extract candidate filename/path similar to other flows
            let candidate = if let Some(tok) = extract_file_candidate(input) {
                tok
            } else {
                input
                    .split_whitespace()
                    .rev()
                    .find(|t| !t.is_empty())
                    .map(|t| t.trim_matches(&[',', '.', '!', '?', '"', '\'' ][..]).to_string())
                    .unwrap_or_default()
            };

            if candidate.is_empty() {
                return Ok(None);
            }

            // Locate first match for delete safety preview
            let locate_tool = match self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("locate")) { Some(t) => t, None => return Ok(Some("Locate tool is not available.".to_string())) };
            let loc_res = match locate_tool.invoke(&candidate) { Ok(r) => r, Err(e) => return Ok(Some(format!("Error locating '{}': {}", candidate, e))) };
            if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&loc_res) {
                    if let Some(first) = arr.first().and_then(|v| v.get("path")).and_then(|p| p.as_str()) {
                        if self.config.confirm_before_tools {
                            self.pending_action = Some(("delete_file".to_string(), first.to_string()));
                            let msg = self.preview_destructive(input, "delete_file", first);
                            return Ok(Some(msg));
                        }
                        if let Some(del_tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("delete_file")) {
                            match del_tool.invoke(first) {
                                Ok(res) => { self.last_tool_invoked = Some(("delete_file".to_string(), first.to_string())); return Ok(Some(res)); },
                                Err(e) => return Ok(Some(format!("Error deleting '{}': {}", first, e))),
                            }
                        } else {
                            return Ok(Some("Delete tool is not available.".to_string()));
                        }
                    } else {
                        return Ok(Some(format!("No file named '{}' found under search roots.", candidate)));
                    }
                }
        }

        // Extract candidate for general file operations (locate, read, open, modify)
        let candidate = if let Some(tok) = extract_file_candidate(input) {
            tok
        } else {
            input
                .split_whitespace()
                .rev()
                .find(|t| !t.is_empty())
                .map(|t| t.trim_matches(&[',', '.', '!', '?', '"', '\'' ][..]).to_string())
                .unwrap_or_default()
        };

        if candidate.is_empty() {
            return Ok(Some("No filename detected in the query.".to_string()));
        }

        let locate_tool = match self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("locate")) {
            Some(t) => t,
            None => return Ok(Some("Locate tool is not available.".to_string())),
        };

        let loc_res = match locate_tool.invoke(&candidate) {
            Ok(r) => r,
            Err(e) => return Ok(Some(format!("Error locating '{}': {}", candidate, e))),
        };

        match serde_json::from_str::<Value>(&loc_res) {
            Ok(Value::Array(arr)) => {
                if arr.is_empty() {
                    return Ok(Some(format!("No file named '{}' found under search roots.", candidate)));
                }

                if is_locate_verb {
                    let human = format_locate_results(&arr);
                    return Ok(Some(human));
                }

                if let Some(first) = arr.first().and_then(|v| v.get("path")).and_then(|p| p.as_str()) {
                    // Detect modify/append intents (e.g., "add", "append", "write", "insert")
                    let is_modify_verb = has_word_in(&low, &["add", "append", "write", "insert"]) || low.contains("add content") || low.contains("append content") || low.contains("write to");
                    if is_modify_verb {
                            // Build arguments for add_content: prefer any explicit content detected by build_add_content_args,
                            // otherwise append empty content per user preference.
                            let add_args = if let Some(a) = build_add_content_args(input) {
                                if a.contains("--content") {
                                    // replace the candidate filename with the full resolved path from locate
                                    if let Some(pos) = a.find(" --content") {
                                        let content_part = a[pos..].trim().to_string();
                                        format!("{} {}", first, content_part)
                                    } else {
                                        format!("{} --content ", first)
                                    }
                                } else {
                                    format!("{} --content ", first)
                                }
                            } else {
                                format!("{} --content ", first)
                            };

                            if self.config.confirm_before_tools {
                                self.pending_action = Some(("add_content".to_string(), add_args.clone()));
                                let msg = self.preview_destructive(input, "add_content", &add_args);
                                return Ok(Some(msg));
                            }

                            if let Some(add_tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("add_content")) {
                                match add_tool.invoke(&add_args) {
                                    Ok(add_res) => {
                                        self.last_tool_invoked = Some(("add_content".to_string(), add_args.clone()));
                                        // If add_content returned JSON with { "success": true }, treat as success.
                                        if let Ok(val) = serde_json::from_str::<Value>(&add_res) {
                                            if let Value::Object(map) = &val {
                                                if map.get("success").and_then(|b| b.as_bool()).unwrap_or(false) {
                                                    return Ok(Some(add_res));
                                                }
                                                // Otherwise, fall back to reading the file and return that output alongside the add result.
                                            } else {
                                                // Non-object JSON, treat it as success output.
                                                return Ok(Some(add_res));
                                            }
                                        } else {
                                            // Non-JSON output from add_content — return it as-is.
                                            return Ok(Some(add_res));
                                        }

                                        // Fallback: attempt to read the file and return its contents with add error info.
                                        if let Some(read_tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("read_file")) {
                                            match read_tool.invoke(first) {
                                                Ok(read_res) => {
                                                    self.last_tool_invoked = Some(("read_file".to_string(), first.to_string()));
                                                    match serde_json::from_str::<Value>(&read_res) {
                                                        Ok(Value::Object(map)) => {
                                                            let human = format_read_output(&map);
                                                            return Ok(Some(format!("AddContent returned: {}\n\n(Fallback read) {}", add_res, human)));
                                                        }
                                                        _ => return Ok(Some(format!("AddContent returned: {}\n\n(Fallback read) {}", add_res, read_res))),
                                                    }
                                                }
                                                Err(e) => return Ok(Some(format!("Error adding content to '{}': {}. Also failed to read file: {}", first, add_res, e))),
                                            }
                                        } else {
                                            return Ok(Some(format!("AddContent returned: {}\n\n(Fallback read not available)", add_res)));
                                        }
                                    }
                                    Err(e) => {
                                        // On error invoking add_content, attempt to read the file as fallback
                                        if let Some(read_tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("read_file")) {
                                            match read_tool.invoke(first) {
                                                Ok(read_res) => {
                                                    self.last_tool_invoked = Some(("read_file".to_string(), first.to_string()));
                                                    match serde_json::from_str::<Value>(&read_res) {
                                                        Ok(Value::Object(map)) => {
                                                            let human = format_read_output(&map);
                                                            return Ok(Some(format!("Error adding content to '{}': {}\n\n(Fallback read) {}", first, e, human)));
                                                        }
                                                        _ => return Ok(Some(format!("Error adding content to '{}': {}\n\n(Fallback read) {}", first, e, read_res))),
                                                    }
                                                }
                                                Err(read_err) => return Ok(Some(format!("Error adding content to '{}': {}. Also failed to read file: {}", first, e, read_err))),
                                            }
                                        } else {
                                            return Ok(Some(format!("Error adding content to '{}': {}", first, e)));
                                        }
                                    }
                                }
                            } else {
                                return Ok(Some("AddContent tool is not available.".to_string()));
                            }
                        }

                    if is_open_verb {
                        if self.config.confirm_before_tools {
                            // Preview and defer open
                            let msg = self.preview_destructive(input, "open_file", first);
                            return Ok(Some(msg));
                        }
                        if let Some(open_tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("open_file")) {
                            match open_tool.invoke(first) {
                                Ok(open_res) => { self.last_tool_invoked = Some(("open_file".to_string(), first.to_string())); return Ok(Some(open_res)); },
                                Err(e) => return Ok(Some(format!("Error opening '{}': {}", first, e))),
                            }
                        }
                    }

                    if let Some(read_tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("read_file")) {
                        match read_tool.invoke(first) {
                            Ok(read_res) => {
                                self.last_tool_invoked = Some(("read_file".to_string(), first.to_string()));
                                match serde_json::from_str::<Value>(&read_res) {
                                    Ok(Value::Object(map)) => {
                                        let human = format_read_output(&map);
                                        return Ok(Some(human));
                                    }
                                    _ => return Ok(Some(read_res)),
                                }
                            }
                            Err(e) => return Ok(Some(format!("Error reading '{}': {}", first, e))),
                        }
                    }
                }
                Ok(None)
            }
            _ => Ok(Some(format!("Locate returned unexpected output for '{}'.", candidate))),
        }
    }
}

// ===== small helpers to reduce duplication =====

impl Agent {
    fn preview_destructive(&mut self, question: &str, tool_name: &str, tool_input: &str) -> String {
        self.pending_action = Some((tool_name.to_string(), tool_input.to_string()));
        let msg = if tool_name.eq_ignore_ascii_case("create_file") {
            // Try to present filename and content more humanly
            let (file_part, content_part) = if let Some(idx) = tool_input.find("--content ") {
                let (f, rest) = tool_input.split_at(idx);
                let content = rest.trim_start_matches("--content ").trim();
                (f.trim().to_string(), content.to_string())
            } else {
                (tool_input.to_string(), String::new())
            };

            if content_part.is_empty() {
                format!(
                    "rustline: I am going to create this file. Can you confirm?\nFile: {}\nReply `yes` to proceed, or `no` to cancel.",
                    file_part
                )
            } else {
                format!(
                    "rustline: I am going to create this file. Can you confirm?\nFile: {}\nContent: {}\nReply `yes` to proceed, or `no` to cancel.",
                    file_part, content_part
                )
            }
        } else if tool_name.eq_ignore_ascii_case("open_file") {
            format!(
                "rustline: I am going to open this file. Can you confirm?\nFile: {}\nReply `yes` to proceed, or `no` to cancel.",
                tool_input
            )
        } else if tool_name.eq_ignore_ascii_case("delete_file") {
            format!(
                "rustline: I am going to delete this file. Can you confirm?\nFile: {}\nReply `yes` to proceed, or `no` to cancel.",
                tool_input
            )
        } else if tool_name.eq_ignore_ascii_case("add_content") {
            format!(
                "rustline: I am going to add content to this file. Can you confirm?\nFile and Content: {}\nReply `yes` to proceed, or `no` to cancel.",
                tool_input
            )
        } else {
            format!(
                "rustline: Planned '{}' with input `{}`. Reply `yes` to proceed, or `no` to cancel.",
                tool_name, tool_input
            )
        };
        self.log_history("user", question.to_string());
        self.log_history("assistant", msg.clone());
        msg
    }
}

// Build arguments for add_content tool from a user question.
// Returns something like: "filename.txt --content <text>" or just "filename.txt".
// Used ONLY for add_content operations.
fn build_add_content_args(question: &str) -> Option<String> {
    let low = question.to_lowercase();
    let candidate = if let Some(tok) = extract_file_candidate(question) {
        tok
    } else {
        question
            .split_whitespace()
            .rev()
            .find(|t| !t.is_empty())
            .map(|t| t.trim_matches(&[',', '.', '!', '?', '"', '\'' ][..]).to_string())
            .unwrap_or_default()
    };

    if candidate.is_empty() { return None; }

    // Case 1: explicit --content ...
    if let Some(pos) = low.find("--content") {
        let content = question[pos..].trim();
        if !content.is_empty() {
            return Some(format!("{} {}", candidate, content));
        }
    }

    // Case 2: quoted text: "..."
    if let Some(start_q) = question.find('"') {
        if let Some(end_q_rel) = question[start_q + 1..].find('"') {
            let end_idx = start_q + 1 + end_q_rel;
            let quoted = &question[start_q + 1..end_idx];
            if !quoted.trim().is_empty() {
                return Some(format!("{} --content {}", candidate, quoted));
            }
        }
    }

    // Case 3: phrases like "with text", "with content", "content is", "text is", "write ", "to X <text>"
    let patterns = ["with text", "with content", "content is", "text is", "write "];
    for pat in patterns.iter() {
        if let Some(p) = low.find(pat) {
            let tail = question[p + pat.len()..].trim();
            if !tail.is_empty() {
                return Some(format!("{} --content {}", candidate, tail));
            }
        }
    }

    // Case 4: "add/append content to <filename> <text>" or "add/append <text> to <filename>"
    // Handle both word orders by checking where content appears relative to filename
    if low.contains("add ") || low.contains("append ") || low.contains("write ") || low.contains("insert ") {
        let mut extracted_content: Option<String> = None;
        
        // First try: content after filename (e.g., "add content to AItest.txt this is text")
        if let Some(pos) = question.find(&candidate) {
            let after_filename = question[pos + candidate.len()..].trim();
            if !after_filename.is_empty() && after_filename != "this" && after_filename != "is" {
                extracted_content = Some(after_filename.to_string());
            }
        }
        
        // Second try: content before filename using " to " marker (e.g., "add this is text to AItest.txt")
        if extracted_content.is_none() && low.contains(" to ") {
            if let Some(to_pos) = low.find(" to ") {
                let after_to = &question[to_pos + 4..];
                // Verify filename appears after "to"
                if after_to.contains(&candidate) {
                    let verbs = ["add ", "append ", "write ", "insert "];
                    for verb in verbs.iter() {
                        if let Some(verb_pos) = low.find(verb) {
                            let start = verb_pos + verb.len();
                            let content = question[start..to_pos].trim();
                            // Strip noise words like "content", "text"
                            let content_clean = content
                                .strip_prefix("content ").unwrap_or(content)
                                .strip_prefix("text ").unwrap_or(content);
                            if !content_clean.is_empty() && content_clean != &candidate {
                                extracted_content = Some(content_clean.to_string());
                            }
                            break;
                        }
                    }
                }
            }
        }
        
        // If we found content, return with --content flag
        if let Some(content) = extracted_content {
            return Some(format!("{} --content {}", candidate, content));
        }
    }

    Some(candidate)
}

// Extract just the filename for create_file operations (no content).
fn build_create_filename(question: &str) -> Option<String> {
    if let Some(tok) = extract_file_candidate(question) {
        return Some(tok);
    }
    
    // Fallback: last non-empty token
    question
        .split_whitespace()
        .rev()
        .find(|t| !t.is_empty())
        .map(|t| t.trim_matches(&[',', '.', '!', '?', '"', '\'' ][..]).to_string())
}

// Helper to run pending action consistently.
impl Agent {
    fn run_pending_action(&mut self) -> Result<String, String> {
        if let Some((tool, inp)) = self.pending_action.take() {
            if let Some(tool_impl) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case(&tool)) {
                match tool_impl.invoke(&inp) {
                    Ok(res) => { self.last_tool_invoked = Some((tool.clone(), inp.clone())); Ok(res) }
                    Err(e) => Err(format!("Tool `{}` error: {}", tool, e)),
                }
            } else {
                Err(format!("Unknown tool `{}`.", tool))
            }
        } else {
            Err("No pending action to run.".to_string())
        }
    }

    pub fn log_history(&mut self, role: &str, content: String) {
        let message = Message::new(role.to_string(), content);
        self.persist_message(&message);
        self.history.push(message);
    }

    /// Persist a message to storage if session manager is available
    fn persist_message(&mut self, message: &Message) {
        if let Some(session_manager) = &mut self.session_manager {
            if let Err(e) = session_manager.save_message_to_current_session(message) {
                log::warn!("Failed to persist message: {}", e);
            }
        }
    }

    /// Persist a message with tool invocation data
    pub fn persist_message_with_tool(&mut self, role: &str, content: &str, tool_name: &str, tool_input: &str, tool_output: &str, success: bool) {
        let tool_invocation = crate::ollama::ToolInvocation {
            tool_name: tool_name.to_string(),
            input: tool_input.to_string(),
            output: tool_output.to_string(),
            success,
        };
        
        let message = Message::new_with_tool(role.to_string(), content.to_string(), tool_invocation);
        self.persist_message(&message);
        self.history.push(message);
    }
}

// Formatting helpers
fn format_locate_results(arr: &[Value]) -> String {
    let mut human = String::new();
    human.push_str(&format!("Found {} match(es):", arr.len()));
    for v in arr.iter() {
        if let Some(p) = v.get("path").and_then(|s| s.as_str()) {
            let size = v.get("size").and_then(|n| n.as_u64()).map(|n| n.to_string()).unwrap_or_else(|| "?".to_string());
            human.push_str(&format!("\n- {} ({} bytes)", p, size));
        }
    }
    human
}

fn format_read_output(map: &serde_json::Map<String, Value>) -> String {
    let path = map.get("path").and_then(|v| v.as_str()).unwrap_or("<unknown>");
    let size = map.get("size").and_then(|v| v.as_u64()).map(|n| n.to_string()).unwrap_or_else(|| "?".to_string());
    let truncated = map.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
    let content = map.get("content").and_then(|v| v.as_str()).unwrap_or("");

    let mut human = String::new();
    human.push_str(&format!("File: {}\nSize: {} bytes\nTruncated: {}\n\n", path, size, truncated));
    human.push_str(content);
    human
}

fn format_web_fetch_output(map: &serde_json::Map<String, Value>) -> String {
    let title = map.get("title").and_then(|v| v.as_str()).unwrap_or("(no title)");
    let text = map.get("text").and_then(|v| v.as_str()).unwrap_or("");
    
    const MAX_TEXT_CHARS: usize = 1000;
    let text_display = if text.len() > MAX_TEXT_CHARS {
        format!("{}...", &text[..MAX_TEXT_CHARS])
    } else {
        text.to_string()
    };
    
    if text_display.is_empty() {
        format!("{}\n\n(No text content extracted)", title)
    } else {
        format!("{}\n\n{}", title, text_display)
    }
}

// Interpret natural-language confirmation: returns Some(true) for affirmative, Some(false) for negative, None otherwise.
fn interpret_confirmation(input: &str) -> Option<bool> {
    let s = input.trim().to_lowercase();
    // Affirmatives
    let yes_list = [
        "yes", "y", "sure", "of course", "confirm", "please do", "go ahead",
        "proceed", "okay", "ok", "do it", "sounds good", "looks good"
    ];
    if yes_list.iter().any(|p| s.contains(p)) {
        return Some(true);
    }

    // Negatives
    let no_list = [
        "no", "n", "don't", "do not", "nope", "cancel", "skip", "stop",
        "not now", "rather not", "please don't", "do n't"
    ];
    if no_list.iter().any(|p| s.contains(p)) {
        return Some(false);
    }

    None
}


