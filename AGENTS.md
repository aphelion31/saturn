# AGENTS.md

Guidance for AI agents working in this repository.

## What this is

Saturn is a lightweight rewrite of [Plane](https://github.com/makeplane/plane): the Django + Celery +
Redis + RabbitMQ + Node backend is replaced by one Rust binary (axum + sqlx), with PostgreSQL as the only
dependency. Plane's React Router apps (web, god-mode admin, public space) are kept and built as static
SPAs that the binary serves. Saturn keeps Plane's database schema, session cookies, password hashes and
API shapes, so an existing Plane database and the unchanged frontend keep working.

| Path          | Role                                                                                 |
| ------------- | ------------------------------------------------------------------------------------ |
| `src/`        | Rust backend: axum HTTP, sqlx Postgres. One module per API area                      |
| `web/`        | Plane's pnpm workspace: `apps/web`, `apps/admin`, `apps/space` and `packages/*`      |
| `migrations/` | Plane's schema (`pg_dump` of Django's migrations) plus database defaults              |
| `tools/`      | Schema regeneration from a Plane checkout, API smoke test                            |
| `scripts/`    | Git hook checks, branch helper, frontend build                                       |
| `docs/`       | Architecture and process docs                                                        |

Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) before making structural changes, and
[`WORKERS.md`](WORKERS.md) before porting an endpoint.

## Non-negotiables

**Every workspace route calls `require_workspace` (or `require_project`) first and filters every query by
`workspace_id` / `project_id`.** Tenant isolation lives in the handler. A query without the filter leaks
one workspace's data to another.

**Roles follow Plane exactly.** 20 admin, 15 member, 5 guest, at workspace and project level.
`ProjectCtx::require` already applies Plane's rule that a workspace admin who is a project member passes.
Restricted guests (`restricted_guest()`) only see work items they created.

**Bind every value.** Use `sqlx::query(...).bind(...)`. The only text formatted into SQL is a constant
(JSON shape constants like `STATE_JSON`) or a column name from a fixed whitelist (`patch_sets`). Never
format request input into SQL.

**Response shapes match Plane's DRF serializers.** The React apps are Plane's, unchanged. Read the
serializer in `plane/apps/api/plane/app/serializers/` and the TypeScript type in `web/packages/types/`
before writing a handler. Paths keep Plane's trailing slash.

**Soft delete.** Tables with `deleted_at` are soft-deleted: set `deleted_at = now()` and add
`deleted_at IS NULL` to every read.

**The frontend talks to the backend over HTTP only.** No Rust types in `web/`, no Node at runtime.

## Commands

```bash
cargo build && cargo test
DATABASE_URL=postgres://localhost/saturn cargo run -- seed admin@example.com 'Change-me-2026' Acme
DATABASE_URL=postgres://localhost/saturn cargo run          # http://localhost:3000
scripts/build-web.sh                                        # build web, admin, space SPAs
BASE=http://127.0.0.1:3000 tools/smoke.sh                   # API smoke test over real HTTP
BASE=http://127.0.0.1:3000 node tools/e2e.mjs               # real web app in Chromium, prints failed API calls
(cd web && pnpm turbo run check:types --filter=web)         # frontend types
```

Verification before claiming work is done: `cargo build`, `cargo test`, then exercise the change over
HTTP against a running binary (sign in with the cookie jar, call the endpoint, check the JSON against
Plane's serializer). A passing build is not evidence a feature works.

## Conventions

Branches are `STN-<number>/<type>-<description>` cut from `origin/develop` (`scripts/new-branch.sh`).
Commits are Conventional Commits in English. Both are enforced by the hooks in `.githooks/`, along with a
block on direct pushes to `develop` and `master`. See [`docs/github/`](docs/github/).

Rust: `cargo fmt` style, `max_width = 120`. TypeScript: Plane's oxfmt and oxlint config in `web/`.

## Traps worth knowing

- **Trailing slashes.** Plane's axios client adds `/` to every API path. Register routes with it.
- **Form posts, not JSON, for auth.** `/auth/sign-in/` and friends receive HTML forms and answer with a
  302; errors travel as `?error_code=&error_message=` query params.
- **Django defaults live in Python.** `migrations/0002_defaults.sql` copies them into the database so plain
  INSERTs work. A new NOT NULL column without a default fails at runtime, not compile time.
- **`smallint` roles.** `workspace_members.role` and `project_members.role` are `smallint`: decode as `i16`.
- **`SECRET_KEY` must match Plane's** when pointing Saturn at an existing Plane database, or every user is
  signed out (sessions are signed with it).
