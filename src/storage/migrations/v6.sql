ALTER TABLE turns ADD COLUMN facts_examined INTEGER NOT NULL DEFAULT 0
    CHECK (facts_examined IN (0, 1));
UPDATE turns SET facts_examined = 1;
CREATE INDEX turns_awaiting_facts ON turns(id)
    WHERE facts_examined = 0 AND outcome = 'completed';
PRAGMA user_version = 6;
