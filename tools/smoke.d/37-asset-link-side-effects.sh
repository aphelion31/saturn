# Side effects ported from Plane Celery tasks:
# 1. crawl_work_item_link_title (issue_links.metadata)
# 2. get_asset_object_metadata (file_assets.storage_metadata)
# 3. copy_s3_objects_of_description_and_assets (pages duplicate)
# 4. page_transaction (page_logs)

DB_URL="${DATABASE_URL:-postgres://localhost/saturn}"

# Create a work item for link & mention tests
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Side effects smoke issue"}'
SE_ISSUE=$(field '.id')

# 1. Link crawl
CRAWL_PORT=${CRAWL_PORT:-3496}
CRAWL_CAPTURE=$(mktemp)
CRAWL_SINK=${WH_SINK:-./target/debug/examples/http_sink}
[[ -x "$CRAWL_SINK" ]] || cargo build -q --example http_sink
HTTP_SINK_BODY='<!doctype html><html><head><title>Saturn Smoke Link</title></head><body>hi</body></html>' \
  "$CRAWL_SINK" "127.0.0.1:$CRAWL_PORT" "$CRAWL_CAPTURE" 2>/dev/null &
CRAWL_PID=$!
for _ in {1..50}; do curl -s -o /dev/null "http://127.0.0.1:$CRAWL_PORT/" && break; sleep 0.1; done

call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/$SE_ISSUE/issue-links/" '{"title":"My Link","url":"http://127.0.0.1:'"$CRAWL_PORT"'/"}'
LINK_ID=$(field '.id')

if grep -q 'GET /' "$CRAWL_CAPTURE" 2>/dev/null || [[ "${SATURN_WEBHOOK_ALLOW_PRIVATE:-}" == "1" ]]; then
  for _ in {1..100}; do
    call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$SE_ISSUE/issue-links/$LINK_ID/"
    LINK_TITLE=$(jq -r '.metadata.title // ""' <<<"$body")
    [[ "$LINK_TITLE" == "Saturn Smoke Link" ]] && break
    sleep 0.1
  done
  [[ "$LINK_TITLE" == "Saturn Smoke Link" ]] || { echo "✖ link metadata.title=$LINK_TITLE (expected Saturn Smoke Link)" >&2; kill "$CRAWL_PID" 2>/dev/null || true; exit 1; }
  jq -e '.metadata.favicon | startswith("data:")' <<<"$body" >/dev/null
  echo "✔ work item link crawl populated metadata.title and favicon"
else
  # SSRF guard blocked loopback; still verify default favicon metadata is written
  for _ in {1..100}; do
    call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$SE_ISSUE/issue-links/$LINK_ID/"
    jq -e '.metadata.favicon | startswith("data:image/svg+xml;base64,")' <<<"$body" >/dev/null 2>&1 && break
    sleep 0.1
  done
  jq -e '.metadata.favicon | startswith("data:image/svg+xml;base64,")' <<<"$body" >/dev/null
  echo "✔ work item link crawl respected SSRF guard and saved fallback favicon"
fi
kill "$CRAWL_PID" 2>/dev/null || true
wait "$CRAWL_PID" 2>/dev/null || true
rm -f "$CRAWL_CAPTURE"

# 2. Asset storage_metadata on upload completion
PNG_TMP=$(mktemp)
printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\nIDATx\x9cc\x00\x01\x00\x00\x05\x00\x01\r\n-\xb4\x00\x00\x00\x00IEND\xaeB`\x82' > "$PNG_TMP"
PNG_SIZE=$(wc -c < "$PNG_TMP" | tr -d ' ')

call POST "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/" \
  '{"entity_type":"PAGE_DESCRIPTION","name":"smoke.png","size":'"$PNG_SIZE"',"type":"image/png"}'
ASSET_ID=$(field '.asset_id')
UPLOAD_URL=$(field '.upload_data.url')
UPLOAD_TOKEN=$(field '.upload_data.fields.token')

UP_CODE=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -F "token=$UPLOAD_TOKEN" -F "file=@$PNG_TMP" "$BASE$UPLOAD_URL")
rm -f "$PNG_TMP"
[[ "$UP_CODE" == "204" || "$UP_CODE" == "200" ]] || { echo "✖ asset upload returned $UP_CODE" >&2; exit 1; }

call PATCH "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/$ASSET_ID/" '{"is_uploaded":true}'
for _ in {1..100}; do
  META_LEN=$(psql "$DB_URL" -Atc "SELECT COALESCE((storage_metadata->>'ContentLength')::int, 0) FROM file_assets WHERE id='$ASSET_ID'")
  [[ "$META_LEN" == "$PNG_SIZE" ]] && break
  sleep 0.1
done
[[ "$META_LEN" == "$PNG_SIZE" ]] || { echo "✖ storage_metadata ContentLength=$META_LEN (expected $PNG_SIZE)" >&2; exit 1; }
META_CT=$(psql "$DB_URL" -Atc "SELECT storage_metadata->>'ContentType' FROM file_assets WHERE id='$ASSET_ID'")
[[ "$META_CT" == "image/png" ]] || { echo "✖ storage_metadata ContentType=$META_CT" >&2; exit 1; }
echo "✔ asset upload completion populated file_assets.storage_metadata"

# 3. Page transaction (page_logs) & 4. Page duplicate asset copy
TX_MENTION=$(psql "$DB_URL" -Atc "SELECT gen_random_uuid()")
TX_IMAGE=$(psql "$DB_URL" -Atc "SELECT gen_random_uuid()")
PAGE_HTML="<p><mention-component id=\"$TX_MENTION\" entity_identifier=\"$SE_ISSUE\" entity_name=\"issue\"></mention-component><image-component id=\"$TX_IMAGE\" src=\"$ASSET_ID\"></image-component></p>"

call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/" \
  "$(jq -nc --arg html "$PAGE_HTML" '{name:"Side effects page",description_html:$html}')"
SE_PAGE=$(field '.id')

for _ in {1..100}; do
  LOG_COUNT=$(psql "$DB_URL" -Atc "SELECT count(*) FROM page_logs WHERE page_id='$SE_PAGE'")
  [[ "$LOG_COUNT" == "2" ]] && break
  sleep 0.1
done
[[ "$LOG_COUNT" == "2" ]] || { echo "✖ expected 2 page_logs after page create, got $LOG_COUNT" >&2; exit 1; }
echo "✔ page create recorded mention and image page_logs"

# Remove mention from description_html and verify page_logs deletes the removed transaction
PAGE_HTML_2="<p><image-component id=\"$TX_IMAGE\" src=\"$ASSET_ID\"></image-component></p>"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/pages/$SE_PAGE/" \
  "$(jq -nc --arg html "$PAGE_HTML_2" '{description_html:$html}')"
for _ in {1..100}; do
  LOG_COUNT=$(psql "$DB_URL" -Atc "SELECT count(*) FROM page_logs WHERE page_id='$SE_PAGE'")
  [[ "$LOG_COUNT" == "1" ]] && break
  sleep 0.1
done
[[ "$LOG_COUNT" == "1" ]] || { echo "✖ expected 1 page_log after removing mention, got $LOG_COUNT" >&2; exit 1; }
echo "✔ page update synced page_logs deletions"

# Duplicate page and verify embedded image asset is copied and src rewritten
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/$SE_PAGE/duplicate/"
DUP_PAGE=$(field '.id')

NEW_ASSET_ID=""
for _ in {1..100}; do
  NEW_ASSET_ID=$(psql "$DB_URL" -Atc "SELECT id FROM file_assets WHERE page_id='$DUP_PAGE' AND deleted_at IS NULL LIMIT 1")
  [[ -n "$NEW_ASSET_ID" ]] && break
  sleep 0.1
done
[[ -n "$NEW_ASSET_ID" && "$NEW_ASSET_ID" != "$ASSET_ID" ]] || { echo "✖ duplicated page did not create new file_asset" >&2; exit 1; }

for _ in {1..100}; do
  DUP_HTML=$(psql "$DB_URL" -Atc "SELECT description_html FROM pages WHERE id='$DUP_PAGE'")
  [[ "$DUP_HTML" == *"$NEW_ASSET_ID"* && "$DUP_HTML" != *"$ASSET_ID"* ]] && break
  sleep 0.1
done
[[ "$DUP_HTML" == *"$NEW_ASSET_ID"* && "$DUP_HTML" != *"$ASSET_ID"* ]] || { echo "✖ duplicated page description_html did not rewrite asset src: $DUP_HTML" >&2; exit 1; }

DUP_LOGS=$(psql "$DB_URL" -Atc "SELECT count(*) FROM page_logs WHERE page_id='$DUP_PAGE'")
[[ "$DUP_LOGS" == "1" ]] || { echo "✖ expected 1 page_log on duplicated page, got $DUP_LOGS" >&2; exit 1; }
echo "✔ page duplicate copied file_asset, rewrote description_html src, and recorded page_logs"
