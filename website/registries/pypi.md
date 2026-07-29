# PyPI

Proxy and cache PyPI through BatleHub for pip, uv, Poetry, and other Python package managers, or host private packages. BatleHub serves the [PEP 503](https://peps.python.org/pep-0503/) Simple index and caches wheels and source distributions after the first download.

## At a glance

| | |
|---|---|
| **Config type** | `pypi` |
| **Default upstream** | `pypi.org` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ `twine upload` |

## Proxy setup

Point pip at the Simple index (`~/.pip/pip.conf` on Linux/macOS, `%APPDATA%\pip\pip.ini` on Windows):

```ini
[global]
index-url = https://batlehub.example.com/proxy/<registry>/simple/
```

For **uv**, add the index to `pyproject.toml`:

```toml
[[tool.uv.index]]
name = "batlehub"
url = "https://batlehub.example.com/proxy/<registry>/simple/"
default = true
```

All three clients (pip, uv, Poetry) read the same `simple/` index. BatleHub rewrites the file download links inside the index so wheels and sdists are fetched — and cached — through the proxy rather than directly from `files.pythonhosted.org`.

## Publishing (local / hybrid)

The registry must be in `local` or `hybrid` mode. Build, then upload with `twine` against the `legacy/` (upload) endpoint — the filename, name, and version are derived from the wheel or sdist metadata automatically:

```bash
python -m build

twine upload \
  --repository-url https://batlehub.example.com/proxy/<registry>/legacy/ \
  --username __token__ \
  --password $BATLEHUB_TOKEN \
  dist/*
```

Or configure `~/.pypirc` with a `repository = https://batlehub.example.com/proxy/<registry>/legacy/` entry and run `twine upload --repository batlehub dist/*`.

## Authentication

Twine sends the token as the password with the literal username `__token__`. For installs, pip/uv/Poetry read credentials from `~/.netrc` automatically:

```
machine batlehub.example.com
login <your-user-id>
password <your-token>
```

Alternatively embed them in the URL: `https://__token__:<your-token>@batlehub.example.com/proxy/<registry>/simple/`.

## Notes

After publishing, the package appears in the Simple index immediately:

```bash
curl -s "https://batlehub.example.com/proxy/<registry>/simple/my-package/" \
  -H "Authorization: Bearer $BATLEHUB_TOKEN"
```

## See also

- [User Guide → PyPI](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
