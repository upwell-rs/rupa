-- Create the usage events table.

CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    occurred_at TEXT NOT NULL
);
