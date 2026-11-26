# Research: `zxcvbn` (`3.1`)

- Primary source: `zxcvbn` crate docs (`docs.rs/zxcvbn`) + Plane reference [`plane/authentication/utils/password.py`](../../plane/apps/api/plane/authentication/utils/password.py).
- API in `zxcvbn = "3"`:
  - `zxcvbn::zxcvbn(password: &str, user_inputs: &[&str]) -> Entropy`
  - `entropy.score()` returns `zxcvbn::Score` enum (`Zero`, `One`, `Two`, `Three`, `Four`), implementing `Ord`/`PartialOrd`, so `entropy.score() >= zxcvbn::Score::Three` checks `score >= 3`.
- Semantic traps:
  - `zxcvbn` v3 does NOT return a `Result` (note: v2 returned `Result<Entropy, ZxcvbnError>`, v3 `zxcvbn()` returns `Entropy` directly). Handle empty string explicitly as `false` before calling.
  - Plane bounds scoring cost to 128-character prefix and suffix windows (`password[:128]` and `password[-128:]`). However, `zxcvbn` crate v3.1.1 (`src/lib.rs:165`) internally truncates inputs via `password.chars().take(100)` as a DoS guard, so passing a 128-char suffix window silently truncates the last 28 characters. Bound `MAX_LEN = 100` in Rust using `.chars().take(100)` / `.chars().skip(total - 100)` (never byte slicing `&s[..100]` on multi-byte UTF-8) so both windows are scored in full.
