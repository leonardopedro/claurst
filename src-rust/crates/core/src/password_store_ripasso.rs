//! Ripasso-backed password store implementation using external GPG.
//!
//! This module provides a PasswordStore implementation compatible with ripasso
//! and pass with maximum portability by using the system's GPG installation
//! rather than linking against GPG libraries.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use crate::password_store::{PasswordStore, PasswordStoreError, Result};

/// Ripasso-backed password store that uses system GPG
pub struct RipassoPasswordStore {
    /// Path to the password store directory
    store_path: PathBuf,
    /// Inner state for thread safety
    inner: Mutex<Inner>,
}

struct Inner {
    /// Cache of password paths
    entries: Vec<String>,
}

impl RipassoPasswordStore {
    /// Create a new ripasso password store
    pub fn new<P: Into<PathBuf>>(store_path: P) -> Result<Self> {
        let path = store_path.into();
        
        if !path.exists() {
            return Err(PasswordStoreError::NotInitialized(
                format!("Password store directory does not exist: {}", path.display())
            ));
        }
        
        // Verify it looks like a pass store (has .gpg-id file)
        let gpg_id_file = path.join(".gpg-id");
        if !gpg_id_file.exists() {
            return Err(PasswordStoreError::NotInitialized(
                format!("Not a valid pass store: missing .gpg-id file at {}", path.display())
            ));
        }
        
        // Verify gpg is available
        if Command::new("gpg").arg("--version").output().is_err() {
            return Err(PasswordStoreError::AccessError(
                "gpg command not found. Please install GPG.".to_string()
            ));
        }
        
        let mut inner = Inner {
            entries: Vec::new(),
        };
        
        // Populate entries by walking the directory
        inner.entries = Self::scan_directory(&path, &path)?;
        
        Ok(Self {
            store_path: path,
            inner: Mutex::new(inner),
        })
    }
    
    /// Scan directory recursively for .gpg files
    fn scan_directory(root: &Path, dir: &Path) -> Result<Vec<String>> {
        let mut entries = Vec::new();
        
        for entry in std::fs::read_dir(dir).map_err(|e| PasswordStoreError::AccessError(e.to_string()))? {
            let entry = entry.map_err(|e| PasswordStoreError::AccessError(e.to_string()))?;
            let path = entry.path();
            
            if path.is_dir() {
                // Skip git directory
                if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                    continue;
                }
                entries.extend(Self::scan_directory(root, &path)?);
            } else if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "gpg" {
                        // Get relative path without .gpg extension
                        let rel_path = path.strip_prefix(root).unwrap();
                        let mut rel_str = rel_path.to_string_lossy().to_string();
                        // Remove .gpg extension
                        if rel_str.ends_with(".gpg") {
                            rel_str.truncate(rel_str.len() - 4);
                        }
                        entries.push(rel_str);
                    }
                }
            }
        }
        
        Ok(entries)
    }
    
    /// Decrypt a password file using system gpg
    fn decrypt_file(&self, path: &str) -> Result<String> {
        let mut full_path = self.store_path.join(path);
        full_path.set_extension("gpg");
        
        if !full_path.exists() {
            return Err(PasswordStoreError::NotFound(path.to_string()));
        }
        
        let output = Command::new("gpg")
            .args(&["--batch", "--yes", "--passphrase-fd", "0", "--decrypt", full_path.to_str().unwrap()])
            .output();
            
        match output {
            Ok(output) if output.status.success() => {
                String::from_utf8(output.stdout)
                    .map_err(|e| PasswordStoreError::DecryptionError(e.to_string()))
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(PasswordStoreError::DecryptionError(format!("GPG error: {}", stderr)))
            }
            Err(e) => Err(PasswordStoreError::DecryptionError(e.to_string())),
        }
    }
    
    /// Refresh the password list from disk
    pub fn refresh(&self) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.entries = Self::scan_directory(&self.store_path, &self.store_path)?;
        Ok(())
    }
}

impl PasswordStore for RipassoPasswordStore {
    fn list_entries(&self) -> Result<Vec<String>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.entries.clone())
    }
    
    fn get_password(&self, path: &str) -> Result<String> {
        let secret = self.decrypt_file(path)?;
        let password = secret.lines().next().unwrap_or("").to_string();
        Ok(password)
    }
    
    fn get_full_secret(&self, path: &str) -> Result<String> {
        self.decrypt_file(path)
    }
    
    fn get_field(&self, path: &str, field_name: &str) -> Result<String> {
        let secret = self.decrypt_file(path)?;
        
        // Look for "fieldname: value" or "fieldname: value\n"
        for line in secret.lines() {
            if let Some(colon_pos) = line.find(':') {
                let key = &line[..colon_pos].trim();
                let value = &line[colon_pos + 1..].trim();
                if key.eq_ignore_ascii_case(field_name) {
                    return Ok(value.to_string());
                }
            }
        }
        
        Err(PasswordStoreError::FieldNotFound(field_name.to_string()))
    }
    
    fn exists(&self, path: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.entries.contains(&path.to_string())
    }
}
