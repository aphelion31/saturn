<div align="center">

<img src="assets/brand/saturn-mark.svg" alt="Saturn" width="128">

# Saturn

<p>
  <strong>Plane, rewritten in Rust.</strong><br>
  Full Plane 1.5.1 backend parity in one ~19 MB Axum binary. Zero Python, zero Redis, zero Celery.
</p>

[![Last Commit](https://img.shields.io/github/last-commit/aphelion31/saturn/develop?style=flat-square&logo=github)](https://github.com/aphelion31/saturn/commits/develop)
[![GitHub Stars](https://img.shields.io/github/stars/aphelion31/saturn?style=flat-square&logo=github)](https://github.com/aphelion31/saturn/stargazers)
[![GitHub Release](https://img.shields.io/github/v/release/aphelion31/saturn?style=flat-square&logo=github)](https://github.com/aphelion31/saturn/releases/latest)
[![CI](https://img.shields.io/github/actions/workflow/status/aphelion31/saturn/ci.yml?branch=develop&style=flat-square&logo=githubactions&logoColor=white)](https://github.com/aphelion31/saturn/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-C96E32?style=flat-square)](LICENSE)

</div>

<div align="center">

  <a href="https://github.com/aphelion31/saturn/releases/download/readme-assets/saturn-demo-v2.mp4">
    <img src="https://github.com/aphelion31/saturn/releases/download/readme-assets/saturn-demo-v2.webp" alt="Saturn showcase demonstration" width="900">
  </a>

  <p><em>Saturn 12-second overview (click for 1080p MP4)</em></p>

</div>

> **Note:** Saturn is a Rust rewrite of [Plane](https://github.com/makeplane/plane), the open source project
> management tool. Plane's Django + Celery + Redis + RabbitMQ + Node backend is replaced by one small Rust binary
> (`axum` + `sqlx`). Plane's React Router SPAs are synced to `1.5.1` (`preview @ 723bb178f9`), rebranded to
> **Saturn** with a custom terracotta ringed-planet emblem and warm terracotta/parchment/obsidian default theme, built with **Bun `1.4.4`** (`bun.lock`), and served by
> the same binary as static files. Saturn keeps Plane's PostgreSQL schema, sessions, password hashes (with
> Argon2id default + transparent legacy `pbkdf2_sha256` upgrade), and API shapes so an existing Plane database
> works out of the box.

<div align="center">

## Quick Start

</div>

Run the containerized stack (`postgres` + hardened `axum` + non-root `nginx` reverse proxy):

```bash
cp .env.example .env
# Edit .env with real POSTGRES_PASSWORD, SECRET_KEY, and WEB_URL
docker compose up -d --build
docker compose exec axum saturn seed admin@example.com 'Change-me-2026' Acme
```

Or build and run natively without Docker:

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
| `/`          | Saturn web app                       |
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
Passwords and sessions are compatible, so users stay signed in and legacy `pbkdf2_sha256` hashes upgrade
to Argon2id automatically on login. Test on a copy first.

<div align="center">

## Benchmarks

</div>

<!-- BENCHMARK:START -->
Measured with [`tools/bench.sh`](tools/bench.sh) on 2026-11-11: Apple M3 (8 cores, 8 GB), macOS 27.0.1,
PostgreSQL 18.6, Redis 8.10, RabbitMQ (Homebrew). Both backends ran alone against the same PostgreSQL, each
with its own copy of the same seeded database (one admin, one workspace, one project, 2000 work items).
Load: [oha](https://github.com/hatoo/oha), 15 s, 32 concurrent connections, signed-in session cookie.
Plane ran its production settings with gunicorn (2 uvicorn workers), Celery worker and beat, and the Node live
server; Saturn ran its release build.

| Metric | Plane (Django + Celery + Redis + RabbitMQ + Node live) | Saturn (Rust) |
| --- | --- | --- |
| Backend processes | 21 (+ Redis, RabbitMQ) | 1 |
| Memory at idle (RSS) | 533 MB (+ 31 MB Redis/RabbitMQ) | 12 MB |
| Memory after load (RSS) | 379 MB | 19 MB |
| Startup to first response | 1.45 s | 0.03 s |
| Backend install size | 284 MB Python venv (plus Node for live) | 10 MB binary |
| `GET /api/users/me/` | 351 req/s, p50 90.4 ms, p99 234.9 ms | 8545 req/s, p50 3.7 ms, p99 5.3 ms |
| `GET .../issues/?per_page=100` | 17 req/s, p50 2032.8 ms, p99 3142.2 ms | 985 req/s, p50 30.3 ms, p99 79.3 ms |

Every request in both runs answered 200. Memory is a single RSS sample of the whole process tree (Plane's idle
sample came 8 s after start, while its workers were still warming up, so it reads higher than after load).
Numbers depend on the machine; rerun `tools/bench.sh <plane-checkout>` to measure your own.
<!-- BENCHMARK:END -->

### Size & Footprint Comparison (Original Plane vs Saturn Rewrite)

Measured directly against `../plane` (`preview @ 723bb178f9`) and `saturn` macOS arm64 release build:

| Metric | Original Plane (`../plane`) | Saturn Rewrite (`saturn`) | Difference |
| --- | --- | --- | --- |
| **Backend source LOC** | `131,251` (`126,151` Python `.py` + `5,100` `live/src` `.ts`/`.tsx`) | `37,348` (`39` Rust `.rs` files) | **−71.5% LOC** (`3.51×` smaller) |
| **Backend source size** | `5.54 MiB` (`apps/api` + `live` + `proxy` excl. `.venv`/`node_modules`) | `1.44 MiB` (`src/`, plus `0.29 MiB` SQL migrations) | **−74.0% source** |
| **Backend runtime footprint** | `246.01 MiB` (`284M` disk) Python `.venv` alone + Node `live` + brokers | `19.12 MiB` (`20,047,360` bytes / `19M`) single release binary | **−92.2% vs `.venv` alone** |
| **Frontend SPA bundles** | React Router workspace (`pnpm`) | React Router workspace built with **Bun `1.4.4`**: `web` `29.51 MiB`, `admin` `11.24 MiB`, `space` `13.72 MiB` | Self-contained static SPAs served by `saturn` |
| **Runtime processes** | `6+` (Django, Celery worker, Celery beat, Node live, proxy, Redis, RabbitMQ, PostgreSQL, MinIO) | `2` (`saturn` binary + PostgreSQL) | Single app process |
| **Cold startup to `/ready`** | Seconds (Django + workers + brokers) | `58–64 ms` | Instant readiness |
| **Idle memory RSS** | Hundreds of MiB across runtimes | `8.95 MiB` at boot (`14.39 MiB` after UI traffic) | Fits in `<15 MiB` |

<div align="center">

## Screenshots

<table>
  <tr>
    <td align="center"><a href="docs/screenshots/sign-in.png"><img src="docs/screenshots/sign-in.png" alt="Saturn Sign In" width="440" /></a><br>Sign In</td>
    <td align="center"><a href="docs/screenshots/home.png"><img src="docs/screenshots/home.png" alt="Saturn Workspace Home" width="440" /></a><br>Workspace Home</td>
  </tr>
  <tr>
    <td align="center"><a href="docs/screenshots/work-items.png"><img src="docs/screenshots/work-items.png" alt="Saturn Work Items" width="440" /></a><br>Work Items</td>
    <td align="center"><a href="docs/screenshots/admin.png"><img src="docs/screenshots/admin.png" alt="Saturn Instance Admin" width="440" /></a><br>Instance Admin (<code>/god-mode</code>)</td>
  </tr>
</table>

</div>

## Why & Scope

Self-hosted Plane runs about nine containers (web, space, admin, live, api, worker, beat-worker, migrator,
proxy) plus Postgres, Redis, RabbitMQ and MinIO, and needs several GB of RAM. Saturn runs as one process with
PostgreSQL as the only dependency.

Ported: full Plane web/admin/space backend parity — instance setup & god-mode, Argon2id + legacy PBKDF2
email/password auth, magic-link and OAuth sign in, SMTP outbound delivery with encrypted instance secrets,
current user / profile / activity / dashboard, workspaces, projects, states, work items & activity logs, cycles
(including analytics/burndown & transfer snapshots), modules, views & intake, pages & real-time Yjs
collaboration + `/live/{health,convert-document,pdf-export}` endpoints, notifications & digests, webhooks &
delivery, issue/analytics/activity exports (CSV, JSON, real OpenXML `.xlsx`), Unsplash & AI assistant, disk
asset storage, public boards, scheduled background cleanup jobs, and the public `/api/v1` API.

Intentionally out of scope: telemetry phone-home (`push_instance_metrics`) and multi-container
Redis/Celery/RabbitMQ/MinIO infrastructure. Unported `/api` or `/auth` paths answer `404`/`501`.

## Layout

```text
src/          Rust backend (axum + sqlx), one module per Plane API area
web/          Saturn Bun workspace (bun.lock): apps/web, apps/admin (god-mode), apps/space (public boards), packages/*
docker/       Container builds and reverse proxy config (`axum/Dockerfile`, `nginx/{Dockerfile,nginx.conf}`)
migrations/   PostgreSQL schema dumped from Plane's Django migrations, plus database defaults
tools/        API smoke test, E2E check, benchmark
scripts/      git hook checks, branch helper, Bun frontend build
assets/       brand logo SVGs, email templates, and workspace seed data
docs/         architecture, process docs, star chart, and retina UI screenshots (`docs/screenshots/`)
```

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

Smoke tests need `bash`, `curl`, `jq` and `psql`. Saturn has no Python dependency.

```bash
cargo test                                  # unit tests (Django session and password compatibility, helpers)
BASE=http://127.0.0.1:3000 tools/smoke.sh   # API smoke test against a running, seeded server
BASE=http://127.0.0.1:3000 node tools/e2e.mjs  # Plane web app in headless Chromium, lists failing API calls
scripts/build-web.sh                        # frontend builds
```

<div align="center">

## Star History

<a href="https://github.com/aphelion31/saturn/stargazers">
  <img src="docs/images/star-history.svg" alt="Saturn GitHub star history" width="760">
</a>

</div>

## License

AGPL-3.0 (see `LICENSE`). Saturn is derived from Plane (AGPL-3.0, © Plane Software, Inc.): it ships Plane's
frontend apps and keeps its database schema and API.
