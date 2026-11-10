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
