import { Link, useLocation, useNavigate } from "react-router-dom";

export default function Navigation() {
  const location = useLocation();
  const navigate = useNavigate();
  const token = localStorage.getItem("token");

  const isActive = (path: string) => {
    return location.pathname === path ? "active" : "";
  };

  const handleLogout = () => {
    localStorage.removeItem("token");
    navigate("/login");
  };

  if (!token) {
    return null; // Don't show navigation if not authenticated
  }

  return (
    <div className="navbar bg-base-100 shadow-md">
      <div className="flex-1">
        <Link to="/" className="btn btn-ghost text-xl">
          🐳 Dockyy
        </Link>
      </div>
      <div className="flex-none">
        <ul className="menu menu-horizontal px-1">
          <li>
            <Link to="/containers" className={isActive("/containers")}>
              Containers
            </Link>
          </li>
          <li>
            <Link to="/repositories" className={isActive("/repositories")}>
              Repositories
            </Link>
          </li>
          <li>
            <Link
              to="/cloudflare-tunnel"
              className={isActive("/cloudflare-tunnel")}
            >
              Cloudflare Tunnel
            </Link>
          </li>
        </ul>
        <button onClick={handleLogout} className="btn btn-ghost ml-4">
          Logout
        </button>
      </div>
    </div>
  );
}
