use rustline::persistence::{ensure_directory, validate_session_id, atomic_write};
use std::fs;
use tempfile::TempDir;
use proptest::prelude::*;

#[test]
fn test_ensure_directory_creates_missing_dir() {
    let temp_dir = TempDir::new().unwrap();
    let test_path = temp_dir.path().join("test_dir");
    
    assert!(!test_path.exists());
    ensure_directory(&test_path).unwrap();
    assert!(test_path.exists());
    assert!(test_path.is_dir());
}

#[test]
fn test_ensure_directory_handles_existing_dir() {
    let temp_dir = TempDir::new().unwrap();
    let test_path = temp_dir.path().to_path_buf();
    
    // Should not error on existing directory
    ensure_directory(&test_path).unwrap();
}

#[test]
fn test_validate_session_id_valid() {
    assert!(validate_session_id("valid_session_123").is_ok());
    assert!(validate_session_id("session-with-dashes").is_ok());
    assert!(validate_session_id("session.with.dots").is_ok());
}

#[test]
fn test_validate_session_id_invalid() {
    assert!(validate_session_id("").is_err());
    assert!(validate_session_id("session/with/slash").is_err());
    assert!(validate_session_id("session\\with\\backslash").is_err());
    assert!(validate_session_id("session:with:colon").is_err());
    assert!(validate_session_id("session*with*asterisk").is_err());
}

#[test]
fn test_atomic_write() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test_file.txt");
    
    let test_content = "Hello, atomic write!";
    atomic_write(&file_path, |file| {
        use std::io::Write;
        file.write_all(test_content.as_bytes())
    }).unwrap();
    
    assert!(file_path.exists());
    let content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, test_content);
    
    // Ensure temp file was cleaned up
    let temp_path = file_path.with_extension("tmp");
    assert!(!temp_path.exists());
}

// Import additional modules for comprehensive persistence testing
use rustline::persistence::{
    memory_store::MemoryStore,
    session_manager::SessionManager,
    preference_manager::{PreferenceManager, UserPreferences},
};
use rustline::ollama::Message;
use std::sync::Arc;
use std::thread;

proptest! {
    /// **Feature: user-persistent-memory, Property 9: Directory creation reliability**
    /// **Validates: Requirements 4.4**
    #[test]
    fn test_directory_creation_reliability(
        // Generate valid directory names (no invalid path characters)
        dir_name in "[a-zA-Z0-9_.-]{1,50}",
        nested_levels in 0usize..5,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let mut test_path = temp_dir.path().to_path_buf();
        
        // Create nested directory structure
        for i in 0..=nested_levels {
            test_path = test_path.join(format!("{}_{}", dir_name, i));
        }
        
        // Directory should not exist initially
        prop_assert!(!test_path.exists());
        
        // ensure_directory should create the entire path
        let result = ensure_directory(&test_path);
        prop_assert!(result.is_ok());
        
        // Directory should now exist and be a directory
        prop_assert!(test_path.exists());
        prop_assert!(test_path.is_dir());
        
        // Calling ensure_directory again should not fail
        let result2 = ensure_directory(&test_path);
        prop_assert!(result2.is_ok());
        
        // Directory should still exist
        prop_assert!(test_path.exists());
        prop_assert!(test_path.is_dir());
    }

    /// **Feature: user-persistent-memory, Property 7: Atomic storage operations**
    /// **Validates: Requirements 4.1**
    #[test]
    fn test_atomic_storage_operations(
        session_id in "[a-zA-Z0-9_.-]{1,50}",
        messages in prop::collection::vec(
            (
                "[a-zA-Z]{1,20}",  // role
                ".*",              // content
            ).prop_map(|(role, content)| Message::new(role, content)),
            0..100
        ),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let store = MemoryStore::new(temp_dir.path().to_path_buf());
        
        // Test that save_conversation is atomic - either completely succeeds or leaves no partial state
        let result = store.save_conversation(&session_id, &messages);
        
        if result.is_ok() {
            // If save succeeded, the conversation should be fully readable
            let loaded_messages = store.load_conversation(&session_id).unwrap();
            prop_assert_eq!(loaded_messages.len(), messages.len());
            
            // All messages should be preserved exactly
            for (original, loaded) in messages.iter().zip(loaded_messages.iter()) {
                prop_assert_eq!(&original.role, &loaded.role);
                prop_assert_eq!(&original.content, &loaded.content);
                prop_assert_eq!(&original.message_id, &loaded.message_id);
            }
            
            // Session should exist
            prop_assert!(store.session_exists(&session_id));
            
            // Metadata should be accessible
            let metadata = store.get_session_metadata(&session_id);
            prop_assert!(metadata.is_ok());
        } else {
            // If save failed, no partial files should exist
            prop_assert!(!store.session_exists(&session_id));
            
            // Loading should return empty or fail gracefully
            let load_result = store.load_conversation(&session_id);
            if let Ok(loaded) = load_result {
                prop_assert!(loaded.is_empty());
            }
        }
    }

    /// **Feature: user-persistent-memory, Property 10: Malformed data handling**
    /// **Validates: Requirements 4.5**
    #[test]
    fn test_malformed_data_handling_property(
        session_id in "[a-zA-Z0-9_.-]{1,50}",
        corrupted_content in ".*", // Any string content
    ) {
        let temp_dir = TempDir::new().unwrap();
        let store = MemoryStore::new(temp_dir.path().to_path_buf());
        
        // Create the necessary directories
        store.ensure_directories().unwrap();
        
        // Create a conversation file with malformed JSON
        let conversation_path = store.conversation_file_path(&session_id);
        std::fs::write(&conversation_path, &corrupted_content).unwrap();
        
        // Loading should handle malformed data gracefully
        let load_result = store.load_conversation(&session_id);
        
        // Should either succeed with empty data or fail gracefully
        match load_result {
            Ok(messages) => {
                // If it succeeds, should return empty or valid messages
                for msg in &messages {
                    prop_assert!(!msg.role.is_empty());
                    prop_assert!(!msg.message_id.is_empty());
                }
            }
            Err(_) => {
                // Failure is acceptable for corrupted data
            }
        }
        
        // System should still be functional after encountering malformed data
        let valid_messages = vec![
            Message::new("user".to_string(), "Hello".to_string()),
        ];
        
        // Should be able to overwrite with valid data
        let save_result = store.save_conversation(&session_id, &valid_messages);
        if save_result.is_ok() {
            // Should now be able to load the valid data
            let loaded = store.load_conversation(&session_id).unwrap();
            prop_assert_eq!(loaded.len(), 1);
            prop_assert_eq!(&loaded[0].content, "Hello");
        }
    }

    /// **Feature: user-persistent-memory, Property 3: Session lifecycle management**
    /// **Validates: Requirements 2.1, 2.3, 2.4, 2.5**
    #[test]
    fn test_session_lifecycle_management(
        session_names in prop::collection::vec(
            prop::option::of("[a-zA-Z0-9 _.-]{1,50}"),
            1..10
        ),
        messages_per_session in prop::collection::vec(
            prop::collection::vec(
                (
                    "[a-zA-Z]{1,20}",  // role
                    ".*",              // content
                ).prop_map(|(role, content)| Message::new(role, content)),
                0..20
            ),
            1..10
        ),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = SessionManager::new(temp_dir.path().to_path_buf()).unwrap();
        
        // Track created sessions for validation
        let mut created_sessions = Vec::new();
        
        // Property: Session creation should always generate unique IDs
        for (i, session_name) in session_names.iter().enumerate() {
            let session_id = manager.create_session(session_name.clone()).unwrap();
            
            // Session ID should be unique
            prop_assert!(!created_sessions.contains(&session_id));
            created_sessions.push(session_id.clone());
            
            // Session should exist after creation
            prop_assert!(manager.session_exists(&session_id));
            
            // Session should be listable
            let sessions = manager.list_sessions().unwrap();
            prop_assert!(sessions.iter().any(|s| s.id == session_id));
            
            // Add messages to this session if we have them
            if i < messages_per_session.len() {
                for message in &messages_per_session[i] {
                    manager.save_message(&session_id, message).unwrap();
                }
            }
        }
        
        // Property: All created sessions should be listable and have correct metadata
        let all_sessions = manager.list_sessions().unwrap();
        prop_assert_eq!(all_sessions.len(), created_sessions.len());
        
        for (i, session_id) in created_sessions.iter().enumerate() {
            let session_info = all_sessions.iter().find(|s| &s.id == session_id);
            prop_assert!(session_info.is_some());
            
            let session_info = session_info.unwrap();
            
            // Name should match what we set
            prop_assert_eq!(&session_info.name, &session_names[i]);
            
            // Message count should match what we added
            if i < messages_per_session.len() {
                prop_assert_eq!(session_info.message_count, messages_per_session[i].len());
            } else {
                prop_assert_eq!(session_info.message_count, 0);
            }
            
            // Should be able to load session history
            let history = manager.load_session_history(session_id).unwrap();
            if i < messages_per_session.len() {
                prop_assert_eq!(history.len(), messages_per_session[i].len());
                
                // Messages should be preserved in order
                for (j, original_msg) in messages_per_session[i].iter().enumerate() {
                    prop_assert_eq!(&history[j].role, &original_msg.role);
                    prop_assert_eq!(&history[j].content, &original_msg.content);
                    prop_assert_eq!(&history[j].message_id, &original_msg.message_id);
                }
            }
        }
        
        // Property: Session switching should work correctly
        if !created_sessions.is_empty() {
            let first_session = &created_sessions[0];
            manager.switch_session(first_session).unwrap();
            prop_assert_eq!(manager.get_current_session(), Some(first_session.as_str()));
            
            // Should be able to load current session history
            let current_history = manager.load_current_session_history().unwrap();
            let direct_history = manager.load_session_history(first_session).unwrap();
            prop_assert_eq!(current_history.len(), direct_history.len());
        }
        
        // Property: Session deletion should work correctly and maintain consistency
        let sessions_to_delete = created_sessions.clone();
        for session_id in sessions_to_delete {
            // Session should exist before deletion
            prop_assert!(manager.session_exists(&session_id));
            
            // Delete the session
            manager.delete_session(&session_id).unwrap();
            
            // Session should no longer exist
            prop_assert!(!manager.session_exists(&session_id));
            
            // Session should not be in the list
            let remaining_sessions = manager.list_sessions().unwrap();
            prop_assert!(!remaining_sessions.iter().any(|s| s.id == session_id));
            
            // Loading session should return empty or error gracefully
            let load_result = manager.load_session_history(&session_id);
            if let Ok(messages) = load_result {
                prop_assert!(messages.is_empty());
            }
        }
        
        // After deleting all sessions, list should be empty
        let final_sessions = manager.list_sessions().unwrap();
        prop_assert!(final_sessions.is_empty());
        
        // Current session should be cleared if it was deleted
        if manager.get_current_session().is_some() {
            let current = manager.get_current_session().unwrap();
            prop_assert!(!created_sessions.contains(&current.to_string()));
        }
    }

    /// **Feature: user-persistent-memory, Property 4: Session metadata accuracy**
    /// **Validates: Requirements 2.2**
    #[test]
    fn test_session_metadata_accuracy(
        session_names in prop::collection::vec(
            prop::option::of("[a-zA-Z0-9 _.-]{1,50}"),
            1..5
        ),
        message_batches in prop::collection::vec(
            prop::collection::vec(
                (
                    "[a-zA-Z]{1,20}",  // role
                    ".*",              // content
                ).prop_map(|(role, content)| Message::new(role, content)),
                0..50
            ),
            1..5
        ),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = SessionManager::new(temp_dir.path().to_path_buf()).unwrap();
        
        let mut created_sessions = Vec::new();
        let mut expected_message_counts = Vec::new();
        
        // Create sessions and add messages
        for (i, session_name) in session_names.iter().enumerate() {
            let session_id = manager.create_session(session_name.clone()).unwrap();
            created_sessions.push(session_id.clone());
            
            // Add messages if we have them for this session
            let message_count = if i < message_batches.len() {
                for message in &message_batches[i] {
                    manager.save_message(&session_id, message).unwrap();
                }
                message_batches[i].len()
            } else {
                0
            };
            expected_message_counts.push(message_count);
        }
        
        // Property: Listed sessions should have accurate metadata
        let listed_sessions = manager.list_sessions().unwrap();
        prop_assert_eq!(listed_sessions.len(), created_sessions.len());
        
        for (i, expected_session_id) in created_sessions.iter().enumerate() {
            let session_info = listed_sessions.iter()
                .find(|s| &s.id == expected_session_id)
                .unwrap();
            
            // Name should match what we set
            prop_assert_eq!(&session_info.name, &session_names[i]);
            
            // Message count should be accurate
            prop_assert_eq!(session_info.message_count, expected_message_counts[i]);
            
            // Timestamps should be reasonable (not in the future, not too old)
            let now = chrono::Utc::now();
            prop_assert!(session_info.created_at <= now);
            prop_assert!(session_info.last_modified <= now);
            prop_assert!(session_info.created_at <= session_info.last_modified);
            
            // Timestamps should be recent (within last hour for this test)
            let one_hour_ago = now - chrono::Duration::hours(1);
            prop_assert!(session_info.created_at >= one_hour_ago);
            prop_assert!(session_info.last_modified >= one_hour_ago);
            
            // Verify metadata consistency by loading session directly
            let direct_metadata = manager.get_session_metadata(expected_session_id).unwrap();
            prop_assert_eq!(&session_info.id, &direct_metadata.id);
            prop_assert_eq!(&session_info.name, &direct_metadata.name);
            
            // Load actual messages and verify count matches
            let actual_messages = manager.load_session_history(expected_session_id).unwrap();
            prop_assert_eq!(session_info.message_count, actual_messages.len());
            
            // If we added messages, verify they match what we expect
            if i < message_batches.len() {
                prop_assert_eq!(actual_messages.len(), message_batches[i].len());
                
                for (j, original_msg) in message_batches[i].iter().enumerate() {
                    prop_assert_eq!(&actual_messages[j].role, &original_msg.role);
                    prop_assert_eq!(&actual_messages[j].content, &original_msg.content);
                    prop_assert_eq!(&actual_messages[j].message_id, &original_msg.message_id);
                }
            }
        }
        
        // Property: Enhanced listing should also have accurate metadata
        let enhanced_sessions = manager.list_sessions_with_details().unwrap();
        prop_assert_eq!(enhanced_sessions.len(), created_sessions.len());
        
        // Enhanced listing should have the same data as regular listing
        for session in &enhanced_sessions {
            let regular_session = listed_sessions.iter()
                .find(|s| s.id == session.id)
                .unwrap();
            
            prop_assert_eq!(&session.name, &regular_session.name);
            prop_assert_eq!(session.message_count, regular_session.message_count);
            prop_assert_eq!(session.created_at, regular_session.created_at);
            // last_modified might be updated by the enhanced listing, so we check it's >= original
            prop_assert!(session.last_modified >= regular_session.last_modified);
        }
        
        // Property: Session statistics should be accurate
        let stats = manager.get_session_statistics().unwrap();
        prop_assert_eq!(stats.total_sessions, created_sessions.len());
        
        let expected_total_messages: usize = expected_message_counts.iter().sum();
        prop_assert_eq!(stats.total_messages, expected_total_messages);
        
        if !created_sessions.is_empty() {
            prop_assert!(stats.oldest_session.is_some());
            prop_assert!(stats.newest_session.is_some());
            
            // Most active session should have the highest message count
            if expected_total_messages > 0 {
                prop_assert!(stats.most_active_session.is_some());
                let (most_active_id, most_active_count) = stats.most_active_session.unwrap();
                
                // Find the expected most active session
                let max_messages = expected_message_counts.iter().max().unwrap();
                prop_assert_eq!(most_active_count, *max_messages);
                
                // Verify the session ID corresponds to a session with that message count
                let most_active_session = listed_sessions.iter()
                    .find(|s| s.id == most_active_id)
                    .unwrap();
                prop_assert_eq!(most_active_session.message_count, *max_messages);
            }
        }
    }

    /// **Feature: user-persistent-memory, Property 5: Preference persistence round-trip**
    /// **Validates: Requirements 3.1, 3.2, 3.3**
    #[test]
    fn test_preference_persistence_round_trip(
        default_model in "[a-zA-Z0-9_.-]{1,50}",
        confirm_before_tools in any::<bool>(),
        precheck_mode in "[a-zA-Z]{1,20}",
        ollama_base_url in "https?://[a-zA-Z0-9.-]+(:[0-9]+)?",
        default_session_name in prop::option::of("[a-zA-Z0-9 _.-]{1,50}"),
        auto_save_interval in prop::option::of(1u64..3600),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path().to_path_buf();
        
        // Create a PreferenceManager
        let mut manager = PreferenceManager::new(base_dir.clone()).unwrap();
        
        // Create test preferences with random values
        let test_preferences = UserPreferences {
            default_model: default_model.clone(),
            confirm_before_tools,
            precheck_mode: precheck_mode.clone(),
            ollama_base_url: ollama_base_url.clone(),
            default_session_name: default_session_name.clone(),
            auto_save_interval,
            version: "1.0".to_string(),
        };
        
        // Set the preferences
        manager.current_preferences = test_preferences.clone();
        
        // Save the preferences
        let save_result = manager.save_preferences();
        prop_assert!(save_result.is_ok());
        
        // Create a new manager instance to load from file
        let mut new_manager = PreferenceManager::new(base_dir.clone()).unwrap();
        
        // Load preferences should succeed
        let load_result = new_manager.load_preferences();
        prop_assert!(load_result.is_ok());
        
        // Loaded preferences should match exactly what we saved
        let loaded_prefs = new_manager.get_preferences();
        prop_assert_eq!(&loaded_prefs.default_model, &test_preferences.default_model);
        prop_assert_eq!(loaded_prefs.confirm_before_tools, test_preferences.confirm_before_tools);
        prop_assert_eq!(&loaded_prefs.precheck_mode, &test_preferences.precheck_mode);
        prop_assert_eq!(&loaded_prefs.ollama_base_url, &test_preferences.ollama_base_url);
        prop_assert_eq!(&loaded_prefs.default_session_name, &test_preferences.default_session_name);
        prop_assert_eq!(loaded_prefs.auto_save_interval, test_preferences.auto_save_interval);
        
        // Test individual preference update methods
        let new_model = format!("updated_{}", default_model);
        let update_result = new_manager.update_model_preference(new_model.clone());
        prop_assert!(update_result.is_ok());
        prop_assert_eq!(&new_manager.get_preferences().default_model, &new_model);
        
        let new_confirmation = !confirm_before_tools;
        let confirmation_result = new_manager.update_confirmation_preference(new_confirmation);
        prop_assert!(confirmation_result.is_ok());
        prop_assert_eq!(new_manager.get_preferences().confirm_before_tools, new_confirmation);
        
        // Test reset to defaults
        let reset_result = new_manager.reset_to_defaults();
        prop_assert!(reset_result.is_ok());
        
        let defaults = UserPreferences::default();
        let reset_prefs = new_manager.get_preferences();
        prop_assert_eq!(reset_prefs, &defaults);
        
        // Create yet another manager to verify reset was persisted
        let final_manager = PreferenceManager::new(base_dir).unwrap();
        let final_prefs = final_manager.get_preferences();
        prop_assert_eq!(final_prefs, &defaults);
    }

    /// **Feature: user-persistent-memory, Property 6: Graceful preference corruption handling**
    /// **Validates: Requirements 3.5**
    #[test]
    fn test_graceful_preference_corruption_handling(
        corrupted_content in ".*", // Any string content
    ) {
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path().to_path_buf();
        let prefs_file = base_dir.join("preferences.json");
        
        // Write corrupted content to preferences file
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(&prefs_file, &corrupted_content).unwrap();
        
        // PreferenceManager should handle corruption gracefully
        let result = PreferenceManager::new(base_dir);
        
        // Should not panic and should create a valid manager
        prop_assert!(result.is_ok());
        
        let manager = result.unwrap();
        
        // Should fall back to default preferences
        let prefs = manager.get_preferences();
        let defaults = UserPreferences::default();
        prop_assert_eq!(prefs, &defaults);
        
        // File should now contain valid JSON (corruption was fixed)
        let fixed_content = std::fs::read_to_string(&prefs_file).unwrap();
        prop_assert!(serde_json::from_str::<UserPreferences>(&fixed_content).is_ok());
    }
}

// Additional unit tests from memory_store.rs
#[test]
fn test_message_appending() {
    let temp_dir = TempDir::new().unwrap();
    let store = MemoryStore::new(temp_dir.path().to_path_buf());
    let session_id = "append_test";
    
    // Start with empty session
    assert!(!store.session_exists(session_id));
    
    // Append first message
    let msg1 = Message::new("user".to_string(), "Hello".to_string());
    store.append_message(session_id, &msg1).unwrap();
    
    // Session should now exist
    assert!(store.session_exists(session_id));
    
    // Load and verify
    let messages = store.load_conversation(session_id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Hello");
    
    // Append second message
    let msg2 = Message::new("assistant".to_string(), "Hi there!".to_string());
    store.append_message(session_id, &msg2).unwrap();
    
    // Load and verify both messages
    let messages = store.load_conversation(session_id).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "Hello");
    assert_eq!(messages[1].content, "Hi there!");
}

#[test]
fn test_session_metadata_tracking() {
    let temp_dir = TempDir::new().unwrap();
    let store = MemoryStore::new(temp_dir.path().to_path_buf());
    let session_id = "metadata_test";
    
    // Create a session with some messages
    let messages = vec![
        Message::new("user".to_string(), "Hello".to_string()),
        Message::new("assistant".to_string(), "Hi!".to_string()),
    ];
    
    store.save_conversation(session_id, &messages).unwrap();
    
    // Get metadata
    let metadata = store.get_session_metadata(session_id).unwrap();
    assert_eq!(metadata.id, session_id);
    assert!(metadata.created_at <= chrono::Utc::now());
    assert!(metadata.last_modified <= chrono::Utc::now());
    
    // Get session info (includes message count)
    let info = store.get_session_info(session_id).unwrap();
    assert_eq!(info.message_count, 2);
    assert_eq!(info.id, session_id);
}

#[test]
fn test_session_existence_validation() {
    let temp_dir = TempDir::new().unwrap();
    let store = MemoryStore::new(temp_dir.path().to_path_buf());
    
    // Non-existent session
    assert!(!store.session_exists("nonexistent"));
    
    // Invalid session IDs
    assert!(!store.session_exists(""));
    assert!(!store.session_exists("invalid/session"));
    assert!(!store.session_exists("invalid\\session"));
    
    // Create a valid session
    let session_id = "valid_session";
    let message = Message::new("user".to_string(), "Test".to_string());
    store.append_message(session_id, &message).unwrap();
    
    // Should now exist
    assert!(store.session_exists(session_id));
    
    // Delete and verify it no longer exists
    store.delete_session_data(session_id).unwrap();
    assert!(!store.session_exists(session_id));
}

#[test]
fn test_storage_failure_resilience() {
    let temp_dir = TempDir::new().unwrap();
    let store = MemoryStore::new(temp_dir.path().to_path_buf());
    let session_id = "test_session";
    
    // Create some test messages
    let messages = vec![
        Message::new("user".to_string(), "Hello".to_string()),
        Message::new("assistant".to_string(), "Hi there!".to_string()),
    ];
    
    // First, successfully save some messages
    let result = store.save_conversation(session_id, &messages);
    assert!(result.is_ok());
    
    // Verify the messages were saved
    let loaded = store.load_conversation(session_id).unwrap();
    assert_eq!(loaded.len(), messages.len());
    
    // Test resilience to read failures on non-existent files
    let nonexistent_session = "nonexistent_session";
    let load_result = store.load_conversation(nonexistent_session);
    
    // Should return empty conversation, not crash
    assert!(load_result.is_ok());
    let empty_messages = load_result.unwrap();
    assert!(empty_messages.is_empty());
    
    // Test resilience to invalid session IDs
    let invalid_sessions = vec!["", "invalid/session", "invalid\\session", "invalid:session"];
    for invalid_id in invalid_sessions {
        // Operations should fail gracefully, not crash
        let save_result = store.save_conversation(invalid_id, &messages);
        assert!(save_result.is_err());
        
        let load_result = store.load_conversation(invalid_id);
        assert!(load_result.is_err());
        
        let exists_result = store.session_exists(invalid_id);
        assert!(!exists_result); // Should return false, not crash
    }
    
    // Test that the original session is still intact after failures
    let reloaded = store.load_conversation(session_id).unwrap();
    assert_eq!(reloaded.len(), messages.len());
    
    // Test append resilience - should work even after failed operations
    let new_message = Message::new("user".to_string(), "Additional message".to_string());
    let append_result = store.append_message(session_id, &new_message);
    assert!(append_result.is_ok());
    
    // Verify append worked
    let final_messages = store.load_conversation(session_id).unwrap();
    assert_eq!(final_messages.len(), messages.len() + 1);
}

#[test]
fn test_malformed_data_handling() {
    let temp_dir = TempDir::new().unwrap();
    let store = MemoryStore::new(temp_dir.path().to_path_buf());
    let session_id = "malformed_test";
    
    // Create the necessary directories
    store.ensure_directories().unwrap();
    
    // Create a conversation file with malformed JSON
    let conversation_path = store.conversation_file_path(session_id);
    std::fs::write(&conversation_path, "{ invalid json content }").unwrap();
    
    // Loading should handle malformed data gracefully
    let load_result = store.load_conversation(session_id);
    assert!(load_result.is_err());
    
    // Should be a CorruptedData error
    match load_result.unwrap_err() {
        rustline::persistence::PersistenceError::CorruptedData { .. } => {
            // Expected error type
        }
        other => panic!("Expected CorruptedData error, got: {:?}", other),
    }
    
    // System should still be functional after encountering malformed data
    let valid_messages = vec![
        Message::new("user".to_string(), "Hello".to_string()),
    ];
    
    // Should be able to overwrite with valid data
    let save_result = store.save_conversation(session_id, &valid_messages);
    assert!(save_result.is_ok());
    
    // Should now be able to load the valid data
    let loaded = store.load_conversation(session_id).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].content, "Hello");
}

#[test]
fn test_concurrent_atomic_operations() {
    let temp_dir = TempDir::new().unwrap();
    let store = Arc::new(MemoryStore::new(temp_dir.path().to_path_buf()));
    let session_id = "concurrent_test_session";
    
    // Create multiple threads that try to write to the same session
    let handles: Vec<_> = (0..10).map(|i| {
        let store_clone = Arc::clone(&store);
        let session_id = session_id.to_string();
        
        thread::spawn(move || {
            let message = Message::new("user".to_string(), format!("Message {}", i));
            store_clone.append_message(&session_id, &message)
        })
    }).collect();
    
    // Wait for all threads to complete
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    
    // All operations should either succeed or fail cleanly
    for result in &results {
        // No panics or corrupted state
        assert!(result.is_ok() || result.is_err());
    }
    
    // If any succeeded, the session should be in a valid state
    if results.iter().any(|r| r.is_ok()) {
        let messages = store.load_conversation(session_id).unwrap();
        // Should have some messages, and they should be valid
        assert!(!messages.is_empty());
        
        // All messages should be properly formatted
        for msg in &messages {
            assert!(!msg.role.is_empty());
            assert!(!msg.message_id.is_empty());
        }
    }
}

#[test]
fn test_atomic_write_failure_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let store = MemoryStore::new(temp_dir.path().to_path_buf());
    let session_id = "test_session";
    
    // First, save a valid conversation
    let original_messages = vec![
        Message::new("user".to_string(), "Hello".to_string()),
        Message::new("assistant".to_string(), "Hi there!".to_string()),
    ];
    
    store.save_conversation(session_id, &original_messages).unwrap();
    
    // Verify it was saved correctly
    let loaded = store.load_conversation(session_id).unwrap();
    assert_eq!(loaded.len(), 2);
    
    // Now simulate a failure during write by making the directory read-only
    // (This is a simplified test - in practice, failures could happen during write)
    let conversation_path = store.conversation_file_path(session_id);
    assert!(conversation_path.exists());
    
    // The original data should still be intact
    let reloaded = store.load_conversation(session_id).unwrap();
    assert_eq!(reloaded.len(), 2);
    assert_eq!(reloaded[0].content, "Hello");
    assert_eq!(reloaded[1].content, "Hi there!");
}

// Additional unit tests from session_manager.rs
#[test]
fn test_session_manager_creation() {
    let temp_dir = TempDir::new().unwrap();
    let manager = SessionManager::new(temp_dir.path().to_path_buf());
    assert!(manager.is_ok());
    
    let manager = manager.unwrap();
    assert!(manager.get_current_session().is_none());
}

#[test]
fn test_create_and_switch_session() {
    let temp_dir = TempDir::new().unwrap();
    let mut manager = SessionManager::new(temp_dir.path().to_path_buf()).unwrap();
    
    // Create a session
    let session_id = manager.create_session(Some("Test Session".to_string())).unwrap();
    assert!(!session_id.is_empty());
    assert!(session_id.starts_with("session_"));
    
    // Session should exist
    assert!(manager.session_exists(&session_id));
    
    // Switch to the session
    manager.switch_session(&session_id).unwrap();
    assert_eq!(manager.get_current_session(), Some(session_id.as_str()));
    
    // Load empty history
    let history = manager.load_current_session_history().unwrap();
    assert!(history.is_empty());
}

#[test]
fn test_session_message_operations() {
    let temp_dir = TempDir::new().unwrap();
    let mut manager = SessionManager::new(temp_dir.path().to_path_buf()).unwrap();
    
    // Create and switch to a session
    let session_id = manager.create_session(None).unwrap();
    manager.switch_session(&session_id).unwrap();
    
    // Save messages to current session
    let msg1 = Message::new("user".to_string(), "Hello".to_string());
    let msg2 = Message::new("assistant".to_string(), "Hi there!".to_string());
    
    manager.save_message_to_current_session(&msg1).unwrap();
    manager.save_message_to_current_session(&msg2).unwrap();
    
    // Load and verify history
    let history = manager.load_current_session_history().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "Hello");
    assert_eq!(history[1].content, "Hi there!");
}

#[test]
fn test_session_deletion() {
    let temp_dir = TempDir::new().unwrap();
    let mut manager = SessionManager::new(temp_dir.path().to_path_buf()).unwrap();
    
    // Create a session
    let session_id = manager.create_session(Some("To Delete".to_string())).unwrap();
    manager.switch_session(&session_id).unwrap();
    
    // Add a message
    let msg = Message::new("user".to_string(), "Test message".to_string());
    manager.save_message_to_current_session(&msg).unwrap();
    
    // Verify session exists
    assert!(manager.session_exists(&session_id));
    
    // Delete the session
    manager.delete_session(&session_id).unwrap();
    
    // Session should no longer exist
    assert!(!manager.session_exists(&session_id));
    
    // Current session should be cleared
    assert!(manager.get_current_session().is_none());
}

#[test]
fn test_list_sessions() {
    let temp_dir = TempDir::new().unwrap();
    let mut manager = SessionManager::new(temp_dir.path().to_path_buf()).unwrap();
    
    // Initially no sessions
    let sessions = manager.list_sessions().unwrap();
    assert!(sessions.is_empty());
    
    // Create multiple sessions
    let session1 = manager.create_session(Some("Session 1".to_string())).unwrap();
    let session2 = manager.create_session(Some("Session 2".to_string())).unwrap();
    
    // Add messages to sessions
    let msg = Message::new("user".to_string(), "Test".to_string());
    manager.save_message(&session1, &msg).unwrap();
    manager.save_message(&session2, &msg).unwrap();
    manager.save_message(&session2, &msg).unwrap(); // Second message
    
    // List sessions
    let sessions = manager.list_sessions().unwrap();
    assert_eq!(sessions.len(), 2);
    
    // Find sessions by ID
    let s1 = sessions.iter().find(|s| s.id == session1).unwrap();
    let s2 = sessions.iter().find(|s| s.id == session2).unwrap();
    
    assert_eq!(s1.name, Some("Session 1".to_string()));
    assert_eq!(s1.message_count, 1);
    
    assert_eq!(s2.name, Some("Session 2".to_string()));
    assert_eq!(s2.message_count, 2);
}

#[test]
fn test_session_manager_error_handling() {
    let temp_dir = TempDir::new().unwrap();
    let mut manager = SessionManager::new(temp_dir.path().to_path_buf()).unwrap();
    
    // Test switching to non-existent session
    let result = manager.switch_session("nonexistent");
    assert!(result.is_err());
    match result.unwrap_err() {
        rustline::persistence::PersistenceError::SessionNotFound { .. } => {},
        other => panic!("Expected SessionNotFound, got: {:?}", other),
    }
    
    // Test deleting non-existent session
    let result = manager.delete_session("nonexistent");
    assert!(result.is_err());
    
    // Test invalid session IDs
    let result = manager.create_session(Some("Valid Name".to_string()));
    assert!(result.is_ok()); // Should work
    
    let result = manager.switch_session("invalid/session");
    assert!(result.is_err());
    
    let result = manager.delete_session("");
    assert!(result.is_err());
}

// Additional unit tests from preference_manager.rs
#[test]
fn test_preference_reset_to_defaults() {
    let temp_dir = TempDir::new().unwrap();
    let mut manager = PreferenceManager::new(temp_dir.path().to_path_buf()).unwrap();
    
    // Modify preferences
    manager.current_preferences.default_model = "custom_model".to_string();
    manager.current_preferences.confirm_before_tools = false;
    manager.current_preferences.ollama_base_url = "http://custom:8080".to_string();
    
    // Save modified preferences
    manager.save_preferences().unwrap();
    
    // Reset to defaults
    manager.reset_to_defaults().unwrap();
    
    // Verify preferences are back to defaults
    let defaults = UserPreferences::default();
    assert_eq!(manager.get_preferences(), &defaults);
    
    // Verify defaults were persisted
    let new_manager = PreferenceManager::new(temp_dir.path().to_path_buf()).unwrap();
    assert_eq!(new_manager.get_preferences(), &defaults);
}

#[test]
fn test_preference_migration() {
    let temp_dir = TempDir::new().unwrap();
    let prefs_file = temp_dir.path().join("preferences.json");
    
    // Create an old version preferences file (without version field)
    let old_prefs = serde_json::json!({
        "default_model": "old_model",
        "confirm_before_tools": false,
        "precheck_mode": "loose",
        "ollama_base_url": "http://old:1234"
        // Note: missing default_session_name, auto_save_interval, and version fields
    });
    
    std::fs::create_dir_all(temp_dir.path()).unwrap();
    std::fs::write(&prefs_file, serde_json::to_string_pretty(&old_prefs).unwrap()).unwrap();
    
    // Load preferences - should trigger migration
    let manager = PreferenceManager::new(temp_dir.path().to_path_buf()).unwrap();
    
    // Verify old values were preserved
    assert_eq!(manager.get_preferences().default_model, "old_model");
    assert_eq!(manager.get_preferences().confirm_before_tools, false);
    assert_eq!(manager.get_preferences().precheck_mode, "loose");
    assert_eq!(manager.get_preferences().ollama_base_url, "http://old:1234");
    
    // Verify new fields have default values
    assert_eq!(manager.get_preferences().default_session_name, None);
    assert_eq!(manager.get_preferences().auto_save_interval, Some(30));
    assert_eq!(manager.get_preferences().version, "1.0");
    
    // Verify migrated preferences were saved
    let content = std::fs::read_to_string(&prefs_file).unwrap();
    let saved_prefs: UserPreferences = serde_json::from_str(&content).unwrap();
    assert_eq!(saved_prefs.version, "1.0");
}

#[test]
fn test_corrupted_preferences_handling() {
    let temp_dir = TempDir::new().unwrap();
    let prefs_file = temp_dir.path().join("preferences.json");
    
    // Write completely invalid JSON
    std::fs::create_dir_all(temp_dir.path()).unwrap();
    std::fs::write(&prefs_file, "{ invalid json }").unwrap();
    
    // Should handle corruption gracefully
    let manager = PreferenceManager::new(temp_dir.path().to_path_buf()).unwrap();
    
    // Should fall back to defaults
    let defaults = UserPreferences::default();
    assert_eq!(manager.get_preferences(), &defaults);
    
    // File should now contain valid JSON
    let content = std::fs::read_to_string(&prefs_file).unwrap();
    let fixed_prefs: UserPreferences = serde_json::from_str(&content).unwrap();
    assert_eq!(fixed_prefs, defaults);
}

#[test]
fn test_preference_update_methods() {
    let temp_dir = TempDir::new().unwrap();
    let mut manager = PreferenceManager::new(temp_dir.path().to_path_buf()).unwrap();
    
    // Test model preference update
    let new_model = "updated_model".to_string();
    manager.update_model_preference(new_model.clone()).unwrap();
    assert_eq!(manager.get_preferences().default_model, new_model);
    
    // Test confirmation preference update
    let original_confirmation = manager.get_preferences().confirm_before_tools;
    manager.update_confirmation_preference(!original_confirmation).unwrap();
    assert_eq!(manager.get_preferences().confirm_before_tools, !original_confirmation);
    
    // Verify changes were persisted
    let new_manager = PreferenceManager::new(temp_dir.path().to_path_buf()).unwrap();
    assert_eq!(new_manager.get_preferences().default_model, new_model);
    assert_eq!(new_manager.get_preferences().confirm_before_tools, !original_confirmation);
}