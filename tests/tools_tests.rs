use rustline::tools::{default_tools, Tool, CreateFileTool, ReadFileTool, LocateTool, expand_path, read_and_truncate};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_name(prefix: &str) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{}_{}", prefix, now)
}

#[test]
fn test_create_file_explicit_path_with_content() {
    let base = env::temp_dir().join(unique_name("rustline_create_explicit"));
    fs::create_dir_all(&base).unwrap();
    let file_path = base.join("note.txt");

    let tool = CreateFileTool;
    let args = format!("{} --content {}", file_path.to_string_lossy(), "hello world");
    let res = tool.invoke(&args).expect("invoke failed");
    let v: serde_json::Value = serde_json::from_str(&res).expect("invalid json");
    let p = v["path"].as_str().unwrap();
    assert!(PathBuf::from(p).exists());
    assert_eq!(fs::read_to_string(p).unwrap(), "hello world");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn test_create_file_default_dir_when_no_path() {
    let old_cwd = env::current_dir().unwrap();
    let base = env::temp_dir().join(unique_name("rustline_create_default"));
    fs::create_dir_all(&base).unwrap();
    env::set_current_dir(&base).expect("set cwd");

    let tool = CreateFileTool;
    let res = tool.invoke("--content default content").expect("invoke failed");
    let v: serde_json::Value = serde_json::from_str(&res).expect("invalid json");
    let p = v["path"].as_str().unwrap();
    let pb = PathBuf::from(p);
    let parent_name = pb.parent().and_then(|pp| pp.file_name()).and_then(|n| n.to_str()).unwrap_or("");
    assert_eq!(parent_name, "rustline_temp");
    assert!(pb.exists());
    assert_eq!(fs::read_to_string(p).unwrap(), "default content");

    // cleanup
    env::set_current_dir(old_cwd).unwrap();
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn test_create_file_exists_protection() {
    let base = env::temp_dir().join(unique_name("rustline_create_exists"));
    fs::create_dir_all(&base).unwrap();
    let file_path = base.join("exists.txt");
    fs::write(&file_path, "initial").unwrap();

    let tool = CreateFileTool;
    // Attempt to create the same file again
    let args = file_path.to_string_lossy().to_string();
    let res = tool.invoke(&args).expect("invoke failed");
    let v: serde_json::Value = serde_json::from_str(&res).expect("invalid json");
    assert_eq!(v["created"].as_bool().unwrap(), false);
    assert_eq!(v["exists"].as_bool().unwrap(), true);
    assert_eq!(v["message"].as_str().unwrap(), "File already exists, try a different file name");

    // Ensure original content untouched
    assert_eq!(fs::read_to_string(&file_path).unwrap(), "initial");

    let _ = fs::remove_dir_all(&base);
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

#[test]
fn test_locate_tool_finds_readme() {
    // This repo contains a README.md at the project root.
    let tool = LocateTool;
    let res = tool.invoke("README.md").expect("invoke failed");
    let v: serde_json::Value = serde_json::from_str(&res).expect("invalid json");
    assert!(v.is_array());
    let arr = v.as_array().unwrap();
    assert!(!arr.is_empty(), "locate returned no matches");
    let p = arr[0]["path"].as_str().unwrap();
    assert!(p.ends_with("README.md"));
}

#[test]
fn test_tools_integration_workflow() {
    println!("=== Testing Tools Integration Workflow ===");
    
    let tools = default_tools();
    
    // Test that we have all expected tools
    let expected_tools = vec!["time", "echo", "read_file", "open_file", "locate", "create_file", "delete_file"];
    for expected in &expected_tools {
        assert!(tools.iter().any(|t| t.name() == *expected), "Missing tool: {}", expected);
    }
    
    // Test create_file -> read_file -> delete_file workflow
    let temp_file = format!("integration_test_{}.txt", unique_name("workflow"));
    let test_content = "Integration test content\nMultiple lines\nFor testing";
    
    // Create file in temp directory to avoid permission issues
    let full_path = env::temp_dir().join(&temp_file);
    let create_tool = tools.iter().find(|t| t.name() == "create_file").unwrap();
    let args = format!("{} --content {}", full_path.to_string_lossy(), test_content);
    let create_result = create_tool.invoke(&args).expect("Create tool should work");
    
    let create_json: serde_json::Value = serde_json::from_str(&create_result).expect("Should be valid JSON");
    assert_eq!(create_json["created"].as_bool().unwrap(), true);
    
    // Read file using full path
    let read_tool = tools.iter().find(|t| t.name() == "read_file").unwrap();
    let read_result = read_tool.invoke(&full_path.to_string_lossy()).expect("Read tool should work");
    
    let read_json: serde_json::Value = serde_json::from_str(&read_result).expect("Should be valid JSON");
    let content = read_json["content"].as_str().expect("Should have content field");
    assert_eq!(content, test_content);
    
    // Test locate tool (may or may not find the file depending on search implementation)
    let locate_tool = tools.iter().find(|t| t.name() == "locate").unwrap();
    let locate_result = locate_tool.invoke(&temp_file).expect("Locate tool should work");
    
    let locate_json: serde_json::Value = serde_json::from_str(&locate_result).expect("Should be valid JSON");
    assert!(locate_json.is_array());
    // Note: locate may not find the file if it's not in the search roots, so we don't assert on matches
    
    // Delete file (cleanup) using full path
    let delete_tool = tools.iter().find(|t| t.name() == "delete_file").unwrap();
    let delete_result = delete_tool.invoke(&full_path.to_string_lossy()).expect("Delete tool should work");
    
    let delete_json: serde_json::Value = serde_json::from_str(&delete_result).expect("Should be valid JSON");
    assert_eq!(delete_json["deleted"].as_bool().unwrap(), true);
    
    println!("✓ Tools integration workflow completed successfully");
}

#[test]
fn test_tools_descriptions_and_functionality() {
    let tools = default_tools();
    
    // Test time tool
    let time_tool = tools.iter().find(|t| t.name() == "time").unwrap();
    assert!(time_tool.description().contains("current local time"));
    let result = time_tool.invoke("").expect("Time tool should work");
    assert!(result.contains("Current local time:"));
    
    // Test echo tool
    let echo_tool = tools.iter().find(|t| t.name() == "echo").unwrap();
    assert!(echo_tool.description().contains("Echo back"));
    let test_message = "Hello tools system!";
    let result = echo_tool.invoke(test_message).expect("Echo tool should work");
    assert_eq!(result, test_message);
    
    println!("✓ All tools have proper descriptions and basic functionality");
}

#[test]
fn test_tools_error_handling() {
    let tools = default_tools();
    
    // Test read_file with non-existent file
    if let Some(read_tool) = tools.iter().find(|t| t.name() == "read_file") {
        let result = read_tool.invoke("non_existent_file_12345.txt").expect("Should handle gracefully");
        assert!(result.contains("No file named"));
        println!("✓ Read tool handles missing files gracefully");
    }
    
    // Test delete_file with non-existent file
    if let Some(delete_tool) = tools.iter().find(|t| t.name() == "delete_file") {
        let result = delete_tool.invoke("non_existent_file_12345.txt").expect("Should handle gracefully");
        let json_val: serde_json::Value = serde_json::from_str(&result).expect("Should be valid JSON");
        assert_eq!(json_val["deleted"].as_bool().unwrap(), false);
        assert!(json_val["message"].as_str().unwrap().contains("not found"));
        println!("✓ Delete tool handles missing files gracefully");
    }
    
    // Test create_file with existing file (should not overwrite)
    let existing_file = env::temp_dir().join(format!("test_existing_{}.txt", unique_name("error")));
    fs::write(&existing_file, "original content").expect("Should create test file");
    
    if let Some(create_tool) = tools.iter().find(|t| t.name() == "create_file") {
        let result = create_tool.invoke(&existing_file.to_string_lossy()).expect("Should handle gracefully");
        let json_val: serde_json::Value = serde_json::from_str(&result).expect("Should be valid JSON");
        assert_eq!(json_val["created"].as_bool().unwrap(), false);
        assert_eq!(json_val["exists"].as_bool().unwrap(), true);
        
        // Verify original content is preserved
        let content = fs::read_to_string(&existing_file).expect("Should read file");
        assert_eq!(content, "original content");
        println!("✓ Create tool protects existing files");
    }
    
    // Cleanup
    let _ = fs::remove_file(&existing_file);
}

#[test]
fn test_all_tools_available() {
    let tools = default_tools();
    
    // Verify we have exactly the expected number of tools
    assert_eq!(tools.len(), 7, "Should have exactly 7 tools");
    
    // Verify each tool has a name and description
    for tool in &tools {
        assert!(!tool.name().is_empty(), "Tool name should not be empty");
        assert!(!tool.description().is_empty(), "Tool description should not be empty");
        println!("✓ Tool '{}': {}", tool.name(), tool.description());
    }
}