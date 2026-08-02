import { createRouter, createWebHistory } from "vue-router";

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/",
      redirect: "/frequent",
    },
    {
      path: "/frequent",
      name: "frequent",
      component: () => import("../views/FrequentView.vue"),
    },
    {
      path: "/passwords",
      name: "passwords",
      component: () => import("../views/PasswordView.vue"),
    },
    {
      path: "/apps",
      name: "apps",
      component: () => import("../views/AppView.vue"),
    },
    {
      path: "/snippets",
      name: "snippets",
      component: () => import("../views/SnippetView.vue"),
    },
    {
      path: "/temp",
      name: "temp",
      component: () => import("../views/TempContentView.vue"),
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("../views/SettingsView.vue"),
    },
  ],
});

export default router;