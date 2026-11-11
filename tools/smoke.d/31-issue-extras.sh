# Issue extras HTTP coverage.
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Issue extras smoke"}'
EXTRA_ISSUE=$(field "['id']")
EXTRA_BASE="/api/workspaces/$SLUG/projects/$PROJECT"
call GET "$EXTRA_BASE/issues/$EXTRA_ISSUE/meta/"
call GET "$EXTRA_BASE/issues/$EXTRA_ISSUE/issue-attachments/"
call GET "$EXTRA_BASE/issues/$EXTRA_ISSUE/versions/"
call GET "$EXTRA_BASE/work-items/$EXTRA_ISSUE/description-versions/"
call GET "$EXTRA_BASE/intake-work-items/$EXTRA_ISSUE/description-versions/"
call GET "$EXTRA_BASE/deleted-issues/"
call GET "$EXTRA_BASE/issues-detail/"
call POST "$EXTRA_BASE/issue-dates/" "{\"updates\":[{\"id\":\"$EXTRA_ISSUE\",\"start_date\":\"2026-01-01\",\"target_date\":\"2026-01-02\"}]}"
