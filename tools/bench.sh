#!/usr/bin/env bash
# Benchmark Saturn against Plane's own backend on the same machine and the same PostgreSQL.
#
#   tools/bench.sh ../plane            # needs: postgres, redis, rabbitmq (brew), uv, node, pnpm, oha
#
# Both servers get an identical database: Plane's Django migrations, one admin, one workspace, one
# project and $ISSUES work items. Each server is measured alone:
#   - processes and resident memory (RSS) of everything the backend needs to run, after warm-up
#   - startup time until the first 200 from /api/instances/
#   - throughput and latency (oha) on GET /api/users/me/ and GET .../issues/?per_page=100 with a session cookie
# Results are printed as a Markdown table.
set -euo pipefail
PLANE=${1:?usage: tools/bench.sh <path-to-plane-checkout>}
PLANE=$(cd "$PLANE" && pwd)
ROOT=$(cd "$(dirname "$0")/.." && pwd)
ISSUES=${ISSUES:-2000}
DURATION=${DURATION:-15s}
CONC=${CONC:-32}
EMAIL=bench@saturn.dev
PASSWORD='Saturn-bench-2026'
SECRET=bench-secret-key-0123456789abcdefghijklmnopqrstuvwxyz
UV="uvx uv@$(grep -oE 'required-version = "==[0-9.]+"' "$PLANE/apps/api/pyproject.toml" | grep -oE '[0-9.]+[0-9]')"
OUT=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$OUT"' EXIT

log() { echo "• $*" >&2; }
rss_mb() { # total RSS (MB) of the given pids and their children
  local pids all
  pids="$*"
  all=$(for p in $pids; do echo "$p"; pgrep -P "$p" || true; done | sort -u | tr '\n' ' ')
  ps -o rss= -p "${all// /,}" 2>/dev/null | awk '{s+=$1} END {printf "%.0f", s/1024}'
}
nproc_of() { local all; all=$(for p in "$@"; do echo "$p"; pgrep -P "$p" || true; done | sort -u | wc -l); echo $all; }
wait_up() { # url -> seconds until 200
  local start; start=$(python3 -c 'import time; print(time.time())')
  for _ in $(seq 1 600); do
    if [[ "$(curl -s -o /dev/null -w '%{http_code}' "$1")" == 200 ]]; then
      python3 -c "import time; print(f'{time.time()-$start:.2f}')"; return 0
    fi
    sleep 0.1
  done
  echo "timeout"; return 1
}
fixture() { # db
  log "fixture: $ISSUES issues in $1"
  psql -qd "$1" <<SQL
INSERT INTO issues (name, description_html, description_stripped, priority, sequence_id, sort_order, project_id, workspace_id,
                    state_id, created_by_id, updated_by_id)
SELECT 'Work item ' || g, '<p>Benchmark item ' || g || '</p>', 'Benchmark item ' || g,
       (ARRAY['urgent','high','medium','low','none'])[1 + g % 5], 100 + g, 65535 + g * 10, p.id, p.workspace_id,
       (SELECT id FROM states s WHERE s.project_id = p.id AND NOT s.is_triage ORDER BY sequence OFFSET (g % 5) LIMIT 1),
       p.created_by_id, p.created_by_id
FROM projects p, generate_series(1, $ISSUES) g;
INSERT INTO issue_sequences (issue_id, sequence, deleted, project_id, workspace_id)
SELECT id, sequence_id, false, project_id, workspace_id FROM issues;
INSERT INTO issue_assignees (issue_id, assignee_id, project_id, workspace_id)
SELECT i.id, i.created_by_id, i.project_id, i.workspace_id FROM issues i WHERE i.sequence_id % 3 = 0;
ANALYZE;
SQL
}
login() { # base cookiejar -> session cookie value (Plane needs Django's CSRF token; Saturn ignores it)
  local csrf
  csrf=$(curl -s -c "$2" "$1/auth/get-csrf-token/" | python3 -c 'import json,sys
try: print(json.load(sys.stdin).get("csrf_token",""))
except Exception: print("")')
  curl -s -o /dev/null -b "$2" -c "$2" -H "Referer: $1/" \
    --data-urlencode "csrfmiddlewaretoken=$csrf" --data-urlencode "email=$EMAIL" --data-urlencode "password=$PASSWORD" \
    "$1/auth/sign-in/"
  local cookie; cookie=$(awk '$6=="session-id"{print $7}' "$2")
  [[ -n "$cookie" ]] || { log "login failed at $1"; exit 1; }
  echo "$cookie"
}
load() { # name url cookie -> "rps p50 p99"
  oha -z "$DURATION" -c "$CONC" --no-tui -j -H "Cookie: session-id=$3" "$2" > "$OUT/$1.json"
  python3 - "$OUT/$1.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
s, p = d["summary"], d["latencyPercentiles"]
ok = d.get("statusCodeDistribution", {})
print(f'{s["requestsPerSec"]:.0f} {p["p50"]*1000:.1f} {p["p99"]*1000:.1f} {ok}')
PY
}

# ---------------------------------------------------------------- Saturn
log "building Saturn (release)"
(cd "$ROOT" && cargo build --release -q)
dropdb --if-exists saturn_bench; createdb saturn_bench
DATABASE_URL=postgres://localhost/saturn_bench SECRET_KEY=$SECRET "$ROOT/target/release/saturn" seed "$EMAIL" "$PASSWORD" Bench >/dev/null 2>&1
fixture saturn_bench
PROJECT=$(psql -Atd saturn_bench -c "select id from projects limit 1")
log "starting Saturn"
DATABASE_URL=postgres://localhost/saturn_bench SECRET_KEY=$SECRET BIND=127.0.0.1:3900 WEB_URL=http://127.0.0.1:3900 \
  RUST_LOG=warn "$ROOT/target/release/saturn" >"$OUT/saturn.log" 2>&1 &
SAT_PID=$!
SAT_START=$(wait_up http://127.0.0.1:3900/api/instances/)
SAT_COOKIE=$(login http://127.0.0.1:3900 "$OUT/sat.jar")
SAT_IDLE=$(rss_mb $SAT_PID)
read -r SAT_ME_RPS SAT_ME_P50 SAT_ME_P99 SAT_ME_CODES < <(load sat_me http://127.0.0.1:3900/api/users/me/ "$SAT_COOKIE")
read -r SAT_IS_RPS SAT_IS_P50 SAT_IS_P99 SAT_IS_CODES < <(load sat_is "http://127.0.0.1:3900/api/workspaces/bench/projects/$PROJECT/issues/?per_page=100" "$SAT_COOKIE")
SAT_PEAK=$(rss_mb $SAT_PID)
SAT_PROCS=$(nproc_of $SAT_PID)
kill $SAT_PID; wait $SAT_PID 2>/dev/null || true
SAT_BIN=$(du -m "$ROOT/target/release/saturn" | cut -f1)
log "saturn codes: $SAT_ME_CODES $SAT_IS_CODES"

# ---------------------------------------------------------------- Plane
log "preparing Plane database"
dropdb --if-exists plane_bench; createdb plane_bench
export DATABASE_URL=postgres://localhost/plane_bench REDIS_URL=redis://localhost:6379/9 SECRET_KEY=$SECRET DEBUG=0 \
  WEB_URL=http://127.0.0.1:3901 APP_BASE_URL=http://127.0.0.1:3901 CORS_ALLOWED_ORIGINS=http://127.0.0.1:3901 \
  RABBITMQ_HOST=localhost USE_MINIO=0 DJANGO_SETTINGS_MODULE=plane.settings.production GUNICORN_WORKERS=${GUNICORN_WORKERS:-2}
(cd "$PLANE/apps/api" && $UV sync -q --group production 2>/dev/null || $UV sync -q)
(cd "$PLANE/apps/api" && $UV run python manage.py migrate -v 0 >/dev/null 2>&1)
# Same seed as Saturn: reuse Saturn's seed against the Plane-migrated database (same schema).
DATABASE_URL=postgres://localhost/plane_bench SECRET_KEY=$SECRET "$ROOT/target/release/saturn" seed "$EMAIL" "$PASSWORD" Bench >/dev/null 2>&1
psql -qd plane_bench -c "DROP TABLE saturn_migrations"
fixture plane_bench
PROJECT=$(psql -Atd plane_bench -c "select id from projects limit 1")
redis-cli -n 9 flushdb >/dev/null
log "starting Plane api (gunicorn x$GUNICORN_WORKERS), worker, beat, live"
cd "$PLANE/apps/api"
$UV run gunicorn -w "$GUNICORN_WORKERS" -k uvicorn.workers.UvicornWorker plane.asgi:application --bind 127.0.0.1:3901 \
  --log-level warning >"$OUT/plane-api.log" 2>&1 &
API_PID=$!
PLANE_START=$(wait_up http://127.0.0.1:3901/api/instances/)
$UV run celery -A plane worker -l warning >"$OUT/plane-worker.log" 2>&1 &
WORKER_PID=$!
$UV run celery -A plane beat -l warning >"$OUT/plane-beat.log" 2>&1 &
BEAT_PID=$!
cd "$PLANE/apps/live"
if [[ -f dist/start.mjs || -f dist/start.js ]]; then
  REDIS_URL=redis://localhost:6379/9 API_BASE_URL=http://127.0.0.1:3901 PORT=3902 LIVE_SERVER_SECRET_KEY=x node . >"$OUT/plane-live.log" 2>&1 &
  LIVE_PID=$!
else
  log "Plane live server not built (cd apps/live && pnpm build); counted as 0"
  LIVE_PID=""
fi
cd "$ROOT"
sleep 8
PLANE_COOKIE=$(login http://127.0.0.1:3901 "$OUT/plane.jar")
PIDS="$API_PID $WORKER_PID $BEAT_PID $LIVE_PID"
REDIS_PID=$(pgrep -x redis-server | head -1); RABBIT_PID=$(pgrep -f 'beam.smp' | head -1 || true)
PLANE_IDLE=$(rss_mb $PIDS)
INFRA_IDLE=$(rss_mb $REDIS_PID $RABBIT_PID)
read -r PL_ME_RPS PL_ME_P50 PL_ME_P99 PL_ME_CODES < <(load plane_me http://127.0.0.1:3901/api/users/me/ "$PLANE_COOKIE")
read -r PL_IS_RPS PL_IS_P50 PL_IS_P99 PL_IS_CODES < <(load plane_is "http://127.0.0.1:3901/api/workspaces/bench/projects/$PROJECT/issues/?per_page=100" "$PLANE_COOKIE")
PLANE_PEAK=$(rss_mb $PIDS)
PLANE_PROCS=$(nproc_of $PIDS)
log "plane codes: $PL_ME_CODES $PL_IS_CODES"
kill $PIDS 2>/dev/null || true
PLANE_VENV=$(du -sm "$PLANE/apps/api/.venv" | cut -f1)

cat <<MD
| Metric (same machine, same PostgreSQL, $ISSUES work items) | Plane (Django + Celery + Redis + RabbitMQ + Node live) | Saturn (Rust) |
| --- | --- | --- |
| Backend processes | $PLANE_PROCS (+ Redis, RabbitMQ) | $SAT_PROCS |
| Memory at idle (RSS) | $PLANE_IDLE MB (+ $INFRA_IDLE MB Redis/RabbitMQ) | $SAT_IDLE MB |
| Memory after load (RSS) | $PLANE_PEAK MB | $SAT_PEAK MB |
| Startup to first response | ${PLANE_START} s | ${SAT_START} s |
| Backend install size | $PLANE_VENV MB Python venv (plus Node for live) | $SAT_BIN MB binary |
| \`GET /api/users/me/\` | $PL_ME_RPS req/s, p50 $PL_ME_P50 ms, p99 $PL_ME_P99 ms | $SAT_ME_RPS req/s, p50 $SAT_ME_P50 ms, p99 $SAT_ME_P99 ms |
| \`GET .../issues/?per_page=100\` | $PL_IS_RPS req/s, p50 $PL_IS_P50 ms, p99 $PL_IS_P99 ms | $SAT_IS_RPS req/s, p50 $SAT_IS_P50 ms, p99 $SAT_IS_P99 ms |
MD
