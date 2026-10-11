# Cardbe Plugin: Jira Tickets

Import Jira tickets into Cardbe using custom JQL. Built on [Cardbe Plugin API](https://github.com/ldslds449/cardbe-plugin-api) 1.0.0, using the typed `plugin` world.

## Configuration

1. Choose a target board. Missing Jira status columns are created automatically during sync. Existing names are matched case-sensitively, ignoring leading/trailing whitespace; duplicate matching names fail the entire sync.
2. Install the ZIP, create an instance, and select a board.
3. Configure the API type, API base URL, Jira site URL, JQL, and the `token` secret.
4. In the permissions section, add the API base URL's host (for example, `jira.example.com`) and select it to approve access. For Cloud, approve the default `api.atlassian.com` host.

The manifest enables `allow_custom_domains`, so the same package can use different Jira API hosts without rebuilding for each host. Entering `base_url` does not automatically add its host to the pending approval list, and the plugin cannot approve hosts itself. The plugin reads the URL from `config_json`; the host checks authorization when the plugin calls `http-get`.

| Type | API base URL | Jira site URL | Token |
| --- | --- | --- | --- |
| Cloud | `https://api.atlassian.com/ex/jira/<cloudId>` | `https://your-team.atlassian.net` | OAuth access token with Jira read permissions |
| Data Center | `https://jira.example.com` (may include a `/jira` context path) | Same as API base URL | Personal access token |

Cloud email + API token authentication uses Basic auth and **cannot be used directly**, because the current Cardbe host only attaches Bearer tokens. The plugin does not access secrets or put credentials in URLs. Users must update OAuth tokens in Cardbe; this plugin does not refresh tokens automatically.

Example JQL: `project = APP AND fixVersion = "1.2.0" ORDER BY updated ASC`.

## Import behavior

- Title: `APP-123 ticket summary`.
- Column: reuse a column matching the status name, or create one when missing. Column creation and ticket upserts commit atomically; subsequent syncs move tickets when their status changes. At most 200 columns can be created per run.
- Tags: version names from `fixVersions`, deduplicated with blank names skipped.
- Description: configurable `description_template`, defaulting to `{url}\n\n{description}`. Supports `{url}`, `{description}`, `{key}`, and `{summary}`; unknown placeholders remain literal, and an empty template produces an empty description. Inserted Jira text is never interpreted as a template. Changing the template updates descriptions still managed by the source; manual card edits remain preserved. The original Jira description supports Data Center strings and Cloud ADF text, paragraphs, and text hyperlinks. ADF images, attachments, and layout are not imported.
- Jira site + issue ID forms a stable identifier, so renaming a ticket key does not create a new card.
- Each sync queries the full JQL result with pagination, up to 100 requests and 2,000 cards. Failures do not return partial updates.
- Tickets that no longer match the JQL or have been deleted remain on the board: the API supports upserts only, without deletion. Cardbe also preserves manual overrides, so manually moved cards may stop following Jira statuses.
- Cardbe manages scheduling. Use one instance per source on each board.

## Logs

Each successful execution records `TICKETS_SYNCED` (unique tickets returned) and `COLUMNS_CREATED` (new columns proposed). These counts describe plugin output; after instance board authorization, Cardbe validates and commits output directly without a per-run preview. The host records failures separately and does not preserve guest logs on failed runs. See [CHANGELOG.md](CHANGELOG.md) for the change log.

## Build and validation

Requires Rust, PowerShell, and wasm-tools 1.259.0:

```powershell
git submodule update --init
rustup target add wasm32-unknown-unknown
cargo test --locked
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
./scripts/build.ps1
# Optional: set the package's default API host; other hosts can still be added manually
./scripts/build.ps1 -Domain jira.example.com
```

Outputs `dist/manifest.json`, `dist/component.wasm`, and ZIP/SHA-256 files in the repository root. Verify the checksum before installing the ZIP. The Cardbe host allows only public HTTPS on port 443 and rejects private IP addresses and redirects. Jira instances accessible only on a private network require additional host support.

## API provenance

`cardbe-plugin-api/wit/plugin.wit` is provided by a Git submodule pinned to API commit `9ea0c295c10c9f9f079d4474ba7f744b860171ba`, using the typed board queries and mixed changeset contract. Its Git blob SHA-256 is `6de624be5cd120cc85fe93889c0f02f6b259a024c102c1143ba23e1b107a5e8f`. Install on a Cardbe host built against the same typed API revision; previous draft hosts are incompatible despite the unchanged API version.

Cloud queries use [enhanced JQL search](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/); Data Center uses REST v2 search. Native tests use mock HTTP responses and do not require real Jira credentials.
