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
call POST "$EXTRA_BASE/issue-labels/" "{\"name\":\"Draft label $RANDOM\",\"color\":\"#123456\"}"
DLABEL=$(field "['id']")
call POST "$EXTRA_BASE/cycles/" "{\"name\":\"Draft cycle $RANDOM\",\"start_date\":\"2031-01-01\",\"end_date\":\"2031-01-10\"}"
DCYCLE=$(field "['id']")
call POST "$EXTRA_BASE/modules/" "{\"name\":\"Draft module $RANDOM\",\"status\":\"planned\"}"
DMODULE=$(field "['id']")
call GET /api/users/me/
DUSER=$(field "['id']")
call POST "/api/workspaces/$SLUG/draft-issues/" "{\"project_id\":\"$PROJECT\",\"name\":\"Smoke draft\",\"description_html\":\"<p>Draft</p><script>bad()</script>\",\"assignee_ids\":[\"$DUSER\"],\"label_ids\":[\"$DLABEL\"],\"cycle_id\":\"$DCYCLE\",\"module_ids\":[\"$DMODULE\"]}"
DRAFT=$(field "['id']")
call GET "/api/workspaces/$SLUG/draft-issues/"
python3 -c 'import json,sys; assert any(x["id"]==sys.argv[2] for x in json.loads(sys.argv[1])["results"])' "$body" "$DRAFT"
call GET "/api/workspaces/$SLUG/draft-issues/$DRAFT/"
python3 -c 'import json,sys; assert "<script" not in json.loads(sys.argv[1])["description_html"]' "$body"
call PATCH "/api/workspaces/$SLUG/draft-issues/$DRAFT/" '{"name":"Converted draft"}'
call POST "/api/workspaces/$SLUG/draft-to-issue/$DRAFT/" '{}'
python3 -c 'import json,sys; x=json.loads(sys.argv[1]); assert x["name"] == "Converted draft" and x["assignee_ids"]==[sys.argv[2]] and x["label_ids"]==[sys.argv[3]] and x["cycle_id"]==sys.argv[4] and x["module_ids"]==[sys.argv[5]]' "$body" "$DUSER" "$DLABEL" "$DCYCLE" "$DMODULE"
call POST "/api/workspaces/$SLUG/draft-issues/" '{}'
DRAFT=$(field "['id']")
call DELETE "/api/workspaces/$SLUG/draft-issues/$DRAFT/"
