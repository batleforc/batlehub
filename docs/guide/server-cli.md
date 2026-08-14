# Server binary subcommands

`batlehub` is the server. It has three invocations, and this page is about
the two that are not "start the server".

Not to be confused with [`batlehub-cli`](/use/cli), which is the client a
developer installs — they are different programs, and this page used to be
titled "CLI Reference" inside the configuration reference, which is how a
reader searching for "CLI" found two pages and could not tell them apart.

```
batlehub --config config.toml          # start the server (default: config.toml)
batlehub dump-spec                     # print the OpenAPI JSON spec to stdout
batlehub hash-token <token>            # generate an Argon2id PHC hash for a static token
```

## `dump-spec`

Redirect the spec to a file for use with code generators:

```sh
batlehub dump-spec > openapi.json
```

## `hash-token`

Generates an Argon2id PHC hash that can be stored in `[[auth.tokens]].value` instead of a raw token string. The raw token is only required at generation time and does not need to be stored anywhere.

```sh
# Generate a hash
batlehub hash-token my-secret-token
# $argon2id$v=19$m=65536,t=3,p=4$<salt>$<hash>

# Paste the output directly into the config:
# [[auth.tokens]]
# value = "$argon2id$v=19$m=65536,t=3,p=4$..."
# role = "admin"
```

See [§3.3.1 Argon2id hashed token values](/guide/configuration#argon2id-hashed-token-values-recommended-for-production) for full context.

