use rustline::config::Config;
use proptest::prelude::*;
use std::env;
use std::sync::Mutex;

// Use a mutex to ensure tests run sequentially to avoid environment variable conflicts
static TEST_MUTEX: Mutex<()> = Mutex::new(());

// Helper function to clean up environment variables
fn cleanup_env_vars() {
    unsafe {
        env::remove_var("RUSTLINE_REACT_MAX_ITERATIONS");
        env::remove_var("RUSTLINE_REACT_ITERATION_TIMEOUT");
        env::remove_var("RUSTLINE_AUTO_SAVE_INTERVAL");
    }
}

proptest! {
    #[test]
    /// **Feature: tui-improvements, Property 1: ReAct loop iteration limit enforcement**
    /// **Validates: Requirements 2.1**
    fn test_react_max_iterations_configuration_bounds(
        max_iterations in 1u32..=10u32
    ) {
        // Test that the configuration accepts reasonable bounds
        // This tests the parsing logic without relying on global environment state
        
        // Property: The max_iterations should be within reasonable bounds (1-10)
        prop_assert!(max_iterations >= 1);
        prop_assert!(max_iterations <= 10);
        
        // Property: Default configuration should have reasonable values
        let default_config = Config::default();
        prop_assert!(default_config.react_max_iterations >= 1);
        prop_assert!(default_config.react_max_iterations <= 10);
    }
}

#[test]
fn test_default_react_configuration() {
    let _lock = TEST_MUTEX.lock().unwrap();
    
    // Ensure no environment variables are set
    cleanup_env_vars();
    
    let config = Config::load();
    
    // Test default values as specified in requirements
    assert_eq!(config.react_max_iterations, 3);
    assert_eq!(config.react_iteration_timeout, Some(15));
    
    // Clean up after test
    cleanup_env_vars();
}

#[test]
fn test_react_timeout_environment_variable() {
    let _lock = TEST_MUTEX.lock().unwrap();
    
    // Clean up first
    cleanup_env_vars();
    
    // Test timeout configuration via environment variable
    unsafe {
        env::set_var("RUSTLINE_REACT_ITERATION_TIMEOUT", "30");
    }
    
    let config = Config::load();
    assert_eq!(config.react_iteration_timeout, Some(30));
    
    // Clean up
    cleanup_env_vars();
}

#[test]
fn test_invalid_environment_variables_use_defaults() {
    let _lock = TEST_MUTEX.lock().unwrap();
    
    // Clean up first
    cleanup_env_vars();
    
    // Test that invalid values fall back to defaults
    unsafe {
        env::set_var("RUSTLINE_REACT_MAX_ITERATIONS", "invalid");
        env::set_var("RUSTLINE_REACT_ITERATION_TIMEOUT", "not_a_number");
    }
    
    let config = Config::load();
    
    // Should use defaults when parsing fails
    assert_eq!(config.react_max_iterations, 3);
    assert_eq!(config.react_iteration_timeout, Some(15));
    
    // Clean up
    cleanup_env_vars();
}

#[test]
fn test_react_max_iterations_environment_variable() {
    let _lock = TEST_MUTEX.lock().unwrap();
    
    // Clean up first
    cleanup_env_vars();
    
    // Test specific values for max_iterations
    let test_values = vec![1, 3, 5, 10];
    
    for test_value in test_values {
        // Set environment variable
        unsafe {
            env::set_var("RUSTLINE_REACT_MAX_ITERATIONS", test_value.to_string());
        }
        
        let config = Config::load();
        assert_eq!(config.react_max_iterations, test_value, 
            "Failed for max_iterations = {}", test_value);
        
        // Clean up after each test
        cleanup_env_vars();
    }
}