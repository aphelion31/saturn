# Sidebar and home preferences (src/workspaces.rs)
call GET "/api/workspaces/$SLUG/sidebar-preferences/"
call PATCH "/api/workspaces/$SLUG/sidebar-preferences/" '[{"key":"views","is_pinned":true}]'
call PATCH "/api/workspaces/$SLUG/sidebar-preferences/analytics/" '{"sort_order":1}'
call GET "/api/workspaces/$SLUG/home-preferences/"
call PATCH "/api/workspaces/$SLUG/home-preferences/recents/" '{"is_enabled":false}'
