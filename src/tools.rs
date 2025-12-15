use std::fs;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use walkdir::WalkDir;
use std::collections::HashSet;
use serde_json::json;
use reqwest::Client;
use async_trait::async_trait;

pub type ToolResult = Result<String, Box<dyn std::error::Error + Send + Sync>>;

// ═══════════════════════════════════════════════════════════════════════════
// Tools for Rustline Agent
// ═══════════════════════════════════════════════════════════════════════════

// Common interface for all tools. Every tool must follow this trait.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn invoke(&self, args: &str) -> ToolResult;
}

pub type DynTool = Box<dyn Tool>;


// ═══════════════════════════════════════════════════════════════════════════
// Read file tool: reads and returns the contents of a file.
// Usage: `!read_file <path>` or called by the agent.
// ═══════════════════════════════════════════════════════════════════════════
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file's contents. Usage: !read_file <path> (returns truncated content if large)"
    }

    async fn invoke(&self, args: &str) -> ToolResult {
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


// ═══════════════════════════════════════════════════════════════════════════
// Locate tool: searches configured roots for files matching a basename and returns a JSON array
// of matches: [{"path": "...", "size": 123}, ...]
// ═══════════════════════════════════════════════════════════════════════════
pub struct LocateTool;

#[async_trait]
impl Tool for LocateTool {
    fn name(&self) -> &str {
        "locate"
    }

    fn description(&self) -> &str {
        "Locate files by basename in configured roots. Usage: !locate <filename>"
    }

    async fn invoke(&self, args: &str) -> ToolResult {
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



// All built-in tools available to the agent.
pub fn default_tools() -> Vec<DynTool> {
    vec![
        Box::new(ReadFileTool),
        Box::new(OpenWithTool),
        Box::new(LocateTool),
        Box::new(CreateFileTool),
        Box::new(AddContentTool),
        Box::new(DeleteFileTool),
        Box::new(WebFetchTool {}),
        Box::new(WebSummaryTool {}),
    ]
}


// ═══════════════════════════════════════════════════════════════════════════
// Open file tool: opens a file in a native application.
// Usage:
//  - `!open_file <path>` opens with the system default application.
//  - On macOS: `!open_file -a <AppName> <path>` opens with specified application (e.g., Notes).
// ═══════════════════════════════════════════════════════════════════════════
pub struct OpenWithTool;

#[async_trait]
impl Tool for OpenWithTool {
    fn name(&self) -> &str {
        "open_file"
    }

    fn description(&self) -> &str {
        "Open a file with the system default app, or on macOS use -a <AppName> to choose an app. Usage: !open_file [-a AppName] <path>"
    }

    async fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();
        if input.is_empty() {
            return Ok("Usage: !open_file [-a AppName] <path>".to_string());
        }

        // parse optional macOS -a AppName (only on macOS)
        #[cfg(target_os = "macos")]
        let mut app: Option<String> = None;
        #[cfg(target_os = "macos")]
        let mut path_str = input.to_string();
        #[cfg(not(target_os = "macos"))]
        let path_str = input.to_string();

        #[cfg(target_os = "macos")]
        {
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
        }

        #[cfg(not(target_os = "macos"))]
        {
            // On non-macOS platforms, reject -a flag usage
            if input.trim_start().starts_with("-a ") {
                return Ok("The -a flag is only supported on macOS".to_string());
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
// Create file tool: creates an empty file at the given path (expands ~ and relative paths).
// Returns single-line JSON.
// Usage: `!create_file <path>`
// ═══════════════════════════════════════════════════════════════════════════
pub struct CreateFileTool;

#[async_trait]
impl Tool for CreateFileTool {
    fn name(&self) -> &str { "create_file" }

    fn description(&self) -> &str {
        "Create a file at the given path with optional content. If no path is provided, saves into ./rustline_temp. Usage: !create_file [<path>] [--content <text>]"
    }

    async fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();
        let mut path_part = String::new();
        let mut content = String::new();

        // Parse: "<path> --content <text>" or just "<path>" or just "--content <text>"
        if let Some(idx) = input.find(" --content") {
            path_part = input[..idx].trim().to_string();
            // Extract content after "--content"
            content = if idx + " --content".len() < input.len() {
                input[idx + " --content".len()..].trim_start().to_string()
            } else {
                String::new()
            };
        } else if input.starts_with("--content") {
            // Just "--content <text>" with no path
            content = if "--content".len() < input.len() {
                input["--content".len()..].trim_start().to_string()
            } else {
                String::new()
            };
        } else {
            // Just a path with no content
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

        match fs::write(&p, &content) {
            Ok(_) => {
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
// Add content tool: appends content to the end of an existing file.
// Returns single-line JSON.
// Usage: `!add_content <path> --content <text>`
// ═══════════════════════════════════════════════════════════════════════════
pub struct AddContentTool;

#[async_trait]
impl Tool for AddContentTool {
    fn name(&self) -> &str { "add_content" }

    fn description(&self) -> &str {
        "Append content to the end of a file. Usage: !add_content <path> --content <text>"
    }

    async fn invoke(&self, args: &str) -> ToolResult {
        let input = args.trim();
        
        // Parse: "<path> --content <text>" or "<path> --content" (with or without trailing content)
        if let Some(idx) = input.find(" --content") {
            let path_part = input[..idx].trim().to_string();
            // Extract content after "--content" (may be empty if nothing follows)
            let content = if idx + " --content".len() < input.len() {
                input[idx + " --content".len()..].trim_start().to_string()
            } else {
                String::new()
            };
            
            if path_part.is_empty() {
                return Ok(json!({
                    "error": "Path is required",
                    "usage": "!add_content <path> --content <text>"
                }).to_string());
            }
            
            let p = match resolve_to_path(&path_part) {
                Some(pb) => pb,
                None => expand_path(&path_part),
            };
            
            // File must exist
            if !p.exists() {
                let obj = json!({
                    "path": p.to_string_lossy().to_string(),
                    "success": false,
                    "message": "File not found. Create it first with !create_file"
                });
                return Ok(serde_json::to_string(&obj)?);
            }
            
            if p.is_dir() {
                let obj = json!({
                    "path": p.to_string_lossy().to_string(),
                    "success": false,
                    "message": "Path is a directory, not a file"
                });
                return Ok(serde_json::to_string(&obj)?);
            }
            
            // Append content to file (supports empty content)
            match fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&p) {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(content.as_bytes())?;
                    let size = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    let obj = json!({
                        "path": p.canonicalize().unwrap_or(p).to_string_lossy().to_string(),
                        "success": true,
                        "size": size
                    });
                    Ok(serde_json::to_string(&obj)?)
                }
                Err(e) => Err(Box::new(e)),
            }
        } else {
            Ok(json!({
                "error": "Invalid arguments",
                "usage": "!add_content <path> --content <text>"
            }).to_string())
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Delete file tool: deletes a file at the given path. Returns single-line JSON.
// Usage: `!delete_file <path>`
// ═══════════════════════════════════════════════════════════════════════════
pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
    fn name(&self) -> &str { "delete_file" }

    fn description(&self) -> &str {
        "Delete a file at the given path. Usage: !delete_file <path>"
    }

    async fn invoke(&self, args: &str) -> ToolResult {
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
#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str { "web_fetch" }
    fn description(&self) -> &str { "Fetch a web page by URL and return title + text snippet as JSON" }
    async fn invoke(&self, input: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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
// Web summary tool: fetches a URL and uses LLM to generate an intelligent summary.
// ═══════════════════════════════════════════════════════════════════════════
pub struct WebSummaryTool {}

#[async_trait]
impl Tool for WebSummaryTool {
    fn name(&self) -> &str { "web_summary" }
    fn description(&self) -> &str { "Fetch a URL and use LLM to generate an intelligent summary" }
    async fn invoke(&self, input: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = input.trim();
        if url.is_empty() { return Ok("{\"error\":\"URL required\"}".to_string()); }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Ok(format!("{{\"error\":\"Invalid URL: {}\"}}", url));
        }
        // Use WebFetchTool to get the page content
        let fetch = WebFetchTool {};
        let res = fetch.invoke(url).await?;
        let v: serde_json::Value = serde_json::from_str(&res).unwrap_or(serde_json::json!({"url": url, "status": 0, "title": "", "text": ""}));
        let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
        let status = v.get("status").and_then(|x| x.as_u64()).unwrap_or(0) as u16;

        // If fetch failed or no content, return simple response
        if status == 0 || text.is_empty() {
            return Ok(serde_json::json!({
                "url": url,
                "status": status,
                "title": title,
                "summary": "No content available to summarize",
            }).to_string());
        }

        // Use LLM to generate intelligent summary
        let client = Client::new();
        
        // Get config from environment or use defaults
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        // Prefer project default, then Ollama env, then sensible fallback
        let model = std::env::var("RUSTLINE_DEFAULT_MODEL")
            .or_else(|_| std::env::var("OLLAMA_MODEL"))
            .unwrap_or_else(|_| "gemma3".to_string());
        
        if std::env::var("RUSTLINE_DEBUG").is_ok() {
            eprintln!("[WebSummaryTool] Calling LLM at {} with model {}", base_url, model);
        }
        let prompt = format!(
            "Summarize the following web page content in 2-3 concise sentences (max 600 characters). \
            Focus on the main points and key information.\n\nTitle: {}\n\nContent:\n{}",
            title, text
        );
        
        let summary = match crate::ollama::chat_single_turn(&client, &base_url, &model, &prompt).await {
            Ok(llm_summary) => {
                if std::env::var("RUSTLINE_DEBUG").is_ok() {
                    eprintln!("[WebSummaryTool] LLM returned summary ({} chars)", llm_summary.len());
                }
                // Cap at 600 chars as specified
                const MAX_SUMMARY: usize = 600;
                if llm_summary.len() > MAX_SUMMARY {
                    format!("{}…", &llm_summary[..MAX_SUMMARY])
                } else {
                    llm_summary
                }
            }
            Err(e) => {
                // Surface the failure explicitly so callers/users know LLM was not used.
                if std::env::var("RUSTLINE_DEBUG").is_ok() {
                    eprintln!("[WebSummaryTool] LLM summarization failed: {}", e);
                }
                let sentences: Vec<&str> = text
                    .split(|c| c == '.' || c == '!' || c == '?')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .take(5)
                    .collect();
                let fallback = if sentences.is_empty() {
                    text.lines().take(5).collect::<Vec<_>>().join(" ")
                } else {
                    sentences.join(". ")
                };
                const MAX_SUMMARY: usize = 600;
                let trimmed = if fallback.len() > MAX_SUMMARY {
                    format!("{}…", &fallback[..MAX_SUMMARY])
                } else {
                    fallback
                };
                format!("[LLM failed: {}] Fallback summary: {}", e, trimmed)
            }
        };

        let json = serde_json::json!({
            "url": url,
            "status": status,
            "title": title,
            "summary": summary,
        });
        Ok(json.to_string())
    }
}

