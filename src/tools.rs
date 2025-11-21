use std::error::Error;

use chrono::Local;

pub type ToolResult = Result<String, Box<dyn Error>>;

/// Common interface for all tools.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn invoke(&self, args: &str) -> ToolResult;
}

pub type DynTool = Box<dyn Tool>;

/// Time tool: returns current local time (based on system timezone).
pub struct TimeTool;

impl Tool for TimeTool {
    fn name(&self) -> &str {
        "time"
    }

    fn description(&self) -> &str {
        "Show the current local time. Usage: !time"
    }

    fn invoke(&self, _args: &str) -> ToolResult {
        let now = Local::now();
        Ok(format!(
            "Current local time: {}",
            now.format("%Y-%m-%d %H:%M:%S")
        ))
    }
}

/// Echo tool: just echoes arguments.
pub struct EchoTool;

impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo back the given text. Usage: !echo <text>"
    }

    fn invoke(&self, args: &str) -> ToolResult {
        Ok(args.trim().to_string())
    }
}

/// All built-in tools available to the agent.
pub fn default_tools() -> Vec<DynTool> {
    vec![Box::new(TimeTool), Box::new(EchoTool)]
}
