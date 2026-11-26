#!/usr/bin/env bash
# Interactive production cutover helper from Plane to Saturn (keeps Plane intact until cutover verified).
set -euo pipefail

SATURN_BIN="${SATURN_BIN:-./target/release/saturn}"
DATABASE_URL="${DATABASE_URL:-postgres://localhost/saturn}"
BASE_URL="${BASE_URL:-http://127.0.0.1:3000}"

step() {
  echo
  echo "==> $1"
  read -r -p "Press Enter to continue (Ctrl-C to abort)..."
}

echo "Saturn Cutover Checklist"
echo "Binary:       $SATURN_BIN"
echo "Database URL: $DATABASE_URL"
echo "Saturn URL:   $BASE_URL"

step "1. Take a full PostgreSQL backup (pg_dump) and snapshot STORAGE_DIR / S3 uploads before running migrations."

step "2. Run Saturn migrations against a copy of your Plane database first (safe & idempotent):"
DATABASE_URL="$DATABASE_URL" "$SATURN_BIN" migrate

step "3. Verify /health and /ready endpoints on the running Saturn instance ($BASE_URL):"
curl -sf "$BASE_URL/health" && echo
curl -sf "$BASE_URL/ready" && echo
curl -sf "$BASE_URL/live/health" && echo

step "4. Run API smoke checks (optional, against staging/copy database):"
echo "Run manually if desired: BASE=$BASE_URL DATABASE_URL=$DATABASE_URL tools/smoke.sh"

step "5. Cut over reverse proxy traffic (Caddy/nginx/Traefik) to Saturn, keeping Plane stopped (do NOT drop Plane containers/data until verified)."
echo "Rollback path: point reverse proxy back to Plane proxy and restart Plane containers; schema & password hashes remain 100% compatible."
