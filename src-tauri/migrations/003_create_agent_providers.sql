CREATE TABLE agent_providers (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('anthropic', 'openai')),
    label TEXT NOT NULL,
    -- Optional override for OpenAI/Anthropic-compatible gateways.
    base_url TEXT,
    -- Default model for this provider; each connection may override it.
    model TEXT,
    -- Keyring reference (see infra/credential_store.rs); never the key itself.
    api_key_ref TEXT,
    created_at TEXT NOT NULL
);
