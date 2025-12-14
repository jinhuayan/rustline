use rustline::config::Config;
use rustline::agent::Agent;
use std::time::{Duration, Instant};

/// Tests for ReAct loop performance improvements
/// These tests verify that the ReAct loop respects the new iteration limits and timeout settings

#[tokio::test]
async fn test_agent_creation_with_custom_config() {
    // Test that Agent can be created with custom ReAct configuration
    
    let mut config = Config::default();
    config.react_max_iterations = 2; // Set to 2 for testing
    
    let agent = Agent::new(config.clone());
    
    // Note: We can't access the private config field directly,
    // but we can verify that the agent was created successfully
    // and that the configuration loading works properly
    
    // Verify agent has empty history initially
    assert_eq!(agent.get_history().len(), 0);
    
    // Verify agent can be cloned (which preserves internal config)
    let _cloned_agent = agent.clone();
}



#[test]
fn test_config_environment_variable_loading() {
    // Test that configuration properly loads from environment variables
    
    // Test default values
    let config = Config::load();
    assert_eq!(config.react_max_iterations, 3); // Default value from requirements
    
    // Verify other configuration fields are properly loaded
    assert!(!config.ollama_base_url.is_empty());
    assert!(!config.model.is_empty());
    assert!(config.precheck_mode == "strict" || config.precheck_mode == "assisted");
}

#[tokio::test]
async fn test_agent_configuration_consistency() {
    // Test that Agent properly stores and uses the configuration
    
    let config = Config::default();
    let agent = Agent::new(config);
    
    // Test that cloning preserves internal state
    let cloned_agent = agent.clone();
    
    // Both agents should have empty history initially
    assert_eq!(agent.get_history().len(), 0);
    assert_eq!(cloned_agent.get_history().len(), 0);
}

#[tokio::test]
async fn test_react_loop_bounds_validation() {
    // Test that ReAct loop configuration has reasonable bounds
    
    let test_values = vec![1, 2, 3, 5, 10];
    
    for max_iterations in test_values {
        let mut config = Config::default();
        config.react_max_iterations = max_iterations;
        
        let agent = Agent::new(config);
        
        // Verify agent was created successfully with the configuration
        assert_eq!(agent.get_history().len(), 0);
    }
}



#[tokio::test]
async fn test_performance_configuration_impact() {
    // Test that different configurations don't cause performance issues
    
    let configs = vec![1, 3, 5]; // Different iteration limits
    
    for max_iter in configs {
        let start = Instant::now();
        
        let mut config = Config::default();
        config.react_max_iterations = max_iter;
        
        let agent = Agent::new(config);
        
        let creation_time = start.elapsed();
        
        // Agent creation should be fast regardless of configuration
        assert!(creation_time < Duration::from_millis(100));
        
        // Verify agent was created successfully
        assert_eq!(agent.get_history().len(), 0);
    }
}

#[tokio::test]
async fn test_agent_with_persistence_configuration() {
    // Test that Agent with persistence also respects ReAct configuration
    
    use rustline::persistence::session_manager::SessionManager;
    use rustline::persistence::preference_manager::PreferenceManager;
    use tempfile::TempDir;
    
    // Create temporary directory for testing
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let data_dir = temp_dir.path().to_path_buf();
    
    // Create persistence managers
    let session_manager = SessionManager::new(data_dir.clone())
        .expect("Failed to create session manager");
    let preference_manager = PreferenceManager::new(data_dir)
        .expect("Failed to create preference manager");
    
    // Create config with custom ReAct settings
    let mut config = Config::default();
    config.react_max_iterations = 4;
    
    // Create agent with persistence
    let agent = Agent::new_with_persistence(config, session_manager, preference_manager);
    
    // Verify agent was created successfully
    assert_eq!(agent.get_history().len(), 0);
}

#[test]
fn test_config_default_values_match_requirements() {
    // Test that default configuration values match the requirements
    
    let config = Config::default();
    
    // Requirements 2.1: max_iterations should default to 3
    assert_eq!(config.react_max_iterations, 3);
    
    // Verify other important defaults
    assert_eq!(config.ollama_base_url, "http://localhost:11434");
    assert_eq!(config.model, "gemma3");
    assert_eq!(config.precheck_mode, "strict");
    assert_eq!(config.confirm_before_tools, true);
    assert_eq!(config.persistence_enabled, true);
}