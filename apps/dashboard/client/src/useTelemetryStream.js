import { useState, useEffect, useRef } from 'react';

/**
 * useTelemetryStream
 * Connects to the SSE endpoint /api/live/stream for realtime push telemetry.
 * Automatically handles reconnection with exponential backoff.
 */
export function useTelemetryStream() {
  const [telemetry, setTelemetry] = useState(null);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState(null);
  const eventSourceRef = useRef(null);
  const retryTimeoutRef = useRef(null);
  const retryDelayRef = useRef(1000);

  useEffect(() => {
    let unmounted = false;

    function connect() {
      if (unmounted) return;
      try {
        const es = new EventSource('/api/live/stream');
        eventSourceRef.current = es;

        es.onopen = () => {
          if (unmounted) return;
          setConnected(true);
          setError(null);
          retryDelayRef.current = 1000;
        };

        es.onmessage = (event) => {
          if (unmounted) return;
          try {
            const data = JSON.parse(event.data);
            setTelemetry(data);
          } catch (err) {
            console.warn('Failed to parse SSE telemetry packet:', err);
          }
        };

        es.onerror = (err) => {
          if (unmounted) return;
          setConnected(false);
          setError('Disconnected from live stream');
          es.close();

          // Exponential backoff reconnect up to 10s
          const delay = Math.min(10000, retryDelayRef.current);
          retryDelayRef.current = delay * 1.5;
          clearTimeout(retryTimeoutRef.current);
          retryTimeoutRef.current = setTimeout(connect, delay);
        };
      } catch (err) {
        if (!unmounted) {
          setError(err.message);
          const delay = Math.min(10000, retryDelayRef.current);
          retryDelayRef.current = delay * 1.5;
          retryTimeoutRef.current = setTimeout(connect, delay);
        }
      }
    }

    connect();

    return () => {
      unmounted = true;
      clearTimeout(retryTimeoutRef.current);
      if (eventSourceRef.current) {
        eventSourceRef.current.close();
      }
    };
  }, []);

  return { telemetry, connected, error };
}
