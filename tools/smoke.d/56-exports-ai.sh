# External integrations: unsplash, ai-assistant, exports (issues, analytics, user-activity)

# 1. Unsplash
# Anonymous request rejected
code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/unsplash/")
[[ "$code" == 401 ]] || { echo "anonymous unsplash returned $code, expected 401" >&2; exit 1; }

# Authenticated without key returns empty array
call GET /api/unsplash/
[[ $(jq -r 'length' <<< "$body") == 0 ]] || { echo "unsplash without key returned non-empty" >&2; exit 1; }

# 2. AI Assistant
# Missing LLM config returns 400 error
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"task":"summarize"}' "$BASE/api/workspaces/$SLUG/ai-assistant/")
[[ "$code" == 400 ]] || { echo "workspace ai-assistant without config returned $code, expected 400" >&2; exit 1; }

code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"task":"summarize"}' "$BASE/api/workspaces/$SLUG/projects/$PROJECT/ai-assistant/")
[[ "$code" == 400 ]] || { echo "project ai-assistant without config returned $code, expected 400" >&2; exit 1; }

# Cross-workspace isolation
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"task":"summarize"}' "$BASE/api/workspaces/foreign-slug-$RANDOM/ai-assistant/")
[[ "$code" == 403 || "$code" == 404 ]] || { echo "ai-assistant with foreign workspace returned $code, expected 403/404" >&2; exit 1; }

# 3. Issue Exports
# Invalid provider returns 400
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"provider":"invalid_fmt"}' "$BASE/api/workspaces/$SLUG/export-issues/")
[[ "$code" == 400 ]] || { echo "export-issues with invalid provider returned $code, expected 400" >&2; exit 1; }

# Valid CSV and JSON export
call POST "/api/workspaces/$SLUG/export-issues/" '{"provider":"csv"}'
call POST "/api/workspaces/$SLUG/export-issues/" '{"provider":"json"}'

# List exporters paginated
call GET "/api/workspaces/$SLUG/export-issues/?cursor=10:0:0&per_page=10"
[[ $(field '.results | length') -gt 0 ]] || { echo "export list empty" >&2; exit 1; }

# Without pagination query params returns 400
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" "$BASE/api/workspaces/$SLUG/export-issues/")
[[ "$code" == 400 ]] || { echo "export-issues without cursor returned $code, expected 400" >&2; exit 1; }

# Wait for background export to complete and test download
token=$(field '.results[0].token')
for _ in {1..20}; do
  call GET "/api/workspaces/$SLUG/export-issues/?cursor=10:0:0&per_page=10"
  status=$(field '.results[0].status')
  if [[ "$status" == "completed" ]]; then break; fi
  sleep 0.1
done
[[ "$status" == "completed" ]] || { echo "export status not completed: $status" >&2; exit 1; }

# Download export
download_code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" "$BASE/api/workspaces/$SLUG/export-issues/$token/download/")
[[ "$download_code" == 200 ]] || { echo "download export returned $download_code, expected 200" >&2; exit 1; }

# Cross-workspace isolation on download
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" "$BASE/api/workspaces/foreign-slug-$RANDOM/export-issues/$token/download/")
[[ "$code" == 403 || "$code" == 404 ]] || { echo "download export with foreign workspace returned $code, expected 403/404" >&2; exit 1; }

# 4. Export Analytics
# Invalid dimensions return 400
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"x_axis":"invalid","y_axis":"issue_count"}' "$BASE/api/workspaces/$SLUG/export-analytics/")
[[ "$code" == 400 ]] || { echo "export-analytics invalid x_axis returned $code, expected 400" >&2; exit 1; }

code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"x_axis":"priority","y_axis":"issue_count","segment":"priority"}' "$BASE/api/workspaces/$SLUG/export-analytics/")
[[ "$code" == 400 ]] || { echo "export-analytics same x and segment returned $code, expected 400" >&2; exit 1; }

# Valid analytics export (background email)
call POST "/api/workspaces/$SLUG/export-analytics/" '{"x_axis":"priority","y_axis":"issue_count"}'

# Cross-workspace isolation
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"x_axis":"priority","y_axis":"issue_count"}' "$BASE/api/workspaces/foreign-slug-$RANDOM/export-analytics/")
[[ "$code" == 403 || "$code" == 404 ]] || { echo "export-analytics foreign workspace returned $code, expected 403/404" >&2; exit 1; }

# 5. User Activity Export
ME_ID=$(curl -s -b "$jar" "$BASE/api/users/me/" | jq -r .id)

# GET activity CSV export
call GET "/api/workspaces/$SLUG/user-activity/$ME_ID/export/"
[[ "$body" == *"Actor name"* ]] || { echo "activity export csv header missing" >&2; exit 1; }

# POST activity CSV export with date
call POST "/api/workspaces/$SLUG/user-activity/$ME_ID/export/" '{"date":"2026-11-11"}'

# POST without date returns 400
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{}' "$BASE/api/workspaces/$SLUG/user-activity/$ME_ID/export/")
[[ "$code" == 400 ]] || { echo "activity export post without date returned $code, expected 400" >&2; exit 1; }

# Cross-workspace isolation
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" "$BASE/api/workspaces/foreign-slug-$RANDOM/user-activity/$ME_ID/export/")
[[ "$code" == 403 || "$code" == 404 ]] || { echo "activity export foreign workspace returned $code, expected 403/404" >&2; exit 1; }
