# Cardbe Plugin: Jira Tickets

Import tickets from self-hosted Jira Server/Data Center using PAT and custom JQL. Jira Cloud OAuth is also supported. Built on [Cardbe Plugin API](https://github.com/ldslds449/cardbe-plugin-api) 1.1.0, using the typed `plugin` world.

## Configuration

1. Choose a target board. Missing Jira status columns are created automatically during sync. Existing names are matched case-sensitively, ignoring leading/trailing whitespace; duplicate matching names fail the entire sync.
2. Install the ZIP, create an instance, and select a board.
3. Choose Self-hosted Jira (the default) or Jira Cloud, then configure API base URL, Jira site URL, JQL, and the `token` secret.
4. In the permissions section, add the API base URL's hostname without scheme or port (for example, `jira.example.com` for `https://jira.example.com:8443/jira`) and select it to approve access. For Cloud, approve the default `api.atlassian.com` host.

The manifest enables `allow_custom_domains` and declares `http_credentials` for the `base_url` setting. Cardbe attaches the `token` Bearer credential only to `/rest/api/2` and `/rest/api/3` and their descendants beneath that Base URL, including approved custom hosts. Connection approval remains separate; approving a host does not send credentials to other paths. Changing the Base URL requires renewed form consent; changing its host requires fresh connection approval.

| API | API base URL | Jira site URL | Token |
| --- | --- | --- | --- |
| Self-hosted Jira REST v2 (default) | `https://jira.example.com` (include any `/jira` context path and optional port, e.g. `:8443`) | Same as API base URL | Personal access token (PAT) |
| Jira Cloud REST v3 | `https://api.atlassian.com/ex/jira/<cloudId>` | `https://your-team.atlassian.net` | OAuth access token with Jira read permissions |

For self-hosted Jira, select `data-center`, enter your Jira URL in both URL fields, and save your PAT as `token`. Existing `data-center` configurations remain supported. Personal access tokens require Jira Server/Data Center 8.14 or later.

### Self-hosted URLs and custom ports

- Enter the Jira root used in the browser, including the context path and any custom HTTPS port: `https://jira.example.com:8443/jira`. Do not include `/rest/api/...` (rejected with `CONFIG_BASE_URL_API_PATH`) or a `/browse/...` link.
- Ports must be 1–65535 without leading zeros. Jira must serve HTTPS on that port: the default Tomcat connector on `http://…:8080` cannot be used, so put Jira behind an HTTPS connector or reverse proxy.
- Approve only the hostname (`jira.example.com`); connection grants are per hostname, and the token is attached for the host, port and path of the API base URL.
- Both URLs are normalized before use: the hostname is lowercased, `:443` is dropped, and trailing slashes are removed. Card identities (`jira:<site URL>:<issue ID>`) therefore stay the same for these spellings, while a different port or context path is a different site and creates new cards.
- The certificate must be valid for the hostname and chain to a public CA, and Jira must answer without redirects; SSO/login redirects fail at the host.

Cloud email + API token authentication uses Basic auth and **cannot be used directly**, because the current Cardbe host only attaches Bearer tokens. The plugin does not access secrets or put credentials in URLs. Users must update OAuth tokens in Cardbe; this plugin does not refresh tokens automatically.

Example JQL: `project = APP AND fixVersion = "1.2.0" ORDER BY updated ASC`.

## Import behavior

- Title: `APP-123 ticket summary`.
- Column: reuse a column matching the status name, or create one when missing. Column creation and ticket upserts commit atomically; subsequent syncs move tickets when their status changes. At most 200 columns can be created per run.
- Tags: version names from `fixVersions`, deduplicated with blank names skipped.
- Description: configurable `description_template`, defaulting to `{url}\n\n{description}`. Supports `{url}`, `{description}`, `{key}`, and `{summary}`; unknown placeholders remain literal, and an empty template produces an empty description. Inserted Jira text is never interpreted as a template. Changing the template updates descriptions still managed by the source; manual card edits remain preserved. The original Jira description supports self-hosted Jira strings and Cloud ADF text, paragraphs, and text hyperlinks. ADF images, attachments, and layout are not imported.
- Jira site + issue ID forms a stable identifier, so renaming a ticket key does not create a new card.
- Each sync queries the full JQL result with POST search requests (JQL in the JSON body, so long JQL is not limited by URL length) in pages of 50, up to 100 pages and 2,000 cards. Failures do not return partial updates.
- Self-hosted JQL without `ORDER BY` gets `ORDER BY key ASC` appended, so offset pagination does not skip tickets updated during a sync.
- Fields beyond Cardbe limits are truncated at a UTF-8 boundary: titles to 4,096 bytes, descriptions to 262,144 bytes, version labels to 256 bytes and 100 labels (`FIELDS_TRUNCATED`). Status names over 64 bytes fail with `STATUS_NAME_TOO_LONG` because they become column names. A missing or `null` `fixVersions` field (hidden in some projects) imports no labels.
- Tickets that no longer match the JQL or have been deleted remain on the board: the API supports upserts only, without deletion. Cardbe also preserves manual overrides, so manually moved cards may stop following Jira statuses.
- Cardbe manages scheduling. Use one instance per source on each board.

## Logs

Execution emits bounded diagnostic codes and numeric counts: `SYNC_STARTED`, `BOARD_COLUMNS_READ`, `HTTP_REQUESTS`, `HTTP_RESPONSES`, `HTTP_RETRIES`, `FIELDS_TRUNCATED`, `HTTP_LAST_STATUS`, `TICKETS_SYNCED`, `COLUMNS_CREATED`, and `SYNC_COMPLETED`. Request counters and the last received status are also emitted when synchronization fails, together with `SYNC_FAILED` and the specific error code. JSON decoding failures emit `RESPONSE_JSON_ERROR_LINE` and `RESPONSE_JSON_ERROR_COLUMN`. At most a small fixed number of logs is emitted, even across 100 pages.

With API 1.1.0 the plugin also writes short text messages (`log-message`): a success summary, a warning per network retry, the unexpected HTTP status, the JSON error position, the host's safe failure code and a hint for each error code. Messages contain only counts, statuses, positions and fixed text. No URLs, JQL, pagination tokens, response bodies, ticket text, or credentials are logged, and serde error text (which can quote response content) is never forwarded.

Cardbe hosts implementing API 1.1.0 keep these codes and messages for failed and cancelled runs too, so the last progress code shows where a run stopped. Older hosts discard logs on failure.

Requests use `http-send`. When the host reports `NETWORK_FAILED` (DNS, connection, TLS, or the 15-second per-request timeout), the plugin retries the same request, at most 2 times per run; retries count toward the host's 100-request limit. Other transport and policy failures still fail the run in the host's safe error namespace.

| Error code | Meaning / what to check |
| --- | --- |
| `CONFIG_JSON_INVALID` | Configuration is malformed or required fields have the wrong type / are missing. |
| `CONFIG_BASE_URL_INVALID`, `CONFIG_SITE_URL_INVALID` | The indicated URL must be HTTPS with a valid hostname, optional port (1–65535) and context path, without credentials, query parameters or duplicate slashes. The site URL must be the site root (not `/browse`) and at most 480 bytes. |
| `CONFIG_BASE_URL_API_PATH` | The API base URL contains `/rest` or `/rest/api/...`; enter only the Jira root. |
| `CONFIG_JQL_EMPTY`, `CONFIG_API_INVALID` | Supply JQL and select `data-center` or `cloud`. |
| `INVALID_JQL` | Jira returned HTTP 400; check JQL and search parameters. |
| `AUTHENTICATION_FAILED`, `PERMISSION_DENIED` | HTTP 401 / 403; check token, credential consent and Jira access rights. |
| `API_ENDPOINT_NOT_FOUND`, `API_ENDPOINT_REMOVED` | HTTP 404 / 410; check Base URL, context path and API selection. |
| `PROXY_AUTHENTICATION_REQUIRED` | HTTP 407; the proxy requires authentication. |
| `JIRA_INTERNAL_SERVER_ERROR`, `JIRA_BAD_GATEWAY`, `JIRA_SERVICE_UNAVAILABLE`, `JIRA_GATEWAY_TIMEOUT` | HTTP 500 / 502 / 503 / 504; check Jira or the reverse proxy. Retry hint: 60 seconds. |
| `RATE_LIMITED` | HTTP 429; retry hint uses Retry-After, bounded to 60–86,400 seconds. |
| `API_FAILED` | Other HTTP status (the status is in the run's text log). Retry hint: 60 seconds. |
| `NETWORK_FAILED` | Network failure persisted after retries; check DNS, connectivity, TLS and Jira response time. Retry hint: 60 seconds. |
| `HTTP_FAILED` | Host policy or limit failure; inspect the host execution error. HTTP redirects (3xx, e.g. SSO login pages) also fail at the host. |
| `RESPONSE_JSON_INVALID`, `RESPONSE_JSON_TRUNCATED`, `RESPONSE_SCHEMA_INVALID` | Invalid/incomplete JSON or unexpected Jira response fields; check API selection and proxy. |
| `ISSUE_REQUIRED_FIELD_EMPTY` | An issue has an empty ID, key, summary, status or update timestamp. |
| `PAGE_TOKEN_MISSING`, `PAGE_TOKEN_REPEATED`, `PAGE_TOTAL_MISSING`, `PAGE_OFFSET_MISMATCH`, `PAGE_EMPTY_BEFORE_TOTAL` | Jira returned inconsistent pagination metadata. No partial updates are returned. |
| `TICKET_LIMIT_EXCEEDED`, `COLUMN_LIMIT_EXCEEDED`, `PAGE_LIMIT_EXCEEDED` | More than 2,000 unique tickets, 200 new columns or 100 pages; narrow the JQL. |
| `STATUS_COLUMN_DUPLICATED`, `STATUS_COLUMN_ID_INVALID` | Multiple matching columns or a negative column ID; fix the board columns. |
| `STATUS_NAME_TOO_LONG` | A Jira status name exceeds 64 UTF-8 bytes (about 21 CJK characters); shorten it in Jira. |
| `BOARD_INVALID_ARGUMENT`, `BOARD_PERMISSION_DENIED`, `BOARD_NOT_FOUND`, `BOARD_RESOURCE_LIMIT`, `BOARD_INTERNAL_ERROR` | The board query failed with the indicated host reason. |
| `BOARD_SNAPSHOT_MISMATCH` | The board snapshot version differs from the execution context. Retry with a fresh snapshot. |

Counts describe plugin output; Cardbe validates and commits output after instance board authorization.

### Error code changes in 1.1.0

Version 1.1.0 replaces several generic error codes with the specific codes above. Update any automation that matches on the old codes:

| Before 1.1.0 | 1.1.0 and later |
| --- | --- |
| `INVALID_CONFIG` | `CONFIG_JSON_INVALID`, `CONFIG_BASE_URL_INVALID`, `CONFIG_SITE_URL_INVALID`, `CONFIG_JQL_EMPTY`, `CONFIG_API_INVALID` |
| `INVALID_RESPONSE` | `RESPONSE_JSON_INVALID`, `RESPONSE_JSON_TRUNCATED`, `RESPONSE_SCHEMA_INVALID`, `ISSUE_REQUIRED_FIELD_EMPTY`, `PAGE_TOKEN_MISSING`, `PAGE_TOKEN_REPEATED`, `PAGE_TOTAL_MISSING`, `PAGE_OFFSET_MISMATCH`, `PAGE_EMPTY_BEFORE_TOTAL` |
| `CHANGESET_TOO_LARGE` | `TICKET_LIMIT_EXCEEDED`, `COLUMN_LIMIT_EXCEEDED` |
| `STATUS_COLUMN_NOT_FOUND` | `STATUS_COLUMN_DUPLICATED`, `STATUS_COLUMN_ID_INVALID` |
| `BOARD_QUERY_FAILED` | `BOARD_INVALID_ARGUMENT`, `BOARD_PERMISSION_DENIED`, `BOARD_NOT_FOUND`, `BOARD_RESOURCE_LIMIT`, `BOARD_INTERNAL_ERROR`, `BOARD_SNAPSHOT_MISMATCH` |
| `API_FAILED` (HTTP 404 / 407 / 410 / 500 / 502 / 503 / 504) | `API_ENDPOINT_NOT_FOUND`, `PROXY_AUTHENTICATION_REQUIRED`, `API_ENDPOINT_REMOVED`, `JIRA_*` codes |
| `HTTP_FAILED` (network failures) | `NETWORK_FAILED`, after automatic retries |

The old codes remain declared in `error_codes` for compatibility but are no longer returned.

Plugin 1.1.0 targets Cardbe Plugin API 1.1.0 (`api_version` `1.1.0`) and requires a Cardbe host built against that API; it does not run on API 1.0.0 hosts. Base URLs may now include a port, for example `https://jira.example.com:8443/jira`. Site URLs are now normalized (lowercase hostname, no `:443`); if an existing instance's site URL used uppercase letters or an explicit `:443`, its cards get new identities and are imported again once.

## Build and validation

Requires Rust, PowerShell, and wasm-tools 1.259.0:

```powershell
git submodule update --init
rustup target add wasm32-unknown-unknown
cargo test --locked
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
./scripts/build.ps1
# Optional: set the package's default API host; credentials remain scoped to base_url
./scripts/build.ps1 -Domain jira.example.com
```

Outputs `dist/manifest.json`, `dist/component.wasm`, and ZIP/SHA-256 files in the repository root. Verify the checksum before installing the ZIP. The Cardbe host allows only public HTTPS (port 443 by default, or an explicit nonzero port) and rejects private IP addresses and redirects. Jira instances accessible only on a private network require additional host support.

## API provenance

`cardbe-plugin-api/wit/plugin.wit` is provided by a Git submodule pinned to API tag `v1.1.0` (commit `023907c8294dfa5ca62e6078f13436b38d01eca0`), package `cardbe:plugin@1.1.0`. Its Git blob SHA-256 is `2243d8404bc212b5bd1335d743b78498296b2497d25eb2b832035259d2859163`. Install on a Cardbe host supporting API 1.1.0; API 1.0.0 draft hosts are incompatible.

Self-hosted Jira uses REST v2 `POST /rest/api/2/search` with `startAt` pagination. Cloud queries use Jira Cloud REST v3 [enhanced JQL search](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/) at `POST /rest/api/3/search/jql`, with `nextPageToken` pagination. Requests send `Content-Type: application/json` and `X-Atlassian-Token: no-check`. Native tests use mock HTTP responses and do not require real Jira credentials.

This package requires a Cardbe host implementing the submodule README's explicit HTTP credential rules. Older hosts may ignore this metadata and must not install this package. Updating credential rules requires instance review and re-entry of credentials. The package keeps manifest v1 `bearer_secret` rules; Jira uses Bearer tokens, so the manifest v2 API-key header rules are not needed.
