-- Cache of GET /v1/models per provider. The UI renders its controls from these
-- capabilities instead of a hardcoded model list.
CREATE TABLE agent_model_cache (
    provider_id TEXT NOT NULL REFERENCES agent_providers(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL,
    display_name TEXT,
    max_input_tokens INTEGER,
    max_output_tokens INTEGER,
    -- Provider capability tree as returned by the API, serialized as JSON.
    capabilities TEXT NOT NULL DEFAULT '{}',
    -- Fetch timestamp (RFC 3339), used for cache invalidation.
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (provider_id, model_id)
);
