# Workspace-level module and cycle lists
call GET "/api/workspaces/$SLUG/workspace-members/me/"
jq -e 'has("member") and has("workspace") and has("created_by") and has("updated_by") and (has("member_id")|not) and (has("workspace_id")|not) and (has("created_by_id")|not) and (has("updated_by_id")|not)' <<<"$body" >/dev/null
call GET "/api/workspaces/$SLUG/projects/"
jq -e 'all(.[]; keys == (["id","name","identifier","sort_order","logo_props","member_role","intake_count","archived_at","workspace","cycle_view","issue_views_view","module_view","page_view","inbox_view","guest_view_all_features","project_lead","network","created_at","updated_at","created_by","updated_by"]|sort))' <<<"$body" >/dev/null
out=$(curl -s -b "$jar" -w '\n%{http_code}' "$BASE/api/workspaces/nonexistent-workspace/workspace-members/me/")
[[ "${out##*$'\n'}" == 403 ]]
jq -e '. == {"detail":"You do not have permission to perform this action."}' <<<"${out%$'\n'*}" >/dev/null
out=$(curl -s -b "$jar" -w '\n%{http_code}' "$BASE/api/release-notes/")
[[ "${out##*$'\n'}" == 404 ]]
jq -e '. == {"error":"Page not found."}' <<<"${out%$'\n'*}" >/dev/null
call GET "/api/workspaces/$SLUG/modules/"
call GET "/api/workspaces/$SLUG/cycles/"
