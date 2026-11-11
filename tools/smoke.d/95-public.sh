# Public boards (src/public.rs): anonymous reads through a deploy board anchor, sanitized comments.
DB=${DATABASE_URL:-postgres://localhost/saturn}
psql "$DB" -X -q -v project="$PROJECT" <<< "UPDATE deploy_boards SET deleted_at=now() WHERE entity_name='project' AND entity_identifier=:'project'::uuid AND deleted_at IS NULL"
INTAKE_ID=$(psql "$DB" -X -q -Atc "select id from intakes where project_id='$PROJECT' and deleted_at is null limit 1")
if [ -z "$INTAKE_ID" ]; then
  WS_ID=$(psql "$DB" -X -q -Atc "select workspace_id from projects where id='$PROJECT'")
  INTAKE_ID=$(psql "$DB" -X -q -Atc "INSERT INTO intakes (name, project_id, workspace_id) VALUES ('Smoke Intake', '$PROJECT', '$WS_ID') RETURNING id")
fi
ANCHOR=$(psql "$DB" -X -q -v ON_ERROR_STOP=1 -At -v project="$PROJECT" -v intake="$INTAKE_ID" <<< "INSERT INTO deploy_boards (anchor, entity_name, entity_identifier, is_comments_enabled, is_reactions_enabled, is_votes_enabled, is_activity_enabled, is_disabled, view_props, project_id, workspace_id, intake_id) SELECT replace(gen_random_uuid()::text,'-',''), 'project', id, true, true, true, true, false, '{}'::jsonb, id, workspace_id, :'intake'::uuid FROM projects WHERE id=:'project'::uuid RETURNING anchor")
[[ -n "$ANCHOR" ]]
for path in settings meta states labels cycles modules project-members; do
  code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/public/anchor/$ANCHOR/$path/")
  [[ "$code" == 200 ]] || { echo "✖ anonymous GET $path -> $code" >&2; exit 1; }
done
echo "✔ anonymous board reads -> 200"
code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/public/anchor/does-not-exist/settings/")
[[ "$code" == 404 ]] || { echo "✖ unknown anchor -> $code, expected 404" >&2; exit 1; }
echo "✔ unknown anchor -> 404"
call GET "/api/public/anchor/$ANCHOR/issues/"
PUB_ISSUE=$(psql "$DB" -Atc "select id from issues where project_id='$PROJECT' and deleted_at is null and archived_at is null and not is_draft limit 1")
if [[ -n "$PUB_ISSUE" ]]; then
  call POST "/api/public/anchor/$ANCHOR/issues/$PUB_ISSUE/comments/" '{"comment_html":"<p>hi</p><script>alert(1)</script><img src=x onerror=alert(1)>"}'
  html=$(field '.comment_html')
  [[ "$html" != *script* && "$html" != *onerror* ]] || { echo "✖ comment not sanitized: $html" >&2; exit 1; }
  echo "✔ public comment sanitized"
  PUB_COMMENT_ID=$(field '.id')
fi

# Plane #9498/#9441: public comment endpoints only accept comments on issues of that board's project
FOREIGN_ISSUE=$(psql "$DB" -Atc "select id from issues where project_id != '$PROJECT' and deleted_at is null limit 1")
if [[ -z "$FOREIGN_ISSUE" ]]; then
  FOREIGN_ISSUE="00000000-0000-0000-0000-000000000000"
fi
code=$(curl -s -b "$jar" -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d '{"comment_html":"<p>leak</p>"}' "$BASE/api/public/anchor/$ANCHOR/issues/$FOREIGN_ISSUE/comments/")
[[ "$code" == 404 ]] || { echo "✖ comment on foreign issue -> $code (expected 404)" >&2; exit 1; }
echo "✔ Plane #9498/#9441: public comment on foreign project issue rejected -> 404"

# Plane #9506: public board voting respects is_votes_enabled
if [[ -n "${PUB_ISSUE:-}" ]]; then
  psql "$DB" -X -q -c "UPDATE deploy_boards SET is_votes_enabled=false WHERE anchor='$ANCHOR'"
  code=$(curl -s -b "$jar" -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d '{"vote":1}' "$BASE/api/public/anchor/$ANCHOR/issues/$PUB_ISSUE/votes/")
  [[ "$code" == 400 ]] || { echo "✖ vote on votes-disabled board -> $code (expected 400)" >&2; exit 1; }
  psql "$DB" -X -q -c "UPDATE deploy_boards SET is_votes_enabled=true WHERE anchor='$ANCHOR'"
  call POST "/api/public/anchor/$ANCHOR/issues/$PUB_ISSUE/votes/" '{"vote":1}'
  call GET "/api/public/anchor/$ANCHOR/issues/$PUB_ISSUE/votes/"
  call DELETE "/api/public/anchor/$ANCHOR/issues/$PUB_ISSUE/votes/"
  echo "✔ Plane #9506: public board voting respects is_votes_enabled"
fi

# Published project boards
call GET "/api/public/workspaces/$SLUG/project-boards/"
jq -e --arg pid "$PROJECT" 'any(.[]; .id == $pid)' <<<"$body" >/dev/null
echo "✔ workspace published project boards listed"

# Public assets: upload slot, upload, mark uploaded, download, bulk bind, delete, restore
call POST "/api/public/assets/v2/anchor/$ANCHOR/" '{"name":"comment_pub.png","type":"image/png","size":100,"entity_type":"COMMENT_DESCRIPTION"}'
PUB_ASSET_ID=$(field '.asset_id')
PUB_TOKEN=$(field '.upload_data.fields.token')
code=$(curl -s -o /dev/null -w '%{http_code}' -F "token=$PUB_TOKEN" -F "file=@-;filename=comment_pub.png" "$BASE/api/assets/v2/upload/$PUB_ASSET_ID/" <<< "public image data")
[[ "$code" == 204 ]] || { echo "✖ public upload -> $code" >&2; exit 1; }
call PATCH "/api/public/assets/v2/anchor/$ANCHOR/$PUB_ASSET_ID/" '{}'
got=$(curl -s "$BASE/api/public/assets/v2/anchor/$ANCHOR/$PUB_ASSET_ID/")
[[ "$got" == "public image data" ]] || { echo "✖ public download returned: $got" >&2; exit 1; }
echo "✔ public asset download roundtrip -> 200"

# Plane #10099: public Space asset writes scoped to the uploader
other_jar=$(mktemp)
curl -s -c "$other_jar" -d "email=other@saturn.dev&password=Saturn-rocks-2026" "$BASE/auth/sign-in/" >/dev/null
code=$(curl -s -b "$other_jar" -o /dev/null -w '%{http_code}' -X PATCH "$BASE/api/public/assets/v2/anchor/$ANCHOR/$PUB_ASSET_ID/")
[[ "$code" == 404 ]] || { echo "✖ other user PATCH public asset -> $code (expected 404)" >&2; exit 1; }
code=$(curl -s -b "$other_jar" -o /dev/null -w '%{http_code}' -X DELETE "$BASE/api/public/assets/v2/anchor/$ANCHOR/$PUB_ASSET_ID/")
[[ "$code" == 404 ]] || { echo "✖ other user DELETE public asset -> $code (expected 404)" >&2; exit 1; }
code=$(curl -s -b "$other_jar" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/public/assets/v2/anchor/$ANCHOR/restore/$PUB_ASSET_ID/")
[[ "$code" == 404 ]] || { echo "✖ other user restore public asset -> $code (expected 404)" >&2; exit 1; }
rm -f "$other_jar"
echo "✔ Plane #10099: public Space asset writes scoped to uploader"

if [[ -n "${PUB_COMMENT_ID:-}" ]]; then
  call POST "/api/public/assets/v2/anchor/$ANCHOR/$PUB_COMMENT_ID/bulk/" "{\"asset_ids\":[\"$PUB_ASSET_ID\"]}"
  echo "✔ public asset bulk bind -> 204"
fi

call DELETE "/api/public/assets/v2/anchor/$ANCHOR/$PUB_ASSET_ID/"
call POST "/api/public/assets/v2/anchor/$ANCHOR/restore/$PUB_ASSET_ID/"
call DELETE "/api/public/assets/v2/anchor/$ANCHOR/$PUB_ASSET_ID/"
echo "✔ public asset delete & restore -> ok"

# Public intake issues: create, retrieve, patch, delete
call POST "/api/public/anchor/$ANCHOR/intakes/$INTAKE_ID/intake-issues/" '{"issue":{"name":"Public intake report","priority":"medium"}}'
PUB_INTAKE_PK=$(field '.bridge_id')
call GET "/api/public/anchor/$ANCHOR/intakes/$INTAKE_ID/intake-issues/$PUB_INTAKE_PK/"
[[ "$(field '.bridge_id')" == "$PUB_INTAKE_PK" ]]
call PATCH "/api/public/anchor/$ANCHOR/intakes/$INTAKE_ID/intake-issues/$PUB_INTAKE_PK/" '{"issue":{"name":"Updated intake report"}}'
[[ "$(field '.name')" == "Updated intake report" ]]
call DELETE "/api/public/anchor/$ANCHOR/intakes/$INTAKE_ID/intake-issues/$PUB_INTAKE_PK/"
echo "✔ public intake-issue CRUD -> ok"

psql "$DB" -X -q -c "UPDATE deploy_boards SET deleted_at=now() WHERE anchor='$ANCHOR'"
