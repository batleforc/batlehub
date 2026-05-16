import { client } from "@/client/client.gen";
import { API_BASE_URL } from "@/config";

// Must be the first import in main.ts so baseUrl is set before any composable
// runs its module-level side effects (e.g. useAuth's refreshIdentity on load).
client.setConfig({ baseUrl: API_BASE_URL });
