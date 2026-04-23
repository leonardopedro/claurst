//! Password replacement utilities for HTTP request layer

use claurst_core::password_store::{replace_placeholders, PasswordStore};
use std::sync::Arc;
use tracing::warn;

/// Replaces password placeholders in request payload before sending to HTTP layer
/// 
/// # Arguments
/// * `payload` - Request body or headers containing placeholders
/// * `password_store` - Password store (optional)
/// * `destination_domain` - Target domain for domain filtering
/// 
/// # Returns
/// Payload with matching placeholders replaced
pub fn replace_passwords_in_payload(
    payload: &str, 
    password_store: &Option<Arc<dyn PasswordStore>>,
    destination_domain: &str
) -> String {
    match password_store {
        Some(store) => {
            match replace_placeholders(payload, store.as_ref(), destination_domain) {
                Ok(replaced) => replaced,
                Err(e) => {
                    warn!("Password replacement failed for domain {}: {}", destination_domain, e);
                    payload.to_string()
                }
            }
        }
        None => payload.to_string(),
    }
}

/// Extract domain from URL
/// 
/// # Examples
/// * `https://api.example.com/v1/users` → `api.example.com`
/// * `http://localhost:8080` → `localhost`
/// * `api.example.com` → `api.example.com`
pub fn extract_domain_from_url(url: &str) -> Option<String> {
    // Handle plain domain (no protocol)
    if !url.contains("://") {
        let domain = url.split('/').next().unwrap_or(url);
        return Some(domain.split(':').next().unwrap_or(domain).to_string());
    }
    
    // Extract from URL with protocol
    url.split("://")
        .nth(1)
        .map(|rest| {
            let domain_part = rest.split('/').next().unwrap_or(rest);
            domain_part.split(':').next().unwrap_or(domain_part).to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_domain_url() {
        assert_eq!(
            extract_domain_from_url("https://api.example.com/v1/users"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            extract_domain_from_url("http://localhost:8080"),
            Some("localhost".to_string())
        );
        assert_eq!(
            extract_domain_from_url("https://api.example.com"),
            Some("api.example.com".to_string())
        );
    }

    #[test]
    fn test_extract_domain_plain() {
        assert_eq!(
            extract_domain_from_url("api.example.com"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            extract_domain_from_url("api.example.com/v1"),
            Some("api.example.com".to_string())
        );
    }
}
