use serde::{Deserialize, Serialize};

use super::AuthMethod;

pub const DEFAULT_PORT: u16 = 22;
pub const DEFAULT_GROUP: &str = "Geral";

/// SQLite record for an SSH connection.
///
/// Secret values are never stored here; only keyring references are persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConnection {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub identity_file: Option<String>,
    pub group_name: String,
    pub notes: Option<String>,
    pub password_secret_key: Option<String>,
    pub key_passphrase_secret_key: Option<String>,
    /// Optional AI provider bound to this server (see `agent_providers`).
    pub provider_id: Option<String>,
    pub last_connected_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Form payload received from the frontend.
///
/// Passwords and passphrases only exist in memory until they are stored in the keyring.
#[derive(Debug, Deserialize)]
pub struct ConnectionInput {
    #[serde(default)]
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub identity_file: Option<String>,
    #[serde(default)]
    pub group_name: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub save_passphrase: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

impl FieldError {
    fn new(field: &str, message: &str) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Normalized and validated connection data ready for persistence.
#[derive(Debug)]
pub struct ValidatedConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub identity_file: Option<String>,
    pub group_name: String,
    pub notes: Option<String>,
}

impl ConnectionInput {
    /// Validates and normalizes the payload.
    ///
    /// During updates, an empty password means "keep the current credential."
    pub fn validate(&self, is_update: bool) -> Result<ValidatedConnection, Vec<FieldError>> {
        let mut errors = Vec::new();

        let host = self.host.trim().to_string();
        if host.is_empty() {
            errors.push(FieldError::new("host", "Host é obrigatório."));
        }

        let username = self.username.trim().to_string();
        if username.is_empty() {
            errors.push(FieldError::new("username", "Usuário é obrigatório."));
        }

        let port = self.port.unwrap_or(DEFAULT_PORT);
        if port == 0 {
            errors.push(FieldError::new("port", "Porta deve estar entre 1 e 65535."));
        }

        let identity_file = self
            .identity_file
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);

        match self.auth_method {
            AuthMethod::Password => {
                let has_password = self
                    .password
                    .as_deref()
                    .is_some_and(|p| !p.is_empty());
                if !has_password && !is_update {
                    errors.push(FieldError::new("password", "Senha é obrigatória."));
                }
            }
            AuthMethod::PrivateKey => match &identity_file {
                None => {
                    errors.push(FieldError::new(
                        "identity_file",
                        "Caminho da chave SSH é obrigatório.",
                    ));
                }
                Some(path) => {
                    if !std::path::Path::new(path).is_file() {
                        errors.push(FieldError::new(
                            "identity_file",
                            "Arquivo de chave não encontrado.",
                        ));
                    }
                }
            },
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        let name = {
            let trimmed = self.name.trim();
            if trimmed.is_empty() {
                format!("{username}@{host}")
            } else {
                trimmed.to_string()
            }
        };

        let group_name = {
            let trimmed = self.group_name.trim();
            if trimmed.is_empty() {
                DEFAULT_GROUP.to_string()
            } else {
                trimmed.to_string()
            }
        };

        Ok(ValidatedConnection {
            name,
            host,
            port,
            username,
            auth_method: self.auth_method,
            identity_file,
            group_name,
            notes: self
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_input() -> ConnectionInput {
        ConnectionInput {
            name: String::new(),
            host: "vps.exemplo.com".into(),
            port: None,
            username: "root".into(),
            auth_method: AuthMethod::Password,
            identity_file: None,
            group_name: String::new(),
            notes: None,
            password: Some("s3cret".into()),
            passphrase: None,
            save_passphrase: false,
        }
    }

    #[test]
    fn applies_defaults() {
        let v = base_input().validate(false).unwrap();
        assert_eq!(v.name, "root@vps.exemplo.com");
        assert_eq!(v.port, 22);
        assert_eq!(v.group_name, "Geral");
    }

    #[test]
    fn rejects_missing_host_and_username() {
        let mut input = base_input();
        input.host = "  ".into();
        input.username = String::new();
        let errs = input.validate(false).unwrap_err();
        let fields: Vec<_> = errs.iter().map(|e| e.field.as_str()).collect();
        assert!(fields.contains(&"host"));
        assert!(fields.contains(&"username"));
    }

    #[test]
    fn rejects_port_zero() {
        let mut input = base_input();
        input.port = Some(0);
        let errs = input.validate(false).unwrap_err();
        assert_eq!(errs[0].field, "port");
    }

    #[test]
    fn password_required_on_create_but_not_on_update() {
        let mut input = base_input();
        input.password = None;
        assert!(input.validate(false).is_err());
        assert!(input.validate(true).is_ok());
    }

    #[test]
    fn private_key_requires_existing_file() {
        let mut input = base_input();
        input.auth_method = AuthMethod::PrivateKey;
        input.password = None;

        input.identity_file = None;
        let errs = input.validate(false).unwrap_err();
        assert_eq!(errs[0].field, "identity_file");

        input.identity_file = Some("/caminho/que/nao/existe".into());
        let errs = input.validate(false).unwrap_err();
        assert_eq!(errs[0].field, "identity_file");
    }

    #[test]
    fn deserializes_frontend_payload() {
        let json = r#"{
            "name": "",
            "host": "10.0.0.5",
            "port": null,
            "username": "ubuntu",
            "auth_method": "private_key",
            "identity_file": "/home/user/.ssh/id_ed25519",
            "group_name": "",
            "notes": null,
            "password": null,
            "passphrase": "frase",
            "save_passphrase": true
        }"#;
        let input: ConnectionInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.auth_method, AuthMethod::PrivateKey);
        assert_eq!(input.passphrase.as_deref(), Some("frase"));
        assert!(input.save_passphrase);
        assert!(input.port.is_none());
    }

    #[test]
    fn keeps_explicit_name_and_group() {
        let mut input = base_input();
        input.name = " VPS Dokploy ".into();
        input.group_name = "Clientes".into();
        let v = input.validate(false).unwrap();
        assert_eq!(v.name, "VPS Dokploy");
        assert_eq!(v.group_name, "Clientes");
    }
}
