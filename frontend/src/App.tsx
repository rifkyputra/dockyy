import "./App.css";
import Navigation from "./components/Navigation";
import ContainersPage from "./pages/ContainersPage";
import RepositoriesPage from "./pages/RepositoriesPage";
import CloudflareTunnelPage from "./pages/CloudflareTunnelPage";
import RepositoryDetail from "./components/RepositoryDetail";
import LoginPage from "./pages/LoginPage";
import ProtectedRoute from "./components/ProtectedRoute";
import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";

function App() {
  return (
    <BrowserRouter>
      <div className="min-h-screen bg-base-200">
        {/* Navigation */}
        <Navigation />

        <div className="container mx-auto p-4 space-y-6">
          <Routes>
            <Route path="/login" element={<LoginPage />} />
            <Route path="/" element={<ProtectedRoute />}>
              <Route index element={<Navigate to="/containers" replace />} />
              <Route path="containers" element={<ContainersPage />} />
              <Route path="repositories" element={<RepositoriesPage />} />
              <Route path="repositories/:id" element={<RepositoryDetail />} />
              <Route
                path="cloudflare-tunnel"
                element={<CloudflareTunnelPage />}
              />
            </Route>
          </Routes>
        </div>
      </div>
    </BrowserRouter>
  );
}

export default App;
