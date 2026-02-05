pub const SCHEMA: &str = r#"
-- Configuration
CREATE TABLE IF NOT EXISTS config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT DEFAULT CURRENT_TIMESTAMP
);

-- Spec sections (indexed from markdown)
CREATE TABLE IF NOT EXISTS spec_sections (
    id INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL,
    heading_path TEXT NOT NULL,
    level INTEGER NOT NULL,
    content TEXT NOT NULL,
    indexed_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE VIRTUAL TABLE IF NOT EXISTS spec_sections_fts USING fts5(
    heading_path, content,
    content='spec_sections', content_rowid='id',
    tokenize='porter'
);

CREATE TRIGGER IF NOT EXISTS spec_sections_ai AFTER INSERT ON spec_sections BEGIN
    INSERT INTO spec_sections_fts(rowid, heading_path, content)
    VALUES (NEW.id, NEW.heading_path, NEW.content);
END;

CREATE TRIGGER IF NOT EXISTS spec_sections_ad AFTER DELETE ON spec_sections BEGIN
    INSERT INTO spec_sections_fts(spec_sections_fts, rowid, heading_path, content)
    VALUES('delete', OLD.id, OLD.heading_path, OLD.content);
END;

CREATE TRIGGER IF NOT EXISTS spec_sections_au AFTER UPDATE ON spec_sections BEGIN
    INSERT INTO spec_sections_fts(spec_sections_fts, rowid, heading_path, content)
    VALUES('delete', OLD.id, OLD.heading_path, OLD.content);
    INSERT INTO spec_sections_fts(rowid, heading_path, content)
    VALUES (NEW.id, NEW.heading_path, NEW.content);
END;

-- Tasks (replaces fix_plan.md)
CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY,
    description TEXT NOT NULL,
    status TEXT DEFAULT 'pending'
        CHECK(status IN ('pending', 'in_progress', 'completed', 'blocked', 'cancelled')),
    priority INTEGER DEFAULT 5,
    blocked_by TEXT,
    spec_section_id INTEGER,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    started_at TEXT,
    completed_at TEXT,
    FOREIGN KEY (spec_section_id) REFERENCES spec_sections(id)
);

CREATE VIRTUAL TABLE IF NOT EXISTS tasks_fts USING fts5(
    description,
    content='tasks', content_rowid='id',
    tokenize='porter'
);

CREATE TRIGGER IF NOT EXISTS tasks_ai AFTER INSERT ON tasks BEGIN
    INSERT INTO tasks_fts(rowid, description) VALUES (NEW.id, NEW.description);
END;

CREATE TRIGGER IF NOT EXISTS tasks_ad AFTER DELETE ON tasks BEGIN
    INSERT INTO tasks_fts(tasks_fts, rowid, description)
    VALUES('delete', OLD.id, OLD.description);
END;

CREATE TRIGGER IF NOT EXISTS tasks_au AFTER UPDATE ON tasks BEGIN
    INSERT INTO tasks_fts(tasks_fts, rowid, description)
    VALUES('delete', OLD.id, OLD.description);
    INSERT INTO tasks_fts(rowid, description) VALUES (NEW.id, NEW.description);
END;

-- Iterations (each loop cycle)
CREATE TABLE IF NOT EXISTS iterations (
    id INTEGER PRIMARY KEY,
    task_id INTEGER NOT NULL,
    status TEXT DEFAULT 'in_progress'
        CHECK(status IN ('in_progress', 'completed', 'failed', 'reverted')),
    attempt_number INTEGER DEFAULT 1,
    started_at TEXT DEFAULT CURRENT_TIMESTAMP,
    ended_at TEXT,
    duration_seconds REAL,
    commit_hash TEXT,
    notes TEXT,
    FOREIGN KEY (task_id) REFERENCES tasks(id)
);

-- Actions (what was attempted)
CREATE TABLE IF NOT EXISTS actions (
    id INTEGER PRIMARY KEY,
    iteration_id INTEGER NOT NULL,
    action_type TEXT NOT NULL,
    description TEXT NOT NULL,
    file_path TEXT,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (iteration_id) REFERENCES iterations(id)
);

-- Outcomes (what happened)
CREATE TABLE IF NOT EXISTS outcomes (
    id INTEGER PRIMARY KEY,
    action_id INTEGER NOT NULL,
    success INTEGER NOT NULL,
    output_summary TEXT,
    error_message TEXT,
    duration_seconds REAL,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (action_id) REFERENCES actions(id)
);

-- Failure patterns (categorized failure types)
CREATE TABLE IF NOT EXISTS failure_patterns (
    id INTEGER PRIMARY KEY,
    pattern_key TEXT UNIQUE NOT NULL,
    description TEXT NOT NULL,
    category TEXT,
    occurrence_count INTEGER DEFAULT 0,
    first_seen_at TEXT DEFAULT CURRENT_TIMESTAMP,
    last_seen_at TEXT
);

-- Failures (specific failure instances)
CREATE TABLE IF NOT EXISTS failures (
    id INTEGER PRIMARY KEY,
    iteration_id INTEGER NOT NULL,
    pattern_id INTEGER,
    error_text TEXT NOT NULL,
    file_path TEXT,
    line_number INTEGER,
    resolved INTEGER DEFAULT 0,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    resolved_at TEXT,
    resolved_by_solution_id INTEGER,
    FOREIGN KEY (iteration_id) REFERENCES iterations(id),
    FOREIGN KEY (pattern_id) REFERENCES failure_patterns(id),
    FOREIGN KEY (resolved_by_solution_id) REFERENCES solutions(id)
);

CREATE VIRTUAL TABLE IF NOT EXISTS failures_fts USING fts5(
    error_text,
    content='failures', content_rowid='id',
    tokenize='porter'
);

CREATE TRIGGER IF NOT EXISTS failures_ai AFTER INSERT ON failures BEGIN
    INSERT INTO failures_fts(rowid, error_text) VALUES (NEW.id, NEW.error_text);
END;

CREATE TRIGGER IF NOT EXISTS failures_ad AFTER DELETE ON failures BEGIN
    INSERT INTO failures_fts(failures_fts, rowid, error_text)
    VALUES('delete', OLD.id, OLD.error_text);
END;

CREATE TRIGGER IF NOT EXISTS failures_au AFTER UPDATE ON failures BEGIN
    INSERT INTO failures_fts(failures_fts, rowid, error_text)
    VALUES('delete', OLD.id, OLD.error_text);
    INSERT INTO failures_fts(rowid, error_text) VALUES (NEW.id, NEW.error_text);
END;

-- Solutions (fixes with earned trust)
CREATE TABLE IF NOT EXISTS solutions (
    id INTEGER PRIMARY KEY,
    pattern_id INTEGER NOT NULL,
    description TEXT NOT NULL,
    code_example TEXT,
    confidence REAL DEFAULT 0.3,
    success_count INTEGER DEFAULT 0,
    failure_count INTEGER DEFAULT 0,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    last_used_at TEXT,
    FOREIGN KEY (pattern_id) REFERENCES failure_patterns(id)
);

CREATE VIRTUAL TABLE IF NOT EXISTS solutions_fts USING fts5(
    description, code_example,
    content='solutions', content_rowid='id',
    tokenize='porter'
);

CREATE TRIGGER IF NOT EXISTS solutions_ai AFTER INSERT ON solutions BEGIN
    INSERT INTO solutions_fts(rowid, description, code_example)
    VALUES (NEW.id, NEW.description, COALESCE(NEW.code_example, ''));
END;

CREATE TRIGGER IF NOT EXISTS solutions_ad AFTER DELETE ON solutions BEGIN
    INSERT INTO solutions_fts(solutions_fts, rowid, description, code_example)
    VALUES('delete', OLD.id, OLD.description, COALESCE(OLD.code_example, ''));
END;

CREATE TRIGGER IF NOT EXISTS solutions_au AFTER UPDATE ON solutions BEGIN
    INSERT INTO solutions_fts(solutions_fts, rowid, description, code_example)
    VALUES('delete', OLD.id, OLD.description, COALESCE(OLD.code_example, ''));
    INSERT INTO solutions_fts(rowid, description, code_example)
    VALUES (NEW.id, NEW.description, COALESCE(NEW.code_example, ''));
END;

-- Solution applications (tracking when solutions were used)
CREATE TABLE IF NOT EXISTS solution_applications (
    id INTEGER PRIMARY KEY,
    solution_id INTEGER NOT NULL,
    failure_id INTEGER NOT NULL,
    iteration_id INTEGER NOT NULL,
    success INTEGER,
    applied_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (solution_id) REFERENCES solutions(id),
    FOREIGN KEY (failure_id) REFERENCES failures(id),
    FOREIGN KEY (iteration_id) REFERENCES iterations(id)
);

-- Learnings (project-specific operational knowledge - AGENT.md equivalent)
CREATE TABLE IF NOT EXISTS learnings (
    id INTEGER PRIMARY KEY,
    category TEXT,
    description TEXT NOT NULL,
    discovered_at TEXT DEFAULT CURRENT_TIMESTAMP,
    times_referenced INTEGER DEFAULT 0
);

CREATE VIRTUAL TABLE IF NOT EXISTS learnings_fts USING fts5(
    category, description,
    content='learnings', content_rowid='id',
    tokenize='porter'
);

CREATE TRIGGER IF NOT EXISTS learnings_ai AFTER INSERT ON learnings BEGIN
    INSERT INTO learnings_fts(rowid, category, description)
    VALUES (NEW.id, COALESCE(NEW.category, ''), NEW.description);
END;

CREATE TRIGGER IF NOT EXISTS learnings_ad AFTER DELETE ON learnings BEGIN
    INSERT INTO learnings_fts(learnings_fts, rowid, category, description)
    VALUES('delete', OLD.id, COALESCE(OLD.category, ''), OLD.description);
END;

CREATE TRIGGER IF NOT EXISTS learnings_au AFTER UPDATE ON learnings BEGIN
    INSERT INTO learnings_fts(learnings_fts, rowid, category, description)
    VALUES('delete', OLD.id, COALESCE(OLD.category, ''), OLD.description);
    INSERT INTO learnings_fts(rowid, category, description)
    VALUES (NEW.id, COALESCE(NEW.category, ''), NEW.description);
END;
"#;
