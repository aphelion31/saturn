// Run against Saturn with GITEA_HOST=http://127.0.0.1:3315 and OAuth enabled.
import http from 'node:http';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';

const base = process.env.BASE || 'http://127.0.0.1:3314';
const email = `oauth-${Date.now()}@example.com`;
const providerId = Date.now();
const server = http.createServer((req, res) => {
  res.setHeader('Content-Type', 'application/json');
  if (req.url === '/login/oauth/access_token' && req.method === 'POST') res.end(JSON.stringify({ access_token: 'test-access-token' }));
  else if (req.url === '/api/v1/user') res.end(JSON.stringify({ id: providerId, full_name: 'Test User' }));
  else if (req.url === '/api/v1/user/emails') res.end(JSON.stringify([{ email, primary: true, verified: true }]));
  else { res.statusCode = 404; res.end('{}'); }
});
await new Promise(resolve => server.listen(3315, '127.0.0.1', resolve));
try {
  const start = await fetch(`${base}/auth/gitea/?next_path=%2Fwelcome`, { redirect: 'manual' });
  assert.equal(start.status, 302);
  const state = new URL(start.headers.get('location')).searchParams.get('state');
  assert.ok(state);
  const cookie = start.headers.get('set-cookie').split(';')[0];
  const result = await fetch(`${base}/auth/gitea/callback/?state=${state}&code=mock`, { redirect: 'manual', headers: { cookie } });
  assert.equal(result.status, 302);
  assert.equal(result.headers.get('location'), `${base}/welcome`);
  assert.match(result.headers.get('set-cookie'), /^session-id=/);
  const count = execFileSync('psql', [process.env.DATABASE_URL || 'postgres://localhost/saturn_w52', '-At', '-v', 'ON_ERROR_STOP=1', '-c', `SELECT count(*) FROM accounts a JOIN users u ON u.id=a.user_id WHERE u.email='${email}' AND a.provider='gitea' AND a.access_token <> 'test-access-token'`], { encoding: 'utf8' }).trim();
  assert.equal(count, '1');
  console.log('OAuth mock callback, login session, encrypted account: ok');
} finally { server.close(); }
