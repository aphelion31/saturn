# Public boards (src/public.rs): anonymous reads through a deploy board anchor, sanitized comments.
DB=${DATABASE_URL:-postgres://localhost/saturn}
ANCHOR=$(psql "$DB" -X -q -v ON_ERROR_STOP=1 -At -v project="$PROJECT" <<< "INSERT INTO deploy_boards (anchor, entity_name, entity_identifier, is_comments_enabled, is_reactions_enabled, is_votes_enabled, is_activity_enabled, is_disabled, view_props, project_id, workspace_id) SELECT replace(gen_random_uuid()::text,'-',''), 'project', id, true, true, true, true, false, '{}'::jsonb, id, workspace_id FROM projects WHERE id=:'project'::uuid RETURNING anchor")
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
  html=$(field "['comment_html']")
  [[ "$html" != *script* && "$html" != *onerror* ]] || { echo "✖ comment not sanitized: $html" >&2; exit 1; }
  echo "✔ public comment sanitized"
fi
psql "$DB" -X -q -c "UPDATE deploy_boards SET deleted_at=now() WHERE anchor='$ANCHOR'"
