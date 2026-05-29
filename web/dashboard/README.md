# Capo Dashboard Webclient

Static first-slice browser dashboard for Capo operator workflows.

## Run

```sh
node web/dashboard/scripts/dev-server.mjs
```

Then open:

```text
http://127.0.0.1:4173
```

## Verify

```sh
node web/dashboard/scripts/verify.mjs
```

The first dev server serves fixture data and a mocked server-command endpoint.
It does not read SQLite directly and does not run provider CLIs.
