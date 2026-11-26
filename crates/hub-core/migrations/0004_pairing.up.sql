-- OTP pairing (09-ARCHITECTURE, decision 1 and 2). Plain DDL for pgsqlite and Postgres (D-011,
-- D-014): no column default, no function, no IF NOT EXISTS. Ids, hashes and times are written by
-- Rust; only the sha256 of a code or a revoke secret ever lands here, never its plaintext.
--
-- `pairing_codes` reuses desktop_auth_codes' mechanics (0001): the row references the minting
-- SESSION, so signing that session out (S9) or its expiry sweep (S14) removes the live code by
-- cascade - one mechanism, no second sweeper. The code resolves to the session's user, so a code
-- minted by one account can never register a bridge for another.
CREATE TABLE pairing_codes (
  code_hash bytea PRIMARY KEY,
  session_id_hash bytea NOT NULL REFERENCES sessions(id_hash) ON DELETE CASCADE,
  expires_at timestamptz NOT NULL
);
-- The FK index, same convention as 0001/0003: without it, signing out (S9) or the expiry sweep
-- (S14) must scan pairing_codes per deleted session to enforce the cascade. expires_at is indexed
-- for P4, the sweep that removes expired codes so a 10-minute credential does not outlive itself
-- by the session's 30 days.
CREATE INDEX pairing_codes_session_id_hash ON pairing_codes (session_id_hash);
CREATE INDEX pairing_codes_expires_at ON pairing_codes (expires_at);
-- One standing revoke secret per bridge, shown once at pairing and stored hashed. A side table
-- rather than a column on mt5_bridges: the ALTER path is unproven on pgsqlite, and CREATE TABLE
-- is what every earlier migration already proves on both engines.
CREATE TABLE mt5_bridge_secrets (
  bridge_id uuid PRIMARY KEY REFERENCES mt5_bridges(id) ON DELETE CASCADE,
  revoke_hash bytea NOT NULL
);
