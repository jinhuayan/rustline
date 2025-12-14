use std::path::PathBuf;
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use crate::ollama::Message;
use super::{
    PersistenceError, PersistenceResult, validate_session_id,
    memory_store::{MemoryStore, SessionInfo, SessionMetadata}
};

/// Statistics about all sessions
#[derive(Debug, Clone)]
pub struct SessionStatistics {
    pub total_sessions: usize,
    pub total_messages: usize,
    pub oldest_session: Option<DateTime<Utc>>,
    pub newest_session: Option<DateTime<Utc>>,
    pub most_active_session: Option<(String, usize)>, // (session_id, message_count)
}

/// Manages session lifecycle and operations
pub struct SessionManager {
    current_session_id: Option<String>,
    sessions_dir: PathBuf,
    memory_store: MemoryStore,
    /// Cache for session metadata to improve performance
    metadata_cache: HashMap<String, SessionMetadata>,
    /// Cache timestamp to know when to refresh
    cache_last_updated: Option<DateTime<Utc>>,
}

impl SessionManager {
    /// Create a new SessionManager with the specified base directory
    pub fn new(base_dir: PathBuf) -> PersistenceResult<Self> {
        let sessions_dir = base_dir.join("sessions");
        let memory_store = MemoryStore::new(base_dir);
        
        // Ensure directories exist
        memory_store.ensure_directories()?;
        
        Ok(Self {
            current_session_id: None,
            sessions_dir,
            memory_store,
            metadata_cache: HashMap::new(),
            cache_last_updated: None,
        })
    }

    /// Create a new session with optional name
    pub fn create_session(&mut self, name: Option<String>) -> PersistenceResult<String> {
        // Generate unique session ID
        let session_id = self.generate_unique_session_id()?;
        
        validate_session_id(&session_id)?;
        
        self.memory_store.save_conversation(&session_id, &[])?;
        
        // If a name was provided, we need to update the metadata with the name
        if name.is_some() {
            // Load the metadata that was created by save_conversation
            let mut metadata = self.memory_store.get_session_metadata(&session_id)?;
            metadata.name = name.clone();
            
            // Save the updated metadata back
            self.save_session_metadata(&metadata)?;
        }
        
        log::info!("Created new session: {} (name: {:?})", session_id, name);
        Ok(session_id)
    }

    /// List all available sessions with metadata
    pub fn list_sessions(&mut self) -> PersistenceResult<Vec<SessionInfo>> {

        self.refresh_metadata_cache_if_needed()?;
        
        // Get sessions from memory store and validate them
        let mut sessions = self.memory_store.list_sessions()?;
        
        // Validate and clean up orphaned sessions
        sessions.retain(|session| {
            if let Err(e) = self.validate_session(&session.id) {
                log::warn!("Removing invalid session {}: {}", session.id, e);
                // Try to clean up the orphaned session
                if let Err(cleanup_err) = self.cleanup_orphaned_session(&session.id) {
                    log::error!("Failed to cleanup orphaned session {}: {}", session.id, cleanup_err);
                }
                false
            } else {
                true
            }
        });
        
        Ok(sessions)
    }

    /// List sessions with enhanced metadata display
    pub fn list_sessions_with_details(&mut self) -> PersistenceResult<Vec<SessionInfo>> {
        let mut sessions = self.list_sessions()?;
        
        // Sort by last modified (most recent first)
        sessions.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
        
        // Enhance with additional metadata if available
        for session in &mut sessions {
            if let Ok(metadata) = self.get_cached_session_metadata(&session.id) {
                // Update with cached metadata if it's more recent
                if metadata.last_modified > session.last_modified {
                    session.last_modified = metadata.last_modified;
                }
            }
        }
        
        Ok(sessions)
    }

    /// Switch to a different session
    pub fn switch_session(&mut self, session_id: &str) -> PersistenceResult<()> {
        validate_session_id(session_id)?;
        
        // Check if session exists
        if !self.memory_store.session_exists(session_id) {
            return Err(PersistenceError::SessionNotFound {
                session_id: session_id.to_string(),
            });
        }
        
        // Switch to the new session
        self.current_session_id = Some(session_id.to_string());
        
        log::info!("Switched to session: {}", session_id);
        Ok(())
    }

    /// Delete a session and all its data
    pub fn delete_session(&mut self, session_id: &str) -> PersistenceResult<()> {
        validate_session_id(session_id)?;
        
        // Check if session exists
        if !self.memory_store.session_exists(session_id) {
            return Err(PersistenceError::SessionNotFound {
                session_id: session_id.to_string(),
            });
        }
        
        // If we're deleting the current session, clear the current session
        if self.current_session_id.as_ref() == Some(&session_id.to_string()) {
            self.current_session_id = None;
        }
        
        // Delete all session data
        self.memory_store.delete_session_data(session_id)?;
        
        log::info!("Deleted session: {}", session_id);
        Ok(())
    }

    /// Get the current session ID
    pub fn get_current_session(&self) -> Option<&str> {
        self.current_session_id.as_deref()
    }

    /// Load conversation history for a specific session
    pub fn load_session_history(&self, session_id: &str) -> PersistenceResult<Vec<Message>> {
        validate_session_id(session_id)?;
        self.memory_store.load_conversation(session_id)
    }

    /// Save a message to a specific session
    pub fn save_message(&mut self, session_id: &str, message: &Message) -> PersistenceResult<()> {
        validate_session_id(session_id)?;
        
        // If session doesn't exist, create it first
        if !self.memory_store.session_exists(session_id) {
            // Create the session with default metadata
            let metadata = SessionMetadata {
                id: session_id.to_string(),
                name: None,
                created_at: Utc::now(),
                last_modified: Utc::now(),
            };
            
            // Initialize with empty conversation
            self.memory_store.save_conversation(session_id, &[])?;
        }
        
        // Append the message
        self.memory_store.append_message(session_id, message)?;
        
        log::debug!("Saved message to session {}: {} chars", session_id, message.content.len());
        Ok(())
    }

    /// Get the current session ID, creating a default session if none exists
    pub fn get_or_create_current_session(&mut self) -> PersistenceResult<String> {
        if let Some(session_id) = &self.current_session_id {
            Ok(session_id.clone())
        } else {
            // Create a default session
            let session_id = self.create_session(Some("Default Session".to_string()))?;
            self.current_session_id = Some(session_id.clone());
            Ok(session_id)
        }
    }

    /// Load conversation history for the current session
    pub fn load_current_session_history(&self) -> PersistenceResult<Vec<Message>> {
        if let Some(session_id) = &self.current_session_id {
            self.load_session_history(session_id)
        } else {
            // No current session, return empty history
            Ok(Vec::new())
        }
    }

    /// Save a message to the current session
    pub fn save_message_to_current_session(&mut self, message: &Message) -> PersistenceResult<()> {
        let session_id = self.get_or_create_current_session()?;
        self.save_message(&session_id, message)
    }

    /// Generate a unique session ID that doesn't conflict with existing sessions
    fn generate_unique_session_id(&self) -> PersistenceResult<String> {
        const MAX_ATTEMPTS: usize = 100;
        
        for _ in 0..MAX_ATTEMPTS {
            let uuid = Uuid::new_v4();
            let session_id = format!("session_{}", uuid.simple());
            
            // Check if this ID is already in use
            if !self.memory_store.session_exists(&session_id) {
                return Ok(session_id);
            }
        }
        
        Err(PersistenceError::InvalidSessionId {
            session_id: "Failed to generate unique session ID after 100 attempts".to_string(),
        })
    }

    /// Get session metadata for a specific session
    pub fn get_session_metadata(&self, session_id: &str) -> PersistenceResult<SessionMetadata> {
        self.memory_store.get_session_metadata(session_id)
    }

    /// Check if a session exists
    pub fn session_exists(&self, session_id: &str) -> bool {
        self.memory_store.session_exists(session_id)
    }

    /// Save session metadata (private helper method)
    fn save_session_metadata(&mut self, metadata: &SessionMetadata) -> PersistenceResult<()> {
        use std::fs;
        use std::io::Write;
        use super::atomic_write;
        
        self.memory_store.ensure_directories()?;
        
        let metadata_path = self.sessions_dir.parent().unwrap().join("metadata").join(format!("{}_meta.json", metadata.id));
        
        atomic_write(&metadata_path, |file| {
            let json_data = serde_json::to_string_pretty(metadata)?;
            file.write_all(json_data.as_bytes())?;
            Ok(())
        })?;

        // Update cache
        self.metadata_cache.insert(metadata.id.clone(), metadata.clone());
        self.cache_last_updated = Some(Utc::now());

        log::debug!("Saved metadata for session {}", metadata.id);
        Ok(())
    }

    /// Get session metadata with caching
    fn get_cached_session_metadata(&mut self, session_id: &str) -> PersistenceResult<SessionMetadata> {
        // Check cache first
        if let Some(cached_metadata) = self.metadata_cache.get(session_id) {
            return Ok(cached_metadata.clone());
        }
        
        // Load from storage and cache it
        let metadata = self.memory_store.get_session_metadata(session_id)?;
        self.metadata_cache.insert(session_id.to_string(), metadata.clone());
        
        Ok(metadata)
    }

    /// Refresh metadata cache if it's stale
    fn refresh_metadata_cache_if_needed(&mut self) -> PersistenceResult<()> {
        const CACHE_DURATION_MINUTES: i64 = 5; // Cache for 5 minutes
        
        let should_refresh = match self.cache_last_updated {
            None => true,
            Some(last_updated) => {
                let now = Utc::now();
                (now - last_updated).num_minutes() > CACHE_DURATION_MINUTES
            }
        };
        
        if should_refresh {
            self.refresh_metadata_cache()?;
        }
        
        Ok(())
    }

    /// Force refresh of metadata cache
    fn refresh_metadata_cache(&mut self) -> PersistenceResult<()> {
        self.metadata_cache.clear();
        
        // Load all session metadata
        let sessions = self.memory_store.list_sessions()?;
        for session in sessions {
            if let Ok(metadata) = self.memory_store.get_session_metadata(&session.id) {
                self.metadata_cache.insert(session.id.clone(), metadata);
            }
        }
        
        self.cache_last_updated = Some(Utc::now());
        log::debug!("Refreshed metadata cache with {} entries", self.metadata_cache.len());
        
        Ok(())
    }

    /// Validate a session for consistency
    fn validate_session(&self, session_id: &str) -> PersistenceResult<()> {
        validate_session_id(session_id)?;
        
        // Check if conversation file exists
        if !self.memory_store.session_exists(session_id) {
            return Err(PersistenceError::SessionNotFound {
                session_id: session_id.to_string(),
            });
        }
        
        // Try to load conversation to ensure it's not corrupted
        match self.memory_store.load_conversation(session_id) {
            Ok(_) => {},
            Err(PersistenceError::CorruptedData { .. }) => {
                return Err(PersistenceError::CorruptedData {
                    path: format!("Session {} has corrupted conversation data", session_id),
                });
            },
            Err(e) => return Err(e),
        }
        
        // Try to load metadata to ensure it's accessible
        match self.memory_store.get_session_metadata(session_id) {
            Ok(_) => {},
            Err(PersistenceError::CorruptedData { .. }) => {
                log::warn!("Session {} has corrupted metadata, will be regenerated", session_id);
                // Corrupted metadata is recoverable, don't fail validation
            },
            Err(PersistenceError::SessionNotFound { .. }) => {
                log::warn!("Session {} missing metadata, will be regenerated", session_id);
                // Missing metadata is recoverable, don't fail validation
            },
            Err(e) => return Err(e),
        }
        
        Ok(())
    }

    /// Clean up orphaned session data
    fn cleanup_orphaned_session(&self, session_id: &str) -> PersistenceResult<()> {
        log::info!("Cleaning up orphaned session: {}", session_id);
        
        // Try to delete all session data
        match self.memory_store.delete_session_data(session_id) {
            Ok(_) => {
                log::info!("Successfully cleaned up orphaned session: {}", session_id);
            },
            Err(e) => {
                log::error!("Failed to clean up orphaned session {}: {}", session_id, e);
                return Err(e);
            }
        }
        
        Ok(())
    }

    /// Get session statistics
    pub fn get_session_statistics(&mut self) -> PersistenceResult<SessionStatistics> {
        let sessions = self.list_sessions()?;
        
        let total_sessions = sessions.len();
        let total_messages: usize = sessions.iter().map(|s| s.message_count).sum();
        
        let oldest_session = sessions.iter()
            .min_by_key(|s| s.created_at)
            .map(|s| s.created_at);
            
        let newest_session = sessions.iter()
            .max_by_key(|s| s.created_at)
            .map(|s| s.created_at);
            
        let most_active_session = sessions.iter()
            .max_by_key(|s| s.message_count)
            .map(|s| (s.id.clone(), s.message_count));

        Ok(SessionStatistics {
            total_sessions,
            total_messages,
            oldest_session,
            newest_session,
            most_active_session,
        })
    }

    /// Clear metadata cache
    pub fn clear_cache(&mut self) {
        self.metadata_cache.clear();
        self.cache_last_updated = None;
        log::debug!("Cleared metadata cache");
    }

    /// Create a session with a specific ID (for import purposes)
    /// This bypasses the normal unique ID generation
    pub fn create_session_with_id(&mut self, session_id: &str, name: Option<String>) -> PersistenceResult<()> {
        validate_session_id(session_id)?;
        
        // Check if session already exists
        if self.memory_store.session_exists(session_id) {
            return Err(PersistenceError::InvalidSessionId {
                session_id: format!("Session {} already exists", session_id),
            });
        }
        
        // Initialize empty conversation for the session
        self.memory_store.save_conversation(session_id, &[])?;
        
        // If a name was provided, update the metadata with the name
        if name.is_some() {
            let mut metadata = self.memory_store.get_session_metadata(session_id)?;
            metadata.name = name.clone();
            self.save_session_metadata(&metadata)?;
        }
        
        log::info!("Created session with specific ID: {} (name: {:?})", session_id, name);
        Ok(())
    }
}

