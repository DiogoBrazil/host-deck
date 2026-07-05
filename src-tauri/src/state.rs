use std::sync::Arc;

use crate::infra::credential_store::CredentialStore;

/// Cofre de credenciais gerenciado pelo Tauri (`State<CredStore>`).
pub struct CredStore(pub Arc<dyn CredentialStore>);
