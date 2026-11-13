# Webhook delivery over HTTP. Requires SATURN_WEBHOOK_ALLOW_PRIVATE=1 on the server (the sink is on loopback).
# Without it the SSRF guard must refuse the loopback URL, which is asserted instead.
WH_PORT=${WH_PORT:-3494}
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"url":"http://127.0.0.1:'"$WH_PORT"'/","issue":true}' "$BASE/api/workspaces/$SLUG/webhooks/")
if [[ "$code" == 400 ]]; then
  echo "✔ loopback webhook refused (SATURN_WEBHOOK_ALLOW_PRIVATE not set): delivery check skipped"
else
psql "${DATABASE_URL:-postgres://localhost/saturn}" -qc "DELETE FROM webhooks WHERE url='http://127.0.0.1:$WH_PORT/'"
WH_CAPTURE=$(mktemp)
# Test HTTP sink (examples/http_sink.rs): portable, accepts every retry, records raw requests.
WH_SINK=${WH_SINK:-./target/debug/examples/http_sink}
[[ -x "$WH_SINK" ]] || cargo build -q --example http_sink
"$WH_SINK" "127.0.0.1:$WH_PORT" "$WH_CAPTURE" 2>/dev/null &
WH_PID=$!
for _ in {1..50}; do curl -s -o /dev/null "http://127.0.0.1:$WH_PORT/" && break; sleep 0.1; done
: > "$WH_CAPTURE"
call POST "/api/workspaces/$SLUG/webhooks/" '{"url":"http://127.0.0.1:'"$WH_PORT"'/","issue":true,"is_active":true}'
WH_ID=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Webhook smoke"}'
for _ in {1..150}; do
  WH_COUNT=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "SELECT count(*) FROM webhook_logs WHERE webhook='$WH_ID' AND event_type='issue' AND response_status='200'")
  [[ "$WH_COUNT" -gt 0 ]] && break
  sleep 0.1
done
kill "$WH_PID" 2>/dev/null || true
wait "$WH_PID" 2>/dev/null || true
[[ "$WH_COUNT" -gt 0 ]] || { echo "✖ no 200 webhook_logs row; logs: $(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "SELECT response_status||' '||left(response_body,120) FROM webhook_logs WHERE webhook='$WH_ID'")" >&2; exit 1; }
grep -qi '^x-plane-event: issue' "$WH_CAPTURE" || { echo "✖ sink did not receive X-Plane-Event" >&2; exit 1; }
grep -qi '^x-plane-signature: [0-9a-f]\{64\}' "$WH_CAPTURE" || { echo "✖ missing HMAC signature header" >&2; exit 1; }
echo "✔ webhook delivered with X-Plane-Event and signature"
psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "SELECT request_body FROM webhook_logs WHERE webhook='$WH_ID' AND event_type='issue' LIMIT 1" | jq -e '.data.name == "Webhook smoke" and (.data.labels | type == "array") and (.data.assignees | type == "array")' >/dev/null
rm -f "$WH_CAPTURE"
call DELETE "/api/workspaces/$SLUG/webhooks/$WH_ID/"
fi
