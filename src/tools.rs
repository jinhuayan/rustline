use std::error::Error;
use std::fs;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use walkdir::WalkDir;
use std::collections::HashSet;
use serde_json::json;

use chrono::Local;

pub type ToolResult = Result<String, Box<dyn Error>>;

// ═══════════════════════════════════════════════════════════════════════════
// Tools for Rustline Agent
// ═══════════════════════════════════════════════════════════════════════════

// Common interface for all tools. Every tool must follow this trait.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn invoke(&self, args: &str) -> ToolResult;
}

pub type DynTool = Box<dyn Tool>;

// ═══════════════════════════════════════════════════════════════════════════
// Time tool: returns the current local time.
// ═══════════════════════════════════════════════════════════════════════════
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

// ═══════════════════════════════════════════════════════════════════════════
// Echo tool: echoes back the given text.
// ═══════════════════════════════════════════════════════════════════════════
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


// ═══════════════════════════════════════════════════════════════════════════
// Read file tool: reads and returns the contents of a file.
/// Usage: `!read_file <path>` or called by the agent.
// ═══════════════════════════════════════════════════════════════════════════
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

        if let Some(p) = resolve_to_path(input) {
            return read_and_truncate(&p);
        }
        Ok(format!("No file named '{}' found under search roots.", input))
    }
}

//Helper function to read a file and truncate its contents if too large
// The triggering size for truncation is 100 KB
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


// ═══════════════════════════════════════════════════════════════════════════
// Locate tool: searches configured roots for files matching a basename and returns a JSON array
// of matches: [{"path": "...", "size": 123}, ...]
// ═══════════════════════════════════════════════════════════════════════════
pub struct LocateTool;

impl Tool for LocateTool {
    fn name(&self) -> &str {
        "locate"
    }

    fn description(&self) -> &str {
        "Locate files by basename in configured roots. Usage: !locate <filename>"
    }

    fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();
        if input.is_empty() {
            return Ok("Usage: !locate <filename>".to_string());
        }

        let matches = locate_matches(input);
        let results: Vec<serde_json::Value> = matches
            .into_iter()
            .map(|(pb, size)| json!({"path": pb.to_string_lossy().to_string(), "size": size}))
            .collect();
        Ok(serde_json::to_string(&results)?)
    }
}

//Helper function turns a path with ~ or relative into an absolute PathBuf
pub fn expand_path(input: &str) -> PathBuf {

    //If input starts with ~, expand to HOME
    if input == "~" {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    //If input starts with ~/ expand to HOME/rest
    if let Some(rest) = input.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            let mut p = PathBuf::from(home);
            p.push(rest);
            return p;
        }
    }
    //If input is relative, join with current working directory
    let p = PathBuf::from(input);
    if p.is_relative() {
        if let Ok(cwd) = env::current_dir() {
            return cwd.join(p);
        }
    }
    //Else return as is (absolute path)
    p
}

// ----- Shared locating helpers (single source of truth) -----
fn search_roots() -> Vec<PathBuf> {
    //empty vector of roots
    let mut roots: Vec<PathBuf> = Vec::new();
    //Add current working directory and its src/, plus project root if found
    if let Ok(cwd) = env::current_dir() {
        roots.push(cwd.clone());
        roots.push(cwd.join("src"));
        // Attempt to find project root (Cargo.toml) and add it and its src/
        if let Some(root) = find_project_root(&cwd) {
            roots.push(root.clone());
            roots.push(root.join("src"));
        }
    }
    //Add home directory
    if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(home));
    }
    roots
}


//File Search Engine: locate files by direct path or basename across search roots
fn locate_matches(basename_or_path: &str) -> Vec<(PathBuf, u64)> {
    let mut out: Vec<(PathBuf, u64)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Path-like: expand directly
    let looks_like_path = basename_or_path.contains(std::path::MAIN_SEPARATOR)
        || basename_or_path.starts_with('.')
        || basename_or_path.starts_with('~');
    if looks_like_path {
        let p = expand_path(basename_or_path);
        if p.exists() && p.is_file() {
            if let Ok(meta) = fs::metadata(&p) {
                if let Ok(canon) = p.canonicalize() {
                    out.push((canon, meta.len()));
                    return out;
                } else {
                    out.push((p, meta.len()));
                    return out;
                }
            }
        }
        return out;
    }

    // Basename search across roots with exclusions in non-user related dirs
    let default_exclusions = vec![
        ".git", "node_modules", "target", "dist", "build", ".cache",
    ];
    let mut exclusions: HashSet<String> = HashSet::new();
    if let Ok(val) = env::var("RUSTLINE_LOCATE_EXCLUDE") {
        for part in val.split(',') {
            let s = part.trim();
            if !s.is_empty() { exclusions.insert(s.to_string()); }
        }
    } else {
        for d in default_exclusions { exclusions.insert(d.to_string()); }
    }
    // recursively walk each root
    for root in search_roots() {
        if !root.exists() { continue; }
        for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_dir() {
                if let Some(seg) = entry.path().file_name().and_then(|n| n.to_str()) {
                    if exclusions.contains(&seg.to_string()) { continue; }
                }
            }
            if entry.file_type().is_file() {
                if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                    let mut matched = false;
                    if name.eq_ignore_ascii_case(basename_or_path) { matched = true; }
                    if !matched {
                        if let Some(stem) = entry.path().file_stem().and_then(|s| s.to_str()) {
                            if stem.eq_ignore_ascii_case(basename_or_path) { matched = true; }
                        }
                    }
                    if matched {
                        // skip excluded path segments
                        if entry.path().components().any(|c| {
                            use std::path::Component;
                            match c {
                                Component::Normal(os) => os.to_str().map(|s| exclusions.contains(&s.to_string())).unwrap_or(false),
                                _ => false,
                            }
                        }) { continue; }
                        if let Ok(meta) = fs::metadata(entry.path()) {
                            let canonical = match entry.path().canonicalize() {
                                Ok(c) => c.to_string_lossy().to_string(),
                                Err(_) => entry.path().to_string_lossy().to_string(),
                            };
                            if seen.insert(canonical.clone()) {
                                out.push((PathBuf::from(canonical), meta.len()));
                            }
                        }
                    }
                }
            }
        }
    }
    out
}
//Resolve input to a PathBuf by direct path or first locate match
fn resolve_to_path(input: &str) -> Option<PathBuf> {
    // First try direct path
    let p = expand_path(input);
    if p.exists() && p.is_file() { return Some(p.canonicalize().unwrap_or(p)); }
    // Else locate first match
    locate_matches(input).into_iter().map(|(pb, _)| pb).next()
}
//Helper function to find the project root by looking for Cargo.toml
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
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
pub fn read_and_truncate(path: &Path) -> ToolResult {
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
    vec![
        Box::new(TimeTool),
        Box::new(EchoTool),
        Box::new(ReadFileTool),
        Box::new(OpenWithTool),
        Box::new(LocateTool),
        Box::new(CreateFileTool),
        Box::new(DeleteFileTool),
    ]
}

/// Locate tool: searches configured roots for files matching a basename and returns a JSON array
/// of matches: [{"path": "...", "size": 123}, ...]
pub struct LocateTool;

impl Tool for LocateTool {
    fn name(&self) -> &str {
        "locate"
    }

    fn description(&self) -> &str {
        "Locate files by basename in configured roots. Usage: !locate <filename>"
    }

    fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();
        if input.is_empty() {
            return Ok("Usage: !locate <filename>".to_string());
        }

        let matches = locate_matches(input);
        let results: Vec<serde_json::Value> = matches
            .into_iter()
            .map(|(pb, size)| json!({"path": pb.to_string_lossy().to_string(), "size": size}))
            .collect();
        Ok(serde_json::to_string(&results)?)
    }
}

/// Open file tool: opens a file in a native application.
/// Usage:
///  - `!open_file <path>` opens with the system default application.
///  - On macOS: `!open_file -a <AppName> <path>` opens with specified application (e.g., Notes).
// ═══════════════════════════════════════════════════════════════════════════
pub struct OpenWithTool;

impl Tool for OpenWithTool {
    fn name(&self) -> &str {
        "open_file"
    }

    fn description(&self) -> &str {
        "Open a file with the system default app, or on macOS use -a <AppName> to choose an app. Usage: !open_file [-a AppName] <path>"
    }

    fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();
        if input.is_empty() {
            return Ok("Usage: !open_file [-a AppName] <path>".to_string());
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
                        return Ok("Usage: !open_file -a <AppName> <path>".to_string());
                }
            }
        }

        // If no -a prefix was used, path_str remains input
        if path_str.is_empty() {
            return Ok("Usage: !open_file [-a AppName] <path>".to_string());
        }

        // Resolve by direct path or locate first match
        let p = match resolve_to_path(&path_str) {
            Some(pb) => pb,
            None => return Err(format!("Path not found: {}", expand_path(&path_str).display()).into()),
        };

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

// ═══════════════════════════════════════════════════════════════════════════
/// Create file tool: creates a file at the given path (expands ~ and relative paths)
/// and optionally writes provided content. Returns single-line JSON.
/// Usage:
///  - `!create_file <path>`
///  - `!create_file <path> --content <text>`
// ═══════════════════════════════════════════════════════════════════════════
pub struct CreateFileTool;

impl Tool for CreateFileTool {
    fn name(&self) -> &str { "create_file" }

    fn description(&self) -> &str {
        "Create a file at the given path. If no path is provided, saves into ./rustline_temp. Usage: !create_file [<path>] [--content <text>]"
    }

    fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();

        // Parse args: support `--content` followed by the remainder as text
        let mut path_part: String;
        let mut content_part: Option<String> = None;

        // Simple parser: support both forms:
        // 1) "<path> --content <text>"
        // 2) "--content <text>" (no path provided)
        if let Some(idx) = input.find(" --content ") {
            path_part = input[..idx].trim().to_string();
            let rest = input[idx + " --content ".len()..].trim();
            if !rest.is_empty() {
                content_part = Some(rest.to_string());
            }
        } else if let Some(rest) = input.strip_prefix("--content ") {
            // No path provided, only content
            path_part = String::new();
            let txt = rest.trim();
            if !txt.is_empty() {
                content_part = Some(txt.to_string());
            }
        } else if input == "--content" {
            // No path and no content text
            path_part = String::new();
            content_part = Some(String::new());
        } else {
            path_part = input.to_string();
        }

        // Default behavior: if no path provided, create in ./rustline_temp with an auto-generated name
        if path_part.is_empty() {
            let base = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let temp_dir = base.join("rustline_temp");
            if !temp_dir.exists() {
                fs::create_dir_all(&temp_dir)?;
            }
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            path_part = temp_dir.join(format!("untitled_{}.txt", ts)).to_string_lossy().to_string();
        }

        let p = expand_path(&path_part);
        if let Some(parent) = p.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        // Protection: if file already exists, do not overwrite. Return a clear message.
        if p.exists() {
            let obj = json!({
                "path": p.canonicalize()?.to_string_lossy().to_string(),
                "created": false,
                "exists": true,
                "message": "File already exists, try a different file name"
            });
            return Ok(serde_json::to_string(&obj)?);
        }

        match fs::File::create(&p) {
            Ok(_) => {
                if let Some(text) = content_part {
                    fs::write(&p, text.as_bytes())?;
                }
                let size = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                let obj = json!({
                    "path": p.canonicalize()?.to_string_lossy().to_string(),
                    "created": true,
                    "size": size
                });
                Ok(serde_json::to_string(&obj)?)
            }
            Err(e) => Err(Box::new(e)),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
/// Delete file tool: deletes a file at the given path. Returns single-line JSON.
/// Usage: `!delete_file <path>`
/// ═══════════════════════════════════════════════════════════════════════════
pub struct DeleteFileTool;

impl Tool for DeleteFileTool {
    fn name(&self) -> &str { "delete_file" }

    fn description(&self) -> &str {
        "Delete a file at the given path. Usage: !delete_file <path>"
    }

    fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();
        if input.is_empty() {
            return Ok("Usage: !delete_file <path>".to_string());
        }

        let p = match resolve_to_path(input) { Some(pb) => pb, None => expand_path(input) };
        if !p.exists() {
            let obj = json!({
                "path": p.to_string_lossy().to_string(),
                "deleted": false,
                "message": "File not found"
            });
            return Ok(serde_json::to_string(&obj)?);
        }

        if p.is_dir() {
            let obj = json!({
                "path": p.to_string_lossy().to_string(),
                "deleted": false,
                "message": "Path is a directory; delete_file only handles files"
            });
            return Ok(serde_json::to_string(&obj)?);
        }

        match fs::remove_file(&p) {
            Ok(_) => {
                let obj = json!({
                    "path": p.to_string_lossy().to_string(),
                    "deleted": true
                });
                Ok(serde_json::to_string(&obj)?)
            }
            Err(e) => Err(Box::new(e)),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Web fetch tool: fetches a URL and returns extracted content.
// ═══════════════════════════════════════════════════════════════════════════
pub struct WebFetchTool {}

// This tool fetches a web page by URL and returns a JSON object with title, text snippet, status, and size.
impl Tool for WebFetchTool {
    fn name(&self) -> &str { "web_fetch" }
    fn description(&self) -> &str { "Fetch a web page by URL and return title + text snippet as JSON" }
    fn invoke(&self, input: &str) -> Result<String, Box<dyn std::error::Error>> {
        let url = input.trim();
        if url.is_empty() { return Ok("{\"error\":\"URL required\"}".to_string()); }
        // Basic URL check
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Ok(format!("{{\"error\":\"Invalid URL: {}\"}}", url));
        }

        // Fetch the URL using user agent and timeout in 10 seconds
        let agent = ureq::AgentBuilder::new()
            .user_agent("rustline-web-fetch/1.0")
            .timeout(std::time::Duration::from_secs(10))
            .build();
        let resp_result = agent.get(url).call();
        let (status, body) = match resp_result {
            Ok(resp) => {
                let s = resp.status();
                let b = resp.into_string().unwrap_or_default();
                (s, b)
            }
            Err(e) => {
                // Map network/HTTP error to JSON message
                return Ok(format!("{{\"url\":\"{}\",\"status\":0,\"error\":\"{}\"}}", url, e));
            }
        };

        // Try to extract title and main text using scraper 
        let mut title = String::new();
        let mut text = String::new();
        let html = scraper::Html::parse_document(&body);
                let sel_title = scraper::Selector::parse("title").ok();
                if let Some(sel) = sel_title {
            if let Some(el) = html.select(&sel).next() { title = el.text().collect::<String>().trim().to_string(); }
                }
                let sel_p = scraper::Selector::parse("p").ok();
                if let Some(sel) = sel_p {
                    let mut acc = String::new();
                    for el in html.select(&sel).take(20) {
                        let t = el.text().collect::<String>();
                        if !t.trim().is_empty() {
                            if acc.len() + t.len() > 4000 { break; }
                            acc.push_str(t.trim());
                            acc.push_str("\n");
                        }
                    }
                    text = acc;
                }
        

        // Fallback: plain text if no HTML parse
        if title.is_empty() && text.is_empty() {
            text = body.chars().take(4000).collect();
        }

        let size = body.len();
        let json = serde_json::json!({
            "url": url,
            "status": status,
            "title": title,
            "text": text,
            "size": size,
        });
        Ok(json.to_string())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
/// Web summary tool: fetches a URL and returns a brief summary (title + key sentences) as JSON.
/// ═══════════════════════════════════════════════════════════════════════════
pub struct WebSummaryTool {}

impl Tool for WebSummaryTool {
    fn name(&self) -> &str { "web_summary" }
    fn description(&self) -> &str { "Fetch a URL and return a brief summary (title + key sentences) as JSON" }
    fn invoke(&self, input: &str) -> Result<String, Box<dyn std::error::Error>> {
        let url = input.trim();
        if url.is_empty() { return Ok("{\"error\":\"URL required\"}".to_string()); }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Ok(format!("{{\"error\":\"Invalid URL: {}\"}}", url));
        }
        // Use WebFetchTool to get the page content
        let fetch = WebFetchTool {};
        let res = fetch.invoke(url)?;
        let v: serde_json::Value = serde_json::from_str(&res).unwrap_or(serde_json::json!({"url": url, "status": 0, "title": "", "text": ""}));
        let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
        let status = v.get("status").and_then(|x| x.as_u64()).unwrap_or(0) as u16;

        let sentences: Vec<&str> = text
            .split(|c| c == '.' || c == '!' || c == '?')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        let mut summary_parts = Vec::new();
        for s in sentences.iter().take(5) { summary_parts.push(*s); }
        let mut summary = if summary_parts.is_empty() { text.lines().take(5).collect::<Vec<_>>().join(" ") } else { summary_parts.join(". ") };
        // Cap summary length to ~600 chars for TUI readability
        const MAX_SUMMARY: usize = 600;
        if summary.len() > MAX_SUMMARY {
            summary = format!("{}…", &summary[..MAX_SUMMARY]);
        }

        let json = serde_json::json!({
            "url": url,
            "status": status,
            "title": title,
            "summary": summary,
        });
        Ok(json.to_string())
    }
}


// ═══════════════════════════════════════════════════════════════════════════
// All built-in tools available to the agent.
// ═══════════════════════════════════════════════════════════════════════════
pub fn default_tools() -> Vec<DynTool> {
    vec![
        Box::new(TimeTool),
        Box::new(EchoTool),
        Box::new(ReadFileTool),
        Box::new(OpenWithTool),
        Box::new(LocateTool),
        Box::new(CreateFileTool),
        Box::new(DeleteFileTool),
        Box::new(WebFetchTool {}),
        Box::new(WebSummaryTool {}),
    ]
}




// ═══════════════════════════════════════════════════════════════════════════
// Unit tests for tools
// ═══════════════════════════════════════════════════════════════════════════
