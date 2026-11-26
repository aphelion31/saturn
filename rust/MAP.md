# Whole-App Port Map (`plane` -> `saturn`)

Goal: Complete remaining Plane API + `apps/live` parity, verify against live Plane with `rust-anything app record` / `app replay`, harden for production, and release `v0.2.0`.

- Decisions index: [DECISIONS.md](DECISIONS.md) (`ADR-001` .. `ADR-006`)
- Domain glossary: [GLOSSARY.md](GLOSSARY.md)
- Crate research: [research/](research/)
- Final handoff: [HANDOFF.md](HANDOFF.md)

## Wave Status

| Wave | Unit | Target files | State | Notes |
| --- | --- | --- | --- | --- |
| Wave 1 | Existing core parity | `src/*.rs` | `done` | Baseline `09de081`, 42 unit tests passing |
| Wave 2.1 | Cycle analytics, burndown, transfer snapshot, project TZ | `src/common.rs`, `src/cycles.rs` | `done` | `chrono-tz` IANA conversion + full analytics |
| Wave 2.2 | Workspace `user_profile` `project_data`, `user_activity`, `issue_detail` | `src/workspaces.rs`, `src/user_dash.rs` | `done` | Live activity & per-project breakdown |
| Wave 2.3 | Intake `labels`/`assignees`, `snooze_day_start`, default intake on project create | `src/views.rs`, `src/projects.rs` | `done` | Mirrors `IntakeViewSet` + `IntakeIssueSerializer` |
| Wave 2.4 | Page archive descendant propagation + favorite cleanup | `src/pages.rs` | `done` | Recursive CTE + `parent_id` persistence + `user_favorites` delete |
| Wave 2.5 | OpenXML `.xlsx` issue export | `src/external.rs` | `done` | `rust_xlsxwriter` real `.xlsx` bytes + formula sanitization |
| Wave 2.6 | `zxcvbn` password strength | `src/auth.rs` | `done` | `score >= 3` on 100-char prefix/suffix windows |
| Wave 2.7 | Soft-delete cascade across entity deletes | `src/jobs.rs`, `src/jobs_fk.rs`, `src/{workspaces,projects,cycles,modules,issues,pages}.rs` | `done` | Full `PLANE_FKS` graph traversal |
| Wave 2.8 | Method-complete OpenAPI schema & `/robots.txt` | `src/api_v1_assets.rs`, `src/web.rs`, `src/main.rs` | `done` | Covers `/api/v1/schema/*` and `/api/schema/*` (`ENABLE_DRF_SPECTACULAR=1`) |
| Wave 3 | `apps/live` HTTP parity (`/live/health`, `/live/convert-document`, `/live/pdf-export`) | `src/live_doc.rs`, `src/live_http.rs`, `src/main.rs` | `done` | Pure Rust `html5ever` + `yrs` + `pdf-writer` |
| Wave 4 | Production hardening, React Router `1.5.1` sync, Saturn branding & Bun `1.4.4` migration | `src/*.rs`, `Dockerfile`, `docker-compose.prod.yml`, `web/`, `docs/` | `done` | Zero `clippy -D warnings`, `/ready`, SIGTERM flush, custom Saturn SVG logo, Bun SPAs |
| Wave 5 | Ground-truth `app record`/`app replay`, `smoke.sh`, screenshots, README, PRs & `v0.2.0` release | `rust/app-cases/`, `tools/smoke.d/99c-parity-live.sh`, `README.md` | `done` | 0 semantic JSON replay diffs, 48 unit tests, 40 smoke scripts, `19M` binary |

## Out of Scope (`ADR-002`)
- Telemetry phone-home (`push_instance_metrics`)
- Django CLI management commands beyond `migrate`, `seed`, `jobs run`
- Redis / Celery / RabbitMQ / MinIO infrastructure
- Rewriting `web/` React Router SPAs
