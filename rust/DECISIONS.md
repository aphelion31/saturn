# Decisions

## ADR-001: Finish Saturn in place with rust-anything ground-truth verification
Status: accepted (2026-11-25)
Context: `saturn` already implements 386 Axum routes (38 modules, 34.4k LOC) matching Plane's React SPAs and PostgreSQL schema, while `plane/` (`v1.5.0-rc1`) is the read-only Django/Node reference. Scaffolding a second parallel PyO3 bridge inside `plane/rust/` would discard the working standalone binary.
Decision: Finish `saturn` in place as the pure-Rust Axum + sqlx application, keeping `plane/` read-only reference. Maintain `saturn/rust/` whole-app tracking artifacts (`MAP.md`, `DECISIONS.md`, `GLOSSARY.md`, `research/`, `app-cases/`, `cutover.sh`) and verify parity via `rust-anything app record` (against Plane) and `rust-anything app replay` (against Saturn).
Consequences: No Python/Node/Redis/Celery/RabbitMQ runtime dependencies; all behavioral parity is tested over real HTTP against a live Plane reference.

## ADR-002: Scope of remaining parity & production work
Status: accepted (2026-11-25)
Context: Auditing `saturn` against `plane/apps/api` and `plane/apps/live` identified concrete remaining gaps (`ponytail:` stubs, unported `apps/live` HTTP endpoints, and production deployment readiness) plus deliberately out-of-scope infrastructure.
Decision:
- In scope: Wave 2 API/logic parity (`cycles` analytics/burndown/transfer distribution/project TZ, `workspaces` user profile `project_data` & `user_activity` feed, `user_dash` full `issue_detail`, `views` intake `labels`/`assignees` & `snooze_day_start`, `projects` default intake creation, `pages` descendant archive/unarchive propagation + favorite cleanup, `external` real OpenXML `.xlsx`, `auth` `zxcvbn >= 3`, `jobs` soft-delete cascade via `PLANE_FKS`, `api_v1_assets` method-complete OpenAPI + `/api/schema/*`, `web` `/robots.txt`), Wave 3 `apps/live` HTTP parity (`/live/health`, `/live/convert-document`, `/live/pdf-export`), Wave 4 production readiness (graceful shutdown flushing active WS collab rooms, `/ready` DB probe, zero clippy warnings, `Dockerfile`, `docker-compose.prod.yml`, updated docs), and Wave 5 CI + release `v0.2.0`.
- Out of scope: telemetry phone-home (`push_instance_metrics`), Django CLI commands beyond `migrate`/`seed`/`jobs run`, Redis/Celery/RabbitMQ/MinIO infra, and rewriting `web/` React SPAs.
Consequences: Single-binary Postgres-only architecture is preserved while eliminating functional gaps visible to API/web/live clients.

## ADR-003: Equivalence bar & documented intentional diffs
Status: accepted (2026-11-25)
Context: HTTP replay against Plane includes dynamic identifiers, timestamps, and floating-point analytics totals, while matching Plane `1.5.1` (`preview`) API, security, and tenant isolation contracts.
Decision: Require matching HTTP status codes, JSON keys, types, and normalized values (`uuids`, `timestamps`, `key-order`, `1e-9` float tolerance). `GET /api/workspaces/{slug}/entity-search/` aligns with Plane PR #10213 (`200 {query_type: []}` for non-project members and blocking workspace guests from non-member `network=2` projects). Any remaining unavoidable difference during `app replay` must be fixed or explicitly recorded in an accepted ADR.
Consequences: Mismatches during `app replay` are treated as Saturn bugs unless explicitly covered by an accepted ADR.

## ADR-004: Pure-Rust dependencies for remaining features
Status: accepted (2026-11-25)
Context: Remaining features require IANA timezone resolution (`pytz`), password strength scoring (`zxcvbn`), OpenXML `.xlsx` generation (`openpyxl`), and PDF generation (`@react-pdf`/custom PDF builder).
Decision: Add pure-Rust crates with zero external C toolchain build dependencies:
- `chrono-tz` (`0.10`) for `pytz`-compatible IANA timezone conversion (`convert_to_utc`, `convert_utc_to_project_timezone`, `snooze_day_start`).
- `zxcvbn` (`3`) for password strength scoring (`score >= 3` on 100-char prefix/suffix windows, aligned with `zxcvbn` 3.1.1's internal 100-char `chars().take(100)` cap so trailing tail entropy is scored in full).
- `rust_xlsxwriter` (`0.104`) for valid OpenXML `.xlsx` workbooks with CSV/formula injection sanitization.
- `pdf-writer` (`0.15`) for multi-page PDF 1.7 generation in `/live/pdf-export`.
Consequences: `cargo build --release --locked` stays self-contained on Linux (`bookworm-slim`) and macOS.

## ADR-005: Production deployment & release workflow
Status: accepted (2026-11-25)
Context: Saturn must be deployable as a production container/binary with graceful lifecycle management and clean git release history.
Decision: Expose `GET /ready` (PostgreSQL `SELECT 1`), handle `SIGINT`/`SIGTERM` graceful shutdown with room flushing and DB pool close, ship a multi-stage `Dockerfile` + `docker-compose.prod.yml` + `.env.prod.example`, enforce `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --locked`, `tools/smoke.sh`, `rust-anything app replay`, and `scripts/build-web.sh`, then PR into `develop` and release `release/0.2.0` -> `master` (`v0.2.0`).
Consequences: Production deployments get zero-warning builds, readiness health checks, safe WS room persistence on stop, and verifiable release artifacts.


## ADR-006: HTTP response header and JSONB numeric representation under Axum
Status: accepted (2026-11-26)
Context: Replaying live Plane `preview` (`723bb178f9`) HTTP captures against Saturn revealed four genuine JSON field differences (fixed in `src/auth.rs`, `src/api_v1.rs`, `src/api_v1_items.rs`, `src/workitems.rs`, and `src/projects.rs`) plus two expected server-layer differences: Django/DRF middleware headers (`Allow`, `Cache-Control`, `Cross-Origin-Opener-Policy`, `Referrer-Policy: same-origin`, `Vary: Accept-Encoding, Cookie`, `X-Frame-Options: DENY`, `X-RateLimit-*`) versus Saturn's unified Axum SPA+API headers (`X-Frame-Options: SAMEORIGIN`, `Referrer-Policy: origin-when-cross-origin`, CORS `Vary`), and PostgreSQL `jsonb_build_object` integer formatting for whole floats (`65535` vs Python `65535.0`).
Decision: Keep Saturn's unified SPA-compatible security headers and PostgreSQL JSONB number representation unchanged, while fixing all JSON key, nullable base URL, pagination envelope, and v1 work-item serializer parity gaps.
Consequences: JSON consumers receive 100% compatible keys, types, and values while preserving single-binary SPA embedding and zero unnecessary JSON re-serialization overhead.
