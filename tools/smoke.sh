#!/usr/bin/env bash
# API smoke test over real HTTP against a running, seeded Saturn binary.
#   saturn seed admin@saturn.dev 'Saturn-rocks-2026' Acme
#   BASE=http://127.0.0.1:3000 tools/smoke.sh
# Each area appends its own section. Fails on the first non-2xx answer.
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
field() { python3 -c "import json,sys; d=json.loads(sys.argv[1]); print(eval('d'+sys.argv[2]))" "$body" "$1"; }

# --- core: instance, auth, user ---
call GET /api/instances/
loc=$(curl -s -o /dev/null -w '%{redirect_url}' -c "$jar" -d "email=$EMAIL&password=$PASSWORD" "$BASE/auth/sign-in/")
[[ "$loc" != *error_code* ]] || { echo "✖ sign in failed: $loc" >&2; exit 1; }
echo "✔ sign in -> $loc"
call GET /api/users/me/
call GET /api/users/me/settings/
call GET /api/users/me/profile/
call PATCH /api/users/me/profile/ '{"language":"en"}'

# --- states ---
PROJECT=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select p.id from projects p join workspaces w on w.id=p.workspace_id where w.slug='$SLUG' and p.deleted_at is null order by p.created_at limit 1")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/states/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/states/" '{"name":"Smoke '$RANDOM'","color":"#123456","group":"started"}'
STATE=$(field "['id']")
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/states/$STATE/" '{"color":"#654321"}'
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/states/$STATE/"

echo "smoke ok"
