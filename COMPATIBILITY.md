# Host contract provenance

- Source: https://github.com/ldslds449/cardbe-plugin-api, `wit/plugin.wit`.
- Submodule: `cardbe-plugin-api`, pinned to tag `v1.3.0`, commit `dda7023819eaed9776b2665a6adb620b51a61181`.
- API: `cardbe:plugin@1.3.0`, world `plugin`.
- WIT SHA-256 (Git blob, LF line endings): `0625a093462ce54625aa775566d415f70563abefef8527a5ed17fd6220e16762`.
- Required manifest API version: `1.3.0`.
- Manifest format: `manifest_version` omitted (v1); the credential rules use `bearer_secret`.
- Required storage contract: `1`.

The WIT is consumed directly from the pinned API submodule. Initialize it with
`git submodule update --init`; builds do not download host definitions.
Compatibility also includes configuration, typed board snapshots, HTTP policy and changeset
semantics; unchanged WIT alone is not a complete compatibility test.

API 1.2.0 adds source priority reporting; API 1.3.0 adds filtered queries and waiting relationships. The plugin reports Jira priorities with `set-external-task-priority`, preserving manual overrides and atomic commit/rollback. It uses `http-send` with bounded network retries and `log-message`. It does not query or replace native task records, waiting relationships, or completion metadata. Install on a Cardbe host supporting API 1.3.0; older API 1.1.0/1.2.0 hosts cannot load this component.

The host must implement explicit `http_credentials` rules for Base URL settings. The manifest binds `token` to `base_url` under `/rest/api/2` and `/rest/api/3`; approved custom hosts receive credentials only within that scope. Older hosts that ignore this metadata are unsupported. Updating these rules requires instance review and credentials to be re-entered. Self-hosted Jira REST v2 with PAT is the default; Jira Cloud REST v3 with OAuth is also supported.
