# App replay

Build: `/tmp/run-saturn.sh`

Passed: 0/7

Fallback rate: not reported by this build

## Failing cases

### instances-get

Arguments: `GET /api/instances/`

header allow:

```diff
- GET, PATCH, HEAD, OPTIONS
(missing)
```

header cache-control:

```diff
- private, max-age=12
(missing)
```

header cross-origin-opener-policy:

```diff
- same-origin
(missing)
```

header referrer-policy:

```diff
- same-origin
+ origin-when-cross-origin
```

header vary:

```diff
- Accept-Encoding, Cookie, origin
+ origin, access-control-request-method, access-control-request-headers
```

header x-frame-options:

```diff
- DENY
+ SAMEORIGIN
```

### v1-cycles-list

Arguments: `GET /api/v1/workspaces/acme/projects/0db57630-1a10-40a7-b8c7-64363ca33468/cycles/`

header allow:

```diff
- GET, POST
(missing)
```

header cross-origin-opener-policy:

```diff
- same-origin
(missing)
```

header referrer-policy:

```diff
- same-origin
+ origin-when-cross-origin
```

header vary:

```diff
- Accept-Encoding, Cookie, origin
+ origin, access-control-request-method, access-control-request-headers
```

header x-frame-options:

```diff
- DENY
+ SAMEORIGIN
```

header x-ratelimit-remaining:

```diff
- 59
(missing)
```

header x-ratelimit-reset:

```diff
- [..]
(missing)
```

### v1-labels-list

Arguments: `GET /api/v1/workspaces/acme/projects/0db57630-1a10-40a7-b8c7-64363ca33468/labels/`

header allow:

```diff
- GET, POST
(missing)
```

header cross-origin-opener-policy:

```diff
- same-origin
(missing)
```

header referrer-policy:

```diff
- same-origin
+ origin-when-cross-origin
```

header vary:

```diff
- Accept-Encoding, Cookie, origin
+ origin, access-control-request-method, access-control-request-headers
```

header x-frame-options:

```diff
- DENY
+ SAMEORIGIN
```

header x-ratelimit-remaining:

```diff
- 58
(missing)
```

header x-ratelimit-reset:

```diff
- [..]
(missing)
```

### v1-modules-list

Arguments: `GET /api/v1/workspaces/acme/projects/0db57630-1a10-40a7-b8c7-64363ca33468/modules/`

header allow:

```diff
- GET, POST
(missing)
```

header cross-origin-opener-policy:

```diff
- same-origin
(missing)
```

header referrer-policy:

```diff
- same-origin
+ origin-when-cross-origin
```

header vary:

```diff
- Accept-Encoding, Cookie, origin
+ origin, access-control-request-method, access-control-request-headers
```

header x-frame-options:

```diff
- DENY
+ SAMEORIGIN
```

header x-ratelimit-remaining:

```diff
- 57
(missing)
```

header x-ratelimit-reset:

```diff
- [..]
(missing)
```

### v1-projects-list

Arguments: `GET /api/v1/workspaces/acme/projects/`

header allow:

```diff
- GET, POST
(missing)
```

header cross-origin-opener-policy:

```diff
- same-origin
(missing)
```

header referrer-policy:

```diff
- same-origin
+ origin-when-cross-origin
```

header vary:

```diff
- Accept-Encoding, Cookie, origin
+ origin, access-control-request-method, access-control-request-headers
```

header x-frame-options:

```diff
- DENY
+ SAMEORIGIN
```

header x-ratelimit-remaining:

```diff
- 56
(missing)
```

header x-ratelimit-reset:

```diff
- [..]
(missing)
```

body:

```diff
  ⋮
        "network": 2,
        "page_view": true,
        "project_lead": null,
-       "sort_order": 65535.0,
+       "sort_order": 65535,
        "timezone": "UTC",
        "total_cycles": 0,
        "total_members": 1,
  ⋮
```

### v1-states-list

Arguments: `GET /api/v1/workspaces/acme/projects/0db57630-1a10-40a7-b8c7-64363ca33468/states/`

header allow:

```diff
- GET, POST
(missing)
```

header cross-origin-opener-policy:

```diff
- same-origin
(missing)
```

header referrer-policy:

```diff
- same-origin
+ origin-when-cross-origin
```

header vary:

```diff
- Accept-Encoding, Cookie, origin
+ origin, access-control-request-method, access-control-request-headers
```

header x-frame-options:

```diff
- DENY
+ SAMEORIGIN
```

header x-ratelimit-remaining:

```diff
- 55
(missing)
```

header x-ratelimit-reset:

```diff
- [..]
(missing)
```

body:

```diff
  ⋮
        "is_triage": false,
        "name": "Backlog",
        "project": "[..]",
-       "sequence": 15000.0,
+       "sequence": 15000,
        "slug": "backlog",
        "updated_at": "[..]",
        "updated_by": "[..]",
  ⋮
        "is_triage": false,
        "name": "Todo",
        "project": "[..]",
-       "sequence": 25000.0,
+       "sequence": 25000,
        "slug": "todo",
        "updated_at": "[..]",
        "updated_by": "[..]",
  ⋮
        "is_triage": false,
        "name": "In Progress",
        "project": "[..]",
-       "sequence": 35000.0,
+       "sequence": 35000,
        "slug": "in-progress",
        "updated_at": "[..]",
        "updated_by": "[..]",
  ⋮
        "is_triage": false,
        "name": "Done",
        "project": "[..]",
-       "sequence": 45000.0,
+       "sequence": 45000,
        "slug": "done",
        "updated_at": "[..]",
        "updated_by": "[..]",
  ⋮
        "is_triage": false,
        "name": "Cancelled",
        "project": "[..]",
-       "sequence": 55000.0,
+       "sequence": 55000,
        "slug": "cancelled",
        "updated_at": "[..]",
        "updated_by": "[..]",
  ⋮
```

### v1-work-items-list

Arguments: `GET /api/v1/workspaces/acme/projects/0db57630-1a10-40a7-b8c7-64363ca33468/work-items/`

header allow:

```diff
- GET, POST
(missing)
```

header cross-origin-opener-policy:

```diff
- same-origin
(missing)
```

header referrer-policy:

```diff
- same-origin
+ origin-when-cross-origin
```

header vary:

```diff
- Accept-Encoding, Cookie, origin
+ origin, access-control-request-method, access-control-request-headers
```

header x-frame-options:

```diff
- DENY
+ SAMEORIGIN
```

header x-ratelimit-remaining:

```diff
- 54
(missing)
```

header x-ratelimit-reset:

```diff
- [..]
(missing)
```

body:

```diff
  ⋮
        "priority": "medium",
        "project": "[..]",
        "sequence_id": 1,
-       "sort_order": 65535.0,
+       "sort_order": 65535,
        "start_date": null,
        "state": "[..]",
        "target_date": null,
  ⋮
```
