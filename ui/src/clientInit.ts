import { client } from "@/client/client.gen";
import { API_BASE_URL } from "@/config";
import { initAuth } from "@/composables/useAuth";

// Must be the first import in main.ts so baseUrl is set before any request
// (including the identity fetch `initAuth` kicks off) leaves this module.
client.setConfig({ baseUrl: API_BASE_URL });
initAuth();
