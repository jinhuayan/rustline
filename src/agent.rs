use reqwest::Client;
use tokio::time::{sleep, Duration};
use std::time::Instant;

use crate::config::Config;
use crate::ollama;
use crate::ollama::Message;
use crate::tools::{self, DynTool};
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
pub struct Agent {
    http: Client,
    history: Vec<Message>,
    config: Config,
    tools: Vec<DynTool>,
    pending_input: Option<String>,
    pending_action: Option<(String, String)>, // (tool, input)
    last_tool_invoked: Option<(String, String)>, // (tool, input)
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
        }
    }
}

impl Agent {
    /// Create a new agent with given config.
    pub fn new(config: Config) -> Self {
        Agent {
            http: Client::new(),
            history: Vec::new(),
            config,
            tools: tools::default_tools(),
            pending_input: None,
            pending_action: None,
            last_tool_invoked: None,
        }
    }

    /// Clear conversation state (for now just local history).
    pub fn reset(&mut self) {
        self.history.clear();
    }

    /// Change the model name at runtime.
    pub fn set_model(&mut self, model: String) {
        self.config.model = model;
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
                "on" | "true" | "yes" => { self.config.confirm_before_tools = true; return Ok(Some("Confirm-before-tools: ON".to_string())); }
                "off" | "false" | "no" => { self.config.confirm_before_tools = false; return Ok(Some("Confirm-before-tools: OFF".to_string())); }
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
        // avoid hardcoded user-specific folders; rely on cwd/project root and HOME

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
            self.history.push(Message {
                role: "user".to_string(),
                content: input.to_string(),
            });
            self.history.push(Message {
                role: "assistant".to_string(),
                content: tool_reply.clone(),
            });
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
        let max_iterations = 5;
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

                    // If confirmation mode is ON and the input implies a toolable intent (e.g., create),
                    // but the LLM did not actually call a tool, offer to run the appropriate tool.
                    if self.config.confirm_before_tools {
                        let lowq = question.to_lowercase();
                        let is_create_verb = lowq.contains("create file") || lowq.starts_with("create ") || lowq.contains("create a file");
                        let is_negated_create = contains_negation_for_create(&lowq);
                        if is_create_verb && !is_negated_create && self.pending_action.is_none() {
                            if let Some(args) = build_create_args(&question) {
                                let msg = self.preview_destructive(&question, "create_file", &args);
                                return Ok(msg);
                            }
                        }
                    }

                    // store as simple Q/A history
                    self.history.push(Message {
                        role: "user".to_string(),
                        content: question.clone(),
                    });
                    self.history.push(Message {
                        role: "assistant".to_string(),
                        content: answer.clone(),
                    });

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
                            Ok(res) => { self.last_tool_invoked = Some((tool_name.clone(), planned.input.clone())); res },
                            Err(e) => format!("Tool `{}` error: {}", tool_name, e),
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

            // println!("[ReAct] Stopped due to max iterations without finishing.");
        Ok("Agent stopped due to max iterations without finishing.".to_string())
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
            self.history.push(Message {
                role: "user".to_string(),
                content: input.to_string(),
            });
            self.history.push(Message {
                role: "assistant".to_string(),
                content: tool_reply.clone(),
            });
            return Ok(tool_reply);
        }

        if let Some(precheck) = self.strict_precheck_response(input)? {
            on_chunk(&precheck);
            return Ok(precheck);
        }

        let question = input.to_string();
        let mut steps: Vec<AgentStep> = Vec::new();
        let max_iterations = 5;

        for _ in 0..max_iterations {
            let plan = self.plan_once(&question, &steps).await?;

            match plan {
                PlanOutput::FinalAnswer { thought, answer } => {
                    if let Some(t) = thought {
                        on_think(&format!("Thought: {}", t));
                    }
                    
                    // If confirmation mode is ON and input implies a toolable intent (e.g., create) but no tool was planned,
                    // offer to run the appropriate tool instead of finalizing immediately.
                    if self.config.confirm_before_tools {
                        let lowq = question.to_lowercase();
                        let is_create_verb = lowq.contains("create file") || lowq.starts_with("create ") || lowq.contains("create a file");
                        let is_negated_create = contains_negation_for_create(&lowq);
                        if is_create_verb && !is_negated_create && self.pending_action.is_none() {
                            if let Some(args) = build_create_args(&question) {
                                let msg = self.preview_destructive(&question, "create_file", &args);
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
                    self.history.push(Message {
                        role: "user".to_string(),
                        content: question.clone(),
                    });
                    self.history.push(Message {
                        role: "assistant".to_string(),
                        content: answer.clone(),
                    });

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
                            Ok(res) => { self.last_tool_invoked = Some((tool_name.clone(), planned.input.clone())); res },
                            Err(e) => format!("Tool `{}` error: {}", tool_name, e),
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

        let msg = "Agent stopped due to max iterations without finishing.";
        on_think(msg);
        Ok(msg.to_string())
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
        let is_open_verb = has_word_in(&low, &["open"]) && low.contains("file");
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
                if let Some(args) = build_create_args(input) {
                    let msg = self.preview_destructive(input, "create_file", &args);
                    return Ok(Some(msg));
                } else {
                    return Ok(None);
                }
            }

            if let Some(args) = build_create_args(input) {
                if let Some(create_tool) = self.tools.iter().find(|t| t.name().eq_ignore_ascii_case("create_file")) {
                    match create_tool.invoke(&args) {
                        Ok(res) => return Ok(Some(res)),
                        Err(e) => return Ok(Some(format!("Error creating via args '{}': {}", args, e))),
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
                    if is_open_verb {
                        if self.config.confirm_before_tools {
                            // Preview and defer open
                            self.pending_action = Some(("open_file".to_string(), first.to_string()));
                            let msg = format!("Planned tool: 'open_file'\nInput: {}\nType !do to run, or !skip to cancel.", first);
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
                    "rustline: I am going to create this file. Can you confirm?\nFile: {}\nReply `!do` to proceed, or `!skip` to cancel.",
                    file_part
                )
            } else {
                format!(
                    "rustline: I am going to create this file. Can you confirm?\nFile: {}\nContent: {}\nReply `!do` to proceed, or `!skip` to cancel.",
                    file_part, content_part
                )
            }
        } else if tool_name.eq_ignore_ascii_case("open_file") {
            format!(
                "rustline: I am going to open this file. Can you confirm?\nFile: {}\nReply `!do` to proceed, or `!skip` to cancel.",
                tool_input
            )
        } else if tool_name.eq_ignore_ascii_case("delete_file") {
            format!(
                "rustline: I am going to delete this file. Can you confirm?\nFile: {}\nReply `!do` to proceed, or `!skip` to cancel.",
                tool_input
            )
        } else {
            format!(
                "rustline: Planned '{}' with input `{}`. Reply `!do` to proceed, or `!skip` to cancel.",
                tool_name, tool_input
            )
        };
        self.log_history("user", question.to_string());
        self.log_history("assistant", msg.clone());
        msg
    }
}

// Build unified args for create_file tool from a user question.
// Returns something like: "filename.txt --content <text>" or just "filename.txt".
fn build_create_args(question: &str) -> Option<String> {
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

    // Case 3: phrases like "with text", "with content", "content is", "text is", "write "
    let patterns = ["with text", "with content", "content is", "text is", "write "];
    for pat in patterns.iter() {
        if let Some(p) = low.find(pat) {
            let tail = question[p + pat.len()..].trim();
            if !tail.is_empty() {
                return Some(format!("{} --content {}", candidate, tail));
            }
        }
    }

    Some(candidate)
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

    fn log_history(&mut self, role: &str, content: String) {
        self.history.push(Message { role: role.to_string(), content });
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
