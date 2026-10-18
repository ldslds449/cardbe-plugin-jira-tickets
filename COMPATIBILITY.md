# Host contract provenance

- Source: https://github.com/ldslds449/cardbe-plugin-api, `wit/plugin.wit`.
- Submodule: `cardbe-plugin-api`, pinned to tag `v1.1.0`, commit `023907c8294dfa5ca62e6078f13436b38d01eca0`.
- API: `cardbe:plugin@1.1.0`, world `plugin`.
- WIT SHA-256 (Git blob, LF line endings): `2243d8404bc212b5bd1335d743b78498296b2497d25eb2b832035259d2859163`.
- Required manifest API version: `1.1.0`.
- Manifest format: `manifest_version` omitted (v1); the credential rules use `bearer_secret`.
- Required storage contract: `1`.

The WIT is consumed directly from the pinned API submodule. Initialize it with
`git submodule update --init`; builds do not download host definitions.
Compatibility also includes configuration, typed board snapshots, HTTP policy and changeset
semantics; unchanged WIT alone is not a complete compatibility test.

API 1.1.0 changed the `task` record (start/created times and the
`start-at-midnight` / `due-at-midnight` flags) and added `http-send` and `log-message`, so this
component is incompatible with API 1.0.0 hosts. The plugin does not read or write
`task` records; it uses `http-send` (with retries on `NETWORK_FAILED`) and
`log-message`. Install on a Cardbe host built against API 1.1.0.

The host must implement explicit `http_credentials` rules for Base URL settings. The manifest binds `token` to `base_url` under `/rest/api/2` and `/rest/api/3`; approved custom hosts receive credentials only within that scope. Older hosts that ignore this metadata are unsupported. Updating these rules requires instance review and credentials to be re-entered. Self-hosted Jira REST v2 with PAT is the default; Jira Cloud REST v3 with OAuth is also supported.
