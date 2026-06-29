# Database Migrations — Albert

**Engine**: `sqflite` on-device, `sqflite_common_ffi` for tests.
**Versioning**: `PRAGMA user_version` paired with a `schema_migrations` audit row.

## Layout

```
lib/db/
  app_database.dart          # opens DB, runs migrations, exposes Database
  migrations/
    migration.dart           # Migration abstract class (version, up)
    001_init.dart            # v1 schema (see database_schema.md)
    002_<short_name>.dart    # subsequent migrations, append-only
    all.dart                 # ordered list exported as `allMigrations`
```

## Rules

1. **Forward-only**. No DOWN migrations. Mobile DBs get replaced, not rolled back.
2. **Append-only**. Never edit a merged migration file. Fix bugs by writing `NNN_fix_*.dart`.
3. **One transaction per migration**. `AppDatabase` wraps each `up()` call in `db.transaction`.
4. **Idempotent-friendly**. Use `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS` so a half-applied migration can re-run cleanly.
5. **Version bump**. Each migration sets `PRAGMA user_version = <version>` and inserts `(version, now)` into `schema_migrations` as its final step (done by `AppDatabase`, not the migration itself).
6. **Update docs**. Any schema change updates `.agent/System/database_schema.md` in the same commit.

## Writing a Migration

```dart
// lib/db/migrations/002_add_foo.dart
import 'package:sqflite/sqflite.dart';
import 'migration.dart';

final Migration migration002AddFoo = Migration(
  version: 2,
  name: 'add_foo',
  up: (Database db) async {
    await db.execute('''
      CREATE TABLE IF NOT EXISTS foo (
        id INTEGER PRIMARY KEY,
        created_at INTEGER NOT NULL
      )
    ''');
  },
);
```

Register it in `migrations/all.dart`:

```dart
final List<Migration> allMigrations = [
  migration001Init,
  migration002AddFoo,
];
```

## Runtime Flow (AppDatabase)

1. Open DB at `<app-docs>/albert.db`.
2. Read `PRAGMA user_version` → `current`.
3. For each migration where `m.version > current`: run inside transaction, bump `user_version`, insert into `schema_migrations`.
4. Return ready `Database`.

## Testing

Every migration has at least one test in `test/db/`:

- Opens a fresh in-memory DB via `sqflite_common_ffi`.
- Applies all migrations up to target version.
- Asserts expected tables / columns / indexes exist (`sqlite_master` query).
- Running twice is a no-op (`user_version` unchanged on second open).

Run: `flutter test test/db/`.

## Don't

- Don't drop or rename columns in place — add a new column, backfill, stop writing to the old one, remove in a later migration.
- Don't put secrets in the DB. Secrets → `flutter_secure_storage`.
- Don't write an app-level encryption layer yet. OS sandbox is sufficient for v1.
