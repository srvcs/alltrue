# srvcs-alltrue

The universal-quantifier service of the srvcs.cloud distributed standard
library.

Its single concern: **are all the values in a list true?** It does no logic of
its own. It folds the list through [`srvcs-and`](https://github.com/srvcs/and),
starting from `true`:

```text
acc = true
for v in values:
    acc = and(acc, v)   # one HTTP call to srvcs-and per element
```

The conjunction of the **empty list** is `true`, and makes no dependency calls
at all (the vacuous truth).

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/` | Service identity, concern, and dependency list |
| `POST` | `/` | Report whether every value in `values` is `true` |
| `GET` | `/healthz` `/readyz` `/metrics` `/openapi.json` | srvcs service standard surface |

```sh
curl -s -X POST localhost:8080/ -H 'content-type: application/json' -d '{"values": [true, true, true]}'
# {"values":[true,true,true],"result":true}
```

Responses:

- `200 {"values": [...], "result": bool}` — evaluated.
- `422` — an element is not a valid boolean, forwarded from `srvcs-and`.
- `500` — `srvcs-and` returned an unusable response.
- `503` — the `srvcs-and` dependency is unavailable.

## Dependencies

- [`srvcs-and`](https://github.com/srvcs/and)

`srvcs-alltrue` is an orchestrator over a boolean leaf service. Its operands are
booleans, so it does not validate them itself — validation propagates from
`srvcs-and`, whose `422`s it forwards unchanged. A single request fans out
across the dependency graph: one `alltrue → and` call per list element.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `SRVCS_BIND_ADDR` | `0.0.0.0:8080` | Bind address |
| `SRVCS_AND_URL` | `http://127.0.0.1:8081` | Base URL of `srvcs-and` |
| `SRVCS_ENV` | `development` | Environment label for logs |
| `RUST_LOG` | `info,tower_http=info` | Tracing filter |

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Orchestration tests stand up a mock `srvcs-and` in-process that **actually
computes** `a && b` from the request body, so the fold is genuinely exercised
(e.g. `alltrue([true, false]) == false`). See
[`srvcs/platform`](https://github.com/srvcs/platform) for the shared standard.

> Note: the `cargoHash` in `flake.nix` is inherited from the template and must be
> refreshed with a `nix build` before the Nix gates pass.
