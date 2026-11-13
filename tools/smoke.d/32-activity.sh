# Activity feed across issue mutations.
A="/api/workspaces/$SLUG/projects/$PROJECT"
call POST "$A/issues/" '{"name":"Activity smoke"}'
ACT_ISSUE=$(field '.id')
call PATCH "$A/issues/$ACT_ISSUE/" '{"priority":"high"}'
call POST "$A/issue-labels/" '{"name":"Activity label","color":"#123456"}'
ACT_LABEL=$(field '.id')
ACT_USER=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select member_id from project_members where project_id='$PROJECT' and role>=15 and is_active and deleted_at is null limit 1")
call PATCH "$A/issues/$ACT_ISSUE/" "{\"label_ids\":[\"$ACT_LABEL\"],\"assignee_ids\":[\"$ACT_USER\"]}"
ACT_STATE=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from states where project_id='$PROJECT' and \"group\"='completed' and deleted_at is null limit 1")
call PATCH "$A/issues/$ACT_ISSUE/" "{\"state_id\":\"$ACT_STATE\"}"
call POST "$A/issues/$ACT_ISSUE/comments/" '{"comment_html":"<p>Activity smoke</p>"}'
call POST "$A/issues/$ACT_ISSUE/issue-links/" '{"url":"https://example.com/activity"}'
call POST "$A/cycles/" '{"name":"Activity cycle","start_date":"2030-01-01","end_date":"2030-01-10"}'
ACT_CYCLE=$(field '.id')
call POST "$A/cycles/$ACT_CYCLE/cycle-issues/" "{\"issues\":[\"$ACT_ISSUE\"]}"
call POST "$A/modules/" '{"name":"Activity module","status":"planned"}'
ACT_MODULE=$(field '.id')
call POST "$A/modules/$ACT_MODULE/issues/" "{\"issues\":[\"$ACT_ISSUE\"]}"
call POST "$A/issues/$ACT_ISSUE/archive/" '{}'
call DELETE "$A/issues/$ACT_ISSUE/archive/"
call GET "$A/issues/$ACT_ISSUE/history/"
jq -e '[.[] | select(.verb != null) | .field // "created"] | (["created","priority","assignees","labels","state","link","cycles","modules","archived_at","archived_at"] as $wanted | . as $actual | reduce $actual[] as $v (0; if $v == $wanted[.] then .+1 else . end) == ($wanted|length))' <<<"$body" >/dev/null
