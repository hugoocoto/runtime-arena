-- CPU times are microseconds now, so ties at a millisecond (common with
-- parallel programs and fast tests) still rank correctly. Old rows held
-- milliseconds; scale them. Reports stored as JSON before this migration keep
-- their old field names and no longer deserialize, but the submission's
-- verdict, time and memory remain available.
ALTER TABLE submissions RENAME COLUMN time_ms TO time_us;
UPDATE submissions SET time_us = time_us * 1000 WHERE time_us IS NOT NULL;
