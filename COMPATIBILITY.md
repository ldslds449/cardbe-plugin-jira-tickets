# Host contract provenance

- Source: https://github.com/ldslds449/cardbe-plugin-api, `wit/plugin.wit`.
- Submodule: `cardbe-plugin-api`, pinned to commit `435e0bf1e194fb7b0e3ab07a508d05624590f16b`.
- API: `cardbe:plugin@1.0.0`, world `plugin`.
- WIT SHA-256 (Git blob, LF line endings): `6de624be5cd120cc85fe93889c0f02f6b259a024c102c1143ba23e1b107a5e8f`.
- Required manifest API version: `1.0.0`.
- Required storage contract: `1`.

The WIT is consumed directly from the pinned API submodule. Initialize it with
`git submodule update --init`; builds do not download host definitions.
Compatibility also includes configuration, typed board snapshots, HTTP policy and changeset
semantics; unchanged WIT alone is not a complete compatibility test.

This typed ABI is incompatible with the previous `importer` world despite the
unchanged API version. Install on a Cardbe host built against the same revision.

The host must implement explicit `http_credentials` rules for Base URL settings. The manifest binds `token` to `base_url` under `/rest/api/2` and `/rest/api/3`; approved custom hosts receive credentials only within that scope. Older hosts that ignore this metadata are unsupported. Updating these rules requires instance review and credentials to be re-entered. Self-hosted Jira REST v2 with PAT is the default; Jira Cloud REST v3 with OAuth is also supported.
