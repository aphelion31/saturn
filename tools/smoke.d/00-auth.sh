# Authentication smoke assertions (src/auth.rs)

# Plane #9672: sign-in with no User-Agent header must succeed, not 500
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'User-Agent:' -d "email=$EMAIL&password=$PASSWORD" "$BASE/auth/sign-in/")
[[ "$code" == 302 ]] || { echo "✖ sign-in with empty User-Agent -> $code (expected 302)" >&2; exit 1; }
echo "✔ sign-in with empty User-Agent -> $code"

# Plane #9913: rate limiting triggers 429 on email-check. The key is the TCP peer, so rotating
# X-Forwarded-For (not trusted without TRUST_PROXY=1) must not dodge the limit.
rl_code=""
for i in $(seq 1 12); do
  c=$(curl -s -o /dev/null -w '%{http_code}' -H 'content-type: application/json' -H "X-Forwarded-For: 198.51.100.$i" -d '{"email":"nobody@example.com"}' "$BASE/auth/email-check/")
  if [[ "$c" == "429" ]]; then
    rl_code="429"
    break
  fi
done
if [[ "${SATURN_RATE_LIMIT:-1}" != "0" ]]; then
  [[ "$rl_code" == "429" ]] || { echo "✖ rate limit did not trigger 429 (spoofed X-Forwarded-For bypass?)" >&2; exit 1; }
  echo "✔ email-check rate limit triggered 429 despite rotating X-Forwarded-For"
fi

# Plane #9913: unknown email vs wrong password gives same error code on sign-in
loc_unknown=$(curl -s -o /dev/null -w '%{redirect_url}' -d "email=unknown-$RANDOM@example.com&password=WrongPassword1!" "$BASE/auth/sign-in/")
loc_wrongpw=$(curl -s -o /dev/null -w '%{redirect_url}' -d "email=$EMAIL&password=WrongPassword1!" "$BASE/auth/sign-in/")
err_unknown=$(echo "$loc_unknown" | grep -o 'error_code=[^&]*' || true)
err_wrongpw=$(echo "$loc_wrongpw" | grep -o 'error_code=[^&]*' || true)
[[ "$err_unknown" == "error_code=5065" ]] || { echo "✖ unknown email error code: $err_unknown (expected 5065)" >&2; exit 1; }
[[ "$err_wrongpw" == "error_code=5065" ]] || { echo "✖ wrong password error code: $err_wrongpw (expected 5065)" >&2; exit 1; }
echo "✔ sign-in failure error codes match (5065)"
