# Magic credentials and password reset against the isolated smoke database.
psql "$DATABASE_URL" -q -c "UPDATE instance_configurations SET value='1' WHERE key='ENABLE_MAGIC_LINK_LOGIN'" >/dev/null
magic_email="magic-$RANDOM@example.com"
call POST /auth/magic-generate/ "{\"email\":\"$magic_email\"}"
magic_key=$(field '.key')
magic_code=$(psql "$DATABASE_URL" -Atc "SELECT token FROM saturn_magic_codes WHERE key = 'magic_$magic_email'")
loc=$(curl -s -o /dev/null -w '%{redirect_url}' -d "email=$magic_email&code=000000" "$BASE/auth/magic-sign-up/")
[[ "$loc" == *error_code=* ]] || { echo 'wrong code accepted' >&2; exit 1; }
loc=$(curl -s -o /dev/null -w '%{redirect_url}' -d "email=$magic_email&code=$magic_code" "$BASE/auth/magic-sign-up/")
[[ "$loc" != *error_code=* ]] || { echo "magic signup failed: $loc" >&2; exit 1; }
call POST /auth/magic-generate/ "{\"email\":\"$magic_email\"}"
magic_code=$(psql "$DATABASE_URL" -Atc "SELECT token FROM saturn_magic_codes WHERE key = 'magic_$magic_email'")
loc=$(curl -s -o /dev/null -w '%{redirect_url}' -d "email=$magic_email&code=$magic_code" "$BASE/auth/magic-sign-in/")
[[ "$loc" != *error_code=* ]] || { echo "magic signin failed: $loc" >&2; exit 1; }
call POST /auth/forgot-password/ "{\"email\":\"$magic_email\"}"
known=$body
call POST /auth/forgot-password/ '{"email":"absent@example.com"}'
[[ "$known" == "$body" ]] || { echo 'forgot password responses differ' >&2; exit 1; }
uid=$(psql "$DATABASE_URL" -Atc "SELECT id FROM users WHERE email = '$magic_email'")
reset_token=$(psql "$DATABASE_URL" -Atc "SELECT token FROM saturn_magic_codes WHERE key = 'reset:$uid'")
uidb64=$(printf %s "$uid" | base64 | tr '+/' '-_' | tr -d '=\n')
loc=$(curl -s -o /dev/null -w '%{redirect_url}' -d 'password=Saturn-rocks-2027&confirm_password=Saturn-rocks-2027' "$BASE/auth/reset-password/$uidb64/$reset_token/")
[[ "$loc" == *success=True* ]] || { echo "reset failed: $loc" >&2; exit 1; }
status=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -H 'content-type: application/json' -d '{"old_password":"wrong","new_password":"Saturn-rocks-2027"}' "$BASE/auth/change-password/")
[[ "$status" == 400 ]] || { echo "wrong password returned $status" >&2; exit 1; }
echo '✔ magic authentication and password reset'
