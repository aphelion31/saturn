# Work item lifecycle against the real API.
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?per_page=10"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?group_by=state_id&sub_group_by=priority"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/list/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/v2/issues/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Smoke issue"}'
ISSUE=$(field "['id']")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Smoke related issue"}'
RELATED=$(field "['id']")
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/" '{"name":"Smoke issue updated"}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/history/?activity_type=issue-property"
python3 -c 'import json,sys; a=json.loads(sys.argv[1]); assert any(x.get("actor_detail") and x.get("issue_detail") and x.get("project_detail") and x.get("workspace_detail") for x in a)' "$body"
call GET "/api/workspaces/$SLUG/search/?search=Smoke"
python3 -c 'import json,sys; d=json.loads(sys.argv[1])["results"]; assert all(k in d for k in ("issue", "page", "issue_view", "intake")) and d["issue"]' "$body"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/search-issues/?search=Smoke"
python3 -c 'import json,sys; assert json.loads(sys.argv[1])' "$body"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/comments/" '{"comment_html":"<p>Smoke comment</p>"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/issue-links/" '{"url":"https://example.com","title":"Example"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/issue-relation/" "{\"relation_type\":\"relates_to\",\"issues\":[\"$RELATED\"]}"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/issue-relation/"
STATE=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from states where project_id='$PROJECT' and \"group\"='completed' and deleted_at is null limit 1")
[[ -n "$STATE" ]] || { echo 'No completed state for issue smoke test' >&2; exit 1; }
call POST "/api/workspaces/$SLUG/projects/$PROJECT/bulk-operation-issues/" "{\"issue_ids\":[\"$ISSUE\"],\"properties\":{\"state_id\":\"$STATE\",\"priority\":\"high\"}}"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/bulk-archive-issues/" "{\"issue_ids\":[\"$ISSUE\"]}"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/archived-issues/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/archive/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/bulk-delete-issues/" "{\"issue_ids\":[\"$RELATED\"]}"
