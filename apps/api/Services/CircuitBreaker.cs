namespace Gaia.Api.Services;

public enum CircuitState
{
    Closed,
    Open,
    HalfOpen
}

public class CircuitBreaker
{
    private readonly int _failureThreshold;
    private readonly TimeSpan _openDuration;
    private readonly object _lock = new();

    private CircuitState _state = CircuitState.Closed;
    private int _consecutiveFailures;
    private DateTime _openedAt = DateTime.MinValue;

    public CircuitBreaker(int failureThreshold = 3, TimeSpan? openDuration = null)
    {
        _failureThreshold = failureThreshold;
        _openDuration = openDuration ?? TimeSpan.FromSeconds(20);
    }

    public CircuitState State
    {
        get
        {
            lock (_lock)
            {
                if (_state == CircuitState.Open && DateTime.UtcNow - _openedAt >= _openDuration)
                {
                    _state = CircuitState.HalfOpen;
                }
                return _state;
            }
        }
    }

    public bool CanExecute()
    {
        lock (_lock)
        {
            if (_state == CircuitState.Closed) return true;
            if (_state == CircuitState.Open)
            {
                if (DateTime.UtcNow - _openedAt >= _openDuration)
                {
                    _state = CircuitState.HalfOpen;
                    return true; // Allow single probe request
                }
                return false;
            }
            return true; // HalfOpen allows probe
        }
    }

    public bool RecordSuccess()
    {
        lock (_lock)
        {
            var transitioned = _state != CircuitState.Closed;
            _consecutiveFailures = 0;
            _state = CircuitState.Closed;
            return transitioned;
        }
    }

    public bool RecordFailure()
    {
        lock (_lock)
        {
            _consecutiveFailures++;
            if ((_consecutiveFailures >= _failureThreshold || _state == CircuitState.HalfOpen) && _state != CircuitState.Open)
            {
                _state = CircuitState.Open;
                _openedAt = DateTime.UtcNow;
                return true;
            }
            return false;
        }
    }
}
