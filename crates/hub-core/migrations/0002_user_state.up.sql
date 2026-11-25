-- Per-user saved state (spec "Saved state and sync", items 37; 05-ARCHITECTURE). Plain DDL for
-- pgsqlite and Postgres (D-011, D-014): no column default, no function, no IF NOT EXISTS. Ids
-- and times are written by Rust. `value` is text holding JSON: the server never queries inside
-- it. `version` is the optimistic-concurrency counter (D-024's sibling: an integer CAS, not an
-- ETag). Backups are rows under `backup:*` keys and count toward the key cap.
CREATE TABLE user_state (
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  key text NOT NULL,
  value text NOT NULL,
  version integer NOT NULL,
  updated_at timestamptz NOT NULL,
  PRIMARY KEY (user_id, key)
);
-- The only index beyond the primary key: the daily backup prune scans by time (S21). Every
-- per-user statement is served by the PK.
CREATE INDEX user_state_updated_at ON user_state (updated_at);
