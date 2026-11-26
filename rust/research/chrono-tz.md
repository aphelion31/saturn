# Research: `chrono-tz` (`0.10`)

- Primary source: `chrono-tz` crate docs (`docs.rs/chrono-tz`) + Plane reference [`plane/utils/timezone_converter.py`](../../plane/apps/api/plane/utils/timezone_converter.py).
- Usage:
  - Parse IANA zone via `"Asia/Jakarta".parse::<chrono_tz::Tz>()`.
  - Resolve naive midnight `NaiveDate::and_hms_opt(0, 0, 0)` with `tz.from_local_datetime(&naive)`:
    - Returns `MappedLocalTime<DateTime<Tz>>` (`Single`, `Ambiguous(earliest, latest)`, `None` during DST spring-forward gap).
    - Semantic trap: `.unwrap()` panics on DST spring-forward gaps or ambiguous fall-back times! Use `.earliest()` or fallback to `and_hms_opt(1, 0, 0)` / UTC so DST transitions never panic or fail requests (`pytz.localize` picks earliest/is_dst=False by default).
  - Convert `.with_timezone(&chrono::Utc)` for PostgreSQL `timestamptz` storage, and `.with_timezone(&tz)` when computing project-local dates (`date_naive()`).
