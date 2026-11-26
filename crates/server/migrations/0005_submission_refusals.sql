CREATE TABLE submission_refusals (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users (id),
    problem         TEXT,
    reason          TEXT NOT NULL,
    client_at       INTEGER,
    received_at     INTEGER NOT NULL,
    refused_at      INTEGER NOT NULL
);

CREATE INDEX submission_refusals_user ON submission_refusals (user_id, id);
