using System.ComponentModel;
using System.IO;
using System.Windows;

namespace ILoveCompression;

public enum JobStatus { Waiting, Running, Done, Failed }

/// <summary>A file in the list. Every change re-raises all properties; the row just redraws.</summary>
public sealed class Job : INotifyPropertyChanged
{
    public Job(string path, bool isVideo) { Path = path; IsVideo = isVideo; }

    public string Path { get; }
    public bool IsVideo { get; }
    public string Name => System.IO.Path.GetFileName(Path);
    public JobStatus Status { get; private set; } = JobStatus.Waiting;
    public double Progress { get; private set; }
    public long Before { get; private set; }
    public long After { get; private set; }
    public string? Output { get; private set; }
    public string Message { get; private set; } = "";

    // Segoe Fluent Icons: clock, sync, completed, error
    public string Glyph => Status switch
    {
        JobStatus.Waiting => "",
        JobStatus.Running => "",
        JobStatus.Done => "",
        _ => "",
    };

    public string Detail => Status switch
    {
        JobStatus.Waiting => "Waiting",
        JobStatus.Running => IsVideo ? $"{(int)(Progress * 100)}%" : "Compressing…",
        JobStatus.Done => $"{Bytes(Before)} → {Bytes(After)}   {Change}{(Message.Length > 0 ? "   · " + Message : "")}",
        _ => Message,
    };

    private string Change
    {
        get
        {
            if (Before <= 0) return "";
            var saved = 100 - (int)(After * 100 / Before);
            return saved >= 0 ? $"−{saved}%" : $"+{-saved}%";
        }
    }

    public Visibility ProgressVisibility => Status == JobStatus.Running && IsVideo ? Visibility.Visible : Visibility.Collapsed;
    public Visibility ShowVisibility => Status == JobStatus.Done && Output != null ? Visibility.Visible : Visibility.Collapsed;

    public void SetRunning() { Status = JobStatus.Running; Progress = 0; Changed(); }
    public void SetWaiting() { Status = JobStatus.Waiting; Changed(); }
    public void SetFailed(string message) { Status = JobStatus.Failed; Message = message; Changed(); }

    public void Apply(EngineEvent e)
    {
        switch (e.Event)
        {
            case "progress":
                Progress = e.Progress ?? Progress;
                break;
            case "done":
                Status = JobStatus.Done;
                Progress = 1;
                Before = e.Before;
                After = e.After;
                Output = e.Output;
                Message = e.Note ?? "";
                break;
            default:
                Status = JobStatus.Failed;
                Message = e.Message ?? "Failed";
                break;
        }
        Changed();
    }

    public static string Bytes(long n) => n switch
    {
        >= 1_000_000_000 => $"{n / 1e9:0.0} GB",
        >= 1_000_000 => $"{n / 1e6:0.0} MB",
        _ => $"{Math.Max(1, n / 1000)} KB",
    };

    /// What screen readers announce for the row (ListView uses the item's ToString()).
    public override string ToString() => $"{Name}, {Detail}";

    public event PropertyChangedEventHandler? PropertyChanged;
    private void Changed() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(string.Empty));
}
