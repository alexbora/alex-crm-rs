# CONTEXT.md

## Glossary

**Instant in-memory company search**: On app startup, all records from the `companies` table in `notes_app.db` are loaded into memory. The UI displays all companies immediately after loading. All search/filter operations are performed in-memory for speed, using case-insensitive substring matching. The in-memory list is the UI's source of truth and is refreshed from the database only on explicit reloads or edits. For very large tables, the user is warned and can opt for paging instead.

- On startup or refresh, the UI displays a static "Loading..." message until all companies are loaded.
- After loading, a status bar shows the number of companies loaded (e.g., "Loaded 1,234 companies").
