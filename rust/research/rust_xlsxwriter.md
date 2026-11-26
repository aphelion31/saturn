# Research: `rust_xlsxwriter` (`0.104`)

- Primary source: `rust_xlsxwriter` crate docs (`docs.rs/rust_xlsxwriter`) + Plane reference [`plane/utils/porters/formatters.py`](../../plane/apps/api/plane/utils/porters/formatters.py).
- Pure-Rust dependency profile: uses `zip` / `flate2` in pure Rust (`default-features` includes no C library).
- Usage:
  - `let mut workbook = rust_xlsxwriter::Workbook::new();`
  - `let worksheet = workbook.add_worksheet();`
  - `worksheet.write_string(row, col, &cell)?;`
  - `let bytes: Vec<u8> = workbook.save_to_buffer()?;`
- Semantic traps:
  - `write_string` writes literal string values (`<t>` / shared strings, not formulas unless `write_formula` is called), AND we still prefix leading `=`, `+`, `-`, `@`, `\t`, `\r` with `'` (`sanitize_csv_cell`) to match Plane's `sanitize_csv_row` defense-in-depth when exported files are re-opened in Excel/LibreOffice.
  - MIME type for `.xlsx` download is `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`.
