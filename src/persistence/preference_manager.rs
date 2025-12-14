use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use crate::persistence::{PersistenceError, PersistenceResult, ensure_directory, atomic_write};

/// User preferences for the application
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct UserPreferences {
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default = "default_confirm_before_tools")]
    pub confirm_before_tools: bool,
    #[serde(default = "default_precheck_mode")]
    pub precheck_mode: String,
    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,
    #[serde(default)]
    pub default_session_name: Option<String>,
    #[serde(default = "default_auto_save_interval")]
    pub auto_save_interval: Option<u64>, // seconds
    #[serde(default = "default_version")]
    pub version: String, // For future migration compatibility
}

// Default value functions for serde
fn default_model() -> String {
    "gemma3".to_string()
}

fn default_confirm_before_tools() -> bool {
    true
}

fn default_precheck_mode() -> String {
    "strict".to_string()
}

fn default_ollama_base_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_auto_save_interval() -> Option<u64> {
    Some(30)
}

fn default_version() -> String {
    "1.0".to_string()
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            default_model: default_model(),
            confirm_before_tools: default_confirm_before_tools(),
            precheck_mode: default_precheck_mode(),
            ollama_base_url: default_ollama_base_url(),
            default_session_name: None,
            auto_save_interval: default_auto_save_interval(),
            version: default_version(),
        }
    }
}

/// Manages user preferences and application settings
pub struct PreferenceManager {
    preferences_file: PathBuf,
    pub current_preferences: UserPreferences,
}

impl PreferenceManager {
    /// Create a new PreferenceManager with the given base directory
    pub fn new(base_dir: PathBuf) -> PersistenceResult<Self> {
        ensure_directory(&base_dir)?;
        
        let preferences_file = base_dir.join("preferences.json");
        let mut manager = Self {
            preferences_file,
            current_preferences: UserPreferences::default(),
        };
        
        // Try to load existing preferences, fall back to defaults on error
        if let Err(e) = manager.load_preferences() {
            log::warn!("Failed to load preferences, using defaults: {}", e);
            manager.current_preferences = UserPreferences::default();
        }
        
        Ok(manager)
    }
    
    /// Load preferences from file, gracefully handling corruption and migration
    pub fn load_preferences(&mut self) -> PersistenceResult<()> {
        if !self.preferences_file.exists() {
            log::info!("Preferences file does not exist, using defaults");
            self.current_preferences = UserPreferences::default();
            return self.save_preferences();
        }
        
        let content = std::fs::read_to_string(&self.preferences_file)?;
        
        match serde_json::from_str::<UserPreferences>(&content) {
            Ok(mut prefs) => {
                // Check if migration is needed
                if self.needs_migration(&prefs) {
                    log::info!("Migrating preferences from version {} to {}", 
                              prefs.version, default_version());
                    prefs = self.migrate_preferences(prefs)?;
                    // Save migrated preferences
                    self.current_preferences = prefs;
                    self.save_preferences()?;
                } else {
                    self.current_preferences = prefs;
                }
                log::info!("Successfully loaded preferences");
                Ok(())
            }
            Err(e) => {
                log::error!("Corrupted preferences file, falling back to defaults: {}", e);
                self.current_preferences = UserPreferences::default();
                // Save defaults to fix the corrupted file
                self.save_preferences()?;
                Err(PersistenceError::CorruptedData {
                    path: self.preferences_file.to_string_lossy().to_string(),
                })
            }
        }
    }
    
    /// Save current preferences to file
    pub fn save_preferences(&self) -> PersistenceResult<()> {
        atomic_write(&self.preferences_file, |file| {
            let json = serde_json::to_string_pretty(&self.current_preferences)?;
            use std::io::Write;
            file.write_all(json.as_bytes())?;
            Ok(())
        })?;
        
        log::info!("Preferences saved successfully");
        Ok(())
    }
    
    /// Update model preference
    pub fn update_model_preference(&mut self, model: String) -> PersistenceResult<()> {
        self.current_preferences.default_model = model;
        self.save_preferences()
    }
    
    /// Update tool confirmation preference
    pub fn update_confirmation_preference(&mut self, enabled: bool) -> PersistenceResult<()> {
        self.current_preferences.confirm_before_tools = enabled;
        self.save_preferences()
    }
    
    /// Reset preferences to defaults
    #[allow(dead_code)]
    pub fn reset_to_defaults(&mut self) -> PersistenceResult<()> {
        self.current_preferences = UserPreferences::default();
        self.save_preferences()
    }
    
    /// Get current preferences
    pub fn get_preferences(&self) -> &UserPreferences {
        &self.current_preferences
    }
    
    /// Check if preferences need migration
    fn needs_migration(&self, prefs: &UserPreferences) -> bool {
        prefs.version != default_version()
    }
    
    /// Migrate preferences from older versions
    fn migrate_preferences(&self, mut prefs: UserPreferences) -> PersistenceResult<UserPreferences> {
        let current_version = default_version();
        
        match prefs.version.as_str() {
            // Future migrations can be added here
            // For now, we just update the version and ensure all fields have defaults
            _ => {
                // For any unknown version, merge with defaults to ensure all fields exist
                // This is handled automatically by serde's #[serde(default)] attributes
                prefs.version = current_version;
                
                log::info!("Migrated preferences to version {}", prefs.version);
                Ok(prefs)
            }
        }
    }
}

