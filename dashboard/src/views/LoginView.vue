<script setup lang="ts">
import { ref } from "vue";
import { api } from "../api";

const emit = defineEmits<{ login: [] }>();

const username = ref("");
const password = ref("");
const error = ref("");

async function handleLogin() {
  error.value = "";
  try {
    const { token } = await api.login(username.value, password.value);
    localStorage.setItem("dockyy_token", token);
    emit("login");
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : "Login failed";
  }
}
</script>

<template>
  <div class="login-wrapper">
    <div class="login-card">
      <div class="login-header">
        <div class="login-logo">D</div>
        <h1>Welcome to Dockyy</h1>
        <p>Sign in to manage your containers</p>
      </div>
      <form @submit.prevent="handleLogin">
        <div class="form-group">
          <label class="form-label" for="username">Username</label>
          <input v-model="username" class="form-input" id="username" type="text" placeholder="admin" autocomplete="username" />
        </div>
        <div class="form-group">
          <label class="form-label" for="password">Password</label>
          <input v-model="password" class="form-input" id="password" type="password" placeholder="••••••••" autocomplete="current-password" />
        </div>
        <div v-if="error" class="form-error">{{ error }}</div>
        <button class="btn btn-primary btn-login" type="submit">Sign In</button>
      </form>
    </div>
  </div>
</template>
