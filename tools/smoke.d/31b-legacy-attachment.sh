# Legacy multipart issue attachment upload (src/assets.rs legacy_attachment_upload): Plane IssueAttachmentEndpoint.post.
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Legacy attachment target"}'
LEGACY_ISSUE=$(field '.id')
out=$(curl -s -b "$jar" -F 'attributes={"name":"note.txt","type":"text/plain","size":5}' -F 'asset=@-;filename=note.txt;type=text/plain' -w '\n%{http_code}' "$BASE/api/workspaces/$SLUG/projects/$PROJECT/issues/$LEGACY_ISSUE/issue-attachments/" <<<"hello")
[[ "${out##*$'\n'}" == 201 ]] || { echo "✖ legacy attachment upload -> $out" >&2; exit 1; }
echo "✔ POST legacy issue-attachments -> 201"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$LEGACY_ISSUE/issue-attachments/"
[[ $(jq length <<<"$body") == 1 ]] || { echo "✖ legacy attachment not listed" >&2; exit 1; }
