namespace OpenKache;

public class Client
{
    public void Connect(string host, int port) {}
    public byte[]? Get(string key) => null;
    public void Set(string key, byte[] value, int? ttl = null) {}
    public bool Delete(string key) => false;
    public void Close() {}
}
