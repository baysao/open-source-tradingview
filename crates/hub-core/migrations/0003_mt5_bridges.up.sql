-- Per-user MT5 bridges (spec "MT5 bridges"; 07-ARCHITECTURE). Plain DDL for pgsqlite and Postgres
-- (D-011, D-014): no column default, no function, no IF NOT EXISTS. Ids and times are written by
-- Rust. `token_enc` is the 24-byte XChaCha20-Poly1305 nonce followed by the ciphertext of the
-- bridge token (the key derives from VTV_SECRET_KEY; the plaintext never touches the database).
-- `symbols_json` is the cached /symbols answer from the last successful test (D-032: /api/symbols
-- never makes a live bridge call). Deleting a user cascades (CUJ 8).
CREATE TABLE mt5_bridges (
  id uuid PRIMARY KEY,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  label text NOT NULL,
  url text NOT NULL,
  token_enc bytea NOT NULL,
  symbols_json text,
  last_ok_at timestamptz,
  last_error text,
  created_at timestamptz NOT NULL
);
-- The only index beyond the primary key: the per-user list and the owner checks (B2, B3, B4, B5).
CREATE INDEX mt5_bridges_user_id ON mt5_bridges (user_id);
