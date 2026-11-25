# Local storage keys and sync (Q5)

What this app writes to `localStorage`, what the sync layer (`hub/sync.js`) uploads to the user's
account, and what it deliberately ignores. This page answers the spec's open question Q5 ("what are
Vela's key names?") **by measurement, not by reading the spec**: the premise turned out to be wrong.

## The measured finding: no Vela workspace key is written

The Vela bundle contains a workspace-persistence factory (`function sZ(e){...}` with a
`vela-workspace` default key), **but `hub/vela.html:148` passes `persist: false`**, so the workspace
key is never written in this app. There is no runtime inventory of Vela keys to take, because no
runtime Vela key occurs.

`vela:` stays in the sync layer's prefix list as a deliberate placeholder: if a later Vela option
turns persistence on, the key lands inside the allow-list without another plan.

## The keys that exist today

| Key | Written by | Page | In the iframe? | Synced? |
|---|---|---|---|---|
| `chartHub` | `hub/hub.html` (`save()`) | parent | no | yes |
| `chartHubTheme` | `hub/theme.js:36` (`HubSettingsSave`) | every page | both | yes |
| `chartHubIndicators` | `hub/ind_core.js` | parent | no | yes |
| `chartHubFavs` | `hub/ind_core.js`, `hub/ind_dialog.js:62` | parent | no | yes |
| `chartHubScripts` | `hub/ind_core.js`, `hub/pine_editor.js:45` | parent | no | yes |
| `chartHubDraft` | `hub/pine_editor.js:5,6` | parent | no | yes (last-write-wins, no conflict notice) |
| `tradexDrawFavs` | `hub/drawing_favbar.js:7` | iframe (Vela) | yes | yes |
| `tradexFavBar` | `hub/drawing_favbar.js:7` | iframe (Vela) | yes | yes |

## The sync allow-list, exact

`hub/sync.js` forwards exactly these keys and nothing else:

- Exact names: `chartHub`, `chartHubDraft`, `chartHubFavs`, `chartHubIndicators`,
  `chartHubScripts`, `chartHubTheme`, `tradexDrawFavs`, `tradexFavBar`.
- Prefix: `vela:` (placeholder, see above).
- No wildcard `tradex*`: the spec named two keys, not a namespace.

## Namespaces that are never synced

- `vtv:ver:<key>` — the client's cached server version for a key, written by `hub/sync.js` itself.
- `backup:<key>:<YYYY-MM-DD>-<HHMMSS>` (UTC) — a pre-merge backup of a local value that differed
  from the account's, uploaded at first sign-in. Restore through the notice button or
  `VtvSync.restore(backupKey)`; a backup expires on the server after 30 days.

## Related decisions

- `hub/index.html` carries no `sync.js`: it is a ten-line redirect stub with no storage writers
  (spec item 41 listed it as an engine page; on this branch it is not one).
- Drafts (`chartHubDraft`) sync but never show a conflict notice: last-write-wins per the spec.
