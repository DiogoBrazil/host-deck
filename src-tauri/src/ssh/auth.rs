use std::sync::Arc;

use zeroize::Zeroizing;

use crate::domain::{AuthMethod, SshConnection};
use crate::error::{AppError, AppResult};
use crate::infra::credential_store::{CredentialStore, password_ref};
use crate::ssh::client::AuthSpec;

/// Resolves the authentication material for a stored connection using the
/// keyring, producing the `AuthSpec` consumed by `ssh::client::connect`.
///
/// Shared by the terminal and SFTP commands so both resolve credentials the
/// same way.
pub fn resolve_auth(
    conn: &SshConnection,
    store: &Arc<dyn CredentialStore>,
) -> AppResult<AuthSpec> {
    match conn.auth_method {
        AuthMethod::Password => {
            let secret_ref = conn
                .password_secret_key
                .clone()
                .unwrap_or_else(|| password_ref(&conn.id));
            let password = store.get(&secret_ref)?.ok_or_else(|| {
                AppError::Ssh(
                    "Senha não encontrada no armazenamento seguro. \
                     Edite a conexão e cadastre a senha novamente."
                        .into(),
                )
            })?;
            Ok(AuthSpec::Password(Zeroizing::new(password)))
        }
        AuthMethod::PrivateKey => {
            let path = conn.identity_file.clone().ok_or_else(|| {
                AppError::Ssh("Conexão sem caminho de chave privada cadastrado.".into())
            })?;
            let passphrase = match &conn.key_passphrase_secret_key {
                Some(secret_ref) => store.get(secret_ref)?.map(Zeroizing::new),
                None => None,
            };
            Ok(AuthSpec::PrivateKey { path, passphrase })
        }
    }
}
