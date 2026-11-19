-- Notes with normalized titles and slugs, as migrations 1 to 3 leave them.
CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, slug TEXT);
INSERT INTO notes VALUES (1, 'HELLO; WORLD', 'hello-world');
