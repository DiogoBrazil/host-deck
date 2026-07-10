-- Generic key/value store for app-level flags. First consumer: the one-time
-- consent before terminal content is sent to an AI provider (Fase 4).
CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
