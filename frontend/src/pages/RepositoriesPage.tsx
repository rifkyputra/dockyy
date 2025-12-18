import RepositoriesTable from "../components/RepositoriesTable";

export default function RepositoriesPage() {
  return (
    <div className="space-y-6">
      <h1 className="text-3xl font-bold">Repositories</h1>
      <RepositoriesTable />
    </div>
  );
}
