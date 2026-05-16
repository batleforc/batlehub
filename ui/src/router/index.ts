import { createRouter, createWebHistory } from "vue-router";
import PackageList from "@/pages/PackageList.vue";
import AccessCheck from "@/pages/AccessCheck.vue";
import AdminPackages from "@/pages/AdminPackages.vue";
import AuditLog from "@/pages/AuditLog.vue";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", redirect: "/packages" },
    { path: "/packages", component: PackageList },
    { path: "/access-check", component: AccessCheck },
    { path: "/admin/packages", component: AdminPackages },
    { path: "/admin/audit-log", component: AuditLog },
  ],
});
