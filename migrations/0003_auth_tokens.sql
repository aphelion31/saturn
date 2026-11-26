-- Short-lived credentials. Reset tokens are sensitive and must not be logged by default.
CREATE TABLE IF NOT EXISTS saturn_magic_codes (
    key text PRIMARY KEY,
    token text NOT NULL,
    attempts integer NOT NULL DEFAULT 0,
    expires_at timestamptz NOT NULL
);
