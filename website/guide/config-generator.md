# Config Generator

Fill in the form below to generate a `config.toml` for your BatleHub instance. The preview on the right updates live as you type.

When you're done, click **Download** to save the file, or **Copy** to paste it into your editor.

::: info What's next?
Place the generated file at the path you pass to `--config` (default: `./config.toml`), then start BatleHub.
For Helm deployments, paste the contents into your `values.yaml` under `registriesRaw` and set the individual fields.
See [Administration](/guide/administration) for a full reference of every option.
:::

<ClientOnly>
  <ConfigGenerator />
</ClientOnly>
