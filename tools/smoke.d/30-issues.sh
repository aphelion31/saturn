# Work items (src/issues.rs, list engine in src/workitems.rs)
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?per_page=10"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?group_by=state_id&sub_group_by=priority"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/list/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/v2/issues/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/archived-issues/"
