# Activity feed across issue mutations.
A="/api/workspaces/$SLUG/projects/$PROJECT"
call POST "$A/issues/" '{"name":"Activity smoke"}'
ACT_ISSUE=$(field '.id')
call PATCH "$A/issues/$ACT_ISSUE/" '{"priority":"high"}'
call POST "$A/issue-labels/" '{"name":"Activity label '$RANDOM'","color":"#123456"}'
ACT_LABEL=$(field '.id')
ACT_USER=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select member_id from project_members where project_id='$PROJECT' and role>=15 and is_active and deleted_at is null limit 1")
call PATCH "$A/issues/$ACT_ISSUE/" "{\"label_ids\":[\"$ACT_LABEL\"],\"assignee_ids\":[\"$ACT_USER\"]}"
ACT_STATE=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from states where project_id='$PROJECT' and \"group\"='completed' and deleted_at is null limit 1")
call PATCH "$A/issues/$ACT_ISSUE/" "{\"state_id\":\"$ACT_STATE\"}"
call POST "$A/issues/$ACT_ISSUE/comments/" '{"comment_html":"<p>Activity smoke</p>"}'
ACT_COMMENT=$(field '.id')
call POST "$A/issues/$ACT_ISSUE/reactions/" '{"reaction":"thumbs_up"}'
call DELETE "$A/issues/$ACT_ISSUE/reactions/thumbs_up/"
call POST "$A/comments/$ACT_COMMENT/reactions/" '{"reaction":"heart"}'
call DELETE "$A/comments/$ACT_COMMENT/reactions/heart/"
call POST "$A/issues/" '{"name":"Activity related"}'
ACT_OTHER=$(field '.id')
call POST "$A/issues/$ACT_ISSUE/issue-relation/" "{\"relation_type\":\"blocked_by\",\"issues\":[\"$ACT_OTHER\"]}"
call GET "$A/issues/$ACT_OTHER/history/"
jq -e 'any(.[]; .field == "blocking" and .comment == "added blocking relation")' <<<"$body" >/dev/null
call POST "$A/issues/$ACT_ISSUE/remove-relation/" "{\"relation_type\":\"blocked_by\",\"related_issue\":\"$ACT_OTHER\"}"
call PATCH "$A/issues/$ACT_OTHER/" "{\"parent_id\":\"$ACT_ISSUE\"}"
call GET "$A/issues/$ACT_OTHER/history/"
jq -e 'any(.[]; .field == "parent" and .comment == "updated the parent issue to")' <<<"$body" >/dev/null
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
