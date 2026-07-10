-- Optional provider/model binding per server.
ALTER TABLE ssh_connections
    ADD COLUMN provider_id TEXT REFERENCES agent_providers(id) ON DELETE SET NULL;
