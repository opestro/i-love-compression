using System.Diagnostics;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Windows;

namespace ILoveCompression;

/// <summary>One JSON line from ilc-engine (protocol in engine/src/main.rs).</summary>
public sealed class EngineEvent
{
    [JsonPropertyName("event")] public string Event { get; set; } = "";
    [JsonPropertyName("progress")] public double? Progress { get; set; }
    [JsonPropertyName("index")] public int? Index { get; set; }
    [JsonPropertyName("output")] public string? Output { get; set; }
    [JsonPropertyName("before")] public long Before { get; set; }
    [JsonPropertyName("after")] public long After { get; set; }
    [JsonPropertyName("note")] public string? Note { get; set; }
    [JsonPropertyName("message")] public string? Message { get; set; }
}

public sealed class Hardware
{
    [JsonPropertyName("nvenc_hevc")] public bool NvencHevc { get; set; }
    [JsonPropertyName("nvenc_av1")] public bool NvencAv1 { get; set; }
}

/// <summary>
/// One run of ilc-engine.exe, which ships next to this app. Closing its stdin cancels it;
/// Windows does the same when this app exits or crashes, so ffmpeg never outlives us.
/// </summary>
public sealed class EngineRun
{
    public static string EnginePath => Path.Combine(AppContext.BaseDirectory, "ilc-engine.exe");

    private readonly Process _process;
    private readonly Action<EngineEvent> _onEvent;
    private readonly Action _onExit;

    public EngineRun(IEnumerable<string> args, Action<EngineEvent> onEvent, Action onExit)
    {
        _onEvent = onEvent;
        _onExit = onExit;
        var info = new ProcessStartInfo(EnginePath)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8,
        };
        foreach (var a in args) info.ArgumentList.Add(a);
        _process = new Process { StartInfo = info };
    }

    public void Start()
    {
        _process.Start();
        _ = _process.StandardError.ReadToEndAsync(); // drained so the engine never blocks on it
        _ = Task.Run(ReadEvents);
    }

    public void Stop()
    {
        try { _process.StandardInput.Close(); } catch (InvalidOperationException) { }
    }

    private async Task ReadEvents()
    {
        string? line;
        while ((line = await _process.StandardOutput.ReadLineAsync()) != null)
        {
            EngineEvent? e = null;
            try { e = JsonSerializer.Deserialize<EngineEvent>(line); } catch (JsonException) { }
            if (e != null) Application.Current.Dispatcher.Invoke(() => _onEvent(e));
        }
        await _process.WaitForExitAsync();
        Application.Current.Dispatcher.Invoke(_onExit);
    }

    /// <summary>What the GPU can do; NVENC is test-encoded, so a too-old driver reads as "no".</summary>
    public static async Task<Hardware> ProbeAsync()
    {
        try
        {
            var info = new ProcessStartInfo(EnginePath, "probe")
            {
                UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardInput = true,
            };
            using var p = Process.Start(info)!;
            var json = await p.StandardOutput.ReadLineAsync();
            p.StandardInput.Close();
            return json is null ? new Hardware() : JsonSerializer.Deserialize<Hardware>(json) ?? new Hardware();
        }
        catch (Exception) { return new Hardware(); }
    }
}
