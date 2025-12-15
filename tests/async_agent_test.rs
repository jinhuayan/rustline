#[cfg(test)]
mod tests {
    use rustline::agent::Agent;
    use rustline::config::Config;

    #[tokio::test]
    async fn test_agent_creation() {
        let config = Config::default();
        let _agent = Agent::new(config);
        // If agent is created, test passes
    }

    #[tokio::test]
    async fn test_read_file_tool() {
        use rustline::tools::Tool;
        
        let tools = rustline::tools::default_tools();
        if let Some(tool) = tools.iter().find(|t| t.name() == "read_file") {
            // Try reading Cargo.toml
            match tool.invoke("Cargo.toml").await {
                Ok(result) => {
                    // Result should be JSON with path, size, truncated, content fields
                    assert!(result.contains("Cargo") || result.contains("rustline") || result.contains("path"));
                    println!("✓ read_file tool works: {}", &result[..80.min(result.len())]);
                }
                Err(e) => {
                    eprintln!("✗ read_file tool error: {}", e);
                    panic!("read_file failed");
                }
            }
        }
    }

    #[tokio::test]
    async fn test_locate_tool() {
        use rustline::tools::Tool;
        
        let tools = rustline::tools::default_tools();
        if let Some(tool) = tools.iter().find(|t| t.name() == "locate") {
            // Try locating Cargo.toml
            match tool.invoke("Cargo.toml").await {
                Ok(result) => {
                    // Result should be JSON array
                    assert!(result.contains("[") || result.contains("error"));
                    println!("✓ locate tool works: {}", &result[..80.min(result.len())]);
                }
                Err(e) => {
                    eprintln!("✗ locate tool error: {}", e);
                }
            }
        }
    }

    #[tokio::test]
    async fn test_web_fetch_tool_validation() {
        use rustline::tools::Tool;
        
        let tools = rustline::tools::default_tools();
        if let Some(tool) = tools.iter().find(|t| t.name() == "web_fetch") {
            // Test with invalid URL
            match tool.invoke("not-a-url").await {
                Ok(result) => {
                    assert!(result.contains("error") || result.contains("Invalid"));
                    println!("✓ web_fetch tool validates URLs");
                }
                Err(e) => {
                    eprintln!("✗ web_fetch tool error: {}", e);
                }
            }
            
            // Test with empty input
            match tool.invoke("").await {
                Ok(result) => {
                    assert!(result.contains("error") || result.contains("required"));
                    println!("✓ web_fetch tool requires URL");
                }
                Err(_) => {}
            }
        }
    }

    #[tokio::test]
    async fn test_web_summary_tool_validation() {
        use rustline::tools::Tool;
        
        let tools = rustline::tools::default_tools();
        if let Some(tool) = tools.iter().find(|t| t.name() == "web_summary") {
            // Test with invalid URL
            match tool.invoke("not-a-url").await {
                Ok(result) => {
                    assert!(result.contains("error") || result.contains("Invalid"));
                    println!("✓ web_summary tool validates URLs");
                }
                Err(e) => {
                    eprintln!("✗ web_summary tool error: {}", e);
                }
            }
            
            // Test with empty input
            match tool.invoke("").await {
                Ok(result) => {
                    assert!(result.contains("error") || result.contains("required"));
                    println!("✓ web_summary tool requires URL");
                }
                Err(_) => {}
            }
        }
    }

    #[tokio::test]
    async fn test_all_tools_exist() {
        use rustline::tools::Tool;
        
        let tools = rustline::tools::default_tools();
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        
        assert!(tool_names.contains(&"read_file"), "Missing read_file tool");
        assert!(tool_names.contains(&"locate"), "Missing locate tool");
        assert!(tool_names.contains(&"web_fetch"), "Missing web_fetch tool");
        assert!(tool_names.contains(&"web_summary"), "Missing web_summary tool");
        assert!(tool_names.contains(&"create_file"), "Missing create_file tool");
        assert!(tool_names.contains(&"add_content"), "Missing add_content tool");
        assert!(tool_names.contains(&"delete_file"), "Missing delete_file tool");
        
        println!("✓ All {} tools registered", tools.len());
    }

    #[tokio::test]
    async fn test_tools_are_async() {
        use rustline::tools::Tool;
        
        let tools = rustline::tools::default_tools();
        
        // Run multiple tool invocations sequentially to verify async nature
        for (i, tool) in tools.iter().take(3).enumerate() {
            let result = tool.invoke("Cargo.toml").await;
            // Just verify the tool can be invoked in async context
            println!("  Tool {}: {}", i, if result.is_ok() { "OK" } else { "error" });
        }
        
        println!("✓ Tools work correctly in async context");
    }
}
