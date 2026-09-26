import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App.jsx'
import './index.css'

class ErrorBoundary extends React.Component {
  constructor(props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error) {
    return { hasError: true, error };
  }

  componentDidCatch(error, errorInfo) {
    console.error("Dashboard caught render error:", error, errorInfo);
  }

  render() {
    if (this.state.hasError) {
      return (
        <div style={{ backgroundColor: '#0a0a0a', color: '#ef4444', minHeight: '100vh', padding: '2rem', fontFamily: 'monospace' }}>
          <h2 style={{ fontSize: '1.25rem', fontWeight: 'bold', marginBottom: '1rem' }}>Dashboard Render Error</h2>
          <pre style={{ backgroundColor: '#141414', padding: '1rem', borderRadius: '0.5rem', overflow: 'auto', border: '1px solid #333' }}>
            {this.state.error?.toString()}
          </pre>
          <button
            onClick={() => window.location.reload()}
            style={{ marginTop: '1rem', padding: '0.5rem 1rem', backgroundColor: '#222', color: '#fff', border: '1px solid #444', borderRadius: '0.25rem', cursor: 'pointer' }}
          >
            Reload Dashboard
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

ReactDOM.createRoot(document.getElementById('root')).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>
)