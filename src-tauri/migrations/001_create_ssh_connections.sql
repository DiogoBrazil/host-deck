CREATE TABLE ssh_connections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 22 CHECK (port BETWEEN 1 AND 65535),
    username TEXT NOT NULL,
    auth_method TEXT NOT NULL CHECK (auth_method IN ('password', 'private_key')),
    identity_file TEXT,
    group_name TEXT NOT NULL DEFAULT 'Geral',
    notes TEXT,
    password_secret_key TEXT,
    key_passphrase_secret_key TEXT,
    last_connected_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_conn_group ON ssh_connections(group_name);
