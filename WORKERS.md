# Saturn worker contract

Goal: port Plane's Django API (`../plane/apps/api/plane`, read-only reference) to Rust endpoints in `src/`,
so Plane's unchanged React apps in `web/apps/*` work against the Saturn binary. Never edit `../plane`.

## Rules

- Edit only the module file(s) you own. Do not touch `main.rs`, `common.rs`, `session.rs`, `auth.rs`,
  `web.rs`, `Cargo.toml` or `migrations/`. If you need something there, message the coordinator.
- Your module exposes `pub fn routes() -> Router<AppState>` with FULL paths including the trailing slash,
  exactly as Plane's `urls/*.py` define them: `.route("/api/workspaces/{slug}/projects/{project_id}/cycles/", get(list).post(create))`.
- Copy the patterns in `src/states.rs` (the reference module):
  - Handler args: `State(st): State<AppState>`, `user: CurrentUser`, `Path((slug, project_id)): Path<(String, Uuid)>`,
    then `Json(b): Json<Value>` for bodies.
  - First line: `let p = require_project(&st.db, &user, &slug, project_id).await?;` then
    `p.require(&[ADMIN, MEMBER])?;` with the roles from Plane's `@allow_permission([...])` decorator.
    Workspace-level routes: `let ws = require_workspace(&st.db, &user, &slug).await?; ws.require(&[ADMIN])?;`.
  - Filter EVERY query by `workspace_id` and/or `project_id` from the context. Add `deleted_at IS NULL` on reads.
  - Build response JSON in SQL with `jsonb_build_object(...)` constants and return with `json_rows` / `json_row`.
    Keys must equal the DRF serializer fields (`plane/app/serializers/*.py`) and the TS types (`web/packages/types/src`).
  - Errors: `ApiError::bad("msg")` -> 400 `{"error": msg}`, `ApiError::bad_body(json!({"name": ["..."]}))`,
    `ApiError::not_found()`, `ApiError::forbidden()`. `?` on sqlx works (unique violation -> 409).
  - Status codes as Plane: create usually `201` (`created(v)`), delete `204` (`no_content()`).
  - Delete = soft delete: `UPDATE ... SET deleted_at = now()`.
  - PATCH with many optional columns: `crate::users::patch_sets(&body, FIELDS, n)` with a fixed whitelist.
- Runtime `sqlx::query`/`query_as`/`query_scalar` only, never the `query!` macros. Bind every value.
- Types: ids are `uuid` (`Uuid`), roles `smallint` (`i16`), timestamps `timestamptz` (use `chrono::DateTime<Utc>`
  or let Postgres put them into JSON), `sort_order` is `float8` (`f64`).
- Work-item lists (project issues, cycle/module issues, views, archived, drafts) MUST use
  `workitems::list(&st.db, Base::new(where_sql, args), &query_map, Scope::...)`. It implements Plane's filters,
  `order_by`, `group_by`/`sub_group_by`, cursor pagination and response envelope. Pass your own base WHERE on
  alias `i` (start from `workitems::ACTIVE` for normal lists) and `base.and("i.created_by_id = {}", ...)` for
  restricted guests. Single work-item JSON: `workitems::ITEM_JSON`. Do not reimplement grouping.
- Helpers: `common::{AppState, ApiError, ApiResult, CurrentUser, MaybeUser, require_workspace, require_project,
  ADMIN, MEMBER, GUEST, json_rows, json_row, created, no_content, s, uuid_of, strip_tags}`,
  `users::{USER_LITE_JSON, patch_sets}`, `states::{STATE_JSON, slugify}`.
- Ponytail: smallest correct implementation. Skip Celery side effects (emails, webhooks, activity fan-out) unless
  the UI reads the result; mark skipped bits with `// ponytail: ...`. Do write `issue_activities` rows where the
  UI shows an activity feed.
- Verify before reporting:
  1. `cargo build` with no errors, `cargo test` green.
  2. Run your own server on a private port and DB:
     `createdb saturn_<you> && DATABASE_URL=postgres://localhost/saturn_<you> ./target/debug/saturn seed a@b.co 'Saturn-rocks-2026' Acme`
     then `DATABASE_URL=postgres://localhost/saturn_<you> BIND=127.0.0.1:33xx WEB_URL=http://127.0.0.1:33xx ./target/debug/saturn &`
  3. Sign in: `curl -s -c jar -d 'email=a@b.co&password=Saturn-rocks-2026' http://127.0.0.1:33xx/auth/sign-in/`
     then call your endpoints with `-b jar`. Kill your server and `dropdb saturn_<you>` after.
  4. Add your endpoints to `tools/smoke.d/NN-<module>.sh` (your own file; see `10-states.sh`).
- Commit only your files on your lane branch: `git add src/<module>.rs tools/smoke.d/NN-<module>.sh && git commit -m "feat(<module>): ..."`.
  Conventional Commits, English, no AI co-author lines. Never `git add -A`.

## Module ownership

| Module               | Plane source                                                                     |
| -------------------- | -------------------------------------------------------------------------------- |
| `workspaces.rs`      | `app/urls/workspace.py`, `app/views/workspace/*`, `user.py` workspace routes     |
| `projects.rs`        | `app/urls/project.py`, `app/views/project/*`, estimates, labels                  |
| `issues.rs`          | `app/urls/issue.py`, `app/views/issue/*`, `search.py`                            |
| `cycles.rs`          | `app/urls/cycle.py`, `app/views/cycle/*`                                         |
| `modules.rs`         | `app/urls/module.py`, `app/views/module/*`                                       |
| `views.rs`           | `app/urls/views.py`, `app/views/view/*`, `intake.py`                             |
| `pages.rs`           | `app/urls/page.py`, `app/views/page/*`, live collaboration websocket             |
| `notifications.rs`   | `app/urls/notification.py`, stickies, recent visits, quick links, home prefs     |
| `assets.rs`          | `app/urls/asset.py`, `app/views/asset/v2.py` (disk storage in `STORAGE_DIR`)     |
| `public.rs`          | `space/urls/*`, `space/views/*` (public boards at `/api/public/...`)             |
