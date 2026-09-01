use crate::core::mcp::oauth_provider::MCPOAuthConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ResourceMismatchError {
    pub message: String,
}

impl std::fmt::Display for ResourceMismatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ResourceMismatchError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthAuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
    pub revocation_endpoint: Option<String>,
    pub revocation_endpoint_auth_methods_supported: Option<Vec<String>>,
    pub registration_endpoint: Option<String>,
    pub response_types_supported: Option<Vec<String>>,
    pub grant_types_supported: Option<Vec<String>>,
    pub code_challenge_methods_supported: Option<Vec<String>>,
    pub scopes_supported: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProtectedResourceMetadata {
    pub resource: String,
    pub authorization_servers: Option<Vec<String>>,
    pub bearer_methods_supported: Option<Vec<String>>,
    pub resource_documentation: Option<String>,
    pub resource_signing_alg_values_supported: Option<Vec<String>>,
    pub resource_encryption_alg_values_supported: Option<Vec<String>>,
    pub resource_encryption_enc_values_supported: Option<Vec<String>>,
}

pub const FIVE_MIN_BUFFER_MS: u64 = 5 * 60 * 1000;

pub struct OAuthUtils;

impl OAuthUtils {
    pub fn build_well_known_urls(base_url: &str, include_path_suffix: bool) -> WellKnownUrls {
        let server_url = url::Url::parse(base_url).unwrap();
        let base = format!(
            "{}://{}",
            server_url.scheme(),
            server_url.host_str().unwrap()
        );

        if !include_path_suffix {
            return WellKnownUrls {
                protected_resource: format!("{}/.well-known/oauth-protected-resource", base),
                authorization_server: format!("{}/.well-known/oauth-authorization-server", base),
            };
        }

        let path_suffix = server_url.path().trim_end_matches('/');
        WellKnownUrls {
            protected_resource: format!(
                "{}/.well-known/oauth-protected-resource{}",
                base, path_suffix
            ),
            authorization_server: format!(
                "{}/.well-known/oauth-authorization-server{}",
                base, path_suffix
            ),
        }
    }

    pub fn metadata_to_oauth_config(metadata: &OAuthAuthorizationServerMetadata) -> MCPOAuthConfig {
        MCPOAuthConfig {
            authorization_url: Some(metadata.authorization_endpoint.clone()),
            token_url: Some(metadata.token_endpoint.clone()),
            scopes: metadata.scopes_supported.clone(),
            registration_url: metadata.registration_endpoint.clone(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct WellKnownUrls {
    pub protected_resource: String,
    pub authorization_server: String,
}

impl Default for MCPOAuthConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            client_id: None,
            client_secret: None,
            authorization_url: None,
            token_url: None,
            scopes: None,
            audiences: None,
            redirect_uri: None,
            token_param_name: None,
            registration_url: None,
        }
    }
}
