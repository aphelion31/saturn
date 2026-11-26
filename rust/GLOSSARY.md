# Glossary

| Plane term / module | Meaning | Saturn location |
| --- | --- | --- |
| `Workspace` / `WorkspaceMember` | Top-level tenant and role (`20` admin, `15` member, `5` guest) | `src/common.rs` (`WorkspaceCtx`, `require_workspace`), `src/workspaces.rs` |
| `Project` / `ProjectMember` | Project within workspace; `guest_view_all_features` controls guest scope | `src/common.rs` (`ProjectCtx`, `require_project`), `src/projects.rs` |
| `Issue` / Work item | Core tracked item; soft-deleted via `deleted_at` | `src/issues.rs`, `src/workitems.rs` |
| `Cycle` / `CycleProgress` / `burndown_plot` | Time-boxed iteration with progress & burndown charts and `progress_snapshot` | `src/cycles.rs` (`compute_cycle_distribution`, `analytics`, `transfer`) |
| `Intake` / `IntakeIssue` | Triage inbox (`status`: `-2` pending, `-1` rejected, `0` snoozed, `1` accepted, `2` duplicate) | `src/views.rs` (`intake_issue_*`, `ensure_default_intake`) |
| `Page` / `PageVersion` / `PageLog` | Collaborative doc backed by Yjs binary + HTML/JSON | `src/pages.rs`, `src/live_doc.rs`, `src/live_http.rs` |
| `apps/live` (`/health`, `/convert-document`, `/pdf-export`, `/collaboration`) | Plane Node/Hocuspocus live server | `src/pages.rs` (WS `/live/collaboration`), `src/live_http.rs` (HTTP endpoints) |
| `soft_delete_related_objects` | Recursive soft-delete/SET_NULL cascade using Django FK graph | `src/jobs.rs` (`soft_delete_cascade`), `src/jobs_fk.rs` (`PLANE_FKS`) |
| `DataExporter` (`csv`, `json`, `xlsx`) | Workspace issue & analytics async exports stored under `STORAGE_DIR` | `src/external.rs` (`run_issue_export_bg`, `download_export`) |
| `snooze_day_start` | Midnight in `projects.timezone` for bare `YYYY-MM-DD` intake snooze | `src/common.rs` (`tz_day_start`), `src/views.rs` |
| `is_password_strong` | `zxcvbn` score >= 3 on 128-char prefix/suffix windows | `src/auth.rs` |
| `SpectacularAPIView` | OpenAPI 3.0.3 schema, Swagger UI, ReDoc (`ENABLE_DRF_SPECTACULAR=1`) | `src/api_v1_assets.rs` |
