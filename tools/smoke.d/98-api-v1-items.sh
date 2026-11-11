# Public API v1 work items (X-Api-Key, src/api_v1_items.rs). Mints a token with the session cookie first.
TOKEN=$(curl -s -b "$jar" -H 'content-type: application/json' -d '{"label":"smoke-v1-items"}' "$BASE/api/users/api-tokens/" | jq -r .token)
[[ -n "$TOKEN" && "$TOKEN" != null ]] || { echo "✖ api token create failed" >&2; exit 1; }
v1() { # v1 METHOD PATH [JSON]: same as call, with X-Api-Key instead of the cookie jar
  local out code
  out=$(curl -s -H "X-Api-Key: $TOKEN" -X "$1" -H 'content-type: application/json' ${3:+-d "$3"} -w '\n%{http_code}' "$BASE$2")
  code=${out##*$'\n'}
  body=${out%$'\n'*}
  if [[ "$code" != 2* ]]; then echo "✖ v1 $1 $2 -> $code $body" >&2; exit 1; fi
  echo "✔ v1 $1 $2 -> $code"
}
V1="/api/v1/workspaces/$SLUG/projects/$PROJECT/work-items"
v1 POST "$V1/" '{"name":"V1 smoke item","priority":"high","description_html":"<p>hi</p>"}'
ITEM=$(field '.id'); IDENT=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select p.identifier||'-'||i.sequence_id from issues i join projects p on p.id=i.project_id where i.id='$ITEM'")
# 9011: accept completed_at on create
v1 POST "$V1/" '{"name":"V1 completed item","completed_at":"2026-01-02T00:00:00Z"}'
COMP_ITEM=$(field '.id')
jq -e '.completed_at | startswith("2026-01-02")' <<<"$body" >/dev/null
# 9011: accept completed_at on patch
v1 PATCH "$V1/$COMP_ITEM/" '{"completed_at":"2026-01-03T12:00:00Z"}'
jq -e '.completed_at | startswith("2026-01-03")' <<<"$body" >/dev/null
v1 DELETE "$V1/$COMP_ITEM/"
v1 GET "$V1/?per_page=5"
jq -e '.results and .next_cursor and (has("total_results"))' <<<"$body" >/dev/null
# 9765: work-items list honors group_by and returns grouped_by
v1 GET "$V1/?group_by=state"
jq -e '.grouped_by == "state" and (.results | type == "object")' <<<"$body" >/dev/null
v1 GET "$V1/?group_by=priority"
jq -e '.grouped_by == "priority" and (.results | type == "object")' <<<"$body" >/dev/null
# 9429: non-positive or non-numeric per_page rejected with 400
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $TOKEN" "$BASE$V1/?per_page=0")
[[ "$c" == "400" ]] || { echo "✖ per_page=0 -> $c" >&2; exit 1; }
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $TOKEN" "$BASE$V1/?per_page=-5")
[[ "$c" == "400" ]] || { echo "✖ per_page=-5 -> $c" >&2; exit 1; }
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $TOKEN" "$BASE$V1/?per_page=abc")
[[ "$c" == "400" ]] || { echo "✖ per_page=abc -> $c" >&2; exit 1; }
v1 GET "$V1/$ITEM/"
jq -e '.assignees and .labels and (has("description_json") | not)' <<<"$body" >/dev/null
v1 PATCH "$V1/$ITEM/" '{"name":"V1 smoke renamed"}'
# 9947: start_date > target_date rejected with 400
c=$(curl -s -o /dev/null -w '%{http_code}' -X PATCH -H "X-Api-Key: $TOKEN" -H 'content-type: application/json' -d '{"start_date":"2026-05-10","target_date":"2026-05-01"}' "$BASE$V1/$ITEM/")
[[ "$c" == "400" ]] || { echo "✖ start>target -> $c" >&2; exit 1; }
# 9115: PATCH archived_at on non-completed/non-cancelled work item rejected with 400
c=$(curl -s -o /dev/null -w '%{http_code}' -X PATCH -H "X-Api-Key: $TOKEN" -H 'content-type: application/json' -d '{"archived_at":"2026-01-01"}' "$BASE$V1/$ITEM/")
[[ "$c" == "400" ]] || { echo "✖ archive open item -> $c" >&2; exit 1; }
v1 POST "$V1/$ITEM/comments/" '{"comment_html":"<p>smoke</p>"}'
CMT=$(field '.id')
v1 GET "$V1/$ITEM/comments/"
v1 PATCH "$V1/$ITEM/comments/$CMT/" '{"comment_html":"<p>smoke edited</p>"}'
v1 DELETE "$V1/$ITEM/comments/$CMT/"
v1 POST "$V1/$ITEM/links/" '{"url":"https://example.com","title":"Example"}'
LNK=$(field '.id')
v1 GET "$V1/$ITEM/links/"
v1 DELETE "$V1/$ITEM/links/$LNK/"
v1 GET "$V1/$ITEM/activities/"
v1 GET "/api/v1/workspaces/$SLUG/work-items/$IDENT/"
v1 GET "/api/v1/workspaces/$SLUG/work-items/search/?search=V1%20smoke"
jq -e '.issues | length >= 1' <<<"$body" >/dev/null
# relations (Plane IssueRelationListCreateAPIEndpoint) and attachments list/delete
v1 POST "$V1/" '{"name":"V1 relation peer"}'
PEER=$(field '.id')
v1 POST "$V1/$ITEM/relations/" '{"relation_type":"blocking","issues":["'"$PEER"'"]}'
v1 GET "$V1/$ITEM/relations/"
jq -e '.blocking | length == 1' <<<"$body" >/dev/null
v1 GET "$V1/$PEER/relations/"
jq -e '.blocked_by | length == 1' <<<"$body" >/dev/null
# 9585: remove relation endpoint -> 204
c=$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "X-Api-Key: $TOKEN" -H 'content-type: application/json' -d "{\"related_issue\":\"$PEER\"}" "$BASE$V1/$ITEM/relations/remove/")
[[ "$c" == "204" ]] || { echo "✖ relations/remove -> $c" >&2; exit 1; }
v1 GET "$V1/$ITEM/relations/"
jq -e '.blocking | length == 0' <<<"$body" >/dev/null
# 9585: removing non-existent relation -> 404
c=$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "X-Api-Key: $TOKEN" -H 'content-type: application/json' -d "{\"related_issue\":\"$PEER\"}" "$BASE$V1/$ITEM/relations/remove/")
[[ "$c" == "404" ]] || { echo "✖ non-existent relations/remove -> $c" >&2; exit 1; }
v1 GET "$V1/$ITEM/attachments/"
jq -e 'type == "array"' <<<"$body" >/dev/null
v1 DELETE "$V1/$PEER/"
v1 DELETE "$V1/$ITEM/"
