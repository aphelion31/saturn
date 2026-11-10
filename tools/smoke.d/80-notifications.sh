# Notifications (src/notifications.rs)
call GET "/api/workspaces/$SLUG/users/notifications/"
call GET "/api/workspaces/$SLUG/users/notifications/unread/"
call POST "/api/workspaces/$SLUG/users/notifications/mark-all-read/" '{"snoozed":false,"archived":false}'

USER_ID=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from users where email='$EMAIL'")
WS_ID=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from workspaces where slug='$SLUG'")
ISSUE_ID=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from issues where workspace_id='$WS_ID' limit 1")
if [ -n "$ISSUE_ID" ]; then
  NOTIF_ID=$(python3 -c 'import uuid; print(uuid.uuid4())')
  psql "${DATABASE_URL:-postgres://localhost/saturn}" -qc "
    INSERT INTO notifications (id, created_at, updated_at, workspace_id, receiver_id, entity_name, entity_identifier, title, message_html, sender)
    VALUES ('$NOTIF_ID', now(), now(), '$WS_ID', '$USER_ID', 'issue', '$ISSUE_ID', 'Test notification', '<p>Test</p>', 'assigned_by');
  "
  call GET "/api/workspaces/$SLUG/users/notifications/$NOTIF_ID/"
  call PATCH "/api/workspaces/$SLUG/users/notifications/$NOTIF_ID/read/"
  call DELETE "/api/workspaces/$SLUG/users/notifications/$NOTIF_ID/read/"
  call POST "/api/workspaces/$SLUG/users/notifications/$NOTIF_ID/archive/"
  call DELETE "/api/workspaces/$SLUG/users/notifications/$NOTIF_ID/archive/"
  call DELETE "/api/workspaces/$SLUG/users/notifications/$NOTIF_ID/"
fi

# User Notification Preferences
call GET "/api/users/me/notification-preferences/"
call PATCH "/api/users/me/notification-preferences/" '{"mention":false}'

# Stickies
call POST "/api/workspaces/$SLUG/stickies/" '{"name":"Smoke Sticky","description_html":"<p>Sticky note</p>"}'
STICKY_ID=$(field "['id']")
call GET "/api/workspaces/$SLUG/stickies/"
call GET "/api/workspaces/$SLUG/stickies/$STICKY_ID/"
call PATCH "/api/workspaces/$SLUG/stickies/$STICKY_ID/" '{"name":"Updated Sticky"}'
call DELETE "/api/workspaces/$SLUG/stickies/$STICKY_ID/"

# Quick Links
call POST "/api/workspaces/$SLUG/quick-links/" '{"title":"Test Link","url":"https://example.com"}'
LINK_ID=$(field "['id']")
call GET "/api/workspaces/$SLUG/quick-links/"
call GET "/api/workspaces/$SLUG/quick-links/$LINK_ID/"
call PATCH "/api/workspaces/$SLUG/quick-links/$LINK_ID/" '{"title":"Updated Link"}'
call DELETE "/api/workspaces/$SLUG/quick-links/$LINK_ID/"

# Recent Visits
psql "${DATABASE_URL:-postgres://localhost/saturn}" -qc "
  INSERT INTO user_recent_visits (id, created_at, updated_at, workspace_id, user_id, entity_name, entity_identifier, visited_at)
  VALUES (gen_random_uuid(), now(), now(), '$WS_ID', '$USER_ID', 'project', '$PROJECT', now());
"
call GET "/api/workspaces/$SLUG/recent-visits/"
call GET "/api/workspaces/$SLUG/recent-visits/?entity_name=project"

# Timezones
call GET "/api/timezones/"
