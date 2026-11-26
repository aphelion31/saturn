# Plane parity + live HTTP endpoints (Waves 2-4)

# 1. /robots.txt and /ready
robots=$(curl -sf "$BASE/robots.txt")
[[ "$robots" == *"Disallow: /"* ]] || { echo "✖ /robots.txt mismatch: $robots" >&2; exit 1; }
call GET /ready
[[ "$(field '.status')" == "OK" ]] || { echo "✖ /ready status != OK" >&2; exit 1; }

# 2. Live HTTP endpoints (/live/health, /live/convert-document, /live/pdf-export)
call GET /live/health
[[ "$(field '.status')" == "OK" ]] || { echo "✖ /live/health status != OK" >&2; exit 1; }

call POST /live/convert-document '{"description_html":"<p>Hello <strong>live</strong></p>","variant":"document"}'
[[ "$(field '.description_json.type')" == "doc" ]] || { echo "✖ convert-document json invalid" >&2; exit 1; }
[[ -n "$(field '.description_binary')" ]] || { echo "✖ convert-document binary empty" >&2; exit 1; }

code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"description_html":"   ","variant":"document"}' "$BASE/live/convert-document")
[[ "$code" == 400 ]] || { echo "✖ convert-document empty html -> $code (expected 400)" >&2; exit 1; }

code=$(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d '{"pageId":"00000000-0000-0000-0000-000000000001","workspaceSlug":"'"$SLUG"'"}' "$BASE/live/pdf-export")
[[ "$code" == 401 ]] || { echo "✖ anonymous pdf-export -> $code (expected 401)" >&2; exit 1; }

code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' -d '{"pageId":"00000000-0000-0000-0000-000000000001","workspaceSlug":"../evil"}' "$BASE/live/pdf-export")
[[ "$code" == 400 ]] || { echo "✖ invalid workspaceSlug pdf-export -> $code (expected 400)" >&2; exit 1; }

# 3. Page archive descendant propagation, favorite cleanup, live-access & live PDF export
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/" '{"name":"Parent Parity Page","description_html":"<p>Parent PDF content</p>"}'
P_PARENT=$(field '.id')
call GET "/api/workspaces/$SLUG/pages/$P_PARENT/live-access/?action=view"
jq -e --arg pid "$PROJECT" '.project_id==$pid and .read_only==false' <<<"$body" >/dev/null \
  || { echo "✖ live-access view mismatch: $body" >&2; exit 1; }
call GET "/api/workspaces/$SLUG/pages/$P_PARENT/live-access/?action=edit"
jq -e --arg pid "$PROJECT" '.project_id==$pid' <<<"$body" >/dev/null \
  || { echo "✖ live-access edit mismatch: $body" >&2; exit 1; }
call POST "/api/workspaces/$SLUG/projects/$PROJECT/favorite-pages/$P_PARENT/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/" '{"name":"Child Parity Page","parent":"'"$P_PARENT"'"}'
P_CHILD=$(field '.id')

pdf_hdr=$(mktemp)
pdf_body=$(mktemp)
pdf_code=$(curl -s -D "$pdf_hdr" -o "$pdf_body" -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' \
  -d '{"pageId":"'"$P_PARENT"'","workspaceSlug":"'"$SLUG"'","projectId":"'"$PROJECT"'","pageSize":"A4","pageOrientation":"portrait"}' \
  "$BASE/live/pdf-export")
[[ "$pdf_code" == 200 ]] || { echo "✖ pdf-export -> $pdf_code" >&2; rm -f "$pdf_hdr" "$pdf_body"; exit 1; }
grep -qi 'content-type: application/pdf' "$pdf_hdr" || { echo "✖ pdf-export wrong content-type" >&2; rm -f "$pdf_hdr" "$pdf_body"; exit 1; }
[[ "$(head -c 5 "$pdf_body")" == "%PDF-" ]] || { echo "✖ pdf-export missing %PDF- header" >&2; rm -f "$pdf_hdr" "$pdf_body"; exit 1; }
rm -f "$pdf_hdr" "$pdf_body"

call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/$P_PARENT/archive/"
[[ "$(field '.archived_at')" != "null" ]] || { echo "✖ page archive missing archived_at" >&2; exit 1; }
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$P_PARENT/"
[[ "$(field '.is_favorite')" == "false" ]] || { echo "✖ page archive did not clear favorite" >&2; exit 1; }
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$P_CHILD/"
[[ "$(field '.archived_at')" != "null" ]] || { echo "✖ page archive did not propagate to child" >&2; exit 1; }

call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/pages/$P_PARENT/archive/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$P_CHILD/"
[[ "$(field '.archived_at')" == "null" ]] || { echo "✖ page unarchive did not propagate to child" >&2; exit 1; }

call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/$P_PARENT/archive/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/pages/$P_CHILD/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/pages/$P_PARENT/"

# 4. Cycle analytics, burndown, transfer snapshot & draft 400 check
C_BASE="/api/workspaces/$SLUG/projects/$PROJECT/cycles"
call POST "$C_BASE/" '{"name":"Draft cycle no dates"}'
C_DRAFT=$(field '.id')
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" "$BASE$C_BASE/$C_DRAFT/analytics/")
[[ "$code" == 400 ]] || { echo "✖ draft cycle analytics -> $code (expected 400)" >&2; exit 1; }
call DELETE "$C_BASE/$C_DRAFT/"

call POST "$C_BASE/" '{"name":"Analytics C1","start_date":"2035-01-01","end_date":"2035-01-03"}'
C1=$(field '.id')
call POST "$C_BASE/" '{"name":"Analytics C2","start_date":"2035-02-01","end_date":"2035-02-03"}'
C2=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Analytics issue"}'
A_ISSUE=$(field '.id')
call POST "$C_BASE/$C1/cycle-issues/" '{"issues":["'"$A_ISSUE"'"]}'
call GET "$C_BASE/$C1/analytics/"
jq -e '(.labels|type=="array") and (.assignees|type=="array") and (.completion_chart|type=="object") and (.completion_chart|has("2035-01-01"))' <<<"$body" >/dev/null \
  || { echo "✖ cycle analytics shape mismatch: $body" >&2; exit 1; }
call POST "$C_BASE/$C1/transfer-issues/" '{"new_cycle_id":"'"$C2"'"}'
call GET "$C_BASE/$C1/analytics/"
jq -e '.completion_chart|has("2035-01-01")' <<<"$body" >/dev/null \
  || { echo "✖ transferred cycle analytics missing snapshot chart: $body" >&2; exit 1; }
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issues/$A_ISSUE/"
call DELETE "$C_BASE/$C1/"
call DELETE "$C_BASE/$C2/"

# 5. Workspace user-profile project_data & user-activity non-empty feed
ME_ID=$(curl -s -b "$jar" "$BASE/api/users/me/" | jq -r '.id')
call GET "/api/workspaces/$SLUG/user-profile/$ME_ID/"
jq -e '(.project_data|type=="array") and (.project_data|length>0) and (.user_data.display_name!=null)' <<<"$body" >/dev/null \
  || { echo "✖ user-profile missing project_data or display_name: $body" >&2; exit 1; }
call GET "/api/workspaces/$SLUG/user-activity/$ME_ID/"
jq -e '(.results|type=="array") and (.total_results|type=="number") and (.total_results>0)' <<<"$body" >/dev/null \
  || { echo "✖ user-activity missing real activity results: $body" >&2; exit 1; }

# 6. Intake labels/assignees & snoozed_till project timezone day start
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issue-labels/" '{"name":"IntakeLbl-'$RANDOM'","color":"#112233"}'
IN_LBL=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT/intake-issues/" '{"issue":{"name":"Intake parity '$RANDOM'","label_ids":["'"$IN_LBL"'"],"assignee_ids":["'"$ME_ID"'"]}}'
IN_IID=$(field '.issue.id')
jq -e --arg lid "$IN_LBL" --arg uid "$ME_ID" '(.issue.label_ids|index($lid)!=null) and (.issue.assignee_ids|index($uid)!=null)' <<<"$body" >/dev/null \
  || { echo "✖ intake create did not persist label_ids/assignee_ids: $body" >&2; exit 1; }
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/intake-issues/$IN_IID/" '{"status":0,"snoozed_till":"2035-03-01"}'
[[ "$(field '.snoozed_till')" != "null" ]] || { echo "✖ intake snoozed_till missing" >&2; exit 1; }
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X PATCH -H 'content-type: application/json' -d '{"duplicate_to":"00000000-0000-0000-0000-000000000099"}' "$BASE/api/workspaces/$SLUG/projects/$PROJECT/intake-issues/$IN_IID/")
[[ "$code" == 400 ]] || { echo "✖ cross-project duplicate_to -> $code (expected 400)" >&2; exit 1; }
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/intake-issues/$IN_IID/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issue-labels/$IN_LBL/"

# 7. Real OpenXML .xlsx issue export
call POST "/api/workspaces/$SLUG/export-issues/" '{"provider":"xlsx"}'
xlsx_token=""
for _ in {1..30}; do
  call GET "/api/workspaces/$SLUG/export-issues/?cursor=10:0:0&per_page=10"
  xlsx_token=$(jq -r '.results[] | select(.provider=="xlsx" and .status=="completed") | .token' <<<"$body" | head -n1)
  [[ -n "$xlsx_token" ]] && break
  sleep 0.1
done
[[ -n "$xlsx_token" ]] || { echo "✖ xlsx export did not complete" >&2; exit 1; }
xlsx_file=$(mktemp)
xlsx_code=$(curl -s -o "$xlsx_file" -w '%{http_code}' -b "$jar" "$BASE/api/workspaces/$SLUG/export-issues/$xlsx_token/download/")
[[ "$xlsx_code" == 200 ]] || { echo "✖ xlsx download -> $xlsx_code" >&2; rm -f "$xlsx_file"; exit 1; }
[[ "$(head -c 2 "$xlsx_file")" == "PK" ]] || { echo "✖ xlsx download is not PK ZIP OpenXML" >&2; rm -f "$xlsx_file"; exit 1; }
rm -f "$xlsx_file"

# 8. Weak password rejected by zxcvbn
weak_loc=$(curl -s -o /dev/null -w '%{redirect_url}' -d "email=weak-$RANDOM@example.com&password=password123" "$BASE/auth/sign-up/")
[[ "$weak_loc" == *PASSWORD_TOO_WEAK* ]] || { echo "✖ weak password not rejected with PASSWORD_TOO_WEAK: $weak_loc" >&2; exit 1; }

# 9. Plane 1.5.1 parity: OpenAPI schema gating + paths, masked API token PATCH, non-numeric points estimate rejection
if [[ "${ENABLE_DRF_SPECTACULAR:-0}" == "1" ]]; then
  call GET /api/schema/
  jq -e '(.paths | type == "object") and ((.paths | keys | length) > 20)' <<<"$body" >/dev/null \
    || { echo "✖ /api/schema/ missing paths" >&2; exit 1; }
  call GET /api/v1/schema/
  jq -e '(.paths | type == "object") and ((.paths | keys | length) > 10)' <<<"$body" >/dev/null \
    || { echo "✖ /api/v1/schema/ missing paths" >&2; exit 1; }
else
  code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" "$BASE/api/schema/")
  [[ "$code" == 404 ]] || { echo "✖ disabled /api/schema/ -> $code (expected 404)" >&2; exit 1; }
fi

call POST /api/users/api-tokens/ '{"label":"parity-smoke-tok"}'
TOK_ID=$(field '.id')
call PATCH "/api/users/api-tokens/$TOK_ID/" '{"label":"parity-smoke-renamed"}'
jq -e 'has("token") | not' <<<"$body" >/dev/null \
  || { echo "✖ PATCH api-tokens leaked plaintext token: $body" >&2; exit 1; }
call DELETE "/api/users/api-tokens/$TOK_ID/"

code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X POST -H 'content-type: application/json' \
  -d '{"estimate":{"name":"BadPts-'$RANDOM'","type":"points"},"estimate_points":[{"key":0,"value":"abc"}]}' \
  "$BASE/api/workspaces/$SLUG/projects/$PROJECT/estimates/")
[[ "$code" == 400 ]] || { echo "✖ non-numeric points estimate -> $code (expected 400)" >&2; exit 1; }

echo "✔ parity and live smoke checks passed"
