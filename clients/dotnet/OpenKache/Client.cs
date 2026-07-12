#nullable enable
using System;
using System.Net.Sockets;
using System.Text;

namespace OpenKache;

public class Client : IDisposable
{
    private TcpClient? _tcp;
    private NetworkStream? _stream;
    private readonly string _host;
    private readonly int _port;
    private readonly int _timeoutMs;
    private byte[] _readBuf = new byte[4096];
    private int _readLen = 0;

    public Client(string host = "127.0.0.1", int port = 7123, int timeoutMs = 5000)
    {
        _host = host;
        _port = port;
        _timeoutMs = timeoutMs;
    }

    public void Connect()
    {
        if (_tcp?.Connected == true) return;
        _tcp = new TcpClient();
        try
        {
            var task = _tcp.ConnectAsync(_host, _port);
            if (!task.Wait(_timeoutMs))
            {
                _tcp.Dispose();
                _tcp = null;
                throw new OpenKacheException("TIMEOUT", "Connection timeout");
            }
        }
        catch (AggregateException ae)
        {
            _tcp.Dispose();
            _tcp = null;
            var inner = ae.InnerException;
            throw new OpenKacheException("CONNECTION_REFUSED", inner?.Message ?? ae.Message);
        }
        _stream = _tcp.GetStream();
        _stream.ReadTimeout = _timeoutMs;
        _stream.WriteTimeout = _timeoutMs;
    }

    private string Send(string cmd)
    {
        if (_stream == null || _tcp?.Connected != true)
            throw new OpenKacheException("CONNECTION_REFUSED", "Not connected");

        var data = Encoding.UTF8.GetBytes(cmd + "\n");
        _stream.Write(data, 0, data.Length);

        while (true)
        {
            for (int i = 0; i < _readLen; i++)
            {
                if (_readBuf[i] == '\n')
                {
                    var line = Encoding.UTF8.GetString(_readBuf, 0, i).Trim();
                    _readLen -= i + 1;
                    Array.Copy(_readBuf, i + 1, _readBuf, 0, _readLen);
                    return line;
                }
            }
            int n = _stream.Read(_readBuf, _readLen, _readBuf.Length - _readLen);
            if (n <= 0)
                throw new OpenKacheException("CONNECTION_REFUSED", "Connection closed");
            _readLen += n;
        }
    }

    public string? Get(string key)
    {
        var resp = Send($"GET {key}");
        if (resp == "NOT_FOUND") return null;
        if (resp.StartsWith("OK ")) return resp[3..];
        throw new OpenKacheException("PROTOCOL_ERROR", $"Unexpected response: {resp}");
    }

    public void Set(string key, string value)
    {
        var resp = Send($"SET {key} {value}");
        if (resp != "OK")
            throw new OpenKacheException("PROTOCOL_ERROR", $"Unexpected response: {resp}");
    }

    public bool Delete(string key)
    {
        var resp = Send($"DEL {key}");
        return resp switch
        {
            "OK" => true,
            "NOT_FOUND" => false,
            _ => throw new OpenKacheException("PROTOCOL_ERROR", $"Unexpected response: {resp}")
        };
    }

    public bool Ping()
    {
        var resp = Send("PING");
        if (resp == "PONG") return true;
        throw new OpenKacheException("PROTOCOL_ERROR", $"Unexpected response: {resp}");
    }

    public void Flush()
    {
        var resp = Send("FLUSH");
        if (resp != "OK")
            throw new OpenKacheException("PROTOCOL_ERROR", $"Unexpected response: {resp}");
    }

    public void Close()
    {
        _stream?.Close();
        _tcp?.Close();
        _stream = null;
        _tcp = null;
    }

    public void Dispose()
    {
        Close();
    }
}

public class OpenKacheException : Exception
{
    public string Code { get; }

    public OpenKacheException(string code, string message) : base($"[{code}] {message}")
    {
        Code = code;
    }
}

