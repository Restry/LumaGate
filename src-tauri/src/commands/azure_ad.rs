//! Azure AD OAuth state management.

use std::sync::Arc;

use crate::proxy::providers::azure_ad_auth::{AzureAdManager, SharedAzureAdManager};

/// Tauri managed state for Azure AD token management.
pub struct AzureAdState(pub SharedAzureAdManager);

/// Create a new Azure AD manager instance.
pub fn new_manager(client: reqwest::Client) -> Arc<AzureAdManager> {
    crate::proxy::providers::azure_ad_auth::new_shared(client)
}
