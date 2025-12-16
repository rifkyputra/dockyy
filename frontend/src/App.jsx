import { useState, useEffect } from 'react'
import './App.css'

function App() {
  const [containers, setContainers] = useState([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(null)

  useEffect(() => {
    fetchContainers()
  }, [])

  const fetchContainers = async () => {
    try {
      setLoading(true)
      const response = await fetch('/api/containers')
      if (!response.ok) throw new Error('Failed to fetch containers')
      const data = await response.json()
      setContainers(data)
    } catch (err) {
      setError(err.message)
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="App">
      <header className="App-header">
        <h1>🐳 Dockyy - Docker Dashboard</h1>
      </header>
      <main>
        {loading && <p>Loading containers...</p>}
        {error && <p className="error">Error: {error}</p>}
        {!loading && !error && (
          <div className="container-list">
            <h2>Containers ({containers.length})</h2>
            {containers.length === 0 ? (
              <p>No containers found</p>
            ) : (
              <table>
                <thead>
                  <tr>
                    <th>ID</th>
                    <th>Name</th>
                    <th>Status</th>
                    <th>Image</th>
                  </tr>
                </thead>
                <tbody>
                  {containers.map((container) => (
                    <tr key={container.id}>
                      <td>{container.id}</td>
                      <td>{container.name}</td>
                      <td className={`status-${container.status}`}>
                        {container.status}
                      </td>
                      <td>{container.image}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
            <button onClick={fetchContainers}>Refresh</button>
          </div>
        )}
      </main>
    </div>
  )
}

export default App
