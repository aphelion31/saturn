# --- users accounts/email ---
# Second user via magic sign-up. The admin email stays unchanged: email change runs on the second user,
# whose session ends on success (Plane logs the user out), so the admin session in $jar keeps working.
ua_jar=$(mktemp)
ua_email="ua-$RANDOM@example.com"
curl -s -o /dev/null -H 'content-type: application/json' -d "$(jq -nc --arg e "$ua_email" '{email:$e}')" "$BASE/auth/magic-generate/"
ua_code=$(psql "$DATABASE_URL" -Atqc "SELECT token FROM saturn_magic_codes WHERE key = 'magic_$ua_email'")
loc=$(curl -s -o /dev/null -w '%{redirect_url}' -c "$ua_jar" -d "email=$ua_email&code=$ua_code" "$BASE/auth/magic-sign-up/")
[[ "$loc" != *error_code=* ]] || { echo "✖ second user sign-up failed: $loc" >&2; exit 1; }
echo "✔ second user sign-up"

# ua_api JAR METHOD PATH [JSON-BODY] -> prints "<http_code> <body>". Bodies are built with jq, never by hand.
ua_api() {
  local out
  if [[ -n "${4:-}" ]]; then
    out=$(curl -s -b "$1" -X "$2" -H 'content-type: application/json' --data-binary "$4" -w '\n%{http_code}' "$BASE$3")
  else
    out=$(curl -s -b "$1" -X "$2" -w '\n%{http_code}' "$BASE$3")
  fi
  echo "${out##*$'\n'} ${out%$'\n'*}"
}
ua_expect() { # ua_expect WANT_CODE "<code> <body>" LABEL
  [[ "${2%% *}" == "$1" ]] || { echo "✖ $3 -> $2" >&2; exit 1; }
  echo "✔ $3 -> $1"
}
admin_id=$(psql "$DATABASE_URL" -Atqc "SELECT id FROM users WHERE email = 'admin@saturn.dev'")
ua_id=$(psql "$DATABASE_URL" -Atqc "SELECT id FROM users WHERE email = '$ua_email'")

# Session and login accounts. Tokens are never returned.
ua_expect 200 "$(ua_api "$ua_jar" GET /api/users/session/)" "session when signed in"
ua_expect 401 "$(ua_api "$(mktemp)" GET /api/users/me/accounts/)" "accounts anonymous rejected"
acc_a=$(psql "$DATABASE_URL" -Atqc "INSERT INTO accounts (id, created_at, updated_at, provider_account_id, provider, access_token, refresh_token, last_connected_at, metadata, user_id, id_token)
  VALUES (gen_random_uuid(), now(), now(), 'pa-$RANDOM', 'github', 'secret-access-a', 'secret-refresh-a', now(), '{}', '$ua_id', 'secret-id-a') RETURNING id")
acc_b=$(psql "$DATABASE_URL" -Atqc "INSERT INTO accounts (id, created_at, updated_at, provider_account_id, provider, access_token, refresh_token, last_connected_at, metadata, user_id, id_token)
  VALUES (gen_random_uuid(), now(), now(), 'pb-$RANDOM', 'google', 'secret-access-b', 'secret-refresh-b', now(), '{}', '$ua_id', 'secret-id-b') RETURNING id")
listed=$(ua_api "$ua_jar" GET /api/users/me/accounts/)
ua_expect 200 "$listed" "accounts list"
[[ "$listed" != *secret-* ]] || { echo "✖ account tokens leaked" >&2; exit 1; }
echo "✔ account tokens not exposed"
ua_expect 200 "$(ua_api "$ua_jar" GET /api/users/me/accounts/$acc_a/)" "account retrieve own"
ua_expect 404 "$(ua_api "$jar" GET /api/users/me/accounts/$acc_a/)" "account retrieve other user 404"
ua_expect 404 "$(ua_api "$jar" DELETE /api/users/me/accounts/$acc_b/)" "delete other user's account 404"
# Two linked accounts: removing one is fine, removing the last is refused.
ua_expect 204 "$(ua_api "$ua_jar" DELETE /api/users/me/accounts/$acc_b/)" "delete one of two login methods"
ua_expect 400 "$(ua_api "$ua_jar" DELETE /api/users/me/accounts/$acc_a/)" "last login method delete refused"
echo "✔ account DELETE guard"

# Wrong-user code and taken address.
target_a="ua-target-$RANDOM@example.com"
body_target=$(jq -nc --arg e "$target_a" '{email:$e}')
ua_expect 200 "$(ua_api "$jar" POST /api/users/me/email/generate-code/ "$body_target")" "admin requests code for target"
admin_code=$(psql "$DATABASE_URL" -Atqc "SELECT token FROM saturn_magic_codes WHERE key = 'email_change:$admin_id:$target_a'")
[[ -n "$admin_code" ]] || { echo "✖ admin code not stored" >&2; exit 1; }
body_stolen=$(jq -nc --arg e "$target_a" --arg c "$admin_code" '{email:$e,code:$c}')
ua_expect 400 "$(ua_api "$ua_jar" PATCH /api/users/me/email/ "$body_stolen")" "other user cannot use admin's code"
body_taken=$(jq -nc --arg e "$ua_email" '{email:$e,code:"000000"}')
ua_expect 400 "$(ua_api "$jar" PATCH /api/users/me/email/ "$body_taken")" "taken email rejected"
body_wrong=$(jq -nc --arg e "$target_a" '{email:$e,code:"000000"}')
ua_expect 400 "$(ua_api "$jar" PATCH /api/users/me/email/ "$body_wrong")" "wrong code rejected"

# Success path on the second user: code consumed, email changed, session ended, reuse fails.
new_email="ua-new-$RANDOM@example.com"
body_new=$(jq -nc --arg e "$new_email" '{email:$e}')
ua_expect 200 "$(ua_api "$ua_jar" POST /api/users/me/email/generate-code/ "$body_new")" "second user requests code"
new_code=$(psql "$DATABASE_URL" -Atqc "SELECT token FROM saturn_magic_codes WHERE key = 'email_change:$ua_id:$new_email'")
[[ -n "$new_code" ]] || { echo "✖ new code not stored" >&2; exit 1; }
body_ok=$(jq -nc --arg e "$new_email" --arg c "$new_code" '{email:$e,code:$c}')
ua_expect 200 "$(ua_api "$ua_jar" PATCH /api/users/me/email/ "$body_ok")" "email change with correct code"
[[ "$(psql "$DATABASE_URL" -Atqc "SELECT email FROM users WHERE id = '$ua_id'")" == "$new_email" ]] || { echo "✖ email not stored" >&2; exit 1; }
[[ -z "$(psql "$DATABASE_URL" -Atqc "SELECT 1 FROM saturn_magic_codes WHERE key = 'email_change:$ua_id:$new_email'")" ]] || { echo "✖ code not consumed" >&2; exit 1; }
echo "✔ email stored, code consumed"
ended=$(ua_api "$ua_jar" GET /api/users/session/)
[[ "$ended" == '200 {"is_authenticated":false}' ]] || { echo "✖ session not ended: $ended" >&2; exit 1; }
echo "✔ session ended after email change"

rm -f "$ua_jar"
[[ "$(psql "$DATABASE_URL" -Atqc "SELECT count(*) FROM users WHERE email = 'admin@saturn.dev'")" == 1 ]] || { echo "✖ admin email changed" >&2; exit 1; }
echo '✔ users accounts/email'
