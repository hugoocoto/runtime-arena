-- Self-registered participants log in with a password (argon2id, PHC string).
-- NULL for participants created by the admin, who use their token.
ALTER TABLE users ADD COLUMN password_hash TEXT;

-- One token per device a participant logged in from, so logging in on a second
-- machine doesn't log out the first.
CREATE TABLE sessions (
    token_hash TEXT    PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions (user_id);
