use std::sync::Arc;

use crate::infra::credential_store::CredentialStore;

/// Credential store managed as Tauri state.
pub struct CredStore(pub Arc<dyn CredentialStore>);
