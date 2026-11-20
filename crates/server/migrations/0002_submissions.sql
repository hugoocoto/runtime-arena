CREATE TABLE submissions (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users (id),
    round_id        INTEGER NOT NULL REFERENCES rounds (id),
    problem_id      INTEGER NOT NULL REFERENCES problems (id),
    -- The problem version judged against, so later uploads don't change old results.
    problem_version INTEGER NOT NULL,
    problem_dir     TEXT    NOT NULL,
    status          TEXT    NOT NULL CHECK (status IN ('queued', 'judging', 'done', 'failed')),
    -- arena_protocol::Verdict in snake_case, once judged.
    verdict         TEXT,
    time_ms         INTEGER,
    memory_kib      INTEGER,
    -- arena_protocol::JudgeReport as JSON.
    report          TEXT,
    -- Why judging failed (status 'failed'); for admins only.
    error           TEXT,
    submitted_at    INTEGER NOT NULL,
    judged_at       INTEGER
);

CREATE INDEX submissions_queue ON submissions (status, id);
CREATE INDEX submissions_board ON submissions (round_id, problem_id, user_id);
