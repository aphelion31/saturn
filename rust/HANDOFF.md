# Saturn Port Handoff (`v0.2.0`)

## Target Baseline
- Upstream reference: `../plane` (`preview` @ `723bb178f9`, post-`1.5.1`), kept untouched and read-only.
- Rust service: `/Users/beyondb/Halim/Projects/CEG/prjtss/saturn` (`39` modules under `src/`, `37,348` Rust LOC, `403` `.route(...)` registrations, single `20,047,360`-byte / `19M` release binary).
- Frontend SPAs: `web/apps/{web,admin,space}` synced to Plane `preview @ 723bb178f9`, branded as **Saturn** with a custom geometric ringed-planet SVG logo (`PlaneLogo` / `PlaneLockup` / `LogoSpinner` + generated multi-resolution favicons) and built with **Bun `1.4.4`** (`bun.lock`, `workspaces.catalog` in `web/package.json`).

## Architecture Decisions
- `ADR-001` .. `ADR-006` recorded in [DECISIONS.md](DECISIONS.md):
  - Pure-Rust Axum + sqlx single-binary replacement for Django, Celery, Redis, RabbitMQ, and `apps/live` Node servers.
  - Pure-Rust crates (`chrono-tz`, `zxcvbn`, `rust_xlsxwriter`, `pdf-writer`) with zero external C toolchain build dependencies.
  - Live `rust-anything app record` / `app replay` JSON contract alignment (`/api/instances/`, v1 pagination envelope, v1 `IssueSerializer` projection `ITEM_V1`, `project_user_properties.sort_order` fallback).
  - Intentional non-semantic diffs under `ADR-006`: SPA-compatible Axum security/CORS headers vs DRF headers, and PostgreSQL JSONB whole-float formatting (`65535` vs `65535.0`).

## Verification Evidence
- `cargo fmt --check`: pass.
- `cargo clippy --all-targets -- -D warnings`: pass (0 warnings).
- `cargo test --locked`: `48 passed; 0 failed`.
- `ENABLE_DRF_SPECTACULAR=1 SATURN_WEBHOOK_ALLOW_PRIVATE=1 tools/smoke.sh` + `SIGTERM`: all 40 smoke scripts (`00`–`99c-parity-live`) passed and `kill -TERM` exited `0`.
- `rust-anything app record` (live Plane `723bb178f9`) -> `rust-anything app replay` (Saturn release binary): `0` semantic JSON key/type/value diffs across all 7 recorded cases (4/7 body-identical; `v1-projects-list`, `v1-states-list`, `v1-work-items-list` differ only on `.0` whole-float formatting and HTTP headers per `ADR-006`).
- `./scripts/build-web.sh` (`bun install --frozen-lockfile && turbo run build --filter=web --filter=admin --filter=space`): `12 successful, 12 total`.
- Local demo runtime + headless Chromium retina (`1440x900@2x`) inspection of `docs/screenshots/{sign-in,home,work-items,admin}.png`: `58–64 ms` cold boot to `GET /ready == 200`, `9,168 KiB` idle RSS at boot (`14,736 KiB` after UI traffic), zero HTTP 500/404 or console errors.

## Ori (`plane`) vs Rewrite (`saturn`) Snapshot
| Metric | Plane (`../plane`) | Saturn (`saturn`) |
| --- | --- | --- |
| Backend language & modules | Python + TypeScript (`784` `.py` + `49` `live/src` `.ts`/`.tsx`) | Rust (`39` `.rs` files) |
| Backend code LOC | `131,251` (`126,151` `.py` + `5,100` `live/src`) | `37,348` |
| Backend source size | `5.54 MiB` (`api` + `live` + `proxy` excl. `.venv`/`node_modules`) | `1.44 MiB` (`src/`, plus `0.29 MiB` SQL migrations) |
| Runtime artifact footprint | `246.01 MiB` (`284M` disk) Python `.venv` alone + Node `live` + Redis/RabbitMQ/MinIO | `19.12 MiB` (`20,047,360` bytes) single binary + static SPAs (`54.48 MiB` / `61M` on disk) |
| Required runtime processes | `6+` (Django, Celery worker/beat, Node live, proxy, Redis, RabbitMQ, PG, MinIO) | `2` (`saturn` + PostgreSQL) |
| Cold boot to `/ready` | Seconds (Django + workers + brokers) | `58–64 ms` |
| Idle memory RSS | Hundreds of MiB across runtimes | `8.95 MiB` (`9,168 KiB`) idle |
