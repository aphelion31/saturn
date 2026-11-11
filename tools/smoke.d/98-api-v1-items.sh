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
v1 GET "$V1/?per_page=5"
jq -e '.results and .next_cursor and (has("total_results"))' <<<"$body" >/dev/null
v1 GET "$V1/$ITEM/"
jq -e '.assignees and .labels and (has("description_json") | not)' <<<"$body" >/dev/null
v1 PATCH "$V1/$ITEM/" '{"name":"V1 smoke renamed"}'
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
v1 DELETE "$V1/$ITEM/"
