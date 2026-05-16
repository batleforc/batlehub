import { defineConfig } from "@hey-api/openapi-ts";

// Generate the spec file with: cargo run -p proxy-cache-server -- dump-spec > openapi.json
// Or set OPENAPI_SPEC to a URL for a running server, e.g. http://localhost:8080/api/openapi.json
export default defineConfig({
  input: process.env.OPENAPI_SPEC ?? "openapi.json",
  output: "src/client",
  plugins: ["@hey-api/typescript", "@hey-api/sdk", "@hey-api/client-fetch"],
});
