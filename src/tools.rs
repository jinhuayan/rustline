use std::error::Error;
use std::fs;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use walkdir::WalkDir;
use serde_json::json;

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

/// Read file tool: returns the contents of a file.
/// Usage: `!read_file <path>` or called by the agent.
pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file's contents. Usage: !read_file <path> (returns truncated content if large)"
    }

    fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();
        if input.is_empty() {
            return Ok("Usage: !read_file <path_or_filename>".to_string());
        }

        // If the input looks like a path (contains a separator or starts with '.' or '~'),
        // try to read it directly (with tilde expansion).
        let looks_like_path = input.contains(std::path::MAIN_SEPARATOR) || input.starts_with('.') || input.starts_with('~');

        if looks_like_path {
            let p = expand_path(input);
            return read_and_truncate(&p);
        }

        // Otherwise treat as a bare filename and search recursively through sensible roots.
        let filename = input;

        let mut search_roots: Vec<PathBuf> = Vec::new();
        // cwd stands for current working directory
        if let Ok(cwd) = env::current_dir() {
            search_roots.push(cwd.clone());
            search_roots.push(cwd.join("src"));

            if let Some(root) = find_project_root(&cwd) {
                if !search_roots.contains(&root) {
                    search_roots.push(root.clone());
                    search_roots.push(root.join("src"));
                }
            }
        }

        // Search each root using WalkDir and return the first match.
        for root in search_roots {
            if !root.exists() {
                continue;
            }

            for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                        if name.eq_ignore_ascii_case(filename) {
                            return read_and_truncate(entry.path());
                        }
                    }
                }
            }
        }

        Ok(format!(
            "No file named '{}' found under current dir or project src. Try providing a path.",
            filename
        ))
    }
}
//Helper function turns a path with ~ or relative into an absolute PathBuf
fn expand_path(input: &str) -> PathBuf {
    if input == "~" {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home);
        }
    }

    if let Some(rest) = input.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            let mut p = PathBuf::from(home);
            p.push(rest);
            return p;
        }
    }

    let p = PathBuf::from(input);
    if p.is_relative() {
        if let Ok(cwd) = env::current_dir() {
            return cwd.join(p);
        }
    }

    p
}
//Helper function to find the project root by looking for Cargo.toml
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join("Cargo.toml").exists() {
            return Some(cur);
        }

        if !cur.pop() {
            break;
        }
    }
    None
}

//Helper function to read a file and truncate its contents if too large
fn read_and_truncate(path: &Path) -> ToolResult {
    // Return a JSON object as a single-line string: { path, size, truncated, content }
    let abs = path.canonicalize()?;
    match fs::read_to_string(&abs) {
        Ok(contents) => {
            const MAX_BYTES: usize = 100_000; // 100 KB
            let size = contents.len();
            let (truncated, content_str) = if size > MAX_BYTES {
                (true, contents[..MAX_BYTES].to_string())
            } else {
                (false, contents)
            };

            let obj = json!({
                "path": abs.to_string_lossy(),
                "size": size,
                "truncated": truncated,
                "content": content_str
            });

            Ok(serde_json::to_string(&obj)?)
        }
        Err(e) => Err(Box::new(e)),
    }
}

/// All built-in tools available to the agent.
pub fn default_tools() -> Vec<DynTool> {
    vec![Box::new(TimeTool), Box::new(EchoTool), Box::new(ReadFileTool), Box::new(OpenWithTool)]
}

/// Open file with application tool: opens a file in a native application.
/// Usage:
///  - `!open_with <path>` opens with the system default application.
///  - On macOS: `!open_with -a <AppName> <path>` opens with specified application (e.g., Notes).
pub struct OpenWithTool;

impl Tool for OpenWithTool {
    fn name(&self) -> &str {
        "open_with"
    }

    fn description(&self) -> &str {
        "Open a file with the system default app, or on macOS use -a <AppName> to choose an app. Usage: !open_with [-a AppName] <path>"
    }

    fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();
        if input.is_empty() {
            return Ok("Usage: !open_with [-a AppName] <path>".to_string());
        }

        // parse optional macOS -a AppName
        let mut app: Option<String> = None;
        let mut path_str = input.to_string();

        let mut parts = input.split_whitespace();
        if let Some(first) = parts.next() {
            if first == "-a" {
                if let Some(appname) = parts.next() {
                    if let Some(rest) = parts.collect::<Vec<&str>>().join(" ").as_str().get(0..) {
                        // rest is the path joined
                        app = Some(appname.to_string());
                        path_str = rest.to_string();
                    }
                } else {
                    return Ok("Usage: !open_with -a <AppName> <path>".to_string());
                }
            }
        }

        // If no -a prefix was used, path_str remains input
        if path_str.is_empty() {
            return Ok("Usage: !open_with [-a AppName] <path>".to_string());
        }

        let p = expand_path(&path_str);
        if !p.exists() {
            return Err(format!("Path not found: {}", p.display()).into());
        }

        // Platform-specific open command
        #[cfg(target_os = "macos")]
        let result = {
                if let Some(appname) = app {
                Command::new("open")
                    .arg("-a")
                    .arg(appname)
                    .arg(&p)
                    .spawn()
            } else {
                Command::new("open").arg(&p).spawn()
            }
        };

        #[cfg(target_os = "linux")]
        let result = {
            // xdg-open is the common opener on many Linux systems
            Command::new("xdg-open").arg(&p).spawn()
        };

        #[cfg(target_os = "windows")]
        let result = {
            // 'start' is a shell builtin; run via cmd
            Command::new("cmd").args(&["/C", "start", "", &p.to_string_lossy()]).spawn()
        };

        match result {
            Ok(_child) => Ok(format!("Opened: {}", p.display())),
            Err(e) => Err(Box::new(e)),
        }
    }
}

// OpenFileTool removed: use OpenWithTool or ReadFileTool (returns JSON) instead.

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_name(prefix: &str) -> String {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        format!("{}_{}", prefix, now)
    }

    #[test]
    fn test_read_and_truncate_small_file() {
        let tmp = env::temp_dir().join(unique_name("rustline_small") + ".txt");
        let content = "hello rustline";
        fs::write(&tmp, content).expect("write failed");

        let got = read_and_truncate(&tmp).expect("read failed");
        let v: serde_json::Value = serde_json::from_str(&got).expect("invalid json");
        assert_eq!(v["content"].as_str().unwrap(), content);
        assert_eq!(v["truncated"].as_bool().unwrap(), false);

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_read_and_truncate_large_file() {
        let tmp = env::temp_dir().join(unique_name("rustline_large") + ".txt");
        let large = "a".repeat(150_000);
        fs::write(&tmp, &large).expect("write failed");

        let got = read_and_truncate(&tmp).expect("read failed");
        let v: serde_json::Value = serde_json::from_str(&got).expect("invalid json");
        assert_eq!(v["truncated"].as_bool().unwrap(), true);
        assert_eq!(v["content"].as_str().unwrap().len(), 100_000);

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_expand_path_tilde() {
        // If HOME is set in the environment, verify expansion uses it.
        if let Ok(home) = env::var("HOME") {
            let p1 = expand_path("~");
            assert_eq!(p1, PathBuf::from(&home));

            let p2 = expand_path("~/foo/bar");
            assert_eq!(p2, PathBuf::from(&home).join("foo/bar"));
        } else {
            // If HOME isn't set in this environment, skip the tilde expansion assertions.
            eprintln!("HOME not set; skipping tilde expansion test");
        }
    }

    #[test]
    fn test_read_file_tool_direct_path() {
        let tmp = env::temp_dir().join(unique_name("rustline_tool") + ".txt");
        let content = "direct path content";
        fs::write(&tmp, content).expect("write failed");

        let tool = ReadFileTool;
        let res = tool.invoke(tmp.to_str().unwrap()).expect("invoke failed");
        let v: serde_json::Value = serde_json::from_str(&res).expect("invalid json");
        assert_eq!(v["content"].as_str().unwrap(), content);

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_read_file_tool_search_filename() {
        // Create a temporary directory structure and set cwd to it for the test.
        let base = env::temp_dir().join(unique_name("rustline_search"));
        let src_dir = base.join("src");
        fs::create_dir_all(&src_dir).unwrap();

        let filename = "search_me.txt";
        let file_path = src_dir.join(filename);
        let content = "found via search";
        fs::write(&file_path, content).expect("write failed");

        let old_cwd = env::current_dir().unwrap();
        env::set_current_dir(&base).expect("set cwd");

        let tool = ReadFileTool;
        let res = tool.invoke(filename).expect("invoke failed");
        let v: serde_json::Value = serde_json::from_str(&res).expect("invalid json");
        assert_eq!(v["content"].as_str().unwrap(), content);

        // restore cwd
        env::set_current_dir(old_cwd).unwrap();
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn test_open_file_tool_returns_path_and_content() {
        let tmp = env::temp_dir().join(unique_name("rustline_open") + ".txt");
        let content = "lorem ipsum dolor";
        fs::write(&tmp, content).expect("write failed");
        let _ = fs::remove_file(&tmp);
    }
}
