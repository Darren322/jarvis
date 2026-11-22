ALTER TABLE memory_records ADD COLUMN correction_state TEXT NOT NULL DEFAULT 'none'
    CHECK (correction_state IN ('none', 'applied', 'needs_review'));
ALTER TABLE memory_records ADD COLUMN supersedes_id INTEGER REFERENCES memory_records(id);
PRAGMA user_version = 3;
