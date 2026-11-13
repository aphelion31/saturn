# Live document persistence over a real websocket.
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/" '{"name":"Live smoke initial"}'
LIVE_PAGE=$(field '.id')
LIVE_COOKIE=$(awk '$6 == "sessionid" { print $6 "=" $7; exit }' "$jar")
[[ -n "$LIVE_COOKIE" ]] || { echo 'missing session cookie' >&2; exit 1; }
./target/debug/examples/live_ws_probe "$BASE" "$SLUG" "$PROJECT" "$LIVE_PAGE" "$LIVE_COOKIE"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$LIVE_PAGE/"
jq -e '.description_html | contains("Live smoke content")' <<<"$body" >/dev/null
jq -e '.name == "Live smoke title"' <<<"$body" >/dev/null
LIVE_VERSIONS=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "SELECT count(*) FROM page_versions WHERE page_id='$LIVE_PAGE' AND deleted_at IS NULL")
[[ "$LIVE_VERSIONS" -gt 0 ]] || { echo 'missing live page version' >&2; exit 1; }
echo '✔ live document content, title and version persisted'
