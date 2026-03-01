<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRouter, useRoute } from "vue-router";
import { api } from "./api";
import LoginView from "./views/LoginView.vue";

const router = useRouter();
const route = useRoute();
const loggedIn = ref(false);
const checking = ref(true);

const pageTitles: Record<string, string> = {
  "/containers": "Containers",
  "/repositories": "Repositories",
  "/deployments": "Deployments",
  "/proxy": "Reverse Proxy",
  "/health": "Server Health",
  "/settings": "Settings",
};

function pageTitle(): string {
  if (route.path.startsWith("/repository/")) return "Repository Details";
  return pageTitles[route.path] || "Dashboard";
}

const navItems = [
  { section: "Management" },
  { path: "/containers", icon: "\u{1F4E6}", label: "Containers" },
  { path: "/repositories", icon: "\u{1F4C2}", label: "Repositories" },
  { path: "/deployments", icon: "\u{1F680}", label: "Deployments" },
  { path: "/proxy", icon: "\u{1F500}", label: "Proxy" },
  { section: "System" },
  { path: "/health", icon: "\u{1F3E5}", label: "Health" },
  { path: "/settings", icon: "\u2699\uFE0F", label: "Settings" },
];

async function checkAuth() {
  const token = localStorage.getItem("dockyy_token");
  if (!token) {
    loggedIn.value = false;
    checking.value = false;
    return;
  }
  try {
    const res = await api.verify(token);
    loggedIn.value = res.valid;
  } catch {
    loggedIn.value = false;
  }
  checking.value = false;
}

function onLogin() {
  loggedIn.value = true;
}

function logout() {
  localStorage.removeItem("dockyy_token");
  loggedIn.value = false;
}

function refresh() {
  router.replace({ path: route.fullPath, force: true } as any);
  // Force re-render by toggling a key
  refreshKey.value++;
}

const refreshKey = ref(0);

onMounted(checkAuth);
</script>

<template>
  <div v-if="checking" class="spinner" style="min-height: 100vh"></div>
  <LoginView v-else-if="!loggedIn" @login="onLogin" />
  <div v-else class="layout">
    <aside class="sidebar">
      <div class="sidebar-header">
        <div class="sidebar-logo">D</div>
        <span class="sidebar-title">Dockyy</span>
        <span class="sidebar-version">v0.2</span>
      </div>
      <nav class="sidebar-nav">
        <template v-for="item in navItems" :key="item.section || item.path">
          <div v-if="item.section" class="nav-section">{{ item.section }}</div>
          <router-link
            v-else
            :to="item.path!"
            class="nav-item"
            :class="{ active: route.path === item.path || (item.path === '/repositories' && route.path.startsWith('/repository/')) }"
          >
            <span class="icon">{{ item.icon }}</span> {{ item.label }}
          </router-link>
        </template>
      </nav>
      <div class="sidebar-footer">
        <button class="btn btn-ghost btn-sm" style="width: 100%" @click="logout">Sign Out</button>
      </div>
    </aside>
    <div class="main-content">
      <header class="topbar">
        <h1 class="topbar-title">{{ pageTitle() }}</h1>
        <div class="topbar-actions">
          <button class="btn btn-ghost btn-sm" @click="refresh">&#x21bb; Refresh</button>
        </div>
      </header>
      <main class="page-content">
        <router-view :key="refreshKey" />
      </main>
    </div>
  </div>
  <div id="modal-root"></div>
</template>
