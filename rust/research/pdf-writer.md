# Research: `pdf-writer` (`0.15`)

- Primary source: `pdf-writer` crate docs (`docs.rs/pdf-writer`) + Plane reference [`plane/apps/live/src/services/pdf-export/pdf-export.service.ts`](../../plane/apps/live/src/services/pdf-export/pdf-export.service.ts).
- Pure-Rust zero-C-dep library generating PDF 1.7 bytes directly into `Vec<u8>`.
- Page sizes (in PDF points, 72 pt = 1 inch):
  - `A4`: 595.28 x 841.89
  - `A3`: 841.89 x 1190.55
  - `A2`: 1190.55 x 1683.78
  - `LETTER`: 612.0 x 792.0
  - `LEGAL`: 612.0 x 1008.0
  - `TABLOID`: 792.0 x 1224.0
  - Swap width & height for `landscape`.
- Semantic traps:
  - Standard PDF Type1 Base14 fonts (`Helvetica`, `Helvetica-Bold`, `Helvetica-Oblique`, `Courier`) use `WinAnsiEncoding` (`encoding.win_ansi()`). Characters outside WinAnsi (emoji, CJK, arbitrary Unicode) should be safely mapped/transliterated to `?` or ASCII equivalents instead of emitting corrupted byte sequences in `Str(bytes)`.
  - `Content::show(Str(...))` handles PDF literal string escaping in `pdf-writer`, using `&[u8]` encoded in WinAnsi/Latin-1.
  - Document catalog/pages tree must allocate consecutive or explicitly tracked `Ref` IDs via `Pdf::new()`, finishing with `pdf.finish()` to produce valid `%PDF-1.7` + `%%EOF`.
