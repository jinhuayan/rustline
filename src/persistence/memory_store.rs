use std::path::PathBuf;
use std::fs;
use std::io::Write;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use crate::ollama::Message;
use super::{PersistenceError, PersistenceResult, ensure_directory, validate_session_id, atomic_write};

/// Session metadata for tracking session information
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct SessionMetadata {
    pub id: String,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_modified: DateTime<Utc>,
}

/// Session information with additional statistics
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_modified: DateTime<Utc>,
    pub message_count: usize,
}

/// Low-level storage operations for persistent memory
pub struct MemoryStore {
    base_dir: PathBuf,
}

impl MemoryStore {
    /// Create a new MemoryStore with the specified base directory
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Ensure all necessary directories exist
    pub fn ensure_directories(&self) -> PersistenceResult<()> {
        ensure_directory(&self.base_dir)?;
        ensure_directory(&self.sessions_dir())?;
        ensure_directory(&self.metadata_dir())?;
        Ok(())
    }

    /// Get the sessions directory path
    fn sessions_dir(&self) -> PathBuf {
        self.base_dir.join("sessions")
    }

    /// Get the metadata directory path
    fn metadata_dir(&self) -> PathBuf {
        self.base_dir.join("metadata")
    }

    /// Get the conversation file path for a session
    pub fn conversation_file_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{}.json", session_id))
    }

    /// Get the metadata file path for a session
    fn metadata_file_path(&self, session_id: &str) -> PathBuf {
        self.metadata_dir().join(format!("{}_meta.json", session_id))
    }

    /// Save a complete conversation to storage using atomic operations
    pub fn save_conversation(&self, session_id: &str, messages: &[Message]) -> PersistenceResult<()> {
        validate_session_id(session_id)?;
        self.ensure_directories()?;

        let conversation_path = self.conversation_file_path(session_id);
        
        // Use atomic write to ensure data integrity
        atomic_write(&conversation_path, |file| {
            let json_data = serde_json::to_string_pretty(messages)?;
            file.write_all(json_data.as_bytes())?;
            Ok(())
        })?;

        // Update session metadata
        self.update_session_metadata(session_id, messages.len())?;

        log::debug!("Saved conversation for session {} with {} messages", session_id, messages.len());
        Ok(())
    }

    /// Load a conversation from storage
    pub fn load_conversation(&self, session_id: &str) -> PersistenceResult<Vec<Message>> {
        validate_session_id(session_id)?;
        
        let conversation_path = self.conversation_file_path(session_id);
        
        if !conversation_path.exists() {
            log::debug!("No conversation file found for session {}", session_id);
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&conversation_path)
            .map_err(|e| {
                log::error!("Failed to read conversation file for session {}: {}", session_id, e);
                PersistenceError::Io(e)
            })?;

        let messages: Vec<Message> = serde_json::from_str(&content)
            .map_err(|e| {
                log::error!("Failed to parse conversation file for session {}: {}", session_id, e);
                PersistenceError::CorruptedData {
                    path: conversation_path.to_string_lossy().to_string(),
                }
            })?;

        log::debug!("Loaded conversation for session {} with {} messages", session_id, messages.len());
        Ok(messages)
    }

    /// Append a single message to an existing conversation
    pub fn append_message(&self, session_id: &str, message: &Message) -> PersistenceResult<()> {
        validate_session_id(session_id)?;
        self.ensure_directories()?;

        // Load existing messages
        let mut messages = self.load_conversation(session_id)?;
        
        // Add the new message
        messages.push(message.clone());
        
        // Save the updated conversation
        self.save_conversation(session_id, &messages)?;

        log::debug!("Appended message to session {}", session_id);
        Ok(())
    }

    /// Check if a session exists
    pub fn session_exists(&self, session_id: &str) -> bool {
        if validate_session_id(session_id).is_err() {
            return false;
        }
        
        let conversation_path = self.conversation_file_path(session_id);
        conversation_path.exists()
    }

    /// Get session metadata
    pub fn get_session_metadata(&self, session_id: &str) -> PersistenceResult<SessionMetadata> {
        validate_session_id(session_id)?;
        
        let metadata_path = self.metadata_file_path(session_id);
        
        if !metadata_path.exists() {
            // If metadata doesn't exist but session does, create default metadata
            if self.session_exists(session_id) {
                let messages = self.load_conversation(session_id)?;
                let metadata = SessionMetadata {
                    id: session_id.to_string(),
                    name: None,
                    created_at: Utc::now(),
                    last_modified: Utc::now(),
                };
                
                // Save the metadata for future use
                self.save_session_metadata(&metadata)?;
                self.update_session_metadata(session_id, messages.len())?;
                
                return Ok(metadata);
            } else {
                return Err(PersistenceError::SessionNotFound {
                    session_id: session_id.to_string(),
                });
            }
        }

        let content = fs::read_to_string(&metadata_path)?;
        let metadata: SessionMetadata = serde_json::from_str(&content)
            .map_err(|e| {
                log::error!("Failed to parse metadata file for session {}: {}", session_id, e);
                PersistenceError::CorruptedData {
                    path: metadata_path.to_string_lossy().to_string(),
                }
            })?;

        Ok(metadata)
    }

    /// Save session metadata
    fn save_session_metadata(&self, metadata: &SessionMetadata) -> PersistenceResult<()> {
        self.ensure_directories()?;
        
        let metadata_path = self.metadata_file_path(&metadata.id);
        
        atomic_write(&metadata_path, |file| {
            let json_data = serde_json::to_string_pretty(metadata)?;
            file.write_all(json_data.as_bytes())?;
            Ok(())
        })?;

        log::debug!("Saved metadata for session {}", metadata.id);
        Ok(())
    }

    /// Update session metadata with current timestamp and message count
    fn update_session_metadata(&self, session_id: &str, _message_count: usize) -> PersistenceResult<()> {
        let mut metadata = self.get_session_metadata(session_id)
            .unwrap_or_else(|_| SessionMetadata {
                id: session_id.to_string(),
                name: None,
                created_at: Utc::now(),
                last_modified: Utc::now(),
            });

        metadata.last_modified = Utc::now();
        self.save_session_metadata(&metadata)?;
        Ok(())
    }

    /// Delete all data for a session
    pub fn delete_session_data(&self, session_id: &str) -> PersistenceResult<()> {
        validate_session_id(session_id)?;
        
        let conversation_path = self.conversation_file_path(session_id);
        let metadata_path = self.metadata_file_path(session_id);
        
        // Remove conversation file if it exists
        if conversation_path.exists() {
            fs::remove_file(&conversation_path)?;
            log::debug!("Deleted conversation file for session {}", session_id);
        }
        
        // Remove metadata file if it exists
        if metadata_path.exists() {
            fs::remove_file(&metadata_path)?;
            log::debug!("Deleted metadata file for session {}", session_id);
        }
        
        Ok(())
    }

    /// List all available sessions with their information
    pub fn list_sessions(&self) -> PersistenceResult<Vec<SessionInfo>> {
        self.ensure_directories()?;
        
        let sessions_dir = self.sessions_dir();
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        
        for entry in fs::read_dir(&sessions_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                if let Some(stem) = path.file_stem() {
                    if let Some(session_id) = stem.to_str() {
                        match self.get_session_info(session_id) {
                            Ok(info) => sessions.push(info),
                            Err(e) => {
                                log::warn!("Failed to get info for session {}: {}", session_id, e);
                            }
                        }
                    }
                }
            }
        }
        
        // Sort sessions by last modified time
        sessions.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
        
        Ok(sessions)
    }

    /// Get detailed session information including message count
    pub fn get_session_info(&self, session_id: &str) -> PersistenceResult<SessionInfo> {
        let metadata = self.get_session_metadata(session_id)?;
        let messages = self.load_conversation(session_id)?;
        
        Ok(SessionInfo {
            id: metadata.id,
            name: metadata.name,
            created_at: metadata.created_at,
            last_modified: metadata.last_modified,
            message_count: messages.len(),
        })
    }
}

