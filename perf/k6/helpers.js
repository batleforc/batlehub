import { check, fail } from "k6";
import http from "k6/http";
import { ADMIN_TOKEN, BASE_URL } from "./config.js";

export function authHeaders(token) {
  return { Authorization: `Bearer ${token || ADMIN_TOKEN}` };
}

/** GET with auth, assert status, return response. */
export function getOk(url, params) {
  const res = http.get(url, { headers: authHeaders(), ...params });
  check(res, { "status 200": (r) => r.status === 200 });
  return res;
}

/** Generate a minimal npm publish payload for a package of ~sizeKb. */
export function npmPublishPayload(name, version, sizeKb) {
  const tarball = generateBase64(sizeKb * 1024);
  return JSON.stringify({
    name,
    versions: {
      [version]: {
        name,
        version,
        description: "k6 perf test package",
        dist: {
          tarball: `${BASE_URL}/proxy/perf-npm/${name}/-/${name}-${version}.tgz`,
          shasum: "aabbccdd112233445566778899aabbccdd112233",
        },
      },
    },
    _attachments: {
      [`${name}-${version}.tgz`]: {
        content_type: "application/octet-stream",
        data: tarball,
      },
    },
  });
}

/** Return a base64 string of ~byteLen bytes (padded to nearest 3). */
function generateBase64(byteLen) {
  const chars =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const b64len = Math.ceil(byteLen / 3) * 4;
  let s = "";
  for (let i = 0; i < b64len; i++) {
    s += chars[Math.floor(Math.random() * 64)];
  }
  return s;
}
