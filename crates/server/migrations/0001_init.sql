-- Times are Unix seconds (UTC).

CREATE TABLE users (
    id           INTEGER PRIMARY KEY,
    name         TEXT    NOT NULL UNIQUE,
    display_name TEXT    NOT NULL,
    -- SHA-256 of the token, hex. The token itself is never stored.
    token_hash   TEXT    NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL
);

CREATE TABLE problems (
    id               INTEGER PRIMARY KEY,
    slug             TEXT    NOT NULL UNIQUE,
    title            TEXT    NOT NULL,
    -- Every upload gets a new version directory; old ones stay for past submissions.
    version          INTEGER NOT NULL,
    dir              TEXT    NOT NULL,
    time_limit_ms    INTEGER NOT NULL,
    memory_limit_kib INTEGER NOT NULL,
    test_count       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE TABLE rounds (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    duration_s INTEGER NOT NULL,
    freeze_s   INTEGER NOT NULL,
    cooldown_s INTEGER NOT NULL,
    -- NULL until started. Stopping early sets ends_at to the stop time.
    started_at INTEGER,
    ends_at    INTEGER,
    -- When public boards freeze (NULL: no freeze). Fixed once reached, so
    -- extending a frozen round doesn't reveal anything.
    freeze_at  INTEGER,
    created_at INTEGER NOT NULL
);

CREATE TABLE round_problems (
    round_id   INTEGER NOT NULL REFERENCES rounds (id),
    problem_id INTEGER NOT NULL REFERENCES problems (id),
    position   INTEGER NOT NULL,
    PRIMARY KEY (round_id, problem_id)
);
