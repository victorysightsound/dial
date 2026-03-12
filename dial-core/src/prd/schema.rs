/// SQL schema for the PRD database (prd.db).
///
/// This is a separate database from the main DIAL phase database,
/// storing structured specifications, terminology, and wizard state.
pub const SCHEMA: &str = r#"
-- Metadata key-value store
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Hierarchical document sections parsed from markdown
CREATE TABLE IF NOT EXISTS sections (
    id INTEGER PRIMARY KEY,
    section_id TEXT UNIQUE NOT NULL,
    title TEXT NOT NULL,
    parent_id TEXT,
    level INTEGER NOT NULL CHECK(level BETWEEN 1 AND 6),
    sort_order INTEGER NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    word_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%S', 'now')),
    updated_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_sections_parent ON sections(parent_id);
CREATE INDEX IF NOT EXISTS idx_sections_level ON sections(level);
CREATE INDEX IF NOT EXISTS idx_sections_sort ON sections(sort_order);

-- Full-text search index for sections
CREATE VIRTUAL TABLE IF NOT EXISTS sections_fts USING fts5(
    title,
    content,
    content='sections',
    content_rowid='id',
    tokenize='porter'
);

-- Keep FTS in sync with sections table
CREATE TRIGGER IF NOT EXISTS sections_ai AFTER INSERT ON sections BEGIN
    INSERT INTO sections_fts(rowid, title, content)
    VALUES (new.id, new.title, new.content);
END;

CREATE TRIGGER IF NOT EXISTS sections_ad AFTER DELETE ON sections BEGIN
    INSERT INTO sections_fts(sections_fts, rowid, title, content)
    VALUES ('delete', old.id, old.title, old.content);
END;

CREATE TRIGGER IF NOT EXISTS sections_au AFTER UPDATE ON sections BEGIN
    INSERT INTO sections_fts(sections_fts, rowid, title, content)
    VALUES ('delete', old.id, old.title, old.content);
    INSERT INTO sections_fts(rowid, title, content)
    VALUES (new.id, new.title, new.content);
END;

-- Canonical terminology with variant tracking
CREATE TABLE IF NOT EXISTS terminology (
    id INTEGER PRIMARY KEY,
    canonical TEXT UNIQUE NOT NULL,
    variants TEXT NOT NULL DEFAULT '[]',
    definition TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL DEFAULT 'general',
    first_used_in TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%S', 'now')),
    updated_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_terminology_category ON terminology(category);

-- Full-text search index for terminology
CREATE VIRTUAL TABLE IF NOT EXISTS terminology_fts USING fts5(
    canonical,
    definition,
    content='terminology',
    content_rowid='id',
    tokenize='porter'
);

CREATE TRIGGER IF NOT EXISTS terminology_ai AFTER INSERT ON terminology BEGIN
    INSERT INTO terminology_fts(rowid, canonical, definition)
    VALUES (new.id, new.canonical, new.definition);
END;

CREATE TRIGGER IF NOT EXISTS terminology_ad AFTER DELETE ON terminology BEGIN
    INSERT INTO terminology_fts(terminology_fts, rowid, canonical, definition)
    VALUES ('delete', old.id, old.canonical, old.definition);
END;

CREATE TRIGGER IF NOT EXISTS terminology_au AFTER UPDATE ON terminology BEGIN
    INSERT INTO terminology_fts(terminology_fts, rowid, canonical, definition)
    VALUES ('delete', old.id, old.canonical, old.definition);
    INSERT INTO terminology_fts(rowid, canonical, definition)
    VALUES (new.id, new.canonical, new.definition);
END;

-- Tracks which source files were imported
CREATE TABLE IF NOT EXISTS sources (
    id INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL,
    imported_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%S', 'now')),
    file_size INTEGER,
    modified_at TEXT
);

-- Wizard state for pause/resume
CREATE TABLE IF NOT EXISTS wizard_state (
    id INTEGER PRIMARY KEY,
    current_phase INTEGER NOT NULL DEFAULT 1,
    completed_phases TEXT NOT NULL DEFAULT '[]',
    gathered_info TEXT NOT NULL DEFAULT '{}',
    template TEXT NOT NULL DEFAULT 'spec',
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%S', 'now')),
    updated_at TEXT
);
"#;
