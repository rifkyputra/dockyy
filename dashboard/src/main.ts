import "./style.css";
import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import App from "./App.vue";
import ContainersView from "./views/ContainersView.vue";
import RepositoriesView from "./views/RepositoriesView.vue";
import RepositoryDetailView from "./views/RepositoryDetailView.vue";
import DeploymentsView from "./views/DeploymentsView.vue";
import ProxyView from "./views/ProxyView.vue";
import HealthView from "./views/HealthView.vue";
import SettingsView from "./views/SettingsView.vue";

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/containers" },
    { path: "/containers", component: ContainersView },
    { path: "/repositories", component: RepositoriesView },
    { path: "/repository/:id", component: RepositoryDetailView },
    { path: "/deployments", component: DeploymentsView },
    { path: "/proxy", component: ProxyView },
    { path: "/health", component: HealthView },
    { path: "/settings", component: SettingsView },
  ],
});

const app = createApp(App);
app.use(router);
app.mount("#app");
