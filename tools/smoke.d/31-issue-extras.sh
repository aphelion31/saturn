# Issue extras HTTP coverage.
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Issue extras smoke"}'
EXTRA_ISSUE=$(field '.id')
EXTRA_BASE="/api/workspaces/$SLUG/projects/$PROJECT"
call GET "$EXTRA_BASE/issues/$EXTRA_ISSUE/meta/"
call GET "$EXTRA_BASE/issues/$EXTRA_ISSUE/issue-attachments/"
call GET "$EXTRA_BASE/issues/$EXTRA_ISSUE/versions/"
call GET "$EXTRA_BASE/work-items/$EXTRA_ISSUE/description-versions/"
call GET "$EXTRA_BASE/intake-work-items/$EXTRA_ISSUE/description-versions/"
call GET "$EXTRA_BASE/deleted-issues/"
call GET "$EXTRA_BASE/issues-detail/"
call POST "$EXTRA_BASE/issue-dates/" "{\"updates\":[{\"id\":\"$EXTRA_ISSUE\",\"start_date\":\"2026-01-01\",\"target_date\":\"2026-01-02\"}]}"
call POST "$EXTRA_BASE/issue-labels/" "{\"name\":\"Draft label $RANDOM\",\"color\":\"#123456\"}"
DLABEL=$(field '.id')
call POST "$EXTRA_BASE/cycles/" "{\"name\":\"Draft cycle $RANDOM\",\"start_date\":\"2031-01-01\",\"end_date\":\"2031-01-10\"}"
DCYCLE=$(field '.id')
call POST "$EXTRA_BASE/modules/" "{\"name\":\"Draft module $RANDOM\",\"status\":\"planned\"}"
DMODULE=$(field '.id')
call GET /api/users/me/
DUSER=$(field '.id')
call POST "/api/workspaces/$SLUG/draft-issues/" "{\"project_id\":\"$PROJECT\",\"name\":\"Smoke draft\",\"description_html\":\"<p>Draft</p><script>bad()</script>\",\"assignee_ids\":[\"$DUSER\"],\"label_ids\":[\"$DLABEL\"],\"cycle_id\":\"$DCYCLE\",\"module_ids\":[\"$DMODULE\"]}"
DRAFT=$(field '.id')
call GET "/api/workspaces/$SLUG/draft-issues/"
jq -e --arg id "$DRAFT" 'any(.results[]; .id == $id)' <<<"$body" >/dev/null
call GET "/api/workspaces/$SLUG/draft-issues/$DRAFT/"
jq -e '.description_html | contains("<script") | not' <<<"$body" >/dev/null
call PATCH "/api/workspaces/$SLUG/draft-issues/$DRAFT/" '{"name":"Converted draft"}'
call POST "/api/workspaces/$SLUG/draft-to-issue/$DRAFT/" '{}'
jq -e --arg user "$DUSER" --arg label "$DLABEL" --arg cycle "$DCYCLE" --arg module "$DMODULE" '.name == "Converted draft" and .assignee_ids == [$user] and .label_ids == [$label] and .cycle_id == $cycle and .module_ids == [$module]' <<<"$body" >/dev/null
call POST "/api/workspaces/$SLUG/draft-issues/" '{}'
DRAFT=$(field '.id')
call DELETE "/api/workspaces/$SLUG/draft-issues/$DRAFT/"
