# Work item lifecycle against the real API.
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?per_page=10"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?group_by=state_id&sub_group_by=priority"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/list/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/v2/issues/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Smoke issue"}'
ISSUE=$(field '.id')
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/" '{"start_date":"2030-01-01","target_date":"2030-01-20"}'
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/" '{"target_date":"2030-01-10"}'
[[ "$(field '.target_date')" == '2030-01-10' ]]
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Smoke related issue"}'
RELATED=$(field '.id')
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/" '{"name":"Smoke issue updated"}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/history/?activity_type=issue-property"
jq -e 'any(.[]; .actor_detail and .issue_detail and .project_detail and .workspace_detail)' <<<"$body" >/dev/null
call GET "/api/workspaces/$SLUG/search/?search=Smoke"
jq -e '.results | (has("issue") and has("page") and has("issue_view") and has("intake")) and .issue' <<<"$body" >/dev/null
call GET "/api/workspaces/$SLUG/projects/$PROJECT/search-issues/?search=Smoke"
jq -e . <<<"$body" >/dev/null
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/comments/" '{"comment_html":"<p>Smoke comment</p>"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/issue-links/" '{"url":"https://example.com","title":"Example"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/issue-relation/" "{\"relation_type\":\"relates_to\",\"issues\":[\"$RELATED\"]}"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/issue-relation/"
STATE=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from states where project_id='$PROJECT' and \"group\"='completed' and deleted_at is null limit 1")
[[ -n "$STATE" ]] || { echo 'No completed state for issue smoke test' >&2; exit 1; }
call POST "/api/workspaces/$SLUG/projects/$PROJECT/bulk-operation-issues/" "{\"issue_ids\":[\"$ISSUE\"],\"properties\":{\"state_id\":\"$STATE\",\"priority\":\"high\"}}"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/archive/" '{}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/archived-issues/"
jq -e --arg id "$ISSUE" 'any(.results[]; .id == $id)' <<<"$body" >/dev/null
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/archive/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/bulk-delete-issues/" "{\"issue_ids\":[\"$RELATED\"]}"
for per_page in 0 -1 nope; do
  code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" "$BASE/api/workspaces/$SLUG/projects/$PROJECT/issues/?per_page=$per_page")
  [[ "$code" == 400 ]] || { echo "✖ per_page=$per_page -> $code, expected 400" >&2; exit 1; }
done
