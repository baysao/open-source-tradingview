-- Accounts (spec "Phase 3 facts and decisions", R3). Plain DDL for pgsqlite and Postgres (D-011,
-- D-014): no column default, no function, no IF NOT EXISTS. Ids and times are written by Rust.
CREATE TABLE users (
  id uuid PRIMARY KEY,
  email text NOT NULL,
  email_verified boolean NOT NULL,
  name text NULL,
  picture text NULL,
  created_at timestamptz NOT NULL,
  last_login_at timestamptz NOT NULL
);
CREATE TABLE identities (
  provider text NOT NULL,
  subject text NOT NULL,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL,
  PRIMARY KEY (provider, subject)
);
CREATE INDEX identities_user_id ON identities (user_id);
CREATE TABLE sessions (
  id_hash bytea PRIMARY KEY,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL,
  last_seen_at timestamptz NOT NULL,
  user_agent text NULL
);
CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE TABLE desktop_auth_codes (
  code_hash bytea PRIMARY KEY,
  desktop_key_hash bytea NOT NULL,
  session_id_hash bytea NOT NULL REFERENCES sessions(id_hash) ON DELETE CASCADE,
  expires_at timestamptz NOT NULL
);
