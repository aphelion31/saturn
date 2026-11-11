# Saturn

> **Note:** Saturn is a Rust rewrite of [Plane](https://github.com/makeplane/plane), the open source project
> management tool. Plane's Django + Celery + Redis + RabbitMQ + Node backend is replaced by one small Rust binary
> (axum + sqlx). Plane's React Router apps are kept and served by the same binary as static files. Saturn keeps
> Plane's PostgreSQL schema, sessions, password hashes and API shapes, so an existing Plane database and the
> unchanged Plane frontend keep working.
> Plane is © Plane Software, Inc., licensed AGPL-3.0. Saturn is a derivative work under the same license
> (see `LICENSE`). Saturn is not affiliated with Plane Software, Inc.

## Why

Self-hosted Plane runs about nine containers (web, space, admin, live, api, worker, beat-worker, migrator,
proxy) plus Postgres, Redis, RabbitMQ and MinIO, and needs several GB of RAM. Saturn runs as one process with
PostgreSQL as the only dependency.

## Benchmark

<!-- BENCHMARK:START -->
Measured on the same machine against the same PostgreSQL. See [`tools/bench.sh`](tools/bench.sh).
Results are filled in by the benchmark run.
<!-- BENCHMARK:END -->

## Layout

```
src/          Rust backend (axum + sqlx), one module per Plane API area
web/          Plane's pnpm workspace: apps/web, apps/admin (god-mode), apps/space (public boards), packages/*
migrations/   PostgreSQL schema dumped from Plane's Django migrations, plus database defaults
tools/        schema regeneration, API smoke test, benchmark
scripts/      git hook checks, branch helper, frontend build
docs/         architecture and process docs
```

## Run

```bash
# 1. Database (plain PostgreSQL, no extensions needed). Migrations run on startup.
createdb saturn

# 2. Frontend: build the three apps once into web/apps/*/build/client
scripts/build-web.sh

# 3. First admin user, workspace and project (or open /god-mode/ and set up the instance there)
DATABASE_URL=postgres://localhost/saturn cargo run --release -- seed admin@example.com 'Change-me-2026' Acme

# 4. Serve
DATABASE_URL=postgres://localhost/saturn cargo run --release    # http://localhost:3000
```

| Path         | App                                  |
| ------------ | ------------------------------------ |
| `/`          | Plane web app                        |
| `/god-mode/` | Instance admin                       |
| `/spaces/`   | Published boards                     |
| `/api/`      | API (session cookie or `X-Api-Key`)  |

| Env            | Default                       | Purpose                                                    |
| -------------- | ----------------------------- | ---------------------------------------------------------- |
| `DATABASE_URL` | `postgres://localhost/saturn` | Postgres connection                                        |
| `BIND`         | `0.0.0.0:3000`                | Listen address                                             |
| `WEB_URL`      | `http://localhost:3000`       | Public origin, used for redirects and the CSRF origin check |
| `SECRET_KEY`   | dev value                     | Signs sessions. Set it in production                       |
| `WEB_DIR`      | `web/apps`                    | Built frontend apps                                        |
| `STORAGE_DIR`  | `./storage`                   | Uploaded files                                             |

An existing Plane database works too: point `DATABASE_URL` at it and set `SECRET_KEY` to Plane's value.
Passwords and sessions are compatible, so users stay signed in. Test on a copy first.

## Scope

Ported: instance setup and god-mode sign in, email and password sign in / sign up / sign out for the app and
public boards, current user, profile and onboarding, project states. The remaining API areas (workspaces,
projects, work items, cycles, modules, views, intake, pages, notifications, uploads, public boards) are being
ported module by module; see [`docs/architectures/03.PLANE-COMPAT.md`](docs/architectures/03.PLANE-COMPAT.md).

Not ported: magic-link and OAuth sign in, email delivery, webhooks delivery, importers and exporters,
GitHub and Slack integrations, AI features. Unported `/api` routes answer `501` and log `unported endpoint`.

## Contributing

Read [`AGENTS.md`](AGENTS.md) and [`docs/`](docs/ARCHITECTURE.md) first. Short version:

- `develop` is the default branch; `master` holds released code.
- Work branches are `STN-<number>/<type>-<description>`, cut from `develop`, for example
  `STN-001/chore-repo-tooling` or `STN-014/feat-cycles-api`. Create one with
  `scripts/new-branch.sh feat-<description>`.
- Commits and PR titles are Conventional Commits in English (`feat(cycles): add cycle progress endpoint`).
  No AI co-author lines.
- Install the hooks once: `git config core.hooksPath .githooks`.

How to commit on a feature branch:

```bash
git switch develop && git pull origin develop
scripts/new-branch.sh feat-cycles-api            # -> STN-<next>/feat-cycles-api
# ... edit ...
git add src/cycles.rs tools/smoke.d/40-cycles.sh            # stage files explicitly, never `git add -A`
git commit -m "feat(cycles): add cycle list and create endpoints"
git push -u origin STN-014/feat-cycles-api       # then open a PR into develop, squash merge
```

Details: [commit](docs/github/01.GIT-COMMIT.md), [branch](docs/github/02.GIT-BRANCH.md),
[pull request and release](docs/github/03.GIT-PR-FLOW.md).

## Tests

Smoke tests need `bash`, `curl`, `jq` and `psql`. Python is only needed to regenerate the schema from Plane (`tools/plane-schema.sh`, `tools/django_defaults.py`), never to run Saturn.

```bash
cargo test                                  # unit tests (Django session and password compatibility, helpers)
BASE=http://127.0.0.1:3000 tools/smoke.sh   # API smoke test against a running, seeded server
BASE=http://127.0.0.1:3000 node tools/e2e.mjs  # Plane web app in headless Chromium, lists failing API calls
scripts/build-web.sh                        # frontend builds
```

## License

AGPL-3.0 (see `LICENSE`). Saturn is derived from Plane (AGPL-3.0, © Plane Software, Inc.): it ships Plane's
frontend apps and keeps its database schema and API.
