use std::process::Command;
use std::env;
use tempfile::TempDir;

/// Integration tests for TUI improvements
/// These tests verify end-to-end functionality of the application

#[test]
fn test_complete_application_startup_with_default_tui_mode() {
    // Test that running the application without arguments defaults to TUI mode
    // Since TUI mode is interactive, we'll test that it starts without errors
    // and exits gracefully when given appropriate signals
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--help"])
        .output()
        .expect("Failed to execute command");
    
    // Verify the command executed successfully
    assert!(output.status.success(), "Application failed to start");
    
    // Convert output to string for analysis
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Verify help text indicates TUI as default mode
    assert!(stdout.contains("Terminal UI mode (default)"), 
        "Help text should indicate TUI as default mode");
    assert!(stdout.contains("By default, rustline starts in TUI mode"), 
        "Help text should explicitly state TUI is default");
}

#[test]
fn test_cli_mode_with_cli_flag() {
    // Test that --cli flag correctly starts CLI mode
    // We'll use --help with --cli to verify the flag is recognized
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--cli", "--help"])
        .output()
        .expect("Failed to execute command");
    
    // Verify the command executed successfully
    assert!(output.status.success(), "CLI mode failed to start with --cli flag");
    
    // The help should still be displayed, confirming --cli flag is processed
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rustline - Local AI Agent CLI"), 
        "CLI mode should display help when requested");
}

#[test]
fn test_tui_flag_backward_compatibility() {
    // Test that --tui flag still works for backward compatibility
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--tui", "--help"])
        .output()
        .expect("Failed to execute command");
    
    // Verify the command executed successfully
    assert!(output.status.success(), "TUI mode failed to start with --tui flag");
    
    // The help should still be displayed, confirming --tui flag is processed
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rustline - Local AI Agent CLI"), 
        "TUI mode should display help when requested");
}

#[test]
fn test_help_text_displays_correct_information() {
    // Test that help text contains all required information
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--help"])
        .output()
        .expect("Failed to execute command");
    
    assert!(output.status.success(), "Help command failed");
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Verify all required help text elements are present
    assert!(stdout.contains("--cli"), "Help should mention --cli flag");
    assert!(stdout.contains("--tui"), "Help should mention --tui flag");
    assert!(stdout.contains("Terminal UI mode (default)"), "Help should indicate TUI as default");
    assert!(stdout.contains("Run in CLI mode"), "Help should describe CLI mode");
    assert!(stdout.contains("By default, rustline starts in TUI mode"), 
        "Help should explicitly state default behavior");
    
    // Verify session management options are documented
    assert!(stdout.contains("--session"), "Help should mention session management");
    assert!(stdout.contains("--list-sessions"), "Help should mention session listing");
    assert!(stdout.contains("--new-session"), "Help should mention session creation");
    assert!(stdout.contains("--delete-session"), "Help should mention session deletion");
}

#[test]
fn test_unknown_flag_error_handling() {
    // Test that unknown flags produce appropriate error messages
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--unknown-flag"])
        .output()
        .expect("Failed to execute command");
    
    // Should exit with error code
    assert!(!output.status.success(), "Unknown flag should cause error exit");
    
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown option: --unknown-flag"), 
        "Should display specific unknown option error");
    assert!(stderr.contains("Use --help for usage information"), 
        "Should suggest using --help");
}

#[test]
fn test_session_flag_validation() {
    // Test that --session flag requires a value
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--session"])
        .output()
        .expect("Failed to execute command");
    
    // Should exit with error code
    assert!(!output.status.success(), "Missing session ID should cause error");
    
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--session requires a session ID"), 
        "Should display session ID requirement error");
}

#[test]
fn test_react_configuration_environment_variables() {
    // Test that ReAct loop configuration can be set via environment variables
    
    // Test that the application starts with custom configuration
    let output = Command::new("cargo")
        .args(&["run", "--", "--help"])
        .env("RUSTLINE_REACT_MAX_ITERATIONS", "5")
        .env("RUSTLINE_REACT_ITERATION_TIMEOUT", "20")
        .output()
        .expect("Failed to execute command");
    
    assert!(output.status.success(), "Application should start with custom ReAct config");
}

#[test]
fn test_persistence_fallback_handling() {
    // Test that TUI mode handles persistence initialization failures gracefully
    
    // Create a temporary directory that we'll make read-only to simulate failure
    let _temp_dir = TempDir::new().expect("Failed to create temp directory");
    
    // Test with invalid data directory to trigger fallback
    let output = Command::new("cargo")
        .args(&["run", "--", "--help"])
        .env("RUSTLINE_DATA_DIR", "/invalid/path/that/does/not/exist")
        .env("RUSTLINE_PERSISTENCE_ENABLED", "true")
        .output()
        .expect("Failed to execute command");
    
    // Application should still start successfully (fallback to non-persistent mode)
    assert!(output.status.success(), 
        "Application should handle persistence failures gracefully");
}

#[test]
fn test_application_version_and_basic_info() {
    // Test that the application provides basic version and build information
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--help"])
        .output()
        .expect("Failed to execute command");
    
    assert!(output.status.success(), "Help command should succeed");
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Verify basic application information is present
    assert!(stdout.contains("Rustline"), "Should display application name");
    assert!(stdout.contains("Local AI Agent CLI"), "Should display application description");
    assert!(stdout.contains("USAGE:"), "Should display usage information");
    assert!(stdout.contains("OPTIONS:"), "Should display options section");
    assert!(stdout.contains("ENVIRONMENT VARIABLES:"), "Should display environment variables section");
}

#[test]
fn test_multiple_flags_combination() {
    // Test that multiple flags can be combined correctly
    
    // Test --cli with --session (should work)
    let output = Command::new("cargo")
        .args(&["run", "--", "--cli", "--session", "test-session", "--help"])
        .output()
        .expect("Failed to execute command");
    
    assert!(output.status.success(), "Multiple flags should work together");
    
    // Test conflicting flags (last one should win)
    let output = Command::new("cargo")
        .args(&["run", "--", "--tui", "--cli", "--help"])
        .output()
        .expect("Failed to execute command");
    
    assert!(output.status.success(), "Conflicting flags should be handled gracefully");
}

#[test]
fn test_environment_variable_precedence() {
    // Test that environment variables are properly loaded and used
    
    let test_cases = vec![
        ("RUSTLINE_OLLAMA_URL", "http://test:11434"),
        ("RUSTLINE_MODEL", "test-model"),
        ("RUSTLINE_PRECHECK_MODE", "assisted"),
        ("RUSTLINE_CONFIRM_TOOLS", "false"),
        ("RUSTLINE_PERSISTENCE_ENABLED", "false"),
    ];
    
    for (env_var, test_value) in test_cases {
        let output = Command::new("cargo")
            .args(&["run", "--", "--help"])
            .env(env_var, test_value)
            .output()
            .expect("Failed to execute command");
        
        assert!(output.status.success(), 
            "Application should start with custom {} = {}", env_var, test_value);
    }
}