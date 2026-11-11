#!/usr/bin/env bash
# API smoke test over real HTTP against a running, seeded Saturn binary.
#   saturn seed admin@saturn.dev 'Saturn-rocks-2026' Acme
#   BASE=http://127.0.0.1:3000 tools/smoke.sh
# Each API area has its own file in tools/smoke.d/<module>.sh, sourced in name order.
# Fails on the first non-2xx answer. Helpers: call METHOD PATH [JSON], field '.id', $SLUG, $PROJECT.
set -euo pipefail
BASE=${BASE:-http://127.0.0.1:3000}
EMAIL=${EMAIL:-admin@saturn.dev}
PASSWORD=${PASSWORD:-Saturn-rocks-2026}
SLUG=${SLUG:-acme}
jar=$(mktemp)
trap 'rm -f "$jar"' EXIT

call() { # call METHOD PATH [JSON]
  local out code
  out=$(curl -s -b "$jar" -X "$1" -H 'content-type: application/json' ${3:+-d "$3"} -w '\n%{http_code}' "$BASE$2")
  code=${out##*$'\n'}
  body=${out%$'\n'*}
  if [[ "$code" != 2* ]]; then echo "✖ $1 $2 -> $code $body" >&2; exit 1; fi
  echo "✔ $1 $2 -> $code"
}
field() { jq -r "$1" <<<"$body"; }

# --- core: instance, auth, user ---
call GET /api/instances/
loc=$(curl -s -o /dev/null -w '%{redirect_url}' -c "$jar" -d "email=$EMAIL&password=$PASSWORD" "$BASE/auth/sign-in/")
[[ "$loc" != *error_code* ]] || { echo "✖ sign in failed: $loc" >&2; exit 1; }
echo "✔ sign in -> $loc"
call GET /api/users/me/
call GET /api/users/me/settings/
call GET /api/users/me/profile/
call PATCH /api/users/me/profile/ '{"language":"en"}'

PROJECT=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select p.id from projects p join workspaces w on w.id=p.workspace_id where w.slug='$SLUG' and p.deleted_at is null order by p.created_at limit 1")

for f in "$(dirname "$0")"/smoke.d/*.sh; do
  echo "== $(basename "$f")"
  # shellcheck source=/dev/null
  source "$f"
done

# --- v1 assets/schema ---
call_v1 POST /api/v1/assets/user-assets/ '{"name":"avatar.png","type":"image/png","size":4,"entity_type":"USER_AVATAR"}'
V1_ASSET=$(field '.asset_id')
V1_UPLOAD=$(field '.upload_data.url')
V1_UPLOAD_TOKEN=$(field '.upload_data.fields.token')
code=$(curl -s -o /dev/null -w '%{http_code}' -F "token=$V1_UPLOAD_TOKEN" -F 'file=@-;filename=avatar.png;type=image/png' "$BASE$V1_UPLOAD" <<<"test")
[[ "$code" == 204 ]] || { echo "✖ asset upload -> $code" >&2; exit 1; }
call_v1 PATCH "/api/v1/assets/user-assets/$V1_ASSET/" '{}'
call_v1 DELETE "/api/v1/assets/user-assets/$V1_ASSET/"
call_v1 POST /api/v1/assets/user-assets/server/ '{"name":"cover.png","type":"image/png","size":4,"entity_type":"USER_COVER"}'
V1_ASSET=$(field '.asset_id')
call_v1 PATCH "/api/v1/assets/user-assets/$V1_ASSET/server/" '{}'
call_v1 DELETE "/api/v1/assets/user-assets/$V1_ASSET/server/"
call_v1 POST "/api/v1/workspaces/$SLUG/assets/" '{"name":"doc.txt","type":"text/plain","size":4,"project_id":"'"$PROJECT"'"}'
V1_ASSET=$(field '.asset_id')
call_v1 PATCH "/api/v1/workspaces/$SLUG/assets/$V1_ASSET/" '{"is_uploaded":true}'
call_v1 GET "/api/v1/workspaces/$SLUG/assets/$V1_ASSET/"
[[ $(field '.asset_name') == doc.txt ]] || exit 1
code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/v1/schema/")
[[ "$code" == 404 || "$code" == 200 ]] || { echo "✖ schema -> $code" >&2; exit 1; }

echo "smoke ok"
