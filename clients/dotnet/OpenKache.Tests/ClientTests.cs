// Copyright (C) 2026 OpenStd Inc.

using System;
using System.Collections.Concurrent;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading.Tasks;
using Xunit;

namespace OpenKache.Tests;

public class ClientTests
{
    private static int MockServer(Func<string, string> handler)
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var port = ((IPEndPoint)listener.LocalEndpoint).Port;

        _ = Task.Run(() =>
        {
            while (true)
            {
                var client = listener.AcceptTcpClient();
                _ = HandleConnection(client, handler);
            }
        });

        return port;
    }

    private static async Task HandleConnection(TcpClient client, Func<string, string> handler)
    {
        using (client)
        using (var stream = client.GetStream())
        {
            var buf = new byte[4096];
            var sb = new StringBuilder();

            while (true)
            {
                int n = await stream.ReadAsync(buf, 0, buf.Length);
                if (n <= 0) break;

                sb.Append(Encoding.UTF8.GetString(buf, 0, n));
                string data = sb.ToString();

                int idx;
                while ((idx = data.IndexOf('\n')) >= 0)
                {
                    var line = data[..idx].Trim();
                    data = data[(idx + 1)..];
                    if (string.IsNullOrEmpty(line)) continue;

                    var resp = handler(line) + "\n";
                    await stream.WriteAsync(Encoding.UTF8.GetBytes(resp));
                }
                sb.Clear();
                sb.Append(data);
            }
        }
    }

    [Fact]
    public void Connect_Succeeds()
    {
        var port = MockServer(_ => "");
        var client = new Client("127.0.0.1", port);
        client.Connect();
        client.Close();
    }

    [Fact]
    public void Connect_RejectsWrongPort()
    {
        var client = new Client("127.0.0.1", 1, 1000);
        Assert.Throws<OpenKacheException>(() => client.Connect());
    }

    [Fact]
    public void SetAndGet()
    {
        var store = new ConcurrentDictionary<string, string>();
        var port = MockServer(line =>
        {
            var parts = line.Split(' ', 3);
            return parts[0] switch
            {
                "SET" when parts.Length >= 3 => (store[parts[1]] = parts[2], "OK").Item2,
                "GET" => store.TryGetValue(parts[1], out var v) ? $"OK {v}" : "NOT_FOUND",
                _ => "ERR unknown command"
            };
        });

        var client = new Client("127.0.0.1", port);
        client.Connect();
        client.Set("foo", "bar");
        Assert.Equal("bar", client.Get("foo"));
        Assert.Null(client.Get("nonexistent"));
        client.Close();
    }

    [Fact]
    public void Delete()
    {
        var store = new ConcurrentDictionary<string, string>();
        store["foo"] = "bar";
        var port = MockServer(line =>
        {
            var parts = line.Split(' ');
            return parts[0] switch
            {
                "DEL" => store.TryRemove(parts[1], out _) ? "OK" : "NOT_FOUND",
                _ => "ERR unknown command"
            };
        });

        var client = new Client("127.0.0.1", port);
        client.Connect();
        Assert.True(client.Delete("foo"));
        Assert.False(client.Delete("nonexistent"));
        client.Close();
    }

    [Fact]
    public void Ping()
    {
        var port = MockServer(line => line == "PING" ? "PONG" : "ERR unknown command");

        var client = new Client("127.0.0.1", port);
        client.Connect();
        Assert.True(client.Ping());
        client.Close();
    }

    [Fact]
    public void Flush()
    {
        var flushed = false;
        var port = MockServer(line =>
        {
            if (line == "FLUSH") { flushed = true; return "OK"; }
            return "ERR unknown command";
        });

        var client = new Client("127.0.0.1", port);
        client.Connect();
        client.Flush();
        Assert.True(flushed);
        client.Close();
    }
}
