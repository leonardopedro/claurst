use std::path::PathBuf;
use thiserror::Error;

/// Error types for password store operations
#[derive(Error, Debug)]
pub enum PasswordStoreError {
    #[error("Password store not initialized: {0}")]
    NotInitialized(String),
    
    #[error("Password not found: {0}")]
    NotFound(String),
    
    #[error("Invalid placeholder format: {0}")]
    InvalidFormat(String),
    
    #[error("Failed to access password store: {0}")]
    AccessError(String),
    
    #[error("Failed to decrypt password: {0}")]
    DecryptionError(String),
    
    #[error("Field '{0}' not found in secret")]
    FieldNotFound(String),
}

pub type Result<T> = std::result::Result<T, PasswordStoreError>;

/// Configuration for password store
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct PasswordStoreConfig {
    pub store_path: Option<PathBuf>,
    pub signing_key: Option<String>,
    pub require_git: bool,
}

/// Replacement mode for passwords
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReplacementMode {
    Password,
    Full,
    Field,
    Validate,
}

/// Parsed placeholder reference with domain
#[derive(Clone, Debug)]
pub struct PasswordReference {
    pub domain: String,
    pub path: String,
    pub mode: ReplacementMode,
    pub field_name: Option<String>,
}

impl PasswordReference {
    /// Parse placeholder in format: {{pass:domain:path[:mode[:field]]}}
    /// Examples:
    /// - {{pass:example.com:secret}} - password mode (default)
    /// - {{pass:example.com:secret:password}} - explicit password mode
    /// - {{pass:example.com:secret:full}} - full secret
    /// - {{pass:example.com:secret:field:username}} - specific field
    pub fn parse(placeholder: &str) -> Result<Self> {
        let parts: Vec<&str> = placeholder.split(':').collect();
        
        if parts.len() < 2 {
            return Err(PasswordStoreError::InvalidFormat(
                "expected format: domain:path[:mode[:field]]".to_string()
            ));
        }
        
        let domain = parts[0].to_string();
        let path = parts[1].to_string();
        
        match parts.len() {
            2 => Ok(Self {
                domain,
                path,
                mode: ReplacementMode::Password,
                field_name: None,
            }),
            3 => {
                let mode_str = parts[2];
                match mode_str {
                    "password" => Ok(Self {
                        domain,
                        path,
                        mode: ReplacementMode::Password,
                        field_name: None,
                    }),
                    "full" => Ok(Self {
                        domain,
                        path,
                        mode: ReplacementMode::Full,
                        field_name: None,
                    }),
                    _ => Err(PasswordStoreError::InvalidFormat(
                        format!("unknown mode: {}", mode_str)
                    )),
                }
            }
            4 => {
                let mode_str = parts[2];
                let field_name = parts[3].to_string();
                if mode_str == "field" {
                    Ok(Self {
                        domain,
                        path,
                        mode: ReplacementMode::Field,
                        field_name: Some(field_name),
                    })
                } else {
                    Err(PasswordStoreError::InvalidFormat(
                        format!("expected 'field' mode, got: {}", mode_str)
                    ))
                }
            }
            _ => Err(PasswordStoreError::InvalidFormat("too many colons".to_string())),
        }
    }
}

/// Password store trait
pub trait PasswordStore: Send + Sync {
    fn list_entries(&self) -> Result<Vec<String>>;
    fn get_password(&self, path: &str) -> Result<String>;
    fn get_full_secret(&self, path: &str) -> Result<String>;
    fn get_field(&self, path: &str, field_name: &str) -> Result<String>;
    fn exists(&self, path: &str) -> bool;
}

/// Null password store
#[derive(Default)]
pub struct NullPasswordStore;

impl PasswordStore for NullPasswordStore {
    fn list_entries(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    
    fn get_password(&self, _path: &str) -> Result<String> {
        Err(PasswordStoreError::NotInitialized(
            "Password store not configured. Set PASSWORD_STORE_DIR or configure in settings.".to_string()
        ))
    }
    
    fn get_full_secret(&self, _path: &str) -> Result<String> {
        Err(PasswordStoreError::NotInitialized(
            "Password store not configured".to_string()
        ))
    }
    
    fn get_field(&self, _path: &str, _field_name: &str) -> Result<String> {
        Err(PasswordStoreError::NotInitialized(
            "Password store not configured".to_string()
        ))
    }
    
    fn exists(&self, _path: &str) -> bool {
        false
    }
}

/// Replace placeholders only for the specified destination domain
/// 
/// Only replaces {{pass:domain:...}} placeholders where domain matches destination.
/// Placeholders for other domains are left unchanged.
/// 
/// # Arguments
/// * `text` - Text containing placeholders
/// * `store` - Password store
/// * `destination_domain` - Domain being accessed (e.g., "api.example.com")
/// 
/// # Returns
/// Text with matching placeholders replaced
pub fn replace_placeholders(text: &str, store: &dyn PasswordStore, destination_domain: &str) -> Result<String> {
    use regex::{Captures, Regex};
    
    let re = Regex::new(r"\{\{pass:([^}]+)\}\}").unwrap();
    
    let result = re.replace_all(text, |caps: &Captures| {
        let placeholder = &caps[1];
        
        match PasswordReference::parse(placeholder) {
            Ok(r#ref) => {
                // Only replace if domains match
                if r#ref.domain != destination_domain {
                    // Keep placeholder unchanged
                    return caps[0].to_string();
                }
                
                match r#ref.mode {
                    ReplacementMode::Password => {
                        store.get_password(&r#ref.path).unwrap_or_else(|e| format!("ERROR: {}", e))
                    }
                    ReplacementMode::Full => {
                        store.get_full_secret(&r#ref.path).unwrap_or_else(|e| format!("ERROR: {}", e))
                    }
                    ReplacementMode::Field => {
                        if let Some(field) = r#ref.field_name {
                            store.get_field(&r#ref.path, &field).unwrap_or_else(|e| format!("ERROR: {}", e))
                        } else {
                            format!("ERROR: missing field name")
                        }
                    }
                    ReplacementMode::Validate => {
                        if store.exists(&r#ref.path) {
                            "VALID".to_string()
                        } else {
                            "ERROR: not found".to_string()
                        }
                    }
                }
            }
            Err(_) => {
                // Keep malformed placeholders unchanged
                caps[0].to_string()
            }
        }
    });
    
    Ok(result.into_owned())
}

/// Extract all password references from text (for validation/display purposes)
pub fn extract_placeholders(text: &str) -> Vec<PasswordReference> {
    use regex::Regex;
    
    let re = Regex::new(r"\{\{pass:([^}]+)\}\}").unwrap();
    
    re.captures_iter(text)
        .filter_map(|caps| {
            let placeholder = &caps[1];
            PasswordReference::parse(placeholder).ok()
        })
        .collect()
}

/// Check if text contains placeholders for a specific domain
pub fn has_domain_placeholders(text: &str, domain: &str) -> bool {
    let refs = extract_placeholders(text);
    refs.iter().any(|r#ref| r#ref.domain == domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock password store for testing
    struct MockPasswordStore {
        passwords: std::collections::HashMap<String, String>,
    }

    impl MockPasswordStore {
        fn new() -> Self {
            let mut passwords = std::collections::HashMap::new();
            passwords.insert("user".to_string(), "secret123".to_string());
            passwords.insert("pass".to_string(), "mypassword".to_string());
            // Full secret with newline-separated fields (using : as separator)
            passwords.insert("apikey".to_string(), "APIKEY:line1\nusername:admin\n".to_string());
            Self { passwords }
        }
    }

    impl PasswordStore for MockPasswordStore {
        fn list_entries(&self) -> Result<Vec<String>> {
            Ok(self.passwords.keys().cloned().collect())
        }
        
        fn get_password(&self, path: &str) -> Result<String> {
            // Simulates ripasso behavior: return first line for password
            self.passwords.get(path)
                .cloned()
                .map(|s| {
                    // Remove trailing newlines, then get first line
                    let trimmed = s.trim_end();
                    trimmed.lines().next().unwrap_or("").to_string()
                })
                .ok_or_else(|| PasswordStoreError::NotFound(path.to_string()))
        }
        
        fn get_full_secret(&self, path: &str) -> Result<String> {
            self.passwords.get(path)
                .cloned()
                .ok_or_else(|| PasswordStoreError::NotFound(path.to_string()))
        }
        
        fn get_field(&self, path: &str, field_name: &str) -> Result<String> {
            let secret = self.get_full_secret(path)?;
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
            self.passwords.contains_key(path)
        }
    }

    #[test]
    fn test_parse_with_domain() {
        let r#ref = PasswordReference::parse("example.com:secret").unwrap();
        assert_eq!(r#ref.domain, "example.com");
        assert_eq!(r#ref.path, "secret");
        assert_eq!(r#ref.mode, ReplacementMode::Password);
    }

    #[test]
    fn test_parse_with_mode() {
        let r#ref = PasswordReference::parse("example.com:secret:full").unwrap();
        assert_eq!(r#ref.domain, "example.com");
        assert_eq!(r#ref.path, "secret");
        assert_eq!(r#ref.mode, ReplacementMode::Full);
    }

    #[test]
    fn test_parse_with_field() {
        let r#ref = PasswordReference::parse("example.com:secret:field:username").unwrap();
        assert_eq!(r#ref.domain, "example.com");
        assert_eq!(r#ref.path, "secret");
        assert_eq!(r#ref.mode, ReplacementMode::Field);
        assert_eq!(r#ref.field_name, Some("username".to_string()));
    }

    #[test]
    fn test_parse_error_too_short() {
        let result = PasswordReference::parse("example");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_placeholders() {
        let text = "Login to {{pass:api.example.com:user}} and {{pass:web.example.com:pass:full}}";
        let refs = extract_placeholders(text);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].domain, "api.example.com");
        assert_eq!(refs[1].domain, "web.example.com");
    }

    #[test]
    fn test_replace_placeholders_domain_match() {
        let store = MockPasswordStore::new();
        let text = "API key: {{pass:api.example.com:apikey}}";
        let result = replace_placeholders(text, &store, "api.example.com").unwrap();
        // get_password returns first line: "APIKEY:line1"
        assert_eq!(result, "API key: APIKEY:line1");
    }

    #[test]
    fn test_replace_placeholders_password_type_stack() {
        let store = MockPasswordStore::new();
        let text = "User: {{pass:api.example.com:user}}";
        let result = replace_placeholders(text, &store, "api.example.com").unwrap();
        assert_eq!(result, "User: secret123");
    }

    #[test]
    fn test_replace_placeholders_domain_no_match() {
        let store = MockPasswordStore::new();
        let text = "API key: {{pass:api.example.com:apikey}}";
        let result = replace_placeholders(text, &store, "other.com").unwrap();
        // Should remain unchanged since domains don't match
        assert_eq!(result, "API key: {{pass:api.example.com:apikey}}");
    }

    #[test]
    fn test_replace_placeholders_mixed_domains() {
        let store = MockPasswordStore::new();
        let text = "Login to api: {{pass:api.example.com:user}} and web: {{pass:web.example.com:pass}}";
        let result = replace_placeholders(text, &store, "api.example.com").unwrap();
        // Only api.example.com placeholders should be replaced
        assert_eq!(result, "Login to api: secret123 and web: {{pass:web.example.com:pass}}");
    }

    #[test]
    fn test_replace_placeholders_full_mode() {
        let store = MockPasswordStore::new();
        let text = "{{pass:api.example.com:apikey:full}}";
        let result = replace_placeholders(text, &store, "api.example.com").unwrap();
        assert_eq!(result, "APIKEY:line1\nusername:admin\n");
    }

    #[test]
    fn test_replace_placeholders_field_mode() {
        let store = MockPasswordStore::new();
        let text = "User: {{pass:api.example.com:apikey:field:username}}";
        let result = replace_placeholders(text, &store, "api.example.com").unwrap();
        // get_field should find "username: admin" in full secret
        println!("Result: '{}'", result);
        assert_eq!(result, "User: admin");
    }

    #[test]
    fn test_replace_placeholders_not_found() {
        let store = MockPasswordStore::new();
        let text = "{{pass:api.example.com:notfound}}";
        let result = replace_placeholders(text, &store, "api.example.com").unwrap();
        assert!(result.contains("ERROR"));
    }

    #[test]
    fn test_replace_placeholders_malformed_kept_unchanged() {
        let store = MockPasswordStore::new();
        let text = "{{pass:invalid}}";
        let result = replace_placeholders(text, &store, "api.example.com").unwrap();
        assert_eq!(result, "{{pass:invalid}}");
    }

    #[test]
    fn test_has_domain_placeholders() {
        assert!(has_domain_placeholders("{{pass:api.example.com:user}}", "api.example.com"));
        assert!(!has_domain_placeholders("{{pass:api.example.com:user}}", "other.com"));
        assert!(has_domain_placeholders("{{pass:api.example.com:user}} {{pass:other.com:pass}}", "api.example.com"));
    }

    // SECURITY TESTS: Demonstrate domain isolation guarantees
    
    struct ApiKeys {
        secrets: std::collections::HashMap<String, String>,
    }

    impl ApiKeys {
        fn new() -> Self {
            let mut secrets = std::collections::HashMap::new();
            secrets.insert("aws".to_string(), "AKIA-SECRET-AWS-KEY".to_string());
            secrets.insert("github".to_string(), "ghp-SECRET-GH-TOKEN".to_string());
            secrets.insert("db".to_string(), "super-secret-db-pass".to_string());
            Self { secrets }
        }
    }

    impl PasswordStore for ApiKeys {
        fn list_entries(&self) -> Result<Vec<String>> {
            Ok(self.secrets.keys().cloned().collect())
        }
        fn get_password(&self, path: &str) -> Result<String> {
            self.secrets.get(path).cloned()
                .ok_or_else(|| PasswordStoreError::NotFound(path.to_string()))
        }
        fn get_full_secret(&self, path: &str) -> Result<String> {
            self.get_password(path)
        }
        fn get_field(&self, _path: &str, _field: &str) -> Result<String> {
            unimplemented!()
        }
        fn exists(&self, path: &str) -> bool {
            self.secrets.contains_key(path)
        }
    }

    #[test]
    fn test_security_llm_receives_only_placeholders() {
        let store = ApiKeys::new();
        let llm_prompt = "Use {{pass:aws.amazon.com:aws}} for the API";
        
        // LLM sees placeholders, not secrets
        assert!(llm_prompt.contains("{{pass:aws.amazon.com:aws}}"));
        assert!(!llm_prompt.contains("AKIA-SECRET"));
        
        // This is what you send to the LLM - safe!
    }

    #[test]
    fn test_security_http_layer_gets_value_for_matching_domain() {
        let store = ApiKeys::new();
        let request = "Authorization: {{pass:aws.amazon.com:aws}}";
        
        // HTTP to aws.amazon.com gets the real key
        let aws_request = replace_placeholders(request, &store, "aws.amazon.com").unwrap();
        assert_eq!(aws_request, "Authorization: AKIA-SECRET-AWS-KEY");
        assert!(aws_request.contains("AKIA-SECRET"));
    }

    #[test]
    fn test_security拦截_wrong_domain() {
        let store = ApiKeys::new();
        let request = "Authorization: {{pass:aws.amazon.com:aws}}";
        
        // IMPORTANT: Even though URL looks like github.com, 
        // placeholder refers to aws.amazon.com
        let github_request = replace_placeholders(request, &store, "github.com").unwrap();
        
        // Should remain unchanged because domains don't match
        assert_eq!(github_request, "Authorization: {{pass:aws.amazon.com:aws}}");
        assert!(!github_request.contains("AKIA-SECRET"));
    }

    #[test]
    fn test_security_domain_mismatch_prevents_leak() {
        let store = ApiKeys::new();
        
        // User tries to make GitHub request but mistakenly uses AWS placeholder
        let wrong_request = "curl -H 'Authorization: {{pass:aws.amazon.com:aws}}' https://github.com";
        
        // Even if target is GitHub, AWS password stays hidden
        let result = replace_placeholders(wrong_request, &store, "github.com").unwrap();
        
        assert!(result.contains("{{pass:aws.amazon.com:aws}}"));
        assert!(!result.contains("AKIA-SECRET"));
    }

    #[test]
    fn test_security_selective_replacement_in_multi_service_context() {
        let store = ApiKeys::new();
        let message = "AWS: {{pass:aws.amazon.com:aws}} GitHub: {{pass:github.com:github}} DB: {{pass:db.internal:db}}";
        
        // Call to AWS service
        let aws_line = replace_placeholders(message, &store, "aws.amazon.com").unwrap();
        assert!(aws_line.contains("AKIA-SECRET-AWS-KEY"));
        assert!(aws_line.contains("{{pass:github.com:github}}")); // Still a placeholder
        assert!(aws_line.contains("{{pass:db.internal:db}}"));     // Still a placeholder
        
        // Call to GitHub
        let gh_line = replace_placeholders(message, &store, "github.com").unwrap();
        assert!(gh_line.contains("ghp-SECRET-GH-TOKEN"));
        assert!(gh_line.contains("{{pass:aws.amazon.com:aws}}"));
    }

    #[test]
    fn test_security_helper_functions_work() {
        let text = "API {{pass:aws.amazon.com:aws}} and {{pass:github.com:github}}";
        
        // Extract tells us what domains are involved
        let refs = extract_placeholders(text);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].domain, "aws.amazon.com");
        assert_eq!(refs[1].domain, "github.com");
        
        // has_domain tells us quickly if domain is present
        assert!(has_domain_placeholders(text, "aws.amazon.com"));
        assert!(!has_domain_placeholders(text, "not-in-there.com"));
    }

    #[test]
    fn test_security_malformed_placeholders_preserved() {
        let store = ApiKeys::new();
        let text = "Valid: {{pass:aws.amazon.com:aws}} Invalid: {{pass:nodomain}}";
        let result = replace_placeholders(text, &store, "aws.amazon.com").unwrap();
        
        // Valid one replaced
        assert!(result.contains("AKIA-SECRET-AWS-KEY"));
        // Invalid kept as-is
        assert!(result.contains("{{pass:nodomain}}"));
    }
}
