#!/usr/bin/env python3
"""
DIAL - Deterministic Iterative Agent Loop

A methodology for running AI coding agents autonomously with:
- SQLite + FTS5 for selective recall
- Trust-based solution learning
- Phase support for staged development
"""

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

# ============================================================
# CONFIGURATION
# ============================================================

VERSION = "1.1.0"
DEFAULT_PHASE = "default"
DEFAULT_BUILD_TIMEOUT = 600  # 10 minutes
DEFAULT_TEST_TIMEOUT = 600   # 10 minutes
MAX_FIX_ATTEMPTS = 3
TRUST_THRESHOLD = 0.6
TRUST_INCREMENT = 0.15
TRUST_DECREMENT = 0.20
INITIAL_CONFIDENCE = 0.3

# ============================================================
# COLORED OUTPUT
# ============================================================

def supports_color():
    """Check if terminal supports color."""
    if os.environ.get("NO_COLOR"):
        return False
    if not hasattr(sys.stdout, "isatty"):
        return False
    return sys.stdout.isatty()

USE_COLOR = supports_color()

def green(text):
    return f"\033[32m{text}\033[0m" if USE_COLOR else text

def red(text):
    return f"\033[31m{text}\033[0m" if USE_COLOR else text

def yellow(text):
    return f"\033[33m{text}\033[0m" if USE_COLOR else text

def blue(text):
    return f"\033[34m{text}\033[0m" if USE_COLOR else text

def bold(text):
    return f"\033[1m{text}\033[0m" if USE_COLOR else text

def dim(text):
    return f"\033[2m{text}\033[0m" if USE_COLOR else text

# ============================================================
# DATABASE SCHEMA
# ============================================================

SCHEMA = """
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
"""

# ============================================================
# DATABASE OPERATIONS
# ============================================================

def get_dial_dir():
    """Get the .dial directory for the current project."""
    return Path.cwd() / ".dial"

def get_db_path(phase=None):
    """Get the database path for the given phase."""
    dial_dir = get_dial_dir()
    if phase is None:
        # Try to read current phase from a marker file
        phase_file = dial_dir / "current_phase"
        if phase_file.exists():
            phase = phase_file.read_text().strip()
        else:
            phase = DEFAULT_PHASE
    return dial_dir / f"{phase}.db"

def get_current_phase():
    """Get the current phase name."""
    dial_dir = get_dial_dir()
    phase_file = dial_dir / "current_phase"
    if phase_file.exists():
        return phase_file.read_text().strip()
    return DEFAULT_PHASE

def set_current_phase(phase):
    """Set the current phase."""
    dial_dir = get_dial_dir()
    phase_file = dial_dir / "current_phase"
    phase_file.write_text(phase)

def get_db(phase=None):
    """Get a database connection."""
    db_path = get_db_path(phase)
    if not db_path.exists():
        print(red(f"Error: DIAL not initialized. Run 'dial init' first."))
        sys.exit(1)
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    # Run migrations for schema updates
    migrate_db(conn)
    return conn

def migrate_db(conn):
    """Apply any needed schema migrations."""
    # Check if learnings table exists
    cursor = conn.execute("""
        SELECT name FROM sqlite_master
        WHERE type='table' AND name='learnings'
    """)
    if not cursor.fetchone():
        # Add learnings table and FTS
        conn.executescript("""
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
        """)
        conn.commit()

def init_db(phase=DEFAULT_PHASE, import_solutions_from=None):
    """Initialize the database with schema."""
    dial_dir = get_dial_dir()
    dial_dir.mkdir(exist_ok=True)

    db_path = dial_dir / f"{phase}.db"

    if db_path.exists():
        print(yellow(f"Warning: Database {db_path} already exists."))
        response = input("Overwrite? [y/N]: ").strip().lower()
        if response != 'y':
            print("Aborted.")
            return False
        db_path.unlink()

    conn = sqlite3.connect(str(db_path))
    conn.executescript(SCHEMA)

    # Set default config
    now = datetime.now().isoformat()
    defaults = [
        ('phase', phase),
        ('project_name', Path.cwd().name),
        ('build_cmd', ''),
        ('test_cmd', ''),
        ('build_timeout', str(DEFAULT_BUILD_TIMEOUT)),
        ('test_timeout', str(DEFAULT_TEST_TIMEOUT)),
        ('created_at', now),
    ]
    conn.executemany(
        "INSERT INTO config (key, value) VALUES (?, ?)",
        defaults
    )

    # Import solutions from another phase if requested
    if import_solutions_from:
        source_db_path = dial_dir / f"{import_solutions_from}.db"
        if not source_db_path.exists():
            print(red(f"Error: Source phase '{import_solutions_from}' not found."))
            conn.close()
            db_path.unlink()
            return False

        source_conn = sqlite3.connect(str(source_db_path))
        source_conn.row_factory = sqlite3.Row

        # Copy trusted solutions and their failure patterns
        cursor = source_conn.execute("""
            SELECT fp.* FROM failure_patterns fp
            INNER JOIN solutions s ON s.pattern_id = fp.id
            WHERE s.confidence >= ?
        """, (TRUST_THRESHOLD,))
        patterns = cursor.fetchall()

        pattern_id_map = {}
        for p in patterns:
            conn.execute("""
                INSERT INTO failure_patterns (pattern_key, description, category, occurrence_count, first_seen_at, last_seen_at)
                VALUES (?, ?, ?, ?, ?, ?)
            """, (p['pattern_key'], p['description'], p['category'], p['occurrence_count'], p['first_seen_at'], p['last_seen_at']))
            pattern_id_map[p['id']] = conn.execute("SELECT last_insert_rowid()").fetchone()[0]

        cursor = source_conn.execute("""
            SELECT * FROM solutions WHERE confidence >= ?
        """, (TRUST_THRESHOLD,))
        solutions = cursor.fetchall()

        for s in solutions:
            new_pattern_id = pattern_id_map.get(s['pattern_id'])
            if new_pattern_id:
                conn.execute("""
                    INSERT INTO solutions (pattern_id, description, code_example, confidence, success_count, failure_count, created_at, last_used_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """, (new_pattern_id, s['description'], s['code_example'], s['confidence'], s['success_count'], s['failure_count'], s['created_at'], s['last_used_at']))

        source_conn.close()
        print(green(f"Imported {len(solutions)} trusted solutions from '{import_solutions_from}'."))

    conn.commit()
    conn.close()

    # Set current phase
    set_current_phase(phase)

    print(green(f"Initialized DIAL database: {db_path}"))
    return True

# ============================================================
# CONFIG MANAGEMENT
# ============================================================

def config_get(key):
    """Get a config value."""
    conn = get_db()
    cursor = conn.execute("SELECT value FROM config WHERE key = ?", (key,))
    row = cursor.fetchone()
    conn.close()
    return row['value'] if row else None

def config_set(key, value):
    """Set a config value."""
    conn = get_db()
    conn.execute("""
        INSERT INTO config (key, value, updated_at) VALUES (?, ?, ?)
        ON CONFLICT(key) DO UPDATE SET value = ?, updated_at = ?
    """, (key, value, datetime.now().isoformat(), value, datetime.now().isoformat()))
    conn.commit()
    conn.close()

def config_show():
    """Show all config values."""
    conn = get_db()
    cursor = conn.execute("SELECT key, value FROM config ORDER BY key")
    rows = cursor.fetchall()
    conn.close()

    print(bold("DIAL Configuration"))
    print("=" * 40)
    for row in rows:
        print(f"  {row['key']}: {row['value']}")

# ============================================================
# TASK MANAGEMENT
# ============================================================

def task_add(description, priority=5, spec_section_id=None):
    """Add a new task."""
    conn = get_db()
    conn.execute("""
        INSERT INTO tasks (description, priority, spec_section_id)
        VALUES (?, ?, ?)
    """, (description, priority, spec_section_id))
    task_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]
    conn.commit()
    conn.close()
    print(green(f"Added task #{task_id}: {description}"))
    return task_id

def task_list(show_all=False):
    """List tasks."""
    conn = get_db()
    if show_all:
        cursor = conn.execute("""
            SELECT id, description, status, priority, blocked_by, created_at
            FROM tasks ORDER BY priority, id
        """)
    else:
        cursor = conn.execute("""
            SELECT id, description, status, priority, blocked_by, created_at
            FROM tasks WHERE status NOT IN ('completed', 'cancelled')
            ORDER BY priority, id
        """)
    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(dim("No tasks found."))
        return

    print(bold("Tasks"))
    print("=" * 60)

    status_colors = {
        'pending': dim,
        'in_progress': yellow,
        'completed': green,
        'blocked': red,
        'cancelled': dim,
    }

    for row in rows:
        status_fn = status_colors.get(row['status'], str)
        status_str = status_fn(f"[{row['status']}]")
        priority_str = f"P{row['priority']}" if row['priority'] != 5 else ""
        blocked_str = red(f" (blocked: {row['blocked_by']})") if row['blocked_by'] else ""
        print(f"  #{row['id']:3} {status_str:20} {priority_str:4} {row['description']}{blocked_str}")

def task_next():
    """Get the next task to work on."""
    conn = get_db()
    cursor = conn.execute("""
        SELECT id, description, priority, spec_section_id
        FROM tasks WHERE status = 'pending'
        ORDER BY priority, id LIMIT 1
    """)
    row = cursor.fetchone()
    conn.close()

    if not row:
        print(dim("No pending tasks."))
        return None

    print(bold("Next task:"))
    print(f"  #{row['id']}: {row['description']}")
    if row['spec_section_id']:
        print(dim(f"  Spec section: {row['spec_section_id']}"))
    return dict(row)

def task_done(task_id):
    """Mark a task as completed."""
    conn = get_db()
    conn.execute("""
        UPDATE tasks SET status = 'completed', completed_at = ?
        WHERE id = ?
    """, (datetime.now().isoformat(), task_id))
    if conn.total_changes == 0:
        print(red(f"Task #{task_id} not found."))
    else:
        print(green(f"Task #{task_id} marked as completed."))
    conn.commit()
    conn.close()

def task_block(task_id, reason):
    """Mark a task as blocked."""
    conn = get_db()
    conn.execute("""
        UPDATE tasks SET status = 'blocked', blocked_by = ?
        WHERE id = ?
    """, (reason, task_id))
    if conn.total_changes == 0:
        print(red(f"Task #{task_id} not found."))
    else:
        print(yellow(f"Task #{task_id} marked as blocked: {reason}"))
    conn.commit()
    conn.close()

def task_cancel(task_id):
    """Cancel a task."""
    conn = get_db()
    conn.execute("""
        UPDATE tasks SET status = 'cancelled'
        WHERE id = ?
    """, (task_id,))
    if conn.total_changes == 0:
        print(red(f"Task #{task_id} not found."))
    else:
        print(dim(f"Task #{task_id} cancelled."))
    conn.commit()
    conn.close()

def task_search(query):
    """Search tasks using FTS5."""
    conn = get_db()
    cursor = conn.execute("""
        SELECT t.id, t.description, t.status, t.priority
        FROM tasks t
        INNER JOIN tasks_fts fts ON t.id = fts.rowid
        WHERE tasks_fts MATCH ?
        ORDER BY rank
    """, (query,))
    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(dim(f"No tasks matching '{query}'."))
        return

    print(bold(f"Tasks matching '{query}':"))
    for row in rows:
        print(f"  #{row['id']} [{row['status']}] {row['description']}")

# ============================================================
# SPEC INDEXER
# ============================================================

def parse_markdown_sections(file_path):
    """Parse a markdown file into sections based on headers."""
    sections = []
    current_headers = []
    current_content = []
    current_level = 0

    with open(file_path, 'r') as f:
        lines = f.readlines()

    for line in lines:
        header_match = re.match(r'^(#{1,6})\s+(.+)$', line)

        if header_match:
            # Save previous section if exists
            if current_headers:
                sections.append({
                    'heading_path': ' > '.join(current_headers),
                    'level': current_level,
                    'content': ''.join(current_content).strip()
                })

            level = len(header_match.group(1))
            title = header_match.group(2).strip()

            # Adjust header stack based on level
            while len(current_headers) >= level:
                current_headers.pop()
            current_headers.append(title)

            current_level = level
            current_content = []
        else:
            current_content.append(line)

    # Save last section
    if current_headers:
        sections.append({
            'heading_path': ' > '.join(current_headers),
            'level': current_level,
            'content': ''.join(current_content).strip()
        })

    return sections

def index_specs(specs_dir="specs"):
    """Index all markdown files in the specs directory."""
    specs_path = Path(specs_dir)

    if not specs_path.exists():
        print(red(f"Specs directory '{specs_dir}' not found."))
        return False

    conn = get_db()

    # Clear existing spec sections
    conn.execute("DELETE FROM spec_sections")

    md_files = list(specs_path.glob("**/*.md"))

    if not md_files:
        print(yellow(f"No markdown files found in '{specs_dir}'."))
        conn.close()
        return True

    total_sections = 0
    cwd = Path.cwd().resolve()

    for md_file in md_files:
        md_file_resolved = md_file.resolve()
        try:
            relative_path = str(md_file_resolved.relative_to(cwd))
        except ValueError:
            # Fallback: just use the path relative to specs_dir
            relative_path = str(md_file.relative_to(specs_path.parent) if specs_path.is_absolute() else md_file)
        sections = parse_markdown_sections(md_file)

        for section in sections:
            if section['content']:  # Only index non-empty sections
                conn.execute("""
                    INSERT INTO spec_sections (file_path, heading_path, level, content)
                    VALUES (?, ?, ?, ?)
                """, (relative_path, section['heading_path'], section['level'], section['content']))
                total_sections += 1

    conn.commit()
    conn.close()

    print(green(f"Indexed {total_sections} sections from {len(md_files)} files."))
    return True

def spec_search(query):
    """Search specs using FTS5."""
    conn = get_db()
    cursor = conn.execute("""
        SELECT s.id, s.file_path, s.heading_path, s.content
        FROM spec_sections s
        INNER JOIN spec_sections_fts fts ON s.id = fts.rowid
        WHERE spec_sections_fts MATCH ?
        ORDER BY rank
        LIMIT 10
    """, (query,))
    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(dim(f"No spec sections matching '{query}'."))
        return []

    print(bold(f"Spec sections matching '{query}':"))
    print("=" * 60)

    for row in rows:
        section_id_str = f"[{row['id']}]"
        print(f"\n{blue(section_id_str)} {bold(row['heading_path'])}")
        print(dim(f"    File: {row['file_path']}"))
        # Show first 200 chars of content
        preview = row['content'][:200] + "..." if len(row['content']) > 200 else row['content']
        print(f"    {preview}")

    return [dict(row) for row in rows]

def spec_show(section_id):
    """Show a specific spec section."""
    conn = get_db()
    cursor = conn.execute("""
        SELECT * FROM spec_sections WHERE id = ?
    """, (section_id,))
    row = cursor.fetchone()
    conn.close()

    if not row:
        print(red(f"Spec section #{section_id} not found."))
        return None

    print(bold(row['heading_path']))
    print(dim(f"File: {row['file_path']}"))
    print("=" * 60)
    print(row['content'])

    return dict(row)

def spec_list():
    """List all indexed spec sections."""
    conn = get_db()
    cursor = conn.execute("""
        SELECT id, file_path, heading_path, level
        FROM spec_sections ORDER BY file_path, id
    """)
    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(dim("No spec sections indexed. Run 'dial index' first."))
        return

    print(bold("Indexed Spec Sections"))
    print("=" * 60)

    current_file = None
    for row in rows:
        if row['file_path'] != current_file:
            current_file = row['file_path']
            print(f"\n{blue(current_file)}")
        indent = "  " * row['level']
        print(f"  {indent}[{row['id']}] {row['heading_path']}")

# ============================================================
# FAILURE PATTERN DETECTION
# ============================================================

def detect_failure_pattern(error_text):
    """Detect failure pattern from error text."""
    patterns = [
        (r'ImportError', 'ImportError', 'import'),
        (r'ModuleNotFoundError', 'ModuleNotFoundError', 'import'),
        (r'SyntaxError', 'SyntaxError', 'syntax'),
        (r'IndentationError', 'IndentationError', 'syntax'),
        (r'NameError', 'NameError', 'runtime'),
        (r'TypeError', 'TypeError', 'runtime'),
        (r'ValueError', 'ValueError', 'runtime'),
        (r'AttributeError', 'AttributeError', 'runtime'),
        (r'KeyError', 'KeyError', 'runtime'),
        (r'IndexError', 'IndexError', 'runtime'),
        (r'FileNotFoundError', 'FileNotFoundError', 'runtime'),
        (r'PermissionError', 'PermissionError', 'runtime'),
        (r'ConnectionError', 'ConnectionError', 'runtime'),
        (r'TimeoutError', 'TimeoutError', 'runtime'),
        (r'FAILED.*test_', 'TestFailure', 'test'),
        (r'AssertionError', 'AssertionError', 'test'),
        (r'error\[E\d+\]', 'RustCompileError', 'build'),
        (r'error: could not compile', 'RustCompileError', 'build'),
        (r'npm ERR!', 'NpmError', 'build'),
        (r'tsc.*error TS\d+', 'TypeScriptError', 'build'),
        (r'cargo build.*failed', 'CargoBuildError', 'build'),
    ]

    for regex, pattern_key, category in patterns:
        if re.search(regex, error_text, re.IGNORECASE):
            return pattern_key, category

    return 'UnknownError', 'unknown'

def get_or_create_failure_pattern(conn, pattern_key, category):
    """Get or create a failure pattern."""
    cursor = conn.execute(
        "SELECT id FROM failure_patterns WHERE pattern_key = ?",
        (pattern_key,)
    )
    row = cursor.fetchone()

    if row:
        conn.execute("""
            UPDATE failure_patterns
            SET occurrence_count = occurrence_count + 1,
                last_seen_at = ?
            WHERE id = ?
        """, (datetime.now().isoformat(), row['id']))
        return row['id']
    else:
        conn.execute("""
            INSERT INTO failure_patterns (pattern_key, description, category)
            VALUES (?, ?, ?)
        """, (pattern_key, f"Auto-detected {pattern_key}", category))
        return conn.execute("SELECT last_insert_rowid()").fetchone()[0]

def record_failure(conn, iteration_id, error_text, file_path=None, line_number=None):
    """Record a failure."""
    pattern_key, category = detect_failure_pattern(error_text)
    pattern_id = get_or_create_failure_pattern(conn, pattern_key, category)

    conn.execute("""
        INSERT INTO failures (iteration_id, pattern_id, error_text, file_path, line_number)
        VALUES (?, ?, ?, ?, ?)
    """, (iteration_id, pattern_id, error_text, file_path, line_number))

    failure_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]
    return failure_id, pattern_id

# ============================================================
# SOLUTION MANAGEMENT
# ============================================================

def find_trusted_solutions(conn, pattern_id):
    """Find trusted solutions for a failure pattern."""
    cursor = conn.execute("""
        SELECT * FROM solutions
        WHERE pattern_id = ? AND confidence >= ?
        ORDER BY confidence DESC
    """, (pattern_id, TRUST_THRESHOLD))
    return cursor.fetchall()

def record_solution(conn, pattern_id, description, code_example=None):
    """Record a new solution."""
    conn.execute("""
        INSERT INTO solutions (pattern_id, description, code_example)
        VALUES (?, ?, ?)
    """, (pattern_id, description, code_example))
    return conn.execute("SELECT last_insert_rowid()").fetchone()[0]

def apply_solution_success(conn, solution_id):
    """Record successful application of a solution."""
    conn.execute("""
        UPDATE solutions
        SET confidence = MIN(1.0, confidence + ?),
            success_count = success_count + 1,
            last_used_at = ?
        WHERE id = ?
    """, (TRUST_INCREMENT, datetime.now().isoformat(), solution_id))

def apply_solution_failure(conn, solution_id):
    """Record failed application of a solution."""
    conn.execute("""
        UPDATE solutions
        SET confidence = MAX(0.0, confidence - ?),
            failure_count = failure_count + 1,
            last_used_at = ?
        WHERE id = ?
    """, (TRUST_DECREMENT, datetime.now().isoformat(), solution_id))

def solutions_list(trusted_only=False):
    """List solutions."""
    conn = get_db()

    if trusted_only:
        cursor = conn.execute("""
            SELECT s.*, fp.pattern_key
            FROM solutions s
            INNER JOIN failure_patterns fp ON s.pattern_id = fp.id
            WHERE s.confidence >= ?
            ORDER BY s.confidence DESC
        """, (TRUST_THRESHOLD,))
    else:
        cursor = conn.execute("""
            SELECT s.*, fp.pattern_key
            FROM solutions s
            INNER JOIN failure_patterns fp ON s.pattern_id = fp.id
            ORDER BY s.confidence DESC
        """)

    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(dim("No solutions recorded."))
        return

    print(bold("Solutions"))
    print("=" * 60)

    for row in rows:
        trusted = row['confidence'] >= TRUST_THRESHOLD
        trust_indicator = green("TRUSTED") if trusted else yellow("untrusted")
        print(f"\n  #{row['id']} [{trust_indicator}] {row['pattern_key']}")
        print(f"     Confidence: {row['confidence']:.2f} ({row['success_count']} success, {row['failure_count']} fail)")
        print(f"     {row['description']}")
        if row['code_example']:
            print(dim(f"     Example: {row['code_example'][:100]}..."))

# ============================================================
# LEARNINGS MANAGEMENT (AGENT.md equivalent)
# ============================================================

LEARNING_CATEGORIES = ['build', 'test', 'setup', 'gotcha', 'pattern', 'tool', 'other']

def add_learning(description, category=None):
    """Add a new learning."""
    conn = get_db()

    # Validate category
    if category and category not in LEARNING_CATEGORIES:
        print(yellow(f"Warning: Unknown category '{category}'. Using 'other'."))
        category = 'other'

    conn.execute("""
        INSERT INTO learnings (category, description)
        VALUES (?, ?)
    """, (category, description))
    learning_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]
    conn.commit()
    conn.close()

    cat_str = f" [{category}]" if category else ""
    print(green(f"Added learning #{learning_id}{cat_str}: {description[:60]}..."))
    return learning_id

def list_learnings(category=None):
    """List all learnings, optionally filtered by category."""
    conn = get_db()

    if category:
        cursor = conn.execute("""
            SELECT id, category, description, discovered_at, times_referenced
            FROM learnings WHERE category = ?
            ORDER BY discovered_at DESC
        """, (category,))
    else:
        cursor = conn.execute("""
            SELECT id, category, description, discovered_at, times_referenced
            FROM learnings ORDER BY discovered_at DESC
        """)

    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(dim("No learnings recorded."))
        return

    title = f"Learnings ({category})" if category else "Learnings"
    print(bold(title))
    print("=" * 60)

    for row in rows:
        cat_str = f"[{row['category']}]" if row['category'] else "[uncategorized]"
        ref_str = f"(referenced {row['times_referenced']}x)" if row['times_referenced'] > 0 else ""
        print(f"\n  #{row['id']} {blue(cat_str)} {ref_str}")
        print(f"     {row['description']}")
        print(dim(f"     Discovered: {row['discovered_at'][:10]}"))

def search_learnings(query):
    """Search learnings using FTS5."""
    conn = get_db()
    cursor = conn.execute("""
        SELECT l.id, l.category, l.description, l.times_referenced
        FROM learnings l
        INNER JOIN learnings_fts fts ON l.id = fts.rowid
        WHERE learnings_fts MATCH ?
        ORDER BY rank LIMIT 10
    """, (query,))
    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(dim(f"No learnings matching '{query}'."))
        return []

    print(bold(f"Learnings matching '{query}':"))
    print("=" * 60)

    for row in rows:
        cat_str = f"[{row['category']}]" if row['category'] else ""
        print(f"\n  #{row['id']} {blue(cat_str)}")
        print(f"     {row['description']}")

    return [dict(row) for row in rows]

def delete_learning(learning_id):
    """Delete a learning."""
    conn = get_db()
    conn.execute("DELETE FROM learnings WHERE id = ?", (learning_id,))
    if conn.total_changes == 0:
        print(red(f"Learning #{learning_id} not found."))
    else:
        print(green(f"Deleted learning #{learning_id}."))
    conn.commit()
    conn.close()

def increment_learning_reference(conn, learning_id):
    """Increment the reference count for a learning."""
    conn.execute("""
        UPDATE learnings SET times_referenced = times_referenced + 1
        WHERE id = ?
    """, (learning_id,))

# ============================================================
# GIT OPERATIONS
# ============================================================

def git_is_repo():
    """Check if current directory is a git repo."""
    result = subprocess.run(
        ["git", "rev-parse", "--git-dir"],
        capture_output=True, text=True
    )
    return result.returncode == 0

def git_has_changes():
    """Check if there are uncommitted changes."""
    result = subprocess.run(
        ["git", "status", "--porcelain"],
        capture_output=True, text=True
    )
    return bool(result.stdout.strip())

def git_commit(message):
    """Commit all changes."""
    subprocess.run(["git", "add", "-A"], capture_output=True)
    result = subprocess.run(
        ["git", "commit", "-m", message],
        capture_output=True, text=True
    )
    if result.returncode == 0:
        # Get commit hash
        hash_result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            capture_output=True, text=True
        )
        return hash_result.stdout.strip()
    return None

def git_revert_to(commit_hash):
    """Revert to a specific commit."""
    result = subprocess.run(
        ["git", "reset", "--hard", commit_hash],
        capture_output=True, text=True
    )
    return result.returncode == 0

def git_get_last_commit():
    """Get the last commit hash."""
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        capture_output=True, text=True
    )
    if result.returncode == 0:
        return result.stdout.strip()
    return None

# ============================================================
# COMMAND EXECUTION
# ============================================================

def run_command(cmd, timeout=None):
    """Run a command with timeout."""
    if not cmd:
        return True, "", 0

    start_time = time.time()

    try:
        result = subprocess.run(
            cmd,
            shell=True,
            capture_output=True,
            text=True,
            timeout=timeout
        )
        duration = time.time() - start_time

        if result.returncode == 0:
            return True, result.stdout, duration
        else:
            error_output = result.stderr or result.stdout
            return False, error_output, duration

    except subprocess.TimeoutExpired:
        duration = time.time() - start_time
        return False, f"Command timed out after {timeout} seconds", duration
    except Exception as e:
        duration = time.time() - start_time
        return False, str(e), duration

# ============================================================
# ITERATION MANAGEMENT
# ============================================================

def create_iteration(conn, task_id, attempt_number=1):
    """Create a new iteration record."""
    now = datetime.now().isoformat()
    conn.execute("""
        INSERT INTO iterations (task_id, attempt_number, started_at)
        VALUES (?, ?, ?)
    """, (task_id, attempt_number, now))
    iteration_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]

    # Update task status
    conn.execute("""
        UPDATE tasks SET status = 'in_progress', started_at = ?
        WHERE id = ?
    """, (datetime.now().isoformat(), task_id))

    conn.commit()
    return iteration_id

def complete_iteration(conn, iteration_id, status, commit_hash=None, notes=None):
    """Complete an iteration."""
    started_at = conn.execute(
        "SELECT started_at FROM iterations WHERE id = ?",
        (iteration_id,)
    ).fetchone()['started_at']

    started = datetime.fromisoformat(started_at)
    duration = (datetime.now() - started).total_seconds()

    conn.execute("""
        UPDATE iterations
        SET status = ?, ended_at = ?, duration_seconds = ?, commit_hash = ?, notes = ?
        WHERE id = ?
    """, (status, datetime.now().isoformat(), duration, commit_hash, notes, iteration_id))

    conn.commit()

def record_action(conn, iteration_id, action_type, description, file_path=None):
    """Record an action."""
    conn.execute("""
        INSERT INTO actions (iteration_id, action_type, description, file_path)
        VALUES (?, ?, ?, ?)
    """, (iteration_id, action_type, description, file_path))
    return conn.execute("SELECT last_insert_rowid()").fetchone()[0]

def record_outcome(conn, action_id, success, output_summary=None, error_message=None, duration=None):
    """Record an outcome."""
    conn.execute("""
        INSERT INTO outcomes (action_id, success, output_summary, error_message, duration_seconds)
        VALUES (?, ?, ?, ?, ?)
    """, (action_id, 1 if success else 0, output_summary, error_message, duration))
    conn.commit()

# ============================================================
# ITERATION EXECUTION
# ============================================================

def gather_context(conn, task):
    """Gather context for an iteration."""
    context = []

    # Get relevant spec sections
    if task.get('spec_section_id'):
        cursor = conn.execute(
            "SELECT * FROM spec_sections WHERE id = ?",
            (task['spec_section_id'],)
        )
        spec = cursor.fetchone()
        if spec:
            context.append(f"## Relevant Specification\n\n{spec['content']}")

    # Search for task-related specs
    cursor = conn.execute("""
        SELECT s.heading_path, s.content
        FROM spec_sections s
        INNER JOIN spec_sections_fts fts ON s.id = fts.rowid
        WHERE spec_sections_fts MATCH ?
        ORDER BY rank LIMIT 3
    """, (task['description'],))
    related_specs = cursor.fetchall()

    if related_specs:
        context.append("## Related Specifications\n")
        for spec in related_specs:
            context.append(f"### {spec['heading_path']}\n{spec['content'][:500]}")

    # Get trusted solutions for common patterns
    cursor = conn.execute("""
        SELECT s.description, fp.pattern_key
        FROM solutions s
        INNER JOIN failure_patterns fp ON s.pattern_id = fp.id
        WHERE s.confidence >= ?
        ORDER BY fp.occurrence_count DESC LIMIT 5
    """, (TRUST_THRESHOLD,))
    solutions = cursor.fetchall()

    if solutions:
        context.append("## Trusted Solutions (apply if relevant failure occurs)\n")
        for sol in solutions:
            context.append(f"- **{sol['pattern_key']}**: {sol['description']}")

    # Get recent failures to avoid
    cursor = conn.execute("""
        SELECT f.error_text, fp.pattern_key
        FROM failures f
        INNER JOIN failure_patterns fp ON f.pattern_id = fp.id
        WHERE f.resolved = 0
        ORDER BY f.created_at DESC LIMIT 5
    """)
    failures = cursor.fetchall()

    if failures:
        context.append("## Recent Unresolved Failures (avoid these)\n")
        for fail in failures:
            context.append(f"- **{fail['pattern_key']}**: {fail['error_text'][:200]}")

    # Get project learnings (AGENT.md equivalent)
    cursor = conn.execute("""
        SELECT id, category, description
        FROM learnings
        ORDER BY times_referenced DESC, discovered_at DESC
        LIMIT 10
    """)
    learnings = cursor.fetchall()

    if learnings:
        context.append("## Project Learnings (apply these patterns)\n")
        for learning in learnings:
            cat_str = f"[{learning['category']}]" if learning['category'] else ""
            context.append(f"- {cat_str} {learning['description']}")
            # Increment reference count
            increment_learning_reference(conn, learning['id'])
        conn.commit()

    return "\n\n".join(context)

def run_validation(conn, iteration_id):
    """Run build and test validation."""
    build_cmd = config_get('build_cmd')
    test_cmd = config_get('test_cmd')
    build_timeout = int(config_get('build_timeout') or DEFAULT_BUILD_TIMEOUT)
    test_timeout = int(config_get('test_timeout') or DEFAULT_TEST_TIMEOUT)

    # Run build
    if build_cmd:
        print(dim(f"Running build: {build_cmd}"))
        action_id = record_action(conn, iteration_id, 'build', f"Build: {build_cmd}")

        success, output, duration = run_command(build_cmd, timeout=build_timeout)
        record_outcome(conn, action_id, success,
                      output[:500] if success else None,
                      output[:1000] if not success else None,
                      duration)

        if not success:
            print(red("Build failed."))
            return False, output
        print(green("Build passed."))

    # Run tests
    if test_cmd:
        print(dim(f"Running tests: {test_cmd}"))
        action_id = record_action(conn, iteration_id, 'test', f"Test: {test_cmd}")

        success, output, duration = run_command(test_cmd, timeout=test_timeout)
        record_outcome(conn, action_id, success,
                      output[:500] if success else None,
                      output[:1000] if not success else None,
                      duration)

        if not success:
            print(red("Tests failed."))
            return False, output
        print(green("Tests passed."))

    return True, ""

def iterate_once():
    """Run a single iteration."""
    conn = get_db()

    # Get next task
    cursor = conn.execute("""
        SELECT * FROM tasks WHERE status = 'pending'
        ORDER BY priority, id LIMIT 1
    """)
    task = cursor.fetchone()

    if not task:
        print(dim("No pending tasks. Task queue empty."))
        conn.close()
        return False, "empty_queue"

    task = dict(task)
    print(bold(f"\n{'=' * 60}"))
    print(bold(f"Iteration: Task #{task['id']}"))
    print(f"Description: {task['description']}")
    print(bold(f"{'=' * 60}\n"))

    # Check for existing in-progress iteration
    cursor = conn.execute("""
        SELECT MAX(attempt_number) as max_attempt
        FROM iterations WHERE task_id = ? AND status = 'failed'
    """, (task['id'],))
    row = cursor.fetchone()
    attempt_number = (row['max_attempt'] or 0) + 1

    if attempt_number > MAX_FIX_ATTEMPTS:
        print(red(f"Task #{task['id']} has failed {MAX_FIX_ATTEMPTS} times. Skipping."))
        conn.execute(
            "UPDATE tasks SET status = 'blocked', blocked_by = ? WHERE id = ?",
            (f"Failed {MAX_FIX_ATTEMPTS} times", task['id'])
        )
        conn.commit()
        conn.close()
        return True, "max_attempts"

    # Create iteration
    iteration_id = create_iteration(conn, task['id'], attempt_number)
    print(f"Attempt {attempt_number} of {MAX_FIX_ATTEMPTS}")

    # Gather context
    context = gather_context(conn, task)
    if context:
        print(dim("\nContext gathered. Relevant specs and solutions loaded."))

    # Store the context for the agent
    context_file = get_dial_dir() / "current_context.md"
    with open(context_file, 'w') as f:
        f.write(f"# Task: {task['description']}\n\n")
        f.write(context)
    print(f"Context written to: {context_file}")

    # Get last good commit before changes
    last_commit = git_get_last_commit() if git_is_repo() else None

    # Wait for agent to do work
    print(yellow("\n>>> Agent should now implement the task <<<"))
    print(yellow(">>> Run 'dial validate' when ready to validate <<<"))
    print(yellow(">>> Or 'dial complete' to mark complete without validation <<<\n"))

    # For now, we'll pause here - the agent calls 'dial validate' separately
    # In a full autonomous mode, this would invoke the agent

    conn.close()
    return True, "awaiting_work"

def validate_current():
    """Validate the current iteration."""
    conn = get_db()

    # Find current in-progress iteration
    cursor = conn.execute("""
        SELECT i.*, t.description as task_description
        FROM iterations i
        INNER JOIN tasks t ON i.task_id = t.id
        WHERE i.status = 'in_progress'
        ORDER BY i.id DESC LIMIT 1
    """)
    iteration = cursor.fetchone()

    if not iteration:
        print(red("No iteration in progress."))
        conn.close()
        return False

    iteration = dict(iteration)
    print(f"Validating iteration #{iteration['id']} for task #{iteration['task_id']}")

    # Run validation
    success, error_output = run_validation(conn, iteration['id'])

    if success:
        # Commit changes
        commit_hash = None
        if git_is_repo() and git_has_changes():
            commit_message = f"DIAL: {iteration['task_description']}"
            commit_hash = git_commit(commit_message)
            if commit_hash:
                print(green(f"Committed: {commit_hash[:8]}"))

        # Complete iteration
        complete_iteration(conn, iteration['id'], 'completed', commit_hash)

        # Complete task
        conn.execute("""
            UPDATE tasks SET status = 'completed', completed_at = ?
            WHERE id = ?
        """, (datetime.now().isoformat(), iteration['task_id']))
        conn.commit()

        print(green(f"\nIteration #{iteration['id']} completed successfully!"))
        print(green(f"Task #{iteration['task_id']} marked as completed."))

        conn.close()
        return True
    else:
        # Record failure
        failure_id, pattern_id = record_failure(conn, iteration['id'], error_output)
        print(red(f"Recorded failure #{failure_id}"))

        # Check for trusted solutions
        solutions = find_trusted_solutions(conn, pattern_id)
        if solutions:
            print(yellow("\nTrusted solutions available:"))
            for sol in solutions:
                print(f"  - {sol['description']}")

        # Complete iteration as failed
        complete_iteration(conn, iteration['id'], 'failed', notes=error_output[:500])

        # Check if we should revert
        cursor = conn.execute("""
            SELECT COUNT(*) as fail_count
            FROM iterations WHERE task_id = ? AND status = 'failed'
        """, (iteration['task_id'],))
        fail_count = cursor.fetchone()['fail_count']

        if fail_count >= MAX_FIX_ATTEMPTS:
            print(red(f"\nMax attempts ({MAX_FIX_ATTEMPTS}) reached."))

            # Find last successful commit
            cursor = conn.execute("""
                SELECT commit_hash FROM iterations
                WHERE status = 'completed' AND commit_hash IS NOT NULL
                ORDER BY id DESC LIMIT 1
            """)
            row = cursor.fetchone()

            if row and git_is_repo():
                print(yellow(f"Reverting to last good commit: {row['commit_hash'][:8]}"))
                git_revert_to(row['commit_hash'])

            # Block the task
            conn.execute("""
                UPDATE tasks SET status = 'blocked', blocked_by = ?
                WHERE id = ?
            """, (f"Failed {MAX_FIX_ATTEMPTS} attempts", iteration['task_id']))
        else:
            # Reset task to pending for retry
            conn.execute("""
                UPDATE tasks SET status = 'pending'
                WHERE id = ?
            """, (iteration['task_id'],))
            print(yellow(f"\nTask reset to pending. {MAX_FIX_ATTEMPTS - fail_count} attempts remaining."))

        conn.commit()
        conn.close()
        return False

def run_loop(max_iterations=None):
    """Run iterations continuously."""
    iteration_count = 0
    stop_file = get_dial_dir() / "stop"

    # Remove any existing stop file
    if stop_file.exists():
        stop_file.unlink()

    print(bold("Starting DIAL run loop..."))
    print(dim("Create .dial/stop file to stop gracefully.\n"))

    while True:
        # Check stop flag
        if stop_file.exists():
            print(yellow("\nStop flag detected. Stopping gracefully."))
            stop_file.unlink()
            break

        # Check iteration limit
        if max_iterations and iteration_count >= max_iterations:
            print(yellow(f"\nReached max iterations ({max_iterations}). Stopping."))
            break

        # Run one iteration
        success, result = iterate_once()

        if result == "empty_queue":
            print(bold("\n" + "=" * 60))
            print(bold("Task queue empty. DIAL run complete."))
            show_run_summary()
            break

        if result == "awaiting_work":
            # In manual mode, we stop here
            print(dim("\nWaiting for work. Run 'dial validate' after implementing."))
            break

        iteration_count += 1

def show_run_summary():
    """Show summary after a run."""
    conn = get_db()

    cursor = conn.execute("""
        SELECT
            COUNT(*) as total,
            SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed,
            SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed
        FROM iterations
    """)
    row = cursor.fetchone()

    print(f"\nCompleted: {row['completed'] or 0}")
    print(f"Failed: {row['failed'] or 0}")

    cursor = conn.execute("""
        SELECT COUNT(*) as count FROM solutions WHERE confidence >= ?
    """, (TRUST_THRESHOLD,))
    solutions = cursor.fetchone()['count']
    print(f"Solutions learned: {solutions}")

    conn.close()

# ============================================================
# STATUS AND HISTORY
# ============================================================

def show_status():
    """Show current DIAL status."""
    conn = get_db()

    phase = get_current_phase()
    project = config_get('project_name')

    print(bold(f"DIAL Status: {project} (phase: {phase})"))
    print("=" * 60)

    # Current iteration
    cursor = conn.execute("""
        SELECT i.*, t.description
        FROM iterations i
        INNER JOIN tasks t ON i.task_id = t.id
        WHERE i.status = 'in_progress'
        ORDER BY i.id DESC LIMIT 1
    """)
    current = cursor.fetchone()

    if current:
        print(yellow(f"\nIn Progress: Task #{current['task_id']}"))
        print(f"  {current['description']}")
        print(f"  Attempt {current['attempt_number']} of {MAX_FIX_ATTEMPTS}")
    else:
        print(dim("\nNo iteration in progress."))

    # Task counts
    cursor = conn.execute("""
        SELECT status, COUNT(*) as count
        FROM tasks GROUP BY status
    """)
    task_counts = {row['status']: row['count'] for row in cursor.fetchall()}

    print(f"\nTasks:")
    print(f"  Pending:   {task_counts.get('pending', 0)}")
    print(f"  Completed: {task_counts.get('completed', 0)}")
    print(f"  Blocked:   {task_counts.get('blocked', 0)}")

    # Recent iterations
    cursor = conn.execute("""
        SELECT i.id, i.status, i.duration_seconds, t.description
        FROM iterations i
        INNER JOIN tasks t ON i.task_id = t.id
        ORDER BY i.id DESC LIMIT 5
    """)
    recent = cursor.fetchall()

    if recent:
        print(f"\nRecent Iterations:")
        for row in recent:
            status_color = green if row['status'] == 'completed' else red
            duration = f"{row['duration_seconds']:.1f}s" if row['duration_seconds'] else "..."
            print(f"  #{row['id']} {status_color(row['status']):12} {duration:8} {row['description'][:40]}")

    conn.close()

def show_history(limit=20):
    """Show iteration history."""
    conn = get_db()

    cursor = conn.execute("""
        SELECT i.*, t.description
        FROM iterations i
        INNER JOIN tasks t ON i.task_id = t.id
        ORDER BY i.id DESC LIMIT ?
    """, (limit,))
    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(dim("No iteration history."))
        return

    print(bold("Iteration History"))
    print("=" * 80)

    for row in rows:
        status_colors = {
            'completed': green,
            'failed': red,
            'reverted': yellow,
            'in_progress': blue,
        }
        status_fn = status_colors.get(row['status'], str)
        duration = f"{row['duration_seconds']:.1f}s" if row['duration_seconds'] else "..."
        commit = row['commit_hash'][:8] if row['commit_hash'] else "--------"

        print(f"#{row['id']:4} {status_fn(row['status']):12} {duration:8} {commit} {row['description'][:40]}")

def show_failures(unresolved_only=True):
    """Show failures."""
    conn = get_db()

    if unresolved_only:
        cursor = conn.execute("""
            SELECT f.*, fp.pattern_key
            FROM failures f
            LEFT JOIN failure_patterns fp ON f.pattern_id = fp.id
            WHERE f.resolved = 0
            ORDER BY f.created_at DESC
        """)
    else:
        cursor = conn.execute("""
            SELECT f.*, fp.pattern_key
            FROM failures f
            LEFT JOIN failure_patterns fp ON f.pattern_id = fp.id
            ORDER BY f.created_at DESC LIMIT 50
        """)

    rows = cursor.fetchall()
    conn.close()

    if not rows:
        print(dim("No failures found."))
        return

    print(bold("Failures" + (" (unresolved)" if unresolved_only else "")))
    print("=" * 60)

    for row in rows:
        resolved = green("resolved") if row['resolved'] else red("unresolved")
        print(f"\n#{row['id']} [{row['pattern_key']}] {resolved}")
        print(f"  Iteration: #{row['iteration_id']}")
        if row['file_path']:
            print(f"  File: {row['file_path']}:{row['line_number'] or '?'}")
        print(f"  {row['error_text'][:200]}")

def show_stats():
    """Show statistics dashboard."""
    conn = get_db()

    phase = get_current_phase()
    project = config_get('project_name')

    print(bold(f"\nDIAL Statistics: {project} (phase: {phase})"))
    print("=" * 60)

    # Iterations
    cursor = conn.execute("""
        SELECT
            COUNT(*) as total,
            SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed,
            SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed,
            SUM(duration_seconds) as total_duration,
            AVG(duration_seconds) as avg_duration,
            MAX(duration_seconds) as max_duration
        FROM iterations
    """)
    iter_stats = cursor.fetchone()

    total = iter_stats['total'] or 0
    completed = iter_stats['completed'] or 0
    failed = iter_stats['failed'] or 0
    success_rate = (completed / total * 100) if total > 0 else 0

    print(f"\n{bold('Iterations')}")
    print(f"  Total:      {total}")
    print(f"  Successful: {green(str(completed))} ({success_rate:.1f}%)")
    print(f"  Failed:     {red(str(failed))} ({100-success_rate:.1f}%)" if failed else f"  Failed:     {failed}")

    # Tasks
    cursor = conn.execute("""
        SELECT status, COUNT(*) as count
        FROM tasks GROUP BY status
    """)
    task_counts = {row['status']: row['count'] for row in cursor.fetchall()}

    print(f"\n{bold('Tasks')}")
    print(f"  Completed:  {task_counts.get('completed', 0)}")
    print(f"  Pending:    {task_counts.get('pending', 0)}")
    print(f"  Blocked:    {task_counts.get('blocked', 0)}")
    print(f"  Cancelled:  {task_counts.get('cancelled', 0)}")

    # Time
    if iter_stats['total_duration']:
        total_mins = iter_stats['total_duration'] / 60
        avg_mins = (iter_stats['avg_duration'] or 0) / 60
        max_mins = (iter_stats['max_duration'] or 0) / 60

        print(f"\n{bold('Time')}")
        if total_mins >= 60:
            print(f"  Total runtime:    {total_mins/60:.1f}h")
        else:
            print(f"  Total runtime:    {total_mins:.1f}m")
        print(f"  Avg iteration:    {avg_mins:.1f}m")
        print(f"  Longest:          {max_mins:.1f}m")

    # Failure patterns
    cursor = conn.execute("""
        SELECT pattern_key, occurrence_count
        FROM failure_patterns
        ORDER BY occurrence_count DESC LIMIT 5
    """)
    patterns = cursor.fetchall()

    if patterns:
        print(f"\n{bold('Failure Patterns (top 5)')}")
        for p in patterns:
            print(f"  {p['pattern_key']:25} {p['occurrence_count']} occurrences")

    # Solutions
    cursor = conn.execute("""
        SELECT
            COUNT(*) as total,
            SUM(CASE WHEN confidence >= ? THEN 1 ELSE 0 END) as trusted,
            SUM(success_count) as total_success,
            SUM(failure_count) as total_failure
        FROM solutions
    """, (TRUST_THRESHOLD,))
    sol_stats = cursor.fetchone()

    if sol_stats['total']:
        total_apps = (sol_stats['total_success'] or 0) + (sol_stats['total_failure'] or 0)
        hit_rate = (sol_stats['total_success'] / total_apps * 100) if total_apps > 0 else 0

        print(f"\n{bold('Solutions')}")
        print(f"  Total:            {sol_stats['total']}")
        print(f"  Trusted (≥0.6):   {green(str(sol_stats['trusted']))}")
        if total_apps > 0:
            print(f"  Hit rate:         {hit_rate:.0f}% ({total_apps} applications)")

    # Learnings
    cursor = conn.execute("""
        SELECT
            COUNT(*) as total,
            SUM(times_referenced) as total_refs
        FROM learnings
    """)
    learn_stats = cursor.fetchone()

    if learn_stats['total']:
        print(f"\n{bold('Learnings')}")
        print(f"  Total:            {learn_stats['total']}")
        print(f"  Total references: {learn_stats['total_refs'] or 0}")

        # Breakdown by category
        cursor = conn.execute("""
            SELECT category, COUNT(*) as count
            FROM learnings
            GROUP BY category
            ORDER BY count DESC
        """)
        categories = cursor.fetchall()
        if categories:
            print(f"  By category:")
            for cat in categories:
                cat_name = cat['category'] or 'uncategorized'
                print(f"    {cat_name}: {cat['count']}")

    print("\n" + "=" * 60)
    conn.close()

# ============================================================
# RECOVERY
# ============================================================

def revert_to_last_good():
    """Revert to the last successful commit."""
    if not git_is_repo():
        print(red("Not a git repository."))
        return False

    conn = get_db()
    cursor = conn.execute("""
        SELECT commit_hash FROM iterations
        WHERE status = 'completed' AND commit_hash IS NOT NULL
        ORDER BY id DESC LIMIT 1
    """)
    row = cursor.fetchone()
    conn.close()

    if not row:
        print(red("No successful commits found."))
        return False

    commit_hash = row['commit_hash']
    print(yellow(f"Reverting to: {commit_hash}"))

    if git_revert_to(commit_hash):
        print(green("Reverted successfully."))
        return True
    else:
        print(red("Revert failed."))
        return False

def reset_current():
    """Reset the current iteration."""
    conn = get_db()

    cursor = conn.execute("""
        SELECT * FROM iterations
        WHERE status = 'in_progress'
        ORDER BY id DESC LIMIT 1
    """)
    iteration = cursor.fetchone()

    if not iteration:
        print(dim("No iteration in progress."))
        conn.close()
        return

    # Mark iteration as reverted
    conn.execute("""
        UPDATE iterations SET status = 'reverted', ended_at = ?
        WHERE id = ?
    """, (datetime.now().isoformat(), iteration['id']))

    # Reset task to pending
    conn.execute("""
        UPDATE tasks SET status = 'pending'
        WHERE id = ?
    """, (iteration['task_id'],))

    conn.commit()
    conn.close()

    print(green(f"Reset iteration #{iteration['id']}. Task returned to pending."))

# ============================================================
# MAIN CLI
# ============================================================

def main():
    parser = argparse.ArgumentParser(
        description="DIAL - Deterministic Iterative Agent Loop",
        formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument('--version', action='version', version=f'DIAL {VERSION}')

    subparsers = parser.add_subparsers(dest='command', help='Commands')

    # init
    init_parser = subparsers.add_parser('init', help='Initialize DIAL in current directory')
    init_parser.add_argument('--phase', default=DEFAULT_PHASE, help='Phase name')
    init_parser.add_argument('--import-solutions', dest='import_solutions',
                            help='Import trusted solutions from another phase')

    # index
    index_parser = subparsers.add_parser('index', help='Index spec files')
    index_parser.add_argument('--dir', default='specs', help='Specs directory')

    # config
    config_parser = subparsers.add_parser('config', help='Manage configuration')
    config_sub = config_parser.add_subparsers(dest='config_cmd')
    config_set_parser = config_sub.add_parser('set', help='Set a config value')
    config_set_parser.add_argument('key', help='Config key')
    config_set_parser.add_argument('value', help='Config value')
    config_sub.add_parser('show', help='Show all config')

    # task
    task_parser = subparsers.add_parser('task', help='Manage tasks')
    task_sub = task_parser.add_subparsers(dest='task_cmd')

    task_add_parser = task_sub.add_parser('add', help='Add a task')
    task_add_parser.add_argument('description', help='Task description')
    task_add_parser.add_argument('--priority', '-p', type=int, default=5, help='Priority (1-10)')
    task_add_parser.add_argument('--spec', type=int, help='Spec section ID')

    task_list_parser = task_sub.add_parser('list', help='List tasks')
    task_list_parser.add_argument('--all', '-a', action='store_true', help='Show all tasks')

    task_sub.add_parser('next', help='Show next task')

    task_done_parser = task_sub.add_parser('done', help='Mark task done')
    task_done_parser.add_argument('id', type=int, help='Task ID')

    task_block_parser = task_sub.add_parser('block', help='Block a task')
    task_block_parser.add_argument('id', type=int, help='Task ID')
    task_block_parser.add_argument('reason', help='Block reason')

    task_cancel_parser = task_sub.add_parser('cancel', help='Cancel a task')
    task_cancel_parser.add_argument('id', type=int, help='Task ID')

    task_search_parser = task_sub.add_parser('search', help='Search tasks')
    task_search_parser.add_argument('query', help='Search query')

    # spec
    spec_parser = subparsers.add_parser('spec', help='Query specs')
    spec_sub = spec_parser.add_subparsers(dest='spec_cmd')

    spec_search_parser = spec_sub.add_parser('search', help='Search specs')
    spec_search_parser.add_argument('query', help='Search query')

    spec_show_parser = spec_sub.add_parser('show', help='Show spec section')
    spec_show_parser.add_argument('id', type=int, help='Section ID')

    spec_sub.add_parser('list', help='List spec sections')

    # iterate
    subparsers.add_parser('iterate', help='Run one iteration')

    # validate
    subparsers.add_parser('validate', help='Validate current iteration')

    # run
    run_parser = subparsers.add_parser('run', help='Run iterations continuously')
    run_parser.add_argument('--max', type=int, help='Max iterations')

    # stop
    subparsers.add_parser('stop', help='Stop after current iteration')

    # status
    subparsers.add_parser('status', help='Show current status')

    # history
    history_parser = subparsers.add_parser('history', help='Show iteration history')
    history_parser.add_argument('--limit', '-n', type=int, default=20, help='Number of entries')

    # failures
    failures_parser = subparsers.add_parser('failures', help='Show failures')
    failures_parser.add_argument('--all', '-a', action='store_true', help='Show all failures')

    # solutions
    solutions_parser = subparsers.add_parser('solutions', help='Show solutions')
    solutions_parser.add_argument('--trusted', '-t', action='store_true', help='Show only trusted')

    # learn
    learn_parser = subparsers.add_parser('learn', help='Add a learning')
    learn_parser.add_argument('description', help='Learning description')
    learn_parser.add_argument('--category', '-c', choices=LEARNING_CATEGORIES,
                             help=f'Category: {", ".join(LEARNING_CATEGORIES)}')

    # learnings
    learnings_parser = subparsers.add_parser('learnings', help='Show learnings')
    learnings_sub = learnings_parser.add_subparsers(dest='learnings_cmd')

    learnings_list_parser = learnings_sub.add_parser('list', help='List learnings')
    learnings_list_parser.add_argument('--category', '-c', choices=LEARNING_CATEGORIES,
                                       help='Filter by category')

    learnings_search_parser = learnings_sub.add_parser('search', help='Search learnings')
    learnings_search_parser.add_argument('query', help='Search query')

    learnings_delete_parser = learnings_sub.add_parser('delete', help='Delete a learning')
    learnings_delete_parser.add_argument('id', type=int, help='Learning ID')

    # stats
    subparsers.add_parser('stats', help='Show statistics')

    # revert
    subparsers.add_parser('revert', help='Revert to last good commit')

    # reset
    subparsers.add_parser('reset', help='Reset current iteration')

    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        return

    # Commands that don't need initialization
    if args.command == 'init':
        init_db(args.phase, args.import_solutions)
        return

    # Check if initialized
    if args.command != 'init' and not get_dial_dir().exists():
        print(red("DIAL not initialized. Run 'dial init' first."))
        sys.exit(1)

    # Route commands
    if args.command == 'index':
        index_specs(args.dir)

    elif args.command == 'config':
        if args.config_cmd == 'set':
            config_set(args.key, args.value)
            print(green(f"Set {args.key} = {args.value}"))
        elif args.config_cmd == 'show':
            config_show()
        else:
            config_show()

    elif args.command == 'task':
        if args.task_cmd == 'add':
            task_add(args.description, args.priority, args.spec)
        elif args.task_cmd == 'list':
            task_list(args.all)
        elif args.task_cmd == 'next':
            task_next()
        elif args.task_cmd == 'done':
            task_done(args.id)
        elif args.task_cmd == 'block':
            task_block(args.id, args.reason)
        elif args.task_cmd == 'cancel':
            task_cancel(args.id)
        elif args.task_cmd == 'search':
            task_search(args.query)
        else:
            task_list()

    elif args.command == 'spec':
        if args.spec_cmd == 'search':
            spec_search(args.query)
        elif args.spec_cmd == 'show':
            spec_show(args.id)
        elif args.spec_cmd == 'list':
            spec_list()
        else:
            spec_list()

    elif args.command == 'iterate':
        iterate_once()

    elif args.command == 'validate':
        validate_current()

    elif args.command == 'run':
        run_loop(args.max)

    elif args.command == 'stop':
        stop_file = get_dial_dir() / "stop"
        stop_file.touch()
        print(yellow("Stop flag created. DIAL will stop after current iteration."))

    elif args.command == 'status':
        show_status()

    elif args.command == 'history':
        show_history(args.limit)

    elif args.command == 'failures':
        show_failures(not args.all)

    elif args.command == 'solutions':
        solutions_list(args.trusted)

    elif args.command == 'learn':
        add_learning(args.description, args.category)

    elif args.command == 'learnings':
        if args.learnings_cmd == 'list':
            list_learnings(args.category)
        elif args.learnings_cmd == 'search':
            search_learnings(args.query)
        elif args.learnings_cmd == 'delete':
            delete_learning(args.id)
        else:
            list_learnings()

    elif args.command == 'stats':
        show_stats()

    elif args.command == 'revert':
        revert_to_last_good()

    elif args.command == 'reset':
        reset_current()

if __name__ == "__main__":
    main()
