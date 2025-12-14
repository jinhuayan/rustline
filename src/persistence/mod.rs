use std::path::PathBuf;
use thiserror::Error;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

pub mod memory_store;
pub mod session_manager;
pub mod preference_manager;

/// Comprehensive error type for all persistence operations
#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("Session not found: {session_id}")]
    SessionNotFound { session_id: String },
    
    #[error("Invalid session ID: {session_id}")]
    InvalidSessionId { session_id: String },
    
    #[error("Corrupted data file: {path}")]
    CorruptedData { path: String },
    
    #[error("Directory creation failed: {path}")]
    DirectoryCreation { path: String },
    
    #[error("Import validation failed: {reason}")]
    ImportValidation { reason: String },
}

/// Result type alias for persistence operations
pub type PersistenceResult<T> = Result<T, PersistenceError>;

/// Export/Import data structures for complete data export
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExportData {
    pub version: String,
    pub exported_at: DateTime<Utc>,
    pub sessions: Vec<SessionExport>,
    pub preferences: crate::persistence::preference_manager::UserPreferences,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionExport {
    pub metadata: crate::persistence::memory_store::SessionInfo,
    pub messages: Vec<crate::ollama::Message>,
}

/// Utility function to ensure a directory exists, creating it if necessary
pub fn ensure_directory(path: &PathBuf) -> PersistenceResult<()> {
    if !path.exists() {
        std::fs::create_dir_all(path).map_err(|e| {
            log::error!("Failed to create directory {}: {}", path.display(), e);
            PersistenceError::DirectoryCreation {
                path: path.to_string_lossy().to_string(),
            }
        })?;
        log::info!("Created directory: {}", path.display());
    }
    Ok(())
}

/// Utility function to validate session ID format
pub fn validate_session_id(session_id: &str) -> PersistenceResult<()> {
    if session_id.is_empty() || session_id.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err(PersistenceError::InvalidSessionId {
            session_id: session_id.to_string(),
        });
    }
    Ok(())
}

/// Utility function for atomic file operations using temporary files
pub fn atomic_write<F>(file_path: &PathBuf, write_fn: F) -> PersistenceResult<()>
where
    F: FnOnce(&mut std::fs::File) -> std::io::Result<()>,
{
    let temp_path = file_path.with_extension("tmp");
    
    // Write to temporary file first
    {
        let mut temp_file = std::fs::File::create(&temp_path)?;
        write_fn(&mut temp_file)?;
        temp_file.sync_all()?; // Ensure data is written to disk
    }
    
    // Atomically rename temporary file to final location
    std::fs::rename(&temp_path, file_path)?;
    
    log::debug!("Atomically wrote file: {}", file_path.display());
    Ok(())
}

