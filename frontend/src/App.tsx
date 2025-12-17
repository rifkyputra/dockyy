import "./App.css";
import ContainersTable from "./components/ContainersTable";
import RepositoriesTable from "./components/RepositoriesTable";
import RepositoryDetail from "./components/RepositoryDetail";
import { BrowserRouter, Routes, Route, Link } from "react-router-dom";

function App() {
  return (
    <BrowserRouter>
      <div className="min-h-screen bg-base-200">
        {/* Header */}
        <div className="navbar bg-linear-to-r from-primary to-secondary text-primary-content shadow-lg">
          <div className="flex-1">
            <Link to="/" className="btn btn-ghost text-xl">
              🐳 Dockyy
            </Link>
          </div>
        </div>

        <div className="container mx-auto p-4 space-y-6">
          <Routes>
            <Route
              path="/"
              element={
                <>
                  {/* Containers Section */}
                  <ContainersTable />

                  {/* Repositories Section */}
                  <RepositoriesTable />
                </>
              }
            />
            <Route path="/repositories/:id" element={<RepositoryDetail />} />
          </Routes>
        </div>
      </div>
    </BrowserRouter>
  );
}

export default App;
