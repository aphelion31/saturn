# Method parity: own tokens, identifier guard, board lifecycle and public invite lookup.
call GET "/api/users/me/workspaces/$SLUG/projects/invitations/"
call PATCH /api/instances/ '{"instance_name":"Saturn smoke"}'
[[ "$(field '.instance_name')" == 'Saturn smoke' ]]
call PUT "/api/workspaces/$SLUG/" '{"name":"Acme"}'
call PUT "/api/workspaces/$SLUG/projects/$PROJECT/" '{}'
METHOD_MEMBER=$(psql "$DATABASE_URL" -Atc "SELECT id FROM project_members WHERE project_id='$PROJECT' AND is_active AND deleted_at IS NULL LIMIT 1")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/members/$METHOD_MEMBER/"
[[ "$(field '.id')" == "$METHOD_MEMBER" ]]
call POST /api/users/api-tokens/ '{"label":"method-smoke"}'
METHOD_TOKEN=$(field '.id')
call PATCH "/api/users/api-tokens/$METHOD_TOKEN/" '{"description":"updated"}'
[[ "$(field '.description')" == updated ]]
call DELETE "/api/users/api-tokens/$METHOD_TOKEN/"
call DELETE "/api/workspaces/$SLUG/project-identifiers/" '{"name":"UNUSED-SMOKE-IDENTIFIER"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/project-deploy-boards/" '{}'
METHOD_BOARD=$(field '.id')
call GET "/api/workspaces/$SLUG/projects/$PROJECT/project-deploy-boards/$METHOD_BOARD/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/project-deploy-boards/$METHOD_BOARD/"
