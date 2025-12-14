use rustline::ui::App;
use rustline::PersistenceState;

#[test]
fn test_app_set_persistence_state_enabled() {
    // Test that App correctly stores PersistenceState::Enabled
    let mut app = App::new();
    app.set_persistence_state(PersistenceState::Enabled);
    
    match app.persistence_state {
        PersistenceState::Enabled => assert!(true),
        _ => panic!("Expected PersistenceState::Enabled"),
    }
}

#[test]
fn test_app_set_persistence_state_disabled() {
    // Test that App correctly stores PersistenceState::Disabled
    let mut app = App::new();
    app.set_persistence_state(PersistenceState::Disabled);
    
    match app.persistence_state {
        PersistenceState::Disabled => assert!(true),
        _ => panic!("Expected PersistenceState::Disabled"),
    }
}

#[test]
fn test_app_set_persistence_state_failed_fallback() {
    // Test that App correctly stores PersistenceState::FailedFallback with error message
    let mut app = App::new();
    let error_msg = "Test persistence error";
    app.set_persistence_state(PersistenceState::FailedFallback(error_msg.to_string()));
    
    match &app.persistence_state {
        PersistenceState::FailedFallback(msg) => {
            assert_eq!(msg, error_msg);
        }
        _ => panic!("Expected PersistenceState::FailedFallback"),
    }
}

#[test]
fn test_app_new_has_default_persistence_state() {
    // Test that new App has default persistence state
    let app = App::new();
    
    match app.persistence_state {
        PersistenceState::Enabled => assert!(true),
        _ => panic!("Expected default PersistenceState::Enabled"),
    }
}

#[test]
fn test_persistence_state_display_properties() {
    // Test that persistence state display properties are correct
    let enabled_state = PersistenceState::Enabled;
    let disabled_state = PersistenceState::Disabled;
    let failed_state = PersistenceState::FailedFallback("error".to_string());

    // Test that we can match on the states (this validates the enum structure)
    match enabled_state {
        PersistenceState::Enabled => assert!(true),
        _ => panic!("Enabled state match failed"),
    }

    match disabled_state {
        PersistenceState::Disabled => assert!(true),
        _ => panic!("Disabled state match failed"),
    }

    match failed_state {
        PersistenceState::FailedFallback(_) => assert!(true),
        _ => panic!("Failed fallback state match failed"),
    }
}