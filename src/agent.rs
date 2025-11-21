use reqwest::Client;
use tokio::time::{sleep, Duration};

use crate::config::Config;
use crate::ollama;
use crate::ollama::Message;
use crate::tools::{self, DynTool};

const REACT_PROMPT_TEMPLATE: &str = r#"
You are Rustline, a helpful assistant that solves problems by reasoning step-by-step
and using tools when needed.

You have access to the following tools:

{tool_descriptions}

Each tool is described as: name | description

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
    name: String,
    http: Client,
    history: Vec<Message>,
    config: Config,
    tools: Vec<DynTool>,
}

impl Agent {
    /// Create a new agent with given config.
    pub fn new(config: Config) -> Self {
        Agent {
            name: "Rustline Agent".to_string(),
            http: Client::new(),
            history: Vec::new(),
            config,
            tools: tools::default_tools(),
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
    fn try_run_tool(&self, input: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if !input.starts_with('!') {
            return Ok(None);
        }

        let rest = &input[1..];
        let mut parts = rest.splitn(2, ' ');
        let name_part = parts.next().unwrap_or("").to_lowercase();
        let args = parts.next().unwrap_or("").trim();

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

        let prompt = REACT_PROMPT_TEMPLATE
            .replace("{tool_descriptions}", &tool_descs)
            .replace("{tool_names}", &tool_names)
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

        let question = input.to_string();
        let mut steps: Vec<AgentStep> = Vec::new();
        let max_iterations = 5;

        println!("\n[ReAct] Starting reasoning loop for question: {question}");

        for iter in 0..max_iterations {
            println!("[ReAct] Iteration {}", iter + 1);

            // small delay just so we see the loop
            sleep(Duration::from_millis(50)).await;

            let plan = self.plan_once(&question, &steps).await?;

            match plan {
                PlanOutput::FinalAnswer { thought, answer } => {
                    if let Some(t) = thought {
                        println!("[Thought] {}", t);
                    }
                    println!("[Final Answer] {}", answer);

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
                            Ok(res) => res,
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

                    let short_obs = if observation.len() > 200 {
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

        println!("[ReAct] Stopped due to max iterations without finishing.");
        Ok("Agent stopped due to max iterations without finishing.".to_string())
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

        if line.starts_with("Thought:") {
            let thought = line["Thought:".len()..].trim().to_string();
            if !thought.is_empty() {
                last_thought = Some(thought);
            }
        } else if line.starts_with("Action:") {
            let action_name = line["Action:".len()..].trim().to_string();

            let mut action_input = String::new();
            if i + 1 < lines.len() && lines[i + 1].starts_with("Action Input:") {
                action_input = lines[i + 1]["Action Input:".len()..].trim().to_string();
                i += 1;
            }

            last_action = Some((action_name, action_input));
        } else if line.starts_with("Final Answer:") {
            let ans = line["Final Answer:".len()..].trim().to_string();
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
