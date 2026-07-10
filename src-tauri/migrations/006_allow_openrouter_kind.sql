-- Adds 'openrouter' to the allowed provider kinds. SQLite cannot alter a
-- CHECK constraint, so the table is rebuilt. foreign_keys is disabled during
-- the rebuild: with it enabled, DROP TABLE would cascade-delete the model
-- cache and SET NULL every ssh_connections.provider_id.
PRAGMA foreign_keys = OFF;

CREATE TABLE agent_providers_new (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('anthropic', 'openai', 'openrouter')),
    label TEXT NOT NULL,
    -- Optional override for OpenAI/Anthropic-compatible gateways.
    base_url TEXT,
    -- Default model for this provider; each connection may override it.
    model TEXT,
    -- Keyring reference (see infra/credential_store.rs); never the key itself.
    api_key_ref TEXT,
    created_at TEXT NOT NULL
);

INSERT INTO agent_providers_new (id, kind, label, base_url, model, api_key_ref, created_at)
    SELECT id, kind, label, base_url, model, api_key_ref, created_at FROM agent_providers;

DROP TABLE agent_providers;

ALTER TABLE agent_providers_new RENAME TO agent_providers;

PRAGMA foreign_keys = ON;
