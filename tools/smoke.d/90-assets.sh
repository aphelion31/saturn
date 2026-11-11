# File uploads on local disk (src/assets.rs): presign, multipart upload, mark uploaded, download.
call POST "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/" '{"name":"smoke.txt","type":"text/plain","size":12,"entity_type":"ISSUE_DESCRIPTION"}'
ASSET=$(field '.asset_id')
TOKEN=$(field '.upload_data.fields.token')
code=$(curl -s -o /dev/null -w '%{http_code}' -F "token=$TOKEN" -F "file=@-;filename=smoke.txt" "$BASE/api/assets/v2/upload/$ASSET/" <<< "hello saturn")
[[ "$code" == 204 ]] || { echo "✖ upload -> $code" >&2; exit 1; }
echo "✔ POST /api/assets/v2/upload/{id}/ -> $code"
code=$(curl -s -o /dev/null -w '%{http_code}' -F "token=bad" -F "file=@-;filename=x.txt" "$BASE/api/assets/v2/upload/$ASSET/" <<< "x")
[[ "$code" == 400 || "$code" == 403 ]] || { echo "✖ bad token upload -> $code" >&2; exit 1; }
echo "✔ bad token rejected -> $code"
call PATCH "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/$ASSET/" '{}'
got=$(curl -s -b "$jar" "$BASE/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/$ASSET/")
[[ "$got" == "hello saturn" ]] || { echo "✖ download returned: $got" >&2; exit 1; }
echo "✔ download round trip"
call DELETE "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/$ASSET/"

# Plane #10128 & #9551: empty MIME (.txt) and .md attachment files accepted
WS_ID=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from workspaces where slug='$SLUG'")
ISSUE_ID=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from issues where workspace_id='$WS_ID' limit 1")
if [ -n "$ISSUE_ID" ]; then
  # Empty MIME type
  call POST "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE_ID/attachments/" '{"name":"notes.txt","type":"","size":5}'
  TXT_ASSET=$(field '.asset_id')
  TXT_TOKEN=$(field '.upload_data.fields.token')
  curl -s -o /dev/null -F "token=$TXT_TOKEN" -F "file=@-;filename=notes.txt" "$BASE/api/assets/v2/upload/$TXT_ASSET/" <<< "notes"
  call PATCH "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE_ID/attachments/$TXT_ASSET/" '{}'
  hdr=$(curl -s -I -b "$jar" "$BASE/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE_ID/attachments/$TXT_ASSET/")
  [[ "$hdr" =~ "attachment" ]] || { echo "✖ expected attachment Content-Disposition" >&2; exit 1; }
  [[ "$hdr" =~ "nosniff" ]] || { echo "✖ expected nosniff header" >&2; exit 1; }
  echo "✔ empty MIME attachment served as attachment + nosniff"
  call DELETE "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE_ID/attachments/$TXT_ASSET/"

  # .md file
  call POST "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE_ID/attachments/" '{"name":"README.md","type":"text/markdown","size":6}'
  MD_ASSET=$(field '.asset_id')
  MD_TOKEN=$(field '.upload_data.fields.token')
  curl -s -o /dev/null -F "token=$MD_TOKEN" -F "file=@-;filename=README.md" "$BASE/api/assets/v2/upload/$MD_ASSET/" <<< "# test"
  call PATCH "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE_ID/attachments/$MD_ASSET/" '{}'
  call DELETE "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE_ID/attachments/$MD_ASSET/"
  echo "✔ .md attachment accepted and deleted"

  # Plane #9564: description image assets must not get asset_url with project None and must set project_id
  call POST "/api/assets/v2/workspaces/$SLUG/" "{\"entity_type\":\"ISSUE_DESCRIPTION\",\"entity_identifier\":\"$ISSUE_ID\",\"name\":\"desc.png\",\"type\":\"image/png\",\"size\":10}"
  DESC_ASSET=$(field '.asset_id')
  DESC_URL=$(field '.asset_url')
  [[ "$DESC_URL" != *"None"* ]] || { echo "✖ description asset_url contains None: $DESC_URL" >&2; exit 1; }
  db_pid=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select project_id from file_assets where id='$DESC_ASSET'")
  [[ -n "$db_pid" && "$db_pid" != "" ]] || { echo "✖ description asset project_id is empty" >&2; exit 1; }
  echo "✔ description asset has valid project_id ($db_pid) and no None in asset_url"
  call DELETE "/api/assets/v2/workspaces/$SLUG/$DESC_ASSET/"
fi
